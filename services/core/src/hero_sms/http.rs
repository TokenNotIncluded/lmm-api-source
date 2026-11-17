// Copyright (C) 2026 LIghtJUNction. SPDX-License-Identifier: AGPL-3.0-or-later.
//! Private UDS routes. The host token cannot substitute for an administrator session.
use super::*;
use crate::identity::{self, CredentialKind, permissions as p};
use axum::{
    Json, Router,
    body::Bytes,
    extract::{DefaultBodyLimit, Path as RoutePath, State},
    http::{HeaderMap, StatusCode, header},
    response::{IntoResponse, Response},
    routing::post,
};
use sha2::{Digest, Sha256};
use sqlx::{Postgres, Transaction};
use subtle::ConstantTimeEq;
use tokio::sync::Semaphore;

#[derive(Clone)]
struct StateData {
    service: Service,
    token: [u8; 32],
    slots: Arc<Semaphore>,
}

fn single<'a>(headers: &'a HeaderMap, key: &str) -> Result<&'a str> {
    let mut values = headers.get_all(key).iter();
    let value = values.next().ok_or(Error::Unauthorized)?;
    if values.next().is_some() {
        return Err(Error::Unauthorized);
    }
    value.to_str().map_err(|_| Error::Unauthorized)
}

impl StateData {
    fn check(&self, headers: &HeaderMap) -> Result<()> {
        let value = single(headers, "authorization")?
            .strip_prefix("Bearer ")
            .ok_or(Error::Unauthorized)?;
        let digest: [u8; 32] = Sha256::digest(value.as_bytes()).into();
        if !(32..=256).contains(&value.len()) || !bool::from(self.token.ct_eq(&digest)) {
            return Err(Error::Unauthorized);
        }
        if single(headers, "x-lmm-protocol")? != "1" {
            return Err(Error::Invalid);
        }
        Ok(())
    }
}

fn identity_error(error: identity::IdentityError) -> Error {
    match error {
        identity::IdentityError::Unauthorized => Error::Unauthorized,
        identity::IdentityError::Forbidden => Error::Forbidden,
        identity::IdentityError::Invalid => Error::Invalid,
        _ => Error::Unavailable,
    }
}

async fn administrator<'a>(
    service: &'a Service,
    secret: &str,
) -> Result<(Transaction<'a, Postgres>, identity::Principal)> {
    let mut tx = service.pool.begin().await.map_err(|_| Error::Unavailable)?;
    sqlx::query("SET LOCAL statement_timeout='3s'")
        .execute(&mut *tx)
        .await
        .map_err(|_| Error::Unavailable)?;
    sqlx::query("SET LOCAL lock_timeout='500ms'")
        .execute(&mut *tx)
        .await
        .map_err(|_| Error::Unavailable)?;
    let actor = identity::authorize_in_transaction(&mut tx, secret)
        .await
        .map_err(identity_error)?;
    if actor.credential_kind != CredentialKind::Session
        || actor.owner.kind != crate::accounts::AccountKind::Personal
        || actor.owner.id != actor.user_id
    {
        return Err(Error::Forbidden);
    }
    let (role, version): (String, i64) = sqlx::query_as(
        "SELECT platform_role,auth_version FROM core_identity.users WHERE id=$1 FOR SHARE",
    )
    .bind(actor.user_id)
    .fetch_one(&mut *tx)
    .await
    .map_err(|_| Error::Unavailable)?;
    if role != "superadmin" {
        return Err(Error::Forbidden);
    }
    let loaded = match p::load_complete_policy(&mut tx, actor.user_id).await {
        Ok(policy) => Some(policy),
        Err(p::PolicyLoadError::Missing)
            if !p::has_privileged_import_facts(&mut tx, actor.user_id)
                .await
                .map_err(|_| Error::Unavailable)? =>
        {
            None
        }
        Err(p::PolicyLoadError::Storage) => return Err(Error::Unavailable),
        Err(_) => return Err(Error::Forbidden),
    };
    if let Some(policy) = loaded {
        let allowed = p::decide(
            &p::Actor {
                user_id: actor.user_id,
                personal_account_id: actor.owner_account_id,
                role: p::PlatformRole::Root,
                credential: p::Credential::Session,
                active: true,
                account_active: true,
                auth_version: version,
                credential_auth_version: actor.credential_generation(),
                developer: p::DeveloperState::Granted,
            },
            &policy.as_policy(),
            p::Action::RootSettings,
            p::Scope::Platform(p::PlatformResource::Settings),
            None,
        )
        .allowed();
        if !allowed {
            return Err(Error::Forbidden);
        }
    }
    Ok((tx, actor))
}

fn snapshot(policy: &Policy, configured: bool) -> serde_json::Value {
    serde_json::json!({
        "revision": policy.revision,
        "enabled": policy.enabled,
        "sms_enabled": policy.sms_enabled,
        "email_enabled": policy.email_enabled,
        "price_multiplier": policy.price_multiplier,
        "api_key_configured": configured,
        "pending_work": false,
        "purchases_available": false,
    })
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Empty {}

async fn handle(
    State(state): State<StateData>,
    RoutePath(action): RoutePath<String>,
    headers: HeaderMap,
    body: Bytes,
) -> Response {
    let result = async {
        state.check(&headers)?;
        let _slot = state.slots.try_acquire().map_err(|_| Error::Unavailable)?;
        if action == "ready" {
            let _: Empty = serde_json::from_slice(&body).map_err(|_| Error::Invalid)?;
            state.service.policy.read().await?;
            return Ok(serde_json::json!({
                "protocol": 1, "service": "hero-sms", "ready": true,
                "features": ["settings.read.v1", "settings.write.v1", "connection-test.v1"],
                "purchases_available": false,
            }));
        }
        let secret = single(&headers, "x-lmm-user-credential")?;
        if !matches!(action.as_str(), "settings" | "save-policy" | "test") {
            // No supplier request or wallet mutation is attempted for unavailable
            // purchase, history, cancellation or reconciliation capabilities.
            identity::IdentityStore::from_pool(state.service.pool.clone())
                .authorize(secret)
                .await
                .map_err(identity_error)?;
            let policy = state.service.policy.read().await?;
            return Err(if policy.enabled {
                Error::PurchasesUnavailable
            } else {
                Error::Disabled
            });
        }
        let (tx, actor) = administrator(&state.service, secret).await?;
        let result = match action.as_str() {
            "settings" => {
                let _: Empty = serde_json::from_slice(&body).map_err(|_| Error::Invalid)?;
                let policy = state.service.policy.read().await?;
                Ok(snapshot(&policy, state.service.credential_configured()?))
            }
            "save-policy" => {
                let request: SavePolicy =
                    serde_json::from_slice(&body).map_err(|_| Error::Invalid)?;
                let configured = state.service.credential_configured()?;
                if request.enabled && !configured {
                    return Err(Error::CredentialMissing);
                }
                let policy = state.service.policy.save(request, actor.user_id).await?;
                Ok(snapshot(&policy, configured))
            }
            "test" => {
                let _: Empty = serde_json::from_slice(&body).map_err(|_| Error::Invalid)?;
                // Release authority locks before an external read. No financial
                // mutation is performed by the connection test.
                drop(tx);
                state.service.test_connection().await?;
                return Ok(serde_json::json!({"ok": true}));
            }
            _ => unreachable!(),
        };
        drop(tx);
        result
    }
    .await;
    let (status, value) = match result {
        Ok(value) => (StatusCode::OK, value),
        Err(error) => {
            let (status, code) = match error {
                Error::Invalid => (StatusCode::BAD_REQUEST, "INVALID_REQUEST"),
                Error::Unauthorized => (StatusCode::UNAUTHORIZED, "UNAUTHORIZED"),
                Error::Forbidden => (StatusCode::FORBIDDEN, "FORBIDDEN"),
                Error::StateConflict => (StatusCode::CONFLICT, "STATE_CONFLICT"),
                Error::CredentialMissing => (StatusCode::CONFLICT, "CREDENTIAL_NOT_CONFIGURED"),
                Error::CredentialRejected => {
                    (StatusCode::BAD_GATEWAY, "PROVIDER_CREDENTIAL_REJECTED")
                }
                Error::CommitUnknown => (StatusCode::SERVICE_UNAVAILABLE, "POLICY_WRITE_UNKNOWN"),
                Error::Disabled => (StatusCode::SERVICE_UNAVAILABLE, "SERVICE_DISABLED"),
                Error::PurchasesUnavailable => {
                    (StatusCode::SERVICE_UNAVAILABLE, "PURCHASES_UNAVAILABLE")
                }
                Error::Unavailable => (StatusCode::SERVICE_UNAVAILABLE, "HERO_SMS_UNAVAILABLE"),
            };
            (status, serde_json::json!({"error": code}))
        }
    };
    (status, [(header::CACHE_CONTROL, "no-store")], Json(value)).into_response()
}

pub fn router(service: Service, token: &[u8]) -> Result<Router> {
    if !(32..=256).contains(&token.len()) || !token.iter().all(u8::is_ascii_graphic) {
        return Err(Error::Invalid);
    }
    Ok(Router::new()
        .route("/internal/hero-sms/v1/{action}", post(handle))
        .layer(DefaultBodyLimit::max(64 * 1024))
        .with_state(StateData {
            service,
            token: Sha256::digest(token).into(),
            slots: Arc::new(Semaphore::new(2)),
        }))
}

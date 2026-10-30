//! Administrator account suspension and credential recovery for ordinary users.
//! These operations use the existing identity generation and audit transaction.
use super::{AdminUsersStore, ProfileError};
use crate::identity::{self, CredentialKind};
use axum::{
    Json, Router,
    extract::{DefaultBodyLimit, Path, State, rejection::JsonRejection},
    http::{HeaderMap, header},
    routing::{get, post},
};
use serde::{Deserialize, Serialize};
use sqlx::{PgConnection, Row};

const CONTRACT: &str = "core.admin-user-security.v1";
type Result<T> = std::result::Result<T, ProfileError>;

#[derive(Clone, Copy, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum SecurityAction {
    Disable,
    Enable,
    ResetMfa,
    RevokeCredentials,
}
impl SecurityAction {
    fn audit_name(self) -> &'static str {
        match self {
            Self::Disable => "admin.user.disable",
            Self::Enable => "admin.user.enable",
            Self::ResetMfa => "admin.user.mfa.reset",
            Self::RevokeCredentials => "admin.user.credentials.revoke",
        }
    }
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SecurityRequest {
    pub expected_auth_version: String,
    pub action: SecurityAction,
    pub reason: String,
}
impl SecurityRequest {
    fn validate(&self) -> Result<i64> {
        let value = &self.expected_auth_version;
        if value.is_empty()
            || value.starts_with('0')
            || value.len() > 19
            || !value.bytes().all(|b| b.is_ascii_digit())
            || self.reason.trim().is_empty()
            || self.reason.len() > 1000
            || self.reason.chars().any(char::is_control)
        {
            return Err(ProfileError::Invalid);
        }
        let version = value.parse::<i64>().map_err(|_| ProfileError::Invalid)?;
        if version <= 0 || version == i64::MAX {
            return Err(ProfileError::Invalid);
        }
        Ok(version)
    }
}

#[derive(Debug, Serialize)]
pub struct SecuritySnapshot {
    pub contract: &'static str,
    pub user_id: String,
    pub auth_version: String,
    pub active: bool,
    pub account_active: bool,
    pub mfa_enabled: bool,
}
#[derive(Serialize)]
pub struct SecurityUpdate {
    pub changed: bool,
    pub user: SecuritySnapshot,
}

async fn target(c: &mut PgConnection, id: i64, lock: bool) -> Result<SecuritySnapshot> {
    if id <= 0 {
        return Err(ProfileError::Invalid);
    }
    let suffix = if lock { " FOR UPDATE OF u" } else { "" };
    let row = sqlx::query(&format!(
        "SELECT u.auth_version,u.active,u.platform_role,a.active AS account_active,
         EXISTS(SELECT 1 FROM core_identity.mfa_factors f WHERE f.user_id=u.id AND f.enabled) AS mfa_enabled,
         EXISTS(SELECT 1 FROM core_identity.privileged_import_facts p WHERE p.user_id=u.id) AS imported_admin
         FROM core_identity.users u JOIN core_identity.accounts a ON a.id=u.personal_account_id
         WHERE u.id=$1{suffix}"
    ))
    .bind(id)
    .fetch_optional(&mut *c)
    .await?
    .ok_or(ProfileError::NotFound)?;
    // Imported privileged activation stays with the offline approval executor.
    // Admins cannot recover another administrator through a generic user action.
    if row.try_get::<String, _>("platform_role")? != "user"
        || row.try_get::<bool, _>("imported_admin")?
    {
        return Err(ProfileError::Forbidden);
    }
    Ok(SecuritySnapshot {
        contract: CONTRACT,
        user_id: id.to_string(),
        auth_version: row.try_get::<i64, _>("auth_version")?.to_string(),
        active: row.try_get("active")?,
        account_active: row.try_get("account_active")?,
        mfa_enabled: row.try_get("mfa_enabled")?,
    })
}

impl AdminUsersStore {
    pub async fn user_security(&self, secret: &str, id: i64) -> Result<SecuritySnapshot> {
        let mut tx = self.begin().await?;
        Self::authorize(&mut tx, secret).await?;
        let snapshot = target(&mut tx, id, false).await?;
        tx.commit().await?;
        Ok(snapshot)
    }

    pub async fn update_user_security(
        &self,
        secret: &str,
        id: i64,
        input: &SecurityRequest,
    ) -> Result<SecurityUpdate> {
        let expected = input.validate()?;
        let mut tx = self.identity.pool.begin().await?;
        sqlx::query("SET LOCAL statement_timeout='5s'")
            .execute(&mut *tx)
            .await?;
        sqlx::query("SET LOCAL lock_timeout='500ms'")
            .execute(&mut *tx)
            .await?;
        let actor = identity::authorize_in_transaction(&mut tx, secret).await?;
        if actor.credential_kind != CredentialKind::Session || actor.user_id == id {
            return Err(ProfileError::Forbidden);
        }
        // Includes the complete imported-admin permission policy check while
        // the current Session, user, account and factor locks remain held.
        Self::authorize(&mut tx, secret).await?;
        let before = target(&mut tx, id, true).await?;
        if before.auth_version != input.expected_auth_version {
            return Err(ProfileError::Conflict);
        }
        if input.action == SecurityAction::Enable && !before.account_active {
            return Err(ProfileError::Forbidden);
        }
        let changed = match input.action {
            SecurityAction::Disable => before.active,
            SecurityAction::Enable => !before.active,
            SecurityAction::ResetMfa | SecurityAction::RevokeCredentials => true,
        };
        if changed {
            if input.action == SecurityAction::ResetMfa {
                sqlx::query("UPDATE core_identity.mfa_factors SET enabled=FALSE,revision=revision+1,key_id=NULL,encrypted_secret=NULL,last_used_step=NULL,failed_attempts=0,locked_until=NULL WHERE user_id=$1")
                    .bind(id).execute(&mut *tx).await?;
                sqlx::query("UPDATE core_identity.mfa_backup_codes SET consumed_at=COALESCE(consumed_at,clock_timestamp()) WHERE user_id=$1")
                    .bind(id).execute(&mut *tx).await?;
                sqlx::query("UPDATE core_identity.mfa_enrollments SET revoked_at=COALESCE(revoked_at,clock_timestamp()) WHERE user_id=$1 AND consumed_at IS NULL")
                    .bind(id).execute(&mut *tx).await?;
                sqlx::query("UPDATE core_identity.mfa_challenges SET consumed_at=COALESCE(consumed_at,clock_timestamp()) WHERE user_id=$1")
                    .bind(id).execute(&mut *tx).await?;
                sqlx::query("UPDATE core_identity.user_profiles SET second_factor_required=FALSE WHERE user_id=$1")
                    .bind(id).execute(&mut *tx).await?;
            }
            let active = match input.action {
                SecurityAction::Disable => false,
                SecurityAction::Enable => true,
                _ => before.active,
            };
            // The profile trigger may already have advanced the generation.
            // Either way every old Session and API key stays invalid forever.
            sqlx::query("UPDATE core_identity.users SET active=$2,auth_version=GREATEST(auth_version,$3) WHERE id=$1")
                .bind(id).bind(active).bind(expected + 1).execute(&mut *tx).await?;
            sqlx::query("UPDATE core_identity.credentials SET revoked_at=clock_timestamp() WHERE user_id=$1 AND revoked_at IS NULL")
                .bind(id).execute(&mut *tx).await?;
            sqlx::query("UPDATE core_identity.web_sessions SET revoked_at=clock_timestamp() WHERE user_id=$1 AND revoked_at IS NULL")
                .bind(id).execute(&mut *tx).await?;
            identity::audit(
                &mut tx,
                &actor,
                input.action.audit_name(),
                serde_json::json!({
                    "user_id": id, "previous_auth_version": input.expected_auth_version,
                    "reason": input.reason.trim(), "active": active
                }),
            )
            .await?;
        }
        let user = target(&mut tx, id, false).await?;
        tx.commit().await?;
        Ok(SecurityUpdate { changed, user })
    }
}

pub(super) fn bearer(headers: &HeaderMap) -> Result<&str> {
    let mut values = headers.get_all(header::AUTHORIZATION).iter();
    let raw = values
        .next()
        .ok_or(ProfileError::Unauthorized)?
        .to_str()
        .map_err(|_| ProfileError::Unauthorized)?;
    if values.next().is_some() {
        return Err(ProfileError::Unauthorized);
    }
    let (scheme, secret) = raw.split_once(' ').ok_or(ProfileError::Unauthorized)?;
    if !scheme.eq_ignore_ascii_case("Bearer") {
        return Err(ProfileError::Unauthorized);
    }
    Ok(secret)
}
pub(super) fn routes() -> Router<AdminUsersStore> {
    Router::new()
        .route("/core/v1/admin/user-security/{id}", get(read))
        .route("/core/v1/admin/user-security/{id}/actions", post(update))
        .layer(DefaultBodyLimit::max(4096))
}
async fn read(
    State(store): State<AdminUsersStore>,
    headers: HeaderMap,
    Path(id): Path<i64>,
) -> Result<Json<SecuritySnapshot>> {
    Ok(Json(store.user_security(bearer(&headers)?, id).await?))
}
async fn update(
    State(store): State<AdminUsersStore>,
    headers: HeaderMap,
    Path(id): Path<i64>,
    input: std::result::Result<Json<SecurityRequest>, JsonRejection>,
) -> Result<Json<SecurityUpdate>> {
    let secret = bearer(&headers)?;
    let Json(input) = input.map_err(|_| ProfileError::Invalid)?;
    Ok(Json(store.update_user_security(secret, id, &input).await?))
}

#[cfg(test)]
#[path = "security_tests.rs"]
mod tests;

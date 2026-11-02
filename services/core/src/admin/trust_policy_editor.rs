//! Offline owner ceremony. No HTTP endpoint can enroll or recover an editor.
use lmm_core::{config, options_http::trust_policy::editor::Owner};
use serde::Deserialize;
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use sqlx::PgPool;
use std::io::{self, Read};
use webauthn_rs::prelude::RegisterPublicKeyCredential;

#[derive(Deserialize)]
#[serde(tag = "operation", rename_all = "snake_case", deny_unknown_fields)]
pub enum Request {
    BeginRegistration {
        editor: String,
        actor_user_id: i64,
        expected_version: i64,
    },
    ReviewRegistration {
        enrollment: String,
        response: RegisterPublicKeyCredential,
    },
    ApproveRegistration {
        enrollment: String,
        fingerprint: String,
    },
    Revoke {
        editor: String,
        expected_version: i64,
    },
    RegistrationStatus {
        enrollment: String,
    },
    EditorStatus {
        editor: String,
    },
    ProvisionSigningKey {
        expected_current_sha256: Option<String>,
    },
    SigningKeyStatus {},
}

pub fn read(input: impl Read) -> io::Result<Request> {
    let mut bytes = Vec::new();
    input.take(100_001).read_to_end(&mut bytes)?;
    if bytes.len() > 100_000 {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "policy editor input exceeds 100000 bytes",
        ));
    }
    serde_json::from_slice(&bytes).map_err(|_| {
        io::Error::new(
            io::ErrorKind::InvalidInput,
            "invalid policy editor owner request",
        )
    })
}

fn failed() -> io::Error {
    io::Error::other("TRUST_POLICY_OWNER_UNCONFIRMED")
}

pub async fn run(pool: &PgPool, request: Request) -> io::Result<Value> {
    lmm_core::core_schema::check(pool)
        .await
        .map_err(|_| failed())?;
    match request {
        Request::ProvisionSigningKey {
            expected_current_sha256,
        } => return key(pool, true, expected_current_sha256).await,
        Request::SigningKeyStatus {} => return key(pool, false, None).await,
        _ => {}
    }
    let owner = Owner::new(pool.clone(), config::trust_policy_editor_config()?);
    let value = match request {
        Request::BeginRegistration {
            editor,
            actor_user_id,
            expected_version,
        } => serde_json::to_value(
            owner
                .begin_registration(&editor, actor_user_id, expected_version)
                .await
                .map_err(|_| failed())?,
        ),
        Request::ReviewRegistration {
            enrollment,
            response,
        } => serde_json::to_value(
            owner
                .review_registration(&enrollment, &response)
                .await
                .map_err(|_| failed())?,
        ),
        Request::ApproveRegistration {
            enrollment,
            fingerprint,
        } => {
            let version = owner
                .approve_registration(&enrollment, &fingerprint)
                .await
                .map_err(|_| failed())?;
            Ok(json!({"version":version}))
        }
        Request::Revoke {
            editor,
            expected_version,
        } => {
            let version = owner
                .revoke(&editor, expected_version)
                .await
                .map_err(|_| failed())?;
            Ok(json!({"version":version}))
        }
        Request::RegistrationStatus { enrollment } => serde_json::to_value(
            owner
                .registration_status(&enrollment)
                .await
                .map_err(|_| failed())?,
        ),
        Request::EditorStatus { editor } => {
            serde_json::to_value(owner.editor_status(&editor).await.map_err(|_| failed())?)
        }
        Request::ProvisionSigningKey { .. } | Request::SigningKeyStatus {} => unreachable!(),
    }
    .map_err(|_| failed())?;
    Ok(json!({"protocol":"core.trust-policy-owner.v1","status":"confirmed","data":value}))
}

async fn key(pool: &PgPool, provision: bool, expected: Option<String>) -> io::Result<Value> {
    if expected.as_ref().is_some_and(|v| {
        v.len() != 64
            || !v
                .bytes()
                .all(|b| b.is_ascii_hexdigit() && !b.is_ascii_uppercase())
    }) {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "invalid expected key fingerprint",
        ));
    }
    let secret = config::trust_policy_signing_key()?;
    let desired = format!("{:x}", Sha256::digest(secret.as_slice()));
    let mut tx = pool.begin().await.map_err(|_| failed())?;
    let owner: bool = sqlx::query_scalar("SELECT relowner=current_user::regrole FROM pg_catalog.pg_class WHERE oid='core_trust_pricing.policy_edit_signing_key'::regclass")
        .fetch_one(&mut *tx).await.map_err(|_| failed())?;
    if !owner {
        return Err(io::Error::other("TRUST_POLICY_OWNER_REQUIRED"));
    }
    sqlx::query("SELECT pg_advisory_xact_lock(741803219)")
        .execute(&mut *tx)
        .await
        .map_err(|_| failed())?;
    let current: Option<String> = sqlx::query_scalar("SELECT encode(sha256(secret),'hex') FROM core_trust_pricing.policy_edit_signing_key WHERE singleton")
        .fetch_optional(&mut *tx).await.map_err(|_| failed())?;
    if !provision || current.as_deref() == Some(desired.as_str()) {
        tx.rollback().await.map_err(|_| failed())?;
        return Ok(
            json!({"protocol":"core.trust-policy-owner.v1","status":"observed",
            "data":{"installed":current.is_some(),"sha256":current,"matches_configured_key":current.as_deref()==Some(desired.as_str())}}),
        );
    }
    if current != expected {
        return Err(io::Error::other("TRUST_POLICY_KEY_REVISION_CONFLICT"));
    }
    // These settings apply only to this transaction. The offline owner needs
    // explicit SET grants; missing authority rejects before sending key bytes.
    sqlx::raw_sql("SET LOCAL log_parameter_max_length=0; SET LOCAL log_parameter_max_length_on_error=0; SET LOCAL log_error_verbosity=terse;")
        .execute(&mut *tx).await.map_err(|_| io::Error::other("TRUST_POLICY_PRIVATE_PARAMETER_LOGGING_REQUIRED"))?;
    let private: bool = sqlx::query_scalar(
        r#"
        SELECT current_setting('log_parameter_max_length')='0'
          AND current_setting('log_parameter_max_length_on_error')='0'
          AND current_setting('log_error_verbosity')='terse'
          AND coalesce(current_setting('pgaudit.log_parameter',true),'off') IN ('off','false','0')
    "#,
    )
    .fetch_one(&mut *tx)
    .await
    .map_err(|_| failed())?;
    if !private {
        return Err(io::Error::other(
            "TRUST_POLICY_PRIVATE_PARAMETER_LOGGING_REQUIRED",
        ));
    }
    sqlx::query("INSERT INTO core_trust_pricing.policy_edit_signing_key(singleton,secret) VALUES(true,$1) ON CONFLICT(singleton) DO UPDATE SET secret=EXCLUDED.secret")
        .bind(secret.as_slice()).execute(&mut *tx).await.map_err(|_| failed())?;
    tx.commit().await.map_err(|_| failed())?;
    Ok(
        json!({"protocol":"core.trust-policy-owner.v1","status":"confirmed","data":{"sha256":desired}}),
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn owner_requests_are_bounded_explicit_and_reject_extra_fields() {
        assert!(read(br#"{"operation":"signing_key_status"}"#.as_slice()).is_ok());
        assert!(
            read(
                br#"{"operation":"approve_registration","enrollment":"e","fingerprint":"f"}"#
                    .as_slice()
            )
            .is_ok()
        );
        assert!(read(br#"{"operation":"approve_registration","enrollment":"e","fingerprint":"f","admin":true}"#.as_slice()).is_err());
        assert!(
            read(
                br#"{"operation":"provision_signing_key","expected_current_sha256":null}"#
                    .as_slice()
            )
            .is_ok()
        );
        assert!(
            read(br#"{"operation":"signing_key_status","secret":"never-accepted"}"#.as_slice())
                .is_err()
        );
        assert!(read(vec![b' '; 100_001].as_slice()).is_err());
    }
}

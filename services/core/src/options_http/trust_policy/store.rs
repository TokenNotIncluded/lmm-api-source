//! Explicitly configured preview/resolve service and transaction-bound signer.
use super::{ConsolePolicy, PolicyPreview, TrustPolicy};
use crate::identity::{self, CredentialKind, IdentityError};
use hmac::{Hmac, Mac};
use rand::{RngCore, rngs::OsRng};
use serde::Serialize;
use sha2::{Digest, Sha256};
use sqlx::{PgConnection, PgPool, Row, types::Json};

#[derive(Debug)]
pub enum Error {
    Auth(IdentityError),
    Invalid,
    Conflict,
    Limited,
    Storage,
    CommitUnknown,
}
impl From<sqlx::Error> for Error {
    fn from(error: sqlx::Error) -> Self {
        match error.as_database_error().and_then(|e| e.code()).as_deref() {
            Some("40001" | "40P01") => Self::Conflict,
            Some("22023" | "23505") => Self::Invalid,
            Some("54000") => Self::Limited,
            Some("42501") => Self::Auth(IdentityError::Forbidden),
            _ => Self::Storage,
        }
    }
}

#[derive(Serialize)]
pub struct CreatedPreview {
    /// Bearer-like one-time secret. Never log or store the plaintext token.
    pub token: String,
    pub preview: PolicyPreview,
}

pub(super) use crate::hex_codec::encode as hex;
pub(super) fn decode_hex(value: &str) -> Result<[u8; 32], Error> {
    if value.len() != 64
        || !value
            .bytes()
            .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
    {
        return Err(Error::Invalid);
    }
    let mut result = [0_u8; 32];
    zhex::decode(value.as_bytes(), &mut result).map_err(|_| Error::Invalid)?;
    Ok(result)
}

/// Explicit process-only secret. No default, Debug or serialization implementation.
pub struct SigningKey(zeroize::Zeroizing<[u8; 32]>);
impl SigningKey {
    pub fn new(bytes: [u8; 32]) -> Self {
        Self(zeroize::Zeroizing::new(bytes))
    }
}
#[derive(Clone)]
pub struct Service {
    pub(super) pool: PgPool,
    pub(super) config: super::editor::EditorConfig,
    key: std::sync::Arc<SigningKey>,
}
impl Service {
    pub fn new(pool: PgPool, config: super::editor::EditorConfig, key: SigningKey) -> Self {
        Self {
            pool,
            config,
            key: std::sync::Arc::new(key),
        }
    }
    /// Startup-only, session-independent key agreement check. The caller must
    /// complete this before mounting enabled trust routes; errors fail startup.
    pub async fn check_ready(&self) -> Result<(), Error> {
        let mut tx = self.pool.begin().await?;
        sqlx::query("SET TRANSACTION READ ONLY")
            .execute(&mut *tx)
            .await?;
        let proof = self.proof(&mut tx, "ready", serde_json::json!({})).await?;
        let ready: bool = sqlx::query_scalar("SELECT core_trust_pricing.policy_editor_ready($1)")
            .bind(proof)
            .fetch_one(&mut *tx)
            .await?;
        if !ready {
            return Err(Error::Storage);
        }
        tx.commit().await.map_err(|_| Error::Storage)?;
        Ok(())
    }

    pub(super) async fn proof(
        &self,
        c: &mut PgConnection,
        action: &str,
        payload: serde_json::Value,
    ) -> Result<Vec<u8>, Error> {
        let binding: Vec<u8> =
            sqlx::query_scalar("SELECT core_trust_pricing.policy_edit_binding($1,$2)")
                .bind(action)
                .bind(Json(payload))
                .fetch_one(c)
                .await?;
        let mut signer =
            Hmac::<Sha256>::new_from_slice(self.key.0.as_ref()).map_err(|_| Error::Storage)?;
        signer.update(&binding);
        Ok(signer.finalize().into_bytes().to_vec())
    }
}

pub(super) async fn authorize(
    c: &mut PgConnection,
    secret: &str,
) -> Result<identity::Principal, Error> {
    let actor = identity::authorize_in_transaction(c, secret)
        .await
        .map_err(Error::Auth)?;
    if actor.credential_kind != CredentialKind::Session || actor.platform_level != 6 {
        return Err(Error::Auth(IdentityError::Forbidden));
    }
    Ok(actor)
}

impl Service {
    pub async fn preview(&self, secret: &str, raw: &str) -> Result<CreatedPreview, Error> {
        let mut tx = self.pool.begin().await?;
        let actor = authorize(&mut tx, secret).await?;
        sqlx::query("SELECT pg_advisory_xact_lock_shared(741803219)")
            .execute(&mut *tx)
            .await?;
        let row =
            sqlx::query("SELECT revision,spec FROM core_trust_pricing.policies WHERE singleton")
                .fetch_one(&mut *tx)
                .await?;
        let revision: i64 = row.try_get("revision")?;
        // Preserve the stored JSON for CAS: valid existing ratios can be unreduced
        // (for example 94/100), while TrustPolicy canonicalizes them to 47/50.
        let Json(before): Json<serde_json::Value> = row.try_get("spec")?;
        let current: TrustPolicy =
            serde_json::from_value(before.clone()).map_err(|_| Error::Invalid)?;
        let previous: Option<Json<serde_json::Value>> =
            sqlx::query_scalar("SELECT core_trust_pricing.read_policy_edit_description($1)")
                .bind(revision)
                .fetch_one(&mut *tx)
                .await?;
        let previous_raw = previous.as_ref().map(|value| value.0.to_string());
        let preview = PolicyPreview::build(
            actor.user_id,
            actor.credential_id,
            revision,
            current,
            previous_raw.as_deref(),
            raw,
        )
        .map_err(|_| Error::Invalid)?;
        let mut token = [0_u8; 32];
        OsRng
            .try_fill_bytes(&mut token)
            .map_err(|_| Error::Storage)?;
        let token_digest = Sha256::digest(token);
        let transition_digest = decode_hex(&preview.digest)?;
        let after = serde_json::to_value(&preview.proposed).map_err(|_| Error::Invalid)?;
        let before_console = preview
            .current_console
            .as_ref()
            .map(serde_json::to_value)
            .transpose()
            .map_err(|_| Error::Invalid)?;
        let after_console =
            serde_json::to_value(&preview.proposed_console).map_err(|_| Error::Invalid)?;
        let signature = self
            .proof(
                &mut tx,
                "preview",
                serde_json::json!([
                    hex(&token_digest),
                    actor.user_id,
                    actor.credential_id,
                    revision,
                    &before,
                    &after,
                    &before_console,
                    &after_console,
                    hex(&transition_digest)
                ]),
            )
            .await?;
        sqlx::query(
            "SELECT core_trust_pricing.authorized_policy_preview($1,$2,$3,$4,$5,$6,$7,$8,$9,$10)",
        )
        .bind(token_digest.as_slice())
        .bind(actor.user_id)
        .bind(actor.credential_id)
        .bind(revision)
        .bind(Json(&before))
        .bind(Json(&after))
        .bind(before_console.as_ref().map(Json))
        .bind(Json(&after_console))
        .bind(transition_digest.as_slice())
        .bind(signature)
        .execute(&mut *tx)
        .await?;
        tx.commit().await.map_err(|_| Error::CommitUnknown)?;
        Ok(CreatedPreview {
            token: hex(&token),
            preview,
        })
    }
}

#[derive(Debug, Serialize)]
pub struct Resolution {
    pub state: String,
    pub revision: Option<i64>,
}

impl Service {
    pub async fn resolve(&self, secret: &str, token: &str) -> Result<Resolution, Error> {
        let token_digest = Sha256::digest(decode_hex(token)?);
        let mut tx = self.pool.begin().await?;
        let actor = authorize(&mut tx, secret).await?;
        let signature = self
            .proof(
                &mut tx,
                "resolve",
                serde_json::json!([hex(&token_digest), actor.user_id, actor.credential_id]),
            )
            .await?;
        let row = sqlx::query(
            "SELECT state,revision FROM core_trust_pricing.authorized_policy_resolve($1,$2,$3,$4)",
        )
        .bind(token_digest.as_slice())
        .bind(actor.user_id)
        .bind(actor.credential_id)
        .bind(signature)
        .fetch_one(&mut *tx)
        .await?;
        let state: String = row.try_get("state")?;
        let revision: Option<i64> = row.try_get("revision")?;
        if !matches!(
            (state.as_str(), revision),
            ("confirmed", Some(2..)) | ("unknown" | "pending" | "expired", None)
        ) {
            return Err(Error::Storage);
        }
        tx.commit().await.map_err(|_| Error::CommitUnknown)?;
        Ok(Resolution { state, revision })
    }
}

/// Numeric policy retains the stored JSON, including unreduced rational values.
/// It is validated as TrustPolicy before being returned, never synthesized from
/// console defaults. Descriptions absent from the historical audit remain null.
#[derive(Serialize)]
pub struct PolicyRead {
    pub revision: i64,
    pub policy: serde_json::Value,
    pub console: Option<ConsolePolicy>,
    pub editors: Vec<RegisteredEditor>,
}
#[derive(Serialize)]
pub struct RegisteredEditor {
    pub editor_id: String,
    pub version: i64,
    pub status: EditorAvailability,
}
#[derive(Serialize)]
#[serde(rename_all = "snake_case")]
pub enum EditorAvailability {
    Active,
    Revoked,
    ConfigurationMismatch,
}

impl Service {
    pub async fn read(&self, session: &str) -> Result<PolicyRead, Error> {
        let mut tx = self.pool.begin().await?;
        let actor = authorize(&mut tx, session).await?;
        // Same owner/confirmation lock gives one consistent policy, description
        // and editor-registry view without changing application state.
        sqlx::query("SELECT pg_advisory_xact_lock_shared(741803219)")
            .execute(&mut *tx)
            .await?;
        let row =
            sqlx::query("SELECT revision,spec FROM core_trust_pricing.policies WHERE singleton")
                .fetch_one(&mut *tx)
                .await?;
        let revision: i64 = row.try_get("revision")?;
        let Json(policy): Json<serde_json::Value> = row.try_get("spec")?;
        let authority: TrustPolicy =
            serde_json::from_value(policy.clone()).map_err(|_| Error::Storage)?;
        if revision <= 0 || authority.validate().is_err() {
            return Err(Error::Storage);
        }
        let description: Option<Json<serde_json::Value>> =
            sqlx::query_scalar("SELECT core_trust_pricing.read_policy_edit_description($1)")
                .bind(revision)
                .fetch_one(&mut *tx)
                .await?;
        let console = match description {
            None => None,
            Some(Json(value)) => {
                let (console, described) =
                    ConsolePolicy::parse(&value.to_string()).map_err(|_| Error::Storage)?;
                if described != authority {
                    return Err(Error::Storage);
                }
                Some(console)
            }
        };
        let rows = sqlx::query("SELECT * FROM core_trust_pricing.read_policy_editors($1)")
            .bind(actor.user_id)
            .fetch_all(&mut *tx)
            .await?;
        let mut editors = Vec::with_capacity(rows.len());
        for row in rows {
            let revoked: bool = row.try_get("revoked")?;
            let rp_id: String = row.try_get("rp_id")?;
            let origin: String = row.try_get("origin")?;
            let status = if revoked {
                EditorAvailability::Revoked
            } else if self.config.matches(&rp_id, &origin).is_err() {
                EditorAvailability::ConfigurationMismatch
            } else {
                EditorAvailability::Active
            };
            editors.push(RegisteredEditor {
                editor_id: row.try_get("editor_id")?,
                version: row.try_get("version")?,
                status,
            });
        }
        tx.commit().await.map_err(|_| Error::Storage)?;
        Ok(PolicyRead {
            revision,
            policy,
            console,
            editors,
        })
    }
}

//! An independently enrolled passkey gates policy editing. Ordinary identity
//! credentials are not enrollment authority, even when their role says root.
use super::store::{authorize, decode_hex, hex};
use super::{Error, Service};
use rand::{RngCore, rngs::OsRng};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use sqlx::{PgConnection, PgPool, Row, types::Json};
use webauthn_rs::prelude::{
    Passkey, PasskeyAuthentication, PasskeyRegistration, PublicKeyCredential,
    RegisterPublicKeyCredential, Url, Uuid, Webauthn, WebauthnBuilder,
};

#[derive(Clone)]
pub struct EditorConfig {
    webauthn: Webauthn,
    rp_id: String,
    origin: String,
}
impl EditorConfig {
    pub fn new(rp_id: &str, origin: &str) -> Result<Self, Error> {
        let url = Url::parse(origin).map_err(|_| Error::Invalid)?;
        if url.host_str() != Some(rp_id)
            || !url.username().is_empty()
            || url.password().is_some()
            || url.query().is_some()
            || url.fragment().is_some()
            || url.path() != "/"
            || (url.scheme() != "https" && !(url.scheme() == "http" && rp_id == "localhost"))
            || url.origin().ascii_serialization() != origin.trim_end_matches('/')
        {
            return Err(Error::Invalid);
        }
        let webauthn = WebauthnBuilder::new(rp_id, &url)
            .map_err(|_| Error::Invalid)?
            .allow_subdomains(false)
            .allow_any_port(false)
            .rp_name("LMM policy editor")
            .timeout(std::time::Duration::from_secs(300))
            .build()
            .map_err(|_| Error::Invalid)?;
        Ok(Self {
            webauthn,
            rp_id: rp_id.into(),
            origin: url.origin().ascii_serialization(),
        })
    }
    pub(super) fn matches(&self, rp_id: &str, origin: &str) -> Result<(), Error> {
        if self.rp_id != rp_id || self.origin != origin {
            return Err(Error::Invalid);
        }
        Ok(())
    }
}
fn random_id() -> Result<[u8; 32], Error> {
    let mut id = [0; 32];
    OsRng.try_fill_bytes(&mut id).map_err(|_| Error::Storage)?;
    Ok(id)
}
fn encode<T: Serialize>(v: &T) -> Result<Value, Error> {
    serde_json::to_value(v).map_err(|_| Error::Storage)
}
fn decode<T: serde::de::DeserializeOwned>(v: Value) -> Result<T, Error> {
    serde_json::from_value(v).map_err(|_| Error::Invalid)
}
#[derive(Deserialize)]
struct Editor {
    editor_id: String,
    version: i64,
    passkey: Passkey,
    rp_id: String,
    origin: String,
}
#[derive(Serialize)]
pub struct Challenge {
    pub challenge: String,
    pub options: Value,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Confirmation {
    pub token: String,
    pub digest: String,
    pub challenge: String,
    /// The browser must supply a real assertion, never a service-generated flag.
    pub assertion: PublicKeyCredential,
}
impl Service {
    /// Called only after the user reviews the exact preview and clicks confirm.
    pub async fn challenge(
        &self,
        secret: &str,
        token: &str,
        digest: &str,
        editor: &str,
    ) -> Result<Challenge, Error> {
        let token_digest = Sha256::digest(decode_hex(token)?);
        decode_hex(digest)?;
        let mut tx = self.pool.begin().await?;
        let actor = authorize(&mut tx, secret).await?;
        let record: Option<Json<Value>> =
            sqlx::query_scalar("SELECT core_trust_pricing.policy_editor_context($1,$2)")
                .bind(editor)
                .bind(actor.user_id)
                .fetch_one(&mut *tx)
                .await?;
        let editor: Editor = decode(record.ok_or(Error::Invalid)?.0)?;
        self.config.matches(&editor.rp_id, &editor.origin)?;
        let (options, state) = self
            .config
            .webauthn
            .start_passkey_authentication(&[editor.passkey])
            .map_err(|_| Error::Invalid)?;
        let challenge = hex(&random_id()?);
        let payload = json!({"token":hex(&token_digest),"actor":actor.user_id,"session":actor.credential_id,
            "editor":editor.editor_id,"version":editor.version,"digest":digest,"challenge":challenge,
            "authentication":encode(&state)?});
        let proof = self.proof(&mut tx, "challenge", payload.clone()).await?;
        sqlx::query("SELECT core_trust_pricing.authorized_policy_challenge($1,$2)")
            .bind(Json(payload))
            .bind(proof)
            .execute(&mut *tx)
            .await?;
        tx.commit().await.map_err(|_| Error::CommitUnknown)?;
        Ok(Challenge {
            challenge,
            options: encode(&options)?,
        })
    }
    pub async fn confirm(&self, secret: &str, input: &Confirmation) -> Result<i64, Error> {
        let token_digest = Sha256::digest(decode_hex(&input.token)?);
        let challenge = decode_hex(&input.challenge)?;
        decode_hex(&input.digest)?;
        let mut tx = self.pool.begin().await?;
        let actor = authorize(&mut tx, secret).await?;
        let Json(context): Json<Value> =
            sqlx::query_scalar("SELECT core_trust_pricing.policy_challenge_context($1,$2,$3,$4)")
                .bind(challenge.as_slice())
                .bind(token_digest.as_slice())
                .bind(actor.user_id)
                .bind(actor.credential_id)
                .fetch_one(&mut *tx)
                .await?;
        if context["digest"].as_str() != Some(&input.digest) {
            return Err(Error::Invalid);
        }
        let mut editor: Editor = decode(context["editor"].clone())?;
        self.config.matches(&editor.rp_id, &editor.origin)?;
        let state: PasskeyAuthentication = decode(context["authentication"].clone())?;
        let result = self
            .config
            .webauthn
            .finish_passkey_authentication(&input.assertion, &state)
            .map_err(|_| Error::Invalid)?;
        if !result.user_verified() || editor.passkey.update_credential(&result).is_none() {
            return Err(Error::Invalid);
        }
        // Challenge state is immutable and signed. The commit also checks the
        // latest persisted counter, not merely the counter captured at start.
        let assertion_digest =
            Sha256::digest(serde_json::to_vec(&input.assertion).map_err(|_| Error::Invalid)?);
        let payload = json!({"token":hex(&token_digest),"actor":actor.user_id,"session":actor.credential_id,
            "digest":input.digest,"challenge":input.challenge,"editor":editor.editor_id,"version":editor.version,
            "credential":hex(result.cred_id().as_ref()),"user_verified":result.user_verified(),
            "counter":result.counter(),"backup_state":result.backup_state(),"backup_eligible":result.backup_eligible(),
            "assertion_digest":hex(&assertion_digest)});
        let proof = self
            .proof(&mut tx, "confirm-passkey-v1", payload.clone())
            .await?;
        let revision =
            sqlx::query_scalar("SELECT core_trust_pricing.authorized_policy_confirm($1,$2)")
                .bind(Json(payload))
                .bind(proof)
                .fetch_one(&mut *tx)
                .await?;
        tx.commit().await.map_err(|_| Error::CommitUnknown)?;
        Ok(revision)
    }
}

/// Offline command service, never an HTTP endpoint. The supplied pool must
/// authenticate as the table owner; no ordinary application session is accepted.
pub struct Owner {
    pool: PgPool,
    config: EditorConfig,
}
#[derive(Serialize)]
pub struct Registration {
    pub enrollment: String,
    pub options: Value,
}
#[derive(Serialize)]
pub struct Review {
    pub enrollment: String,
    pub editor_id: String,
    pub actor_user_id: i64,
    pub expected_version: i64,
    pub rp_id: String,
    pub origin: String,
    pub credential_id: String,
    /// Owner must explicitly approve this exact fingerprint out of band.
    pub fingerprint: String,
}
#[derive(Serialize)]
#[serde(tag = "state", rename_all = "snake_case")]
pub enum RegistrationStatus {
    Unknown,
    Pending,
    Reviewed { review: Review },
    Expired,
    Approved { editor_id: String, version: i64 },
}
#[derive(Serialize, sqlx::FromRow)]
pub struct EditorStatus {
    pub editor_id: String,
    pub actor_user_id: i64,
    pub version: i64,
    pub credential_id: String,
    pub rp_id: String,
    pub origin: String,
    pub revoked: bool,
}
impl Owner {
    pub fn new(pool: PgPool, config: EditorConfig) -> Self {
        Self { pool, config }
    }
    async fn begin(&self) -> Result<sqlx::Transaction<'_, sqlx::Postgres>, Error> {
        let mut tx = self.pool.begin().await?;
        let owner: bool=sqlx::query_scalar("SELECT relowner=current_user::regrole FROM pg_catalog.pg_class WHERE oid='core_trust_pricing.policy_editors'::regclass")
            .fetch_one(&mut *tx).await?;
        if !owner {
            return Err(Error::Auth(crate::identity::IdentityError::Forbidden));
        }
        sqlx::query("SELECT pg_advisory_xact_lock(741803219)")
            .execute(&mut *tx)
            .await?;
        Ok(tx)
    }
    /// Recover a lost review/approval response without replaying registration.
    pub async fn registration_status(&self, enrollment: &str) -> Result<RegistrationStatus, Error> {
        let mut tx = self.begin().await?;
        let id = decode_hex(enrollment)?;
        let row=sqlx::query("SELECT *,approved_at IS NOT NULL AS approved,expires_at<=clock_timestamp() AS expired FROM core_trust_pricing.policy_editor_enrollments WHERE enrollment_id=$1")
            .bind(id.as_slice()).fetch_optional(&mut *tx).await?;
        let result = match row {
            None => RegistrationStatus::Unknown,
            Some(row) => {
                let editor_id: String = row.try_get("editor_id")?;
                let expected_version: i64 = row.try_get("expected_version")?;
                let fingerprint: Option<Vec<u8>> = row.try_get("candidate_digest")?;
                if row.try_get::<bool, _>("approved")? {
                    RegistrationStatus::Approved {
                        editor_id,
                        version: expected_version + 1,
                    }
                } else if row.try_get::<bool, _>("expired")? {
                    RegistrationStatus::Expired
                } else if let Some(fingerprint) = fingerprint {
                    let Json(passkey): Json<Passkey> = row.try_get("candidate")?;
                    RegistrationStatus::Reviewed {
                        review: Review {
                            enrollment: enrollment.into(),
                            editor_id,
                            expected_version,
                            actor_user_id: row.try_get("actor_user_id")?,
                            rp_id: row.try_get("rp_id")?,
                            origin: row.try_get("origin")?,
                            credential_id: hex(passkey.cred_id().as_ref()),
                            fingerprint: hex(&fingerprint),
                        },
                    }
                } else {
                    RegistrationStatus::Pending
                }
            }
        };
        tx.commit().await.map_err(|_| Error::CommitUnknown)?;
        Ok(result)
    }
    pub async fn editor_status(&self, editor: &str) -> Result<Option<EditorStatus>, Error> {
        let mut tx = self.begin().await?;
        let result=sqlx::query_as("SELECT editor_id,actor_user_id,version,encode(credential_id,'hex') AS credential_id,rp_id,origin,revoked FROM core_trust_pricing.policy_editors WHERE editor_id=$1")
            .bind(editor).fetch_optional(&mut *tx).await?;
        tx.commit().await.map_err(|_| Error::CommitUnknown)?;
        Ok(result)
    }
    /// expected_version=0 creates a new editor; otherwise prepares a rotation.
    /// Transport the returned options to a browser at the configured origin.
    pub async fn begin_registration(
        &self,
        editor: &str,
        actor: i64,
        expected_version: i64,
    ) -> Result<Registration, Error> {
        if editor.is_empty()
            || editor.len() > 80
            || editor.chars().any(char::is_control)
            || actor <= 0
            || expected_version < 0
            || expected_version == i64::MAX
        {
            return Err(Error::Invalid);
        }
        let mut tx = self.begin().await?;
        check_version(&mut tx, editor, actor, expected_version).await?;
        let mut handle = [0u8; 16];
        OsRng
            .try_fill_bytes(&mut handle)
            .map_err(|_| Error::Storage)?;
        let (options, state) = self
            .config
            .webauthn
            .start_passkey_registration(Uuid::from_bytes(handle), editor, editor, None)
            .map_err(|_| Error::Invalid)?;
        let id = random_id()?;
        // Enrollment challenges are owner-only and expire; old rows are not a
        // reusable registration authority and are safe to discard before review.
        sqlx::query("DELETE FROM core_trust_pricing.policy_editor_enrollments WHERE expires_at<=clock_timestamp() AND approved_at IS NULL")
            .execute(&mut *tx).await?;
        sqlx::query("INSERT INTO core_trust_pricing.policy_editor_enrollments(enrollment_id,editor_id,actor_user_id,expected_version,rp_id,origin,registration,expires_at) VALUES($1,$2,$3,$4,$5,$6,$7,clock_timestamp()+interval '5 minutes')")
            .bind(id.as_slice()).bind(editor).bind(actor).bind(expected_version).bind(&self.config.rp_id)
            .bind(&self.config.origin).bind(Json(encode(&state)?)).execute(&mut *tx).await?;
        tx.commit().await.map_err(|_| Error::CommitUnknown)?;
        Ok(Registration {
            enrollment: hex(&id),
            options: encode(&options)?,
        })
    }
    pub async fn review_registration(
        &self,
        enrollment: &str,
        response: &RegisterPublicKeyCredential,
    ) -> Result<Review, Error> {
        let id = decode_hex(enrollment)?;
        let mut tx = self.begin().await?;
        let row=sqlx::query("SELECT * FROM core_trust_pricing.policy_editor_enrollments WHERE enrollment_id=$1 AND expires_at>clock_timestamp() AND approved_at IS NULL AND candidate IS NULL FOR UPDATE")
            .bind(id.as_slice()).fetch_optional(&mut *tx).await?.ok_or(Error::Invalid)?;
        let rp_id: String = row.try_get("rp_id")?;
        let origin: String = row.try_get("origin")?;
        self.config.matches(&rp_id, &origin)?;
        let Json(state): Json<PasskeyRegistration> = row.try_get("registration")?;
        let passkey = self
            .config
            .webauthn
            .finish_passkey_registration(response, &state)
            .map_err(|_| Error::Invalid)?;
        let editor_id: String = row.try_get("editor_id")?;
        let actor_user_id: i64 = row.try_get("actor_user_id")?;
        let expected_version: i64 = row.try_get("expected_version")?;
        let candidate = encode(&passkey)?;
        let fingerprint = Sha256::digest(
            serde_json::to_vec(&json!([
                "lmm-policy-editor-enrollment-v1",
                enrollment,
                editor_id,
                actor_user_id,
                expected_version,
                rp_id,
                origin,
                candidate
            ]))
            .map_err(|_| Error::Storage)?,
        );
        sqlx::query("UPDATE core_trust_pricing.policy_editor_enrollments SET candidate=$2,candidate_digest=$3 WHERE enrollment_id=$1")
            .bind(id.as_slice()).bind(Json(candidate)).bind(fingerprint.as_slice()).execute(&mut *tx).await?;
        tx.commit().await.map_err(|_| Error::CommitUnknown)?;
        Ok(Review {
            enrollment: enrollment.into(),
            editor_id,
            actor_user_id,
            expected_version,
            rp_id,
            origin,
            credential_id: hex(passkey.cred_id().as_ref()),
            fingerprint: hex(&fingerprint),
        })
    }
    /// Explicit offline approval, including first registration and replacement.
    /// Returns the new version; failures leave the previous editor untouched.
    pub async fn approve_registration(
        &self,
        enrollment: &str,
        fingerprint: &str,
    ) -> Result<i64, Error> {
        let mut tx = self.begin().await?;
        let id = decode_hex(enrollment)?;
        let fingerprint = decode_hex(fingerprint)?;
        let row=sqlx::query("SELECT * FROM core_trust_pricing.policy_editor_enrollments WHERE enrollment_id=$1 AND candidate_digest=$2 AND expires_at>clock_timestamp() AND approved_at IS NULL FOR UPDATE")
            .bind(id.as_slice()).bind(fingerprint.as_slice()).fetch_optional(&mut *tx).await?.ok_or(Error::Invalid)?;
        let editor: String = row.try_get("editor_id")?;
        let actor: i64 = row.try_get("actor_user_id")?;
        let expected: i64 = row.try_get("expected_version")?;
        check_version(&mut tx, &editor, actor, expected).await?;
        let rp: String = row.try_get("rp_id")?;
        let origin: String = row.try_get("origin")?;
        self.config.matches(&rp, &origin)?;
        let Json(passkey): Json<Passkey> = row.try_get("candidate")?;
        let version = expected.checked_add(1).ok_or(Error::Invalid)?;
        sqlx::query("INSERT INTO core_trust_pricing.policy_editors(editor_id,actor_user_id,version,credential_id,passkey,rp_id,origin) VALUES($1,$2,$3,$4,$5,$6,$7) ON CONFLICT(editor_id) DO UPDATE SET version=EXCLUDED.version,credential_id=EXCLUDED.credential_id,passkey=EXCLUDED.passkey,rp_id=EXCLUDED.rp_id,origin=EXCLUDED.origin,revoked=false")
            .bind(&editor).bind(actor).bind(version).bind(passkey.cred_id().as_ref() as &[u8])
            .bind(Json(encode(&passkey)?)).bind(rp).bind(origin).execute(&mut *tx).await?;
        sqlx::query("UPDATE core_trust_pricing.policy_editor_enrollments SET approved_at=clock_timestamp() WHERE enrollment_id=$1")
            .bind(id.as_slice()).execute(&mut *tx).await?;
        event(&mut tx, &editor, version, "approve").await?;
        tx.commit().await.map_err(|_| Error::CommitUnknown)?;
        Ok(version)
    }
    pub async fn revoke(&self, editor: &str, expected_version: i64) -> Result<i64, Error> {
        let version = expected_version.checked_add(1).ok_or(Error::Invalid)?;
        let mut tx = self.begin().await?;
        let changed=sqlx::query("UPDATE core_trust_pricing.policy_editors SET revoked=true,version=$3 WHERE editor_id=$1 AND version=$2 AND NOT revoked")
            .bind(editor).bind(expected_version).bind(version).execute(&mut *tx).await?.rows_affected();
        if changed != 1 {
            return Err(Error::Conflict);
        }
        event(&mut tx, editor, version, "revoke").await?;
        tx.commit().await.map_err(|_| Error::CommitUnknown)?;
        Ok(version)
    }
}
async fn check_version(
    c: &mut PgConnection,
    editor: &str,
    actor: i64,
    expected: i64,
) -> Result<(), Error> {
    let row=sqlx::query("SELECT actor_user_id,version FROM core_trust_pricing.policy_editors WHERE editor_id=$1 FOR UPDATE")
        .bind(editor).fetch_optional(c).await?;
    match row {
        None if expected == 0 => Ok(()),
        Some(r)
            if r.try_get::<i64, _>("actor_user_id")? == actor
                && r.try_get::<i64, _>("version")? == expected =>
        {
            Ok(())
        }
        _ => Err(Error::Conflict),
    }
}
async fn event(
    c: &mut PgConnection,
    editor: &str,
    version: i64,
    action: &str,
) -> Result<(), Error> {
    sqlx::query("INSERT INTO core_trust_pricing.policy_editor_events(editor_id,version,action,owner_role) VALUES($1,$2,$3,current_user)")
        .bind(editor).bind(version).bind(action).execute(c).await?;
    Ok(())
}

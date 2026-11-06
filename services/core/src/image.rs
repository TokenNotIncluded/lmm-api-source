//! Shared image input and drawing authority contracts. Financial admission and
//! model dispatch validate the same stored source without importing each other.
use crate::identity::{self, CredentialKind};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use sqlx::PgConnection;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum Error {
    Invalid,
    Forbidden,
    Conflict,
    Retry,
    Storage,
}
type Result<T> = std::result::Result<T, Error>;

impl From<sqlx::Error> for Error {
    fn from(e: sqlx::Error) -> Self {
        match e.as_database_error().and_then(|e| e.code()).as_deref() {
            Some("40001" | "40P01" | "55P03") => Self::Retry,
            Some("23505") => Self::Conflict,
            _ => Self::Storage,
        }
    }
}
pub(crate) const MAX_REFERENCES: usize = 8;
pub(crate) const MAX_REFERENCE_BYTES: usize = 10 * 1024 * 1024;
pub(crate) const MAX_TOTAL_REFERENCE_BYTES: usize = 16 * 1024 * 1024;

#[derive(Clone, Copy, Eq, PartialEq)]
pub(crate) enum ImageMediaType {
    Png,
    Jpeg,
    Webp,
}

#[derive(Clone, Debug, Deserialize, Serialize, Eq, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct DrawingInput {
    pub prompt: String,
    pub model: String,
    pub group: String,
    #[serde(default)]
    pub size: String,
    #[serde(default)]
    pub quality: String,
    #[serde(default = "one")]
    pub n: u32,
}
fn one() -> u32 {
    1
}
#[derive(Clone, Debug, Deserialize, Serialize, Eq, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct ReferenceImage {
    pub media_type: String,
    pub sha256: String,
    pub byte_length: u64,
}
#[derive(Clone, Debug, Serialize)]
pub struct DrawingPreviewInput {
    #[serde(flatten)]
    pub settings: DrawingInput,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub reference_images: Vec<ReferenceImage>,
}
impl std::ops::Deref for DrawingPreviewInput {
    type Target = DrawingInput;
    fn deref(&self) -> &DrawingInput {
        &self.settings
    }
}
impl From<DrawingInput> for DrawingPreviewInput {
    fn from(settings: DrawingInput) -> Self {
        Self {
            settings,
            reference_images: vec![],
        }
    }
}
// Reviewed additive reference-image contract. The binding comes from the
// persisted Core quote, never the browser's claimed tool/version metadata.
pub(crate) fn authorize_references(raw: &Value, version: &str, schema: &str) -> Result<()> {
    let parsed = parse_arguments(raw)?;
    if !parsed.reference_images.is_empty()
        && (version != "ca3a1996-723d-5695-b29e-40e3bef58644"
            || schema != "abd718715e097d729e362afd0f5cdf19ae664886a878902e1c38ac31a2c2964e")
    {
        return Err(Error::Forbidden);
    }
    Ok(())
}
pub(crate) fn media_type(value: &str) -> Result<ImageMediaType> {
    match value {
        "image/png" => Ok(ImageMediaType::Png),
        "image/jpeg" => Ok(ImageMediaType::Jpeg),
        "image/webp" => Ok(ImageMediaType::Webp),
        _ => Err(Error::Invalid),
    }
}
pub(crate) fn valid_reference(reference: &ReferenceImage) -> bool {
    reference.byte_length > 0
        && reference.byte_length <= MAX_REFERENCE_BYTES as u64
        && reference.sha256.len() == 64
        && reference
            .sha256
            .bytes()
            .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
        && media_type(&reference.media_type).is_ok()
}
pub(crate) fn parse_arguments(raw: &Value) -> Result<DrawingPreviewInput> {
    let mut object = raw.as_object().ok_or(Error::Invalid)?.clone();
    let references: Vec<ReferenceImage> = match object.remove("reference_images") {
        Some(v) => serde_json::from_value(v).map_err(|_| Error::Invalid)?,
        None => vec![],
    };
    if references.len() > MAX_REFERENCES
        || references.iter().any(|r| !valid_reference(r))
        || references
            .iter()
            .try_fold(0u64, |n, r| n.checked_add(r.byte_length))
            .is_none_or(|n| n > MAX_TOTAL_REFERENCE_BYTES as u64)
    {
        return Err(Error::Invalid);
    }
    let settings: DrawingInput =
        serde_json::from_value(Value::Object(object)).map_err(|_| Error::Invalid)?;
    Ok(DrawingPreviewInput {
        settings,
        reference_images: references,
    })
}
/// Only this exact Core-created envelope may contain persisted upload bytes.
/// Legacy generation/native-Key rows keep their original flat representation.
pub(crate) fn arguments(raw: &Value) -> Result<Value> {
    if raw.get("kind").and_then(Value::as_str) == Some("drawing_reference_v1") {
        let object = raw.as_object().ok_or(Error::Storage)?;
        if object.len() != 3 || !object.contains_key("uploads") {
            return Err(Error::Storage);
        }
        Ok(object.get("arguments").ok_or(Error::Storage)?.clone())
    } else {
        Ok(raw.clone())
    }
}
/// Private storage projection used by the same-transaction image charge guard.
/// It never accepts a caller envelope: the argument is the immutable Core row.
pub(crate) fn confirmed_drawing_arguments(stored: &Value) -> Result<Value> {
    let arguments = arguments(stored)?;
    parse_arguments(&arguments).map_err(|_| Error::Storage)?;
    Ok(arguments)
}
/// A persisted drawing keeps its real original credential as an immutable
/// anchor. Only a live current credential in that same browser family may
/// replace it; native Sessions still require the exact original credential.
/// The caller's original transaction and identity/financial locks are retained.
pub(crate) async fn drawing_session_anchor_matches(
    c: &mut PgConnection,
    actor: &identity::Principal,
    original: i64,
    version: i64,
) -> Result<bool> {
    if actor.credential_kind != CredentialKind::Session {
        return Ok(false);
    }
    let allowed: bool = sqlx::query_scalar(
        "SELECT EXISTS(SELECT 1 FROM core_identity.credentials old JOIN core_identity.credentials live ON live.id=$2 JOIN core_identity.users u ON u.id=$3 JOIN core_identity.accounts a ON a.id=$4 WHERE old.id=$1 AND old.kind='session' AND old.user_id=u.id AND old.owner_account_id=a.id AND old.user_version=$5 AND live.kind='session' AND live.user_id=u.id AND live.owner_account_id=a.id AND live.user_version=$5 AND live.revoked_at IS NULL AND live.expires_at>clock_timestamp() AND u.active AND a.active AND u.auth_version=$5 AND ((old.id=live.id AND NOT EXISTS(SELECT 1 FROM core_identity.web_session_credentials WHERE credential_id=old.id) AND NOT EXISTS(SELECT 1 FROM core_identity.web_sessions WHERE current_credential_id=old.id)) OR EXISTS(SELECT 1 FROM core_identity.web_session_credentials original_map JOIN core_identity.web_session_credentials current_map ON current_map.sid=original_map.sid JOIN core_identity.web_sessions family ON family.sid=original_map.sid WHERE original_map.credential_id=old.id AND current_map.credential_id=live.id AND family.current_credential_id=live.id AND family.user_id=u.id AND family.user_version=$5 AND family.revoked_at IS NULL AND family.expires_at>clock_timestamp() AND (NOT family.weekly_logout OR family.created_at>clock_timestamp()-interval '7 days'))))",
    )
    .bind(original)
    .bind(actor.credential_id)
    .bind(actor.user_id)
    .bind(actor.owner_account_id)
    .bind(version)
    .fetch_one(c)
    .await?;
    Ok(allowed)
}

pub(crate) async fn drawing_cookie_anchor_matches(
    c: &mut PgConnection,
    original: i64,
    sid: &str,
) -> Result<bool> {
    Ok(sqlx::query_scalar("SELECT (NOT EXISTS(SELECT 1 FROM core_identity.web_session_credentials WHERE credential_id=$1) AND NOT EXISTS(SELECT 1 FROM core_identity.web_sessions WHERE current_credential_id=$1)) OR EXISTS(SELECT 1 FROM core_identity.web_session_credentials m JOIN core_identity.web_sessions s ON s.sid=m.sid WHERE m.credential_id=$1 AND s.sid=$2 AND s.revoked_at IS NULL AND s.expires_at>clock_timestamp())")
        .bind(original).bind(sid).fetch_one(c).await?)
}

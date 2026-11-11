//! Owner-only, source-bound continuation for imported keys during one admin activation.
//! Credential identity and permanent revocation are never rewritten.
use super::{ApprovalDto, PlanAdminActivationRequest};
use serde::{Deserialize, Serialize};
use std::path::PathBuf;

pub const SCOPE: &str = "legacy-admin-key-continuity.v1";

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct MaterialFiles {
    pub identity_manifest: PathBuf,
    pub identity_map: PathBuf,
    pub credential_receipt: PathBuf,
    pub alias_manifest: PathBuf,
    pub source_projection: PathBuf,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PlanRequest {
    pub activation: PlanAdminActivationRequest,
    pub materials: MaterialFiles,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SourceProjection {
    pub scope: String,
    pub source_archive_sha256: String,
    pub source_receipt_sha256: String,
    pub source_receipt_file: PathBuf,
    pub source_rows_file: PathBuf,
    pub key_import_sql_file: PathBuf,
    pub observed_at: i64,
    pub identity_migration_id: String,
    pub key_import_migration_id: String,
    pub key_import_sql_sha256: String,
    pub source_user_id: i64,
    pub source_user_sha256: String,
    pub source_status: i16,
    pub source_deleted: bool,
    pub source_role: i16,
    pub source_auth_version: i64,
    pub keys: Vec<SourceKey>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SourceKey {
    pub source_key_id: i64,
    pub source_user_id: i64,
    pub source_secret_sha256: String,
    pub source_row_sha256: String,
    pub deleted: bool,
    pub managed: bool,
    pub status: i16,
    pub used_quota: i64,
    pub remain_quota: i64,
    pub unlimited_quota: bool,
    pub expires_at_unix: i64,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct KeyBinding {
    pub credential_id: i64,
    pub digest_sha256: String,
    pub user_id: i64,
    pub owner_account_id: i64,
    pub from_auth_version: i64,
    pub to_auth_version: i64,
    pub retained: bool,
    pub prestate_sha256: String,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Continuity {
    pub scope: String,
    pub materials: MaterialFiles,
    pub material_sha256: std::collections::BTreeMap<String, String>,
    pub keys: Vec<KeyBinding>,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ApplyRequest {
    pub confirmation_sha256: String,
    pub activation_confirmation_sha256: String,
    pub approval: ApprovalDto,
    pub continuity: Continuity,
}

#[derive(Debug, Serialize)]
pub struct PlanResponse {
    pub status: &'static str,
    pub confirmation_sha256: String,
    pub activation_confirmation_sha256: String,
    pub approval: ApprovalDto,
    pub continuity: Continuity,
}

use super::{
    ApplyAdminActivationResponse, Approval, ApprovalError, db, decode_hex_32, encode_hex_32,
};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use sqlx::{Connection, PgConnection, Postgres, Row, Transaction};
use std::{
    collections::{BTreeMap, BTreeSet},
    fs,
    io::Read,
    os::unix::fs::MetadataExt,
};

fn hash(bytes: &[u8]) -> String {
    encode_hex_32(&Sha256::digest(bytes).into())
}

fn value_hash(value: &impl Serialize) -> Result<String, ApprovalError> {
    Ok(hash(
        &serde_json::to_vec(value).map_err(|_| ApprovalError::InvalidJson)?,
    ))
}

fn private_file(path: &std::path::Path) -> Result<Vec<u8>, ApprovalError> {
    if !path.is_absolute() {
        return Err(ApprovalError::InvalidJson);
    }
    let metadata = fs::symlink_metadata(path).map_err(|_| ApprovalError::InvalidJson)?;
    let uid = fs::metadata("/proc/self")
        .map_err(|_| ApprovalError::Storage)?
        .uid();
    if !metadata.is_file()
        || metadata.uid() != uid
        || metadata.mode() & 0o077 != 0
        || metadata.len() > 4 * 1024 * 1024
    {
        return Err(ApprovalError::InvalidJson);
    }
    let mut file = fs::File::open(path).map_err(|_| ApprovalError::InvalidJson)?;
    let opened = file.metadata().map_err(|_| ApprovalError::InvalidJson)?;
    if (opened.dev(), opened.ino()) != (metadata.dev(), metadata.ino()) {
        return Err(ApprovalError::InvalidJson);
    }
    let mut bytes = Vec::new();
    file.by_ref()
        .take(4 * 1024 * 1024 + 1)
        .read_to_end(&mut bytes)
        .map_err(|_| ApprovalError::InvalidJson)?;
    if bytes.len() > 4 * 1024 * 1024 {
        return Err(ApprovalError::InputTooLarge);
    }
    Ok(bytes)
}

struct Materials {
    hashes: BTreeMap<String, String>,
    identity: Value,
    map: Vec<Value>,
    receipt: Value,
    aliases: Value,
    source: SourceProjection,
    source_rows: Value,
}

fn materials(files: &MaterialFiles) -> Result<Materials, ApprovalError> {
    let mut hashes = BTreeMap::new();
    let mut values = BTreeMap::new();
    let mut map = Vec::new();
    for (name, path) in [
        ("identity_manifest", &files.identity_manifest),
        ("identity_map", &files.identity_map),
        ("credential_receipt", &files.credential_receipt),
        ("alias_manifest", &files.alias_manifest),
        ("source_projection", &files.source_projection),
    ] {
        let bytes = private_file(path)?;
        hashes.insert(name.to_owned(), hash(&bytes));
        if name == "identity_map" {
            for line in bytes.split(|b| *b == b'\n').filter(|line| !line.is_empty()) {
                map.push(serde_json::from_slice(line).map_err(|_| ApprovalError::InvalidJson)?);
            }
        } else {
            values.insert(
                name,
                serde_json::from_slice::<Value>(&bytes).map_err(|_| ApprovalError::InvalidJson)?,
            );
        }
    }
    let source: SourceProjection = serde_json::from_value(
        values
            .remove("source_projection")
            .ok_or(ApprovalError::InvalidJson)?,
    )
    .map_err(|_| ApprovalError::InvalidJson)?;
    let source_rows_bytes = private_file(&source.source_rows_file)?;
    let source_receipt_bytes = private_file(&source.source_receipt_file)?;
    let import_sql_bytes = private_file(&source.key_import_sql_file)?;
    if hash(&source_receipt_bytes) != source.source_receipt_sha256
        || hash(&import_sql_bytes) != source.key_import_sql_sha256
    {
        return Err(ApprovalError::SourceMismatch);
    }
    let source_receipt: Value =
        serde_json::from_slice(&source_receipt_bytes).map_err(|_| ApprovalError::InvalidJson)?;
    if source_receipt["archive"]["sha256"] != source.source_archive_sha256 {
        return Err(ApprovalError::SourceMismatch);
    }
    hashes.insert("source_rows".into(), hash(&source_rows_bytes));
    hashes.insert("source_receipt".into(), hash(&source_receipt_bytes));
    hashes.insert("key_import_sql".into(), hash(&import_sql_bytes));
    let source_rows =
        serde_json::from_slice(&source_rows_bytes).map_err(|_| ApprovalError::InvalidJson)?;
    Ok(Materials {
        hashes,
        map,
        source,
        source_rows,
        identity: values
            .remove("identity_manifest")
            .ok_or(ApprovalError::InvalidJson)?,
        receipt: values
            .remove("credential_receipt")
            .ok_or(ApprovalError::InvalidJson)?,
        aliases: values
            .remove("alias_manifest")
            .ok_or(ApprovalError::InvalidJson)?,
    })
}

fn array<'a>(value: &'a Value, key: &str) -> Result<&'a Vec<Value>, ApprovalError> {
    value
        .get(key)
        .and_then(Value::as_array)
        .ok_or(ApprovalError::SourceMismatch)
}

fn source_row_hash(value: &Value) -> Result<String, ApprovalError> {
    let mut value = value.clone();
    value.sort_all_objects();
    value_hash(&value)
}

fn validate_source_rows(m: &Materials) -> Result<(), ApprovalError> {
    let s = &m.source;
    let users: Vec<_> = array(&m.source_rows, "users")?
        .iter()
        .filter(|u| u["id"] == s.source_user_id)
        .collect();
    if users.len() != 1 {
        return Err(ApprovalError::SourceMismatch);
    }
    let u = users[0];
    if source_row_hash(u)? != s.source_user_sha256
        || u["status"] != s.source_status
        || u["role"] != s.source_role
        || !u["deleted_at"].is_null() != s.source_deleted
        || u["auth_version"] != s.source_auth_version
    {
        return Err(ApprovalError::SourceMismatch);
    }
    let tokens: Vec<_> = array(&m.source_rows, "tokens")?
        .iter()
        .filter(|t| t["user_id"] == s.source_user_id)
        .collect();
    if tokens.len() != s.keys.len() {
        return Err(ApprovalError::SourceMismatch);
    }
    for key in &s.keys {
        let rows: Vec<_> = tokens
            .iter()
            .filter(|t| t["id"] == key.source_key_id)
            .collect();
        if rows.len() != 1 {
            return Err(ApprovalError::SourceMismatch);
        }
        let row = rows[0];
        let secret = row["key"].as_str().ok_or(ApprovalError::SourceMismatch)?;
        let lower = secret.to_ascii_lowercase();
        let ordinary = (32..=256).contains(&secret.len())
            && secret
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || b == b'_')
            && !lower.contains("lmm_")
            && !lower.contains("oauth_managed_")
            && ![
                "lmmk_", "lmms_", "lmmr_", "lmmi_", "lmmf_", "lmmt_", "lmmo_",
            ]
            .iter()
            .any(|p| lower.starts_with(p));
        let managed = row["oauth_managed"].as_bool().unwrap_or(false)
            || row["creation_source"] == "assistant_runtime"
            || !ordinary;
        let expiry = match row["expired_time"].as_i64() {
            None | Some(-1) => 253402300799,
            Some(v) => v.checked_add(1).ok_or(ApprovalError::SourceMismatch)?,
        };
        if hash(secret.as_bytes()) != key.source_secret_sha256
            || source_row_hash(row)? != key.source_row_sha256
            || !row["deleted_at"].is_null() != key.deleted
            || managed != key.managed
            || row["status"] != key.status
            || row["used_quota"].as_i64().unwrap_or(0) != key.used_quota
            || row["remain_quota"].as_i64().unwrap_or(0) != key.remain_quota
            || row["unlimited_quota"].as_bool().unwrap_or(false) != key.unlimited_quota
            || expiry != key.expires_at_unix
        {
            return Err(ApprovalError::SourceMismatch);
        }
    }
    Ok(())
}

fn validate_materials(m: &Materials, a: &Approval) -> Result<(), ApprovalError> {
    validate_source_rows(m)?;
    let s = &m.source;
    let archive = encode_hex_32(&a.source.archive_sha256);
    let role = match a.source.role {
        super::PlatformRole::Root => 100,
        super::PlatformRole::Admin => 10,
        _ => return Err(ApprovalError::SourceMismatch),
    };
    for digest in [
        &s.source_receipt_sha256,
        &s.key_import_sql_sha256,
        &s.source_user_sha256,
    ] {
        decode_hex_32(digest)?;
    }
    if s.scope != "legacy-admin-key-continuity-source.v1"
        || s.source_archive_sha256 != archive
        || s.identity_migration_id != a.source.migration_id
        || s.source_status != 1
        || s.source_deleted
        || s.source_role != role
        || s.source_auth_version != a.source.auth_version
        || s.observed_at <= 0
        || s.key_import_migration_id.is_empty()
        || s.keys.is_empty()
        || s.keys.len() > 1024
        || m.identity["scope"] != "identity-only"
        || m.identity["source_archive_sha256"] != archive
        || m.identity["mapping_file_sha256"] != encode_hex_32(&a.source.a2_file_sha256)
        || m.identity["source_target_id_map_sha256"] != m.hashes["identity_map"]
        || m.receipt["scope"] != "a02-credential-mapping-receipt"
        || m.receipt["source_archive_sha256"] != archive
        || m.aliases["version"] != 1
        || m.aliases["scope"] != "legacy-api-key-wire-aliases-v1"
        || m.aliases["source_archive_sha256"] != archive
        || m.aliases["credential_receipt_sha256"] != m.hashes["credential_receipt"]
        || m.aliases["target"]["database_oid"] != a.target.database_oid
        || m.aliases["target"]["instance_nonce"] != encode_hex_32(&a.target.instance_nonce)
    {
        return Err(ApprovalError::SourceMismatch);
    }
    let mapping: Vec<_> = m
        .map
        .iter()
        .filter(|row| row["source_user_id"] == s.source_user_id)
        .collect();
    if mapping.len() != 1
        || mapping[0]["target_user_id"] != a.source.user_id
        || mapping[0]["target_personal_account_id"] != a.source.personal_account_id
    {
        return Err(ApprovalError::SourceMismatch);
    }
    let original: Vec<_> = array(&m.receipt, "records")?
        .iter()
        .filter(|r| r["target_user_id"] == a.source.user_id)
        .collect();
    let aliases: Vec<_> = array(&m.aliases, "keys")?
        .iter()
        .filter(|r| r["target_user_id"] == a.source.user_id)
        .collect();
    if original.len() != s.keys.len() || aliases.len() != s.keys.len() {
        return Err(ApprovalError::SourceMismatch);
    }
    let mut ids = BTreeSet::new();
    for key in &s.keys {
        decode_hex_32(&key.source_secret_sha256)?;
        decode_hex_32(&key.source_row_sha256)?;
        if key.source_key_id <= 0
            || key.source_user_id != s.source_user_id
            || !ids.insert(key.source_key_id)
            || ![1, 2].contains(&key.status)
            || key.used_quota < 0
            || key.used_quota.checked_add(key.remain_quota).is_none()
        {
            return Err(ApprovalError::SourceMismatch);
        }
        let matches: Vec<_> = original
            .iter()
            .filter(|r| r["source_key_id"] == key.source_key_id)
            .collect();
        let binds: Vec<_> = aliases
            .iter()
            .filter(|r| r["source_key_id"] == key.source_key_id)
            .collect();
        if matches.len() != 1 || binds.len() != 1 {
            return Err(ApprovalError::SourceMismatch);
        }
        for r in [matches[0], binds[0]] {
            if r["target_credential_id"] != key.source_key_id
                || r["target_account_id"] != a.source.personal_account_id
                || r["source_secret_sha256"] != key.source_secret_sha256
            {
                return Err(ApprovalError::SourceMismatch);
            }
        }
        if binds[0]["credential_user_version"] != a.source.auth_version
            || binds[0]["revoked_rekey_required"]
                .as_bool()
                .unwrap_or(false)
                != key.managed
        {
            return Err(ApprovalError::SourceMismatch);
        }
    }
    Ok(())
}

async fn capture(
    tx: &mut Transaction<'_, Postgres>,
    a: &Approval,
    m: &Materials,
    lock: bool,
) -> Result<Vec<KeyBinding>, ApprovalError> {
    validate_materials(m, a)?;
    let actual: Value =
        sqlx::query_scalar("SELECT manifest FROM core_migration.import_runs WHERE migration_id=$1")
            .bind(&a.source.migration_id)
            .fetch_one(&mut **tx)
            .await
            .map_err(db)?;
    if actual != m.identity {
        return Err(ApprovalError::SourceMismatch);
    }
    let ids: Vec<i64> = m.source.keys.iter().map(|k| k.source_key_id).collect();
    if lock {
        for statement in [
            "SELECT credential_id FROM core_identity.api_key_settings WHERE credential_id=ANY($1) ORDER BY credential_id FOR SHARE",
            "SELECT credential_id FROM core_identity.key_funding_rules WHERE credential_id=ANY($1) ORDER BY credential_id,position FOR SHARE",
            "SELECT id FROM core_charging.budgets WHERE key_id=ANY($1) ORDER BY id FOR SHARE",
            "SELECT key_id FROM core_charging.budget_opening_consumption WHERE key_id=ANY($1) ORDER BY key_id,start_at FOR SHARE",
        ] {
            sqlx::query(statement)
                .bind(&ids)
                .fetch_all(&mut **tx)
                .await
                .map_err(db)?;
        }
    }
    let used_native:bool=sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM core_charging.requests WHERE key_id=ANY($1)) OR EXISTS(SELECT 1 FROM core_charging.external_consumption WHERE key_id=ANY($1))")
        .bind(&ids).fetch_one(&mut **tx).await.map_err(db)?;
    if used_native {
        return Err(ApprovalError::SourceMismatch);
    }
    let suffix = if lock { " FOR UPDATE OF c" } else { "" };
    let query = format!(
        r#"SELECT c.*,extract(epoch FROM c.expires_at)::bigint AS expiry,
        s.enabled, jsonb_build_object('credential',to_jsonb(c),'settings',to_jsonb(s),
        'funding',(SELECT coalesce(jsonb_agg(to_jsonb(f) ORDER BY position),'[]') FROM core_identity.key_funding_rules f WHERE f.credential_id=c.id),
        'budgets',(SELECT coalesce(jsonb_agg(to_jsonb(b) ORDER BY id),'[]') FROM core_charging.budgets b WHERE b.key_id=c.id),
        'opening',(SELECT coalesce(jsonb_agg(to_jsonb(o) ORDER BY start_at,import_reference),'[]') FROM core_charging.budget_opening_consumption o WHERE o.key_id=c.id)) AS state
        FROM core_identity.credentials c LEFT JOIN core_identity.api_key_settings s ON s.credential_id=c.id
        WHERE c.user_id=$1 AND c.kind='api_key' ORDER BY c.id{suffix}"#
    );
    let rows = sqlx::query(&query)
        .bind(a.source.user_id)
        .fetch_all(&mut **tx)
        .await
        .map_err(db)?;
    if rows.len() != m.source.keys.len() {
        return Err(ApprovalError::SourceMismatch);
    }
    let to = a
        .source
        .auth_version
        .checked_add(1)
        .ok_or(ApprovalError::VersionConflict)?;
    let mut keys = Vec::new();
    for row in rows {
        let id: i64 = row.try_get("id").map_err(db)?;
        let source = m
            .source
            .keys
            .iter()
            .find(|k| k.source_key_id == id)
            .ok_or(ApprovalError::SourceMismatch)?;
        let digest: Vec<u8> = row.try_get("digest").map_err(db)?;
        let state: Value = row.try_get("state").map_err(db)?;
        let revoked = !state["credential"]["revoked_at"].is_null();
        let budgets = array(&state, "budgets")?;
        let opening = array(&state, "opening")?;
        let funding = array(&state, "funding")?;
        if digest.as_slice() != decode_hex_32(&source.source_secret_sha256)?
            || row.try_get::<i64, _>("user_version").map_err(db)? != a.source.auth_version
            || row.try_get::<i64, _>("owner_account_id").map_err(db)?
                != a.source.personal_account_id
            || row.try_get::<i64, _>("expiry").map_err(db)? != source.expires_at_unix
            || row.try_get::<bool, _>("enabled").map_err(db)? != (source.status == 1)
            || revoked != (source.deleted || source.managed)
            || funding.len() != 1
            || funding[0]["position"] != 0
            || funding[0]["payer_account_id"] != a.source.personal_account_id
            || budgets.len() != 1
            || budgets[0]["scope"] != "key"
            || budgets[0]["period"] != "lifetime"
            || budgets[0]["unlimited"] != source.unlimited_quota
            || budgets[0]["issuer_user_id"] != a.source.user_id
            || opening.len() != 1
            || opening[0]["scope"] != "key"
            || opening[0]["period"] != "lifetime"
            || opening[0]["consumed_credits"] != source.used_quota
            || opening[0]["anchor"] != 0
            || opening[0]["seconds"] != 0
            || opening[0]["timezone"] != "UTC"
            || opening[0]["start_at"] != -62135596800_i64
            || opening[0]["end_at"] != 253402300800_i64
            || opening[0]["import_reference"]
                != format!("key-quota:{}:{id}", m.source.key_import_migration_id)
        {
            return Err(ApprovalError::SourceMismatch);
        }
        let limit = source.used_quota + source.remain_quota;
        if budgets[0]["limit_credits"] != if source.unlimited_quota { 0 } else { limit }
            || budgets[0]["display_limit_credits"]
                != if source.unlimited_quota {
                    json!(limit)
                } else {
                    Value::Null
                }
        {
            return Err(ApprovalError::SourceMismatch);
        }
        keys.push(KeyBinding {
            credential_id: id,
            digest_sha256: source.source_secret_sha256.clone(),
            user_id: a.source.user_id,
            owner_account_id: a.source.personal_account_id,
            from_auth_version: a.source.auth_version,
            to_auth_version: to,
            retained: !revoked,
            prestate_sha256: value_hash(&state)?,
        });
    }
    Ok(keys)
}

fn confirmation(approval_hash: &str, continuity: &Continuity) -> Result<String, ApprovalError> {
    decode_hex_32(approval_hash)?;
    value_hash(&json!([SCOPE, approval_hash, continuity]))
}

pub async fn plan(
    connection: &mut PgConnection,
    request: &PlanRequest,
) -> Result<PlanResponse, ApprovalError> {
    let response = super::plan_admin_activation(connection, &request.activation).await?;
    let approval = super::dto_to_approval(&response.candidate)?;
    let m = materials(&request.materials)?;
    let mut tx = connection.begin().await.map_err(db)?;
    sqlx::query("SET TRANSACTION ISOLATION LEVEL REPEATABLE READ READ ONLY")
        .execute(&mut *tx)
        .await
        .map_err(db)?;
    // Re-read the exact frozen identity in this snapshot before capturing keys.
    let (source, target, _, _, _) =
        super::load_frozen_source(&mut tx, approval.source.user_id, false, true).await?;
    if source != approval.source || target != approval.target {
        return Err(ApprovalError::SourceMismatch);
    }
    let keys = capture(&mut tx, &approval, &m, false).await?;
    let continuity = Continuity {
        scope: SCOPE.to_owned(),
        materials: request.materials.clone(),
        material_sha256: m.hashes,
        keys,
    };
    tx.commit().await.map_err(db)?;
    Ok(PlanResponse {
        status: "candidate",
        confirmation_sha256: confirmation(&response.confirmation_sha256, &continuity)?,
        activation_confirmation_sha256: response.confirmation_sha256,
        approval: response.candidate,
        continuity,
    })
}

pub async fn apply(
    connection: &mut PgConnection,
    request: &ApplyRequest,
) -> Result<ApplyAdminActivationResponse, ApprovalError> {
    if request.continuity.scope != SCOPE
        || confirmation(&request.activation_confirmation_sha256, &request.continuity)?
            != request.confirmation_sha256
    {
        return Err(ApprovalError::ConfirmationMismatch);
    }
    let normal = super::ApplyAdminActivationRequest {
        confirmation_sha256: request.activation_confirmation_sha256.clone(),
        approval: request.approval.clone(),
    };
    super::apply_admin_activation_inner(connection, &normal, Some(request)).await
}

fn operation(approval: &Approval) -> String {
    format!(
        "admin-key-continuity-u{}-v{}",
        approval.source.user_id, approval.source.auth_version
    )
}

pub(super) async fn replay(
    tx: &mut Transaction<'_, Postgres>,
    a: &Approval,
    r: &ApplyRequest,
) -> Result<bool, ApprovalError> {
    let manifest: Option<Value> =
        sqlx::query_scalar("SELECT manifest FROM core_migration.import_runs WHERE migration_id=$1")
            .bind(operation(a))
            .fetch_optional(&mut **tx)
            .await
            .map_err(db)?;
    if let Some(manifest) = manifest {
        if manifest["confirmation_sha256"] != r.confirmation_sha256
            || manifest["activation_payload_sha256"] != r.activation_confirmation_sha256
        {
            return Err(ApprovalError::ConfirmationMismatch);
        }
        return Ok(true);
    }
    Ok(false)
}

pub(super) async fn check_apply(
    tx: &mut Transaction<'_, Postgres>,
    a: &Approval,
    r: &ApplyRequest,
) -> Result<Vec<i64>, ApprovalError> {
    let m = materials(&r.continuity.materials)?;
    if m.hashes != r.continuity.material_sha256 {
        return Err(ApprovalError::SourceMismatch);
    }
    let keys = capture(tx, a, &m, true).await?;
    if serde_json::to_value(&keys).map_err(|_| ApprovalError::Storage)?
        != serde_json::to_value(&r.continuity.keys).map_err(|_| ApprovalError::Storage)?
    {
        return Err(ApprovalError::VersionConflict);
    }
    Ok(keys
        .into_iter()
        .filter(|k| k.retained)
        .map(|k| k.credential_id)
        .collect())
}

pub(super) async fn persist(
    tx: &mut Transaction<'_, Postgres>,
    a: &Approval,
    r: &ApplyRequest,
) -> Result<(), ApprovalError> {
    let manifest = json!({"scope":SCOPE,"user_id":a.source.user_id,"owner_account_id":a.source.personal_account_id,
        "source_archive_sha256":encode_hex_32(&a.source.archive_sha256),"approval_id":encode_hex_32(&a.id),
        "activation_payload_sha256":r.activation_confirmation_sha256,"confirmation_sha256":r.confirmation_sha256,
        "continuity":r.continuity,"keys":r.continuity.keys});
    sqlx::query("INSERT INTO core_migration.import_runs(migration_id,source_archive_sha256,mapping_sha256,manifest) VALUES($1,$2,$3,$4)")
        .bind(operation(a)).bind(encode_hex_32(&a.source.archive_sha256)).bind(&r.confirmation_sha256).bind(manifest)
        .execute(&mut **tx).await.map_err(db)?;
    Ok(())
}

/// Equality remains the normal path. A receipt authorizes exactly one imported
/// API-key generation, never a session or a chain of future version changes.
pub(crate) async fn generation_matches(
    connection: &mut PgConnection,
    id: i64,
    kind: &str,
    from: i64,
    current: i64,
) -> Result<bool, sqlx::Error> {
    if from == current {
        return Ok(true);
    }
    if kind != "api_key" || from.checked_add(1) != Some(current) {
        return Ok(false);
    }
    // Identity-only stores have no migration registry and cannot possess a
    // source migration bridge. Native version invalidation stays a denial.
    if !sqlx::query_scalar::<_, bool>(
        "SELECT to_regclass('core_migration.import_runs') IS NOT NULL",
    )
    .fetch_one(&mut *connection)
    .await?
    {
        return Ok(false);
    }
    sqlx::query_scalar(r#"SELECT EXISTS(SELECT 1 FROM core_identity.credentials c
        JOIN core_identity.users u ON u.id=c.user_id
        JOIN core_migration.import_runs m ON m.migration_id='admin-key-continuity-u'||c.user_id::text||'-v'||c.user_version::text
        CROSS JOIN LATERAL jsonb_array_elements(CASE WHEN jsonb_typeof(m.manifest->'keys')='array' THEN m.manifest->'keys' ELSE '[]'::jsonb END) k
        WHERE c.id=$1 AND m.manifest->>'scope'='legacy-admin-key-continuity.v1'
          AND m.manifest->'user_id'=to_jsonb(c.user_id) AND m.manifest->'owner_account_id'=to_jsonb(c.owner_account_id)
          AND k->'credential_id'=to_jsonb(c.id) AND k->>'digest_sha256'=encode(c.digest,'hex')
          AND k->'user_id'=to_jsonb(c.user_id) AND k->'owner_account_id'=to_jsonb(c.owner_account_id)
          AND k->'from_auth_version'=to_jsonb(c.user_version) AND k->'to_auth_version'=to_jsonb(u.auth_version)
          AND k->'retained'='true'::jsonb AND c.kind='api_key' AND c.revoked_at IS NULL)"#)
        .bind(id).fetch_one(connection).await
}

#[cfg(test)]
#[path = "admin_key_continuity_pg_tests.rs"]
mod pg_tests;

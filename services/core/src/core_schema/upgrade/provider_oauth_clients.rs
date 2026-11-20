//! Exact owner77->78 public-client transition; preserve every old row and ACL.
use super::*;

pub(super) const PROCEDURE: &str = "core-provider-oauth-clients-v1";
const TARGET: &str = include_str!("provider_oauth_clients_target.json");
const ENGINE: suffix::Suffix = suffix::Suffix {
    hooks: &registry::PROVIDER_OAUTH_CLIENTS,
    procedure: PROCEDURE,
    predecessor_count: 77,
    target_count: 78,
    predecessor_manifest: "45ff3f3ddb387dc2c587effd739bfb4cc152b50864511ab10e8d48389e6a3abe",
    predecessor_catalog: "151c66efa9e5d0af276e1b69a541b432c46d6c1c22d6e7838158595d2e110905",
    target_json: TARGET,
    receipt: "core_meta.provider_oauth_clients_upgrades",
};
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Captured {
    manifest_sha256: String,
    catalog_sha256: String,
    oauth_predecessor_catalog_sha256: String,
    oauth_catalog_sha256: String,
    raw_schema_sha256: String,
}
fn captured() -> Result<Captured, UpgradeError> {
    let target: Captured = serde_json::from_str(TARGET)
        .map_err(|_| reject("PROVIDER_OAUTH_CLIENTS_CAPTURE_REQUIRED"))?;
    if COMPONENTS.get(77).map(|v| v.0) != Some("provider-oauth-clients")
        || ![
            &target.manifest_sha256,
            &target.catalog_sha256,
            &target.oauth_predecessor_catalog_sha256,
            &target.oauth_catalog_sha256,
            &target.raw_schema_sha256,
        ]
        .into_iter()
        .all(|v| digest_hex(v))
        || hex(Sha256::digest(COMPONENTS[77].1)) != target.raw_schema_sha256
    {
        return Err(reject("PROVIDER_OAUTH_CLIENTS_CAPTURE_MISMATCH"));
    }
    Ok(target)
}
pub(super) fn captured_target() -> Result<(String, String), UpgradeError> {
    let t = captured()?;
    Ok((t.manifest_sha256, t.catalog_sha256))
}
pub(super) fn validate(r: &UpgradeRequest) -> Result<(), UpgradeError> {
    captured()?;
    ENGINE.validate(r)
}
pub(super) fn authorize_target() -> Result<(), UpgradeError> {
    captured()?;
    ENGINE.authorize_target()
}
pub(super) async fn connected(
    c: &mut PgConnection,
    r: &UpgradeRequest,
) -> Result<UpgradeReport, UpgradeError> {
    ENGINE.connected(c, r).await
}
/// The namespace check is read-only; a mismatch is never repaired by startup.
pub(in crate::core_schema) async fn check_oauth_contract(
    c: &mut PgConnection,
) -> Result<(), crate::identity::IdentityError> {
    let t = captured().map_err(|_| crate::identity::IdentityError::Storage)?;
    let rows: Vec<serde_json::Value> =
        sqlx::query_scalar("SELECT to_jsonb(c) FROM core_oauth.contract c")
            .fetch_all(&mut *c)
            .await?;
    let required = serde_json::json!({
        "singleton":true,"version":1,
        "ddl_fingerprint":format!("\\x{}",hex(Sha256::digest(crate::identity::grants::SCHEMA))),
        "catalog_fingerprint":format!("\\x{}",t.oauth_catalog_sha256)
    });
    if rows.as_slice() != [required]
        || hex(crate::identity::grants::catalog(c).await?) != t.oauth_catalog_sha256
    {
        return Err(crate::identity::IdentityError::Storage);
    }
    Ok(())
}

// Same row-multiset digest as preservation::data_projected, including the
// singleton table. The optional bytea replaces exactly one JSONB field before
// hashing; no row/table is omitted from either preservation snapshot.
async fn singleton_hash(
    c: &mut PgConnection,
    successor: Option<Vec<u8>>,
) -> Result<Vec<u8>, UpgradeError> {
    let fact: String = sqlx::query_scalar(
        r#"
SELECT json_build_array(encode(sha256(convert_to(
    (CASE WHEN $1::bytea IS NULL THEN to_jsonb(c)
          ELSE jsonb_set(to_jsonb(c),'{catalog_fingerprint}',to_jsonb($1::bytea)) END)::text,
    'UTF8')),'hex'),'1'::text)::text FROM core_oauth.contract c
"#,
    )
    .bind(successor)
    .fetch_one(c)
    .await?;
    let mut h = Sha256::new();
    h.update((fact.len() as u64).to_be_bytes());
    h.update(fact.as_bytes());
    Ok(h.finalize().to_vec())
}

pub(super) async fn advance_oauth_contract(
    c: &mut PgConnection,
    expected_data: &mut BTreeMap<String, Vec<u8>>,
) -> Result<(), UpgradeError> {
    let target = captured()?;
    let ddl = Sha256::digest(crate::identity::grants::SCHEMA).to_vec();
    let old = crate::hex_codec::decode(&target.oauth_predecessor_catalog_sha256)
        .map_err(|_| UpgradeError::Storage)?;
    let new = crate::hex_codec::decode(&target.oauth_catalog_sha256)
        .map_err(|_| UpgradeError::Storage)?;
    let rows: Vec<serde_json::Value> =
        sqlx::query_scalar("SELECT to_jsonb(c) FROM core_oauth.contract c")
            .fetch_all(&mut *c)
            .await?;
    let predecessor = serde_json::json!({
        "singleton":true,"version":1,"ddl_fingerprint":format!("\\x{}",hex(&ddl)),
        "catalog_fingerprint":format!("\\x{}",hex(&old))
    });
    if rows.as_slice() != [predecessor]
        || expected_data.get("core_oauth.contract") != Some(&singleton_hash(c, None).await?)
        || crate::identity::grants::catalog(c)
            .await
            .map_err(|_| UpgradeError::Storage)?
            != new
    {
        return Err(reject("PROVIDER_OAUTH_CLIENTS_OAUTH_PREDECESSOR_MISMATCH"));
    }
    let expected = singleton_hash(c, Some(new.clone())).await?;
    let changed=sqlx::query("UPDATE core_oauth.contract SET catalog_fingerprint=$1 WHERE singleton=true AND version=1 AND ddl_fingerprint=$2 AND catalog_fingerprint=$3")
        .bind(new).bind(ddl).bind(old).execute(&mut *c).await?;
    if changed.rows_affected() != 1 || singleton_hash(c, None).await? != expected {
        return Err(reject("PROVIDER_OAUTH_CLIENTS_OAUTH_CAS_MISMATCH"));
    }
    check_oauth_contract(c)
        .await
        .map_err(|_| UpgradeError::Storage)?;
    #[cfg(test)]
    if FAIL_AFTER_OAUTH_CAS
        .try_with(|value| *value)
        .unwrap_or(false)
    {
        return Err(reject("PROVIDER_OAUTH_CLIENTS_INJECTED_AFTER_OAUTH_CAS"));
    }
    *expected_data
        .get_mut("core_oauth.contract")
        .ok_or_else(|| reject("PROVIDER_OAUTH_CLIENTS_OAUTH_NOT_PRESERVED"))? = expected;
    Ok(())
}

#[cfg(test)]
tokio::task_local! { static FAIL_AFTER_OAUTH_CAS: bool; }

#[cfg(test)]
#[path = "provider_oauth_clients_tests.rs"]
mod tests;

//! Exact Core83 -> Core84; preserve existing identity, keys and acquisition history.
use super::*;
pub(super) const PROCEDURE: &str = "core-current-connection-history-v1";
const TARGET: &str = include_str!("current_connection_history_target.json");
const ENGINE: suffix::Suffix = suffix::Suffix {
    hooks: &registry::CURRENT_CONNECTION_HISTORY,
    procedure: PROCEDURE,
    predecessor_count: 83,
    target_count: 84,
    predecessor_manifest: "9a9be9a1295290fe2a2267cc843d93bf739b7cf6d0dddec2fa3ab790ecd7cf4c",
    predecessor_catalog: "6f635e46f3d68a1a08c9252976c14a1983b396acc46c1ef0edfdd1e5ffd6c3f6",
    target_json: TARGET,
    receipt: "core_meta.current_connection_history_upgrades",
};
fn captured() -> Result<(), UpgradeError> {
    let target: serde_json::Value =
        serde_json::from_str(TARGET).map_err(|_| UpgradeError::Storage)?;
    if !target["manifest_sha256"].as_str().is_some_and(digest_hex)
        || !target["catalog_sha256"].as_str().is_some_and(digest_hex)
    {
        return Err(reject("CURRENT_CONNECTION_HISTORY_TARGET_NOT_CAPTURED"));
    }
    Ok(())
}
pub(super) fn validate(r: &UpgradeRequest) -> Result<(), UpgradeError> {
    captured()?;
    ENGINE.validate(r)
}
pub(super) fn authorize_target() -> Result<(), UpgradeError> {
    captured()?;
    if COMPONENTS.get(83).map(|v| v.0) != Some("current-connection-history") {
        return Err(reject("CURRENT_CONNECTION_HISTORY_COMPONENT_CHANGED"));
    }
    ENGINE.authorize_target()
}
pub(super) async fn connected(
    c: &mut PgConnection,
    r: &UpgradeRequest,
) -> Result<UpgradeReport, UpgradeError> {
    captured()?;
    ENGINE.connected(c, r).await
}

// Only an already-enabled community reader receives the new read surface.
// No old object ACL, source importer authority, or owner receipt is granted.
pub(super) async fn grant_runtime(c: &mut PgConnection, role: &str) -> Result<(), UpgradeError> {
    let reads_history: bool = sqlx::query_scalar(
        "SELECT has_schema_privilege($1,'core_history','USAGE') AND NOT EXISTS(SELECT FROM unnest(ARRAY['document','mapping','native_credential_floor','snapshot_observed_at','target_database','target_database_oid','target_nonce']) AS columns(name) WHERE NOT has_column_privilege($1,'core_history.connection_snapshot',columns.name,'SELECT'))",
    ).bind(role).fetch_one(&mut *c).await?;
    if reads_history {
        sqlx::raw_sql(sqlx::AssertSqlSafe(format!(
            "GRANT SELECT(document,mapping,snapshot_observed_at,target_database,target_database_oid,target_nonce) ON core_history.current_connection_snapshot TO {role}; GRANT SELECT(target_credential_id) ON core_history.current_connection_credential_map TO {role}",
            role=quote(role)
        ))).execute(c).await?;
    }
    Ok(())
}

#[cfg(test)]
#[path = "current_connection_history_tests.rs"]
mod tests;

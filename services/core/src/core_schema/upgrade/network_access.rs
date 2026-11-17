//! Explicit 75 -> 76 using the existing exact-predecessor offline suffix engine.
//! Pending target captures cannot authorize a database connection.
use super::*;

pub(super) const PROCEDURE: &str = "core-network-access-v1";
const TARGET: &str = include_str!("network_access_target.json");
const ENGINE: suffix::Suffix = suffix::Suffix {
    hooks: &registry::NETWORK_ACCESS,
    procedure: PROCEDURE,
    predecessor_count: 75,
    target_count: 76,
    predecessor_manifest: "12f21130aefc0e5039e4674a673190fb71b236549872f5f2b1c9bd4ae495e7f8",
    predecessor_catalog: "034166f1f07cd08a4a46d4022d8c56766da843bc3d544f64a20aa6d6b1879a7c",
    target_json: TARGET,
    receipt: "core_meta.network_access_upgrades",
};

fn captured() -> Result<(), UpgradeError> {
    let target: serde_json::Value =
        serde_json::from_str(TARGET).map_err(|_| UpgradeError::Storage)?;
    if !target["manifest_sha256"].as_str().is_some_and(digest_hex)
        || !target["catalog_sha256"].as_str().is_some_and(digest_hex)
    {
        return Err(reject("NETWORK_ACCESS_TARGET_NOT_CAPTURED"));
    }
    Ok(())
}
pub(super) fn validate(request: &UpgradeRequest) -> Result<(), UpgradeError> {
    captured()?;
    ENGINE.validate(request)
}
pub(super) fn authorize_target() -> Result<(), UpgradeError> {
    captured()?;
    if COMPONENTS.get(75).map(|value| value.0) != Some("network-access") {
        return Err(reject("NETWORK_ACCESS_COMPONENT_CHANGED"));
    }
    ENGINE.authorize_target()
}
pub(super) async fn connected(
    connection: &mut PgConnection,
    request: &UpgradeRequest,
) -> Result<UpgradeReport, UpgradeError> {
    captured()?;
    ENGINE.connected(connection, request).await
}
pub(super) async fn grant_runtime(
    connection: &mut PgConnection,
    runtime: &str,
) -> Result<(), UpgradeError> {
    sqlx::raw_sql(sqlx::AssertSqlSafe(format!(
        "GRANT EXECUTE ON FUNCTION core_console.write_ip_access_rules(text) TO {}",
        quote(runtime)
    )))
    .execute(connection)
    .await?;
    Ok(())
}

#[cfg(test)]
#[path = "network_access_tests.rs"]
mod tests;

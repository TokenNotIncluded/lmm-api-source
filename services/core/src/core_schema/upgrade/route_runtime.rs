//! Exact Core88 -> Core89. Runtime health and affinity tables start empty.
use super::*;
pub(super) const PROCEDURE: &str = "core-route-runtime-v1";
const TARGET: &str = include_str!("route_runtime_target.json");
const ENGINE: suffix::Suffix = suffix::Suffix {
    hooks: &registry::ROUTE_RUNTIME,
    procedure: PROCEDURE,
    predecessor_count: 88,
    target_count: 89,
    predecessor_manifest: "c5f124d66e316089e24670a7cae5a705bb2a2ef4d8f85e9fdbcc3dc282022b9c",
    predecessor_catalog: "528ee163e1b557b65c4549b4d0487e83fd5d1f6d2316e5d55a574a2a0b6f3290",
    target_json: TARGET,
    receipt: "core_meta.route_runtime_upgrades",
};
fn captured() -> Result<(), UpgradeError> {
    let target: serde_json::Value =
        serde_json::from_str(TARGET).map_err(|_| UpgradeError::Storage)?;
    if !target["manifest_sha256"].as_str().is_some_and(digest_hex)
        || !target["catalog_sha256"].as_str().is_some_and(digest_hex)
    {
        return Err(reject("ROUTE_RUNTIME_TARGET_NOT_CAPTURED"));
    }
    Ok(())
}
pub(super) fn validate(r: &UpgradeRequest) -> Result<(), UpgradeError> {
    captured()?;
    ENGINE.validate(r)
}
pub(super) fn authorize_target() -> Result<(), UpgradeError> {
    captured()?;
    if COMPONENTS.get(88).map(|v| v.0) != Some("route-runtime") {
        return Err(reject("ROUTE_RUNTIME_COMPONENT_CHANGED"));
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
pub(super) async fn grant_runtime(c: &mut PgConnection, role: &str) -> Result<(), UpgradeError> {
    let channels: bool = sqlx::query_scalar(
        "SELECT has_schema_privilege($1,'core_channels','USAGE')
         AND has_table_privilege($1,'core_channels.identities','SELECT')
         AND has_column_privilege($1,'core_channels.catalog_observations','result','UPDATE')",
    )
    .bind(role)
    .fetch_one(&mut *c)
    .await?;
    if channels {
        let grants = include_str!("../../../schema/route_runtime_grants.sql")
            .replace(":\"runtime_role\"", &quote(role));
        sqlx::raw_sql(sqlx::AssertSqlSafe::<&str>(&grants))
            .execute(c)
            .await?;
    }
    Ok(())
}

#[cfg(test)]
#[path = "route_runtime_tests.rs"]
pub(super) mod tests;

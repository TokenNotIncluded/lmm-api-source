//! Explicit 68 -> 69. Target hashes require capture on isolated PostgreSQL 18.6;
//! uncaptured targets reject before connecting, rather than accepting caller SQL.
use super::*;
pub(super) const PROCEDURE: &str = "core-responses-continuity-v1";
const TARGET: &str = include_str!("responses_continuity_target.json");
const ENGINE: suffix::Suffix = suffix::Suffix {
    hooks: &registry::RESPONSES_CONTINUITY,
    procedure: PROCEDURE,
    predecessor_count: 68,
    target_count: 69,
    predecessor_manifest: "eeae72df806b9069e2c3bddc65c5f8271bf8b3c4b937b0de9ebb13e3ebcf8ced",
    predecessor_catalog: "2f067e60e07c28a38fb9ea03fe36f8740da8bddd087f80e794b843da6ff6e190",
    target_json: TARGET,
    receipt: "core_meta.responses_continuity_upgrades",
};
fn captured() -> Result<(), UpgradeError> {
    let target: serde_json::Value =
        serde_json::from_str(TARGET).map_err(|_| UpgradeError::Storage)?;
    if !target["manifest_sha256"].as_str().is_some_and(digest_hex)
        || !target["catalog_sha256"].as_str().is_some_and(digest_hex)
    {
        return Err(reject("RESPONSES_CONTINUITY_TARGET_NOT_CAPTURED"));
    }
    Ok(())
}
pub(super) fn validate(r: &UpgradeRequest) -> Result<(), UpgradeError> {
    captured()?;
    ENGINE.validate(r)
}
pub(super) fn authorize_target() -> Result<(), UpgradeError> {
    captured()?;
    if COMPONENTS.get(68).map(|v| v.0) != Some("responses-continuity") {
        return Err(reject("RESPONSES_CONTINUITY_COMPONENT_CHANGED"));
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
pub(super) async fn grant_runtime(c: &mut PgConnection, runtime: &str) -> Result<(), UpgradeError> {
    sqlx::raw_sql(sqlx::AssertSqlSafe(
        include_str!("../../../schema/responses_continuity_grants.sql")
            .replace(":\"runtime_role\"", &quote(runtime)),
    ))
    .execute(c)
    .await?;
    Ok(())
}
#[cfg(test)]
#[path = "responses_continuity_tests.rs"]
mod tests;

#[cfg(test)]
tokio::task_local! { pub(super) static FAIL_AFTER_DDL: bool; }
#[cfg(test)]
pub(super) fn fail_after_ddl() -> bool {
    FAIL_AFTER_DDL.try_with(|v| *v).unwrap_or(false)
}

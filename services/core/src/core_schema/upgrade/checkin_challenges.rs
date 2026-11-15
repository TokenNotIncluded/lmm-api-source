//! Explicit 74 -> 75. Target hashes require capture on isolated PostgreSQL 18.6;
//! uncaptured targets reject before connecting, rather than accepting caller SQL.
use super::*;
pub(super) const PROCEDURE: &str = "core-checkin-challenges-v1";
const TARGET: &str = include_str!("checkin_challenges_target.json");
const ENGINE: suffix::Suffix = suffix::Suffix {
    hooks: &registry::CHECKIN_CHALLENGES,
    procedure: PROCEDURE,
    predecessor_count: 74,
    target_count: 75,
    predecessor_manifest: "d193076302254b791db8815bc294944b05fa6dd6f05cd0484d66137f4b98cab6",
    predecessor_catalog: "5b2676238552abb5b0d19fe7f371464a5c84b500211b135e16087e61966f701c",
    target_json: TARGET,
    receipt: "core_meta.checkin_challenges_upgrades",
};
fn captured() -> Result<(), UpgradeError> {
    let target: serde_json::Value =
        serde_json::from_str(TARGET).map_err(|_| UpgradeError::Storage)?;
    if !target["manifest_sha256"].as_str().is_some_and(digest_hex)
        || !target["catalog_sha256"].as_str().is_some_and(digest_hex)
    {
        return Err(reject("CHECKIN_CHALLENGES_TARGET_NOT_CAPTURED"));
    }
    Ok(())
}
pub(super) fn validate(r: &UpgradeRequest) -> Result<(), UpgradeError> {
    captured()?;
    ENGINE.validate(r)
}
pub(super) fn authorize_target() -> Result<(), UpgradeError> {
    captured()?;
    if COMPONENTS.get(74).map(|v| v.0) != Some("checkin-challenges") {
        return Err(reject("CHECKIN_CHALLENGES_COMPONENT_CHANGED"));
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
        include_str!("../../../schema/checkin_challenges_grants.sql")
            .replace(":\"runtime_role\"", &quote(runtime)),
    ))
    .execute(c)
    .await?;
    Ok(())
}

#[cfg(test)]
#[path = "checkin_challenges_tests.rs"]
mod tests;

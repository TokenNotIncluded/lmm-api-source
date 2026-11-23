//! Exact Core78 -> Core79, no rewrite of predecessor rows or historical usage.
use super::*;
pub(super) const PROCEDURE: &str = "core-subscription-reset-v1";
const TARGET: &str = include_str!("subscription_reset_target.json");
const ENGINE: suffix::Suffix = suffix::Suffix {
    hooks: &registry::SUBSCRIPTION_RESET,
    procedure: PROCEDURE,
    predecessor_count: 78,
    target_count: 79,
    predecessor_manifest: "b9d0a472bddc826e0f6bfa0e725e409e67cbaabad6b0ad80cd512de24da74c2b",
    predecessor_catalog: "dbc1df98f792d1484700e36005e900ec97a54a21a56d752bab0863bd84ab7177",
    target_json: TARGET,
    receipt: "core_meta.subscription_reset_upgrades",
};
fn captured() -> Result<(), UpgradeError> {
    let target: serde_json::Value =
        serde_json::from_str(TARGET).map_err(|_| UpgradeError::Storage)?;
    if !target["manifest_sha256"].as_str().is_some_and(digest_hex)
        || !target["catalog_sha256"].as_str().is_some_and(digest_hex)
    {
        return Err(reject("SUBSCRIPTION_RESET_TARGET_NOT_CAPTURED"));
    }
    Ok(())
}
pub(super) fn validate(r: &UpgradeRequest) -> Result<(), UpgradeError> {
    captured()?;
    ENGINE.validate(r)
}
pub(super) fn authorize_target() -> Result<(), UpgradeError> {
    captured()?;
    if COMPONENTS.get(78).map(|v| v.0) != Some("subscription-reset-vouchers") {
        return Err(reject("SUBSCRIPTION_RESET_COMPONENT_CHANGED"));
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
    let sql = include_str!("../../../schema/subscription_reset_grants.sql")
        .replace(":\"runtime_role\"", &quote(role));
    sqlx::raw_sql(sqlx::AssertSqlSafe(sql)).execute(c).await?;
    Ok(())
}

#[cfg(test)]
#[path = "subscription_reset_tests.rs"]
mod tests;

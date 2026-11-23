//! Exact Core79 -> Core80, no rewrite of predecessor rows or historical usage.
use super::*;
pub(super) const PROCEDURE: &str = "core-legacy-gift-continuation-v1";
const TARGET: &str = include_str!("legacy_gift_target.json");
const ENGINE: suffix::Suffix = suffix::Suffix {
    hooks: &registry::LEGACY_GIFT,
    procedure: PROCEDURE,
    predecessor_count: 79,
    target_count: 80,
    predecessor_manifest: "4be06ebe40d321858934227b3b7069d0083eb83337bcd87588ea03a21f30fa94",
    predecessor_catalog: "eb2e77b6786846fb8d6ba322628dc76aa66c2c28f1106e5a1e4046a873f561fb",
    target_json: TARGET,
    receipt: "core_meta.legacy_gift_upgrades",
};
fn captured() -> Result<(), UpgradeError> {
    let target: serde_json::Value =
        serde_json::from_str(TARGET).map_err(|_| UpgradeError::Storage)?;
    if !target["manifest_sha256"].as_str().is_some_and(digest_hex)
        || !target["catalog_sha256"].as_str().is_some_and(digest_hex)
    {
        return Err(reject("LEGACY_GIFT_TARGET_NOT_CAPTURED"));
    }
    Ok(())
}
pub(super) fn validate(r: &UpgradeRequest) -> Result<(), UpgradeError> {
    captured()?;
    ENGINE.validate(r)
}
pub(super) fn authorize_target() -> Result<(), UpgradeError> {
    captured()?;
    if COMPONENTS.get(79).map(|v| v.0) != Some("legacy-gift-continuation") {
        return Err(reject("LEGACY_GIFT_COMPONENT_CHANGED"));
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
    let sql = include_str!("../../../schema/gift_legacy_continuation_grants.sql")
        .replace(":\"runtime_role\"", &quote(role));
    sqlx::raw_sql(sqlx::AssertSqlSafe(sql)).execute(c).await?;
    Ok(())
}

#[cfg(test)]
#[path = "legacy_gift_tests.rs"]
mod tests;

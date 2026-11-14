//! Explicit 67 -> 68. Target hashes require capture on isolated PostgreSQL 18.6;
//! uncaptured targets reject before connecting, rather than accepting caller SQL.
use super::*;
pub(super) const PROCEDURE: &str = "core-payment-discount-management-v1";
const TARGET: &str = include_str!("payment_discount_management_target.json");
const ENGINE: suffix::Suffix = suffix::Suffix {
    hooks: &registry::PAYMENT_DISCOUNT_MANAGEMENT,
    procedure: PROCEDURE,
    predecessor_count: 67,
    target_count: 68,
    predecessor_manifest: "8a6bed1a727ea891f270b0d78b5dbc67912a94e244661f29e3ab9541ccf94fa0",
    predecessor_catalog: "77dc2ffca5d5d4870452f15c7e1cbd5a29d238412bdcd4e592a388492dac321a",
    target_json: TARGET,
    receipt: "core_meta.payment_discount_management_upgrades",
};
fn captured() -> Result<(), UpgradeError> {
    let target: serde_json::Value =
        serde_json::from_str(TARGET).map_err(|_| UpgradeError::Storage)?;
    if !target["manifest_sha256"].as_str().is_some_and(digest_hex)
        || !target["catalog_sha256"].as_str().is_some_and(digest_hex)
    {
        return Err(reject("PAYMENT_DISCOUNT_MANAGEMENT_TARGET_NOT_CAPTURED"));
    }
    Ok(())
}
pub(super) fn validate(r: &UpgradeRequest) -> Result<(), UpgradeError> {
    captured()?;
    ENGINE.validate(r)
}
pub(super) fn authorize_target() -> Result<(), UpgradeError> {
    captured()?;
    if COMPONENTS.get(67).map(|v| v.0) != Some("payment-discount-management") {
        return Err(reject("PAYMENT_DISCOUNT_MANAGEMENT_COMPONENT_CHANGED"));
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
        include_str!("../../../schema/payment_discount_management_grants.sql")
            .replace(":\"runtime_role\"", &quote(runtime)),
    ))
    .execute(c)
    .await?;
    Ok(())
}
#[cfg(test)]
#[path = "payment_discount_management_tests.rs"]
mod tests;

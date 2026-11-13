//! Explicit 66 -> 67. Target hashes require capture on isolated PostgreSQL 18.6;
//! uncaptured targets reject before connecting, rather than accepting caller SQL.
use super::*;
pub(super) const PROCEDURE: &str = "core-payment-discounts-v1";
const TARGET: &str = include_str!("payment_discounts_target.json");
const ENGINE: suffix::Suffix = suffix::Suffix {
    procedure: PROCEDURE,
    predecessor_count: 66,
    target_count: 67,
    predecessor_manifest: "9a5f3b495c2818da2d011907457a2358591a68af115f399fd1a9e1496b142427",
    predecessor_catalog: "19340f14a352f458f58dd167bd8784c1e97a9c63708c40b107ab70f222c68909",
    target_json: TARGET,
    receipt: "core_meta.payment_discount_upgrades",
};
fn captured() -> Result<(), UpgradeError> {
    let target: serde_json::Value =
        serde_json::from_str(TARGET).map_err(|_| UpgradeError::Storage)?;
    if !target["manifest_sha256"].as_str().is_some_and(digest_hex)
        || !target["catalog_sha256"].as_str().is_some_and(digest_hex)
    {
        return Err(reject("PAYMENT_DISCOUNTS_TARGET_NOT_CAPTURED"));
    }
    Ok(())
}
pub(super) fn validate(r: &UpgradeRequest) -> Result<(), UpgradeError> {
    captured()?;
    ENGINE.validate(r)
}
pub(super) fn authorize_target() -> Result<(), UpgradeError> {
    captured()?;
    if COMPONENTS.get(66).map(|v| v.0) != Some("payment-discounts") {
        return Err(reject("PAYMENT_DISCOUNTS_COMPONENT_CHANGED"));
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
    sqlx::raw_sql(
        &include_str!("../../../schema/payment_discounts_grants.sql")
            .replace(":\"runtime_role\"", &quote(runtime)),
    )
    .execute(c)
    .await?;
    Ok(())
}
#[cfg(test)]
#[path = "payment_discounts_tests.rs"]
mod tests;

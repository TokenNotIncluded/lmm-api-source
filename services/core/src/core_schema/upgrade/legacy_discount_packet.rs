//! Exact Core80 -> Core81, no rewrite of predecessor rows or historical usage.
use super::*;
pub(super) const PROCEDURE: &str = "core-legacy-discount-packet-continuation-v1";
const TARGET: &str = include_str!("legacy_discount_packet_target.json");
const ENGINE: suffix::Suffix = suffix::Suffix {
    hooks: &registry::LEGACY_DISCOUNT_PACKET,
    procedure: PROCEDURE,
    predecessor_count: 80,
    target_count: 81,
    predecessor_manifest: "d2847d661269363530feb27a0ffdfbaaf2d0053bb5cf51926acc3b2debf0c64d",
    predecessor_catalog: "50687a7ae8c568df1d6a769bbf7c1f08d9e85c960e40fb88e9ac58590a35c59c",
    target_json: TARGET,
    receipt: "core_meta.legacy_discount_packet_upgrades",
};
fn captured() -> Result<(), UpgradeError> {
    let target: serde_json::Value =
        serde_json::from_str(TARGET).map_err(|_| UpgradeError::Storage)?;
    if !target["manifest_sha256"].as_str().is_some_and(digest_hex)
        || !target["catalog_sha256"].as_str().is_some_and(digest_hex)
    {
        return Err(reject("LEGACY_DISCOUNT_PACKET_TARGET_NOT_CAPTURED"));
    }
    Ok(())
}
pub(super) fn validate(r: &UpgradeRequest) -> Result<(), UpgradeError> {
    captured()?;
    ENGINE.validate(r)
}
pub(super) fn authorize_target() -> Result<(), UpgradeError> {
    captured()?;
    if COMPONENTS.get(80).map(|v| v.0) != Some("legacy-discount-packet-continuation") {
        return Err(reject("LEGACY_DISCOUNT_PACKET_COMPONENT_CHANGED"));
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
    let sql = include_str!("../../../schema/legacy_discount_packet_grants.sql")
        .replace(":\"runtime_role\"", &quote(role));
    sqlx::raw_sql(sqlx::AssertSqlSafe(sql)).execute(c).await?;
    Ok(())
}

#[cfg(test)]
#[path = "legacy_discount_packet_tests.rs"]
mod tests;

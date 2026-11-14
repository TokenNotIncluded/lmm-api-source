//! Explicit Core70 -> Core71. Attribution only; no financial reward replay.
use super::*;
pub(super) const PROCEDURE: &str = "core-affiliate-registration-v1";
const TARGET: &str = include_str!("affiliate_registration_target.json");
const ENGINE: suffix::Suffix = suffix::Suffix {
    hooks: &registry::AFFILIATE_REGISTRATION,
    procedure: PROCEDURE,
    predecessor_count: 70,
    target_count: 71,
    predecessor_manifest: "ab6af1eeb4272142213545589c5cc40e56af75cc0083b0f886b95c540fe9d81d",
    predecessor_catalog: "bb78a6b036addd3fc86b18c0052498bb54929fcaad69197d751c8e817c22b3fa",
    target_json: TARGET,
    receipt: "core_meta.affiliate_registration_upgrades",
};
pub(super) fn validate(r: &UpgradeRequest) -> Result<(), UpgradeError> {
    ENGINE.validate(r)
}
pub(super) fn authorize_target() -> Result<(), UpgradeError> {
    if COMPONENTS.get(70).map(|c| c.0) != Some("affiliate-registration") {
        return Err(reject("AFFILIATE_REGISTRATION_COMPONENT_CHANGED"));
    }
    ENGINE.authorize_target()
}
pub(super) async fn connected(
    c: &mut PgConnection,
    r: &UpgradeRequest,
) -> Result<UpgradeReport, UpgradeError> {
    ENGINE.connected(c, r).await
}
pub(super) async fn grant_runtime(c: &mut PgConnection, role: &str) -> Result<(), UpgradeError> {
    let grants = include_str!("../../../schema/affiliate_registration_grants.sql")
        .replace(":\"runtime_role\"", &quote(role));
    sqlx::raw_sql(sqlx::AssertSqlSafe::<&str>(&grants))
        .execute(c)
        .await?;
    Ok(())
}

#[cfg(test)]
#[path = "affiliate_registration_tests.rs"]
mod tests;

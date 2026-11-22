//! Exact Core85 -> Core86. Retain legacy invitation links without moving funds.
use super::*;
pub(super) const PROCEDURE: &str = "core-legacy-affiliate-aliases-v1";
const TARGET: &str = include_str!("legacy_affiliate_aliases_target.json");
const ENGINE: suffix::Suffix = suffix::Suffix {
    hooks: &registry::LEGACY_AFFILIATE_ALIASES,
    procedure: PROCEDURE,
    predecessor_count: 85,
    target_count: 86,
    predecessor_manifest: "2c4563c35a29fe5c0d579c5d0ea41e9300a3027343fbae641ad098d5524e8d80",
    predecessor_catalog: "1890e9f888a13b535dd653bffe5771745c6e4e44421211ae6723fcffedd64462",
    target_json: TARGET,
    receipt: "core_meta.legacy_affiliate_alias_upgrades",
};
pub(super) fn validate(r: &UpgradeRequest) -> Result<(), UpgradeError> {
    ENGINE.validate(r)
}
pub(super) fn authorize_target() -> Result<(), UpgradeError> {
    if COMPONENTS.get(85).map(|v| v.0) != Some("legacy-affiliate-aliases") {
        return Err(reject("LEGACY_AFFILIATE_ALIASES_COMPONENT_CHANGED"));
    }
    ENGINE.authorize_target()
}
pub(super) async fn connected(
    c: &mut PgConnection,
    r: &UpgradeRequest,
) -> Result<UpgradeReport, UpgradeError> {
    ENGINE.connected(c, r).await
}

// Only an existing registration reader receives the public code and owner.
// Source rows, import material, financial authority and receipts stay private.
pub(super) async fn grant_runtime(c: &mut PgConnection, role: &str) -> Result<(), UpgradeError> {
    let enabled: bool = sqlx::query_scalar(
        "SELECT has_schema_privilege($1,'core_identity','USAGE')
          AND has_table_privilege($1,'core_identity.affiliate_codes','SELECT')
          AND has_table_privilege($1,'core_identity.affiliate_registrations','INSERT')",
    )
    .bind(role)
    .fetch_one(&mut *c)
    .await?;
    if enabled {
        let grants = include_str!("../../../schema/legacy_affiliate_alias_grants.sql")
            .replace(":\"runtime_role\"", &quote(role));
        sqlx::raw_sql(sqlx::AssertSqlSafe::<&str>(&grants))
            .execute(c)
            .await?;
    }
    Ok(())
}

#[cfg(test)]
#[path = "legacy_affiliate_aliases_tests.rs"]
mod tests;

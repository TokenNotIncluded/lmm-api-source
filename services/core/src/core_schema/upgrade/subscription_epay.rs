//! Explicit Core58 -> Core59. No payment replay or balance reconstruction.
use super::*;
pub(super) const PROCEDURE: &str = "core-subscription-epay-v1";
const TARGET: &str = include_str!("subscription_epay_target.json");
const ENGINE: suffix::Suffix = suffix::Suffix {
    hooks: &registry::SUBSCRIPTION_EPAY,
    procedure: PROCEDURE,
    predecessor_count: 58,
    target_count: 59,
    predecessor_manifest: "02a013915ad4b77dd4cd4006ff62c07653e474afe690b6927892c10168ab400a",
    predecessor_catalog: "e4ca2e9475c7513fbc551dcc78fdfed2f5f792ce98961620c329dbf74836b55b",
    target_json: TARGET,
    receipt: "core_meta.subscription_epay_upgrades",
};
pub(super) fn validate(r: &UpgradeRequest) -> Result<(), UpgradeError> {
    ENGINE.validate(r)
}
pub(super) fn authorize_target() -> Result<(), UpgradeError> {
    if COMPONENTS.get(58).map(|c| c.0) != Some("subscription-epay") {
        return Err(reject("SUBSCRIPTION_EPAY_COMPONENT_CHANGED"));
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
    let grants = include_str!("../../../schema/subscription_epay_grants.sql")
        .replace(":\"runtime_role\"", &quote(role));
    sqlx::raw_sql(sqlx::AssertSqlSafe::<&str>(&grants))
        .execute(c)
        .await?;
    Ok(())
}

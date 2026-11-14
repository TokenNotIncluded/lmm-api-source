//! Explicit 73 -> 74. Target hashes require capture on isolated PostgreSQL 18.6;
//! uncaptured targets reject before connecting, rather than accepting caller SQL.
use super::*;
pub(super) const PROCEDURE: &str = "core-subscription-creem-v1";
const TARGET: &str = include_str!("subscription_creem_target.json");
const ENGINE: suffix::Suffix = suffix::Suffix {
    hooks: &registry::SUBSCRIPTION_CREEM,
    procedure: PROCEDURE,
    predecessor_count: 73,
    target_count: 74,
    predecessor_manifest: "17366890c80d85761a0121a3979e6125786d0253b69c67ea0fd5136e3aed6781",
    predecessor_catalog: "59d19c7fc8b5522ed529d8584321c7dc493bcc9f6b21f0ff586c2e4cb6a99c83",
    target_json: TARGET,
    receipt: "core_meta.subscription_creem_upgrades",
};
fn captured() -> Result<(), UpgradeError> {
    let target: serde_json::Value =
        serde_json::from_str(TARGET).map_err(|_| UpgradeError::Storage)?;
    if !target["manifest_sha256"].as_str().is_some_and(digest_hex)
        || !target["catalog_sha256"].as_str().is_some_and(digest_hex)
    {
        return Err(reject("SUBSCRIPTION_CREEM_TARGET_NOT_CAPTURED"));
    }
    Ok(())
}
pub(super) fn validate(r: &UpgradeRequest) -> Result<(), UpgradeError> {
    captured()?;
    ENGINE.validate(r)
}
pub(super) fn authorize_target() -> Result<(), UpgradeError> {
    captured()?;
    if COMPONENTS.get(73).map(|v| v.0) != Some("subscription-creem") {
        return Err(reject("SUBSCRIPTION_CREEM_COMPONENT_CHANGED"));
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
        include_str!("../../../schema/subscription_creem_grants.sql")
            .replace(":\"runtime_role\"", &quote(runtime)),
    ))
    .execute(c)
    .await?;
    Ok(())
}

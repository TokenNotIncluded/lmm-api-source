//! Explicit 71 -> 72. Target hashes require capture on isolated PostgreSQL 18.6;
//! uncaptured targets reject before connecting, rather than accepting caller SQL.
use super::*;
pub(super) const PROCEDURE: &str = "core-oauth2-oidc-v1";
const TARGET: &str = include_str!("oidc_target.json");
const ENGINE: suffix::Suffix = suffix::Suffix {
    hooks: &registry::OIDC,
    procedure: PROCEDURE,
    predecessor_count: 71,
    target_count: 72,
    predecessor_manifest: "cf78e11ffe80989ca629a41e46f9abefcb56bd13dbd5d1638a174e7815e4d44e",
    predecessor_catalog: "8bad3ff1e32703253a3b56e1a39913c9301274a603a17f8905d0988fd25d2e6f",
    target_json: TARGET,
    receipt: "core_meta.oauth2_oidc_upgrades",
};
fn captured() -> Result<(), UpgradeError> {
    let target: serde_json::Value =
        serde_json::from_str(TARGET).map_err(|_| UpgradeError::Storage)?;
    if !target["manifest_sha256"].as_str().is_some_and(digest_hex)
        || !target["catalog_sha256"].as_str().is_some_and(digest_hex)
    {
        return Err(reject("OIDC_TARGET_NOT_CAPTURED"));
    }
    Ok(())
}
pub(super) fn validate(r: &UpgradeRequest) -> Result<(), UpgradeError> {
    captured()?;
    ENGINE.validate(r)
}
pub(super) fn authorize_target() -> Result<(), UpgradeError> {
    captured()?;
    if COMPONENTS.get(71).map(|v| v.0) != Some("oauth2-oidc") {
        return Err(reject("OIDC_COMPONENT_CHANGED"));
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
        include_str!("../../../schema/oauth2_oidc_grants.sql")
            .replace(":\"runtime_role\"", &quote(runtime)),
    ))
    .execute(c)
    .await?;
    Ok(())
}

#[cfg(test)]
#[path = "oidc_tests.rs"]
mod tests;

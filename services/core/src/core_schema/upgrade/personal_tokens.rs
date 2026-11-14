//! Explicit 72 -> 73. Target hashes require capture on isolated PostgreSQL 18.6;
//! uncaptured targets reject before connecting, rather than accepting caller SQL.
use super::*;
pub(super) const PROCEDURE: &str = "core-personal-tokens-v1";
const TARGET: &str = include_str!("personal_tokens_target.json");
const ENGINE: suffix::Suffix = suffix::Suffix {
    hooks: &registry::PERSONAL_TOKENS,
    procedure: PROCEDURE,
    predecessor_count: 72,
    target_count: 73,
    predecessor_manifest: "0b2dc83b6a436b3da27a549ecd7d246588c479db31601ae7bb818be7c3de49d5",
    predecessor_catalog: "61c6fca5f00edec723277471818150f4f647321dc68f3deb03dba0d61d86d180",
    target_json: TARGET,
    receipt: "core_meta.personal_tokens_upgrades",
};
fn captured() -> Result<(), UpgradeError> {
    let target: serde_json::Value =
        serde_json::from_str(TARGET).map_err(|_| UpgradeError::Storage)?;
    if !target["manifest_sha256"].as_str().is_some_and(digest_hex)
        || !target["catalog_sha256"].as_str().is_some_and(digest_hex)
    {
        return Err(reject("PERSONAL_TOKENS_TARGET_NOT_CAPTURED"));
    }
    Ok(())
}
pub(super) fn validate(r: &UpgradeRequest) -> Result<(), UpgradeError> {
    captured()?;
    ENGINE.validate(r)
}
pub(super) fn authorize_target() -> Result<(), UpgradeError> {
    captured()?;
    if COMPONENTS.get(72).map(|v| v.0) != Some("personal-tokens") {
        return Err(reject("PERSONAL_TOKENS_COMPONENT_CHANGED"));
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
        include_str!("../../identity/personal_token/grants.sql")
            .replace(":\"runtime_role\"", &quote(runtime)),
    ))
    .execute(c)
    .await?;
    Ok(())
}

#[cfg(test)]
#[path = "personal_tokens_tests.rs"]
mod tests;

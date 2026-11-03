//! Explicit, offline 46 -> 47. No legacy probe backfill or financial mutations.
use super::*;
pub(super) const PROCEDURE: &str = "core-navigation-options-v1";
const PRE_MANIFEST: &str = "7e69eafaa516a67dfc1b719ae2f5a950a90596f912f47d187422c470f9c170ee";
const PRE_CATALOG: &str = "c1b305360f39946be511368c8aba21635158dc767ca65eaab380784673a5d9a6";
const TARGET: &str = include_str!("navigation_options_target.json");
const ENGINE: suffix::Suffix = suffix::Suffix {
    procedure: PROCEDURE,
    predecessor_count: 46,
    target_count: 47,
    predecessor_manifest: PRE_MANIFEST,
    predecessor_catalog: PRE_CATALOG,
    target_json: TARGET,
    receipt: "core_meta.navigation_options_upgrades",
};
fn captured() -> Result<(), UpgradeError> {
    let target: serde_json::Value =
        serde_json::from_str(TARGET).map_err(|_| UpgradeError::Storage)?;
    if !target["catalog_sha256"].as_str().is_some_and(digest_hex) {
        return Err(reject("NAVIGATION_OPTIONS_TARGET_NOT_CAPTURED"));
    }
    Ok(())
}
pub(super) fn validate(request: &UpgradeRequest) -> Result<(), UpgradeError> {
    captured()?;
    ENGINE.validate(request)
}
pub(super) fn authorize_target() -> Result<(), UpgradeError> {
    captured()?;
    if COMPONENTS.get(46).map(|c| c.0) != Some("navigation-options") {
        return Err(reject("NAVIGATION_OPTIONS_COMPILED_COMPONENTS_CHANGED"));
    }
    ENGINE.authorize_target()
}
pub(super) async fn connected(
    c: &mut PgConnection,
    request: &UpgradeRequest,
) -> Result<UpgradeReport, UpgradeError> {
    captured()?;
    ENGINE.connected(c, request).await
}

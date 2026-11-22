//! Exact Core81 -> Core82; preserve the original service and financial state.
use super::*;
pub(super) const PROCEDURE: &str = "core-assistant-service-funding-v1";
const TARGET: &str = include_str!("assistant_service_funding_target.json");
const ENGINE: suffix::Suffix = suffix::Suffix {
    hooks: &registry::ASSISTANT_SERVICE_FUNDING,
    procedure: PROCEDURE,
    predecessor_count: 81,
    target_count: 82,
    predecessor_manifest: "9aa883b8065bc042d15ad396856d5522bfabe022fea82c2e5b2226ee2060f329",
    predecessor_catalog: "44836b3500c3fb92519e13030a3622726856cdb6b5d32a284f730b04069f8acf",
    target_json: TARGET,
    receipt: "core_meta.assistant_service_funding_upgrades",
};
fn captured() -> Result<(), UpgradeError> {
    let target: serde_json::Value =
        serde_json::from_str(TARGET).map_err(|_| UpgradeError::Storage)?;
    if !target["manifest_sha256"].as_str().is_some_and(digest_hex)
        || !target["catalog_sha256"].as_str().is_some_and(digest_hex)
    {
        return Err(reject("ASSISTANT_SERVICE_FUNDING_TARGET_NOT_CAPTURED"));
    }
    Ok(())
}
pub(super) fn validate(r: &UpgradeRequest) -> Result<(), UpgradeError> {
    captured()?;
    ENGINE.validate(r)
}
pub(super) fn authorize_target() -> Result<(), UpgradeError> {
    captured()?;
    if COMPONENTS.get(81).map(|v| v.0) != Some("assistant-service-funding") {
        return Err(reject("ASSISTANT_SERVICE_FUNDING_COMPONENT_CHANGED"));
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

#[cfg(test)]
#[path = "assistant_service_funding_tests.rs"]
mod tests;

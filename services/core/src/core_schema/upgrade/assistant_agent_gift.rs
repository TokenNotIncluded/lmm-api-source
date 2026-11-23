//! Exact Core82 -> Core83; preserve the original service and financial state.
use super::*;
pub(super) const PROCEDURE: &str = "core-assistant-agent-gift-v1";
const TARGET: &str = include_str!("assistant_agent_gift_target.json");
const ENGINE: suffix::Suffix = suffix::Suffix {
    hooks: &registry::ASSISTANT_AGENT_GIFT,
    procedure: PROCEDURE,
    predecessor_count: 82,
    target_count: 83,
    predecessor_manifest: "48d4601d3777c21c6380c15806ff7fd012ef2c9392a4662ac78d197da3de5e67",
    predecessor_catalog: "6e05b4b12782f500818737a38c64e153fcfbf6f7eb98a93517210c9646573deb",
    target_json: TARGET,
    receipt: "core_meta.assistant_agent_gift_upgrades",
};
fn captured() -> Result<(), UpgradeError> {
    let target: serde_json::Value =
        serde_json::from_str(TARGET).map_err(|_| UpgradeError::Storage)?;
    if !target["manifest_sha256"].as_str().is_some_and(digest_hex)
        || !target["catalog_sha256"].as_str().is_some_and(digest_hex)
    {
        return Err(reject("ASSISTANT_AGENT_GIFT_TARGET_NOT_CAPTURED"));
    }
    Ok(())
}
pub(super) fn validate(r: &UpgradeRequest) -> Result<(), UpgradeError> {
    captured()?;
    ENGINE.validate(r)
}
pub(super) fn authorize_target() -> Result<(), UpgradeError> {
    captured()?;
    if COMPONENTS.get(82).map(|v| v.0) != Some("assistant-agent-gift") {
        return Err(reject("ASSISTANT_AGENT_GIFT_COMPONENT_CHANGED"));
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
#[path = "assistant_agent_gift_tests.rs"]
mod tests;

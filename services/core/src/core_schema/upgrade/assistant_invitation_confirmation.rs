//! Exact Core86 -> Core87. Enable the existing invitation browser confirmation.
use super::*;
pub(super) const PROCEDURE: &str = "core-assistant-invitation-confirmation-v1";
const TARGET: &str = include_str!("assistant_invitation_confirmation_target.json");
const ENGINE: suffix::Suffix = suffix::Suffix {
    hooks: &registry::ASSISTANT_INVITATION_CONFIRMATION,
    procedure: PROCEDURE,
    predecessor_count: 86,
    target_count: 87,
    predecessor_manifest: "ddd230cfd5c336d0f8f9245932b840db8d88b6c422308360bf6a0b06473a899b",
    predecessor_catalog: "1936d5baa2a16385260975aa7b995609f8c9636759fe36ba3a51c7e7d3396828",
    target_json: TARGET,
    receipt: "core_meta.assistant_invitation_confirmation_upgrades",
};
pub(super) fn validate(r: &UpgradeRequest) -> Result<(), UpgradeError> {
    ENGINE.validate(r)
}
pub(super) fn authorize_target() -> Result<(), UpgradeError> {
    if COMPONENTS.get(86).map(|v| v.0) != Some("assistant-invitation-confirmation") {
        return Err(reject(
            "ASSISTANT_INVITATION_CONFIRMATION_COMPONENT_CHANGED",
        ));
    }
    ENGINE.authorize_target()
}
pub(super) async fn connected(
    c: &mut PgConnection,
    r: &UpgradeRequest,
) -> Result<UpgradeReport, UpgradeError> {
    ENGINE.connected(c, r).await
}
#[cfg(test)]
#[path = "assistant_invitation_confirmation_tests.rs"]
mod tests;

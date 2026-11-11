//! Exact Core66 -> Core67. Only checkout language semantics change.
use super::*;
pub(super) const PROCEDURE: &str = "core-subscription-pancake-language-optional-v1";
const TARGET: &str = include_str!("subscription_pancake_language_target.json");
const ENGINE: suffix::Suffix = suffix::Suffix {
    procedure: PROCEDURE,
    predecessor_count: 66,
    target_count: 67,
    predecessor_manifest: "424ab67e21d21b7147b01e5f438a1d5938fa16ad9bc464f05d3aa8223e487d9f",
    predecessor_catalog: "be0b981025782d501487bcc8b9cd9c59bccf9cae2351c77708a1c1ce16b0358e",
    target_json: TARGET,
    receipt: "core_meta.pancake_language_upgrades",
};
pub(super) fn validate(r: &UpgradeRequest) -> Result<(), UpgradeError> {
    ENGINE.validate(r)
}
pub(super) fn authorize_target() -> Result<(), UpgradeError> {
    if COMPONENTS.get(66).map(|c| c.0) != Some("subscription-pancake-language-optional") {
        return Err(reject("PANCAKE_LANGUAGE_COMPONENT_CHANGED"));
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
mod tests {
    use super::*;
    #[test]
    fn predecessor66_is_frozen_and_business_delta_only_changes_language() {
        let mut manifest = schema_manifest();
        manifest.components.truncate(66);
        let mut bytes = serde_json::to_vec(&manifest).unwrap();
        bytes.push(b'\n');
        assert_eq!(hex(&Sha256::digest(bytes)), ENGINE.predecessor_manifest);
        assert_eq!(COMPONENTS.len(), 67);
        let sql = COMPONENTS[66].1;
        assert!(sql.contains("CHECK(length(checkout_language) <= 32)"));
        assert!(sql.contains("CREATE TABLE core_meta.pancake_language_upgrades"));
        assert!(!sql.contains("CREATE TABLE core_subscriptions"));
    }
}

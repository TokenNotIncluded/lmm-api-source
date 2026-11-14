//! Explicit offline Core56 -> Core58: funded self-purchases and display FX.
//! Existing economic history and runtime authority are preserved by Suffix.
use super::*;

pub(super) const PROCEDURE: &str = "core-commerce-display-rate-v1";
const PRE_MANIFEST: &str = "f2340fb850e81f168ababe0301d80bd85769b235a8c5b5abb91f0bcd7429624e";
const PRE_CATALOG: &str = "46cbd8e0a9e9bb6c55cb899558b40fa8c8df1affa2e366cbbde733a8527bda43";
const TARGET: &str = include_str!("commerce_display_rate_target.json");
const ENGINE: suffix::Suffix = suffix::Suffix {
    hooks: &registry::COMMERCE_DISPLAY_RATE,
    procedure: PROCEDURE,
    predecessor_count: 56,
    target_count: 58,
    predecessor_manifest: PRE_MANIFEST,
    predecessor_catalog: PRE_CATALOG,
    target_json: TARGET,
    receipt: "core_meta.commerce_display_rate_upgrades",
};

pub(super) fn validate(request: &UpgradeRequest) -> Result<(), UpgradeError> {
    ENGINE.validate(request)
}
pub(super) fn authorize_target() -> Result<(), UpgradeError> {
    if COMPONENTS.get(56).map(|c| c.0) != Some("commerce-self-purchase")
        || COMPONENTS.get(57).map(|c| c.0) != Some("display-exchange-rate")
    {
        return Err(reject("COMMERCE_DISPLAY_RATE_COMPILED_COMPONENTS_CHANGED"));
    }
    ENGINE.authorize_target()
}
pub(super) async fn connected(
    c: &mut PgConnection,
    request: &UpgradeRequest,
) -> Result<UpgradeReport, UpgradeError> {
    ENGINE.connected(c, request).await
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn fresh_install_binary_refuses_preserved_commerce_upgrade() {
        let mut manifest = schema_manifest();
        manifest.components.truncate(56);
        let mut bytes = serde_json::to_vec(&manifest).unwrap();
        bytes.push(b'\n');
        assert_ne!(hex(Sha256::digest(bytes)), PRE_MANIFEST);
        let target: serde_json::Value = serde_json::from_str(TARGET).unwrap();
        assert_ne!(target["manifest_sha256"], ENGINE.target_manifest().unwrap());
        assert!(matches!(
            authorize_target(),
            Err(UpgradeError::Rejected(
                "COMMERCE_DISPLAY_RATE_COMPILED_COMPONENTS_CHANGED"
            ))
        ));
    }
}

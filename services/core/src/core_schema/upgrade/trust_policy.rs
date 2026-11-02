//! Exact frozen 44-component predecessor to the explicit policy-editor successor.
use super::*;
pub(super) const PROCEDURE: &str = "core-trust-policy-edit-v1";
const PRE_MANIFEST: &str = "5f6b75aaae3110c460c58270aac77028d8d4c32360481a52831872515131261d";
const PRE_CATALOG: &str = "d4cb2dbfd24de133c92dbd8128ea691d50dc93f72c4753ec995135d081b8f584";
const TARGET: &str = include_str!("trust_policy_target.json");
const ENGINE: suffix::Suffix = suffix::Suffix {
    procedure: PROCEDURE,
    predecessor_count: 44,
    target_count: 45,
    predecessor_manifest: PRE_MANIFEST,
    predecessor_catalog: PRE_CATALOG,
    target_json: TARGET,
    receipt: "core_meta.trust_policy_upgrades",
};
pub(super) fn validate(r: &UpgradeRequest) -> Result<(), UpgradeError> {
    ENGINE.validate(r)
}
pub(super) fn authorize_target() -> Result<(), UpgradeError> {
    if COMPONENTS.get(44).map(|component| component.0) != Some("trust-policy-edit") {
        return Err(reject("TRUST_POLICY_EDIT_COMPILED_COMPONENTS_CHANGED"));
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
    fn editor_upgrade_does_not_reinterpret_frozen_44_successors() {
        let prior: serde_json::Value =
            serde_json::from_str(include_str!("channel_projection_target.json")).unwrap();
        assert_eq!(prior["manifest_sha256"], PRE_MANIFEST);
        assert_eq!(prior["catalog_sha256"], PRE_CATALOG);
        assert_eq!(ENGINE.predecessor_count, 44);
        assert_eq!(ENGINE.target_count, 45);
        authorize_target().unwrap();
    }
}

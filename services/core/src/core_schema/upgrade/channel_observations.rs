//! Explicit, offline 45 -> 46. No legacy probe backfill or financial mutations.
use super::*;
pub(super) const PROCEDURE: &str = "core-channel-observations-v1";
const PRE_MANIFEST: &str = "567d84fa06adf6916a62b6a5de0ae4234da1c0876972254f8393f0a55cd8d9c8";
const PRE_CATALOG: &str = "b3e3db14753bcb575eba90541b09cf9fac2955a22e513741a91b1f11019dbe6f";
const TARGET: &str = include_str!("channel_observations_target.json");
const ENGINE: suffix::Suffix = suffix::Suffix {
    procedure: PROCEDURE,
    predecessor_count: 45,
    target_count: 46,
    predecessor_manifest: PRE_MANIFEST,
    predecessor_catalog: PRE_CATALOG,
    target_json: TARGET,
    receipt: "core_meta.channel_observations_upgrades",
};
fn captured() -> Result<(), UpgradeError> {
    let target: serde_json::Value =
        serde_json::from_str(TARGET).map_err(|_| UpgradeError::Storage)?;
    if !target["catalog_sha256"].as_str().is_some_and(digest_hex) {
        return Err(reject("CHANNEL_OBSERVATIONS_TARGET_NOT_CAPTURED"));
    }
    Ok(())
}
pub(super) fn validate(request: &UpgradeRequest) -> Result<(), UpgradeError> {
    captured()?;
    ENGINE.validate(request)
}
pub(super) fn authorize_target() -> Result<(), UpgradeError> {
    captured()?;
    if COMPONENTS.get(45).map(|c| c.0) != Some("channel-observations") {
        return Err(reject("CHANNEL_OBSERVATIONS_COMPILED_COMPONENTS_CHANGED"));
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
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn predecessor_and_old_successors_remain_frozen() {
        assert_eq!(
            hex(&Sha256::digest(include_bytes!(
                "observations_predecessor_manifest.json"
            ))),
            PRE_MANIFEST
        );
        let prior: serde_json::Value =
            serde_json::from_str(include_str!("trust_policy_target.json")).unwrap();
        assert_eq!(prior["manifest_sha256"], PRE_MANIFEST);
        assert_eq!(prior["catalog_sha256"], PRE_CATALOG);
        let new: serde_json::Value = serde_json::from_str(TARGET).unwrap();
        assert_eq!(new["manifest_sha256"], ENGINE.target_manifest().unwrap());
        let mut manifest = schema_manifest();
        manifest.components.truncate(45);
        let mut bytes = serde_json::to_vec(&manifest).unwrap();
        bytes.push(b'\n');
        assert_eq!(hex(&Sha256::digest(bytes)), PRE_MANIFEST);
        for old in [
            include_str!("completion_target.json"),
            include_str!("account_appeals_target.json"),
            include_str!("channel_accounting_target.json"),
        ] {
            assert_eq!(old, include_str!("channel_projection_target.json"));
        }
        super::super::trust_policy::authorize_target().unwrap();
        super::super::channel_projection::authorize_target().unwrap();
        if new["catalog_sha256"].is_null() {
            assert!(captured().is_err());
        } else {
            authorize_target().unwrap();
        }
    }
}

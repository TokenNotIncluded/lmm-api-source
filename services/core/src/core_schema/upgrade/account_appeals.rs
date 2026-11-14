//! Exact frozen 41-component predecessor to the explicit 44-component successor.
use super::*;
pub(super) const PROCEDURE: &str = "core-account-appeals-v1";
const PRE_MANIFEST: &str = "12c971ce84ca71eb5605a7fe284535216a5777858d5aaf766c3876655bf8aee6";
const PRE_CATALOG: &str = "05d1aa6913236fc12cc099640ead1ac2d88a1bde073923ce8353907cfc46c8bb";
const TARGET: &str = include_str!("account_appeals_target.json");
const ENGINE: suffix::Suffix = suffix::Suffix {
    hooks: &registry::ACCOUNT_APPEALS,
    procedure: PROCEDURE,
    predecessor_count: 41,
    target_count: 44,
    predecessor_manifest: PRE_MANIFEST,
    predecessor_catalog: PRE_CATALOG,
    target_json: TARGET,
    receipt: "core_meta.account_appeal_upgrades",
};
pub(super) fn validate(r: &UpgradeRequest) -> Result<(), UpgradeError> {
    ENGINE.validate(r)
}
pub(super) fn authorize_target() -> Result<(), UpgradeError> {
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

    fn request() -> UpgradeRequest {
        UpgradeRequest {
            version: 1,
            procedure: PROCEDURE.into(),
            upgrade_id: "synthetic-account-appeals-upgrade".into(),
            database: UpgradeDatabase {
                name: "synthetic_appeals".into(),
                oid: 1,
                system_identifier: "1".into(),
                server_version_num: 180006,
            },
            roles: UpgradeRoles {
                installer: "appeals_installer".into(),
                owner: "appeals_owner".into(),
                runtime: "appeals_runtime".into(),
                extensions: "appeals_extensions".into(),
            },
            predecessor_manifest_sha256: PRE_MANIFEST.into(),
            predecessor_catalog_sha256: PRE_CATALOG.into(),
            successor_manifest_sha256: ENGINE.target_manifest().unwrap(),
        }
    }

    #[test]
    fn frozen_41_manifest_and_catalog_remain_exact() {
        let frozen = include_str!("appeals_predecessor_manifest.json");
        assert_eq!(hex(Sha256::digest(frozen.as_bytes())), PRE_MANIFEST);
        let mut compiled: super::super::historical_fixture::Manifest =
            serde_json::from_str(include_str!("appeals_predecessor_manifest.json")).unwrap();
        assert_eq!(ENGINE.target_count, 44);
        let target: serde_json::Value = serde_json::from_str(TARGET).unwrap();
        assert_ne!(ENGINE.target_manifest().unwrap(), target["manifest_sha256"]);
        assert_eq!(schema_manifest().components[41].name, "account-appeals");
        compiled.components.truncate(41);
        let mut bytes = serde_json::to_vec(&compiled).unwrap();
        bytes.push(b'\n');
        assert_eq!(bytes, frozen.as_bytes());
        let catalog: serde_json::Value =
            serde_json::from_str(include_str!("appeals_predecessor_target.json")).unwrap();
        assert_eq!(catalog["manifest_sha256"], PRE_MANIFEST);
        assert_eq!(catalog["catalog_sha256"], PRE_CATALOG);
        // The old 32-component predecessor is still frozen independently.
        compiled.components.truncate(32);
        let mut bytes = serde_json::to_vec(&compiled).unwrap();
        bytes.push(b'\n');
        assert_eq!(
            hex(Sha256::digest(bytes)),
            "9c9726a7fa204a150d9fe18139d4580d23feb4cec40145fcb829338331a4cd15"
        );
    }

    #[test]
    fn account_appeals_request_rejects_wrong_predecessor_roles_and_server() {
        let valid = request();
        validate(&valid).unwrap();
        for case in 0..7 {
            let mut wrong = valid.clone();
            match case {
                0 => wrong.predecessor_manifest_sha256 = "0".repeat(64),
                1 => wrong.predecessor_catalog_sha256 = "0".repeat(64),
                2 => wrong.successor_manifest_sha256 = "0".repeat(64),
                3 => wrong.database.server_version_num = 170006,
                4 => wrong.roles.runtime = wrong.roles.owner.clone(),
                5 => wrong.database.oid = 0,
                _ => wrong.procedure = "untrusted".into(),
            }
            assert!(validate(&wrong).is_err(), "case {case}");
        }
    }

    #[test]
    fn both_upgrade_paths_bind_the_same_measured_release_target() {
        // Both preserved upgrade paths refuse the fresh-install binary.
        assert!(matches!(
            authorize_target(),
            Err(UpgradeError::Rejected(
                "ACCOUNT_APPEALS_COMPILED_PREDECESSOR_CHANGED"
            ))
        ));
        assert!(matches!(
            super::super::completion::authorize_target(),
            Err(UpgradeError::Rejected(
                "COMPLETION_COMPILED_PREDECESSOR_CHANGED"
            ))
        ));
        assert_eq!(TARGET, include_str!("completion_target.json"));
        assert!(matches!(
            UpgradeRequest::read(serde_json::to_vec(&request()).unwrap().as_slice()),
            Err(UpgradeError::Rejected(
                "ACCOUNT_APPEALS_COMPILED_PREDECESSOR_CHANGED"
            ))
        ));
    }
}

//! Exact frozen 42-component predecessor to the explicit 43-component successor.
use super::*;
pub(super) const PROCEDURE: &str = "core-channel-accounting-v1";
const PRE_MANIFEST: &str = "cbc4b9567d9754250afb1646324e61fc13c9368d69e72c5506fa692ad169cd8f";
const PRE_CATALOG: &str = "e1c5e00088abac46cdec398830bd275945f9e269d2aef33bbe6fd1982c227076";
const TARGET: &str = include_str!("channel_accounting_target.json");
const ENGINE: suffix::Suffix = suffix::Suffix {
    procedure: PROCEDURE,
    predecessor_count: 42,
    predecessor_manifest: PRE_MANIFEST,
    predecessor_catalog: PRE_CATALOG,
    target_json: TARGET,
    receipt: "core_meta.channel_accounting_upgrades",
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
            upgrade_id: "synthetic-channel-accounting-upgrade".into(),
            database: UpgradeDatabase {
                name: "synthetic_accounting".into(),
                oid: 1,
                system_identifier: "1".into(),
                server_version_num: 180006,
            },
            roles: UpgradeRoles {
                installer: "accounting_installer".into(),
                owner: "accounting_owner".into(),
                runtime: "accounting_runtime".into(),
                extensions: "accounting_extensions".into(),
            },
            predecessor_manifest_sha256: PRE_MANIFEST.into(),
            predecessor_catalog_sha256: PRE_CATALOG.into(),
            successor_manifest_sha256: manifest_sha256().unwrap(),
        }
    }
    #[test]
    fn frozen42_is_exact_and_all_paths_have_the_same_explicit43_target() {
        let frozen = include_str!("accounting_predecessor_manifest.json");
        assert_eq!(hex(&Sha256::digest(frozen.as_bytes())), PRE_MANIFEST);
        let mut compiled = schema_manifest();
        assert_eq!(compiled.components.len(), 43);
        compiled.components.truncate(42);
        let mut bytes = serde_json::to_vec(&compiled).unwrap();
        bytes.push(b'\n');
        assert_eq!(bytes, frozen.as_bytes());
        let target: serde_json::Value =
            serde_json::from_str(include_str!("accounting_predecessor_target.json")).unwrap();
        assert_eq!(target["catalog_sha256"], PRE_CATALOG);
        assert_eq!(TARGET, include_str!("account_appeals_target.json"));
        assert_eq!(TARGET, include_str!("completion_target.json"));
        // Fail closed until the real private PG18 catalog capture completes.
        authorize_target().unwrap();
    }
    #[test]
    fn accounting_requires_explicit_successor_roles_and_frozen42() {
        let valid = request();
        validate(&valid).unwrap();
        for case in 0..7 {
            let mut wrong = valid.clone();
            match case {
                0 => wrong.successor_manifest_sha256 = PRE_MANIFEST.into(),
                1 => wrong.predecessor_manifest_sha256 = "0".repeat(64),
                2 => wrong.predecessor_catalog_sha256 = "0".repeat(64),
                3 => wrong.roles.runtime = wrong.roles.owner.clone(),
                4 => wrong.database.server_version_num = 170006,
                5 => wrong.procedure = "core-account-appeals-v1".into(),
                _ => wrong.database.oid = 0,
            }
            assert!(validate(&wrong).is_err(), "case {case}");
        }
        let mut old = valid;
        old.procedure = "core-account-appeals-v1".into();
        old.predecessor_manifest_sha256 =
            "12c971ce84ca71eb5605a7fe284535216a5777858d5aaf766c3876655bf8aee6".into();
        old.predecessor_catalog_sha256 =
            "05d1aa6913236fc12cc099640ead1ac2d88a1bde073923ce8353907cfc46c8bb".into();
        old.successor_manifest_sha256 = PRE_MANIFEST.into();
        assert!(super::super::account_appeals::validate(&old).is_err());
    }
}

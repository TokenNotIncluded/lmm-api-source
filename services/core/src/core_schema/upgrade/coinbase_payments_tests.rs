use super::*;

fn request() -> UpgradeRequest {
    UpgradeRequest {
        version: 1,
        procedure: PROCEDURE.into(),
        upgrade_id: "synthetic-coinbase-upgrade".into(),
        database: UpgradeDatabase {
            name: "synthetic_coinbase".into(),
            oid: 1,
            system_identifier: "1".into(),
            server_version_num: 180006,
        },
        roles: UpgradeRoles {
            installer: "synthetic_installer".into(),
            owner: "synthetic_owner".into(),
            runtime: "synthetic_runtime".into(),
            extensions: "synthetic_extensions".into(),
        },
        predecessor_manifest_sha256: PRE_MANIFEST.into(),
        predecessor_catalog_sha256: PRE_CATALOG.into(),
        successor_manifest_sha256: TARGET_MANIFEST.into(),
    }
}

#[test]
fn coinbase_upgrade_binds_exact_predecessor_and_successor() {
    authorize_target().unwrap();
    let valid = request();
    UpgradeRequest::read(serde_json::to_vec(&valid).unwrap().as_slice()).unwrap();
    for case in 0..7 {
        let mut wrong = valid.clone();
        match case {
            0 => wrong.database.oid = 0,
            1 => wrong.roles.runtime = wrong.roles.owner.clone(),
            2 => wrong.predecessor_catalog_sha256 = "0".repeat(64),
            3 => wrong.predecessor_manifest_sha256 = "0".repeat(64),
            4 => wrong.successor_manifest_sha256 = "0".repeat(64),
            5 => wrong.database.server_version_num = 170000,
            _ => wrong.procedure = "untrusted".into(),
        }
        assert!(validate(&wrong).is_err(), "case {case}");
    }
}

#[test]
fn coinbase_upgrade_receipt_only_replays_the_original_binding() {
    let original = request();
    let mut changed = original.clone();
    changed.database.oid += 1;
    assert_ne!(original.digest().unwrap(), changed.digest().unwrap());
    let first = report(&original, "committed").unwrap();
    let retry = report(&original, "already_committed").unwrap();
    assert_eq!(first.request_sha256, retry.request_sha256);
    assert_eq!(retry.components, 32);
    assert_eq!(retry.successor_catalog_sha256, TARGET_CATALOG);
}

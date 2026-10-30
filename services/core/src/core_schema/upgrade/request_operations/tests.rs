use super::*;

fn request() -> UpgradeRequest {
    UpgradeRequest {
        version: 1,
        procedure: PROCEDURE.into(),
        upgrade_id: "synthetic-operations-upgrade".into(),
        database: UpgradeDatabase {
            name: "synthetic_operations".into(),
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
        successor_manifest_sha256: manifest_sha256().unwrap(),
    }
}
#[test]
fn operations_upgrade_binds_old_contract_and_compiled_successor() {
    authorize_target().unwrap();
    let valid = request();
    UpgradeRequest::read(serde_json::to_vec(&valid).unwrap().as_slice()).unwrap();
    for case in 0..5 {
        let mut wrong = valid.clone();
        match case {
            0 => wrong.database.oid = 0,
            1 => wrong.roles.runtime = wrong.roles.owner.clone(),
            2 => wrong.predecessor_catalog_sha256 = "0".repeat(64),
            3 => wrong.successor_manifest_sha256 = "0".repeat(64),
            _ => wrong.procedure = "untrusted".into(),
        }
        assert!(validate(&wrong).is_err());
    }
    let mut defs = predecessor_definitions().unwrap();
    defs.extend(PARENT_ADDITIONS.iter().map(|s| s.to_string()));
    defs.sort();
    preserves_catalog(&defs).unwrap();
    for index in 0..PARENT_ADDITIONS.len() {
        let mut wrong = defs.clone();
        let at = wrong
            .iter()
            .position(|d| d == PARENT_ADDITIONS[index])
            .unwrap();
        wrong.remove(at);
        assert!(preserves_catalog(&wrong).is_err());
    }
    defs.push("[\"relation\",\"foreign\",\"unexpected\",\"r\"]".into());
    assert!(preserves_catalog(&defs).is_err());
}

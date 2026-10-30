use super::*;

fn request() -> UpgradeRequest {
    UpgradeRequest {
        version: 1,
        procedure: PROCEDURE.into(),
        upgrade_id: "payment-gateways-fixture-001".into(),
        database: UpgradeDatabase {
            name: "isolated_payment_core".into(),
            oid: 12345,
            system_identifier: "1234567890123456789".into(),
            server_version_num: 180006,
        },
        roles: UpgradeRoles {
            installer: "isolated_installer".into(),
            owner: "isolated_owner".into(),
            runtime: "isolated_runtime".into(),
            extensions: "isolated_extensions".into(),
        },
        predecessor_manifest_sha256: PRE_MANIFEST.into(),
        predecessor_catalog_sha256: PRE_CATALOG.into(),
        successor_manifest_sha256: manifest_sha256().unwrap(),
    }
}

#[test]
fn payment_upgrade_authorizes_the_fixed_current_successor_before_legacy_routing_guard() {
    authorize_target().unwrap();
    let good = request();
    let encoded = serde_json::to_vec(&good).unwrap();
    let parsed = UpgradeRequest::read(&encoded[..]).unwrap();
    assert_eq!(parsed.digest().unwrap(), good.digest().unwrap());
    let compiled = target().unwrap();
    assert_eq!(compiled.definitions.len(), 7068);
    assert_eq!(
        compiled.manifest_sha256,
        "48578c69632d80d7d4e41a5fb36c67f365875e8bb67e0695f62e1d688177f530"
    );
    assert_eq!(
        compiled.catalog_sha256,
        "cb9349ca60bafbd49204dc237a88eec5ca10ac4654e654e7c816ea59673bc21e"
    );
    assert_ne!(compiled.catalog_sha256, PRE_CATALOG);
}

#[test]
fn payment_upgrade_rejects_unbound_database_roles_versions_and_fingerprints() {
    let good = request();
    for change in 0..14 {
        let mut bad = good.clone();
        match change {
            0 => bad.version = 2,
            1 => bad.procedure = "core-payment-gateways-v2".into(),
            2 => bad.upgrade_id = "short".into(),
            3 => bad.upgrade_id = "untrusted\nrequest".into(),
            4 => bad.database.name.clear(),
            5 => bad.database.oid = 0,
            6 => bad.database.server_version_num = 170009,
            7 => bad.database.server_version_num = 190000,
            8 => bad.database.system_identifier = "0".into(),
            9 => bad.database.system_identifier = "18446744073709551616".into(),
            10 => bad.roles.installer = bad.roles.runtime.clone(),
            11 => bad.predecessor_manifest_sha256 = "0".repeat(64),
            12 => bad.predecessor_catalog_sha256 = "0".repeat(64),
            _ => bad.successor_manifest_sha256 = "0".repeat(64),
        }
        assert!(
            matches!(
                validate(&bad),
                Err(UpgradeError::Rejected(
                    "PAYMENT_GATEWAYS_UPGRADE_CONTRACT_INVALID"
                ))
            ),
            "case {change}"
        );
    }
    let mut unknown = serde_json::to_value(&good).unwrap();
    unknown["database"]["sql"] = "SELECT true".into();
    assert!(matches!(
        UpgradeRequest::read(&serde_json::to_vec(&unknown).unwrap()[..]),
        Err(UpgradeError::Rejected("UPGRADE_DB_INPUT_INVALID"))
    ));
    let encoded = serde_json::to_string(&good).unwrap();
    let duplicate = encoded.replacen("{", "{\"version\":1,", 1);
    assert!(matches!(
        UpgradeRequest::read(duplicate.as_bytes()),
        Err(UpgradeError::Rejected("UPGRADE_DB_INPUT_INVALID"))
    ));
    assert!(matches!(
        UpgradeRequest::read(&vec![b' '; 16_385][..]),
        Err(UpgradeError::Rejected("UPGRADE_DB_INPUT_TOO_LARGE"))
    ));
}

#[test]
fn payment_receipt_identity_is_stable_only_for_the_original_typed_request() {
    let good = request();
    let digest = good.digest().unwrap();
    let reordered = serde_json::to_vec_pretty(&serde_json::to_value(&good).unwrap()).unwrap();
    assert_eq!(
        UpgradeRequest::read(&reordered[..])
            .unwrap()
            .digest()
            .unwrap(),
        digest
    );
    for change in 0..5 {
        let mut changed = good.clone();
        match change {
            0 => changed.upgrade_id.push('2'),
            1 => changed.database.oid += 1,
            2 => changed.database.system_identifier.push('1'),
            3 => changed.roles.runtime.push('2'),
            _ => changed.database.name.push('2'),
        }
        validate(&changed).unwrap();
        assert_ne!(changed.digest().unwrap(), digest, "case {change}");
    }
    let committed = report(&good, &target().unwrap().catalog_sha256, "committed").unwrap();
    let replay = report(
        &good,
        &target().unwrap().catalog_sha256,
        "already_committed",
    )
    .unwrap();
    assert_eq!(committed.request_sha256, replay.request_sha256);
    assert_eq!(replay.upgrade_id, good.upgrade_id);
    assert_eq!(replay.components, 29);
}

#[tokio::test]
async fn payment_successor_does_not_authorize_the_old_finance_procedure() {
    let mut historical = request();
    historical.procedure = "core-site-finance-v1".into();
    historical.predecessor_manifest_sha256 =
        "47ad36927c16ccc4237cf16c9889705ae4d53db184c77aaac4a264c4e5203078".into();
    historical.predecessor_catalog_sha256 =
        "557d9775042b94356f752ae4993bc73be645e9956e9fe1110ed5aa1cd531ee2d".into();
    assert!(matches!(
        upgrade_database("not-a-postgresql-url", historical).await,
        Err(UpgradeError::Rejected(
            "SITE_FINANCE_COMPILED_COMPONENTS_CHANGED"
        ))
    ));
}

#[test]
fn pinned_catalog_keeps_transaction_ownership_global_and_upgrade_history_immutable() {
    let facts: Vec<Vec<serde_json::Value>> = target()
        .unwrap()
        .definitions
        .iter()
        .map(|s| serde_json::from_str(s).unwrap())
        .collect();
    let constraint = |name: &str| -> String {
        let matching: Vec<_> = facts
            .iter()
            .filter(|a| a[0] == "constraint" && a[1] == "core_payments" && a[2] == name)
            .collect();
        assert_eq!(matching.len(), 1);
        matching[0][3].as_str().unwrap().to_owned()
    };
    assert_eq!(
        constraint("intents_provider_transaction_owner"),
        "UNIQUE (provider, merchant, environment, provider_transaction)"
    );
    assert_eq!(
        constraint("intents_currency_check"),
        "CHECK (currency ~ '^[A-Z]{3}$'::text)"
    );
    assert_eq!(
        constraint("intents_credits_per_minor_check"),
        "CHECK (credits_per_minor > 0)"
    );
    assert_eq!(
        constraint("intents_provider_check"),
        "CHECK (provider = ANY (ARRAY['stripe'::text, 'epay'::text, 'creem'::text, 'waffo'::text, 'waffo_pancake'::text]))"
    );
    let triggers: Vec<_> = facts
        .iter()
        .filter(|a| a[0] == "trigger" && a[1] == "core_meta" && a[2] == "payment_gateway_upgrades")
        .collect();
    assert_eq!(triggers.len(), 2);
    assert!(triggers.iter().all(|a| {
        a[4] == "O"
            && a[5]
                .as_str()
                .unwrap()
                .contains("core_billing.ledger_immutable()")
    }));
    assert!(
        triggers
            .iter()
            .any(|a| a[5].as_str().unwrap().contains("BEFORE DELETE OR UPDATE"))
    );
    assert!(
        triggers
            .iter()
            .any(|a| a[5].as_str().unwrap().contains("BEFORE TRUNCATE"))
    );
}

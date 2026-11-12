//! Test-only models of preserved installers, never current schema gold.
//! The oracle is the already captured manifest; SQL bytes are checked against
//! it independently. Old catalog query bytes belong to that captured protocol.
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::collections::BTreeMap;

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct Manifest {
    format_version: u32,
    scope: String,
    pub components: Vec<Component>,
    identity: Identity,
    installer_contract_sql_sha256: String,
    catalog: Catalog,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct Component {
    pub name: String,
    pub raw_schema_sha256: String,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Identity {
    version: i32,
    fingerprint_sha256: String,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Catalog {
    query_utf8: String,
    query_sha256: String,
    query_hash_domain: String,
    fingerprint_hash_domain: String,
    definition_order: String,
}
impl Manifest {
    pub fn stdout(&self) -> Vec<u8> {
        let mut bytes = serde_json::to_vec(self).unwrap();
        bytes.push(b'\n');
        bytes
    }
    pub fn sha256(&self) -> String {
        super::hex(&Sha256::digest(self.stdout()))
    }
    pub fn component_map(&self) -> BTreeMap<String, String> {
        self.components
            .iter()
            .map(|c| (c.name.clone(), c.raw_schema_sha256.clone()))
            .collect()
    }
}

pub(super) fn old_ddl(name: &str, unchanged: &'static str) -> &'static str {
    match name {
        "identity" => include_str!("../../upgrade_offline/historical/identity_v5.sql"),
        "console" => include_str!("../../upgrade_offline/historical/console.sql"),
        "ledger" => include_str!("../../../tests/core_schema/ledger_before_escrow_submit.sql"),
        "routing" => include_str!("../../../tests/core_schema/routing_before_payload_policy.sql"),
        "wallet" => {
            include_str!("../../../tests/core_schema/wallet_before_legacy_transfer_history.sql")
        }
        _ => unchanged,
    }
}

pub(super) fn preserved(count: usize) -> Manifest {
    let (record, digest) = match count {
        24 => (
            include_str!("../predecessor24.json"),
            "9d7b93584b646b0d7f26a0a67e8570a213a22f06c9001838c29b21beb06722df",
        ),
        25 => (
            include_str!("../predecessor25.json"),
            "a9c69e01a631fd9e1b005b892ca855fb19a12041c279d04f19b2e9189d4ea301",
        ),
        26 => (
            include_str!("../predecessor26.json"),
            "310937ea692be55a4b5070e3d6bc2ddf86071945b717e47359d68d3571fad351",
        ),
        31 => (
            include_str!("appeals_predecessor_manifest.json"),
            "6991ca03dbef6ea9ec788bf191d022f8e6a45f5f7ecbb1190255e5dd9c1b5735",
        ),
        32 => (
            include_str!("appeals_predecessor_manifest.json"),
            "9c9726a7fa204a150d9fe18139d4580d23feb4cec40145fcb829338331a4cd15",
        ),
        _ => panic!("no captured installer for {count} components"),
    };
    let capture: serde_json::Value = serde_json::from_str(record).unwrap();
    let mut expected: Manifest = serde_json::from_value(if count <= 26 {
        capture["manifest"].clone()
    } else {
        capture
    })
    .unwrap();
    expected.components.truncate(count);
    assert_eq!(
        expected.sha256(),
        digest,
        "captured installer bytes must never be regenerated"
    );
    assert_eq!(
        super::hex(&Sha256::digest(expected.catalog.query_utf8.as_bytes())),
        expected.catalog.query_sha256
    );

    // A fresh-install binary does not reconstruct preserved installers from its
    // current SQL. The captured bytes remain the historical contract.
    expected
}

//! Exact retained Core58 manifest for synthetic historical predecessor fixtures.
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

#[derive(Clone, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Manifest {
    pub(crate) format_version: u32,
    pub(crate) scope: String,
    pub(crate) components: Vec<Component>,
    pub(crate) identity: Identity,
    pub(crate) installer_contract_sql_sha256: String,
    pub(crate) catalog: Catalog,
}
#[derive(Clone, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Component {
    pub(crate) name: String,
    pub(crate) raw_schema_sha256: String,
}
#[derive(Clone, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Identity {
    pub(crate) version: i32,
    pub(crate) fingerprint_sha256: String,
}
#[derive(Clone, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Catalog {
    pub(crate) query_utf8: String,
    pub(crate) query_sha256: String,
    pub(crate) query_hash_domain: String,
    pub(crate) fingerprint_hash_domain: String,
    pub(crate) definition_order: String,
}

pub(crate) fn schema_manifest() -> Manifest {
    let bytes = include_str!("../fixtures/preserved_core58/schema-manifest.json");
    let target: serde_json::Value = serde_json::from_str(include_str!(
        "../../src/core_schema/upgrade/commerce_display_rate_target.json"
    ))
    .unwrap();
    assert_eq!(
        crate::hex_codec::encode(Sha256::digest(bytes.as_bytes())),
        target["manifest_sha256"],
        "retained Core58 manifest must match the original captured target"
    );
    let manifest: Manifest = serde_json::from_str(bytes).unwrap();
    assert_eq!(manifest.components.len(), 58);
    assert_eq!(manifest.components[57].name, "display-exchange-rate");
    let mut canonical = serde_json::to_vec(&manifest).unwrap();
    canonical.push(b'\n');
    assert_eq!(canonical, bytes.as_bytes());
    manifest
}

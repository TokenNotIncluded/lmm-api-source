//! The installed descriptors must bind the real Core read and paid executor.
use super::{STATUS_TOOL_NAME, STATUS_TOOL_SCHEMA, agent};
use crate::assistant_model::agent::registry::{AgentRegistry, ToolDomain};
use crate::assistant_tools_core::{TypedReadArgs, implemented_manifest};
use serde_json::Value;
use sha2::{Digest, Sha256};

const REVIEWED: &[u8] = include_bytes!(concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/../../contracts/assistant-tools-v2.json"
));
const REVIEWED_SHA: &str = "97e6f6c91308e6d8d8ec42380ecfc2df6cbada21651c9fcc9ea5d2f7db6b652e";

#[test]
fn reviewed_weekly_registry_matches_core_read_and_paid_executor() {
    let registry = AgentRegistry::from_reviewed(REVIEWED, REVIEWED_SHA).unwrap();
    let status = registry.get(STATUS_TOOL_NAME).unwrap();
    assert_eq!(status.domain, ToolDomain::RegisteredCore);
    assert_eq!(status.version, agent::TOOL_VERSION);
    assert_eq!(
        status.schema_sha256,
        <[u8; 32]>::from(Sha256::digest(STATUS_TOOL_SCHEMA.as_bytes()))
    );
    assert!(TypedReadArgs::parse(STATUS_TOOL_NAME, br#"{}"#).is_ok());

    let manifest = implemented_manifest().unwrap();
    let rows = manifest.as_array().unwrap();
    let matching: Vec<&Value> = rows
        .iter()
        .filter(|row| row["name"] == STATUS_TOOL_NAME)
        .collect();
    assert_eq!(matching.len(), 1);
    let read = matching[0];
    assert_eq!(
        serde_json::to_vec(&read["parameters"]).unwrap(),
        STATUS_TOOL_SCHEMA.as_bytes()
    );
    assert_eq!(read["version"], status.version);
    assert_eq!(read["description"], status.description);
    assert_eq!(read["effect"], "read_only");
    assert_eq!(read["access"], "user");
    assert_eq!(read["domain"], "registered_core");
    assert_eq!(
        read["schema_sha256"],
        crate::hex_codec::encode(status.schema_sha256)
    );
    assert_eq!(
        read["price_sha256"],
        crate::hex_codec::encode(status.price_sha256)
    );

    let prepare = registry.get(agent::TOOL_NAME).unwrap();
    assert!(agent::validate_installed_tool(prepare).is_ok());
    assert!(!rows.iter().any(|row| row["name"] == agent::TOOL_NAME));
    let mut wrong = prepare.clone();
    wrong.domain = ToolDomain::Extension;
    assert!(agent::validate_installed_tool(&wrong).is_err());
    let mut wrong = prepare.clone();
    wrong.version = "unreviewed-weekly-version".into();
    assert!(agent::validate_installed_tool(&wrong).is_err());
    let mut wrong = prepare.clone();
    wrong.schema_sha256[0] ^= 1;
    assert!(agent::validate_installed_tool(&wrong).is_err());
}

#[test]
fn reviewed_weekly_schemas_reject_actor_injection_and_unbounded_arguments() {
    let registry = AgentRegistry::from_reviewed(REVIEWED, REVIEWED_SHA).unwrap();
    let status = registry.get(STATUS_TOOL_NAME).unwrap();
    assert!(status.validate_arguments(br#"{}"#).is_ok());
    for raw in [
        br#"{"user_id":7}"#.as_slice(),
        br#"{"week_start":345600}"#,
        br#"{"claim":true}"#,
    ] {
        assert!(status.validate_arguments(raw).is_err());
        assert!(TypedReadArgs::parse(STATUS_TOOL_NAME, raw).is_err());
    }
    let prepare = registry.get(agent::TOOL_NAME).unwrap();
    for raw in [
        br#"{"discount_percent":0,"reason":"No qualifying request."}"#.as_slice(),
        br#"{"discount_percent":99,"reason":"Schema limit only; Core still enforces its configured policy."}"#,
    ] {
        assert!(prepare.validate_arguments(raw).is_ok());
    }
    for raw in [
        br#"{"discount_percent":100,"reason":"Over schema maximum."}"#.as_slice(),
        br#"{"discount_percent":-1,"reason":"Negative percentage."}"#,
        br#"{"discount_percent":10}"#,
        br#"{"discount_percent":10,"reason":"A"}"#,
        br#"{"discount_percent":10,"reason":"Allowed length.","user_id":7}"#,
        br#"{"discount_percent":10,"reason":"Allowed length.","claim":true}"#,
    ] {
        assert!(prepare.validate_arguments(raw).is_err());
    }
}

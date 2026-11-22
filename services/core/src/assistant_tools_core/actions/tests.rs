use super::*;

#[test]
fn fixed_action_arguments_and_user_selection_are_closed() {
    assert!(serde_json::from_slice::<KeyArgs>(br#"{"action":"disable","token_id":7}"#).is_ok());
    for bytes in [
        br#"{"action":"rotate","token_id":7}"#.as_slice(),
        br#"{"action":"delete","token_id":7,"user_id":8}"#,
        br#"{"action":"delete","token_id":7,"two_factor_code":"123456"}"#,
    ] {
        assert!(serde_json::from_slice::<KeyArgs>(bytes).is_err());
    }
    assert!(
        serde_json::from_slice::<ProfileArgs>(
            br#"{"action":"change_display_name","display_name":"new","target_user_id":7}"#
        )
        .is_err()
    );
    assert!(selected_id("请禁用 ID: 71", 71));
    assert!(!selected_id("请禁用 ID: 710", 71));
    assert!(selected_id("#71", 71));
    assert!(selected_name("请停用「研发」", "研发"));
    assert!(!selected_name("请停用研发备份", "研发"));
    assert!(!selected_name("disable backup-two", "backup"));
    assert!(explicitly_declined("不要删除 ID: 71"));
    assert!(explicitly_declined("do not disable #71"));
    assert!(!assistant_actions::valid_display_name(" padded "));
    assert!(!assistant_actions::valid_display_name("line\nbreak"));
    assert!(!assistant_actions::valid_display_name(&"名".repeat(21)));
    assert!(assistant_actions::valid_display_name("正常昵称"));
}

#[test]
fn reviewed_action_schema_binds_domain_version_and_exact_bytes() {
    use crate::assistant_model::agent::registry::AgentRegistry;
    let mut entries = Vec::new();
    for name in [
        KEY_TOOL,
        PROFILE_TOOL,
        CREATE_TOOL,
        crate::tool_market::assistant_connect::TOOL,
    ] {
        let (schema, version) = descriptor(name).unwrap();
        entries.push(json!({"name":name,"description":"Synthetic confirmation qualification",
            "version":version,"schema_json":schema,"schema_sha256":crate::hex_codec::encode(Sha256::digest(schema.as_bytes())),
            "price_version":"builtin-free-v1","price_sha256":"5d6fb460c679922a1976e87ef729440d5394187e9b12331f33faf12904a64192",
            "domain":"registered_core"}));
    }
    let bytes = serde_json::to_vec(&json!({"tools":entries})).unwrap();
    let registry =
        AgentRegistry::from_reviewed(&bytes, &crate::hex_codec::encode(Sha256::digest(&bytes)))
            .unwrap();
    for name in [
        KEY_TOOL,
        PROFILE_TOOL,
        CREATE_TOOL,
        crate::tool_market::assistant_connect::TOOL,
    ] {
        validate_installed_tool(registry.get(name).unwrap()).unwrap();
        let mut wrong = registry.get(name).unwrap().clone();
        wrong.version = "unreviewed".into();
        assert!(validate_installed_tool(&wrong).is_err());
        wrong = registry.get(name).unwrap().clone();
        wrong.schema_sha256 = [0; 32];
        assert!(validate_installed_tool(&wrong).is_err());
    }
}

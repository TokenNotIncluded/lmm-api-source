use super::*;

fn parsed(items: Value) -> Result<Request, RelayError> {
    Request::parse_deferred_openai(
        ClientProtocol::Responses,
        &serde_json::to_vec(&json!({"model":"m","instructions":"preamble","input":items})).unwrap(),
        &Limits::default(),
    )
}

#[test]
fn complete_input_message_ids_follow_their_own_roles_and_order_only_on_responses() {
    let input = json!([
        {"type":"message","role":"user","id":"msg_A-1","content":[{"type":"input_text","text":"same"}]},
        {"role":"user","content":"same"},
        {"role":"developer","id":"msg_dev","content":"policy"},
        {"type":"message","role":"system","id":"msg_sys","content":"system"},
        {"role":"assistant","content":[{"type":"output_text","text":"ack"}]},
        {"role":"user","id":"msg_B-2","content":"same"}
    ]);
    let req = parsed(input).unwrap();
    let native = req
        .upstream_to(UpstreamProtocol::Responses, "supplier")
        .unwrap();
    let items = native["input"].as_array().unwrap();
    let actual: Vec<_> = items
        .iter()
        .map(|item| {
            (
                item["role"].as_str().unwrap(),
                item.get("id").and_then(Value::as_str),
                item["content"][0]["text"].as_str().unwrap(),
            )
        })
        .collect();
    assert_eq!(
        actual,
        [
            ("system", None, "preamble"),
            ("user", Some("msg_A-1"), "same"),
            ("user", None, "same"),
            ("developer", Some("msg_dev"), "policy"),
            ("system", Some("msg_sys"), "system"),
            ("assistant", None, "ack"),
            ("user", Some("msg_B-2"), "same"),
        ]
    );
    let chat = req.upstream_to(UpstreamProtocol::Chat, "supplier").unwrap();
    let messages = chat["messages"].as_array().unwrap();
    assert_eq!(messages.len(), actual.len());
    for (message, (role, _, text)) in messages.iter().zip(actual) {
        assert_eq!(message["role"], role);
        assert_eq!(message["content"], text);
        assert!(message.get("id").is_none());
    }
}

#[test]
fn responses_input_id_requires_a_complete_message_and_never_resolves_references() {
    let full = json!({"role":"user","id":"a".repeat(128),"content":"complete"});
    assert!(parsed(json!([full.clone()])).is_ok());
    for id in [
        Value::Null,
        json!(42),
        json!(""),
        json!("a".repeat(129)),
        json!("invalid id"),
    ] {
        let mut bad = full.clone();
        bad["id"] = id;
        assert!(parsed(json!([bad])).is_err());
    }
    for bad in [
        json!({"type":"item_reference","id":"msg_valid"}),
        json!({"type":"message","role":"user","id":"msg_valid"}),
        json!({"role":"developer","content":[],"id":"msg_valid"}),
        json!({"role":"system","content":"","id":"msg_valid"}),
        json!({"role":"user","content":"x","id":"msg_valid","phase":"commentary"}),
        json!({"role":"user","content":"x","id":"msg_valid","status":"completed"}),
        json!({"role":"tool","content":"x","id":"msg_valid"}),
    ] {
        assert!(parsed(json!([bad])).is_err());
    }
}

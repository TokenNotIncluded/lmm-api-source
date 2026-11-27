use super::*;
use serde_json::{Value, json};

fn parse(body: &Value) -> Result<Request, RelayError> {
    Request::parse_deferred_openai(
        ClientProtocol::Responses,
        &serde_json::to_vec(body).unwrap(),
        &Limits::default(),
    )
}

fn first_turn() -> Value {
    json!({
        "model": "lmm/gpt-6.1-sol",
        "input": [{"role":"user", "content":[{"type":"input_text","text":"hi"}]}],
        "store":false,"stream":true,"reasoning":{"effort":"medium","summary":"auto"},
        "include":["reasoning.encrypted_content"],"prompt_cache_key":"example-session",
        "tools":[{"type":"function","name":"read","description":"Read a file",
            "parameters":{"type":"object","properties":{"path":{"type":"string"}}},"strict":false}]
    })
}

#[test]
fn codex_client_metadata_stays_on_responses_and_keeps_tool_gates() {
    let mut body = first_turn();
    body["client_metadata"] = json!({
        "turn_id":"synthetic-turn", "root_turn_id":"synthetic-root", "session_id":"synthetic-session",
        "thread_id":"synthetic-thread", "x-codex-window-id":"window",
        "x-codex-installation-id":"installation", "x-codex-turn-metadata":"{}"
    });
    let request = parse(&body).unwrap();
    let responses = request
        .upstream_to(UpstreamProtocol::Responses, "m")
        .unwrap();
    assert_eq!(responses["client_metadata"], body["client_metadata"]);
    let chat = request.upstream_to(UpstreamProtocol::Chat, "m").unwrap();
    assert!(chat.get("client_metadata").is_none());
    for kind in ["namespace", "web_search"] {
        let mut unsupported = body.clone();
        unsupported["tools"] = json!([{"type":kind}]);
        assert!(parse(&unsupported).is_err());
    }
    for bad in [
        json!({"unknown":"x"}),
        json!({"turn_id":true}),
        json!({"turn_id":"x".repeat(4097)}),
        json!([]),
    ] {
        body["client_metadata"] = bad;
        assert!(parse(&body).is_err());
    }
}

#[test]
fn pi_responses_first_turn_projects_preferences_without_changing_spend_or_tools() {
    for mode in ["auto", "concise", "detailed"] {
        let mut body = first_turn();
        body["reasoning"]["summary"] = json!(mode);
        let request = parse(&body).unwrap();
        assert!(request.explicit_max_output_tokens.is_none());
        assert_eq!(
            request.legacy_precharge_body.as_deref().unwrap(),
            serde_json::to_vec(&body).unwrap()
        );
        let wire = request
            .upstream_to(UpstreamProtocol::Responses, "supplier-model")
            .unwrap();
        assert_eq!(wire["reasoning"], body["reasoning"]);
        assert_eq!(wire["store"], false);
        assert_eq!(wire["stream"], true);
        assert_eq!(wire["tools"][0]["name"], "read");
        assert_eq!(wire["prompt_cache_key"], body["prompt_cache_key"]);
        assert!(wire.get("include").is_none());
        let chat = request
            .upstream_to(UpstreamProtocol::Chat, "supplier-model")
            .unwrap();
        assert_eq!(chat["reasoning_effort"], "medium");
        assert_eq!(chat["tools"][0]["function"]["name"], "read");
        assert_eq!(chat["stream_options"]["include_usage"], true);
        assert!(chat.get("reasoning").is_none() && chat.get("include").is_none());
    }
}

#[test]
fn pi_responses_replays_completed_assistant_and_tool_items_in_order() {
    let mut body = first_turn();
    let items = body["input"].as_array_mut().unwrap();
    let public_id = format!("rs_resp_{}_1", "a".repeat(64));
    items.extend([
        json!({"id":public_id,"type":"reasoning","summary":[{"type":"summary_text","text":"display detail"}],"content":[],"status":"completed"}),
        json!({"id":"msg_a","type":"message","role":"assistant","status":"completed","phase":"commentary","content":[{"type":"output_text","text":"checking","annotations":[]}]}),
        json!({"id":"fc_public","type":"function_call","status":"completed","call_id":"call_a","name":"read","arguments":"{\"path\":\"hello\"}"}),
        json!({"type":"function_call_output","call_id":"call_a","output":"hello"}),
        json!({"id":"msg_b","type":"message","role":"assistant","status":"completed","phase":"final_answer","content":[{"type":"output_text","text":"answer","annotations":[]}]}),
        json!({"role":"user","content":[{"type":"input_text","text":"next"}]}),
    ]);
    let request = parse(&body).unwrap();
    let wire = request
        .upstream_to(UpstreamProtocol::Responses, "supplier-model")
        .unwrap();
    let items = wire["input"].as_array().unwrap();
    assert_eq!(items.len(), 6);
    assert_eq!(items[1]["phase"], "commentary");
    assert_eq!(items[1]["content"][0]["text"], "checking");
    assert_eq!(items[2]["call_id"], "call_a");
    assert_eq!(items[2]["arguments"], "{\"path\":\"hello\"}");
    assert_eq!(items[3]["output"], "hello");
    assert_eq!(items[4]["phase"], "final_answer");
    assert_eq!(items[5]["content"][0]["text"], "next");
    assert!(!wire.to_string().contains("display detail"));
    assert!(
        request
            .upstream_to(UpstreamProtocol::Chat, "supplier-model")
            .is_err()
    );

    body["input"][2].as_object_mut().unwrap().remove("phase");
    let chat = parse(&body)
        .unwrap()
        .upstream_to(UpstreamProtocol::Chat, "supplier-model")
        .unwrap();
    assert!(
        chat["messages"]
            .as_array()
            .unwrap()
            .iter()
            .any(|item| item["tool_call_id"] == "call_a" && item["content"] == "hello")
    );
}

#[test]
fn pi_responses_keeps_provider_state_status_and_tool_validation() {
    for extra in [
        json!({"previous_response_id":"resp_other"}),
        json!({"store":true}),
        json!({"include":["other.contract"]}),
        json!({"reasoning":{"effort":"medium","summary":true}}),
        json!({"input":[{"type":"reasoning","id":"rs_provider","summary":[]}]}),
        json!({"input":[{"type":"reasoning","id":format!("rs_resp_{}_0", "c".repeat(64)),"summary":[], "encrypted_content":"opaque"}]}),
        json!({"input":[{"type":"reasoning","id":format!("rs_resp_{}_0", "c".repeat(64)),"summary":[], "content":[{"type":"reasoning_text","text":"private"}]}]}),
        json!({"input":[{"type":"message","role":"assistant","status":"in_progress","content":"unfinished"}]}),
        json!({"input":[{"type":"function_call","name":"f","call_id":"a","arguments":"not-json","status":"completed"}]}),
        json!({"input":[{"type":"function_call_output","call_id":"unpaired","output":"no"}]}),
        json!({"input":[{"type":"item_reference","id":"other"}]}),
    ] {
        let mut body = first_turn();
        body.as_object_mut()
            .unwrap()
            .extend(extra.as_object().unwrap().clone());
        assert!(parse(&body).is_err(), "{body}");
    }
}

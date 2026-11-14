use super::{ClientProtocol, Limits, Request, UpstreamProtocol, native_anthropic, native_gemini};
use serde_json::{Value, json};

const PNG: &str =
    "iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAIAAACQd1PeAAAADElEQVR4nGP4z8AAAAMBAQDJ/pLvAAAAAElFTkSuQmCC";

fn chat(block: Value) -> Result<Request, super::RelayError> {
    Request::parse(
        ClientProtocol::Chat,
        &serde_json::to_vec(&json!({
            "model":"vision", "messages":[{"role":"user","content":[
                {"type":"text","text":"before"}, block, {"type":"text","text":"after"}
            ]}]
        }))
        .unwrap(),
        &Limits::default(),
    )
}

#[test]
fn vision_inline_images_convert_to_all_four_protocols_in_order() {
    let data_url = format!("data:image/png;base64,{PNG}");
    let request = chat(json!({"type":"image_url", "image_url":{"url":data_url}})).unwrap();
    let chat = request
        .upstream_to(UpstreamProtocol::Chat, "chosen")
        .unwrap();
    assert_eq!(
        chat["messages"][0]["content"],
        json!([
            {"type":"text","text":"before"},
            {"type":"image_url","image_url":{"url":data_url}},
            {"type":"text","text":"after"}
        ])
    );
    let responses = request
        .upstream_to(UpstreamProtocol::Responses, "chosen")
        .unwrap();
    assert_eq!(
        responses["input"][0]["content"],
        json!([
            {"type":"input_text","text":"before"},
            {"type":"input_image","image_url":data_url},
            {"type":"input_text","text":"after"}
        ])
    );
    let anthropic = request
        .upstream_to(UpstreamProtocol::Anthropic, "chosen")
        .unwrap();
    assert_eq!(
        anthropic["messages"][0]["content"][1],
        json!({
            "type":"image","source":{"type":"base64","media_type":"image/png","data":PNG}
        })
    );
    let gemini = request
        .upstream_to(UpstreamProtocol::Gemini, "chosen")
        .unwrap();
    assert_eq!(
        gemini["contents"][0]["parts"],
        json!([
            {"text":"before"}, {"inlineData":{"mimeType":"image/png","data":PNG}}, {"text":"after"}
        ])
    );
    assert!(request.validate_exact_input_count().is_err());
}

#[test]
fn vision_responses_detail_survives_openai_conversion_and_rejects_loss() {
    for detail in ["auto", "low", "high", "original"] {
        let request = Request::parse(ClientProtocol::Responses, &serde_json::to_vec(&json!({
            "model":"vision", "input":[{"role":"user","content":[
                {"type":"input_image", "image_url":format!("data:image/png;base64,{PNG}"), "detail":detail}
            ]}]
        })).unwrap(), &Limits::default()).unwrap();
        assert_eq!(
            request
                .upstream_to(UpstreamProtocol::Chat, "chosen")
                .unwrap()["messages"][0]["content"][0]["image_url"]["detail"],
            detail
        );
        assert_eq!(
            request
                .upstream_to(UpstreamProtocol::Responses, "chosen")
                .unwrap()["input"][0]["content"][0]["detail"],
            detail
        );
        for protocol in [UpstreamProtocol::Anthropic, UpstreamProtocol::Gemini] {
            assert_eq!(
                request.upstream_to(protocol, "chosen").is_ok(),
                detail == "auto"
            );
        }
    }
}

#[test]
fn vision_native_anthropic_and_gemini_images_reach_openai_wire() {
    let anthropic = native_anthropic::parse(
        &serde_json::to_vec(&json!({
            "model":"vision","max_tokens":16,"messages":[{"role":"user","content":[
                {"type":"image","source":{"type":"base64","media_type":"image/png","data":PNG}},
                {"type":"text","text":"describe"}
            ]}]
        }))
        .unwrap(),
        &Limits::default(),
    )
    .unwrap();
    let converted = anthropic
        .upstream_to(UpstreamProtocol::Chat, "chosen")
        .unwrap();
    assert_eq!(converted["messages"].as_array().unwrap().len(), 1);
    assert_eq!(converted["messages"][0]["role"], "user");
    assert_eq!(
        converted["messages"][0]["content"][1],
        json!({"type":"text","text":"describe"})
    );
    assert_eq!(
        anthropic
            .upstream_to(UpstreamProtocol::Chat, "chosen")
            .unwrap()["messages"][0]["content"][0]["image_url"]["url"],
        format!("data:image/png;base64,{PNG}")
    );
    let gemini = native_gemini::parse("vision", false, &serde_json::to_vec(&json!({
        "contents":[{"role":"user","parts":[{"inlineData":{"mimeType":"image/png","data":PNG}},{"text":"describe"}]}]
    })).unwrap(), &Limits::default()).unwrap();
    assert_eq!(
        gemini
            .upstream_to(UpstreamProtocol::Responses, "chosen")
            .unwrap()["input"][0]["content"][0]["image_url"],
        format!("data:image/png;base64,{PNG}")
    );
    assert_eq!(
        gemini
            .upstream_to(UpstreamProtocol::Gemini, "chosen")
            .unwrap()["contents"][0]["parts"][0],
        json!({"inlineData":{"mimeType":"image/png","data":PNG}})
    );
}

#[test]
fn vision_remote_urls_forward_without_synthesizing_a_gemini_media_type() {
    let url = "https://images.example.org/photo.png";
    let request = chat(json!({"type":"image_url", "image_url":{"url":url}})).unwrap();
    let anthropic = request
        .upstream_to(UpstreamProtocol::Anthropic, "chosen")
        .unwrap();
    assert_eq!(
        anthropic["messages"][0]["content"][1],
        json!({"type":"image","source":{"type":"url","url":url}})
    );
    assert!(
        request
            .upstream_to(UpstreamProtocol::Gemini, "chosen")
            .is_err()
    );
    let request = native_anthropic::parse(
        &serde_json::to_vec(&json!({
            "model":"vision","max_tokens":16,"messages":[{"role":"user","content":[
                {"type":"image","source":{"type":"url","url":url}}
            ]}]
        }))
        .unwrap(),
        &Limits::default(),
    )
    .unwrap();
    assert_eq!(
        request
            .upstream_to(UpstreamProtocol::Responses, "chosen")
            .unwrap()["input"][0]["content"][0]["image_url"],
        url
    );
}

#[test]
fn vision_rejects_invalid_sources_detail_roles_and_unsupported_file_references() {
    for url in [
        "",
        "file:///private/image",
        "https://user:password@example.org/image",
        "data:image/png;base64,",
        "data:image/png;base64,invalid!",
        "data:application/pdf;base64,YQ==",
    ] {
        assert!(chat(json!({"type":"image_url", "image_url":{"url":url}})).is_err());
    }
    for detail in [json!(null), json!(1), json!("maximum")] {
        assert!(chat(json!({"type":"image_url","image_url":{"url":"https://example.org/image", "detail":detail}})).is_err());
    }
    for role in ["assistant", "system", "developer", "tool"] {
        assert!(Request::parse(ClientProtocol::Chat, &serde_json::to_vec(&json!({
            "model":"vision", "messages":[{"role":role,"content":[{"type":"image_url","image_url":{"url":"https://example.org/image"}}]}]
        })).unwrap(), &Limits::default()).is_err());
    }
    assert!(Request::parse(ClientProtocol::Responses, &serde_json::to_vec(&json!({
        "model":"vision", "input":[{"role":"user","content":[{"type":"input_image","file_id":"file_other_user"}]}]
    })).unwrap(), &Limits::default()).is_err());
    assert!(native_gemini::parse("vision", false, &serde_json::to_vec(&json!({
        "contents":[{"parts":[{"fileData":{"mimeType":"image/png","fileUri":"gs://private-bucket/image"}}]}]
    })).unwrap(), &Limits::default()).is_err());
}

#[test]
fn legacy_haiku_system_cache_selected_route() {
    use crate::routing::PayloadPolicy;
    for (client, model, system, expected) in [
        (ClientProtocol::Chat, "claude-haiku-4-5-mapped", true, true),
        (ClientProtocol::Chat, "claude-sonnet-4-5", true, false),
        (ClientProtocol::Chat, "claude-haiku-4-5", false, false),
        (ClientProtocol::Anthropic, "claude-haiku-4-5", true, false),
    ] {
        let body = if client == ClientProtocol::Chat {
            let mut messages = vec![];
            if system {
                messages.push(json!({"role":"system","content":[{"type":"text","text":"policy"}]}));
            }
            messages.push(json!({"role":"user","content":"hello"}));
            json!({"model":"client-alias","messages":messages})
        } else {
            json!({"model":"client-alias","max_tokens":16,"system":[{"type":"text","text":"policy"}],"messages":[{"role":"user","content":"hello"}]})
        };
        let request = Request::parse(
            client,
            &serde_json::to_vec(&body).unwrap(),
            &Limits::default(),
        )
        .unwrap();
        let route = super::Route::new(
            "selected".into(),
            UpstreamProtocol::Anthropic,
            "https://example.test/v1/messages",
            model.into(),
            "test-only",
            1,
        )
        .unwrap()
        .with_payload_policy(PayloadPolicy::LegacyAnthropicHaikuSystemCacheV1);
        let output: Value =
            serde_json::from_slice(&request.upstream_body(&route).unwrap()).unwrap();
        assert_eq!(output["model"], model);
        assert_eq!(
            output.pointer("/system/0/cache_control").is_some(),
            expected
        );
        let ordinary = route.with_payload_policy(PayloadPolicy::Adapt);
        let output: Value =
            serde_json::from_slice(&request.upstream_body(&ordinary).unwrap()).unwrap();
        assert!(output.pointer("/system/0/cache_control").is_none());
    }
}

#[test]
fn legacy_haiku_system_cache_preserves_existing_after_conversion() {
    let mut request = chat(json!({"type":"text","text":"hello"})).unwrap();
    request.anthropic_cached_prompt = Some(
        serde_json::from_value(json!({"system":[
            {"type":"text","text":"first"},
            {"type":"text","text":"last","cache_control":{"type":"ephemeral","ttl":"1h"}}
        ]}))
        .unwrap(),
    );
    let route = super::Route::new(
        "selected".into(),
        UpstreamProtocol::Anthropic,
        "https://example.test/v1/messages",
        "claude-haiku-4-5".into(),
        "test-only",
        1,
    )
    .unwrap()
    .with_payload_policy(crate::routing::PayloadPolicy::LegacyAnthropicHaikuSystemCacheV1);
    let output: Value = serde_json::from_slice(&request.upstream_body(&route).unwrap()).unwrap();
    assert_eq!(
        output["system"][1]["cache_control"],
        json!({"type":"ephemeral","ttl":"1h"})
    );
    assert!(output["system"][0].get("cache_control").is_none());
}

#[test]
fn anthropic_system_presence_keeps_normal_admission_and_user_order() {
    for system in [
        None,
        Some(json!(null)),
        Some(json!([])),
        Some(json!([{"type":"text","text":"policy"}])),
    ] {
        let mut body = json!({"model":"vision","max_tokens":16,"messages":[{"role":"user","content":"hello"}]});
        if let Some(value) = &system {
            body["system"] = value.clone();
        }
        let parsed =
            native_anthropic::parse(&serde_json::to_vec(&body).unwrap(), &Limits::default());
        if system == Some(Value::Null) {
            assert!(parsed.is_err());
            continue;
        }
        let converted = parsed
            .unwrap()
            .upstream_to(UpstreamProtocol::Chat, "chosen")
            .unwrap();
        let messages = converted["messages"].as_array().unwrap();
        let nonempty = system
            .as_ref()
            .and_then(Value::as_array)
            .is_some_and(|a| !a.is_empty());
        assert_eq!(messages.len(), if nonempty { 2 } else { 1 });
        assert_eq!(messages.last().unwrap()["role"], "user");
        if nonempty {
            assert_eq!(messages[0]["role"], "system");
        }
    }
}

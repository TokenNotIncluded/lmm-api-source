//! OpenRouter's Chat images stay separate from text and authoritative usage.
use super::{
    ClientProtocol, Event, Limits, Request, Route, UpstreamProtocol, decode::Decoder,
    encode::Encoder, sse::Frame,
};
use serde_json::{Value, json};
fn request(stream: bool, extra: Value) -> Request {
    let mut body = json!({"model":"google/gemini-3.1-flash-image","messages":[{"role":"user","content":"draw"}],"stream":stream});
    if stream {
        body["stream_options"] = json!({"include_usage":true});
    }
    body.as_object_mut()
        .unwrap()
        .extend(extra.as_object().unwrap().clone());
    Request::parse(
        ClientProtocol::Chat,
        &serde_json::to_vec(&body).unwrap(),
        &Limits::default(),
    )
    .unwrap()
}
fn image(index: u64) -> Value {
    json!({"type":"image_url","index":index,"image_url":{"url":"data:image/png;base64,aW1hZ2U="}})
}
fn frame(v: Value) -> Frame {
    Frame {
        event: None,
        data: v.to_string(),
    }
}
fn route(policy: crate::routing::PayloadPolicy) -> Route {
    Route::new(
        "openrouter".into(),
        UpstreamProtocol::Chat,
        "https://openrouter.ai/api/v1",
        "google/gemini-3.1-flash-image".into(),
        "synthetic",
        1,
    )
    .unwrap()
    .with_payload_policy(policy)
}
#[test]
fn openrouter_full_image_only_and_mixed_output_preserve_usage() {
    for content in [Value::Null, json!("rendered")] {
        let limits = Limits::default();
        let mut decoder = Decoder::new(UpstreamProtocol::Chat, &limits);
        let mut encoder = Encoder::new(&request(false, json!({})), "image-full", &limits);
        for event in decoder.json(&json!({"id":"provider-1","choices":[{"index":0,"message":{"role":"assistant","content":content,"images":[image(0),image(1)]},"finish_reason":"stop"}],"usage":{"prompt_tokens":7,"completion_tokens":19,"prompt_tokens_details":{"cached_tokens":2},"completion_tokens_details":{"reasoning_tokens":3},"cost":0.067}})).unwrap() {
            encoder.consume(event).unwrap();
        }
        let output: Value = serde_json::from_slice(&encoder.terminal().unwrap()[0]).unwrap();
        assert_eq!(
            output["choices"][0]["message"]["images"],
            json!([image(0), image(1)])
        );
        assert_eq!(output["choices"][0]["message"]["content"], content);
        assert_eq!(output["usage"]["prompt_tokens"], 7);
        assert_eq!(output["usage"]["completion_tokens"], 19);
        assert_eq!(encoder.usage().cached_input_tokens, Some(2));
        assert_eq!(encoder.usage().reasoning_output_tokens, Some(3));
        assert_eq!(
            encoder.usage().upstream_cost_atto_usd,
            Some(67_000_000_000_000_000)
        );
    }
}
#[test]
fn openrouter_stream_images_arrive_before_finish_and_keep_usage_only_tail() {
    let limits = Limits::default();
    let mut decoder = Decoder::new(UpstreamProtocol::Chat, &limits);
    let mut encoder = Encoder::new(&request(true, json!({})), "image-stream", &limits);
    let mut packets = Vec::new();
    for input in [
        json!({"id":"p","choices":[{"index":0,"delta":{"content":"drawn ","images":[image(0)]},"finish_reason":null}]}),
        json!({"id":"p","choices":[{"index":0,"delta":{"images":[image(1)]},"finish_reason":null}]}),
    ] {
        for event in decoder.frame(frame(input)).unwrap() {
            packets.extend(encoder.consume(event).unwrap());
        }
    }
    let before = String::from_utf8(packets.concat()).unwrap();
    assert!(!before.contains("[DONE]"));
    let values: Vec<Value> = before
        .lines()
        .filter_map(|line| line.strip_prefix("data: "))
        .map(|s| serde_json::from_str(s).unwrap())
        .collect();
    let images: Vec<Value> = values
        .iter()
        .filter_map(|v| v["choices"][0]["delta"]["images"].as_array())
        .flatten()
        .cloned()
        .collect();
    assert_eq!(images, vec![image(0), image(1)]);
    assert!(
        values
            .iter()
            .filter_map(|v| v["choices"][0]["delta"]["content"].as_str())
            .all(|s| !s.contains("base64"))
    );
    for input in [
        json!({"id":"p","choices":[{"index":0,"delta":{},"finish_reason":"stop"}]}),
        json!({"id":"p","choices":[],"usage":{"prompt_tokens":7,"completion_tokens":19,"prompt_tokens_details":{"cached_tokens":0}}}),
    ] {
        for event in decoder.frame(frame(input)).unwrap() {
            assert!(encoder.consume(event).unwrap().is_empty());
        }
    }
    for event in decoder
        .frame(Frame {
            event: None,
            data: "[DONE]".into(),
        })
        .unwrap()
    {
        assert!(encoder.consume(event).unwrap().is_empty());
    }
    let terminal = String::from_utf8(encoder.terminal().unwrap().concat()).unwrap();
    assert!(terminal.contains("[DONE]"));
    assert!(terminal.contains("\"completion_tokens\":19"));
    assert_eq!(encoder.usage().input_tokens, Some(7));
}
#[test]
fn openrouter_images_reject_malformed_oversized_and_lossy_output() {
    let limits = Limits::default();
    for images in [
        json!({}),
        json!([{"type":"text","image_url":{"url":"x"}}]),
        json!([{"type":"image_url","image_url":{"url":""}}]),
        json!([{"type":"image_url","index":-1,"image_url":{"url":"x"}}]),
    ] {
        assert!(
            Decoder::new(UpstreamProtocol::Chat, &limits)
                .json(&json!({"choices":[{"message":{"images":images},"finish_reason":"stop"}]}))
                .is_err()
        );
    }
    let tiny = Limits {
        output_bytes: 32,
        ..limits.clone()
    };
    let mut encoder = Encoder::new(&request(false, json!({})), "limit", &tiny);
    encoder.consume(Event::Start("p".into())).unwrap();
    assert!(encoder.consume(Event::Images(vec![image(0)])).is_err());
    let one = Limits {
        max_blocks: 1,
        ..limits.clone()
    };
    assert!(Decoder::new(UpstreamProtocol::Chat,&one).json(&json!({"choices":[{"message":{"content":"text","images":[image(0)]},"finish_reason":"stop"}]})).is_err());
    let mut responses = request(false, json!({}));
    responses.client = ClientProtocol::Responses;
    let mut encoder = Encoder::new(&responses, "loss", &limits);
    encoder.consume(Event::Start("p".into())).unwrap();
    assert!(encoder.consume(Event::Images(vec![image(0)])).is_err());
}
#[test]
fn openrouter_image_content_after_finish_is_not_silently_accepted() {
    let mut decoder = Decoder::new(UpstreamProtocol::Chat, &Limits::default());
    decoder
        .frame(frame(
            json!({"choices":[{"index":0,"delta":{},"finish_reason":"stop"}]}),
        ))
        .unwrap();
    assert!(
        decoder
            .frame(frame(
                json!({"choices":[{"index":0,"delta":{"images":[image(0)]},"finish_reason":null}]})
            ))
            .is_err()
    );
}
#[test]
fn openrouter_request_contract_is_explicit_and_preserves_controls() {
    use crate::routing::PayloadPolicy;
    let r = request(
        true,
        json!({"modalities":["image","text"],"image_config":{"aspect_ratio":"16:9","image_size":"1K"},"reasoning_effort":"high"}),
    );
    assert!(r.upstream(&route(PayloadPolicy::Adapt)).is_err());
    let wire = r.upstream(&route(PayloadPolicy::OpenRouter)).unwrap();
    assert_eq!(wire["usage"], json!({"include":true}));
    assert!(wire.get("stream_options").is_none());
    assert!(wire.get("reasoning_effort").is_none());
    assert_eq!(wire["reasoning"], json!({"enabled":true,"effort":"high"}));
    assert_eq!(wire["modalities"], json!(["image", "text"]));
    assert_eq!(
        wire["image_config"],
        json!({"aspect_ratio":"16:9","image_size":"1K"})
    );
    let override_request = request(
        false,
        json!({"usage":{"include":false},"reasoning_effort":"low","reasoning":{"enabled":false,"exclude":true}}),
    );
    let wire = override_request
        .upstream(&route(PayloadPolicy::OpenRouter))
        .unwrap();
    assert_eq!(wire["usage"], json!({"include":false}));
    assert_eq!(wire["reasoning"], json!({"enabled":false,"exclude":true}));
    let none = request(false, json!({"reasoning_effort":"none"}))
        .upstream(&route(PayloadPolicy::OpenRouter))
        .unwrap();
    assert!(none.get("reasoning").is_none());
    let generic = request(true, json!({"reasoning_effort":"high"}))
        .upstream(&route(PayloadPolicy::Adapt))
        .unwrap();
    assert_eq!(generic["reasoning_effort"], "high");
    assert_eq!(generic["stream_options"], json!({"include_usage":true}));
    assert!(generic.get("usage").is_none());
}
#[test]
fn openrouter_controls_reject_bad_shapes_and_other_wire_contracts() {
    for extra in [
        json!({"modalities":["audio"]}),
        json!({"modalities":["image","image"]}),
        json!({"usage":{"include":"yes"}}),
        json!({"image_config":{"unknown":"x"}}),
        json!({"reasoning":{"enabled":1}}),
    ] {
        let mut body = json!({"model":"image","messages":[{"role":"user","content":"draw"}]});
        body.as_object_mut()
            .unwrap()
            .extend(extra.as_object().unwrap().clone());
        assert!(
            Request::parse(
                ClientProtocol::Chat,
                &serde_json::to_vec(&body).unwrap(),
                &Limits::default()
            )
            .is_err()
        );
    }
    assert_eq!(
        serde_json::to_value(crate::routing::PayloadPolicy::OpenRouter).unwrap(),
        json!("open_router")
    );
    assert!(!crate::routing::PayloadPolicy::OpenRouter.adapts(
        crate::routing::Protocol::OpenAiResponses,
        crate::routing::Protocol::OpenAiChat
    ));
    let r = request(false, json!({"modalities":["image"]}));
    assert!(r.upstream_to(UpstreamProtocol::Gemini, "model").is_err());
    // Existing Responses reasoning remains independent of the OpenRouter options.
    let r = Request::parse(
        ClientProtocol::Responses,
        &serde_json::to_vec(&json!({"model":"m","input":"hello","reasoning":{"effort":"high"}}))
            .unwrap(),
        &Limits::default(),
    )
    .unwrap();
    assert_eq!(
        r.upstream_to(UpstreamProtocol::Responses, "m").unwrap()["reasoning"]["effort"],
        "high"
    );
}

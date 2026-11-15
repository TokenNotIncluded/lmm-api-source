//! zconv 0.4 migration contracts at Core's admitted protocol boundary.
use super::{
    BlockKind, ClientProtocol, Event, InputBlock, Limits, RelayError, Request, UpstreamProtocol,
    decode::Decoder, encode::Encoder, sse::Frame,
};
use serde_json::{Value, json};

fn frame(decoder: &mut Decoder, value: Value) -> Result<Vec<Event>, RelayError> {
    decoder.frame(Frame {
        event: value["type"].as_str().map(str::to_owned),
        data: value.to_string(),
    })
}

fn encoder(client: ClientProtocol, stream: bool) -> Encoder {
    let limits = Limits::default();
    let mut request = Request::parse(
        ClientProtocol::Chat,
        br#"{"model":"m","messages":[{"role":"user","content":"hello"}]}"#,
        &limits,
    )
    .unwrap();
    request.client = client;
    request.stream = stream;
    Encoder::new(&request, "compat", &limits)
}

fn text_blocks(events: &[Event]) -> Vec<(&'static str, String)> {
    let mut blocks = Vec::new();
    for event in events {
        match event {
            Event::Block { index, kind } => {
                assert_eq!(*index, blocks.len());
                blocks.push((
                    match kind {
                        BlockKind::Thinking => "raw",
                        BlockKind::ReasoningSummary => "summary",
                        BlockKind::Text => "text",
                        BlockKind::Refusal => "refusal",
                        BlockKind::Tool { .. } => "tool",
                    },
                    String::new(),
                ));
            }
            Event::Delta { index, text } => blocks[*index].1.push_str(text),
            _ => {}
        }
    }
    blocks
}

fn encode(client: ClientProtocol, stream: bool, events: Vec<Event>) -> Vec<u8> {
    let mut encoder = encoder(client, stream);
    let mut bytes = Vec::new();
    for event in events {
        bytes.extend(encoder.consume(event).unwrap().concat());
    }
    bytes.extend(encoder.terminal().unwrap().concat());
    bytes
}

fn packets(bytes: &[u8]) -> Vec<Value> {
    std::str::from_utf8(bytes)
        .unwrap()
        .lines()
        .filter_map(|line| line.strip_prefix("data: "))
        .map(|line| serde_json::from_str(line).unwrap())
        .collect()
}

fn mixed_reasoning_response() -> Value {
    json!({
        "id":"resp_up","object":"response","model":"m","status":"completed",
        "output":[{"type":"reasoning","id":"rs_up","status":"completed",
            "summary":[{"type":"summary_text","text":"short explanation"}],
            "content":[{"type":"reasoning_text","text":"raw thought"}]}]
    })
}

#[test]
fn full_and_stream_responses_keep_raw_and_summary_in_distinct_blocks() {
    for stream in [false, true] {
        let response = mixed_reasoning_response();
        let mut decoder = Decoder::new(UpstreamProtocol::Responses, &Limits::default());
        let events = if stream {
            let mut events = Vec::new();
            for value in [
                json!({"type":"response.created","response":{"id":"resp_up","model":"m","status":"in_progress","output":[]}}),
                json!({"type":"response.output_item.added","output_index":0,"item":{"type":"reasoning","id":"rs_up","summary":[],"content":[]}}),
                json!({"type":"response.content_part.added","output_index":0,"content_index":0,"item_id":"rs_up","part":{"type":"reasoning_text","text":""}}),
                json!({"type":"response.reasoning_summary_part.added","output_index":0,"summary_index":0,"item_id":"rs_up","part":{"type":"summary_text","text":""}}),
                json!({"type":"response.reasoning_text.delta","output_index":0,"content_index":0,"item_id":"rs_up","delta":"raw "}),
                json!({"type":"response.reasoning_summary_text.delta","output_index":0,"summary_index":0,"item_id":"rs_up","delta":"short "}),
                json!({"type":"response.reasoning_text.delta","output_index":0,"content_index":0,"item_id":"rs_up","delta":"thought"}),
                json!({"type":"response.content_part.done","output_index":0,"content_index":0,"item_id":"rs_up"}),
                json!({"type":"response.reasoning_summary_text.delta","output_index":0,"summary_index":0,"item_id":"rs_up","delta":"explanation"}),
                json!({"type":"response.output_item.done","output_index":0,"item":response["output"][0]}),
                json!({"type":"response.completed","response":response}),
            ] {
                events.extend(frame(&mut decoder, value).unwrap());
            }
            events
        } else {
            decoder.json(&response).unwrap()
        };
        assert!(decoder.is_done());
        let mut content = text_blocks(&events);
        content.sort_by_key(|(kind, _)| *kind);
        assert_eq!(
            content,
            [
                ("raw", "raw thought".into()),
                ("summary", "short explanation".into())
            ]
        );
        assert_eq!(
            events
                .iter()
                .filter(|e| matches!(e, Event::EndBlock(_)))
                .count(),
            2
        );
        let bytes = encode(ClientProtocol::Responses, stream, events);
        let response = if stream {
            let packets = packets(&bytes);
            assert!(
                packets
                    .iter()
                    .any(|p| p["type"] == "response.reasoning_text.delta")
            );
            assert!(
                packets
                    .iter()
                    .any(|p| p["type"] == "response.reasoning_summary_text.delta")
            );
            packets.last().unwrap()["response"].clone()
        } else {
            serde_json::from_slice(&bytes).unwrap()
        };
        let items = response["output"].as_array().unwrap();
        let summaries: Vec<_> = items
            .iter()
            .flat_map(|v| v["summary"].as_array().unwrap())
            .collect();
        let raw: Vec<_> = items
            .iter()
            .filter_map(|v| v["content"].as_array())
            .flatten()
            .collect();
        assert_eq!(
            summaries,
            [&json!({"type":"summary_text","text":"short explanation"})]
        );
        assert_eq!(
            raw,
            [&json!({"type":"reasoning_text","text":"raw thought"})]
        );
    }
}

#[test]
fn gemini_summary_survives_native_and_responses_json_and_stream_outputs() {
    for stream in [false, true] {
        for client in [ClientProtocol::Gemini, ClientProtocol::Responses] {
            let mut decoder = Decoder::new(UpstreamProtocol::Gemini, &Limits::default());
            let body = json!({"responseId":"gemini_up","modelVersion":"m","candidates":[{
                "index":0,"content":{"role":"model","parts":[
                    {"thought":true,"text":"explanation","thoughtSignature":"opaque"},
                    {"text":"answer"}
                ]},"finishReason":"STOP"
            }]});
            let events = if stream {
                let mut events = frame(&mut decoder, body).unwrap();
                events.extend(decoder.eof().unwrap());
                events
            } else {
                decoder.json(&body).unwrap()
            };
            assert_eq!(
                text_blocks(&events),
                [("summary", "explanation".into()), ("text", "answer".into())]
            );
            let bytes = encode(client, stream, events);
            let responses = if stream {
                packets(&bytes)
            } else {
                vec![serde_json::from_slice(&bytes).unwrap()]
            };
            if client == ClientProtocol::Responses {
                let response = if stream {
                    &responses.last().unwrap()["response"]
                } else {
                    &responses[0]
                };
                assert_eq!(response["output"][0]["summary"][0]["text"], "explanation");
                assert!(
                    response["output"][0]["content"].is_null()
                        || response["output"][0]["content"] == json!([])
                );
                assert_eq!(
                    response["output"][0]["provider_metadata"]["protocol"],
                    "gemini"
                );
                assert_eq!(
                    response["output"][0]["provider_metadata"]["signature"],
                    "opaque"
                );
            } else {
                assert!(responses.iter().any(|response| {
                    response["candidates"][0]["content"]["parts"]
                        .as_array()
                        .is_some_and(|parts| {
                            parts.iter().any(|part| {
                                part["thought"] == true
                                    && part["text"] == "explanation"
                                    && part["thoughtSignature"] == "opaque"
                            })
                        })
                }));
            }
        }
    }
}

#[test]
fn unsupported_reasoning_kind_fails_before_a_successful_terminal() {
    for stream in [false, true] {
        for (client, kind) in [
            (ClientProtocol::Chat, BlockKind::ReasoningSummary),
            (ClientProtocol::Anthropic, BlockKind::ReasoningSummary),
            (ClientProtocol::Gemini, BlockKind::Thinking),
        ] {
            let mut encoder = encoder(client, stream);
            encoder.consume(Event::Start("up".into())).unwrap();
            let error = encoder
                .consume(Event::Block { index: 0, kind })
                .unwrap_err();
            assert!(matches!(error, RelayError::Unsupported(_)));
            assert!(encoder.terminal().is_err());
            let packet = encoder.error_packet(&error).unwrap();
            let text = std::str::from_utf8(&packet).unwrap();
            assert!(text.contains("error"));
            assert!(!text.contains("response.completed"));
            assert!(!text.contains("[DONE]"));
        }
    }
}

#[test]
fn provider_owned_anthropic_blocks_never_become_local_tools_or_success() {
    for block in [
        json!({"type":"server_tool_use","id":"srvtoolu_1","name":"web_search","input":{"query":"rust"}}),
        json!({"type":"web_search_tool_result","tool_use_id":"srvtoolu_1","content":[]}),
        json!({"type":"redacted_thinking","data":"opaque"}),
        json!({"type":"text","text":"answer","citations":[{"type":"web_search_result_location","url":"https://example.test","title":"source","cited_text":"quote","encrypted_index":"index"}]}),
    ] {
        for stream in [false, true] {
            let mut decoder = Decoder::new(UpstreamProtocol::Anthropic, &Limits::default());
            let mut output = encoder(ClientProtocol::Responses, stream);
            let mut sent = Vec::new();
            let error = if stream {
                let start = frame(&mut decoder, json!({"type":"message_start","message":{"id":"msg_up","role":"assistant","content":[],"usage":{"input_tokens":1,"output_tokens":0}}})).unwrap();
                for event in start {
                    sent.extend(output.consume(event).unwrap().concat());
                }
                frame(
                    &mut decoder,
                    json!({"type":"content_block_start","index":0,"content_block":block}),
                )
                .unwrap_err()
            } else {
                decoder.json(&json!({"id":"msg_up","type":"message","role":"assistant","model":"m","content":[block],"stop_reason":"end_turn","stop_sequence":null,"usage":{"input_tokens":1,"output_tokens":1}})).unwrap_err()
            };
            assert!(matches!(error, RelayError::Unsupported(_)), "{error:?}");
            assert!(!decoder.is_done());
            assert!(decoder.eof().is_err());
            assert!(output.terminal().is_err());
            sent.extend(output.error_packet(&error).unwrap());
            let error_packet = std::str::from_utf8(&sent).unwrap();
            assert!(!error_packet.contains("function_call"));
            assert!(!error_packet.contains("response.completed"));
        }
    }
}

#[test]
fn provider_usage_details_fail_at_full_start_and_final_receipt_boundaries() {
    for (field, value) in [
        ("server_tool_use", json!({"web_search_requests":1})),
        ("service_tier", json!("standard")),
        ("inference_geo", json!("us")),
        (
            "iterations",
            json!([{ "type":"message", "input_tokens":1, "output_tokens":1 }]),
        ),
    ] {
        let mut usage = json!({"input_tokens":1,"output_tokens":1});
        usage[field] = value;
        let mut decoder = Decoder::new(UpstreamProtocol::Anthropic, &Limits::default());
        let error = decoder.json(&json!({"id":"msg_up","type":"message","role":"assistant","content":[],"stop_reason":"end_turn","usage":usage})).unwrap_err();
        assert!(matches!(
            error,
            RelayError::Unsupported("provider-owned usage details")
        ));
        assert!(!decoder.is_done());
        let mut decoder = Decoder::new(UpstreamProtocol::Anthropic, &Limits::default());
        let error = frame(&mut decoder, json!({"type":"message_start","message":{"id":"msg_up","role":"assistant","content":[],"usage":usage}})).unwrap_err();
        assert!(matches!(
            error,
            RelayError::Unsupported("provider-owned usage details")
        ));
        assert!(!decoder.is_done());
        let mut decoder = Decoder::new(UpstreamProtocol::Anthropic, &Limits::default());
        frame(&mut decoder, json!({"type":"message_start","message":{"id":"msg_up","role":"assistant","content":[],"usage":{"input_tokens":1,"output_tokens":0}}})).unwrap();
        let error = frame(&mut decoder, json!({"type":"message_delta","delta":{"stop_reason":"end_turn","stop_sequence":null},"usage":usage})).unwrap_err();
        assert!(matches!(
            error,
            RelayError::Unsupported("provider-owned usage details")
        ));
        assert!(!decoder.is_done());
        assert!(decoder.eof().is_err());
    }
}

#[test]
fn provider_citation_delta_is_explicitly_rejected_without_completing() {
    let mut decoder = Decoder::new(UpstreamProtocol::Anthropic, &Limits::default());
    frame(&mut decoder, json!({"type":"message_start","message":{"id":"msg_up","role":"assistant","content":[],"usage":{"input_tokens":1,"output_tokens":0}}})).unwrap();
    frame(
        &mut decoder,
        json!({"type":"content_block_start","index":0,"content_block":{"type":"text","text":""}}),
    )
    .unwrap();
    let error = frame(&mut decoder, json!({"type":"content_block_delta","index":0,"delta":{"type":"citations_delta","citation":{"type":"web_search_result_location","url":"https://example.test","title":"source","cited_text":"quote","encrypted_index":"index"}}})).unwrap_err();
    assert!(matches!(
        error,
        RelayError::Unsupported("provider-owned output content")
    ));
    assert!(!decoder.is_done());
    assert!(decoder.eof().is_err());
}

#[test]
fn chat_reasoning_aliases_arrays_and_object_tool_arguments_use_library_normalization() {
    for field in ["reasoning", "reasoning_content"] {
        let mut message = json!({"role":"assistant","content":null,"tool_calls":[{"id":"call_1","type":"function","function":{"name":"search","arguments":{"query":"rust"}}}]});
        message[field] = json!(["first ", "second"]);
        let mut decoder = Decoder::new(UpstreamProtocol::Chat, &Limits::default());
        let events = decoder.json(&json!({"id":"chat_1","choices":[{"index":0,"message":message,"finish_reason":"tool_calls"}]})).unwrap();
        let blocks = text_blocks(&events);
        assert_eq!(blocks[0], ("raw", "first second".into()));
        assert_eq!(blocks[1].0, "tool");
        assert_eq!(
            serde_json::from_str::<Value>(&blocks[1].1).unwrap(),
            json!({"query":"rust"})
        );
        assert!(decoder.is_done());
    }
}

#[test]
fn gemini_signed_request_history_retains_summary_kind_and_provider_boundaries() {
    let request = super::native_gemini::parse(
        "m", false,
        br#"{"contents":[{"role":"model","parts":[{"text":"brief explanation","thought":true,"thoughtSignature":"opaque"}]},{"role":"user","parts":[{"text":"continue"}]}]}"#,
        &Limits::default(),
    ).unwrap();
    assert!(
        matches!(&request.messages[0].content[0], InputBlock::ReasoningSummary { signature: Some((UpstreamProtocol::Gemini, signature)), .. } if signature == "opaque")
    );
    let native = request.upstream_to(UpstreamProtocol::Gemini, "m").unwrap();
    assert_eq!(native["contents"][0]["parts"][0]["thought"], true);
    assert_eq!(
        native["contents"][0]["parts"][0]["text"],
        "brief explanation"
    );
    assert_eq!(
        native["contents"][0]["parts"][0]["thoughtSignature"],
        "opaque"
    );
    for target in [
        UpstreamProtocol::Chat,
        UpstreamProtocol::Anthropic,
        UpstreamProtocol::Responses,
    ] {
        assert!(matches!(
            request.upstream_to(target, "m"),
            Err(RelayError::Unsupported(_))
        ));
    }
}

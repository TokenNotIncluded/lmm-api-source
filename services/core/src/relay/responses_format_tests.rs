use super::*;

const SCHEMA: &str = r#"{"type":"object","properties":{"zeta":{"type":"string"},"alpha":{"type":"integer"}},"required":["zeta","alpha"],"additionalProperties":false}"#;

#[test]
fn responses_format_schema_roundtrips_across_openai_and_reaches_gemini() {
    for stream in [false, true] {
        let body = format!(
            r#"{{"model":"public","input":"answer as JSON","stream":{stream},"text":{{"format":{{"type":"json_schema","name":"answer","description":"Ordered answer","strict":true,"schema":{SCHEMA}}}}}}}"#
        );
        for request in [
            Request::parse(
                ClientProtocol::Responses,
                body.as_bytes(),
                &Limits::default(),
            )
            .unwrap(),
            Request::parse_deferred_openai(
                ClientProtocol::Responses,
                body.as_bytes(),
                &Limits::default(),
            )
            .unwrap(),
            Request::parse_responses_ws(body.as_bytes(), &Limits::default(), false, None).unwrap(),
        ] {
            for (protocol, prefix) in [
                (UpstreamProtocol::Responses, r#""schema":"#),
                (UpstreamProtocol::Chat, r#""schema":"#),
                (UpstreamProtocol::Gemini, r#""responseJsonSchema":"#),
            ] {
                let bytes = request.upstream_body_to(protocol, "supplier").unwrap();
                let text = String::from_utf8(bytes.clone()).unwrap();
                assert!(
                    text.contains(&format!("{prefix}{SCHEMA}")),
                    "{protocol:?}: {text}"
                );
                let wire: Value = serde_json::from_slice(&bytes).unwrap();
                if protocol == UpstreamProtocol::Gemini {
                    assert_eq!(
                        wire["generationConfig"]["responseMimeType"],
                        "application/json"
                    );
                    continue;
                }
                assert_eq!(wire["model"], "supplier");
                assert_eq!(wire["store"], false);
                assert_eq!(wire["stream"], stream);
                let client = if protocol == UpstreamProtocol::Responses {
                    ClientProtocol::Responses
                } else {
                    ClientProtocol::Chat
                };
                let back = Request::parse(client, &bytes, &Limits::default()).unwrap();
                let converted = back
                    .upstream_to(UpstreamProtocol::Responses, "public")
                    .unwrap();
                let original: Value = serde_json::from_str(&body).unwrap();
                assert_eq!(converted["text"], original["text"]);
            }
            assert!(
                request
                    .upstream_to(UpstreamProtocol::Anthropic, "supplier")
                    .is_err()
            );
            assert!(request.validate_exact_input_count().is_err());
        }
    }
}

#[test]
fn responses_format_text_and_json_object_convert_without_inventing_schema() {
    for kind in ["text", "json_object"] {
        let format = json!({"type":kind});
        let body = json!({"model":"public","input":"answer","text":{"format":format}});
        let request = Request::parse(
            ClientProtocol::Responses,
            &serde_json::to_vec(&body).unwrap(),
            &Limits::default(),
        )
        .unwrap();
        assert_eq!(
            request
                .upstream_to(UpstreamProtocol::Responses, "supplier")
                .unwrap()["text"]["format"],
            format
        );
        assert_eq!(
            request
                .upstream_to(UpstreamProtocol::Chat, "supplier")
                .unwrap()["response_format"],
            format
        );
        let chat = json!({"model":"public","messages":[{"role":"user","content":"answer"}],"response_format":format});
        let chat = Request::parse(
            ClientProtocol::Chat,
            &serde_json::to_vec(&chat).unwrap(),
            &Limits::default(),
        )
        .unwrap();
        assert_eq!(
            chat.upstream_to(UpstreamProtocol::Responses, "supplier")
                .unwrap()["text"]["format"],
            format
        );
    }
}

#[test]
fn responses_format_invalid_or_unrepresentable_controls_fail_explicitly() {
    for text in [
        json!({"verbosity":"low"}),
        json!({"format":{"type":"xml"}}),
        json!({"format":{"type":"text","schema":{}}}),
        json!({"format":{"type":"json_object","name":"ignored"}}),
        json!({"format":{"type":"json_schema","schema":{}}}),
        json!({"format":{"type":"json_schema","name":"bad name","schema":{}}}),
        json!({"format":{"type":"json_schema","name":"answer","schema":[]}}),
        json!({"format":{"type":"json_schema","name":"answer","schema":{},"strict":"yes"}}),
        json!({"format":{"type":"json_schema","name":"answer","schema":{},"extra":true}}),
    ] {
        let body = json!({"model":"public","input":"answer","text":text});
        assert!(
            Request::parse_deferred_openai(
                ClientProtocol::Responses,
                &serde_json::to_vec(&body).unwrap(),
                &Limits::default()
            )
            .is_err(),
            "{body}"
        );
    }
    for text in [Value::Null, json!({}), json!({"format":null})] {
        let body = json!({"model":"public","input":"answer","text":text});
        let request = Request::parse(
            ClientProtocol::Responses,
            &serde_json::to_vec(&body).unwrap(),
            &Limits::default(),
        )
        .unwrap();
        assert!(
            request
                .upstream_to(UpstreamProtocol::Responses, "supplier")
                .unwrap()
                .get("text")
                .is_none()
        );
    }
}

#[test]
fn responses_format_chat_schema_keeps_order_when_converted_to_responses() {
    let body = format!(
        r#"{{"model":"public","messages":[{{"role":"user","content":"answer"}}],"response_format":{{"type":"json_schema","json_schema":{{"name":"answer","strict":true,"schema":{SCHEMA}}}}}}}"#
    );
    let request =
        Request::parse_deferred_openai(ClientProtocol::Chat, body.as_bytes(), &Limits::default())
            .unwrap();
    for protocol in [UpstreamProtocol::Chat, UpstreamProtocol::Responses] {
        let bytes = request.upstream_body_to(protocol, "supplier").unwrap();
        let text = String::from_utf8(bytes).unwrap();
        assert!(text.contains(&format!(r#""schema":{SCHEMA}"#)), "{text}");
    }
}

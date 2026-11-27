use super::*;

const PROTOCOLS: [Protocol; 4] = [
    Protocol::OpenAiChat,
    Protocol::OpenAiResponses,
    Protocol::AnthropicMessages,
    Protocol::Gemini,
];

pub(super) fn openai_stream(protocol: Protocol, reply: &Value) -> Response {
    let mut data = String::new();
    if protocol == Protocol::OpenAiChat {
        let frames = [
            json!({"id":reply["id"],"choices":[{"index":0,"delta":{"role":"assistant","content":"native answer"}}]}),
            json!({"id":reply["id"],"choices":[{"index":0,"delta":{},"finish_reason":"stop"}]}),
            json!({"id":reply["id"],"choices":[],"usage":reply["usage"]}),
        ];
        for frame in frames {
            data.push_str(&format!("data: {frame}\n\n"));
        }
        data.push_str("data: [DONE]\n\n");
    } else {
        let msg = &reply["output"][0];
        let item_id = &msg["id"];
        let part = &msg["content"][0];
        let frames = [
            json!({"type":"response.created","response":{"id":reply["id"],"status":"in_progress"}}),
            json!({"type":"response.output_item.added","output_index":0,"item":{"type":"message","id":item_id,"role":"assistant","content":[]}}),
            json!({"type":"response.content_part.added","item_id":item_id,"output_index":0,"content_index":0,"part":{"type":"output_text","text":"","annotations":[]}}),
            json!({"type":"response.output_text.delta","item_id":item_id,"output_index":0,"content_index":0,"delta":part["text"]}),
            json!({"type":"response.content_part.done","item_id":item_id,"output_index":0,"content_index":0,"part":part}),
            json!({"type":"response.output_item.done","output_index":0,"item":msg}),
            json!({"type":"response.completed","response":reply}),
        ];
        for frame in frames {
            data.push_str(&format!(
                "event: {}\ndata: {frame}\n\n",
                frame["type"].as_str().unwrap()
            ));
        }
    }
    ([("content-type", "text/event-stream")], data).into_response()
}

fn client_input(protocol: Protocol, stream: bool) -> (&'static str, Value, &'static str) {
    match protocol {
        Protocol::OpenAiChat => (
            "/v1/chat/completions",
            json!({"model":"native-model","messages":[{"role":"user","content":"hello"}],
                "max_tokens":20,"stream":stream}),
            "authorization",
        ),
        Protocol::OpenAiResponses => (
            "/v1/responses",
            json!({"model":"native-model","input":"hello","max_output_tokens":20,"stream":stream}),
            "authorization",
        ),
        Protocol::AnthropicMessages | Protocol::Gemini => input(protocol, stream),
        _ => unreachable!(),
    }
}

pub(super) fn assert_supplier_text(protocol: Protocol, request: &Value) {
    let text = match protocol {
        Protocol::OpenAiChat => {
            assert_eq!(request["messages"][0]["role"], "user");
            let content = &request["messages"][0]["content"];
            content
                .as_str()
                .unwrap_or_else(|| content[0]["text"].as_str().unwrap())
        }
        Protocol::OpenAiResponses => request["input"][0]["content"][0]["text"].as_str().unwrap(),
        Protocol::AnthropicMessages => request["messages"][0]["content"][0]["text"]
            .as_str()
            .unwrap(),
        Protocol::Gemini => request["contents"][0]["parts"][0]["text"].as_str().unwrap(),
        _ => unreachable!(),
    };
    assert_eq!(text, "hello");
}

fn ingress_name(protocol: Protocol) -> &'static str {
    match protocol {
        Protocol::OpenAiChat => "chat",
        Protocol::OpenAiResponses => "responses",
        Protocol::AnthropicMessages => "anthropic",
        Protocol::Gemini => "gemini",
        _ => unreachable!(),
    }
}

fn assert_answer(client: Protocol, stream: bool, body: &str) {
    if stream {
        let frames: Vec<_> = body
            .lines()
            .filter_map(|line| line.strip_prefix("data:"))
            .map(str::trim_start)
            .collect();
        let values: Vec<Value> = frames
            .iter()
            .filter(|frame| **frame != "[DONE]")
            .map(|frame| serde_json::from_str(frame).unwrap())
            .collect();
        let terminal = match client {
            Protocol::OpenAiChat => {
                assert_eq!(frames.last(), Some(&"[DONE]"));
                values
                    .iter()
                    .any(|v| v["choices"][0]["finish_reason"] == "stop")
            }
            Protocol::OpenAiResponses => values
                .last()
                .is_some_and(|v| v["type"] == "response.completed"),
            Protocol::AnthropicMessages => {
                values.last().is_some_and(|v| v["type"] == "message_stop")
            }
            Protocol::Gemini => values
                .last()
                .is_some_and(|v| v["candidates"][0]["finishReason"] == "STOP"),
            _ => unreachable!(),
        };
        assert!(terminal, "{body}");
        assert!(
            values
                .iter()
                .any(|v| v.to_string().contains("native answer")),
            "{body}"
        );
        return;
    }
    let reply: Value = serde_json::from_str(body).unwrap();
    let (text, finish, expected) = match client {
        Protocol::OpenAiChat => (
            &reply["choices"][0]["message"]["content"],
            &reply["choices"][0]["finish_reason"],
            "stop",
        ),
        Protocol::OpenAiResponses => (
            &reply["output"][0]["content"][0]["text"],
            &reply["status"],
            "completed",
        ),
        Protocol::AnthropicMessages => (
            &reply["content"][0]["text"],
            &reply["stop_reason"],
            "end_turn",
        ),
        Protocol::Gemini => (
            &reply["candidates"][0]["content"]["parts"][0]["text"],
            &reply["candidates"][0]["finishReason"],
            "STOP",
        ),
        _ => unreachable!(),
    };
    assert_eq!(text, "native answer");
    assert_eq!(finish, expected);
}

#[sqlx::test(migrations = false)]
async fn public_four_by_four_text_json_and_sse(pool: PgPool) {
    let (_, ledger, _, key, key_id, account) = fixture(&pool, 10_000).await;
    let http = reqwest::Client::builder()
        .no_proxy()
        .timeout(Duration::from_secs(8))
        .build()
        .unwrap();
    let mut ids = HashSet::new();
    let mut settled = 0;
    for upstream in PROTOCOLS {
        let supplier = server(upstream, false).await;
        let app = gateway(&pool, &supplier, upstream, key_id).router();
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let origin = format!("http://{}", listener.local_addr().unwrap());
        let serving = tokio::spawn(async move {
            axum::serve(
                listener,
                app.into_make_service_with_connect_info::<std::net::SocketAddr>(),
            )
            .await
            .unwrap();
        });
        for client in PROTOCOLS {
            for stream in [false, true] {
                let (path, body, header) = client_input(client, stream);
                let credential = if header == "authorization" {
                    format!("Bearer {key}")
                } else {
                    key.clone()
                };
                let mut request = http
                    .post(format!("{origin}{path}"))
                    .header(header, credential)
                    .header("content-type", "application/json")
                    .header(
                        "idempotency-key",
                        format!("portable-{upstream:?}-{client:?}-{stream}"),
                    );
                if client == Protocol::AnthropicMessages {
                    request = request.header("anthropic-version", "2023-06-01");
                }
                let previous = supplier.hits.load(Ordering::SeqCst);
                let response = request.json(&body).send().await.unwrap();
                let status = response.status();
                let headers = response.headers().clone();
                let body = response.text().await.unwrap();
                if client == Protocol::AnthropicMessages && upstream == Protocol::Gemini {
                    assert_eq!(status, StatusCode::BAD_REQUEST, "{body}");
                    assert!(body.contains("Gemini usageMetadata does not report cache creation for Anthropic input_tokens"), "{body}");
                    assert_eq!(supplier.hits.load(Ordering::SeqCst), previous);
                    println!(
                        "portable_protocol_case client={client:?} supplier={upstream:?} stream={stream} rejection=unrepresentable_cache_usage supplier_hits=0"
                    );
                    continue;
                }
                assert_eq!(
                    status,
                    StatusCode::OK,
                    "{client:?} -> {upstream:?}, stream={stream}: {body}"
                );
                assert!(
                    headers["content-type"]
                        .to_str()
                        .unwrap()
                        .starts_with(if stream {
                            "text/event-stream"
                        } else {
                            "application/json"
                        })
                );
                assert_answer(client, stream, &body);
                assert_eq!(supplier.hits.load(Ordering::SeqCst), previous + 1);
                {
                    let requests = supplier.requests.lock().unwrap();
                    assert_supplier_text(upstream, requests.last().unwrap());
                }
                let id = headers["x-request-id"].to_str().unwrap().to_owned();
                assert!(ids.insert(id.clone()));
                let proof: (String, i64, i64, i64, String) = sqlx::query_as(
                    "SELECT r.state,r.settled,r.input_tokens,r.output_tokens,e.detail->>'inbound_protocol' \
                     FROM core_charging.requests r JOIN core_charging.usage_events e \
                     ON e.request_id=r.id AND e.event_id='route-dispatch:1' WHERE r.id=$1",
                ).bind(&id).fetch_one(&pool).await.unwrap();
                assert_eq!(
                    proof,
                    ("settled".into(), 49, 10, 20, ingress_name(client).into())
                );
                official_publication::check_attempt(&pool, &id, 1, upstream).await;
                println!(
                    "portable_protocol_case client={client:?} supplier={upstream:?} stream={stream} request_id={id} settled_credits=49"
                );
                settled += 49;
            }
        }
        serving.abort();
    }
    assert_eq!(ids.len(), 30);
    let balance = ledger.balance(account).await.unwrap();
    assert_eq!(balance.available_units, 10_000 - settled);
    assert_eq!(balance.reserved_units, 0);
}

#[sqlx::test(migrations = false)]
async fn unsupported_gemini_operations_fields_and_grounding_do_not_dispatch(pool: PgPool) {
    let (_, ledger, _, key, key_id, account) = fixture(&pool, 1000).await;
    let supplier = server(Protocol::OpenAiChat, false).await;
    let app = gateway(&pool, &supplier, Protocol::OpenAiChat, key_id).router();
    let (path, good, _) = input(Protocol::Gemini, false);
    let mut unknown = good.clone();
    unknown["safetySettings"] = json!([]);
    let mut grounding = good.clone();
    grounding["tools"] = json!([{"googleSearch":{}}]);
    let cases = [
        (
            "/v1beta/models/native-model:embedContent",
            good.clone(),
            "unsupported Gemini model operation",
        ),
        (
            "/v1beta/models/native-model:generateContent?alt=sse",
            good,
            "invalid Gemini model operation or query",
        ),
        (path, unknown, "unsupported Gemini field"),
        (path, grounding, "Google Search requires native Gemini"),
    ];
    for (path, body, reason) in cases {
        let response = app
            .clone()
            .oneshot(
                Request::post(path)
                    .header("x-goog-api-key", &key)
                    .header("content-type", "application/json")
                    .body(Body::from(body.to_string()))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::BAD_REQUEST);
        let err: Value =
            serde_json::from_slice(&to_bytes(response.into_body(), 8192).await.unwrap()).unwrap();
        assert_eq!(err["error"]["status"], "INVALID_ARGUMENT");
        assert!(
            err["error"]["message"].as_str().unwrap().contains(reason),
            "{err}"
        );
        assert_eq!(supplier.hits.load(Ordering::SeqCst), 0);
    }
    let balance = ledger.balance(account).await.unwrap();
    assert_eq!((balance.available_units, balance.reserved_units), (1000, 0));
}

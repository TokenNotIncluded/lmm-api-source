//! Measured request diagnostics through a real loopback supplier and PostgreSQL.
use super::*;
use axum::body::Bytes;
use tokio::sync::mpsc;
use tokio_stream::wrappers::ReceiverStream;

async fn supplier(Json(request): Json<Value>) -> Response {
    let prompt = match &request["messages"][0]["content"] {
        Value::String(text) => text.clone(),
        Value::Array(parts) => parts
            .iter()
            .filter_map(|part| part["text"].as_str())
            .collect(),
        _ => panic!("synthetic supplier requires text"),
    };
    if prompt == "reject" {
        return (
            StatusCode::TOO_MANY_REQUESTS,
            [("x-request-id", "req_rejected")],
            Json(json!({"error":{"message":"synthetic refusal"}})),
        )
            .into_response();
    }
    if request["stream"] == true {
        let cancelled = prompt == "cancel";
        let malformed = prompt == "malformed-frame";
        let (sender, receiver) = mpsc::channel::<Result<Bytes, std::io::Error>>(4);
        tokio::spawn(async move {
            let _ = sender
                .send(Ok(Bytes::from_static(b": keepalive\n\n")))
                .await;
            tokio::time::sleep(Duration::from_millis(25)).await;
            let content = format!(
                "data: {}\n\n",
                json!({"id":"response-body-id",
                "choices":[{"index":0,"delta":{"role":"assistant","content":"measured output"},"finish_reason":null}]})
            );
            if sender.send(Ok(Bytes::from(content))).await.is_err() {
                return;
            }
            if malformed {
                let _ = sender.send(Ok(Bytes::from_static(
                    br#"event: synthetic-private-event
data: {"id":"response-body-id","provider":"synthetic-private-body","choices":[{"index":0,"delta":{"content":"never-in-a-log"},"finish_reason":null}]}

"#
                ))).await;
                return;
            }
            if cancelled {
                sender.closed().await;
                return;
            }
            let terminal = format!(
                "data: {}\n\ndata: [DONE]\n\n",
                json!({"id":"response-body-id",
                "choices":[{"index":0,"delta":{},"finish_reason":"stop"}],
                "usage":{"prompt_tokens":10,"completion_tokens":20,"prompt_tokens_details":{"cached_tokens":2}}})
            );
            let _ = sender.send(Ok(Bytes::from(terminal))).await;
        });
        return (
            [
                ("content-type", "text/event-stream"),
                ("x-request-id", "req_stream"),
            ],
            Body::from_stream(ReceiverStream::new(receiver)),
        )
            .into_response();
    }
    (
        [("x-request-id", "req_nonstream"), ("set-cookie", "must-not-be-recorded")],
        Json(json!({"id":"response-body-id", "object":"chat.completion", "model":"provider-model",
            "choices":[{"index":0,"message":{"role":"assistant","content":"measured output"},"finish_reason":"stop"}],
            "usage":{"prompt_tokens":10,"completion_tokens":20,"prompt_tokens_details":{"cached_tokens":2}}})),
    ).into_response()
}

async fn sample(pool: &PgPool, request_id: &str) -> Value {
    tokio::time::timeout(Duration::from_secs(3), async {
        loop {
            if let Some(value) = sqlx::query_scalar::<_, Value>(
                "SELECT detail FROM core_charging.usage_events WHERE request_id=$1 AND event_id='performance-v1'",
            ).bind(request_id).fetch_optional(pool).await.unwrap() {
                return value;
            }
            tokio::time::sleep(Duration::from_millis(5)).await;
        }
    }).await.expect("request observation did not persist")
}

#[sqlx::test(migrations = false)]
async fn diagnostics_cover_success_rejection_disconnect_and_failed_telemetry(pool: PgPool) {
    let (_, ledger, _, key, key_id, account) = fixture(&pool, 1000).await;
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let endpoint = format!("http://{}/v1", listener.local_addr().unwrap());
    let server = tokio::spawn(async move {
        axum::serve(
            listener,
            Router::new().route("/v1/chat/completions", post(supplier)),
        )
        .await
        .unwrap();
    });
    let gateway = gateway_with_routes(&pool, routes(&endpoint, 0, 0, PayloadPolicy::Adapt), key_id)
        .with_local_streaming();

    let (status, _, id) = call_with_request_id(
        &gateway,
        Some(&key),
        "/v1/chat/completions",
        chat(),
        Some("diagnostic-normal"),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    let normal = sample(&pool, id.as_deref().unwrap()).await;
    assert_eq!(normal["is_stream"], false);
    assert_eq!(normal["status_code"], 200);
    assert_eq!(normal["upstream_request_id"], "req_nonstream");
    assert!(normal["first_output_us"].is_null());
    assert!(normal["elapsed_us"].as_u64().unwrap() > 0);
    assert_eq!(normal["success"], true);
    assert!(!normal.to_string().contains("must-not-be-recorded"));
    assert!(!normal.to_string().contains("synthetic-upstream-key"));
    assert!(!normal.to_string().contains("measured output"));

    let response = stream_response(
        &gateway,
        &key,
        "/v1/chat/completions",
        "hello",
        "diagnostic-stream",
    )
    .await;
    let id = response.headers()["x-request-id"]
        .to_str()
        .unwrap()
        .to_owned();
    let bytes = to_bytes(response.into_body(), 1_000_000).await.unwrap();
    assert!(String::from_utf8_lossy(&bytes).contains("[DONE]"));
    let streaming = sample(&pool, &id).await;
    assert_eq!(streaming["is_stream"], true);
    assert_eq!(streaming["upstream_request_id"], "req_stream");
    let first = streaming["first_output_us"].as_u64().unwrap();
    assert!(first >= 20_000, "keepalive must not count as first output");
    assert!(first <= streaming["elapsed_us"].as_u64().unwrap());

    let mut rejected = chat();
    rejected["messages"][0]["content"] = json!("reject");
    let (status, body, id) = call_with_request_id(
        &gateway,
        Some(&key),
        "/v1/chat/completions",
        rejected,
        Some("diagnostic-rejected"),
    )
    .await;
    // Supplier refusal is a gateway failure; diagnostics retain its actual 429.
    assert_eq!(status, StatusCode::BAD_GATEWAY);
    assert_eq!(body["error"]["code"], "upstream_error");
    let rejected = sample(&pool, id.as_deref().unwrap()).await;
    assert_eq!(rejected["status_code"], 429);
    assert_eq!(rejected["upstream_request_id"], "req_rejected");
    assert_eq!(rejected["success"], false);
    assert!(rejected["first_output_us"].is_null());

    // A failed observation must not roll back or repeat a successful settlement.
    sqlx::raw_sql("CREATE SEQUENCE public.diagnostic_rejections; CREATE FUNCTION public.reject_diagnostic_sample() RETURNS trigger LANGUAGE plpgsql AS $$ BEGIN IF NEW.event_id='performance-v1' THEN PERFORM nextval('public.diagnostic_rejections'); RAISE EXCEPTION 'synthetic diagnostic failure'; END IF; RETURN NEW; END $$; CREATE TRIGGER reject_diagnostic_sample BEFORE INSERT ON core_charging.usage_events FOR EACH ROW EXECUTE FUNCTION public.reject_diagnostic_sample()")
        .execute(&pool).await.unwrap();
    let before = ledger.balance(account).await.unwrap();
    let (status, _, id) = call_with_request_id(
        &gateway,
        Some(&key),
        "/v1/chat/completions",
        chat(),
        Some("diagnostic-write-failed"),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    let after = ledger.balance(account).await.unwrap();
    assert_eq!(before.available_units - after.available_units, 49);
    assert_eq!(before.reserved_units, after.reserved_units);
    // Sequence advancement survives the deliberately rolled-back observation,
    // proving the real writer reached the rejection without a timing guess.
    tokio::time::timeout(Duration::from_secs(3), async {
        loop {
            if sqlx::query_scalar::<_, bool>("SELECT is_called FROM public.diagnostic_rejections")
                .fetch_one(&pool)
                .await
                .unwrap()
            {
                break;
            }
            tokio::time::sleep(Duration::from_millis(5)).await;
        }
    })
    .await
    .expect("diagnostic write was not attempted");
    let count: i64 = sqlx::query_scalar("SELECT count(*) FROM core_charging.usage_events WHERE request_id=$1 AND event_id='performance-v1'")
        .bind(id.as_deref().unwrap()).fetch_one(&pool).await.unwrap();
    assert_eq!(count, 0);
    sqlx::query("DROP TRIGGER reject_diagnostic_sample ON core_charging.usage_events")
        .execute(&pool)
        .await
        .unwrap();

    let response = stream_response(
        &gateway,
        &key,
        "/v1/chat/completions",
        "cancel",
        "diagnostic-disconnect",
    )
    .await;
    let id = response.headers()["x-request-id"]
        .to_str()
        .unwrap()
        .to_owned();
    let mut body = response.into_body().into_data_stream();
    while let Some(Ok(bytes)) = body.next().await {
        if String::from_utf8_lossy(&bytes).contains("measured output") {
            break;
        }
    }
    drop(body);
    assert_eq!(gateway.drain(Duration::from_secs(2)).await.remaining, 0);
    let cancelled = sample(&pool, &id).await;
    assert_eq!(cancelled["is_stream"], true);
    assert_eq!(cancelled["status_code"], 200);
    assert_eq!(cancelled["success"], false);
    assert!(cancelled["first_output_us"].is_number());
    assert_eq!(cancelled["upstream_request_id"], "req_stream");
    assert_unknown_hold(&pool, &id, 140).await;
    balanced(&pool).await;
    server.abort();
}

#[sqlx::test(migrations = false)]
async fn frame_observation_uses_the_real_protocol_pair_and_does_not_retain_supplier_text(
    pool: PgPool,
) {
    let (_, _, _, key, key_id, _) = fixture(&pool, 1000).await;
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let endpoint = format!("http://{}/v1", listener.local_addr().unwrap());
    let server = tokio::spawn(async move {
        axum::serve(
            listener,
            Router::new().route("/v1/chat/completions", post(supplier)),
        )
        .await
        .unwrap()
    });
    let gateway = gateway_with_routes(&pool, routes(&endpoint, 0, 0, PayloadPolicy::Adapt), key_id)
        .with_local_streaming();
    let response = stream_response(
        &gateway,
        &key,
        "/v1/responses",
        "malformed-frame",
        "frame-observation",
    )
    .await;
    assert_eq!(response.status(), StatusCode::OK);
    let id = response.headers()["x-request-id"]
        .to_str()
        .unwrap()
        .to_owned();
    let _ = to_bytes(response.into_body(), 1_000_000).await;
    sample(&pool, &id).await;
    let observation: Value = sqlx::query_scalar(
        "SELECT detail FROM core_charging.usage_events WHERE request_id=$1 AND event_id='relay-protocol:v1'",
    ).bind(&id).fetch_one(&pool).await.unwrap();
    assert_eq!(observation["inbound_protocol"], "responses");
    assert_eq!(observation["error_code"], "invalid_upstream_response");
    assert_eq!(observation["error_stage"], "sse_decode");
    assert_eq!(observation["sse_frame"]["ordinal"], 2);
    assert_eq!(observation["sse_frame"]["event"], "other");
    assert_eq!(observation["sse_frame"]["channel_id"], 1);
    assert_eq!(observation["sse_frame"]["upstream_protocol"], "chat");
    assert!(
        observation["sse_frame"]["converter"]
            .as_str()
            .is_some_and(|variant| !variant.is_empty() && variant != "Other")
    );
    for forbidden in [
        "synthetic-private",
        "never-in-a-log",
        "malformed-frame",
        "measured output",
        "http://",
    ] {
        assert!(!observation.to_string().contains(forbidden));
    }
    server.abort();
}

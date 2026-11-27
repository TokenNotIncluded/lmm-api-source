// Copyright (C) 2026 LIghtJUNction. SPDX-License-Identifier: AGPL-3.0-or-later.
use super::*;
use axum::{body::Bytes, extract::OriginalUri};

/// Capture the entity received on the supplier's TCP socket before decoding JSON.
async fn capture(
    State((provider, captures)): State<(Supplier, Captures)>,
    OriginalUri(uri): OriginalUri,
    headers: HeaderMap,
    body: Bytes,
) -> Response {
    captures.lock().unwrap().push(body.clone());
    let request: Value = serde_json::from_slice(&body).unwrap();
    let stream = uri.path().ends_with(":streamGenerateContent");
    let evidence = match request["contents"][0]["parts"][0]["text"].as_str() {
        Some("confirmed") => Some(("priority", json!("priority"))),
        Some("confirmed-downgrade") => Some(("standard", json!("standard"))),
        Some("malformed-receipt") => Some(("priority", json!(17))),
        _ => None,
    };
    if let Some((tier_header, body_tier)) = evidence {
        let response = supplier(State(provider), headers, Json(request)).await;
        let bytes = to_bytes(response.into_body(), 100_000).await.unwrap();
        let mut receipt: Value = serde_json::from_slice(&bytes).unwrap();
        receipt["usageMetadata"]["serviceTier"] = body_tier;
        if stream {
            return ([
                ("content-type", "text/event-stream"),
                ("x-gemini-service-tier", tier_header),
            ], format!("data: {receipt}\n\n")).into_response();
        }
        return ([("x-gemini-service-tier", tier_header)], Json(receipt)).into_response();
    }
    if stream {
        gemini_stream(State(provider), headers, Json(request)).await
    } else {
        supplier(State(provider), headers, Json(request)).await
    }
}

type Captures = Arc<Mutex<Vec<Bytes>>>;

async fn capturing_supplier() -> (Server, Captures) {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let endpoint = format!("http://{}/v1", listener.local_addr().unwrap());
    let captures = Captures::default();
    let provider = Supplier {
        protocol: Protocol::Gemini,
        hits: Default::default(),
        requests: Default::default(),
        missing_usage: false,
    };
    let app = Router::new()
        .route("/v1/models/provider-model:generateContent", post(capture))
        .route(
            "/v1/models/provider-model:streamGenerateContent",
            post(capture),
        )
        .with_state((provider.clone(), captures.clone()));
    let task = tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
    (
        Server {
            endpoint,
            hits: provider.hits,
            requests: provider.requests,
            task,
        },
        captures,
    )
}

#[sqlx::test(migrations = false)]
async fn native_gemini_ignores_top_level_tier_spellings_and_preserves_supplier_bytes(pool: PgPool) {
    let (_, _, _, key, key_id, _) = fixture(&pool, 10_000).await;
    let (server, captures) = capturing_supplier().await;
    let gateway = gateway(&pool, &server, Protocol::Gemini, key_id);

    // Both strings are also valid user-defined schema property names. Their
    // non-alphabetic order is part of the existing native Gemini wire contract.
    const FIELDS: &str = r#"
        "contents":[{"role":"user","parts":[{"text":"keep serviceTier and service_tier in text"}]}],
        "tools":[{"functionDeclarations":[{"name":"inspect","parametersJsonSchema":{
            "type":"object","properties":{"service_tier":{"type":"string"},"serviceTier":{"type":"number"},"a":{"type":"boolean"}}
        }}]}],
        "generationConfig":{"maxOutputTokens":20}"#;
    for stream in [false, true] {
        let (path, _, header) = input(Protocol::Gemini, stream);
        let mut baseline = None;
        for (index, tier) in [
            "",
            r#""serviceTier":"priority","#,
            r#""service_tier":"flex","#,
        ]
        .into_iter()
        .enumerate()
        {
            let response = gateway
                .router()
                .oneshot(
                    Request::builder()
                        .method("POST")
                        .uri(path)
                        .header(header, &key)
                        .header("content-type", "application/json")
                        .header("idempotency-key", format!("gemini-tier-{stream}-{index}"))
                        .body(Body::from(format!("{{{tier}{FIELDS}}}")))
                        .unwrap(),
                )
                .await
                .unwrap();
            let status = response.status();
            let reply = to_bytes(response.into_body(), 1_000_000).await.unwrap();
            assert_eq!(
                status,
                StatusCode::OK,
                "stream={stream} case={index}: {}",
                String::from_utf8_lossy(&reply)
            );
            assert!(String::from_utf8_lossy(&reply).contains("native answer"));

            let mut seen = std::mem::take(&mut *captures.lock().unwrap());
            assert_eq!(seen.len(), 1, "exactly one upstream call");
            let bytes = seen.pop().unwrap();
            let fields: Value = serde_json::from_slice(&bytes).unwrap();
            assert!(fields.get("serviceTier").is_none());
            assert!(fields.get("service_tier").is_none());
            assert_eq!(
                fields["contents"][0]["parts"][0]["text"],
                "keep serviceTier and service_tier in text"
            );
            let schema = r#""properties":{"service_tier":{"type":"string"},"serviceTier":{"type":"number"},"a":{"type":"boolean"}}"#;
            assert!(std::str::from_utf8(&bytes).unwrap().contains(schema));
            if let Some(ordinary) = &baseline {
                assert_eq!(
                    &bytes, ordinary,
                    "a top-level tier cannot alter the rest of the supplier entity"
                );
            } else {
                baseline = Some(bytes);
            }
        }
    }
}

#[sqlx::test(migrations = false)]
async fn native_gemini_priority_requires_matching_receipt_and_keeps_disputed_holds(pool: PgPool) {
    let (_, ledger, _, key, key_id, account) = fixture(&pool, 10_000).await;
    let (server, captures) = capturing_supplier().await;
    let gateway = gateway_with_priority(&pool, &server, Protocol::Gemini, key_id, true);
    let mut held = 0;
    let mut settled = 0;
    for (index, (stream, tier, label)) in [
        (false, json!({"invalid":"priority"}), "invalid-request"),
        (false, json!("priority"), "confirmed"),
        (true, json!("priority"), "confirmed"),
        (false, json!("priority"), "confirmed-downgrade"),
        (false, json!("priority"), "malformed-receipt"),
    ].into_iter().enumerate() {
        let (path, mut body, header) = input(Protocol::Gemini, stream);
        body["serviceTier"] = tier;
        body["contents"][0]["parts"][0]["text"] = json!(label);
        let response = gateway.router().oneshot(
            Request::builder().method("POST").uri(path)
                .header(header, &key)
                .header("content-type", "application/json")
                .header("idempotency-key", format!("gemini-priority-{index}"))
                .body(Body::from(serde_json::to_vec(&body).unwrap())).unwrap()
        ).await.unwrap();
        let status = response.status();
        let id = response.headers().get("x-request-id")
            .map(|v| v.to_str().unwrap().to_owned());
        let bytes = to_bytes(response.into_body(), 1_000_000).await.unwrap();
        let sent = std::mem::take(&mut *captures.lock().unwrap());
        if label == "invalid-request" {
            assert!(status.is_client_error(), "{status}: {}", String::from_utf8_lossy(&bytes));
            assert!(sent.is_empty());
            let n: i64 = sqlx::query_scalar("SELECT count(*) FROM core_charging.requests")
                .fetch_one(&pool).await.unwrap();
            assert_eq!(n, 0);
            continue;
        }
        assert_eq!(sent.len(), 1);
        let wire: Value = serde_json::from_slice(&sent[0]).unwrap();
        assert_eq!(wire["service_tier"], "priority");
        assert!(wire.get("serviceTier").is_none());
        assert!(wire.get("speed").is_none());
        assert_eq!(wire["contents"][0]["parts"][0]["text"], label);
        let id = id.expect("accepted supplier request ID");
        let (state, reserved, charge, input_rate, cache_rate, output_rate): (String, i64, i64, i64, i64, i64) =
            sqlx::query_as("SELECT state,reserved,settled,(price->>'input_per_million')::bigint,(price->>'cached_per_million')::bigint,(price->>'output_per_million')::bigint FROM core_charging.requests WHERE id=$1")
                .bind(&id).fetch_one(&pool).await.unwrap();
        // Frozen credit rates include both the 2x speed price and the 0.5 conversion.
        assert_eq!((input_rate, cache_rate, output_rate), (2_000_000, 500_000, 4_000_000));
        assert_eq!(reserved, 280);
        if label == "confirmed" {
            assert_eq!(status, StatusCode::OK, "{}", String::from_utf8_lossy(&bytes));
            assert!(String::from_utf8_lossy(&bytes).contains("native answer"));
            // 8 ordinary input, 2 cached, 20 output; rounded once after conversion.
            assert_eq!((state.as_str(), charge), ("settled", 97));
            settled += charge;
        } else {
            assert_eq!(status, StatusCode::BAD_GATEWAY, "{}", String::from_utf8_lossy(&bytes));
            assert_eq!((state.as_str(), charge), ("reconcile", 0));
            held += reserved;
        }
        let balance = ledger.balance(account).await.unwrap();
        assert_eq!(balance.available_units, 10_000 - settled - held);
        assert_eq!(balance.reserved_units, held);
    }
    assert_eq!(server.hits.load(Ordering::SeqCst), 4);
}

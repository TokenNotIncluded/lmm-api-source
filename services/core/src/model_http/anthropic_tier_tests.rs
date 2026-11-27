//! Actual Messages requests to an isolated loopback supplier, using the disposable SQLx wallet.
use super::*;
use axum::{Json, extract::State, routing::post};
use serde_json::Value;
use tower::ServiceExt;

fn native_gateway(f: &Fixture, endpoint: &str, honor: bool) -> ModelGateway {
    let base = gateway(f, endpoint, vec![]);
    let mut release = base.bridge.routing.catalog_document().unwrap();
    release.routing.global_model_policy = Some(
        serde_json::from_value(json!({
            "honor_request_service_tier":honor,
            "pass_through_request_enabled":true,"thinking_model_blacklist":[],
            "chat_completions_to_responses_policy":{},"ping_interval_enabled":false,
            "ping_interval_seconds":30
        }))
        .unwrap(),
    );
    release.routing.upstreams[0].protocol = Protocol::AnthropicMessages;
    release.routing.routes.truncate(1);
    let route = &mut release.routing.routes[0];
    route.speeds = vec![Speed::Standard, Speed::Fast, Speed::Ultrafast];
    route.targets[0].speeds = route.speeds.clone();
    route.targets[0].upstream_model = "claude-opus-5".into();

    let mut rule = release.pricing.rules.remove(0);
    rule.rates.cache_read = 1_000_000;
    release.pricing.rules = [
        (Speed::Standard, 1),
        (Speed::Fast, 2),
        (Speed::Ultrafast, 3),
    ]
    .into_iter()
    .map(|(speed, factor)| {
        let mut current = rule.clone();
        current.id = u64::from(factor);
        current.speed = speed;
        current.speed_multiplier_ppm = factor * MULTIPLIER_ONE;
        current
    })
    .collect();
    // Preserve the constructor-validated loopback Endpoint: deserializing an
    // http:// URL from externally supplied routing configuration is prohibited.
    ModelGateway::new_with_provider_policy(
        f.pool.clone(),
        Arc::new(release.into_router().unwrap()),
        vec![UpstreamCredential {
            reference: CredentialRef { id: 7, revision: 1 },
            secret: "synthetic-anthropic-tier".into(),
        }],
        BTreeMap::from([(f.key_id, "allowed".into())]),
        base.limits.clone(),
        base.bridge.policy,
        ProviderPolicy::new(vec![], GLOBAL_INPUT).unwrap(),
    )
    .unwrap()
    .with_local_streaming()
}

#[sqlx::test(migrations = false)]
async fn anthropic_messages_fast_pricing_matches_supplier_request_and_ledger(pool: PgPool) {
    type Captured = Arc<std::sync::Mutex<Vec<(Value, Option<String>)>>>;
    async fn supplier(
        State(captured): State<Captured>,
        headers: HeaderMap,
        Json(body): Json<Value>,
    ) -> Json<Value> {
        let speed = if body["metadata"]["user_id"] == "supplier-downgrade" {
            "standard"
        } else {
            body.get("speed")
                .and_then(Value::as_str)
                .unwrap_or("standard")
        }
        .to_owned();
        let beta = headers
            .get("anthropic-beta")
            .map(|value| value.to_str().unwrap().to_owned());
        captured.lock().unwrap().push((body, beta));
        Json(json!({
            "type":"message","id":"msg_fixture","model":"claude-opus-5","role":"assistant",
            "content":[{"type":"text","text":"answer"}],
            "stop_reason":"end_turn","stop_sequence":null,
            "usage":{"speed":speed,"service_tier":"standard","input_tokens":2,"cache_read_input_tokens":2,"cache_creation_input_tokens":0,"output_tokens":3}
        }))
    }

    let fixture = fixture(pool, 10_000_000).await;
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let endpoint = format!("http://{}/v1", listener.local_addr().unwrap());
    let captured = Captured::default();
    let provider = axum::Router::new()
        .route("/v1/messages", post(supplier))
        .with_state(captured.clone());
    let server = tokio::spawn(async move { axum::serve(listener, provider).await.unwrap() });
    let mut paid_requests = 0;
    let mut balance = 10_000_000;
    for (honor, hint, multiplier) in [
        (false, Some("fast"), Some(1_i64)),
        (true, None, Some(1)),
        (true, Some("fast"), Some(2)),
        (true, Some("ultrafast"), None),
    ] {
        let gateway = native_gateway(&fixture, &endpoint, honor);
        let mut body = json!({
            "model":"small-model","messages":[{"role":"user","content":"hello"}],"max_tokens":8,
            "metadata":{"user_id":"native-tier-fixture"}
        });
        if let Some(hint) = hint {
            body["speed"] = json!(hint);
        }
        if !honor {
            body["service_tier"] = json!("unsupported-fixture-value");
        }
        let response = gateway
            .router()
            .oneshot(
                axum::http::Request::post("/v1/messages")
                    .header("content-type", "application/json")
                    .header("anthropic-version", "2023-06-01")
                    .header("x-api-key", fixture.key.as_str())
                    .body(axum::body::Body::from(body.to_string()))
                    .unwrap(),
            )
            .await
            .unwrap();
        let status = response.status();
        let id = response
            .headers()
            .get("x-request-id")
            .and_then(|v| v.to_str().ok())
            .unwrap_or_default()
            .to_owned();
        let bytes = axum::body::to_bytes(response.into_body(), 1_048_576)
            .await
            .unwrap();
        if let Some(multiplier) = multiplier {
            assert_eq!(
                status,
                axum::http::StatusCode::OK,
                "{}",
                String::from_utf8_lossy(&bytes)
            );
            assert!(!id.is_empty());
            let decoded: Value = serde_json::from_slice(&bytes).unwrap();
            assert_eq!(decoded["content"][0]["text"], "answer");
            let (wire, beta) = captured.lock().unwrap()[paid_requests].clone();
            assert_eq!(wire["model"], "claude-opus-5");
            assert_eq!(
                beta.as_deref(),
                (multiplier == 2).then_some("fast-mode-2026-02-01")
            );
            assert_eq!(wire["metadata"]["user_id"], "native-tier-fixture");
            assert!(wire.get("service_tier").is_none());
            assert_eq!(
                wire.get("speed").and_then(Value::as_str),
                if multiplier == 2 { Some("fast") } else { None }
            );

            tokio::time::timeout(Duration::from_secs(10), async {
                while gateway.bridge.active.lock().unwrap().contains_key(&id) {
                    tokio::task::yield_now().await;
                }
            })
            .await
            .unwrap();
            let charge = receipt(&gateway, &id).await;
            assert_eq!(charge.state, "settled");
            assert_eq!(charge.reserved, GLOBAL_INPUT * multiplier);
            assert_eq!(charge.price.input_per_million, 1_000_000 * multiplier);
            assert_eq!(charge.price.cached_per_million, 500_000 * multiplier);
            assert_eq!(
                (charge.usage.input, charge.usage.cached, charge.usage.output),
                (4, 2, 3)
            );
            assert_eq!(charge.settled, 3 * multiplier);
            balance -= charge.settled;
            paid_requests += 1;
        } else {
            // An ultrafast price exists, but the native supplier has no matching wire contract.
            assert!(!status.is_success());
        }
        assert_eq!(captured.lock().unwrap().len(), paid_requests);
        assert_eq!(requests(&fixture.pool).await, paid_requests as i64);
        assert_balance(&fixture, balance, 0).await;
    }
    // A 200 response proves the supplier accepted this turn. A reported standard
    // speed contradicts the frozen fast quote and leaves the hold for reconciliation.
    let gateway = native_gateway(&fixture, &endpoint, true);
    let response = gateway.router().oneshot(
        axum::http::Request::post("/v1/messages")
            .header("content-type", "application/json")
            .header("anthropic-version", "2023-06-01")
            .header("x-api-key", fixture.key.as_str())
            .body(axum::body::Body::from(json!({
                "model":"small-model","speed":"fast","max_tokens":8,
                "messages":[{"role":"user","content":"hello"}],"metadata":{"user_id":"supplier-downgrade"}
            }).to_string())).unwrap()
    ).await.unwrap();
    assert!(!response.status().is_success());
    tokio::time::timeout(Duration::from_secs(10), async {
        while !gateway.bridge.active.lock().unwrap().is_empty() {
            tokio::task::yield_now().await;
        }
    })
    .await
    .unwrap();
    assert_eq!(captured.lock().unwrap().len(), paid_requests + 1);
    let held: (String, i64, i64) = sqlx::query_as(
        "SELECT state,reserved,settled FROM core_charging.requests WHERE state <> 'settled'",
    )
    .fetch_one(&fixture.pool)
    .await
    .unwrap();
    assert_eq!(held, ("reconcile".into(), 2 * GLOBAL_INPUT, 0));
    assert_balance(&fixture, balance - 2 * GLOBAL_INPUT, 2 * GLOBAL_INPUT).await;
    server.abort();
}

//! Real PostgreSQL and controlled native HTTP receipts; no live paid provider.
use super::*;
use crate::{
    routing::{self, CredentialRef, Endpoint, PriceConfig, PriceSnapshot, Protocol, RoutingConfig},
    test_runtime::RuntimeRole,
};
use axum::response::IntoResponse;
use axum::{extract::State, http::Request as HttpRequest, routing::post};
use futures_util::StreamExt;
use serde_json::{Value, json};
use std::sync::{
    Arc, Mutex,
    atomic::{AtomicUsize, Ordering},
};
use tokio::sync::Notify;
use tower::ServiceExt;

#[derive(Clone, Copy)]
enum Reply {
    Valid,
    PrettyJson,
    MissingUsage,
    ExcessUsage,
    InvalidTerminal,
    TailAfterTerminal,
    SecondTerminal,
    NeverEof,
}
#[derive(Clone)]
struct Provider {
    base: String,
    hits: Arc<AtomicUsize>,
    seen: Arc<Mutex<Vec<Value>>>,
    reply: Reply,
    pause: Option<(Arc<Notify>, Arc<Notify>)>,
}
async fn reply(State(p): State<Provider>, headers: HeaderMap, Json(body): Json<Value>) -> Response {
    assert_eq!(
        headers[header::AUTHORIZATION],
        "Bearer synthetic-compact-provider"
    );
    assert_eq!(body["model"], "native-model");
    p.hits.fetch_add(1, Ordering::SeqCst);
    p.seen.lock().unwrap().push(body.clone());
    if body["stream"] != true {
        if let Some((entered, resume)) = &p.pause {
            entered.notify_one();
            resume.notified().await;
        }
    }
    let compact = body.get("max_output_tokens").is_none();
    let mut output = if compact {
        json!({"id":"cmp_synthetic", "object":"response.compaction", "created_at":1,
            "output":[{"type":"compaction","id":"cmp_item","encrypted_content":"opaque".repeat(20000)}],
            "usage":{"input_tokens":17,"output_tokens":3,"total_tokens":20,"input_tokens_details":{"cached_tokens":0}}})
    } else {
        assert_eq!(
            body["input"][0]["encrypted_content"],
            "opaque".repeat(20000)
        );
        assert_eq!(body["max_output_tokens"], 100);
        assert_eq!(body["store"], false);
        json!({"id":"resp_synthetic","object":"response","status":"completed","model":"native-model",
            "output":[{"type":"message","id":"msg1","role":"assistant","content":[{"type":"output_text","text":"continued","annotations":[]}]}],
            "usage":{"input_tokens":23,"output_tokens":4,"total_tokens":27,"input_tokens_details":{"cached_tokens":0}}})
    };
    match p.reply {
        Reply::Valid
        | Reply::PrettyJson
        | Reply::TailAfterTerminal
        | Reply::SecondTerminal
        | Reply::NeverEof => (),
        Reply::MissingUsage => {
            output.as_object_mut().unwrap().remove("usage");
        }
        Reply::ExcessUsage => {
            output["usage"]["input_tokens"] = 1001.into();
            output["usage"]["total_tokens"] = 1004.into();
        }
        Reply::InvalidTerminal => {
            output["object"] = "response".into();
        }
    }
    if body["stream"] == true {
        assert_eq!(body["tools"][0]["type"], "function");
        assert_eq!(body["include"], json!(["reasoning.encrypted_content"]));
        output["output"] = json!([{"type":"function_call","id":"fc1","call_id":"call1","name":"lookup","arguments":"{\"q\":\"next\"}"}]);
        let first = format!(
            "event: response.created\ndata: {}\n\nevent: response.output_text.delta\ndata: {}\n\n",
            json!({"type":"response.created","response":{"id":"resp_synthetic","status":"in_progress"}}),
            json!({"type":"response.output_text.delta","delta":"live delta"})
        );
        let terminal = format!(
            "event: response.completed\ndata: {}\n\n",
            json!({"type":"response.completed","response":output})
        );
        let tail = match p.reply {
            Reply::TailAfterTerminal => "invalid-tail".to_owned(),
            Reply::SecondTerminal => terminal.clone(),
            _ => String::new(),
        };
        let never_eof = matches!(p.reply, Reply::NeverEof);
        let initial = futures_util::stream::iter(vec![Ok::<_, std::io::Error>(Bytes::from(first))]);
        let last = futures_util::stream::once(async move {
            if let Some((entered, resume)) = p.pause {
                entered.notify_one();
                resume.notified().await;
            }
            Ok::<_, std::io::Error>(Bytes::from(terminal))
        });
        return (
            [(header::CONTENT_TYPE, "text/event-stream")],
            Body::from_stream(
                initial
                    .chain(last)
                    .chain(futures_util::stream::once(async move {
                        if never_eof {
                            std::future::pending::<()>().await;
                        }
                        Ok::<_, std::io::Error>(Bytes::from(tail))
                    })),
            ),
        )
            .into_response();
    }
    if matches!(p.reply, Reply::PrettyJson) {
        return (
            [(header::CONTENT_TYPE, "application/json")],
            serde_json::to_string_pretty(&output).unwrap(),
        )
            .into_response();
    }
    Json(output).into_response()
}
async fn supplier(mode: Reply, paused: bool) -> (Provider, tokio::task::JoinHandle<()>) {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let p = Provider {
        base: format!("http://{}/v1", listener.local_addr().unwrap()),
        hits: Arc::new(AtomicUsize::new(0)),
        seen: Arc::new(Mutex::new(Vec::new())),
        reply: mode,
        pause: paused.then(|| (Arc::new(Notify::new()), Arc::new(Notify::new()))),
    };
    let app = Router::new()
        .route("/v1/responses/compact", post(reply))
        .route("/v1/responses", post(reply))
        .with_state(p.clone());
    let task = tokio::spawn(async move {
        axum::serve(listener, app).await.unwrap();
    });
    (p, task)
}
fn gateway(pool: &sqlx::PgPool, p: &Provider, enabled: bool, protocol: Protocol) -> ModelGateway {
    gateway_with_reservation(pool, p, enabled, protocol, Default::default())
}
fn gateway_with_reservation(
    pool: &sqlx::PgPool,
    p: &Provider,
    enabled: bool,
    protocol: Protocol,
    reservation: super::super::provider_policy::ReservationProfile,
) -> ModelGateway {
    let model = if matches!(
        reservation,
        super::super::provider_policy::ReservationProfile::OpenaiResponsesExactV1 { .. }
    ) {
        "gpt-4.1-2025-04-14"
    } else {
        "native-model"
    };
    let mut config: RoutingConfig = serde_json::from_value(json!({"version":1,"groups":[{"name":"allowed","enabled":true}],"aliases":[],
        "upstreams":[{"id":1,"endpoint":"https://synthetic.invalid/v1","protocol":protocol,"credential":{"id":7,"revision":1},"enabled":true}],
        "routes":[{"id":1,"model":"compact-public","group":"allowed","enabled":true,"speeds":["standard"],
            "targets":[{"id":1,"upstream_id":1,"upstream_model":model,"enabled":true,"priority":0,"weight":1,"speeds":["standard"],"payload":"adapt","timeout_ms":10000}],
            "failover":{"max_attempts":1,"retry_on":[],"allow_replay_after_send":false}}]})).unwrap();
    config.upstreams[0].endpoint = Endpoint::local_loopback(p.base.clone()).unwrap();
    let prices: PriceConfig = serde_json::from_value(json!({"version":1,"rules":[{"id":1,"model":"compact-public","group":"allowed","speed":"standard",
        "rates":{"input":2000000,"output":4000000,"cache_read":2000000,"cache_write":0,"request":0},"group_multiplier_ppm":1000000,"speed_multiplier_ppm":1000000}]})).unwrap();
    ModelGateway::new_with_provider_policy(
        pool.clone(),
        Arc::new(
            routing::Router::new(config, Arc::new(PriceSnapshot::compile(prices).unwrap()))
                .unwrap(),
        ),
        vec![UpstreamCredential {
            reference: CredentialRef { id: 7, revision: 1 },
            secret: "synthetic-compact-provider".into(),
        }],
        BTreeMap::new(),
        Limits {
            max_output_tokens: 100,
            total_timeout: std::time::Duration::from_secs(5),
            header_timeout: std::time::Duration::from_secs(4),
            ..Default::default()
        },
        ChargePolicy {
            input_token_ceiling: 1000,
            lease_seconds: 30,
        },
        ProviderPolicy::new(
            vec![ExternalModelContext {
                responses_compaction: enabled,
                upstream_id: 1,
                upstream_model: model.into(),
                protocol,
                endpoint: if matches!(
                    reservation,
                    super::super::provider_policy::ReservationProfile::OpenaiResponsesExactV1 { .. }
                ) {
                    "https://api.openai.com/v1".into()
                } else {
                    "https://compact.synthetic.example/v1".into()
                },
                max_input_tokens: 1000,
                max_output_tokens: 100,
                contract_revision: "synthetic-compaction-context-v1".into(),
                missing_cached_tokens_is_zero: false,
                missing_cache_creation_tokens_is_zero: false,
                reservation,
                grounding: None,
                pre_acceptance_rejection_statuses: vec![],
            }],
            1000,
        )
        .unwrap(),
    )
    .unwrap()
}
async fn call(
    g: &ModelGateway,
    key: &str,
    id: &str,
    compact: bool,
    input: Value,
) -> (StatusCode, String, Value) {
    let r = HttpRequest::builder()
        .method("POST")
        .uri(if compact {
            "/v1/responses/compact"
        } else {
            "/v1/responses"
        })
        .header(header::AUTHORIZATION, format!("Bearer {key}"))
        .header(header::CONTENT_TYPE, "application/json")
        .header("idempotency-key", id)
        .body(Body::from(
            json!({"model":"compact-public","input":input}).to_string(),
        ))
        .unwrap();
    let r = g.router().oneshot(r).await.unwrap();
    let status = r.status();
    let id = r
        .headers()
        .get("x-request-id")
        .map(|v| v.to_str().unwrap().to_owned())
        .unwrap_or_default();
    let value = serde_json::from_slice(
        &axum::body::to_bytes(r.into_body(), 1024 * 1024)
            .await
            .unwrap(),
    )
    .unwrap();
    (status, id, value)
}

#[sqlx::test(migrations = false)]
async fn compact_pg_roundtrip_restart_and_unique_real_charges(pool: sqlx::PgPool) {
    let f = native_http::test_support::fixture(pool, 10000, &["compact-public"]).await;
    let runtime = RuntimeRole::create(&f.pool).await;
    runtime.optional("core_charging").await;
    runtime.optional("core_model").await;
    let (p, task) = supplier(Reply::Valid, false).await;
    let g = gateway(&runtime.pool, &p, true, Protocol::OpenAiResponses);
    let (status, id, compact) =
        call(&g, &f.key, "compact-first", true, json!("compress history")).await;
    assert_eq!(status, StatusCode::OK, "{compact}");
    let charge = g.bridge.charging.resolve(&id).await.unwrap().unwrap();
    assert_eq!(
        (charge.state.as_str(), charge.reserved, charge.settled),
        ("settled", 1200, 23)
    );
    let (_, contract) = g.bridge.charging.reservation_contract(&id).await.unwrap();
    assert_eq!(contract.bounds(), (1000, 100));
    drop(g);
    let restarted = gateway(&runtime.pool, &p, true, Protocol::OpenAiResponses);
    assert_eq!(
        call(
            &restarted,
            &f.key,
            "compact-first",
            true,
            json!("compress history")
        )
        .await
        .0,
        StatusCode::CONFLICT
    );
    let mut items = compact["output"].as_array().unwrap().clone();
    items.push(json!({"role":"user","content":"continue"}));
    let (status, id, result) =
        call(&restarted, &f.key, "compact-continue", false, json!(items)).await;
    assert_eq!(status, StatusCode::OK, "{result}");
    assert_eq!(result["id"], "resp_synthetic");
    let charge = restarted
        .bridge
        .charging
        .resolve(&id)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(
        (
            charge.reserved,
            charge.settled,
            charge.usage.input,
            charge.usage.output
        ),
        (1200, 31, 23, 4)
    );
    assert_eq!(p.hits.load(Ordering::SeqCst), 2);
    let b = f.ledger.balance(f.account).await.unwrap();
    assert_eq!((b.available_units, b.reserved_units), (9946, 0));
    drop(restarted);
    task.abort();
    runtime.cleanup().await;
}

#[sqlx::test(migrations = false)]
async fn compact_pg_unknown_receipts_keep_hold_across_restart_without_dispatch(pool: sqlx::PgPool) {
    let f = native_http::test_support::fixture(pool, 10000, &["compact-public"]).await;
    for (i, mode) in [
        Reply::MissingUsage,
        Reply::ExcessUsage,
        Reply::InvalidTerminal,
    ]
    .into_iter()
    .enumerate()
    {
        let (p, task) = supplier(mode, false).await;
        let g = gateway(&f.pool, &p, true, Protocol::OpenAiResponses);
        let key = format!("compact-unknown-{i}");
        let (status, id, _) = call(&g, &f.key, &key, true, json!("history")).await;
        assert!(status.is_server_error());
        let c = g.bridge.charging.resolve(&id).await.unwrap().unwrap();
        assert_eq!(
            (c.state.as_str(), c.reserved, c.settled),
            ("reconcile", 1200, 0)
        );
        drop(g);
        let restarted = gateway(&f.pool, &p, true, Protocol::OpenAiResponses);
        assert_eq!(
            call(&restarted, &f.key, &key, true, json!("history"))
                .await
                .0,
            StatusCode::CONFLICT
        );
        assert_eq!(p.hits.load(Ordering::SeqCst), 1);
        task.abort();
    }
    let b = f.ledger.balance(f.account).await.unwrap();
    assert_eq!((b.available_units, b.reserved_units), (6400, 3600));
}

#[sqlx::test(migrations = false)]
async fn compact_pg_unsupported_and_revoked_requests_never_call_provider(pool: sqlx::PgPool) {
    let f = native_http::test_support::fixture(pool, 10000, &["compact-public"]).await;
    let (p, task) = supplier(Reply::Valid, false).await;
    for (enabled, protocol) in [
        (false, Protocol::OpenAiResponses),
        (true, Protocol::OpenAiChat),
    ] {
        let g = gateway(&f.pool, &p, enabled, protocol);
        assert!(
            call(&g, &f.key, "compact-disabled", true, json!("history"))
                .await
                .0
                .is_client_error()
        );
    }
    sqlx::query("UPDATE core_identity.credentials SET revoked_at=clock_timestamp() WHERE id=$1")
        .bind(f.key_id)
        .execute(&f.pool)
        .await
        .unwrap();
    let g = gateway(&f.pool, &p, true, Protocol::OpenAiResponses);
    assert_eq!(
        call(&g, &f.key, "compact-revoked", true, json!("history"))
            .await
            .0,
        StatusCode::UNAUTHORIZED
    );
    assert_eq!(p.hits.load(Ordering::SeqCst), 0);
    let n: i64 = sqlx::query_scalar("SELECT count(*) FROM core_charging.requests")
        .fetch_one(&f.pool)
        .await
        .unwrap();
    assert_eq!(n, 0);
    task.abort();
}

#[sqlx::test(migrations = false)]
async fn compact_pg_revoke_after_dispatch_still_settles_original_work(pool: sqlx::PgPool) {
    let f = native_http::test_support::fixture(pool, 10000, &["compact-public"]).await;
    let (p, task) = supplier(Reply::Valid, true).await;
    let g = gateway(&f.pool, &p, true, Protocol::OpenAiResponses);
    let pending = call(&g, &f.key, "compact-admitted", true, json!("history"));
    let entered = p.pause.as_ref().unwrap().0.clone();
    let resume = p.pause.as_ref().unwrap().1.clone();
    let (result, ()) = tokio::join!(pending, async {
        entered.notified().await;
        sqlx::query(
            "UPDATE core_identity.credentials SET revoked_at=clock_timestamp() WHERE id=$1",
        )
        .bind(f.key_id)
        .execute(&f.pool)
        .await
        .unwrap();
        resume.notify_one();
    });
    assert_eq!(result.0, StatusCode::OK, "{}", result.2);
    assert_eq!(
        g.bridge
            .charging
            .resolve(&result.1)
            .await
            .unwrap()
            .unwrap()
            .settled,
        23
    );
    assert_eq!(
        call(&g, &f.key, "compact-after-revoke", true, json!("history"))
            .await
            .0,
        StatusCode::UNAUTHORIZED
    );
    assert_eq!(p.hits.load(Ordering::SeqCst), 1);
    task.abort();
}

#[sqlx::test(migrations = false)]
async fn compact_pg_sse_tools_stream_live_and_terminal_waits_for_real_settlement(
    pool: sqlx::PgPool,
) {
    let f = native_http::test_support::fixture(pool, 10000, &["compact-public"]).await;
    let (p, task) = supplier(Reply::Valid, false).await;
    let g = gateway(&f.pool, &p, true, Protocol::OpenAiResponses);
    let (status, _, compact) = call(&g, &f.key, "compact-for-sse", true, json!("history")).await;
    assert_eq!(status, StatusCode::OK, "{compact}");
    task.abort();
    let (p, task) = supplier(Reply::Valid, true).await;
    let g = gateway(&f.pool, &p, true, Protocol::OpenAiResponses);
    let body = json!({"model":"compact-public","input":compact["output"],"stream":true,
        "tools":[{"type":"function","name":"lookup","parameters":{"type":"object","properties":{"q":{"type":"string"}},"required":["q"],"additionalProperties":false}}],
        "reasoning":{"effort":"low"},"include":["reasoning.encrypted_content"]});
    let r = g
        .router()
        .oneshot(
            HttpRequest::builder()
                .method("POST")
                .uri("/v1/responses")
                .header(header::AUTHORIZATION, format!("Bearer {}", f.key))
                .header(header::CONTENT_TYPE, "application/json")
                .header("idempotency-key", "compact-sse-context")
                .body(Body::from(body.to_string()))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(r.headers()[header::CONTENT_TYPE], "text/event-stream");
    let id = r.headers()["x-request-id"].to_str().unwrap().to_owned();
    let mut body = r.into_body().into_data_stream();
    let first = tokio::time::timeout(std::time::Duration::from_secs(5), body.next())
        .await
        .unwrap()
        .unwrap()
        .unwrap();
    assert!(String::from_utf8_lossy(&first).contains("response.created"));
    let before = g.bridge.charging.resolve(&id).await.unwrap().unwrap();
    assert_eq!(before.state, "streaming");
    assert_eq!(before.settled, 0);
    assert_eq!(before.reserved, 1200);
    p.pause.as_ref().unwrap().1.notify_one();
    let mut wire = String::from_utf8(first.to_vec()).unwrap();
    while let Some(chunk) = body.next().await {
        wire.push_str(std::str::from_utf8(&chunk.unwrap()).unwrap());
    }
    assert!(wire.contains("response.output_text.delta"), "{wire}");
    assert!(wire.contains("response.completed"), "{wire}");
    assert!(wire.contains("function_call"), "{wire}");
    assert!(!wire.contains("event: error"), "{wire}");
    let charge = g.bridge.charging.resolve(&id).await.unwrap().unwrap();
    let packets: Vec<&[u8]> = wire.split_inclusive("\n\n").map(str::as_bytes).collect();
    let checkpoints: Vec<(Vec<u8>,i64)> = sqlx::query_as("SELECT packet_sha256,packet_bytes FROM core_model.output_checkpoints WHERE request_id=$1 ORDER BY sequence")
        .bind(&id).fetch_all(&f.pool).await.unwrap();
    assert_eq!(checkpoints.len(), packets.len());
    for (packet, (digest, len)) in packets.iter().zip(checkpoints) {
        assert_eq!(digest, Sha256::digest(packet).to_vec());
        assert_eq!(len, packet.len() as i64);
    }
    assert_eq!((charge.state.as_str(), charge.settled), ("settled", 31));
    task.abort();
}

#[sqlx::test(migrations = false)]
async fn compact_pg_media_and_unpriced_input_rejected_before_reserve(pool: sqlx::PgPool) {
    let f = native_http::test_support::fixture(pool, 10000, &["compact-public"]).await;
    let (p, task) = supplier(Reply::Valid, false).await;
    let g = gateway(&f.pool, &p, true, Protocol::OpenAiResponses);
    for part in [
        json!({"type":"input_image","image_url":"file:///etc/passwd"}),
        json!({"type":"input_image","image_url":"http://169.254.169.254/"}),
        json!({"type":"input_file","file_url":"http://127.0.0.1/private"}),
        json!({"type":"input_audio","input_audio":{"data":"abc"}}),
    ] {
        let input = json!([{"type":"compaction","encrypted_content":"opaque"},{"role":"user","content":[part]}]);
        assert!(
            call(&g, &f.key, "compact-invalid-media", false, input)
                .await
                .0
                .is_client_error()
        );
    }
    assert_eq!(p.hits.load(Ordering::SeqCst), 0);
    let n: i64 = sqlx::query_scalar("SELECT count(*) FROM core_charging.requests")
        .fetch_one(&f.pool)
        .await
        .unwrap();
    assert_eq!(n, 0);
    task.abort();
}

#[sqlx::test(migrations = false)]
async fn compact_pg_sse_terminal_is_not_eof_and_bad_tail_keeps_hold(pool: sqlx::PgPool) {
    let f = native_http::test_support::fixture(pool, 10000, &["compact-public"]).await;
    for (i, mode) in [
        Reply::TailAfterTerminal,
        Reply::SecondTerminal,
        Reply::NeverEof,
    ]
    .into_iter()
    .enumerate()
    {
        let (p, task) = supplier(mode, false).await;
        let mut g = gateway(&f.pool, &p, true, Protocol::OpenAiResponses);
        g.limits.total_timeout = std::time::Duration::from_millis(700);
        g.limits.read_timeout = std::time::Duration::from_millis(300);
        let body = json!({"model":"compact-public","input":[{"type":"compaction","encrypted_content":"opaque".repeat(20000)}],"stream":true,
            "tools":[{"type":"function","name":"lookup","parameters":{"type":"object","properties":{}}}],"include":["reasoning.encrypted_content"]});
        let r = g
            .router()
            .oneshot(
                HttpRequest::builder()
                    .method("POST")
                    .uri("/v1/responses")
                    .header(header::AUTHORIZATION, format!("Bearer {}", f.key))
                    .header(header::CONTENT_TYPE, "application/json")
                    .header("idempotency-key", format!("compact-sse-tail-{i}"))
                    .body(Body::from(body.to_string()))
                    .unwrap(),
            )
            .await
            .unwrap();
        let id = r.headers()["x-request-id"].to_str().unwrap().to_owned();
        let mut stream = r.into_body().into_data_stream();
        let mut wire = Vec::new();
        let mut failed = false;
        while let Some(chunk) = stream.next().await {
            match chunk {
                Ok(chunk) => wire.extend_from_slice(&chunk),
                Err(_) => {
                    failed = true;
                    break;
                }
            }
        }
        let text = std::str::from_utf8(&wire).unwrap();
        assert!(!text.contains("event: response.completed"), "{text}");
        assert!(failed, "missing stream failure: {text}");
        let c = g.bridge.charging.resolve(&id).await.unwrap().unwrap();
        assert_eq!(
            (c.state.as_str(), c.reserved, c.settled),
            ("reconcile", 1200, 0)
        );
        assert_eq!(p.hits.load(Ordering::SeqCst), 1);
        task.abort();
    }
    let b = f.ledger.balance(f.account).await.unwrap();
    assert_eq!((b.available_units, b.reserved_units), (6400, 3600));
}

#[sqlx::test(migrations = false)]
async fn compact_pg_exact_profile_rejected_before_reserve_or_dispatch(pool: sqlx::PgPool) {
    let f = native_http::test_support::fixture(pool, 10000, &["compact-public"]).await;
    let (p, task) = supplier(Reply::Valid, false).await;
    let g = gateway_with_reservation(
        &f.pool,
        &p,
        true,
        Protocol::OpenAiResponses,
        super::super::provider_policy::ReservationProfile::OpenaiResponsesExactV1 {
            count_timeout_ms: std::num::NonZeroU64::new(1000).unwrap(),
        },
    );
    for (compact, input) in [
        (true, json!("history")),
        (
            false,
            json!([{"type":"compaction","encrypted_content":"opaque"}]),
        ),
    ] {
        assert!(
            call(&g, &f.key, "compact-exact-denied", compact, input)
                .await
                .0
                .is_client_error()
        );
    }
    assert_eq!(p.hits.load(Ordering::SeqCst), 0);
    let rows: i64 = sqlx::query_scalar("SELECT count(*) FROM core_charging.requests")
        .fetch_one(&f.pool)
        .await
        .unwrap();
    assert_eq!(rows, 0);
    let balance = f.ledger.balance(f.account).await.unwrap();
    assert_eq!(
        (balance.available_units, balance.reserved_units),
        (10000, 0)
    );
    task.abort();
}

#[sqlx::test(migrations = false)]
async fn compact_pg_pretty_json_checkpoint_matches_exact_client_body(pool: sqlx::PgPool) {
    let f = native_http::test_support::fixture(pool, 10000, &["compact-public"]).await;
    let (p, task) = supplier(Reply::PrettyJson, false).await;
    let g = gateway(&f.pool, &p, true, Protocol::OpenAiResponses);
    for compact in [true, false] {
        let input = if compact {
            json!("history")
        } else {
            json!([{"type":"compaction","encrypted_content":"opaque".repeat(20000)}])
        };
        let response = g
            .router()
            .oneshot(
                HttpRequest::builder()
                    .method("POST")
                    .uri(if compact {
                        "/v1/responses/compact"
                    } else {
                        "/v1/responses"
                    })
                    .header(header::AUTHORIZATION, format!("Bearer {}", f.key))
                    .header(header::CONTENT_TYPE, "application/json")
                    .header(
                        "Idempotency-Key",
                        if compact {
                            "pretty-compact"
                        } else {
                            "pretty-context"
                        },
                    )
                    .body(Body::from(
                        json!({"model":"compact-public","input":input}).to_string(),
                    ))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::OK);
        assert_eq!(response.headers()[header::CONTENT_TYPE], "application/json");
        let id = response.headers()["x-request-id"]
            .to_str()
            .unwrap()
            .to_owned();
        let bytes = axum::body::to_bytes(response.into_body(), 1024 * 1024)
            .await
            .unwrap();
        let _: Value = serde_json::from_slice(&bytes).unwrap();
        let checkpoint: (Vec<u8>, i64) = sqlx::query_as(
            "SELECT packet_sha256,packet_bytes FROM core_model.output_checkpoints WHERE request_id=$1")
            .bind(&id).fetch_one(&f.pool).await.unwrap();
        assert_eq!(checkpoint.0, Sha256::digest(&bytes).to_vec());
        assert_eq!(checkpoint.1, bytes.len() as i64);
        let charge = g.bridge.charging.resolve(&id).await.unwrap().unwrap();
        assert_eq!(
            (charge.state.as_str(), charge.settled),
            ("settled", if compact { 23 } else { 31 })
        );
    }
    assert_eq!(p.hits.load(Ordering::SeqCst), 2);
    task.abort();
}

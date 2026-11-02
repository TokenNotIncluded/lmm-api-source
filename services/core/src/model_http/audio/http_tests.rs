//! Synthetic loopback contract only; tests use real Core HTTP + isolated PG.
//! These fixtures are not provider usage evidence for a real model.
use super::*;
use crate::{
    routing::{CredentialRef, Endpoint, PriceConfig, PriceSnapshot, RoutingConfig},
    test_runtime::RuntimeRole,
};
use axum::{body::to_bytes, http::Request as HttpRequest};
use std::sync::atomic::{AtomicUsize, Ordering};
use tokio::{net::TcpListener, task::JoinHandle};
use tower::ServiceExt;

#[derive(Clone)]
struct SupplierState {
    hits: Arc<AtomicUsize>,
    missing: bool,
    official: bool,
    pause: Option<Arc<tokio::sync::Notify>>,
    tail: u8,
    packed: bool,
    tail_gate: Arc<tokio::sync::Notify>,
}
struct Supplier {
    endpoint: String,
    state: SupplierState,
    task: JoinHandle<()>,
}
impl Drop for Supplier {
    fn drop(&mut self) {
        self.task.abort();
    }
}
impl Supplier {
    async fn start(missing: bool) -> Self {
        Self::start_inner(missing, None).await
    }
    async fn start_inner(missing: bool, pause: Option<Arc<tokio::sync::Notify>>) -> Self {
        Self::start_tail(missing, pause, 0, false).await
    }
    async fn start_tail(
        missing: bool,
        pause: Option<Arc<tokio::sync::Notify>>,
        tail: u8,
        packed: bool,
    ) -> Self {
        Self::start_style(missing, pause, tail, packed, false).await
    }
    async fn start_style(
        missing: bool,
        pause: Option<Arc<tokio::sync::Notify>>,
        tail: u8,
        packed: bool,
        official: bool,
    ) -> Self {
        async fn answer(
            State(state): State<SupplierState>,
            Json(request): Json<Value>,
        ) -> Response {
            state.hits.fetch_add(1, Ordering::SeqCst);
            assert_eq!(request["model"], "synthetic-audio-upstream");
            assert_eq!(request["max_completion_tokens"], 16);
            let mut value = json!({"model":"synthetic-audio-upstream","choices":[{"index":0,"finish_reason":"stop","message":{"role":"assistant","content":"synthetic transcript"}}],
                "usage":{"prompt_tokens":4,"completion_tokens":6,"total_tokens":10,"prompt_tokens_details":{"text_tokens":1,"audio_tokens":3},"completion_tokens_details":{"text_tokens":2,"audio_tokens":4}}});
            if state.missing {
                value["usage"]["completion_tokens_details"]
                    .as_object_mut()
                    .unwrap()
                    .remove("audio_tokens");
            }
            if state.official {
                value["usage"]["prompt_tokens_details"]
                    .as_object_mut()
                    .unwrap()
                    .remove("text_tokens");
                value["usage"]["completion_tokens_details"]
                    .as_object_mut()
                    .unwrap()
                    .remove("text_tokens");
                value["usage"]["prompt_tokens_details"]["cache_write_tokens"] = json!(0);
                value["usage"]["prompt_tokens_details"]["image_tokens"] = json!(0);
                value["service_tier"] = json!("default");
            }
            if request["stream"] == true {
                assert_eq!(request["stream_options"]["include_usage"], true);
                let packet = |choices: Value, usage: Value| {
                    format!(
                        "data: {}\n\n",
                        json!({"id":"stream-1","object":"chat.completion.chunk","model":"synthetic-audio-upstream","choices":choices,"usage":usage,"service_tier":"default","obfuscation":"EPHEMERAL_PADDING"})
                    )
                };
                let mut frames = vec![
                    packet(
                        json!([{"index":0,"delta":{"role":"assistant","content":"hello"},"finish_reason":null}]),
                        Value::Null,
                    ),
                    packet(
                        json!([{"index":0,"delta":{},"finish_reason":"stop"}]),
                        Value::Null,
                    ),
                    packet(json!([]), value["usage"].clone()),
                    "data: [DONE]\n\n".into(),
                ];
                match state.tail {
                    1 => frames.push(frames[2].clone()), // duplicate usage after DONE
                    2 => frames.push("data: [DONE]\n\n".into()),
                    3 => {
                        frames.pop();
                    } // EOF before DONE
                    4 => frames.push("data: {\"error\":\"late\"}\n\n".into()),
                    5 => frames.push("data: partial".into()),
                    _ => (),
                }
                if state.packed {
                    frames = vec![frames.concat()];
                }
                let (tx, rx) =
                    tokio::sync::mpsc::channel::<std::result::Result<Bytes, std::io::Error>>(1);
                tokio::spawn(async move {
                    for (i, frame) in frames.into_iter().enumerate() {
                        if i == 1 {
                            if let Some(pause) = &state.pause {
                                pause.notified().await;
                            }
                        }
                        if tx.send(Ok(Bytes::from(frame))).await.is_err() {
                            break;
                        }
                    }
                    if state.tail == 7 {
                        state.tail_gate.notified().await;
                    }
                    if state.tail == 6 {
                        state.tail_gate.notified().await;
                        let _ = tx
                            .send(Err(std::io::Error::other("late transport failure")))
                            .await;
                    }
                });
                let body = Body::from_stream(tokio_stream::wrappers::ReceiverStream::new(rx));
                let mut response = Response::new(body);
                response.headers_mut().insert(
                    header::CONTENT_TYPE,
                    HeaderValue::from_static("text/event-stream"),
                );
                return response;
            }
            Json(value).into_response()
        }
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let endpoint = format!("http://{}/v1", listener.local_addr().unwrap());
        let state = SupplierState {
            hits: Arc::new(AtomicUsize::new(0)),
            missing,
            official,
            pause,
            tail,
            packed,
            tail_gate: Arc::new(tokio::sync::Notify::new()),
        };
        let app = Router::new()
            .route("/v1/chat/completions", post(answer))
            .with_state(state.clone());
        let task = tokio::spawn(async move {
            axum::serve(listener, app).await.unwrap();
        });
        Self {
            endpoint,
            state,
            task,
        }
    }
}
fn rule(endpoint: &str) -> AudioRule {
    let ratio = |n| AudioRatio {
        numerator: n,
        denominator: 1,
    };
    AudioRule {
        chat_streaming: true,
        route_version: 1,
        upstream_id: 1,
        upstream_model: "synthetic-audio-upstream".into(),
        endpoint: endpoint.into(),
        price: AudioPrice {
            version: 1,
            price_revision: "audio-v1".into(),
            source_sha256: [1; 32],
            model: "audio-public".into(),
            group: "allowed".into(),
            provider_contract_revision: "synthetic-audio-chat-v1".into(),
            tariff: AudioTariff::Multipliers {
                model: ratio(1),
                completion: ratio(2),
                audio: ratio(3),
                audio_completion: ratio(4),
            },
            group_multiplier: ratio(1),
            rounding: AudioRounding::Ceiling,
            usage_format: AudioUsageFormat::OpenAiChatV1,
            bounds: AudioUsage {
                text_input: 100,
                text_output: 16,
                audio_input: 100,
                audio_output: 16,
            },
            max_input_tokens: 100,
            max_output_tokens: 16,
            max_budget: crate::billing::Credits::try_from(10000_i64).unwrap(),
        },
    }
}
fn gateway(pool: &PgPool, supplier: &Supplier) -> ModelGateway {
    let mut routes: RoutingConfig = serde_json::from_value(json!({"version":1,"groups":[{"name":"allowed","enabled":true}],
        "upstreams":[{"id":1,"endpoint":"https://synthetic.invalid/v1","protocol":"openai_chat","credential":{"id":7,"revision":1},"enabled":true}],
        "routes":[{"id":1,"model":"audio-public","group":"allowed","enabled":true,"speeds":["standard"],"targets":[{"id":1,"upstream_id":1,"upstream_model":"synthetic-audio-upstream","enabled":true,"priority":0,"weight":1,"speeds":["standard"],"payload":"adapt","timeout_ms":10000}],"failover":{"max_attempts":1,"retry_on":[],"allow_replay_after_send":false}}],"aliases":[]})).unwrap();
    routes.upstreams[0].endpoint = Endpoint::local_loopback(supplier.endpoint.clone()).unwrap();
    let prices: PriceConfig = serde_json::from_value(json!({"version":1,"rules":[{"id":1,"model":"audio-public","group":"allowed","speed":"standard",
        "rates":{"input":1,"output":1,"cache_read":0,"cache_write":0,"request":0},"group_multiplier_ppm":1000000,"speed_multiplier_ppm":1000000}]})).unwrap();
    ModelGateway::new(
        pool.clone(),
        Arc::new(
            crate::routing::Router::new(routes, Arc::new(PriceSnapshot::compile(prices).unwrap()))
                .unwrap(),
        ),
        vec![UpstreamCredential {
            reference: CredentialRef { id: 7, revision: 1 },
            secret: "synthetic-audio-provider-key".into(),
        }],
        BTreeMap::new(),
        Limits {
            total_timeout: Duration::from_secs(5),
            header_timeout: Duration::from_secs(4),
            hook_timeout: Duration::from_secs(2),
            ..Default::default()
        },
        ChargePolicy {
            input_token_ceiling: 2000,
            lease_seconds: 30,
        },
    )
    .unwrap()
    .with_audio_rules(vec![rule(&supplier.endpoint)])
    .unwrap()
}
fn request(key: &str, id: &str) -> HttpRequest<Body> {
    HttpRequest::builder().method("POST").uri("/v1/chat/completions").header(header::AUTHORIZATION,format!("Bearer {key}"))
        .header(header::CONTENT_TYPE,"application/json").header("idempotency-key",id)
        .body(Body::from(json!({"model":"audio-public","messages":[{"role":"user","content":"synthetic input"}],"max_completion_tokens":16}).to_string())).unwrap()
}

#[sqlx::test(migrations = false)]
async fn audio_http_pg_settles_four_counters_once_and_preserves_raw_receipt(pool: PgPool) {
    let f = native_http::test_support::fixture(pool, 10000, &["audio-public"]).await;
    let runtime = RuntimeRole::create(&f.pool).await;
    runtime.optional("core_charging").await;
    runtime.optional("core_model").await;
    let supplier = Supplier::start(false).await;
    let gateway = gateway(&runtime.pool, &supplier);
    let reply = gateway
        .router()
        .oneshot(request(&f.key, "audio-success"))
        .await
        .unwrap();
    let status = reply.status();
    let raw = to_bytes(reply.into_body(), 1024 * 1024).await.unwrap();
    assert_eq!(status, StatusCode::OK, "{}", String::from_utf8_lossy(&raw));
    let (id, state, settled): (String, String, i64) =
        sqlx::query_as("SELECT id,state,settled FROM core_charging.requests")
            .fetch_one(&f.pool)
            .await
            .unwrap();
    assert_eq!((state.as_str(), settled), ("settled", 62)); // 1 + 2*2 + 3*3 + 4*3*4.
    let charge = gateway.bridge.charging.resolve(&id).await.unwrap().unwrap();
    assert_eq!(
        charge.usage.audio.unwrap().counts,
        AudioUsage {
            text_input: 1,
            text_output: 2,
            audio_input: 3,
            audio_output: 4
        }
    );
    assert_eq!((charge.usage.input, charge.usage.output), (0, 0));
    let receipt: Value = sqlx::query_scalar("SELECT detail FROM core_charging.usage_events WHERE request_id=$1 AND event_id='audio-terminal:v1'").bind(&id).fetch_one(&f.pool).await.unwrap();
    assert!(
        receipt["raw_usage"]
            .as_str()
            .unwrap()
            .contains("audio_tokens")
    );
    assert_eq!(
        receipt["price_sha256"],
        charging::contract::typed_digest(&charge.price).unwrap()
    );
    assert_eq!(
        gateway
            .router()
            .oneshot(request(&f.key, "audio-success"))
            .await
            .unwrap()
            .status(),
        StatusCode::CONFLICT
    );
    assert_eq!(supplier.state.hits.load(Ordering::SeqCst), 1);
    let balance = f.ledger.balance(f.account).await.unwrap();
    assert_eq!((balance.available_units, balance.reserved_units), (9938, 0));
    drop(gateway);
    runtime.cleanup().await;
}

#[sqlx::test(migrations = false)]
async fn audio_http_pg_missing_modality_keeps_hold_and_cannot_settle_zero(pool: PgPool) {
    let f = native_http::test_support::fixture(pool, 10000, &["audio-public"]).await;
    let runtime = RuntimeRole::create(&f.pool).await;
    runtime.optional("core_charging").await;
    runtime.optional("core_model").await;
    let supplier = Supplier::start(true).await;
    let gateway = gateway(&runtime.pool, &supplier);
    let reply = gateway
        .router()
        .oneshot(request(&f.key, "audio-missing"))
        .await
        .unwrap();
    assert!(!reply.status().is_success());
    let (id, state, reserved): (String, String, i64) =
        sqlx::query_as("SELECT id,state,reserved FROM core_charging.requests")
            .fetch_one(&f.pool)
            .await
            .unwrap();
    assert_eq!(state, "reconcile");
    assert_eq!(reserved, 624);
    let charge = gateway.bridge.charging.resolve(&id).await.unwrap().unwrap();
    assert!(matches!(
        gateway
            .bridge
            .charging
            .reconcile(
                &id,
                charge.revision,
                charging::Usage::default(),
                Outcome::Completed,
                "synthetic-no-usage"
            )
            .await,
        Err(charging::Error::Invalid)
    ));
    gateway.recover_expired(10).await.unwrap();
    let balance = f.ledger.balance(f.account).await.unwrap();
    assert_eq!(
        (balance.available_units, balance.reserved_units),
        (9376, 624)
    );
    assert_eq!(supplier.state.hits.load(Ordering::SeqCst), 1);
    drop(gateway);
    runtime.cleanup().await;
}

#[test]
fn unsupported_tools_and_unbounded_requests_are_rejected() {
    let base = json!({"model":"audio-public","messages":[{"role":"user","content":"hi"}],"max_completion_tokens":16});
    for (field, value) in [
        ("stream_options", json!({"include_usage":false})),
        ("tools", json!([])),
        ("max_completion_tokens", json!(0)),
    ] {
        let mut body = base.clone();
        body[field] = value;
        assert!(ChatRequest::parse(&serde_json::to_vec(&body).unwrap()).is_err());
    }
    let mut missing = base;
    missing
        .as_object_mut()
        .unwrap()
        .remove("max_completion_tokens");
    assert!(ChatRequest::parse(&serde_json::to_vec(&missing).unwrap()).is_err());
}

#[sqlx::test(migrations = false)]
async fn audio_pg_recovery_uses_durable_receipt_and_original_price(pool: PgPool) {
    recovery_with_format(pool, AudioUsageFormat::OpenAiChatV1).await;
}
#[sqlx::test(migrations = false)]
async fn audio_chat_v2_pg_recovery_retains_original_format_and_price(pool: PgPool) {
    recovery_with_format(pool, AudioUsageFormat::OpenAiChatV2).await;
}
async fn recovery_with_format(pool: PgPool, format: AudioUsageFormat) {
    // Directly exercise the checkpoint/finalize gap. This is object recreation,
    // not an OS-crash acceptance runner, and does not call an external supplier.
    let f = native_http::test_support::fixture(pool, 10000, &["audio-public"]).await;
    let runtime = RuntimeRole::create(&f.pool).await;
    runtime.optional("core_charging").await;
    runtime.optional("core_model").await;
    let supplier = Supplier::start(false).await;
    let old = gateway(&runtime.pool, &supplier);
    let mut audio = rule(&supplier.endpoint).price;
    audio.usage_format = format;
    let price = charging::Price {
        audio: Some(audio.clone()),
        text_request: None,
        tool: None,
        grounding: None,
        user_trust: None,
        image: None,
        upstream_cost_group_ppm: None,
        tiered: None,
        version: audio.price_revision.clone(),
        model: audio.model.clone(),
        group: audio.group.clone(),
        input_per_million: 0,
        cached_per_million: 0,
        output_per_million: 0,
    };
    let worker = [9; 32];
    let fingerprint = [8; 32];
    let wire_hash = "07".repeat(32);
    let prepared = charging::contract::PreparedReservation::audio_chat(
        charging::contract::hex_digest(&fingerprint),
        charging::contract::ReservationBinding {
            route_id: "r1-t1-u1".into(),
            target_id: 1,
            upstream_id: 1,
            protocol: "audio_chat".into(),
            model_snapshot: "synthetic-audio-upstream".into(),
            generation_url: format!("{}/chat/completions", supplier.endpoint),
            credential_ref: CredentialRef { id: 7, revision: 1 },
            payload_policy_sha256: "06".repeat(32),
            provider_contract_revision: audio.provider_contract_revision.clone(),
            projection_revision: if format == AudioUsageFormat::OpenAiChatV2 {
                "audio-chat-stream-raw-v2-no-other-modalities"
            } else {
                "audio-chat-stream-raw-v1"
            }
            .into(),
            quote_version: 1,
        },
        wire_hash.clone(),
        audio.bounds,
    )
    .unwrap();
    let spec = charging::Request {
        id: "synthetic-audio-recovery".into(),
        fingerprint,
        reserve: price
            .reservation_ceiling(charging::ReservationBasis::Audio)
            .unwrap(),
        price,
        lease_seconds: 30,
        maximum_seconds: 30,
    };
    old.bridge
        .charging
        .reserve_prepared_model_with_context(
            &f.key,
            &spec,
            &worker,
            None,
            None,
            charging::ReservationBasis::Audio,
            &prepared,
        )
        .await
        .unwrap();
    assert!(old.bridge.charging.start(&spec.id, &worker).await.unwrap());
    old.bridge
        .charging
        .record_route_dispatch(&spec.id, &worker, 1, "r1-t1-u1", "allowed", &wire_hash)
        .await
        .unwrap();
    let packet = b"data: [DONE]\n\n";
    let raw = r#"{ "prompt_tokens":4,"completion_tokens":6,"total_tokens":10,"prompt_tokens_details":{"text_tokens":1,"audio_tokens":3},"completion_tokens_details":{"text_tokens":2,"audio_tokens":4} }"#;
    let raw = if format == AudioUsageFormat::OpenAiChatV2 {
        raw.replace("\"text_tokens\":1,", "")
            .replace("\"text_tokens\":2,", "")
    } else {
        raw.to_owned()
    };
    let raw = raw.as_str();
    let finish = format!(
        "data: {}\n\n",
        json!({"id":"s1","object":"chat.completion.chunk","model":"synthetic-audio-upstream","choices":[{"index":0,"delta":{},"finish_reason":"stop"}],"usage":null})
    );
    let usage_packet = format!(
        "data: {{\"id\":\"s1\",\"object\":\"chat.completion.chunk\",\"model\":\"synthetic-audio-upstream\",\"choices\":[],\"usage\":{raw}}}\n\n"
    );
    // Reject the v1 counterexample: DONE alone plus independently supplied usage.
    assert!(
        old.bridge
            .charging
            .checkpoint_audio_stream(&spec.id, &worker, 1, packet, Sha256::digest(packet).into())
            .await
            .is_err()
    );
    assert!(
        old.bridge
            .charging
            .checkpoint_audio(&spec.id, &worker, raw, Sha256::digest(packet).into())
            .await
            .is_err()
    );
    let mut digest = Sha256::new();
    for (i, p) in [finish.as_bytes(), usage_packet.as_bytes()]
        .into_iter()
        .enumerate()
    {
        digest.update(p);
        let sha: [u8; 32] = digest.clone().finalize().into();
        old.bridge
            .charging
            .checkpoint_audio_packet(&spec.id, &worker, (i + 1) as u64, p, sha, false)
            .await
            .unwrap();
    }
    let before:i64=sqlx::query_scalar("SELECT count(*) FROM core_charging.usage_events WHERE request_id=$1 AND event_id='audio-terminal:v1'")
        .bind(&spec.id).fetch_one(&f.pool).await.unwrap();
    assert_eq!(before, 0);
    digest.update(packet);
    let stream_sha: [u8; 32] = digest.finalize().into();
    // A malformed terminal append must leave no terminal row. Body SHA is a trusted worker observation.
    assert!(
        old.bridge
            .charging
            .checkpoint_audio_stream(
                &spec.id,
                &worker,
                3,
                b"data: [DONE]\n\ntrailing",
                stream_sha
            )
            .await
            .is_err()
    );
    let tail:i64=sqlx::query_scalar("SELECT count(*) FROM core_charging.usage_events WHERE request_id=$1 AND event_id='audio-packet:v1:0003'")
        .bind(&spec.id).fetch_one(&f.pool).await.unwrap();
    assert_eq!(tail, 0);
    // Database fault after DONE + terminal receipt INSERT but before normalized
    // usage INSERT: all three rows must roll back together. This is a transaction
    // fault test, not a claimed OS-crash acceptance test.
    sqlx::raw_sql(r#"CREATE FUNCTION public.audio_test_receipt_fault() RETURNS trigger LANGUAGE plpgsql AS $$ BEGIN
        IF NEW.request_id='synthetic-audio-recovery' AND NEW.event_id='usage:audio-terminal:v1' THEN
            RAISE EXCEPTION 'injected receipt boundary failure'; END IF; RETURN NEW; END $$;
        CREATE TRIGGER audio_test_receipt_fault BEFORE INSERT ON core_charging.usage_events
        FOR EACH ROW EXECUTE FUNCTION public.audio_test_receipt_fault();"#).execute(&f.pool).await.unwrap();
    assert!(
        old.bridge
            .charging
            .checkpoint_audio_stream(&spec.id, &worker, 3, packet, stream_sha)
            .await
            .is_err()
    );
    let rolled_back:i64=sqlx::query_scalar("SELECT count(*) FROM core_charging.usage_events WHERE request_id=$1 AND event_id IN ('audio-packet:v1:0003','audio-terminal:v1','usage:audio-terminal:v1')")
        .bind(&spec.id).fetch_one(&f.pool).await.unwrap();
    assert_eq!(rolled_back, 0);
    sqlx::raw_sql("DROP TRIGGER audio_test_receipt_fault ON core_charging.usage_events; DROP FUNCTION public.audio_test_receipt_fault();")
        .execute(&f.pool).await.unwrap();
    old.bridge
        .charging
        .checkpoint_audio_stream(&spec.id, &worker, 3, packet, stream_sha)
        .await
        .unwrap();
    // An ACK-loss retry is identical; no second receipt or debit.
    old.bridge
        .charging
        .checkpoint_audio_stream(&spec.id, &worker, 3, packet, stream_sha)
        .await
        .unwrap();
    let wrong = raw
        .replace("\"text_tokens\":1", "\"text_tokens\":2")
        .replace("\"prompt_tokens\":4", "\"prompt_tokens\":5")
        .replace("\"total_tokens\":10", "\"total_tokens\":11");
    assert!(
        old.bridge
            .charging
            .checkpoint_audio(&spec.id, &worker, &wrong, stream_sha)
            .await
            .is_err()
    );
    let atomic:i64=sqlx::query_scalar("SELECT count(*) FROM core_charging.usage_events WHERE request_id=$1 AND event_id IN ('audio-packet:v1:0003','audio-terminal:v1','usage:audio-terminal:v1')")
        .bind(&spec.id).fetch_one(&f.pool).await.unwrap();
    assert_eq!(atomic, 3);
    old.bridge
        .charging
        .mark_unknown(&spec.id, &worker)
        .await
        .unwrap();
    drop(old);
    let mut new_rule = rule(&supplier.endpoint);
    new_rule.price.price_revision = "audio-v2".into();
    new_rule.price.tariff = AudioTariff::Fixed {
        usd_per_request: AudioRatio {
            numerator: 1,
            denominator: 100,
        },
    };
    let new = gateway(&runtime.pool, &supplier)
        .with_audio_rules(vec![new_rule])
        .unwrap();
    new.recover_expired(10).await.unwrap();
    new.recover_expired(10).await.unwrap();
    let charge = new
        .bridge
        .charging
        .resolve(&spec.id)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(
        (
            charge.state.as_str(),
            charge.settled,
            charge.price.version.as_str()
        ),
        ("settled", 62, "audio-v1")
    );
    let balance = f.ledger.balance(f.account).await.unwrap();
    assert_eq!((balance.available_units, balance.reserved_units), (9938, 0));
    assert_eq!(supplier.state.hits.load(Ordering::SeqCst), 0);
    // A different terminal receipt cannot replace the saved checkpoint.
    let changed = raw
        .replace("\"text_tokens\":1", "\"text_tokens\":2")
        .replace("\"prompt_tokens\":4", "\"prompt_tokens\":5")
        .replace("\"total_tokens\":10", "\"total_tokens\":11");
    assert!(matches!(
        new.bridge
            .charging
            .checkpoint_audio(&spec.id, &worker, &changed, stream_sha)
            .await,
        Err(charging::Error::Invalid)
    ));
    assert_eq!(
        new.bridge
            .charging
            .resolve(&spec.id)
            .await
            .unwrap()
            .unwrap()
            .settled,
        62
    );
    drop(new);
    runtime.cleanup().await;
}

#[sqlx::test(migrations = false)]
async fn audio_stream_pg_checkpoints_before_delivery_then_settles_once(pool: PgPool) {
    use tokio_stream::StreamExt;
    let f = native_http::test_support::fixture(pool, 10000, &["audio-public"]).await;
    let runtime = RuntimeRole::create(&f.pool).await;
    runtime.optional("core_charging").await;
    runtime.optional("core_model").await;
    let supplier = Supplier::start(false).await;
    let gateway = gateway(&runtime.pool, &supplier);
    let stream_request = || {
        let mut r = request(&f.key, "audio-stream-success");
        *r.body_mut() = Body::from(json!({"model":"audio-public","messages":[{"role":"user","content":"input"}],"max_completion_tokens":16,"stream":true}).to_string());
        r
    };
    let reply = gateway.router().oneshot(stream_request()).await.unwrap();
    assert_eq!(reply.status(), StatusCode::OK);
    let mut body = reply.into_body().into_data_stream();
    let first = body.next().await.unwrap().unwrap();
    let detail: Value = sqlx::query_scalar(
        "SELECT detail FROM core_charging.usage_events WHERE event_id='audio-packet:v1:0001'",
    )
    .fetch_one(&f.pool)
    .await
    .unwrap();
    assert_eq!(detail["packet_sha256"], charging::contract::digest(&first));
    let mut bytes = first.to_vec();
    while let Some(chunk) = body.next().await {
        bytes.extend_from_slice(&chunk.unwrap());
    }
    assert!(bytes.ends_with(b"data: [DONE]\n\n"));
    assert_eq!(
        gateway
            .router()
            .oneshot(stream_request())
            .await
            .unwrap()
            .status(),
        StatusCode::CONFLICT
    );
    assert_eq!(gateway.native_http.drain(Duration::from_secs(3)).await, 0);
    let (state, settled): (String, i64) =
        sqlx::query_as("SELECT state,settled FROM core_charging.requests")
            .fetch_one(&f.pool)
            .await
            .unwrap();
    assert_eq!((state.as_str(), settled), ("settled", 62));
    let count: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM core_charging.usage_events WHERE event_id LIKE 'audio-packet:v1:%'",
    )
    .fetch_one(&f.pool)
    .await
    .unwrap();
    assert_eq!(count, 4);
    let rows:Vec<Value>=sqlx::query_scalar("SELECT detail FROM core_charging.usage_events WHERE event_id LIKE 'audio-packet:v1:%' ORDER BY event_id").fetch_all(&f.pool).await.unwrap();
    assert!(rows.iter().all(|r| r.get("packet").is_none()));
    assert_eq!(
        rows.iter().filter(|r| r["raw_usage"].is_string()).count(),
        1
    );
    let journal = serde_json::to_string(&rows).unwrap();
    assert!(!journal.contains("hello"));
    assert!(!journal.contains("delta"));
    assert!(!journal.contains("obfuscation"));
    assert!(!journal.contains("service_tier"));
    assert!(!journal.contains("EPHEMERAL_PADDING"));
    assert!(
        rows[2]["raw_usage"]
            .as_str()
            .unwrap()
            .contains("prompt_tokens_details")
    );

    assert_eq!(supplier.state.hits.load(Ordering::SeqCst), 1);
    let balance = f.ledger.balance(f.account).await.unwrap();
    assert_eq!((balance.available_units, balance.reserved_units), (9938, 0));
    drop(body);
    drop(gateway);
    runtime.cleanup().await;
}

#[sqlx::test(migrations = false)]
async fn audio_stream_pg_missing_terminal_usage_retains_hold(pool: PgPool) {
    let f = native_http::test_support::fixture(pool, 10000, &["audio-public"]).await;
    let runtime = RuntimeRole::create(&f.pool).await;
    runtime.optional("core_charging").await;
    runtime.optional("core_model").await;
    let supplier = Supplier::start(true).await;
    let gateway = gateway(&runtime.pool, &supplier);
    let mut r = request(&f.key, "audio-stream-missing");
    *r.body_mut()=Body::from(json!({"model":"audio-public","messages":[{"role":"user","content":"input"}],"max_completion_tokens":16,"stream":true}).to_string());
    let reply = gateway.router().oneshot(r).await.unwrap();
    assert_eq!(reply.status(), StatusCode::OK);
    assert!(to_bytes(reply.into_body(), 1024 * 1024).await.is_err());
    assert_eq!(gateway.native_http.drain(Duration::from_secs(3)).await, 0);
    let (state, settled): (String, i64) =
        sqlx::query_as("SELECT state,settled FROM core_charging.requests")
            .fetch_one(&f.pool)
            .await
            .unwrap();
    assert_eq!((state.as_str(), settled), ("reconcile", 0));
    assert_eq!(
        gateway
            .bridge
            .charging
            .reconcile_audio_receipts(10)
            .await
            .unwrap(),
        0
    );
    assert_eq!(supplier.state.hits.load(Ordering::SeqCst), 1);
    let balance = f.ledger.balance(f.account).await.unwrap();
    assert_eq!(
        (balance.available_units, balance.reserved_units),
        (9376, 624)
    );
    drop(gateway);
    runtime.cleanup().await;
}

#[sqlx::test(migrations = false)]
async fn audio_stream_pg_disconnect_before_usage_keeps_hold(pool: PgPool) {
    use tokio_stream::StreamExt;
    let f = native_http::test_support::fixture(pool, 10000, &["audio-public"]).await;
    let runtime = RuntimeRole::create(&f.pool).await;
    runtime.optional("core_charging").await;
    runtime.optional("core_model").await;
    let pause = Arc::new(tokio::sync::Notify::new());
    let supplier = Supplier::start_inner(false, Some(pause.clone())).await;
    let gateway = gateway(&runtime.pool, &supplier);
    let mut r = request(&f.key, "audio-stream-disconnect");
    *r.body_mut()=Body::from(json!({"model":"audio-public","messages":[{"role":"user","content":"input"}],"max_completion_tokens":16,"stream":true}).to_string());
    let reply = gateway.router().oneshot(r).await.unwrap();
    let mut body = reply.into_body().into_data_stream();
    assert!(body.next().await.unwrap().is_ok());
    drop(body);
    pause.notify_one();
    assert_eq!(gateway.native_http.drain(Duration::from_secs(3)).await, 0);
    let (state, settled): (String, i64) =
        sqlx::query_as("SELECT state,settled FROM core_charging.requests")
            .fetch_one(&f.pool)
            .await
            .unwrap();
    assert_eq!((state.as_str(), settled), ("reconcile", 0));
    let balance = f.ledger.balance(f.account).await.unwrap();
    assert_eq!(
        (balance.available_units, balance.reserved_units),
        (9376, 624)
    );
    assert_eq!(supplier.state.hits.load(Ordering::SeqCst), 1);
    drop(gateway);
    runtime.cleanup().await;
}

#[sqlx::test(migrations = false)]
async fn audio_stream_pg_unreviewed_rule_rejects_before_hold(pool: PgPool) {
    let f = native_http::test_support::fixture(pool, 10000, &["audio-public"]).await;
    let supplier = Supplier::start(false).await;
    let mut gateway = gateway(&f.pool, &supplier);
    let mut rule = rule(&supplier.endpoint);
    rule.chat_streaming = false;
    gateway.audio_rules = Arc::new(vec![rule]);
    let mut r = request(&f.key, "audio-stream-disabled");
    *r.body_mut()=Body::from(json!({"model":"audio-public","messages":[{"role":"user","content":"input"}],"max_completion_tokens":16,"stream":true}).to_string());
    assert!(
        !gateway
            .router()
            .oneshot(r)
            .await
            .unwrap()
            .status()
            .is_success()
    );
    let count: i64 = sqlx::query_scalar("SELECT count(*) FROM core_charging.requests")
        .fetch_one(&f.pool)
        .await
        .unwrap();
    assert_eq!(count, 0);
    assert_eq!(supplier.state.hits.load(Ordering::SeqCst), 0);
}

#[sqlx::test(migrations = false)]
async fn audio_stream_pg_eof_tails_are_partition_invariant(pool: PgPool) {
    let f = native_http::test_support::fixture(pool, 20000, &["audio-public"]).await;
    let runtime = RuntimeRole::create(&f.pool).await;
    runtime.optional("core_charging").await;
    runtime.optional("core_model").await;
    for tail in 0..=7 {
        for packed in [false, true] {
            let supplier = Supplier::start_tail(false, None, tail, packed).await;
            let mut gateway = gateway(&runtime.pool, &supplier);
            if tail == 7 {
                gateway.limits.read_timeout = Duration::from_millis(100);
            }
            let mut r = request(&f.key, &format!("audio-tail-{tail}-{packed}"));
            *r.body_mut()=Body::from(json!({"model":"audio-public","messages":[{"role":"user","content":"input"}],"max_completion_tokens":16,"stream":true}).to_string());
            let reply = gateway.router().oneshot(r).await.unwrap();
            let id = reply
                .headers()
                .get("x-request-id")
                .unwrap()
                .to_str()
                .unwrap()
                .to_owned();
            if tail == 6 {
                // Ensure the complete usage packet reached durable storage before
                // allowing the provider to fail its body after the DONE bytes.
                tokio::time::timeout(Duration::from_secs(2),async {
                    loop {
                        let n:i64=sqlx::query_scalar("SELECT count(*) FROM core_charging.usage_events WHERE request_id=$1 AND event_id='audio-packet:v1:0003'").bind(&id).fetch_one(&f.pool).await.unwrap();
                        if n==1 {break;}
                        tokio::task::yield_now().await;
                    }
                }).await.unwrap();
                let n:i64=sqlx::query_scalar("SELECT count(*) FROM core_charging.usage_events WHERE request_id=$1 AND event_id='audio-terminal:v1'").bind(&id).fetch_one(&f.pool).await.unwrap();
                assert_eq!(n, 0);
                supplier.state.tail_gate.notify_one();
            }
            let bytes = to_bytes(reply.into_body(), 1024 * 1024).await;
            assert_eq!(gateway.native_http.drain(Duration::from_secs(3)).await, 0);
            let charge = gateway.bridge.charging.resolve(&id).await.unwrap().unwrap();
            if tail == 0 {
                assert!(bytes.unwrap().ends_with(b"data: [DONE]\n\n"));
                assert_eq!(charge.settled, 62);
            } else {
                assert!(bytes.is_err());
                assert_eq!(
                    (charge.state.as_str(), charge.reserved, charge.settled),
                    ("reconcile", 624, 0)
                );
                let terminal:i64=sqlx::query_scalar("SELECT count(*) FROM core_charging.usage_events WHERE request_id=$1 AND event_id='audio-terminal:v1'").bind(&id).fetch_one(&f.pool).await.unwrap();
                assert_eq!(terminal, 0);
                assert_eq!(
                    gateway
                        .bridge
                        .charging
                        .reconcile_audio_receipts(100)
                        .await
                        .unwrap(),
                    0
                );
            }
            assert_eq!(supplier.state.hits.load(Ordering::SeqCst), 1);
            drop(gateway);
        }
    }
    runtime.cleanup().await;
}

#[sqlx::test(migrations = false)]
async fn audio_chat_v2_pg_official_usage_shape_nonstream_and_stream(pool: PgPool) {
    let f = native_http::test_support::fixture(pool, 10000, &["audio-public"]).await;
    let runtime = RuntimeRole::create(&f.pool).await;
    runtime.optional("core_charging").await;
    runtime.optional("core_model").await;
    let supplier = Supplier::start_style(false, None, 0, false, true).await;
    for stream in [false, true] {
        for format in [
            AudioUsageFormat::OpenAiChatV1,
            AudioUsageFormat::OpenAiChatV2,
        ] {
            let mut contract = rule(&supplier.endpoint);
            contract.price.usage_format = format;
            let gateway = gateway(&runtime.pool, &supplier)
                .with_audio_rules(vec![contract])
                .unwrap();
            let mut r = request(&f.key, &format!("official-{stream}-{format:?}"));
            *r.body_mut()=Body::from(json!({"model":"audio-public","messages":[{"role":"user","content":"input"}],"max_completion_tokens":16,"stream":stream}).to_string());
            let reply = gateway.router().oneshot(r).await.unwrap();
            let status = reply.status();
            let id = reply
                .headers()
                .get("x-request-id")
                .unwrap()
                .to_str()
                .unwrap()
                .to_owned();
            let body = to_bytes(reply.into_body(), 1024 * 1024).await;
            assert_eq!(gateway.native_http.drain(Duration::from_secs(3)).await, 0);
            let charge = gateway.bridge.charging.resolve(&id).await.unwrap().unwrap();
            assert_eq!(charge.price.audio.as_ref().unwrap().usage_format, format);
            if format == AudioUsageFormat::OpenAiChatV2 {
                assert_eq!(status, StatusCode::OK);
                assert!(body.is_ok());
                assert_eq!(charge.settled, 62);
                assert_eq!(
                    charge.usage.audio.unwrap().counts,
                    AudioUsage {
                        text_input: 1,
                        text_output: 2,
                        audio_input: 3,
                        audio_output: 4
                    }
                );
            } else {
                assert_eq!(
                    (charge.state.as_str(), charge.settled, charge.reserved),
                    ("reconcile", 0, 624)
                );
            }
            drop(gateway);
        }
    }
    assert_eq!(supplier.state.hits.load(Ordering::SeqCst), 4);
    runtime.cleanup().await;
}

#[test]
fn audio_chat_v2_request_boundary_excludes_images_and_other_modalities() {
    let image = json!({"model":"m","messages":[{"role":"user","content":[{"type":"image_url","image_url":{"url":"https://example.invalid/image.png"}}]}],"max_completion_tokens":16});
    assert!(ChatRequest::parse(&serde_json::to_vec(&image).unwrap()).is_err());
    let valid = json!({"model":"m","messages":[{"role":"user","content":"text"}],"max_completion_tokens":16});
    assert!(ChatRequest::parse(&serde_json::to_vec(&valid).unwrap()).is_ok());
}

#[test]
fn audio_chat_v2_nonstream_official_shape_and_ambiguous_evidence() {
    let raw = r#"{"id":"chatcmpl-synthetic","object":"chat.completion","model":"m","service_tier":"default","choices":[{"index":0,"message":{"role":"assistant","content":"synthetic"},"finish_reason":"stop"}],"usage":{"prompt_tokens":19,"completion_tokens":10,"total_tokens":29,"prompt_tokens_details":{"audio_tokens":0,"cached_tokens":0,"cache_write_tokens":0},"completion_tokens_details":{"audio_tokens":0,"reasoning_tokens":0,"accepted_prediction_tokens":0,"rejected_prediction_tokens":0}}}"#;
    let (output, usage) = response(raw.as_bytes(), "m", AudioUsageFormat::OpenAiChatV2).unwrap();
    assert_eq!(output["service_tier"], "default");
    assert!(!usage.get().contains("service_tier"));
    assert!(response(raw.as_bytes(), "m", AudioUsageFormat::OpenAiChatV1).is_err());
    for bad in [
        raw.replace("\"audio_tokens\":0", "\"audio_tokens\":null"),
        raw.replace("\"audio_tokens\":0", "\"audio_tokens\":-1"),
        raw.replace("\"audio_tokens\":0", "\"audio_tokens\":1.0"),
        raw.replace(
            "\"audio_tokens\":0",
            "\"audio_tokens\":0,\"image_tokens\":1",
        ),
        raw.replace(
            "\"prompt_tokens\":19",
            "\"prompt_tokens\":20,\"prompt_tokens\":19",
        ),
        raw.replace("\"usage\":{", "\"usage\":null,\"usage\":{"),
        raw.replace(
            "\"role\":\"assistant\"",
            "\"role\":\"assistant\",\"images\":[]",
        ),
    ] {
        assert!(response(bad.as_bytes(), "m", AudioUsageFormat::OpenAiChatV2).is_err());
    }
}

#[path = "legacy_journal_tests.rs"]
mod legacy_journal_tests;

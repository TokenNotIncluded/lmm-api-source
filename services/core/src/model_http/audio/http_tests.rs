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
        async fn answer(
            State(state): State<SupplierState>,
            Json(request): Json<Value>,
        ) -> Json<Value> {
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
            Json(value)
        }
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let endpoint = format!("http://{}/v1", listener.local_addr().unwrap());
        let state = SupplierState {
            hits: Arc::new(AtomicUsize::new(0)),
            missing,
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
fn unsupported_stream_tools_and_unbounded_requests_are_rejected() {
    let base = json!({"model":"audio-public","messages":[{"role":"user","content":"hi"}],"max_completion_tokens":16});
    for (field, value) in [
        ("stream", json!(true)),
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
    // Directly exercise the checkpoint/finalize gap. This is object recreation,
    // not an OS-crash acceptance runner, and does not call an external supplier.
    let f = native_http::test_support::fixture(pool, 10000, &["audio-public"]).await;
    let runtime = RuntimeRole::create(&f.pool).await;
    runtime.optional("core_charging").await;
    runtime.optional("core_model").await;
    let supplier = Supplier::start(false).await;
    let old = gateway(&runtime.pool, &supplier);
    let audio = rule(&supplier.endpoint).price;
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
            projection_revision: "audio-chat-raw-v1".into(),
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
    let raw = r#"{ "prompt_tokens":4,"completion_tokens":6,"total_tokens":10,"prompt_tokens_details":{"text_tokens":1,"audio_tokens":3},"completion_tokens_details":{"text_tokens":2,"audio_tokens":4} }"#;
    old.bridge
        .charging
        .checkpoint_audio(&spec.id, &worker, raw, [5; 32])
        .await
        .unwrap();
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
            .checkpoint_audio(&spec.id, &worker, &changed, [5; 32])
            .await,
        Err(charging::Error::Conflict)
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

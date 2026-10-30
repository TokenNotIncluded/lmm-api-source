//! Request-only text tariffs through real HTTP, runtime PostgreSQL and ledger.
use super::*;
use crate::{
    routing::{CredentialRef, Endpoint, PriceConfig, PriceSnapshot, Protocol, RoutingConfig},
    test_runtime::RuntimeRole,
};
use axum::{body::to_bytes, http::Request as HttpRequest};
use serde_json::Value;
use std::sync::atomic::{AtomicUsize, Ordering};
use tokio::{net::TcpListener, sync::Notify, task::JoinHandle};
use tower::ServiceExt;

pub(super) fn gateway(
    pool: &PgPool,
    base: &str,
    protocol: Protocol,
    names: &[&str],
    version: u64,
    reject: bool,
) -> ModelGateway {
    let routes:Vec<_>=names.iter().enumerate().map(|(i,model)|json!({
        "id":i+1,"model":model,"group":"allowed","enabled":true,"speeds":["standard"],
        "targets":[{"id":i+1,"upstream_id":1,"upstream_model":model,"enabled":true,"priority":0,"weight":1,"speeds":["standard"],"payload":"adapt","timeout_ms":10000}],
        "failover":{"max_attempts":1,"retry_on":[],"allow_replay_after_send":false}
    })).collect();
    let mut config:RoutingConfig=serde_json::from_value(json!({"version":version,"groups":[{"name":"allowed","enabled":true}],
        "upstreams":[{"id":1,"endpoint":"https://synthetic.invalid/v1","protocol":protocol,"credential":{"id":1,"revision":1},"enabled":true}],
        "routes":routes,"aliases":[]})).unwrap();
    config.upstreams[0].endpoint = Endpoint::local_loopback(base.into()).unwrap();
    let rules: Vec<_> = names
        .iter()
        .enumerate()
        .map(|(i, model)| {
            json!({"id":i+1,"model":model,"group":"allowed","speed":"standard",
        "rates":{"input":0,"output":0,"cache_read":0,"cache_write":0,"request":750*version},
        "group_multiplier_ppm":1000000,"speed_multiplier_ppm":1000000})
        })
        .collect();
    let prices: PriceConfig =
        serde_json::from_value(json!({"version":version,"rules":rules})).unwrap();
    let contexts = names
        .iter()
        .filter(|_| reject)
        .map(|model| ExternalModelContext {
            upstream_id: 1,
            upstream_model: (*model).into(),
            protocol,
            endpoint: "https://synthetic.invalid/v1".into(),
            max_input_tokens: 2000,
            max_output_tokens: 1000,
            contract_revision: "synthetic-reviewed-preacceptance-v1".into(),
            missing_cached_tokens_is_zero: true,
            missing_cache_creation_tokens_is_zero: true,
            reservation: Default::default(),
            grounding: None,
            pre_acceptance_rejection_statuses: vec![429],
        })
        .collect();
    ModelGateway::new_with_provider_policy(
        pool.clone(),
        Arc::new(
            crate::routing::Router::new(config, Arc::new(PriceSnapshot::compile(prices).unwrap()))
                .unwrap(),
        ),
        vec![UpstreamCredential {
            reference: CredentialRef { id: 1, revision: 1 },
            secret: "synthetic-text-request-provider".into(),
        }],
        BTreeMap::new(),
        Limits {
            total_timeout: Duration::from_secs(5),
            header_timeout: Duration::from_secs(3),
            hook_timeout: Duration::from_secs(2),
            max_output_tokens: 1000,
            ..Default::default()
        },
        ChargePolicy {
            input_token_ceiling: 2000,
            lease_seconds: 30,
        },
        ProviderPolicy::new(contexts, 2000).unwrap(),
    )
    .unwrap()
    .with_local_streaming()
}

#[derive(Clone, Copy)]
enum Reply {
    Complete(u64),
    MissingUsage,
    Rejected,
    Ambiguous,
    Truncated,
}
#[derive(Clone)]
struct SupplierState {
    reply: Reply,
    hits: Arc<AtomicUsize>,
    pause: Option<(Arc<Notify>, Arc<Notify>)>,
}
struct Supplier {
    base: String,
    state: SupplierState,
    task: JoinHandle<()>,
}
impl Drop for Supplier {
    fn drop(&mut self) {
        self.task.abort();
    }
}
async fn supplier_reply(
    State(state): State<SupplierState>,
    headers: HeaderMap,
    Json(body): Json<Value>,
) -> Response {
    assert_eq!(
        headers[header::AUTHORIZATION],
        "Bearer synthetic-text-request-provider"
    );
    state.hits.fetch_add(1, Ordering::SeqCst);
    if let Some((entered, resume)) = &state.pause {
        entered.notify_one();
        resume.notified().await;
    }
    match state.reply {
        Reply::Rejected|Reply::Ambiguous => (StatusCode::TOO_MANY_REQUESTS,Json(json!({"error":{"type":"rate_limit_error","message":"synthetic rejected"}}))).into_response(),
        Reply::Truncated => ([(header::CONTENT_TYPE,"text/event-stream")],"data: {\"id\":\"synthetic\",\"object\":\"chat.completion.chunk\",\"model\":\"text\",\"choices\":[{\"index\":0,\"delta\":{\"role\":\"assistant\",\"content\":\"partial\"}}]}\n\n").into_response(),
        mode => {
            let input=if let Reply::Complete(tokens)=mode {tokens}else{17};
            let usage=json!({"prompt_tokens":input,"completion_tokens":0,"prompt_tokens_details":{"cached_tokens":0}});
            if body["stream"]==true {
                let chunk=json!({"id":"synthetic","object":"chat.completion.chunk","model":"text","choices":[{"index":0,"delta":{"role":"assistant","content":"done"},"finish_reason":"stop"}]});
                let measured=json!({"id":"synthetic","object":"chat.completion.chunk","model":"text","choices":[],"usage":usage});
                return ([(header::CONTENT_TYPE,"text/event-stream")],format!("data: {chunk}\n\ndata: {measured}\n\ndata: [DONE]\n\n")).into_response();
            }
            let mut value=json!({"id":"synthetic","object":"chat.completion","model":"text","choices":[{"index":0,"message":{"role":"assistant","content":"done"},"finish_reason":"stop"}],"usage":usage});
            if matches!(mode,Reply::MissingUsage){value.as_object_mut().unwrap().remove("usage");}
            Json(value).into_response()
        }
    }
}
impl Supplier {
    async fn start(reply: Reply, paused: bool) -> Self {
        let state = SupplierState {
            reply,
            hits: Arc::new(AtomicUsize::new(0)),
            pause: paused.then(|| (Arc::new(Notify::new()), Arc::new(Notify::new()))),
        };
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let base = format!("http://{}/v1", listener.local_addr().unwrap());
        let app = Router::new()
            .route("/v1/chat/completions", post(supplier_reply))
            .with_state(state.clone());
        let task = tokio::spawn(async move {
            axum::serve(listener, app).await.unwrap();
        });
        Self { base, state, task }
    }
}
fn request(secret: &str, key: &str, stream: bool) -> HttpRequest<Body> {
    HttpRequest::builder().method("POST").uri("/v1/chat/completions").header(header::AUTHORIZATION,format!("Bearer {secret}"))
        .header(header::CONTENT_TYPE,"application/json").header("idempotency-key",key)
        .body(Body::from(json!({"model":"text","messages":[{"role":"user","content":"test"}],"max_tokens":8,"stream":stream}).to_string())).unwrap()
}
async fn call(g: &ModelGateway, request: HttpRequest<Body>) -> (StatusCode, HeaderMap, Vec<u8>) {
    let response = g.router().oneshot(request).await.unwrap();
    let status = response.status();
    let headers = response.headers().clone();
    let body = to_bytes(response.into_body(), 1 << 20)
        .await
        .map(|b| b.to_vec())
        .unwrap_or_default();
    // The detached relay always finalizes, including a client-visible stream error.
    tokio::time::timeout(Duration::from_secs(4), async {
        while !g.bridge.active.lock().unwrap().is_empty() {
            tokio::time::sleep(Duration::from_millis(5)).await;
        }
    })
    .await
    .unwrap();
    (status, headers, body)
}
async fn runtime(f: &native_http::test_support::Fixture) -> RuntimeRole {
    let role = RuntimeRole::create(&f.pool).await;
    role.optional("core_charging").await;
    role.optional("core_model").await;
    role
}
async fn usage(pool: &PgPool, session: &str, path: &str) -> Value {
    let response = crate::usage::router(crate::usage::UsageStore::new(pool.clone()))
        .oneshot(
            HttpRequest::builder()
                .uri(path)
                .header(header::AUTHORIZATION, format!("Bearer {session}"))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    serde_json::from_slice(&to_bytes(response.into_body(), 1 << 20).await.unwrap()).unwrap()
}

#[sqlx::test(migrations = false)]
async fn text_request_pg_completed_zero_token_stream_and_duplicate_charge_once(pool: PgPool) {
    let f = native_http::test_support::fixture(pool, 10000, &["text"]).await;
    let role = runtime(&f).await;
    for (index, (tokens, stream)) in [(0, false), (31, false), (17, true)]
        .into_iter()
        .enumerate()
    {
        let supplier = Supplier::start(Reply::Complete(tokens), false).await;
        let g = gateway(
            &role.pool,
            &supplier.base,
            Protocol::OpenAiChat,
            &["text"],
            1,
            false,
        );
        let key = format!("complete-{index}");
        let (a, b) = tokio::join!(
            call(&g, request(&f.key, &key, stream)),
            call(&g, request(&f.key, &key, stream))
        );
        assert_eq!(
            [a.0, b.0]
                .into_iter()
                .filter(|s| *s == StatusCode::OK)
                .count(),
            1
        );
        assert_eq!(
            [a.0, b.0]
                .into_iter()
                .filter(|s| *s == StatusCode::CONFLICT)
                .count(),
            1
        );
        let response = if a.0 == StatusCode::OK { a } else { b };
        let id = response.1["x-request-id"].to_str().unwrap();
        assert!(!response.2.is_empty());
        if stream {
            assert!(String::from_utf8_lossy(&response.2).ends_with("data: [DONE]\n\n"));
        }
        let charge = g.bridge.charging.resolve(id).await.unwrap().unwrap();
        assert_eq!(
            (charge.state.as_str(), charge.reserved, charge.settled),
            ("settled", 375, 375)
        );
        assert_eq!(
            charge.usage.text_request,
            Some(text_request_price::TextRequestUsage::Completed)
        );
        assert_eq!(
            (charge.usage.input, charge.usage.output),
            (tokens as i64, 0)
        );
        let log = usage(
            &role.pool,
            &f.session,
            &format!("/api/log/self?request_id={id}"),
        )
        .await;
        assert_eq!(
            log["data"]["items"][0]["billing_method"],
            "per_completed_request"
        );
        assert_eq!(log["data"]["items"][0]["completed_requests"], 1);
        assert_eq!(log["data"]["items"][0]["token_usage_available"], true);
        assert_eq!(supplier.state.hits.load(Ordering::SeqCst), 1);
    }
    let balance = f.ledger.balance(f.account).await.unwrap();
    assert_eq!((balance.available_units, balance.reserved_units), (8875, 0));
    role.cleanup().await;
}

#[sqlx::test(migrations = false)]
async fn text_request_pg_reviewed_rejection_zero_but_ambiguous_and_truncated_hold(pool: PgPool) {
    let f = native_http::test_support::fixture(pool, 10000, &["text"]).await;
    let role = runtime(&f).await;
    for (index, (mode, reviewed, stream, settled)) in [
        (Reply::Rejected, true, false, true),
        (Reply::Ambiguous, false, false, false),
        (Reply::MissingUsage, false, false, false),
        (Reply::Truncated, false, true, false),
    ]
    .into_iter()
    .enumerate()
    {
        let supplier = Supplier::start(mode, false).await;
        let g = gateway(
            &role.pool,
            &supplier.base,
            Protocol::OpenAiChat,
            &["text"],
            1,
            reviewed,
        );
        let key = format!("evidence-{index}");
        let (_, headers, _) = call(&g, request(&f.key, &key, stream)).await;
        let id = headers["x-request-id"].to_str().unwrap();
        let charge = g.bridge.charging.resolve(id).await.unwrap().unwrap();
        assert_eq!(charge.state, if settled { "settled" } else { "reconcile" });
        assert_eq!(charge.settled, 0);
        assert_eq!(charge.reserved, 375);
        if settled {
            assert!(matches!(
                charge.usage.text_request,
                Some(text_request_price::TextRequestUsage::Rejected {
                    http_status: 429,
                    ..
                })
            ));
            assert_eq!(charge.outcome.as_deref(), Some("failed"));
        } else {
            assert_eq!(charge.usage.text_request, None);
        }
        assert_eq!(g.reconcile_completed_streams(10).await.unwrap(), 0);
        assert_eq!(
            call(&g, request(&f.key, &key, stream)).await.0,
            StatusCode::CONFLICT
        );
        assert_eq!(supplier.state.hits.load(Ordering::SeqCst), 1);
        let log = usage(
            &role.pool,
            &f.session,
            &format!("/api/log/self?request_id={id}"),
        )
        .await;
        assert_eq!(log["data"]["items"][0]["token_usage_available"], false);
        assert_eq!(
            log["data"]["items"][0]["completed_requests"],
            if settled { json!(0) } else { Value::Null }
        );
    }
    let balance = f.ledger.balance(f.account).await.unwrap();
    assert_eq!(
        (balance.available_units, balance.reserved_units),
        (8875, 1125)
    );
    role.cleanup().await;
}

#[sqlx::test(migrations = false)]
async fn text_request_pg_receipts_recover_original_price_for_completion_and_rejection(
    pool: PgPool,
) {
    let f = native_http::test_support::fixture(pool, 10000, &["text"]).await;
    let role = runtime(&f).await;
    for (index, (mode, expected, outcome)) in [
        (Reply::Complete(11), 375, "completed"),
        (Reply::Rejected, 0, "failed"),
    ]
    .into_iter()
    .enumerate()
    {
        let supplier = Supplier::start(mode, true).await;
        let initial = gateway(
            &role.pool,
            &supplier.base,
            Protocol::OpenAiChat,
            &["text"],
            1,
            true,
        );
        let running = initial.clone();
        let key = format!("recover-text-{index}");
        let req = request(&f.key, &key, false);
        let task = tokio::spawn(async move { call(&running, req).await });
        let (entered, resume) = supplier.state.pause.as_ref().unwrap();
        tokio::time::timeout(Duration::from_secs(3), entered.notified())
            .await
            .unwrap();
        sqlx::raw_sql(&format!(
            "REVOKE EXECUTE ON FUNCTION core_billing.post_ledger(jsonb) FROM {}",
            role.name
        ))
        .execute(&f.pool)
        .await
        .unwrap();
        resume.notify_one();
        let (_, headers, _) = task.await.unwrap();
        let id = headers["x-request-id"].to_str().unwrap();
        let pending = initial.bridge.charging.resolve(id).await.unwrap().unwrap();
        assert_eq!(pending.state, "reconcile");
        assert_eq!(
            pending
                .price
                .text_request
                .as_ref()
                .unwrap()
                .usd_micros_per_request,
            750
        );
        drop(initial);
        sqlx::raw_sql(&format!(
            "GRANT EXECUTE ON FUNCTION core_billing.post_ledger(jsonb) TO {}",
            role.name
        ))
        .execute(&f.pool)
        .await
        .unwrap();
        let restarted = gateway(
            &role.pool,
            &supplier.base,
            Protocol::OpenAiChat,
            &["text"],
            3,
            false,
        );
        assert_eq!(restarted.reconcile_completed_streams(10).await.unwrap(), 1);
        assert_eq!(restarted.reconcile_completed_streams(10).await.unwrap(), 0);
        let charge = restarted
            .bridge
            .charging
            .resolve(id)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(
            (charge.state.as_str(), charge.settled),
            ("settled", expected)
        );
        assert_eq!(charge.outcome.as_deref(), Some(outcome));
        assert_eq!(
            charge
                .price
                .text_request
                .as_ref()
                .unwrap()
                .usd_micros_per_request,
            750
        );
        assert_eq!(supplier.state.hits.load(Ordering::SeqCst), 1);
        assert_eq!(
            call(&restarted, request(&f.key, &key, false)).await.0,
            StatusCode::CONFLICT
        );
    }
    role.cleanup().await;
}

#[sqlx::test(migrations = false)]
async fn text_request_pg_permissions_and_funds_prevent_dispatch(pool: PgPool) {
    let f = native_http::test_support::fixture(pool, 100, &["text"]).await;
    let supplier = Supplier::start(Reply::Complete(10), false).await;
    let g = gateway(
        &f.pool,
        &supplier.base,
        Protocol::OpenAiChat,
        &["text"],
        1,
        false,
    );
    assert_eq!(
        call(&g, request(&f.session, "session", false)).await.0,
        StatusCode::FORBIDDEN
    );
    assert_eq!(
        call(&g, request(&f.key, "no-funds", false)).await.0,
        StatusCode::PAYMENT_REQUIRED
    );
    let mut missing = request(&f.key, "missing-route", false);
    *missing.body_mut() = Body::from(
        json!({"model":"not-configured",
        "messages":[{"role":"user","content":"test"}],"max_tokens":8})
        .to_string(),
    );
    assert_eq!(call(&g, missing).await.0, StatusCode::SERVICE_UNAVAILABLE);
    sqlx::query("UPDATE core_identity.api_key_settings SET model_limits=ARRAY['other'] WHERE credential_id=$1").bind(f.key_id).execute(&f.pool).await.unwrap();
    assert_eq!(
        call(&g, request(&f.key, "forbidden", false)).await.0,
        StatusCode::FORBIDDEN
    );
    assert_eq!(supplier.state.hits.load(Ordering::SeqCst), 0);
    assert_eq!(
        sqlx::query_scalar::<_, i64>("SELECT count(*) FROM core_charging.requests")
            .fetch_one(&f.pool)
            .await
            .unwrap(),
        0
    );
}

#[sqlx::test(migrations = false)]
async fn text_request_pg_connect_failure_has_typed_zero_and_releases_hold(pool: PgPool) {
    let f = native_http::test_support::fixture(pool, 10000, &["text"]).await;
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let base = format!("http://{}/v1", listener.local_addr().unwrap());
    drop(listener);
    let g = gateway(&f.pool, &base, Protocol::OpenAiChat, &["text"], 1, false);
    let (status, headers, _) = call(&g, request(&f.key, "not-sent", false)).await;
    assert!(!status.is_success());
    let id = headers["x-request-id"].to_str().unwrap();
    let charge = g.bridge.charging.resolve(id).await.unwrap().unwrap();
    assert_eq!((charge.state.as_str(), charge.settled), ("settled", 0));
    assert_eq!(
        charge.usage.text_request,
        Some(text_request_price::TextRequestUsage::NotSent)
    );
    assert_eq!(charge.outcome.as_deref(), Some("cancelled"));
    assert_eq!(
        call(&g, request(&f.key, "not-sent", false)).await.0,
        StatusCode::CONFLICT
    );
    let balance = f.ledger.balance(f.account).await.unwrap();
    assert_eq!(
        (balance.available_units, balance.reserved_units),
        (10000, 0)
    );
}

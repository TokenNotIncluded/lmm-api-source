//! Real native-tool HTTP dispatch, disposable PostgreSQL and the Core ledger.
use super::*;
use crate::{
    routing::{CredentialRef, Endpoint, PriceConfig, PriceSnapshot, RoutingConfig},
    test_runtime::RuntimeRole,
};
use axum::{Router as HttpRouter, extract::OriginalUri, http::Request as HttpRequest};
use native_http::test_support::{Fixture, fixture};
use std::sync::atomic::{AtomicUsize, Ordering};
use tokio::{net::TcpListener, sync::Notify, task::JoinHandle};
use tower::ServiceExt;

#[derive(Clone, Copy)]
enum Reply {
    Search(usize),
    Fetch(&'static str),
    Missing,
    Error(u16, bool),
    ErrorWithCredential,
}
#[derive(Clone)]
struct SupplierState {
    hits: Arc<AtomicUsize>,
    seen: Arc<Mutex<Vec<(String, Value)>>>,
    reply: Reply,
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
async fn reply(
    State(state): State<SupplierState>,
    OriginalUri(uri): OriginalUri,
    headers: HeaderMap,
    Json(body): Json<Value>,
) -> Response {
    assert_eq!(
        headers[header::AUTHORIZATION],
        "Bearer native-tool-synthetic-provider"
    );
    state.hits.fetch_add(1, Ordering::SeqCst);
    state.seen.lock().unwrap().push((uri.path().into(), body));
    if let Some((entered, resume)) = &state.pause {
        entered.notify_one();
        resume.notified().await;
    }
    let (status, content_type, body) = match state.reply {
        Reply::Search(count) => {
            let results: Vec<_> = (0..count).map(|i| json!({"title":format!("Result {i}"),"url":format!("https://example.invalid/{i}")})).collect();
            (
                200,
                "application/json",
                json!({"results":results}).to_string(),
            )
        }
        Reply::Fetch(content) => (
            200,
            "application/json",
            json!({"content":content}).to_string(),
        ),
        Reply::Missing => (
            200,
            "application/json",
            json!({"unknown_provider_field":true}).to_string(),
        ),
        Reply::Error(status, true) => (
            status,
            "application/json",
            json!({"error":{"message":"Synthetic provider rejection"}}).to_string(),
        ),
        Reply::Error(status, false) => {
            (status, "text/plain", "Synthetic provider rejection".into())
        }
        Reply::ErrorWithCredential => {
            return (
                StatusCode::BAD_GATEWAY,
                [(header::CONTENT_TYPE, "text/plain")],
                b"\xffnative-tool-synthetic-provider".to_vec(),
            )
                .into_response();
        }
    };
    (
        StatusCode::from_u16(status).unwrap(),
        [(header::CONTENT_TYPE, content_type)],
        body,
    )
        .into_response()
}
impl Supplier {
    async fn start(reply: Reply, paused: bool) -> Self {
        let state = SupplierState {
            hits: Arc::new(AtomicUsize::new(0)),
            seen: Arc::new(Mutex::new(Vec::new())),
            reply,
            pause: paused.then(|| (Arc::new(Notify::new()), Arc::new(Notify::new()))),
        };
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let base = format!("http://{}/v1", listener.local_addr().unwrap());
        let app = HttpRouter::new()
            .route("/v1/tools/search", post(self::reply))
            .route("/v1/tools/fetch", post(self::reply))
            .with_state(state.clone());
        let task = tokio::spawn(async move {
            axum::serve(listener, app).await.unwrap();
        });
        Self { base, state, task }
    }
}

fn gateway(pool: &PgPool, supplier: &Supplier, price_version: u64) -> ModelGateway {
    let kinds = [ToolKind::DeepSeekSearch, ToolKind::DeepSeekFetch];
    let upstreams: Vec<_> = kinds
        .iter()
        .enumerate()
        .map(|(i, kind)| {
            json!({
                "id":i+1,"endpoint":"https://synthetic.invalid/v1","protocol":kind.protocol_name(),
                "credential":{"id":7,"revision":1},"enabled":true
            })
        })
        .collect();
    let rows: Vec<_> = kinds.iter().enumerate().map(|(i, kind)| json!({
        "id":i+1,"model":kind.model(),"group":"allowed","enabled":true,"speeds":["standard"],
        "targets":[{"id":i+1,"upstream_id":i+1,"upstream_model":kind.model(),"enabled":true,"priority":0,"weight":1,"speeds":["standard"],"payload":"adapt","timeout_ms":10000}],
        "failover":{"max_attempts":1,"retry_on":[],"allow_replay_after_send":false}
    })).collect();
    let mut routing: RoutingConfig = serde_json::from_value(json!({
        "version":price_version,"groups":[{"name":"allowed","enabled":true}],
        "upstreams":upstreams,"routes":rows,"aliases":[]
    }))
    .unwrap();
    for upstream in &mut routing.upstreams {
        upstream.endpoint = Endpoint::local_loopback(supplier.base.clone()).unwrap();
    }
    let rules: Vec<_> = kinds.iter().enumerate().map(|(i, kind)| json!({
        "id":i+1,"model":kind.model(),"group":"allowed","speed":"standard",
        "rates":{"input":0,"output":0,"cache_read":0,"cache_write":0,"request":(if i == 0 {750} else {100})*price_version},
        "group_multiplier_ppm":1000000,"speed_multiplier_ppm":1000000
    })).collect();
    let prices: PriceConfig =
        serde_json::from_value(json!({"version":price_version,"rules":rules})).unwrap();
    ModelGateway::new(
        pool.clone(),
        Arc::new(
            crate::routing::Router::new(routing, Arc::new(PriceSnapshot::compile(prices).unwrap()))
                .unwrap(),
        ),
        vec![UpstreamCredential {
            reference: CredentialRef { id: 7, revision: 1 },
            secret: "native-tool-synthetic-provider".into(),
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
}
fn request(secret: &str, key: &str, kind: ToolKind) -> HttpRequest<Body> {
    let body = match kind {
        ToolKind::DeepSeekSearch => {
            json!({"query":"rust site:example.invalid","count":50,"need_content":true,"price":0})
        }
        ToolKind::DeepSeekFetch => json!({"url":"https://example.invalid/document","price":0}),
    };
    HttpRequest::builder()
        .method("POST")
        .uri(format!("/v1/{}", kind.endpoint()))
        .header(header::AUTHORIZATION, format!("Bearer {secret}"))
        .header(header::CONTENT_TYPE, "application/json")
        .header("idempotency-key", key)
        .body(Body::from(body.to_string()))
        .unwrap()
}
async fn call(
    gateway: &ModelGateway,
    request: HttpRequest<Body>,
) -> (StatusCode, HeaderMap, Vec<u8>) {
    let response = gateway.router().oneshot(request).await.unwrap();
    let status = response.status();
    let headers = response.headers().clone();
    let bytes = to_bytes(response.into_body(), 1 << 20)
        .await
        .unwrap()
        .to_vec();
    (status, headers, bytes)
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
    let status = response.status();
    let bytes = to_bytes(response.into_body(), 1 << 20).await.unwrap();
    assert_eq!(
        status,
        StatusCode::OK,
        "{}",
        String::from_utf8_lossy(&bytes)
    );
    serde_json::from_slice(&bytes).unwrap()
}
async fn runtime(f: &Fixture) -> RuntimeRole {
    let role = RuntimeRole::create(&f.pool).await;
    role.optional("core_charging").await;
    role.optional("core_model").await;
    role
}

#[sqlx::test(migrations = false)]
async fn tool_api_pg_success_empty_errors_and_native_usage_projection(pool: PgPool) {
    let f = fixture(pool, 10000, &["deepseek-search", "deepseek-fetch"]).await;
    let role = runtime(&f).await;
    let scenarios = [
        (ToolKind::DeepSeekSearch, Reply::Search(1), 200, 1),
        (ToolKind::DeepSeekSearch, Reply::Search(3), 200, 1),
        (
            ToolKind::DeepSeekFetch,
            Reply::Fetch("Fetched content"),
            200,
            1,
        ),
        (ToolKind::DeepSeekSearch, Reply::Search(0), 200, 0),
        (ToolKind::DeepSeekFetch, Reply::Fetch(""), 200, 0),
        (ToolKind::DeepSeekFetch, Reply::Fetch(" \n"), 200, 0),
        (ToolKind::DeepSeekSearch, Reply::Error(429, true), 429, 0),
        (ToolKind::DeepSeekFetch, Reply::Error(503, false), 503, 0),
    ];
    let mut consumed = 0;
    for (index, (kind, mode, http_status, successful)) in scenarios.into_iter().enumerate() {
        let supplier = Supplier::start(mode, false).await;
        let gateway = gateway(&role.pool, &supplier, 1);
        let key = format!("tool-case-{index}");
        let (status, headers, bytes) = call(&gateway, request(&f.key, &key, kind)).await;
        assert_eq!(
            status.as_u16(),
            http_status,
            "{}",
            String::from_utf8_lossy(&bytes)
        );
        assert_eq!(headers[header::CACHE_CONTROL], "no-store");
        let id = headers["x-request-id"].to_str().unwrap();
        let charge = gateway.bridge.charging.resolve(id).await.unwrap().unwrap();
        let reserve = if kind == ToolKind::DeepSeekSearch {
            375
        } else {
            50
        };
        let cost = reserve * i64::from(successful);
        consumed += cost;
        assert_eq!(charge.state, "settled");
        let outcome = if http_status == 200 {
            "completed"
        } else {
            "failed"
        };
        assert_eq!(charge.outcome.as_deref(), Some(outcome));
        assert_eq!(charge.key_id, Some(f.key_id));
        assert_eq!((charge.usage.input, charge.usage.output), (0, 0));
        let receipt = charge.usage.tool.unwrap();
        assert_eq!(
            (
                receipt.kind,
                receipt.successful_requests,
                receipt.http_status
            ),
            (kind, successful, http_status)
        );
        assert_eq!((charge.reserved, charge.settled), (reserve, cost));
        let (_, contract) = gateway
            .bridge
            .charging
            .reservation_contract(id)
            .await
            .unwrap();
        assert_eq!(contract.bounds(), (0, 0));
        assert_eq!(
            call(&gateway, request(&f.key, &key, kind)).await.0,
            StatusCode::CONFLICT
        );
        assert_eq!(supplier.state.hits.load(Ordering::SeqCst), 1);
        let seen = supplier.state.seen.lock().unwrap();
        assert_eq!(seen[0].0, format!("/v1/{}", kind.endpoint()));
        assert!(seen[0].1.get("price").is_none());
        assert!(seen[0].1.get("model").is_none());
        drop(seen);

        let logs = usage(
            &role.pool,
            &f.session,
            &format!("/api/log/self?request_id={id}"),
        )
        .await;
        assert_eq!(logs["data"]["total"], 1);
        let row = &logs["data"]["items"][0];
        assert_eq!(row["type"], if http_status == 200 { 2 } else { 5 });
        assert_eq!(row["content"], outcome);
        assert_eq!(row["pricing_dimension"], "tool");
        assert_eq!(row["billing_method"], "per_successful_request");
        assert_eq!(row["tool_requests"], successful);
        assert_eq!(row["net_consumption_credits"], cost);
        assert_eq!(row["token_usage_available"], false);
        assert!(row.get("prompt_tokens").is_none_or(Value::is_null));
        let data = usage(
            &role.pool,
            &f.session,
            &format!("/api/data/self?request_id={id}"),
        )
        .await;
        let buckets = data["data"].as_array().unwrap();
        assert_eq!(buckets.len(), 1, "{data}");
        assert_eq!(buckets[0]["tool_requests"], successful);
        assert_eq!(buckets[0]["net_consumption_credits"], cost);
        assert!(buckets[0]["token_used"].is_null());
        assert!(buckets[0]["grounded_requests"].is_null());

        let projected = super::super::pricing::PricingSurface::from_gateway(&gateway)
            .unwrap()
            .project(None, None)
            .unwrap();
        assert_eq!(projected.models.len(), 2);
        for row in projected.models {
            assert_eq!(row.billing_status, "configured");
            assert_eq!(row.apis, [row.model.as_str()]);
            let price = row.token_price.unwrap();
            assert_eq!(price.unit, "successful_requests");
            assert!(price.input.is_none() && price.output.is_none());
            assert!(price.request.is_some());
        }
        let catalog = gateway
            .oauth_catalog_surface()
            .project(&["allowed".into()], "https://synthetic.invalid/oauth2", 123)
            .unwrap();
        assert_eq!(catalog.models.len(), 2);
        assert!(
            catalog
                .models
                .iter()
                .all(|m| m.pricing.unit == "successful_requests"
                    && m.pricing.request.is_some()
                    && m.pricing.input.is_none())
        );
    }
    let balance = f.ledger.balance(f.account).await.unwrap();
    assert_eq!(
        (balance.available_units, balance.reserved_units),
        (10000 - consumed, 0)
    );
    role.cleanup().await;
}

#[sqlx::test(migrations = false)]
async fn tool_api_pg_missing_structure_keeps_hold_and_never_replays(pool: PgPool) {
    let f = fixture(pool, 10000, &["deepseek-search", "deepseek-fetch"]).await;
    let role = runtime(&f).await;
    for (index, (kind, reply)) in [
        (ToolKind::DeepSeekSearch, Reply::Missing),
        (ToolKind::DeepSeekFetch, Reply::Missing),
        (ToolKind::DeepSeekSearch, Reply::ErrorWithCredential),
    ]
    .into_iter()
    .enumerate()
    {
        let supplier = Supplier::start(reply, false).await;
        let gateway = gateway(&role.pool, &supplier, 1);
        let key = format!("unusable-tool-{index}");
        let (status, headers, bytes) = call(&gateway, request(&f.key, &key, kind)).await;
        assert!(!status.is_success());
        assert!(
            !bytes
                .windows(b"native-tool-synthetic-provider".len())
                .any(|chunk| chunk == b"native-tool-synthetic-provider")
        );
        let id = headers["x-request-id"].to_str().unwrap();
        let charge = gateway.bridge.charging.resolve(id).await.unwrap().unwrap();
        assert_eq!(charge.state, "reconcile");
        assert_eq!(charge.settled, 0);
        assert_eq!(
            charge.reserved,
            if kind == ToolKind::DeepSeekSearch {
                375
            } else {
                50
            }
        );
        assert!(charge.usage.tool.is_none());
        assert_eq!(gateway.reconcile_completed_streams(10).await.unwrap(), 0);
        assert_eq!(
            call(&gateway, request(&f.key, &key, kind)).await.0,
            StatusCode::CONFLICT
        );
        assert_eq!(supplier.state.hits.load(Ordering::SeqCst), 1);
        let logs = usage(
            &role.pool,
            &f.session,
            &format!("/api/log/self?request_id={id}"),
        )
        .await;
        let row = &logs["data"]["items"][0];
        assert_eq!(row["state"], "reconcile");
        assert!(row["tool_requests"].is_null());
        assert!(row["net_consumption_credits"].is_null());
    }
    let balance = f.ledger.balance(f.account).await.unwrap();
    assert_eq!(
        (balance.available_units, balance.reserved_units),
        (9200, 800)
    );
    role.cleanup().await;
}

#[sqlx::test(migrations = false)]
async fn tool_api_pg_concurrent_same_key_sends_once_and_auth_denials_do_not_reserve(pool: PgPool) {
    let f = fixture(pool, 10000, &["deepseek-search"]).await;
    let role = runtime(&f).await;
    let supplier = Supplier::start(Reply::Search(3), false).await;
    let gateway = gateway(&role.pool, &supplier, 1);
    assert_eq!(
        call(
            &gateway,
            request("invalid", "invalid", ToolKind::DeepSeekSearch)
        )
        .await
        .0,
        StatusCode::UNAUTHORIZED
    );
    assert_eq!(
        call(
            &gateway,
            request(&f.session, "session", ToolKind::DeepSeekSearch)
        )
        .await
        .0,
        StatusCode::FORBIDDEN
    );
    assert_eq!(
        call(
            &gateway,
            request(&f.key, "forbidden", ToolKind::DeepSeekFetch)
        )
        .await
        .0,
        StatusCode::FORBIDDEN
    );
    assert_eq!(supplier.state.hits.load(Ordering::SeqCst), 0);
    let (a, b) = tokio::join!(
        call(
            &gateway,
            request(&f.key, "same-tool", ToolKind::DeepSeekSearch)
        ),
        call(
            &gateway,
            request(&f.key, "same-tool", ToolKind::DeepSeekSearch)
        )
    );
    assert_eq!(
        [a.0, b.0].iter().filter(|s| **s == StatusCode::OK).count(),
        1
    );
    assert_eq!(
        [a.0, b.0]
            .iter()
            .filter(|s| **s == StatusCode::CONFLICT)
            .count(),
        1
    );
    assert_eq!(supplier.state.hits.load(Ordering::SeqCst), 1);
    assert_eq!(
        call(
            &gateway,
            request(&f.key, "same-tool", ToolKind::DeepSeekSearch)
        )
        .await
        .0,
        StatusCode::CONFLICT
    );
    let count: i64 = sqlx::query_scalar("SELECT count(*) FROM core_charging.requests")
        .fetch_one(&f.pool)
        .await
        .unwrap();
    assert_eq!(count, 1);
    let balance = f.ledger.balance(f.account).await.unwrap();
    assert_eq!((balance.available_units, balance.reserved_units), (9625, 0));
    drop(gateway);
    role.cleanup().await;
}

#[sqlx::test(migrations = false)]
async fn tool_api_pg_durable_receipt_recovers_once_at_original_price(pool: PgPool) {
    let f = fixture(pool, 10000, &["deepseek-search"]).await;
    let role = runtime(&f).await;
    for (index, (reply, http_status, successful, expected_cost, outcome)) in [
        (Reply::Search(3), 200, 1, 375, "completed"),
        (Reply::Error(503, false), 503, 0, 0, "failed"),
    ]
    .into_iter()
    .enumerate()
    {
        let supplier = Supplier::start(reply, true).await;
        let initial = gateway(&role.pool, &supplier, 1);
        let running = initial.clone();
        let key = format!("recover-tool-{index}");
        let req = request(&f.key, &key, ToolKind::DeepSeekSearch);
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
        let (status, headers, _) = task.await.unwrap();
        assert!(!status.is_success());
        let id = headers["x-request-id"].to_str().unwrap();
        let pending = initial.bridge.charging.resolve(id).await.unwrap().unwrap();
        assert_eq!(pending.state, "reconcile");
        assert_eq!(pending.outcome, None);
        assert_eq!(
            pending.price.tool.as_ref().unwrap().usd_micros_per_request,
            750
        );
        let checkpoint: Value = sqlx::query_scalar("SELECT provider_usage FROM core_model.output_checkpoints WHERE request_id=$1 AND terminal_observed=TRUE ORDER BY sequence DESC LIMIT 1").bind(id).fetch_one(&f.pool).await.unwrap();
        let saved: relay::Usage = serde_json::from_value(checkpoint).unwrap();
        let receipt = saved.tool.unwrap();
        assert_eq!(
            (receipt.successful_requests, receipt.http_status),
            (successful, http_status)
        );
        drop(initial);
        sqlx::raw_sql(&format!(
            "GRANT EXECUTE ON FUNCTION core_billing.post_ledger(jsonb) TO {}",
            role.name
        ))
        .execute(&f.pool)
        .await
        .unwrap();
        let restarted = gateway(&role.pool, &supplier, 3);
        assert_eq!(restarted.reconcile_completed_streams(10).await.unwrap(), 1);
        assert_eq!(restarted.reconcile_completed_streams(10).await.unwrap(), 0);
        let charge = restarted
            .bridge
            .charging
            .resolve(id)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(charge.state, "settled");
        assert_eq!(charge.outcome.as_deref(), Some(outcome));
        assert_eq!(charge.settled, expected_cost);
        assert_eq!(
            charge.price.tool.as_ref().unwrap().usd_micros_per_request,
            750
        );
        assert_eq!(charge.usage.tool, saved.tool);
        let logs = usage(
            &role.pool,
            &f.session,
            &format!("/api/log/self?request_id={id}"),
        )
        .await;
        let row = &logs["data"]["items"][0];
        assert_eq!(row["type"], if http_status == 200 { 2 } else { 5 });
        assert_eq!(row["content"], outcome);
        assert_eq!(row["net_consumption_credits"], expected_cost);
        assert_eq!(supplier.state.hits.load(Ordering::SeqCst), 1);
        assert_eq!(
            call(&restarted, request(&f.key, &key, ToolKind::DeepSeekSearch))
                .await
                .0,
            StatusCode::CONFLICT
        );
        drop(restarted);
    }
    let balance = f.ledger.balance(f.account).await.unwrap();
    assert_eq!((balance.available_units, balance.reserved_units), (9625, 0));
    role.cleanup().await;
}

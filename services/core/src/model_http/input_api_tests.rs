//! Synthetic local suppliers with real Core identity, PostgreSQL and ledger.
use super::*;
use crate::{
    billing::{
        Credits,
        ledger::{Action, Request as LedgerRequest},
    },
    routing::{CredentialRef, Endpoint, PriceConfig, PriceSnapshot, RoutingConfig},
    test_runtime::RuntimeRole,
};
use axum::{Router as HttpRouter, http::Request as HttpRequest};
use serde_json::Value;
use std::sync::atomic::{AtomicUsize, Ordering};
use tokio::{net::TcpListener, sync::Notify, task::JoinHandle};
use tower::ServiceExt;

#[derive(Clone, Copy)]
enum Reply {
    Valid,
    MissingUsage,
    ExcessUsage,
}
#[derive(Clone)]
struct SupplierState {
    hits: Arc<AtomicUsize>,
    seen: Arc<Mutex<Vec<Value>>>,
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
    headers: HeaderMap,
    Json(body): Json<Value>,
) -> Json<Value> {
    assert_eq!(
        headers[header::AUTHORIZATION],
        "Bearer input-only-synthetic-provider"
    );
    state.hits.fetch_add(1, Ordering::SeqCst);
    state.seen.lock().unwrap().push(body.clone());
    if let Some((entered, resume)) = &state.pause {
        entered.notify_one();
        resume.notified().await;
    }
    let mut response = match body["model"].as_str().unwrap() {
        "embed-upstream" => json!({"object":"list","model":"embed-upstream","data":[
            {"object":"embedding","index":0,"embedding":[0.1,0.2]},
            {"object":"embedding","index":1,"embedding":[0.2,0.1]}],
            "usage":{"prompt_tokens":17,"total_tokens":17}}),
        "rank-upstream" => {
            json!({"model":"rank-upstream","results":[{"index":1,"relevance_score":0.9}],"usage":{"total_tokens":25}})
        }
        other => panic!("unexpected native input model {other}"),
    };
    match state.reply {
        Reply::Valid => (),
        Reply::MissingUsage => {
            response.as_object_mut().unwrap().remove("usage");
        }
        Reply::ExcessUsage => {
            response["usage"] = if body["model"] == "embed-upstream" {
                json!({"prompt_tokens":2001,"total_tokens":2001})
            } else {
                json!({"total_tokens":2001})
            }
        }
    }
    Json(response)
}
impl Supplier {
    async fn start(mode: Reply, paused: bool) -> Self {
        let state = SupplierState {
            hits: Arc::new(AtomicUsize::new(0)),
            seen: Arc::new(Mutex::new(Vec::new())),
            reply: mode,
            pause: paused.then(|| (Arc::new(Notify::new()), Arc::new(Notify::new()))),
        };
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let base = format!("http://{}/v1", listener.local_addr().unwrap());
        let app = HttpRouter::new()
            .route("/v1/embeddings", post(reply))
            .route("/v1/rerank", post(reply))
            .with_state(state.clone());
        let task = tokio::spawn(async move {
            axum::serve(listener, app).await.unwrap();
        });
        Self { base, state, task }
    }
}

async fn fixture(pool: PgPool, funds: i64) -> native_http::test_support::Fixture {
    native_http::test_support::fixture(pool, funds, &["embed-public", "rank-public"]).await
}

fn gateway(pool: &PgPool, supplier: &Supplier, price_version: u64) -> ModelGateway {
    let protocols = [Protocol::OpenAiEmbeddings, Protocol::JinaRerank];
    let upstreams: Vec<_> = protocols
        .iter()
        .enumerate()
        .map(|(i, protocol)| {
            json!({
                "id":i+1,"endpoint":"https://synthetic.invalid/v1","protocol":protocol,
                "credential":{"id":7,"revision":1},"enabled":true
            })
        })
        .collect();
    let route_rows: Vec<_> = [("embed-public","embed-upstream"),("rank-public","rank-upstream")]
        .iter().enumerate().map(|(i,(model,upstream))| json!({
            "id":i+1,"model":model,"group":"allowed","enabled":true,"speeds":["standard"],
            "targets":[{"id":i+1,"upstream_id":i+1,"upstream_model":upstream,"enabled":true,"priority":0,"weight":1,"speeds":["standard"],"payload":"adapt","timeout_ms":10000}],
            "failover":{"max_attempts":1,"retry_on":[],"allow_replay_after_send":false}
        })).collect();
    let mut routes: RoutingConfig = serde_json::from_value(json!({
        "version":price_version,"groups":[{"name":"allowed","enabled":true}],
        "upstreams":upstreams,"routes":route_rows,"aliases":[]
    }))
    .unwrap();
    for upstream in &mut routes.upstreams {
        upstream.endpoint = Endpoint::local_loopback(supplier.base.clone()).unwrap();
    }
    let rules: Vec<_> = ["embed-public","rank-public"].iter().enumerate().map(|(i,model)| json!({
        "id":i+1,"model":model,"group":"allowed","speed":"standard",
        "rates":{"input":2_000_000*price_version,"output":0,"cache_read":0,"cache_write":0,"request":0},
        "group_multiplier_ppm":1000000,"speed_multiplier_ppm":1000000
    })).collect();
    let prices: PriceConfig =
        serde_json::from_value(json!({"version":price_version,"rules":rules})).unwrap();
    ModelGateway::new(
        pool.clone(),
        Arc::new(
            crate::routing::Router::new(routes, Arc::new(PriceSnapshot::compile(prices).unwrap()))
                .unwrap(),
        ),
        vec![UpstreamCredential {
            reference: CredentialRef { id: 7, revision: 1 },
            secret: "input-only-synthetic-provider".into(),
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
fn body(kind: Kind) -> Value {
    match kind {
        Kind::Embeddings => json!({"model":"embed-public","input":["one","two"]}),
        Kind::Rerank => {
            json!({"model":"rank-public","query":"q","documents":["one","two"],"top_n":1})
        }
    }
}
fn http_request(secret: &str, key: &str, kind: Kind) -> HttpRequest<Body> {
    HttpRequest::builder()
        .method("POST")
        .uri(format!("/v1/{}", kind.name()))
        .header(header::AUTHORIZATION, format!("Bearer {secret}"))
        .header(header::CONTENT_TYPE, "application/json")
        .header("idempotency-key", key)
        .body(Body::from(body(kind).to_string()))
        .unwrap()
}
async fn call(
    gateway: &ModelGateway,
    request: HttpRequest<Body>,
) -> (StatusCode, HeaderMap, Value) {
    let response = gateway.router().oneshot(request).await.unwrap();
    let status = response.status();
    let headers = response.headers().clone();
    let output =
        serde_json::from_slice(&to_bytes(response.into_body(), 1024 * 1024).await.unwrap())
            .unwrap();
    (status, headers, output)
}

#[sqlx::test(migrations = false)]
async fn input_api_pg_native_key_restricted_runtime_settles_exact_usage_once(pool: PgPool) {
    let f = fixture(pool, 10000).await;
    let runtime = RuntimeRole::create(&f.pool).await;
    runtime.optional("core_charging").await;
    runtime.optional("core_model").await;
    let supplier = Supplier::start(Reply::Valid, false).await;
    let gateway = gateway(&runtime.pool, &supplier, 1);
    for (kind, tokens) in [(Kind::Embeddings, 17), (Kind::Rerank, 25)] {
        let (status, headers, output) =
            call(&gateway, http_request(&f.key, kind.name(), kind)).await;
        assert_eq!(status, StatusCode::OK, "{output}");
        assert_eq!(headers[header::CACHE_CONTROL], "no-store");
        let id = headers["x-request-id"].to_str().unwrap();
        let charge = gateway.bridge.charging.resolve(id).await.unwrap().unwrap();
        assert_eq!(charge.state, "settled");
        assert_eq!(charge.key_id, Some(f.key_id));
        assert_eq!(charge.usage.input, tokens);
        assert_eq!(charge.usage.output, 0);
        assert_eq!(charge.settled, tokens);
        assert_eq!(charge.reserved, 2000);
        let (_, contract) = gateway
            .bridge
            .charging
            .reservation_contract(id)
            .await
            .unwrap();
        assert_eq!(contract.bounds(), (2000, 0));
        assert_eq!(
            call(&gateway, http_request(&f.key, kind.name(), kind))
                .await
                .0,
            StatusCode::CONFLICT
        );
    }
    let balance = f.ledger.balance(f.account).await.unwrap();
    assert_eq!((balance.available_units, balance.reserved_units), (9958, 0));
    assert_eq!(supplier.state.hits.load(Ordering::SeqCst), 2);
    assert_eq!(
        supplier.state.seen.lock().unwrap()[0]["model"],
        "embed-upstream"
    );
    drop(gateway);
    runtime.cleanup().await;
}

#[sqlx::test(migrations = false)]
async fn input_api_pg_unknown_usage_keeps_original_hold_and_never_replays(pool: PgPool) {
    let f = fixture(pool, 10000).await;
    for (index, mode) in [Reply::MissingUsage, Reply::ExcessUsage]
        .into_iter()
        .enumerate()
    {
        let supplier = Supplier::start(mode, false).await;
        let gateway = gateway(&f.pool, &supplier, 1);
        let key = format!("unknown-{index}");
        let (status, headers, output) =
            call(&gateway, http_request(&f.key, &key, Kind::Embeddings)).await;
        assert!(!status.is_success(), "{output}");
        assert!(output.get("data").is_none());
        let charge = gateway
            .bridge
            .charging
            .resolve(headers["x-request-id"].to_str().unwrap())
            .await
            .unwrap()
            .unwrap();
        assert_eq!(charge.state, "reconcile");
        assert_eq!((charge.reserved, charge.settled), (2000, 0));
        assert_eq!(gateway.reconcile_completed_streams(10).await.unwrap(), 0);
        assert_eq!(
            call(&gateway, http_request(&f.key, &key, Kind::Embeddings))
                .await
                .0,
            StatusCode::CONFLICT
        );
        assert_eq!(supplier.state.hits.load(Ordering::SeqCst), 1);
    }
    let balance = f.ledger.balance(f.account).await.unwrap();
    assert_eq!(
        (balance.available_units, balance.reserved_units),
        (6000, 4000)
    );
}

#[sqlx::test(migrations = false)]
async fn input_api_pg_permissions_funds_and_concurrent_duplicate_admission(pool: PgPool) {
    let f = fixture(pool, 1000).await;
    let supplier = Supplier::start(Reply::Valid, false).await;
    let gateway = gateway(&f.pool, &supplier, 1);
    assert_eq!(
        call(
            &gateway,
            http_request("invalid", "bad-key", Kind::Embeddings)
        )
        .await
        .0,
        StatusCode::UNAUTHORIZED
    );
    assert_eq!(
        call(
            &gateway,
            http_request(&f.session, "session", Kind::Embeddings)
        )
        .await
        .0,
        StatusCode::FORBIDDEN
    );
    sqlx::query("UPDATE core_identity.api_key_settings SET model_limits=ARRAY['embed-public'] WHERE credential_id=$1").bind(f.key_id).execute(&f.pool).await.unwrap();
    assert_eq!(
        call(
            &gateway,
            http_request(&f.key, "forbidden-model", Kind::Rerank)
        )
        .await
        .0,
        StatusCode::FORBIDDEN
    );
    assert_eq!(
        call(&gateway, http_request(&f.key, "no-funds", Kind::Embeddings))
            .await
            .0,
        StatusCode::PAYMENT_REQUIRED
    );
    assert_eq!(supplier.state.hits.load(Ordering::SeqCst), 0);
    assert_eq!(
        sqlx::query_scalar::<_, i64>("SELECT count(*) FROM core_charging.requests")
            .fetch_one(&f.pool)
            .await
            .unwrap(),
        0
    );
    f.ledger
        .execute(&LedgerRequest {
            scope: "input-api-pg".into(),
            operation_key: "additional-test-funds".into(),
            actor_user_id: Some(7),
            reason: "Synthetic funds".into(),
            action: Action::Credit {
                account_id: f.account,
                amount_units: Credits::try_from(3000).unwrap(),
                payment_reference: "synthetic:additional-input-api".into(),
            },
        })
        .await
        .unwrap();
    let (a, b) = tokio::join!(
        call(
            &gateway,
            http_request(&f.key, "same-operation", Kind::Embeddings)
        ),
        call(
            &gateway,
            http_request(&f.key, "same-operation", Kind::Embeddings)
        )
    );
    assert_eq!(
        [a.0, b.0]
            .into_iter()
            .filter(|status| *status == StatusCode::OK)
            .count(),
        1
    );
    assert_eq!(
        [a.0, b.0]
            .into_iter()
            .filter(|status| *status == StatusCode::CONFLICT)
            .count(),
        1
    );
    assert_eq!(supplier.state.hits.load(Ordering::SeqCst), 1);
    let balance = f.ledger.balance(f.account).await.unwrap();
    assert_eq!((balance.available_units, balance.reserved_units), (3983, 0));
}

#[sqlx::test(migrations = false)]
async fn input_api_pg_durable_receipt_recovers_with_original_price_after_settlement_failure(
    pool: PgPool,
) {
    let f = fixture(pool, 10000).await;
    let runtime = RuntimeRole::create(&f.pool).await;
    runtime.optional("core_charging").await;
    runtime.optional("core_model").await;
    let supplier = Supplier::start(Reply::Valid, true).await;
    let initial = gateway(&runtime.pool, &supplier, 1);
    let running = initial.clone();
    let request = http_request(&f.key, "recover-input", Kind::Embeddings);
    let task = tokio::spawn(async move { call(&running, request).await });
    let (entered, resume) = supplier.state.pause.as_ref().unwrap();
    tokio::time::timeout(Duration::from_secs(3), entered.notified())
        .await
        .unwrap();
    // Only this disposable runtime loses the final ledger capability. Reservation and
    // actual dispatch already happened; provider checkpoint writes remain permitted.
    sqlx::raw_sql(&format!(
        "REVOKE EXECUTE ON FUNCTION core_billing.post_ledger(jsonb) FROM {}",
        runtime.name
    ))
    .execute(&f.pool)
    .await
    .unwrap();
    resume.notify_one();
    let (status, headers, output) = task.await.unwrap();
    assert!(!status.is_success(), "{output}");
    let id = headers["x-request-id"].to_str().unwrap();
    let pending = initial.bridge.charging.resolve(id).await.unwrap().unwrap();
    assert_eq!(pending.state, "reconcile");
    assert_eq!(pending.price.input_per_million, 1_000_000);
    assert_eq!(supplier.state.hits.load(Ordering::SeqCst), 1);
    drop(initial);
    sqlx::raw_sql(&format!(
        "GRANT EXECUTE ON FUNCTION core_billing.post_ledger(jsonb) TO {}",
        runtime.name
    ))
    .execute(&f.pool)
    .await
    .unwrap();
    let restarted = gateway(&runtime.pool, &supplier, 3);
    assert_eq!(restarted.reconcile_completed_streams(10).await.unwrap(), 1);
    assert_eq!(restarted.reconcile_completed_streams(10).await.unwrap(), 0);
    let charge = restarted
        .bridge
        .charging
        .resolve(id)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(charge.settled, 17);
    assert_eq!(charge.price.input_per_million, 1_000_000);
    assert_eq!(supplier.state.hits.load(Ordering::SeqCst), 1);
    let balance = f.ledger.balance(f.account).await.unwrap();
    assert_eq!((balance.available_units, balance.reserved_units), (9983, 0));
    drop(restarted);
    runtime.cleanup().await;
}

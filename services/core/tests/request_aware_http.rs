//! Real TCP requests through the production ModelGateway router and a restricted
//! PostgreSQL runtime role. Supplier replies and funds are synthetic.
//! The HTTPS candidate retains its external output contract; numeric-loopback
//! candidates use the existing local transport policy. This mixed constructor
//! fixture does not assert that the production TOML accepts mixed endpoint modes.
use lmm_core::{
    billing::ledger::Ledger,
    model_http::{
        ChargePolicy, ExternalModelContext, ModelGateway, ProviderPolicy, UpstreamCredential,
    },
    relay::Limits,
    routing::{
        self, BillingMode, CredentialRef, Endpoint, MULTIPLIER_ONE, PriceConfig, PriceRule,
        PriceSnapshot, Protocol, RoutingConfig, Speed, TokenRates,
    },
};
use serde_json::{Value, json};
use sqlx::PgPool;
use std::{
    collections::BTreeMap,
    sync::{Arc, atomic::Ordering},
    time::Duration,
};
use tokio::{net::TcpListener, task::JoinHandle};

#[path = "common/model_fixture.rs"]
mod model_fixture;
#[path = "common/runtime.rs"]
mod runtime;

const EXTERNAL: &str = "https://request-aware.invalid/v1";
const INPUT_BOUND: i64 = 1_048_576;
const OUTPUT_BOUND: u32 = 4096;
const INITIAL_FUNDS: i64 = 2_000_000;

fn gateway(pool: PgPool, key_id: i64, attempts: u8, endpoints: [&str; 3]) -> ModelGateway {
    let mut config: RoutingConfig = serde_json::from_value(json!({
        "version":1,
        "groups":[{"name":"allowed","enabled":true}],
        "upstreams": (1..=3).map(|id| json!({
            "id":id,"endpoint":EXTERNAL,"protocol":"openai_chat",
            "credential":{"id":7,"revision":1},"enabled":true
        })).collect::<Vec<_>>(),
        "routes":[{
            "id":1,"model":"local-model","group":"allowed","enabled":true,"speeds":["standard"],
            "targets": (1..=3).map(|id| json!({
                "id":id,"upstream_id":id,"upstream_model":"provider-model","enabled":true,
                "priority":id-1,"weight":1,"speeds":["standard"],"payload":"adapt","timeout_ms":5000
            })).collect::<Vec<_>>(),
            "failover":{"max_attempts":attempts,"retry_on":["connect"],"allow_replay_after_send":false}
        }],
        "aliases":[]
    })).unwrap();
    let mut contracts = Vec::new();
    for (index, endpoint) in endpoints.into_iter().enumerate() {
        if endpoint.starts_with("https:") {
            config.upstreams[index].endpoint = endpoint.to_owned().try_into().unwrap();
            contracts.push(ExternalModelContext {
                metadata: None,
                responses_compaction: false,
                responses_continuity: None,
                pre_acceptance_rejection_statuses: Vec::new(),
                grounding: None,
                reservation: Default::default(),
                missing_cache_creation_tokens_is_zero: false,
                upstream_id: index as u64 + 1,
                upstream_model: "provider-model".into(),
                protocol: Protocol::OpenAiChat,
                endpoint: endpoint.into(),
                max_input_tokens: 65_536,
                max_output_tokens: 128,
                contract_revision: "synthetic-request-aware-tcp-v1".into(),
                missing_cached_tokens_is_zero: false,
            });
        } else {
            config.upstreams[index].endpoint = Endpoint::local_loopback(endpoint.into()).unwrap();
        }
    }
    let prices = PriceSnapshot::compile(PriceConfig {
        version: 1,
        rules: vec![PriceRule {
            rounding: None,
            billing_mode: BillingMode::Tokens,
            tiered: None,
            id: 1,
            model: "local-model".into(),
            group: "allowed".into(),
            speed: Speed::Standard,
            rates: TokenRates {
                input: 2_000_000,
                output: 0,
                cache_read: 2_000_000,
                cache_write: 0,
                request: 0,
            },
            group_multiplier_ppm: MULTIPLIER_ONE,
            speed_multiplier_ppm: MULTIPLIER_ONE,
        }],
    })
    .unwrap();
    ModelGateway::new_with_provider_policy(
        pool,
        Arc::new(routing::Router::new(config, Arc::new(prices)).unwrap()),
        vec![UpstreamCredential {
            reference: CredentialRef { id: 7, revision: 1 },
            secret: "synthetic-request-aware-provider-key".into(),
        }],
        BTreeMap::from([(key_id, "allowed".into())]),
        Limits {
            max_output_tokens: OUTPUT_BOUND,
            connect_timeout: Duration::from_millis(500),
            header_timeout: Duration::from_secs(2),
            read_timeout: Duration::from_secs(2),
            write_timeout: Duration::from_secs(2),
            total_timeout: Duration::from_secs(5),
            hook_timeout: Duration::from_secs(2),
            ..Limits::default()
        },
        ChargePolicy {
            input_token_ceiling: INPUT_BOUND,
            lease_seconds: 30,
        },
        ProviderPolicy::new(contracts, INPUT_BOUND).unwrap(),
    )
    .unwrap()
}

async fn serve(gateway: ModelGateway) -> (String, JoinHandle<()>) {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let endpoint = format!(
        "http://{}/v1/chat/completions",
        listener.local_addr().unwrap()
    );
    let app = gateway.router();
    let task = tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
    (endpoint, task)
}

async fn request(endpoint: &str, key: &str, idempotency: &str) -> (u16, Value, String) {
    let response = reqwest::Client::builder()
        .no_proxy()
        .timeout(Duration::from_secs(10))
        .build()
        .unwrap()
        .post(endpoint)
        .bearer_auth(key)
        .header("idempotency-key", idempotency)
        .json(&json!({
            "model":"local-model",
            "messages":[{"role":"user","content":"hello"}],
            "max_tokens":129,
            "stream":false
        }))
        .send()
        .await
        .unwrap();
    let status = response.status().as_u16();
    let id = response
        .headers()
        .get("x-request-id")
        .map(|value| value.to_str().unwrap().to_owned())
        .unwrap_or_default();
    let body = response.json().await.unwrap();
    (status, body, id)
}

async fn grant_group(pool: &PgPool) {
    sqlx::query("INSERT INTO core_identity.user_routing_groups(user_id,group_name,enabled,source_reference) VALUES(7,'allowed',TRUE,'synthetic-request-aware-tcp')")
        .execute(pool).await.unwrap();
}

async fn assert_balanced(pool: &PgPool, ledger: &Ledger, account: i64, settled: i64) -> Value {
    let balance = ledger.balance(account).await.unwrap();
    assert_eq!(
        (balance.available_units, balance.reserved_units),
        (INITIAL_FUNDS - settled, 0)
    );
    let unbalanced: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM (SELECT journal_id FROM core_billing.ledger_entries GROUP BY journal_id HAVING sum(delta_units)<>0) entries"
    ).fetch_one(pool).await.unwrap();
    assert_eq!(unbalanced, 0);
    json!({"available":balance.available_units,"reserved":balance.reserved_units,"unbalanced_journals":unbalanced})
}

async fn compatible_case(pool: PgPool, connect_failure: bool) {
    let (_, ledger, _, key, key_id, account) = model_fixture::fixture(&pool, INITIAL_FUNDS).await;
    grant_group(&pool).await;
    let runtime = runtime::RuntimeRole::create(&pool).await;
    runtime.optional("core_charging").await;
    runtime.optional("core_model").await;
    let b = if connect_failure {
        None
    } else {
        Some(model_fixture::Provider::start(true).await)
    };
    let c = model_fixture::Provider::start(true).await;
    let b_endpoint = if let Some(provider) = &b {
        provider.endpoint.clone()
    } else {
        let refused = TcpListener::bind("127.0.0.1:0").await.unwrap();
        format!("http://{}/custom/v1", refused.local_addr().unwrap())
    };
    let attempts = if connect_failure { 2 } else { 1 };
    let (endpoint, server) = serve(gateway(
        runtime.pool.clone(),
        key_id,
        attempts,
        [EXTERNAL, &b_endpoint, &c.endpoint],
    ))
    .await;
    let (status, body, id) = request(&endpoint, &key, "request-aware-tcp").await;
    assert_eq!(status, 200, "{body}");
    assert_eq!(
        body["choices"][0]["message"]["content"],
        "local synthetic answer"
    );
    assert!(!id.is_empty());
    tokio::time::timeout(Duration::from_secs(5), async {
        loop {
            let state: Option<String> =
                sqlx::query_scalar("SELECT state FROM core_charging.requests WHERE id=$1")
                    .bind(&id)
                    .fetch_optional(&pool)
                    .await
                    .unwrap();
            if state.as_deref() == Some("settled") {
                break;
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .unwrap();
    let charge: (String, i64, i64, i64, i64) = sqlx::query_as(
        "SELECT state,reserved,settled,input_tokens,output_tokens FROM core_charging.requests WHERE id=$1"
    ).bind(&id).fetch_one(&pool).await.unwrap();
    assert_eq!(charge, ("settled".into(), INPUT_BOUND, 10, 10, 20));
    let dispatches: Vec<Value> = sqlx::query_scalar(
        "SELECT detail FROM core_charging.usage_events WHERE request_id=$1 AND event_id LIKE 'route-dispatch:%' ORDER BY event_id"
    ).bind(&id).fetch_all(&pool).await.unwrap();
    assert_eq!(dispatches.len(), usize::from(attempts));
    assert_eq!(dispatches[0]["number"], 1);
    assert_eq!(dispatches[0]["route_id"], "r1-t2-u2");
    if connect_failure {
        assert_eq!(dispatches[1]["number"], 2);
        assert_eq!(dispatches[1]["route_id"], "r1-t3-u3");
        let events: Vec<Value> =
            sqlx::query_scalar("SELECT detail FROM core_charging.usage_events WHERE request_id=$1")
                .bind(&id)
                .fetch_all(&pool)
                .await
                .unwrap();
        assert!(
            events
                .iter()
                .any(|event| event["failure"]["failure"]["delivery"] == "not_sent")
        );
    }
    let b_hits = b
        .as_ref()
        .map(|provider| provider.hits.load(Ordering::SeqCst));
    let c_hits = c.hits.load(Ordering::SeqCst);
    assert_eq!(b_hits, if connect_failure { None } else { Some(1) });
    assert_eq!(c_hits, usize::from(connect_failure));
    let duplicate_status = if connect_failure {
        None
    } else {
        let duplicate = request(&endpoint, &key, "request-aware-tcp").await;
        assert_eq!(duplicate.0, 409, "{}", duplicate.1);
        assert_eq!(b.as_ref().unwrap().hits.load(Ordering::SeqCst), 1);
        assert_eq!(c.hits.load(Ordering::SeqCst), 0);
        Some(duplicate.0)
    };
    let requests: i64 = sqlx::query_scalar("SELECT count(*) FROM core_charging.requests")
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(requests, 1);
    let hold: (i64, i64, i64, bool) = sqlx::query_as(
        "SELECT count(*),min(reserved),min(captured),bool_and(closed) FROM core_charging.ledger_holds WHERE request_id=$1"
    ).bind(&id).fetch_one(&pool).await.unwrap();
    assert_eq!(hold, (1, INPUT_BOUND, 10, true));
    let mut journals = BTreeMap::new();
    for kind in ["reserve", "capture"] {
        let count: i64 = sqlx::query_scalar(
            "SELECT count(*) FROM core_billing.ledger_journals WHERE scope='core-charging' AND kind=$1 AND reason=$2"
        ).bind(kind).bind(format!("Model request {id}")).fetch_one(&pool).await.unwrap();
        assert_eq!(count, 1, "{kind}");
        journals.insert(kind, count);
    }
    let balance = assert_balanced(&pool, &ledger, account, charge.2).await;
    println!(
        "LMM_REQUEST_AWARE_TCP {}",
        json!({
            "case": if connect_failure { "connect_failure_then_fallback" } else { "skip_incompatible_with_one_attempt" },
            "gateway_transport":"real_tcp_http",
            "database_role":"restricted_runtime",
            "max_attempts":attempts,
            "requested_max_output":129,
            "http_status":status,
            "request_id":id,
            "dispatches":dispatches,
            "b_mode":if connect_failure { "refused_tcp_port" } else { "live_http_supplier" },
            "b_http_hits":b_hits,
            "c_http_hits":c_hits,
            "duplicate_http_status":duplicate_status,
            "charging_requests":requests,
            "charge":{"state":charge.0,"reserved":charge.1,"settled":charge.2,"input_tokens":charge.3,"output_tokens":charge.4},
            "hold":{"count":hold.0,"reserved":hold.1,"captured":hold.2,"closed":hold.3},
            "journals":journals,
            "balance":balance
        })
    );
    server.abort();
    let _ = server.await;
    runtime.cleanup().await;
}

#[sqlx::test(migrations = false)]
async fn request_aware_tcp_single_attempt_selects_compatible(pool: PgPool) {
    compatible_case(pool, false).await;
}

#[sqlx::test(migrations = false)]
async fn request_aware_tcp_connect_failure_keeps_one_hold(pool: PgPool) {
    compatible_case(pool, true).await;
}

#[sqlx::test(migrations = false)]
async fn request_aware_tcp_no_compatible_route_has_no_hold(pool: PgPool) {
    let (_, ledger, _, key, key_id, account) = model_fixture::fixture(&pool, INITIAL_FUNDS).await;
    grant_group(&pool).await;
    let runtime = runtime::RuntimeRole::create(&pool).await;
    runtime.optional("core_charging").await;
    runtime.optional("core_model").await;
    let (endpoint, server) = serve(gateway(runtime.pool.clone(), key_id, 2, [EXTERNAL; 3])).await;
    let (status, body, _) = request(&endpoint, &key, "request-aware-tcp-no-route").await;
    assert_eq!(status, 503, "{body}");
    assert_eq!(body["error"]["code"], "route_unavailable");
    let counts: (i64, i64, i64, i64) = sqlx::query_as(
        "SELECT (SELECT count(*) FROM core_charging.requests),(SELECT count(*) FROM core_charging.ledger_holds),(SELECT count(*) FROM core_charging.usage_events WHERE event_id LIKE 'route-dispatch:%'),(SELECT count(*) FROM core_billing.ledger_journals WHERE scope='core-charging')"
    ).fetch_one(&pool).await.unwrap();
    assert_eq!(counts, (0, 0, 0, 0));
    let balance = assert_balanced(&pool, &ledger, account, 0).await;
    println!(
        "LMM_REQUEST_AWARE_TCP {}",
        json!({
            "case":"all_candidates_incompatible","gateway_transport":"real_tcp_http",
            "database_role":"restricted_runtime","requested_max_output":129,
            "http_status":status,"error":body["error"]["code"],
            "charging_requests":counts.0,"holds":counts.1,"dispatches":counts.2,
            "charging_journals":counts.3,"balance":balance
        })
    );
    server.abort();
    let _ = server.await;
    runtime.cleanup().await;
}

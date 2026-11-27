//! Official scalar publication and historical wallet evidence for one Responses model.
use super::*;
use axum::{Json, Router as HttpRouter, routing::post};
use lmm_core::{
    channel_http::{ManagedChannels, managed_router},
    identity::IdentityStore,
    model_http::ProviderPolicy,
    routing::store::ReleaseDocument,
    usage::{UsageStore, router as usage_router},
};
use std::sync::Mutex;
use tokio::net::TcpListener;

#[allow(dead_code)]
#[path = "../common/channel_control.rs"]
mod admin_support;

const CONFIG: &str = "/core/v1/admin/routing/configuration";

fn service(pool: &PgPool, release: ReleaseDocument, key_id: i64) -> ModelGateway {
    let upstream = &release.routing.upstreams[0];
    let contexts = if upstream.endpoint.as_str().starts_with("https://") {
        vec![serde_json::from_value(json!({
            "upstream_id":1,"upstream_model":"model-a","protocol":"openai_responses",
            "endpoint":upstream.endpoint.as_str(),"max_input_tokens":256,"max_output_tokens":32,
            "contract_revision":"official-wallet-v1"
        })).unwrap()]
    } else {
        Vec::new()
    };
    let credential = UpstreamCredential {
        reference: upstream.credential,
        secret: "synthetic-wallet-key".into(),
    };
    let prices = PriceSnapshot::compile(release.pricing).unwrap();
    let router = routing::Router::new(release.routing, Arc::new(prices)).unwrap();
    ModelGateway::new_with_provider_policy(
        pool.clone(),
        Arc::new(router),
        vec![credential],
        BTreeMap::from([(key_id, "default".to_owned())]),
        Limits {
            total_timeout: Duration::from_secs(3),
            hook_timeout: Duration::from_secs(2),
            max_output_tokens: 32,
            ..Limits::default()
        },
        ChargePolicy {
            input_token_ceiling: 256,
            lease_seconds: 30,
        },
        ProviderPolicy::new(contexts, 256).unwrap(),
    )
    .unwrap()
}

async fn publish(app: &HttpRouter, admin: &str, pool: &PgPool, output_rate: u64) -> ReleaseDocument {
    let before = admin_support::success(app, "GET", CONFIG, admin, Value::Null).await;
    let mut proposal = json!({
        "expected_version":before["version"],"routing":before["routing"],
        "prices":before["prices"],"bindings":before["bindings"]
    });
    proposal["routing"]["version"] = json!(before["version"]["routing"].as_u64().unwrap() + 1);
    proposal["prices"]["version"] = json!(before["version"]["pricing"].as_u64().unwrap() + 1);
    let speeds = json!(["standard", "fast", "ultrafast"]);
    proposal["routing"]["routes"][0]["speeds"] = speeds.clone();
    proposal["routing"]["routes"][0]["targets"][0]["speeds"] = speeds;
    proposal["bindings"]["official_prices"] = json!({
        "models":{"model-a":{"rates":{
            "input":1_000_000,"output":output_rate,"cache_read":200_000,"cache_write":0,"request":0
        },"billing_mode":"tokens","source":"fixture","source_unit":"micro-USD per million tokens"}},
        "group_multipliers_ppm":{"default":1_250_000},
        "speed_multipliers_ppm":{"fast":1_600_000,"ultrafast":2_400_000}
    });
    let preview_path = format!("{CONFIG}/preview");
    let preview = admin_support::success(app, "POST", &preview_path, admin, proposal.clone()).await;
    assert_eq!(preview["confirmation_digest"].as_str().unwrap().len(), 64);
    proposal["confirmation_digest"] = preview["confirmation_digest"].clone();
    admin_support::success(app, "PUT", CONFIG, admin, proposal).await;
    let release = admin_support::active(pool).await;
    assert_eq!(
        serde_json::to_value(&release.pricing).unwrap(),
        preview["prices"]
    );
    release
}

async fn supplier(
    axum::extract::State(grades): axum::extract::State<Arc<Mutex<Vec<Option<Value>>>>>,
    Json(body): Json<Value>,
) -> Json<Value> {
    assert_eq!(body["model"], "model-a");
    assert!(body.get("speed").is_none());
    assert!(body.get("serviceTier").is_none());
    let wire_tier = body.get("service_tier");
    grades.lock().unwrap().push(wire_tier.cloned());
    let reported_tier = match wire_tier {
        None => "default",
        Some(value) => value
            .as_str()
            .expect("Responses service_tier must be a string"),
    };
    Json(json!({
        "id":"wallet-response","status":"completed","service_tier":reported_tier,
        "output":[{"id":"wallet-message","type":"message","role":"assistant",
            "content":[{"type":"output_text","text":"ok","annotations":[]}]}],
        "usage":{"input_tokens":110,"output_tokens":10,"total_tokens":120,
            "input_tokens_details":{"cached_tokens":25,"cache_write_tokens":10,
                "cache_write_tokens_details":{"ttl_30m_tokens":4,"ttl_12h_tokens":6}}}
    }))
}

async fn charge(gateway: &ModelGateway, key: &str, model: &str, operation: &str) -> String {
    let request = Request::builder()
        .method("POST")
        .uri("/v1/responses")
        .header("authorization", format!("Bearer {key}"))
        .header("content-type", "application/json")
        .header("idempotency-key", operation)
        .body(Body::from(
            json!({"model":model,"input":"hi","max_output_tokens":16}).to_string(),
        ))
        .unwrap();
    let response = gateway.router().oneshot(request).await.unwrap();
    let (parts, body) = response.into_parts();
    let body = to_bytes(body, 1 << 20).await.unwrap();
    assert_eq!(
        parts.status,
        StatusCode::OK,
        "{}",
        String::from_utf8_lossy(&body)
    );
    parts.headers["x-request-id"].to_str().unwrap().to_owned()
}

async fn history(pool: &PgPool, session: &str, id: &str, expected: i64) -> Value {
    let url = format!("/api/log/self?request_id={id}");
    let app = usage_router(UsageStore::new(pool.clone()));
    let (status, page) = admin_support::request(app, "GET", &url, session, Value::Null).await;
    assert_eq!(status, StatusCode::OK, "{page}");
    assert_eq!(page["data"]["total"], 1);
    let row = &page["data"]["items"][0];
    assert_eq!(row["request_id"], id);
    assert_eq!(row["gross_settled_credits"], expected);
    assert_eq!(row["refunded_credits"], 0);
    assert_eq!(row["net_consumption_credits"], expected);
    let records: Vec<Value> = sqlx::query_scalar(
        r#"SELECT jsonb_build_object('request', to_jsonb(r), 'hold', to_jsonb(h),
                  'reserve', to_jsonb(res), 'capture', to_jsonb(cap),
                  'entries', (SELECT jsonb_agg(to_jsonb(e) ORDER BY e.journal_id, e.ledger_account_id)
                              FROM core_billing.ledger_entries e WHERE e.journal_id IN (res.id,cap.id)))
           FROM core_charging.requests r JOIN core_charging.ledger_holds h ON h.request_id = r.id
           JOIN core_billing.ledger_journals res ON res.id = h.reservation_journal_id
           LEFT JOIN core_billing.ledger_journals cap ON cap.id = h.capture_journal_id
           WHERE r.id = $1"#,
    )
    .bind(id)
    .fetch_all(pool)
    .await
    .unwrap();
    assert_eq!(records.len(), 1);
    let record = &records[0];
    assert_eq!(record["hold"]["closed"], true);
    assert_eq!(record["hold"]["captured"], expected);
    assert_eq!(record["capture"]["kind"], "capture");
    assert_eq!(record["capture"]["amount_units"], expected);
    assert_eq!(
        record["capture"]["parent_journal_id"],
        record["reserve"]["id"]
    );
    let entries = record["entries"].as_array().unwrap();
    for journal in ["reserve", "capture"] {
        let deltas: Vec<_> = entries
            .iter()
            .filter(|entry| entry["journal_id"] == record[journal]["id"])
            .map(|entry| entry["delta_units"].as_i64().unwrap())
            .collect();
        assert!(deltas.len() >= 2);
        assert_eq!(deltas.iter().sum::<i64>(), 0);
    }
    json!({"user_bill":row,"ledger":record})
}

#[sqlx::test(migrations = false)]
async fn published_official_tiers_settle_free_writes_and_preserve_wallet_history(pool: PgPool) {
    let (_, ledger, session, key, key_id, account) = fixture(&pool, 2_000).await;
    let admin = IdentityStore::from_pool(pool.clone())
        .bootstrap_user(8, 6)
        .await
        .unwrap();
    let mut seed = admin_support::fixture();
    seed.routing.upstreams[0].protocol = Protocol::OpenAiResponses;
    let management = ManagedChannels::recover(pool.clone(), service(&pool, seed, key_id))
        .await
        .unwrap();
    let app = managed_router(pool.clone(), management);
    let grades = Arc::new(Mutex::new(Vec::new()));
    let upstream = HttpRouter::new()
        .route("/v1/responses", post(supplier))
        .with_state(grades.clone());
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let url = format!("http://{}/v1", listener.local_addr().unwrap());
    let endpoint = Endpoint::local_loopback(url).unwrap();
    let server = tokio::spawn(async move { axum::serve(listener, upstream).await.unwrap() });

    let mut release = publish(&app, &admin.secret, &pool, 2_000_000).await;
    release.routing.upstreams[0].endpoint = endpoint.clone();
    let gateway = service(&pool, release, key_id);
    let charging = Charging::new(pool.clone(), Arc::new(TransactionalLedger));
    let mut bills = Vec::new();
    let mut available = 2_000;
    // Of 110 input tokens, 25 are cached and 10 are free writes. The base is
    // (75 * 1 + 25 * 0.2 + 10 * 0 + 10 output * 2) = 100 micro-USD.
    for (model, multiplier, expected) in [
        ("model-a", 1_000_000, 63_i64),
        ("model-a/fast", 1_600_000, 100),
        ("model-a/ultrafast", 2_400_000, 150),
    ] {
        let operation = format!("official-wallet-{multiplier}");
        let id = charge(&gateway, &key, model, &operation).await;
        let receipt = charging.resolve(&id).await.unwrap().unwrap();
        assert_eq!(receipt.state, "settled");
        assert_eq!(receipt.settled, expected);
        assert_eq!(receipt.price.model, "model-a");
        assert_eq!(receipt.price.group, "default");
        let usage = receipt.usage;
        assert_eq!((usage.input, usage.cached, usage.output), (110, 25, 10));
        let writes = receipt.usage.cache_creation.unwrap();
        assert_eq!((writes.total, writes.twelve_hour), (10, Some(6)));
        let frozen = receipt.price.tiered.as_ref().unwrap();
        assert_eq!(frozen.group_multiplier_ppm, 1_250_000);
        assert_eq!(frozen.speed_multiplier_ppm, multiplier);
        assert_eq!(frozen.credits_per_usd, 500_000);
        assert_eq!(receipt.price.cost(receipt.usage).unwrap(), expected);
        available -= expected;
        let balance = ledger.balance(account).await.unwrap();
        assert_eq!(
            (balance.available_units, balance.reserved_units),
            (available, 0)
        );
        let before = history(&pool, &session, &id, expected).await;
        bills.push((id, expected, before));
    }

    let mut repriced = publish(&app, &admin.secret, &pool, 4_000_000).await;
    repriced.routing.upstreams[0].endpoint = endpoint;
    let changed_gateway = service(&pool, repriced, key_id);
    let new_id = charge(&changed_gateway, &key, "model-a", "official-wallet-new").await;
    // The extra 20 micro-USD raises the new standard capture to 75 credits.
    history(&pool, &session, &new_id, 75).await;
    for (id, amount, before) in bills {
        assert_ne!(id, new_id);
        assert_eq!(history(&pool, &session, &id, amount).await, before);
    }
    let balance = ledger.balance(account).await.unwrap();
    assert_eq!(
        (balance.available_units, balance.reserved_units),
        (1_612, 0)
    );
    assert_eq!(
        grades.lock().unwrap().as_slice(),
        [None, Some(json!("priority")), Some(json!("ultrafast")), None]
    );
    server.abort();
}

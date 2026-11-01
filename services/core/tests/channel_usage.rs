//! Real Native receipts, disposable PG and the restricted runtime HTTP reader.
//! Owner-only corruptions below are synthetic failure evidence, never migrations.
use axum::{
    Router,
    body::{Body, to_bytes},
    http::{Request, StatusCode},
};
use lmm_core::{
    billing::{
        charging::{Charging, Outcome, Price, Request as ChargeRequest, Usage},
        transactional_ledger::TransactionalLedger,
    },
    channel_http,
    model_http::{ChargePolicy, ModelGateway, UpstreamCredential},
    relay::Limits,
    routing::{CredentialRef, Endpoint, PriceConfig, PriceSnapshot, RoutingConfig},
};
use serde_json::{Value, json};
use sqlx::PgPool;
use std::{collections::BTreeMap, sync::Arc, time::Duration};
use tokio::net::TcpListener;
use tower::ServiceExt;

#[path = "common/model_fixture.rs"]
mod model_fixture;
#[path = "common/reservation_contract.rs"]
mod reservation_contract;
#[path = "common/runtime.rs"]
mod runtime;

const WORKER: [u8; 32] = [39; 32];

struct Fixture {
    owner: PgPool,
    role: runtime::RuntimeRole,
    app: Router,
    gateway: ModelGateway,
    charging: Charging,
    key: String,
    key_id: i64,
    admin: String,
    account: i64,
    start: i64,
}

impl Fixture {
    async fn new(owner: PgPool, endpoint: &str, funds: i64, zero_price: bool) -> Self {
        let (identity, _, _, key, key_id, account) = model_fixture::fixture(&owner, funds).await;
        let admin = identity.bootstrap_user(8, 5).await.unwrap().secret;
        sqlx::query("INSERT INTO core_identity.user_routing_groups(user_id,group_name,source_reference) VALUES(7,'allowed','channel-usage-test')")
            .execute(&owner).await.unwrap();
        sqlx::query("INSERT INTO core_identity.api_key_settings(credential_id,name,group_name) VALUES($1,'usage key','allowed')")
            .bind(key_id).execute(&owner).await.unwrap();
        let start: i64 =
            sqlx::query_scalar("SELECT floor(extract(epoch FROM clock_timestamp()))::bigint")
                .fetch_one(&owner)
                .await
                .unwrap();
        let role = runtime::RuntimeRole::create(&owner).await;
        role.optional("core_charging").await;
        role.optional("core_model").await;
        // An immediately refused TCP connection proves no supplier acceptance;
        // priority makes the actual settlement use channel 2's fallback binding.
        let refused = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let first = format!("http://{}/v1", refused.local_addr().unwrap());
        drop(refused);
        let mut config: RoutingConfig = serde_json::from_value(json!({
            "version":1,"groups":[{"name":"allowed","enabled":true}],
            "upstreams":[
                {"id":1,"endpoint":"https://synthetic.invalid/first","protocol":"openai_chat","credential":{"id":7,"revision":1},"enabled":true},
                {"id":2,"endpoint":"https://synthetic.invalid/second","protocol":"openai_chat","credential":{"id":7,"revision":1},"enabled":true}],
            "routes":[{"id":1,"model":"local-model","group":"allowed","enabled":true,"speeds":["standard"],
                "targets":[
                    {"id":1,"upstream_id":1,"upstream_model":"provider-model","enabled":true,"priority":0,"weight":1,"speeds":["standard"],"payload":"adapt","timeout_ms":5000},
                    {"id":2,"upstream_id":2,"upstream_model":"provider-model","enabled":true,"priority":1,"weight":1,"speeds":["standard"],"payload":"adapt","timeout_ms":5000}],
                "failover":{"max_attempts":2,"retry_on":["connect"],"allow_replay_after_send":false}}],"aliases":[]
        })).unwrap();
        config.upstreams[0].endpoint = Endpoint::local_loopback(first).unwrap();
        config.upstreams[1].endpoint = Endpoint::local_loopback(endpoint.into()).unwrap();
        let rate = u64::from(!zero_price);
        let prices: PriceConfig = serde_json::from_value(json!({"version":1,"rules":[{
            "id":1,"model":"local-model","group":"allowed","speed":"standard",
            "rates":{"input":2_000_000*rate,"output":4_000_000*rate,"cache_read":1_000_000*rate,"cache_write":0,"request":0},
            "group_multiplier_ppm":1000000,"speed_multiplier_ppm":1000000}]})).unwrap();
        let routing = Arc::new(
            lmm_core::routing::Router::new(
                config,
                Arc::new(PriceSnapshot::compile(prices).unwrap()),
            )
            .unwrap(),
        );
        let gateway = ModelGateway::new(
            role.pool.clone(),
            routing.clone(),
            vec![UpstreamCredential {
                reference: CredentialRef { id: 7, revision: 1 },
                secret: "synthetic-channel-usage-key".into(),
            }],
            BTreeMap::from([(key_id, "allowed".into())]),
            Limits {
                max_output_tokens: 64,
                total_timeout: Duration::from_secs(3),
                ..Limits::default()
            },
            ChargePolicy {
                input_token_ceiling: 1000,
                lease_seconds: 60,
            },
        )
        .unwrap()
        .with_local_streaming();
        let management = channel_http::ManagedChannels::recover(role.pool.clone(), gateway.clone())
            .await
            .unwrap();
        let app = channel_http::managed_router(role.pool.clone(), management);
        let charging = Charging::new(role.pool.clone(), Arc::new(TransactionalLedger));
        Self {
            owner,
            role,
            app,
            gateway,
            charging,
            key,
            key_id,
            admin,
            account,
            start,
        }
    }

    async fn call(&self, id: &str, stream: bool) -> (StatusCode, String) {
        let (status, id, _) = self.call_response(id, stream).await;
        (status, id)
    }

    async fn call_response(&self, id: &str, stream: bool) -> (StatusCode, String, String) {
        let response = self.gateway.router().oneshot(Request::builder()
            .method("POST").uri("/v1/chat/completions")
            .header("authorization",format!("Bearer {}",self.key))
            .header("content-type","application/json").header("idempotency-key",id)
            .body(Body::from(json!({"model":"local-model","messages":[{"role":"user","content":"hello"}],"max_tokens":32,"stream":stream}).to_string()))
            .unwrap()).await.unwrap();
        let status = response.status();
        let id = response.headers()["x-request-id"]
            .to_str()
            .unwrap()
            .to_owned();
        let body = to_bytes(response.into_body(), 1 << 20).await.unwrap();
        if !stream && status == StatusCode::OK {
            assert_eq!(
                serde_json::from_slice::<Value>(&body).unwrap()["choices"][0]["message"]["content"],
                "local synthetic answer"
            );
        }
        if status == StatusCode::OK {
            // HTTP EOF precedes the gateway's background performance receipt.
            // Its SERIALIZABLE read of requests and write of usage_events can
            // conflict with refund's inverse read/write order. Finish that
            // known fixture task before starting a single refund transaction.
            tokio::time::timeout(Duration::from_secs(3), async {
                loop {
                    let recorded: bool = sqlx::query_scalar(
                        "SELECT EXISTS(SELECT FROM core_charging.usage_events \
                         WHERE request_id=$1 AND event_id='performance-v1')",
                    )
                    .bind(&id)
                    .fetch_one(&self.owner)
                    .await
                    .unwrap();
                    if recorded {
                        break;
                    }
                    tokio::time::sleep(Duration::from_millis(5)).await;
                }
            })
            .await
            .expect("Native completion performance receipt did not commit");
        }
        (status, id, String::from_utf8(body.to_vec()).unwrap())
    }

    async fn read(&self, channel: u64) -> (StatusCode, Value) {
        self.get(&format!(
            "/core/v1/admin/routing/channels/{channel}/usage?start_timestamp={}&end_timestamp={}",
            self.start,
            self.start + 60
        ))
        .await
    }

    async fn get(&self, path: &str) -> (StatusCode, Value) {
        let response = self
            .app
            .clone()
            .oneshot(
                Request::builder()
                    .uri(path)
                    .header("authorization", format!("Bearer {}", self.admin))
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        let status = response.status();
        assert_eq!(response.headers()["cache-control"], "no-store");
        let body = serde_json::from_slice(&to_bytes(response.into_body(), 1 << 20).await.unwrap())
            .unwrap();
        (status, body)
    }

    async fn usage(&self, channel: u64) -> Value {
        let (status, body) = self.read(channel).await;
        assert_eq!(status, StatusCode::OK, "{body}");
        assert_eq!(body["data"]["scope"], "native_channel_window");
        assert_eq!(body["data"]["credits_per_usd"], "500000");
        assert_eq!(body["data"]["historical_coverage"], false);
        body["data"].clone()
    }

    async fn register_identities(&self) {
        sqlx::query("INSERT INTO core_channels.identities(id,origin,history_ambiguous) VALUES(1,'bootstrap',FALSE),(2,'bootstrap',FALSE) ON CONFLICT(id) DO NOTHING")
            .execute(&self.owner).await.unwrap();
    }
}

#[sqlx::test(migrations = false)]
async fn unpublished_bootstrap_reads_do_not_register_identities(pool: PgPool) {
    let supplier = model_fixture::Provider::start(true).await;
    let f = Fixture::new(pool, &supplier.endpoint, 1_000_000, false).await;
    let before: i64 = sqlx::query_scalar("SELECT count(*) FROM core_channels.identities")
        .fetch_one(&f.owner)
        .await
        .unwrap();
    assert_eq!(before, 0);
    let usage = f.usage(1).await;
    assert_eq!(usage["settled_requests"], "0");
    assert_eq!(usage["unknown_provenance_requests"], "0");
    let (status, body) = f.get("/core/v1/admin/routing/channels/1/counters").await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(body["data"]["native"]["settled_requests"], "0");
    let after: i64 = sqlx::query_scalar("SELECT count(*) FROM core_channels.identities")
        .fetch_one(&f.owner)
        .await
        .unwrap();
    assert_eq!(before, after);
    f.role.cleanup().await;
}

#[sqlx::test(migrations = false)]
async fn lifetime_uses_all_stored_facts_while_window_excludes_inheritance(pool: PgPool) {
    let supplier = model_fixture::Provider::start(true).await;
    let f = Fixture::new(pool, &supplier.endpoint, 1_000_000, false).await;
    f.register_identities().await;
    synthetic_settled(&f, "native-before-window", 10, true).await;
    synthetic_settled(&f, "native-after-clock", 20, true).await;
    synthetic_settled(&f, "unproven-before-window", 5, false).await;
    // Owner-only time fixtures distinguish a lifetime scan from both a 31-day
    // window and a wall-clock cutoff. No monetary receipt is replaced.
    let mut tx = f.owner.begin().await.unwrap();
    sqlx::query("SET TRANSACTION ISOLATION LEVEL SERIALIZABLE")
        .execute(&mut *tx)
        .await
        .unwrap();
    sqlx::query("ALTER TABLE core_charging.requests DISABLE TRIGGER request_identity")
        .execute(&mut *tx)
        .await
        .unwrap();
    for (id, at) in [
        ("native-before-window", f.start - 40 * 86400),
        ("native-after-clock", f.start + 3600),
        ("unproven-before-window", f.start - 40 * 86400),
    ] {
        sqlx::query("UPDATE core_charging.requests SET authorized_at=$2,lease_until=$2+300,hard_deadline=$2+600 WHERE id=$1")
            .bind(id).bind(at).execute(&mut *tx).await.unwrap();
    }
    sqlx::query("SET CONSTRAINTS ALL IMMEDIATE")
        .execute(&mut *tx)
        .await
        .unwrap();
    sqlx::query("ALTER TABLE core_charging.requests ENABLE TRIGGER request_identity")
        .execute(&mut *tx)
        .await
        .unwrap();
    sqlx::query("INSERT INTO core_channels.counter_baselines(channel_id,gross_credits,refunded_credits,historical_coverage,source_channel_id,source_routing_version) VALUES(1,1000,50,TRUE,2,1)")
        .execute(&mut *tx).await.unwrap();
    tx.commit().await.unwrap();
    let window = f.usage(1).await;
    assert_eq!(window["gross_settled_credits"], "0");
    assert_eq!(window["settled_requests"], "0");
    assert_eq!(window["unknown_provenance_requests"], "0");
    let (status, body) = f.get("/core/v1/admin/routing/channels/1/counters").await;
    assert_eq!(status, StatusCode::OK, "{body}");
    let counters = &body["data"];
    assert_eq!(counters["native"]["scope"], "native_channel_totals");
    assert_eq!(counters["native"]["gross_settled_credits"], "32");
    assert_eq!(counters["native"]["refunded_credits"], "0");
    assert_eq!(counters["native"]["settled_requests"], "2");
    assert_eq!(counters["native"]["unknown_provenance_requests"], "1");
    assert_eq!(counters["inherited"]["net_used_credits"], "950");
    assert_eq!(counters["combined"]["net_used_credits"], "982");
    assert_eq!(counters["complete"], false);
    assert!(counters["used_quota"].is_null());
    f.role.cleanup().await;
}

#[sqlx::test(migrations = false)]
async fn actual_fallback_refund_and_subscription_are_one_logical_charge(pool: PgPool) {
    let supplier = model_fixture::Provider::start(true).await;
    let f = Fixture::new(pool, &supplier.endpoint, 1_000_000, false).await;
    let (status, first) = f.call("wallet", false).await;
    assert_eq!(status, StatusCode::OK);
    let wallet_source: Option<i64> =
        sqlx::query_scalar("SELECT source_subscription_id FROM core_charging.requests WHERE id=$1")
            .bind(&first)
            .fetch_one(&f.owner)
            .await
            .unwrap();
    assert!(wallet_source.is_none());
    f.charging
        .refund(&first, "refund", 20, "synthetic provider adjustment")
        .await
        .unwrap();
    // Replay neither dispatches nor consumes again.
    let replay = f.call("wallet", false).await;
    assert_eq!(replay.0, StatusCode::CONFLICT);
    assert_eq!(replay.1, first);
    let mut tx = f.owner.begin().await.unwrap();
    sqlx::query("SET TRANSACTION ISOLATION LEVEL SERIALIZABLE")
        .execute(&mut *tx)
        .await
        .unwrap();
    sqlx::query("UPDATE core_billing.account_policies SET preference='subscription_only' WHERE account_id=$1")
        .bind(f.account).execute(&mut *tx).await.unwrap();
    // The common fixture issues a wallet_only personal Key. Inherit the real
    // account policy for the second Native admission instead of overriding it.
    sqlx::query(
        "UPDATE core_identity.credentials SET personal_billing_preference=NULL WHERE id=$1",
    )
    .bind(f.key_id)
    .execute(&mut *tx)
    .await
    .unwrap();
    let subscription: i64 = sqlx::query_scalar("INSERT INTO core_charging.subscriptions(account_id,grant_reference,period,anchor,seconds,ends_at,limit_credits,models,groups) VALUES($1,'channel-usage-subscription','custom',0,86400,253402300799,10000,ARRAY['local-model'],ARRAY['allowed']) RETURNING id")
        .bind(f.account).fetch_one(&mut *tx).await.unwrap();
    tx.commit().await.unwrap();
    let (status, second) = f.call("subscription", false).await;
    assert_eq!(status, StatusCode::OK);
    f.charging
        .refund(&second, "refund", 10, "synthetic subscription adjustment")
        .await
        .unwrap();
    let (source, reserved): (Option<i64>, i64) = sqlx::query_as(
        "SELECT source_subscription_id,reserved FROM core_charging.requests WHERE id=$1",
    )
    .bind(&second)
    .fetch_one(&f.owner)
    .await
    .unwrap();
    assert_eq!(source, Some(subscription));
    assert!(
        reserved <= 10_000,
        "the finite subscription must cover the full reservation"
    );
    // Inherited display counters must not inflate actual window consumption.
    f.register_identities().await;
    sqlx::query("UPDATE core_channels.counter_baselines SET gross_credits=1000,refunded_credits=50,historical_coverage=TRUE,source_channel_id=1,source_routing_version=1 WHERE channel_id=2")
        .execute(&f.owner).await.unwrap();
    let before: Value = sqlx::query_scalar("SELECT jsonb_agg(to_jsonb(b) ORDER BY ledger_account_id) FROM core_billing.balance_state b")
        .fetch_one(&f.owner).await.unwrap();
    let usage = f.usage(2).await;
    assert_eq!(usage["gross_settled_credits"], "98");
    assert_eq!(usage["refunded_credits"], "30");
    assert_eq!(usage["net_used_credits"], "68");
    assert_eq!(usage["settled_requests"], "2");
    assert_eq!(usage["pending_potential_requests"], "0");
    assert_eq!(usage["unknown_provenance_requests"], "0");
    let primary = f.usage(1).await;
    assert_eq!(primary["settled_requests"], "0");
    assert_eq!(primary["net_used_credits"], "0");
    let after: Value = sqlx::query_scalar("SELECT jsonb_agg(to_jsonb(b) ORDER BY ledger_account_id) FROM core_billing.balance_state b")
        .fetch_one(&f.owner).await.unwrap();
    assert_eq!(before, after);
    sqlx::query("UPDATE core_channels.identities SET history_ambiguous=TRUE WHERE id=2")
        .execute(&f.owner)
        .await
        .unwrap();
    let (status, body) = f.read(2).await;
    assert_eq!(status, StatusCode::CONFLICT, "{body}");
    assert!(body.get("data").is_none());
    f.role.cleanup().await;
}

fn spec(id: &str, input: i64) -> ChargeRequest {
    ChargeRequest {
        id: id.into(),
        fingerprint: [41; 32],
        reserve: input + 1,
        lease_seconds: 60,
        maximum_seconds: 120,
        price: Price {
            text_request: None,
            tool: None,
            grounding: None,
            user_trust: None,
            image: None,
            upstream_cost_group_ppm: None,
            tiered: None,
            version: "synthetic-channel-v1".into(),
            model: "local-model".into(),
            group: "allowed".into(),
            input_per_million: if input > u32::MAX as i64 {
                input
            } else {
                1_000_000
            },
            cached_per_million: 0,
            output_per_million: 1_000_000,
        },
    }
}

async fn admission_snapshot(pool: &PgPool) -> Value {
    sqlx::query_scalar("SELECT jsonb_build_object(\
        'requests',(SELECT jsonb_agg(to_jsonb(r) ORDER BY id) FROM core_charging.requests r),\
        'events',(SELECT jsonb_agg(to_jsonb(e) ORDER BY request_id,event_id) FROM core_charging.usage_events e),\
        'commands',(SELECT jsonb_agg(to_jsonb(c) ORDER BY request_id,operation_id) FROM core_charging.ledger_commands c),\
        'operations',(SELECT jsonb_agg(to_jsonb(o) ORDER BY scope,operation_key) FROM core_billing.ledger_operations o),\
        'journals',(SELECT jsonb_agg(to_jsonb(j) ORDER BY id) FROM core_billing.ledger_journals j),\
        'entries',(SELECT jsonb_agg(to_jsonb(e) ORDER BY journal_id,ledger_account_id) FROM core_billing.ledger_entries e),\
        'balances',(SELECT jsonb_agg(to_jsonb(b) ORDER BY ledger_account_id) FROM core_billing.balance_state b),\
        'identities',(SELECT jsonb_agg(to_jsonb(i) ORDER BY id) FROM core_channels.identities i),\
        'baselines',(SELECT jsonb_agg(to_jsonb(b) ORDER BY channel_id) FROM core_channels.counter_baselines b),\
        'observations',(SELECT jsonb_agg(to_jsonb(o) ORDER BY channel_id) FROM core_channels.balance_observations o))")
        .fetch_one(pool).await.unwrap()
}

#[sqlx::test(migrations = false)]
async fn native_admission_registers_candidates_and_retirement_blocks_only_new_reservations(
    pool: PgPool,
) {
    let supplier = model_fixture::Provider::start(true).await;
    let f = Fixture::new(pool, &supplier.endpoint, 1_000_000, false).await;
    let (status, first) = f.call("bootstrap-registration", false).await;
    assert_eq!(status, StatusCode::OK);
    let identities: Vec<(i64, String, bool, String)> = sqlx::query_as(
        "SELECT i.id,i.origin,b.historical_coverage,o.kind FROM core_channels.identities i \
         JOIN core_channels.counter_baselines b ON b.channel_id=i.id \
         JOIN core_channels.balance_observations o ON o.channel_id=i.id ORDER BY i.id",
    )
    .fetch_all(&f.owner)
    .await
    .unwrap();
    assert_eq!(
        identities,
        vec![
            (1, "bootstrap".into(), false, "unknown".into()),
            (2, "bootstrap".into(), false, "unknown".into())
        ]
    );
    sqlx::query("UPDATE core_channels.identities SET retired_at=clock_timestamp() WHERE id=2")
        .execute(&f.owner)
        .await
        .unwrap();
    let before = admission_snapshot(&f.owner).await;
    let hits = supplier.hits.load(std::sync::atomic::Ordering::SeqCst);
    assert_ne!(f.call("retired-fallback", false).await.0, StatusCode::OK);
    assert_eq!(
        supplier.hits.load(std::sync::atomic::Ordering::SeqCst),
        hits
    );
    assert_eq!(admission_snapshot(&f.owner).await, before);
    // Accounting corrections to an existing charge survive channel retirement.
    f.charging
        .refund(&first, "after-retirement", 1, "synthetic correction")
        .await
        .unwrap();
    f.role.cleanup().await;
}

#[sqlx::test(migrations = false)]
async fn unfunded_native_admission_rolls_back_contract_and_candidate_metadata(pool: PgPool) {
    let supplier = model_fixture::Provider::start(true).await;
    let f = Fixture::new(pool, &supplier.endpoint, 1, false).await;
    let before = admission_snapshot(&f.owner).await;
    let (status, _, body) = f.call_response("unfunded-bootstrap", false).await;
    assert_eq!(status, StatusCode::PAYMENT_REQUIRED, "{body}");
    assert_eq!(
        serde_json::from_str::<Value>(&body).unwrap()["error"]["code"],
        "insufficient_funds"
    );
    assert_eq!(supplier.hits.load(std::sync::atomic::Ordering::SeqCst), 0);
    assert_eq!(admission_snapshot(&f.owner).await, before);
    f.role.cleanup().await;
}

/// Reuse the existing owner-only preparation fixture; no duplicate hash/contract
/// implementation. Dispatch evidence is synthetic, while all money is posted by
/// the actual Charging/TransactionalLedger under the restricted runtime role.
async fn synthetic_settled(f: &Fixture, id: &str, input: i64, proven: bool) {
    let spec = spec(id, input);
    let tokens = if input > u32::MAX as i64 {
        1_000_000
    } else {
        input
    };
    f.charging.reserve(&f.key, &spec, &WORKER).await.unwrap();
    if proven {
        reservation_contract::install_synthetic_recovery_contract(&f.owner, id, tokens as u64, 1)
            .await;
        let mut tx = f.owner.begin().await.unwrap();
        sqlx::query("SET TRANSACTION ISOLATION LEVEL SERIALIZABLE")
            .execute(&mut *tx)
            .await
            .unwrap();
        sqlx::query("INSERT INTO core_charging.usage_events(request_id,event_id,detail) SELECT $1,'route-dispatch:1',jsonb_build_object('kind','model_route_dispatch','number',1,'route_id',detail->'binding'->>'route_id','actual_group','allowed','wire_sha256',detail->>'wire_sha256') FROM core_charging.usage_events WHERE request_id=$1 AND event_id='reservation-contract:v1'")
            .bind(id).execute(&mut *tx).await.unwrap();
        tx.commit().await.unwrap();
    }
    f.charging.start(id, &WORKER).await.unwrap();
    f.charging
        .settle(
            id,
            &WORKER,
            Usage {
                input: tokens,
                cached: 0,
                output: 1,
                ..Default::default()
            },
            Outcome::Completed,
        )
        .await
        .unwrap();
}

#[sqlx::test(migrations = false)]
async fn unknown_supplier_and_unproven_settlement_do_not_become_zero_consumption(pool: PgPool) {
    let supplier = model_fixture::Provider::start(false).await;
    let f = Fixture::new(pool, &supplier.endpoint, 1_000_000, false).await;
    let (status, id) = f.call("unknown", false).await;
    assert_ne!(status, StatusCode::OK);
    let (state, reserved, settled): (String, i64, i64) =
        sqlx::query_as("SELECT state,reserved,settled FROM core_charging.requests WHERE id=$1")
            .bind(id)
            .fetch_one(&f.owner)
            .await
            .unwrap();
    assert_eq!(state, "reconcile");
    assert!(reserved > 0);
    assert_eq!(settled, 0);
    synthetic_settled(&f, "old-without-provenance", 10, false).await;
    let usage = f.usage(2).await;
    assert_eq!(usage["settled_requests"], "0");
    assert_eq!(usage["pending_potential_requests"], "1");
    assert_eq!(usage["unknown_provenance_requests"], "1");
    assert_eq!(usage["gross_settled_credits"], "0");
    f.role.cleanup().await;
}

#[sqlx::test(migrations = false)]
async fn real_zero_and_sums_above_javascript_safe_integer_are_exact_strings(pool: PgPool) {
    let supplier = model_fixture::Provider::start(true).await;
    let f = Fixture::new(pool, &supplier.endpoint, 20_000_000_000_000_000, true).await;
    assert_eq!(f.call("zero", false).await.0, StatusCode::OK);
    let zero = f.usage(2).await;
    assert_eq!(zero["settled_requests"], "1");
    assert_eq!(zero["gross_settled_credits"], "0");
    for id in ["large-a", "large-b"] {
        synthetic_settled(&f, id, 5_000_000_000_000_000, true).await;
    }
    let large = f.usage(1).await;
    assert_eq!(large["gross_settled_credits"], "10000000000000002");
    assert_eq!(large["net_used_credits"], "10000000000000002");
    assert_eq!(large["settled_requests"], "2");
    assert_eq!(large["unknown_provenance_requests"], "0");
    f.role.cleanup().await;
}

#[sqlx::test(migrations = false)]
async fn excess_receipts_are_an_explicit_limit_not_partial_success(pool: PgPool) {
    let supplier = model_fixture::Provider::start(true).await;
    let f = Fixture::new(pool, &supplier.endpoint, 1_000_000, false).await;
    let (status, id) = f.call("receipt-density", false).await;
    assert_eq!(status, StatusCode::OK);
    // Owner-only density fixture: add audit rows without posting more money.
    // The extra receipt must reject the entire read before any truncation.
    let mut tx = f.owner.begin().await.unwrap();
    sqlx::query("SET TRANSACTION ISOLATION LEVEL SERIALIZABLE")
        .execute(&mut *tx)
        .await
        .unwrap();
    sqlx::query("INSERT INTO core_charging.ledger_commands(operation_id,request_id,command) SELECT md5('density-a:'||n)||md5('density-b:'||n),$1,jsonb_set(c.command,'{operation_id}',to_jsonb(md5('density-a:'||n)||md5('density-b:'||n))) FROM generate_series(1,2048) n CROSS JOIN LATERAL (SELECT command FROM core_charging.ledger_commands WHERE request_id=$1 ORDER BY operation_id LIMIT 1) c")
        .bind(&id).execute(&mut *tx).await.unwrap();
    tx.commit().await.unwrap();
    let (status, body) = f.read(2).await;
    assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY, "{body}");
    assert!(body.get("data").is_none());
    f.role.cleanup().await;
}

#[sqlx::test(migrations = false)]
async fn malformed_frozen_binding_is_unknown_but_malformed_money_receipt_is_an_error(pool: PgPool) {
    let supplier = model_fixture::Provider::start(true).await;
    let f = Fixture::new(pool, &supplier.endpoint, 1_000_000, false).await;
    let (status, id) = f.call("corrupt", false).await;
    assert_eq!(status, StatusCode::OK);
    // Owner-only corruption of the disposable fixture; the reader cannot repair
    // a changed binding using a current endpoint or a readable route ID suffix.
    let mut tx = f.owner.begin().await.unwrap();
    sqlx::query("SET TRANSACTION ISOLATION LEVEL SERIALIZABLE")
        .execute(&mut *tx)
        .await
        .unwrap();
    sqlx::query("ALTER TABLE core_charging.usage_events DISABLE TRIGGER usage_immutable")
        .execute(&mut *tx)
        .await
        .unwrap();
    sqlx::query("UPDATE core_charging.usage_events SET detail=jsonb_set(detail,'{binding,upstream_id}','2') WHERE request_id=$1 AND event_id='reservation-contract:v1'")
        .bind(&id).execute(&mut *tx).await.unwrap();
    sqlx::query("ALTER TABLE core_charging.usage_events ENABLE TRIGGER usage_immutable")
        .execute(&mut *tx)
        .await
        .unwrap();
    tx.commit().await.unwrap();
    let usage = f.usage(2).await;
    assert_eq!(usage["unknown_provenance_requests"], "1");
    assert_eq!(usage["gross_settled_credits"], "0");
    let mut tx = f.owner.begin().await.unwrap();
    sqlx::query("SET TRANSACTION ISOLATION LEVEL SERIALIZABLE")
        .execute(&mut *tx)
        .await
        .unwrap();
    let original: Value = sqlx::query_scalar("SELECT command FROM core_charging.ledger_commands WHERE request_id=$1 AND command->'change'->>'operation'='settle'")
        .bind(&id).fetch_one(&mut *tx).await.unwrap();
    sqlx::query("UPDATE core_charging.ledger_commands SET command=jsonb_set(command,'{change,amount}','999') WHERE request_id=$1 AND command->'change'->>'operation'='settle'")
        .bind(&id).execute(&mut *tx).await.unwrap();
    tx.commit().await.unwrap();
    let (status, body) = f.read(2).await;
    assert_eq!(status, StatusCode::SERVICE_UNAVAILABLE, "{body}");
    assert!(body.get("data").is_none());
    let mut tx = f.owner.begin().await.unwrap();
    sqlx::query("SET TRANSACTION ISOLATION LEVEL SERIALIZABLE")
        .execute(&mut *tx)
        .await
        .unwrap();
    sqlx::query("UPDATE core_charging.ledger_commands SET command=$2 WHERE request_id=$1 AND command->'change'->>'operation'='settle'")
        .bind(&id).bind(original).execute(&mut *tx).await.unwrap();
    sqlx::query("ALTER TABLE core_charging.usage_events DISABLE TRIGGER usage_immutable")
        .execute(&mut *tx)
        .await
        .unwrap();
    sqlx::query("UPDATE core_charging.usage_events SET detail=jsonb_set(detail,'{kind}','\"not-final\"') WHERE request_id=$1 AND event_id='final'")
        .bind(&id).execute(&mut *tx).await.unwrap();
    sqlx::query("ALTER TABLE core_charging.usage_events ENABLE TRIGGER usage_immutable")
        .execute(&mut *tx)
        .await
        .unwrap();
    tx.commit().await.unwrap();
    let (status, body) = f.read(2).await;
    assert_eq!(status, StatusCode::SERVICE_UNAVAILABLE, "{body}");
    assert!(body.get("data").is_none());
    f.role.cleanup().await;
}

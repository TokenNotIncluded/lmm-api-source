//! Independent acceptance: actual HTTP/funding before synthetic receipt scale.
//! Opt-in on one explicitly Main-provisioned empty PG database. No database
//! creation, Cargo invocation, production provider, or direct rollup seeding.
use axum::{
    Json, Router,
    body::{Body, to_bytes},
    http::{Request, StatusCode},
    routing::post,
};
use lmm_core::{
    billing::{
        charging::{Charging, Outcome, Price, Request as ChargeRequest, Usage},
        transactional_ledger::TransactionalLedger,
    },
    channel_http::{ManagedChannels, managed_router},
    model_http::{ChargePolicy, ModelGateway, ProviderPolicy, UpstreamCredential},
    relay::Limits,
    routing::{CredentialRef, Endpoint, PriceConfig, PriceSnapshot, RoutingConfig},
};
use serde_json::{Value, json};
use sqlx::PgPool;
use std::{
    collections::BTreeMap,
    sync::{
        Arc,
        atomic::{AtomicUsize, Ordering},
    },
    time::{Duration, Instant},
};
use tower::ServiceExt;
#[path = "common/model_fixture.rs"]
mod model_fixture;
#[path = "common/channel_projection.rs"]
mod projection;
#[path = "common/runtime.rs"]
mod runtime;

struct Fixture {
    owner: PgPool,
    role: runtime::RuntimeRole,
    app: Router,
    native: ModelGateway,
    charging: Charging,
    key: String,
    key_id: i64,
    admin: String,
    hits: Arc<AtomicUsize>,
    entered: Arc<tokio::sync::Notify>,
    release: Arc<tokio::sync::Semaphore>,
    server: tokio::task::JoinHandle<()>,
}

fn routing() -> (RoutingConfig, PriceConfig) {
    let upstreams: Vec<Value> = (1..=2).map(|id| json!({"id":id,"endpoint":format!("https://provider{id}.example/v1/"),"protocol":"openai_chat","credential":{"id":771,"revision":1},"enabled":true})).collect();
    let routes: Vec<Value> = (1..=2).map(|id| json!({"id":id,"model":format!("model-{id}"),"group":"allowed","enabled":true,"speeds":["standard"],"targets":[{"id":id,"upstream_id":id,"upstream_model":"provider-model","enabled":true,"priority":0,"weight":1,"speeds":["standard"],"payload":"adapt","timeout_ms":5000}],"failover":{"max_attempts":1,"retry_on":[],"allow_replay_after_send":false}})).collect();
    let rules: Vec<Value> = (1..=2).map(|id| json!({"id":id,"model":format!("model-{id}"),"group":"allowed","speed":"standard","rates":{"input":2000000,"output":2000000,"cache_read":2000000,"cache_write":0,"request":0},"group_multiplier_ppm":1000000,"speed_multiplier_ppm":1000000})).collect();
    (serde_json::from_value(json!({"version":1,"groups":[{"name":"allowed","enabled":true}],"upstreams":upstreams,"routes":routes,"aliases":[]})).unwrap(),
     serde_json::from_value(json!({"version":1,"rules":rules})).unwrap())
}
fn gateway(pool: &PgPool, key: i64, endpoint: Option<&str>) -> ModelGateway {
    let (mut config, prices) = routing();
    if let Some(endpoint) = endpoint {
        for upstream in &mut config.upstreams {
            upstream.endpoint = Endpoint::local_loopback(endpoint.into()).unwrap();
        }
    }
    let router = Arc::new(
        lmm_core::routing::Router::new(config, Arc::new(PriceSnapshot::compile(prices).unwrap()))
            .unwrap(),
    );
    let contexts = if endpoint.is_some() {
        Vec::new()
    } else {
        serde_json::from_value(json!((1..=2).map(|id| json!({"upstream_id":id,"upstream_model":"provider-model","protocol":"openai_chat","endpoint":format!("https://provider{id}.example/v1/"),"max_input_tokens":1000,"max_output_tokens":64,"contract_revision":"synthetic-projection-v1","missing_cached_tokens_is_zero":true,"missing_cache_creation_tokens_is_zero":true})).collect::<Vec<_>>())).unwrap()
    };
    ModelGateway::new_with_provider_policy(
        pool.clone(),
        router,
        vec![UpstreamCredential {
            reference: CredentialRef {
                id: 771,
                revision: 1,
            },
            secret: "synthetic-projection-provider".into(),
        }],
        BTreeMap::from([(key, "allowed".into())]),
        Limits {
            total_timeout: Duration::from_secs(3),
            max_output_tokens: 64,
            ..Limits::default()
        },
        ChargePolicy {
            input_token_ceiling: 1000,
            lease_seconds: 60,
        },
        ProviderPolicy::new(contexts, 1000).unwrap(),
    )
    .unwrap()
    .with_local_streaming()
}
impl Fixture {
    async fn new(owner: PgPool) -> Self {
        let (identity, _, _, key, key_id, _) = model_fixture::fixture(&owner, 1_000_000).await;
        let admin = identity.bootstrap_user(8, 6).await.unwrap().secret;
        sqlx::query("INSERT INTO core_identity.user_routing_groups(user_id,group_name,source_reference) VALUES(7,'allowed','projection-fixture')").execute(&owner).await.unwrap();
        sqlx::query("INSERT INTO core_identity.api_key_settings(credential_id,name,group_name) VALUES($1,'projection fixture','allowed')").bind(key_id).execute(&owner).await.unwrap();
        let role = runtime::RuntimeRole::create(&owner).await;
        role.optional("core_charging").await;
        role.optional("core_model").await;
        let hits = Arc::new(AtomicUsize::new(0));
        let entered = Arc::new(tokio::sync::Notify::new());
        let release = Arc::new(tokio::sync::Semaphore::new(0));
        let provider = Router::new().route("/v1/chat/completions",post({
            let hits=hits.clone();let entered=entered.clone();let release=release.clone();
            move |headers:axum::http::HeaderMap,Json(request):Json<Value>| { let hits=hits.clone();let entered=entered.clone();let release=release.clone();async move {
                hits.fetch_add(1,Ordering::SeqCst);
                assert_eq!(headers["authorization"],"Bearer synthetic-projection-provider");
                assert_eq!(request["model"],"provider-model");
                if request["messages"][0]["content"]=="projection-hold" {entered.notify_one();release.acquire().await.unwrap().forget();}
                Json(json!({"id":"synthetic-projection","object":"chat.completion","created":0,"model":"provider-model","choices":[{"index":0,"message":{"role":"assistant","content":"projection proof"},"finish_reason":"stop"}],"usage":{"prompt_tokens":40,"completion_tokens":1,"total_tokens":41,"prompt_tokens_details":{"cached_tokens":0}}}))
            }}
        }));
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let endpoint = format!("http://{}/v1/", listener.local_addr().unwrap());
        let server = tokio::spawn(async move { axum::serve(listener, provider).await.unwrap() });
        let native = gateway(&role.pool, key_id, Some(&endpoint));
        // Publication uses HTTPS metadata; the real HTTP provider is a distinct
        // existing loopback gateway. No production TLS/42 CHECK is weakened.
        let management = gateway(&role.pool, key_id, None)
            .with_channel_secret_key(&[86; 32])
            .unwrap();
        let manager = ManagedChannels::recover(role.pool.clone(), management)
            .await
            .unwrap();
        let app = managed_router(role.pool.clone(), manager);
        let charging = Charging::new(role.pool.clone(), Arc::new(TransactionalLedger));
        let f = Self {
            owner,
            role,
            app,
            native,
            charging,
            key,
            key_id,
            admin,
            hits,
            entered,
            release,
            server,
        };
        f.ok(
            "PUT",
            "/api/channel/1/status",
            json!({"expected_version":{"routing":1,"pricing":1},"enabled":true}),
        )
        .await;
        f
    }
    async fn request(&self, method: &str, path: &str, body: Value) -> (StatusCode, Value) {
        let response = self
            .app
            .clone()
            .oneshot(
                Request::builder()
                    .method(method)
                    .uri(path)
                    .header("authorization", format!("Bearer {}", self.admin))
                    .header("content-type", "application/json")
                    .body(Body::from(body.to_string()))
                    .unwrap(),
            )
            .await
            .unwrap();
        let status = response.status();
        if method == "GET" {
            assert_eq!(response.headers()["cache-control"], "no-store");
        }
        let bytes = to_bytes(response.into_body(), 1 << 20).await.unwrap();
        (status, serde_json::from_slice(&bytes).unwrap())
    }
    async fn ok(&self, method: &str, path: &str, body: Value) -> Value {
        let (status, body) = self.request(method, path, body).await;
        assert_eq!(status, StatusCode::OK, "{method} {path}: {body}");
        body["data"].clone()
    }
    async fn counters(&self, id: u64) -> Value {
        self.ok(
            "GET",
            &format!("/core/v1/admin/routing/channels/{id}/counters"),
            Value::Null,
        )
        .await
    }
    async fn window(&self, id: u64, start: i64, end: i64) -> Value {
        self.ok("GET",&format!("/core/v1/admin/routing/channels/{id}/usage?start_timestamp={start}&end_timestamp={end}"),Value::Null).await
    }
    async fn call(&self, channel: u64, nonce: &str) -> String {
        self.call_status(channel, nonce, StatusCode::OK).await
    }
    async fn call_status(&self, channel: u64, nonce: &str, expected: StatusCode) -> String {
        let response=self.native.router().oneshot(Request::builder().method("POST").uri("/v1/chat/completions")
            .header("authorization",format!("Bearer {}",self.key)).header("content-type","application/json")
            .header("idempotency-key",nonce)
            .body(Body::from(json!({"model":format!("model-{channel}"),"messages":[{"role":"user","content":if nonce=="projection-real-one" {"projection-hold"} else {"hello"}}],"max_tokens":32}).to_string())).unwrap()).await.unwrap();
        let status = response.status();
        let id = response
            .headers()
            .get("x-request-id")
            .expect("native request id")
            .to_str()
            .unwrap()
            .to_owned();
        let bytes = to_bytes(response.into_body(), 1 << 20).await.unwrap();
        assert_eq!(
            status,
            expected,
            "native fixture: {}",
            String::from_utf8_lossy(&bytes)
        );
        if expected == StatusCode::CONFLICT {
            let error: Value = serde_json::from_slice(&bytes).unwrap();
            assert_eq!(error["error"]["code"], "request_conflict");
            return id;
        }
        assert_eq!(
            serde_json::from_slice::<Value>(&bytes).unwrap()["choices"][0]["message"]["content"],
            "projection proof"
        );
        tokio::time::timeout(Duration::from_secs(5),async {
            loop {
                let done:bool=sqlx::query_scalar("SELECT EXISTS(SELECT FROM core_charging.usage_events WHERE request_id=$1 AND event_id='performance-v1')").bind(&id).fetch_one(&self.owner).await.unwrap();
                if done {break;} tokio::time::sleep(Duration::from_millis(5)).await;
            }
        }).await.expect("wait finite Native performance receipt before refund");
        id
    }
    async fn copy(&self, source: u64) -> u64 {
        let active: Value = sqlx::query_scalar("SELECT public.routing_control_read()")
            .fetch_one(&self.owner)
            .await
            .unwrap();
        self.ok("POST",&format!("/core/v1/admin/routing/channels/{source}/copy"),json!({"expected_version":{"routing":active["routing"]["version"],"pricing":active["pricing"]["version"]},"suffix":"_scale","reset_balance":false})).await["id"].as_u64().unwrap()
    }
}
fn amounts(data: &Value, n: u64) {
    assert_eq!(data["gross_settled_credits"], json!((41 * n).to_string()));
    assert_eq!(data["refunded_credits"], json!((5 * n).to_string()));
    assert_eq!(data["net_used_credits"], json!((36 * n).to_string()));
    assert_eq!(data["settled_requests"], json!(n.to_string()));
    assert_eq!(data["credits_per_usd"], "500000");
    assert_eq!(data["historical_coverage"], false);
}
async fn mutate(pool: &PgPool, sql: &str, id: &str) {
    let mut tx = pool.begin().await.unwrap();
    sqlx::query("SET TRANSACTION ISOLATION LEVEL SERIALIZABLE")
        .execute(&mut *tx)
        .await
        .unwrap();
    sqlx::query(sql).bind(id).execute(&mut *tx).await.unwrap();
    tx.commit().await.unwrap();
}
async fn projection_state(pool: &PgPool) -> Value {
    sqlx::query_scalar("SELECT jsonb_build_object(\
      'ready',(SELECT jsonb_agg(to_jsonb(t)) FROM core_channels.projection_state t),\
      'dirty',(SELECT jsonb_agg(to_jsonb(t) ORDER BY request_id) FROM core_channels.projection_dirty t),\
      'rollups',(SELECT jsonb_agg(to_jsonb(t) ORDER BY channel_id,day,bucket) FROM core_channels.usage_rollups t),\
      'contributions',(SELECT count(*) FROM core_channels.usage_contributions))")
        .fetch_one(pool).await.unwrap()
}

async fn opening_guard(f: &Fixture) {
    let financial = projection::financial_snapshot(&f.owner).await;
    // Explicit synthetic reviewed K=3 is confined to this local fixture. It is
    // not evidence that any real legacy archive has a known conversion basis.
    let hash = "b".repeat(64);
    sqlx::query("INSERT INTO core_channels.opening_sources(source_key,archive_sha256,export_sha256,mapping_sha256,unit_evidence_sha256,source_revision,native_overlap,native_request_ids,native_seal_sha256,rows_sha256,plan_sha256) VALUES($1,$1,$1,$1,$1,'synthetic-guard-v1','disjoint',ARRAY[]::text[],$1,$1,$1)").bind(&hash).execute(&f.owner).await.unwrap();
    sqlx::query("INSERT INTO core_channels.opening_baselines(channel_id,source_key,source_channel_id,raw_used_quota,credits_per_usd,offset_numerator,offset_denominator,historical_coverage,kind) VALUES(1,$1,9,1,3,1,3,FALSE,'imported')").bind(&hash).execute(&f.owner).await.unwrap();
    let executable:bool=sqlx::query_scalar("SELECT has_function_privilege(current_user,'core_channels.check_opening_copy()','EXECUTE')").fetch_one(&f.role.pool).await.unwrap();
    assert!(
        !executable,
        "trigger must not require granting runtime direct EXECUTE"
    );
    // Every invalid candidate uses the same otherwise-valid unused identity 2.
    // Thus rejection cannot be credited to a missing FK or duplicate PK.
    for (case, kind, parent, source, raw, basis, num, den, covered) in [
        ("imported", "imported", None, 9, 1, 3, 1, 3, false),
        ("source-channel", "copied", Some(1), 10, 1, 3, 1, 3, false),
        ("raw", "copied", Some(1), 9, 2, 3, 1, 3, false),
        ("basis", "copied", Some(1), 9, 1, 4, 1, 3, false),
        ("offset-numerator", "copied", Some(1), 9, 1, 3, 2, 3, false),
        (
            "offset-denominator",
            "copied",
            Some(1),
            9,
            1,
            3,
            1,
            4,
            false,
        ),
        ("coverage", "copied", Some(1), 9, 1, 3, 1, 3, true),
        ("missing-parent", "copied", Some(2), 9, 1, 3, 1, 3, false),
    ] {
        let error=sqlx::query("INSERT INTO core_channels.opening_baselines(channel_id,source_key,source_channel_id,raw_used_quota,credits_per_usd,offset_numerator,offset_denominator,historical_coverage,kind,parent_channel_id) VALUES(2,$1,$2,$3,$4,$5::bigint::numeric,$6::bigint::numeric,$7,$8,$9)")
            .bind(&hash).bind(i64::from(source)).bind(i64::from(raw)).bind(i64::from(basis)).bind(i64::from(num)).bind(i64::from(den)).bind(covered).bind(kind).bind(parent.map(i64::from)).execute(&f.role.pool).await.expect_err(case);
        assert_eq!(
            error.as_database_error().and_then(|e| e.code()).as_deref(),
            Some("P0001"),
            "{case} must fail at reviewed-facts trigger, not ACL/FK/PK"
        );
        let target: i64 = sqlx::query_scalar(
            "SELECT count(*) FROM core_channels.opening_baselines WHERE channel_id=2",
        )
        .fetch_one(&f.owner)
        .await
        .unwrap();
        assert_eq!(target, 0, "{case} inserted an unreviewed baseline");
    }
    let copied = f.copy(1).await;
    let equal:bool=sqlx::query_scalar("SELECT (a.source_key,a.source_channel_id,a.raw_used_quota,a.credits_per_usd,a.offset_numerator,a.offset_denominator,a.historical_coverage) IS NOT DISTINCT FROM (b.source_key,b.source_channel_id,b.raw_used_quota,b.credits_per_usd,b.offset_numerator,b.offset_denominator,b.historical_coverage) AND b.kind='copied' AND b.parent_channel_id=a.channel_id FROM core_channels.opening_baselines a JOIN core_channels.opening_baselines b ON b.channel_id=$1 WHERE a.channel_id=1").bind(copied as i64).fetch_one(&f.owner).await.unwrap();
    assert!(equal, "legitimate HTTP copy changed reviewed facts");
    let view = f.counters(copied).await;
    assert_eq!(
        view["lifetime"]["complete"], false,
        "incomplete source became covered through copy"
    );
    assert_eq!(view["lifetime"]["total_usd"], Value::Null);
    assert_eq!(
        projection::financial_snapshot(&f.owner).await,
        financial,
        "opening/copy guard wrote wallet ledger"
    );
}

#[tokio::test]
#[ignore = "Main must provide the explicit isolated existing database; see fixed runner"]
async fn channel_projection_acceptance() {
    assert_eq!(projection::DAY % 86400, 0);
    let scale = match std::env::var("LMM_PROJECTION_ROWS").as_deref() {
        Ok("0") => false,
        Ok("1000000") => true,
        _ => panic!("fixed runner requires rows=0 or 1000000"),
    };
    let f = Fixture::new(projection::approved_database().await).await;
    let stage = Instant::now();
    let a = {
        let pending = f.call(1, "projection-real-one");
        tokio::pin!(pending);
        tokio::select! {
            result=tokio::time::timeout(Duration::from_secs(5),f.entered.notified()) => {result.expect("Native provider not entered");},
            _=&mut pending=>panic!("provider response must stay gated"),
        }
        let dirty: i64 = sqlx::query_scalar("SELECT count(*) FROM core_channels.projection_dirty")
            .fetch_one(&f.owner)
            .await
            .unwrap();
        assert_eq!(
            dirty, 0,
            "real held Native request must have a current live contribution"
        );
        let held = f.counters(1).await;
        assert_eq!(held["native"]["pending_potential_requests"], "1");
        assert_eq!(held["native"]["net_used_credits"], "0");
        f.release.add_permits(1);
        pending.await
    };
    let b = f.call(2, "projection-real-two").await;
    for id in [&a, &b] {
        f.charging
            .refund(id, "projection-five", 5, "isolated correction")
            .await
            .unwrap();
    }
    let wallet:i64=sqlx::query_scalar("SELECT balance_units FROM core_billing.balance_state b JOIN core_billing.ledger_accounts a ON a.id=b.ledger_account_id JOIN core_identity.accounts o ON o.id=a.owner_account_id WHERE o.id=(SELECT owner_account_id FROM core_charging.requests WHERE id=$1) AND a.bucket='wallet'").bind(&a).fetch_one(&f.owner).await.unwrap();
    assert_eq!(wallet, 999928, "real Native captures 82 and refunds 10");
    let financial = projection::financial_snapshot(&f.owner).await;
    let before = projection_state(&f.owner).await;
    assert!(
        lmm_core::core_schema::rebuild_channel_usage(&f.role.pool)
            .await
            .is_err(),
        "runtime must not acquire owner rebuild authority"
    );
    assert_eq!(
        projection_state(&f.owner).await,
        before,
        "wrong role changed projection"
    );
    assert_eq!(
        f.call_status(1, "projection-real-one", StatusCode::CONFLICT)
            .await,
        a,
        "HTTP exact replay rejects redispatch and preserves identity"
    );
    assert_eq!(
        f.hits.load(Ordering::SeqCst),
        2,
        "replay dispatched supplier twice"
    );
    f.charging
        .refund(&a, "projection-five", 5, "isolated correction")
        .await
        .unwrap();
    assert_eq!(
        projection::financial_snapshot(&f.owner).await,
        financial,
        "replay changed ledger"
    );
    assert_eq!(
        projection_state(&f.owner).await,
        before,
        "replay changed projection"
    );
    for (id, channel) in [(&a, 1), (&b, 2)] {
        projection::assert_seed(&f.owner, id, channel).await;
    }
    amounts(&f.counters(1).await["native"], 1);
    amounts(&f.counters(2).await["native"], 1);
    projection::rebuild(&f.owner).await;
    assert_eq!(
        projection_state(&f.owner).await,
        before,
        "rebuild changed exact live rollup"
    );
    projection::assert_rollups(&f.owner, false).await;
    assert_eq!(projection::financial_snapshot(&f.owner).await, financial);
    projection::metric("small-real-native-funding-replay-rebuild", stage, 2);

    sqlx::query("UPDATE core_channels.projection_state SET ready=FALSE WHERE singleton")
        .execute(&f.owner)
        .await
        .unwrap();
    let not_ready = projection_state(&f.owner).await;
    assert_eq!(
        f.request(
            "GET",
            "/core/v1/admin/routing/channels/1/counters",
            Value::Null
        )
        .await
        .0,
        StatusCode::SERVICE_UNAVAILABLE,
        "unprepared projection must not claim zero"
    );
    assert_eq!(
        projection_state(&f.owner).await,
        not_ready,
        "GET repaired ready state"
    );
    projection::rebuild(&f.owner).await;
    assert_eq!(projection::financial_snapshot(&f.owner).await, financial);
    opening_guard(&f).await;

    if scale {
        let stage = Instant::now();
        projection::bulk(&f.owner, [&a, &b]).await;
        assert_eq!(
            projection::financial_snapshot(&f.owner).await,
            financial,
            "bulk touched real funding ledger"
        );
        assert_eq!(
            f.request(
                "GET",
                "/core/v1/admin/routing/channels/1/counters",
                Value::Null
            )
            .await
            .0,
            StatusCode::SERVICE_UNAVAILABLE,
            "dirty million must not serve stale total"
        );
        projection::rebuild(&f.owner).await; // every row, no fake rollup insertion
        let count:i64=sqlx::query_scalar("SELECT count(*) FROM core_channels.usage_contributions WHERE request_id LIKE 'projection-scale:%'").fetch_one(&f.owner).await.unwrap();
        assert_eq!(
            count, 1_000_000,
            "validator omitted or duplicated scale requests"
        );
        projection::assert_rollups(&f.owner, true).await;
        for (channel, n) in [(1, 1_000_001), (2, 11)] {
            let mut times = Vec::new();
            for _ in 0..7 {
                let now = Instant::now();
                let view = f.counters(channel).await;
                times.push(now.elapsed());
                amounts(&view["native"], n);
                assert_eq!(
                    view["complete"], false,
                    "Native only is not historical coverage"
                );
            }
            times.sort();
            println!(
                "PROJECTION_MEASURE {}",
                json!({"name":format!("lifetime-channel-{channel}"),"samples":7,"p50_ms":times[3].as_millis(),"max_ms":times[6].as_millis()})
            );
            assert!(
                times[6] < Duration::from_secs(10),
                "bounded local counter budget"
            );
        }
        let d = projection::DAY;
        // Inclusion is [start,end], independent of SQL daily/edge decomposition.
        for (start, end, n) in [
            (d - 1, d - 1, 200000),
            (d, d, 200000),
            (d, d + 86399, 600000),
            (d + 86400, d + 86400, 200000),
            (d - 1, d + 86400, 1000000),
        ] {
            let now = Instant::now();
            amounts(&f.window(1, start, end).await, n);
            projection::metric(&format!("window-{start}-{end}"), now, n as usize);
            assert!(now.elapsed() < Duration::from_secs(10));
        }
        let now = Instant::now();
        let copied = f.copy(1).await;
        projection::metric("million-copy-preserve", now, projection::ROWS);
        assert!(
            now.elapsed() < Duration::from_secs(20),
            "copy(false) must not scan global million"
        );
        let copied_view = f.counters(copied).await;
        assert_eq!(copied_view["combined"]["net_used_credits"], "36000036");
        assert_eq!(copied_view["native"]["net_used_credits"], "0");
        assert_eq!(
            projection::financial_snapshot(&f.owner).await,
            financial,
            "copy(false) wrote source ledger"
        );
        // A later real source refund changes the source, not copied display history.
        f.charging
            .refund(&a, "projection-later", 1, "after immutable display copy")
            .await
            .unwrap();
        assert_eq!(
            f.counters(copied).await["combined"],
            copied_view["combined"]
        );
        assert_eq!(
            f.counters(1).await["native"]["net_used_credits"],
            "36000035"
        );
        projection::metric("million-full-stage", stage, projection::ROWS + 10);
    }

    // A legitimate late terminal attempt invalidates attribution. Keep the real
    // financial receipt intact; backfill must preserve global unknown visibly.
    let late = f.call(1, "projection-late-attempt").await;
    f.charging
        .refund(&late, "projection-five", 5, "isolated correction")
        .await
        .unwrap();
    let financial = projection::financial_snapshot(&f.owner).await;
    mutate(&f.owner,"INSERT INTO core_charging.usage_events(request_id,event_id,detail) SELECT $1,'route-dispatch:2',jsonb_set(detail,'{number}','2') FROM core_charging.usage_events WHERE request_id=$1 AND event_id='route-dispatch:1'",&late).await;
    let dirty = projection_state(&f.owner).await;
    assert_eq!(
        f.request(
            "GET",
            "/core/v1/admin/routing/channels/2/counters",
            Value::Null
        )
        .await
        .0,
        StatusCode::SERVICE_UNAVAILABLE,
        "global dirty must not hide behind another channel"
    );
    // Dirty is window-scoped: today's late attempt cannot poison 2023 data.
    let old = f.window(1, projection::DAY, projection::DAY).await;
    amounts(&old, if scale { 200000 } else { 0 });
    assert_eq!(old["unknown_provenance_requests"], "0");
    assert_eq!(
        projection_state(&f.owner).await,
        dirty,
        "GET repaired derived state"
    );
    projection::rebuild(&f.owner).await;
    assert_eq!(
        f.counters(1).await["native"]["unknown_provenance_requests"],
        "1"
    );
    assert_eq!(
        f.counters(2).await["native"]["unknown_provenance_requests"],
        "0"
    );
    assert_eq!(projection::financial_snapshot(&f.owner).await, financial);

    // A real generic Charging consumer has no provider attribution. Its money
    // evidence remains valid; the missing channel proof is global unknown.
    let price: Value = sqlx::query_scalar("SELECT price FROM core_charging.requests WHERE id=$1")
        .bind(&a)
        .fetch_one(&f.owner)
        .await
        .unwrap();
    let price: Price = serde_json::from_value(price).unwrap();
    let spec = ChargeRequest {
        id: "projection-global-unattributed".into(),
        fingerprint: [119; 32],
        reserve: 41,
        lease_seconds: 60,
        maximum_seconds: 120,
        price,
    };
    let worker = [120; 32];
    f.charging.reserve(&f.key, &spec, &worker).await.unwrap();
    f.charging.start(&spec.id, &worker).await.unwrap();
    f.charging
        .settle(
            &spec.id,
            &worker,
            Usage {
                input: 40,
                output: 1,
                ..Default::default()
            },
            Outcome::Completed,
        )
        .await
        .unwrap();
    f.charging
        .refund(
            &spec.id,
            "global-five",
            5,
            "isolated unattributed correction",
        )
        .await
        .unwrap();
    let financial = projection::financial_snapshot(&f.owner).await;
    assert_eq!(
        f.counters(1).await["native"]["unknown_provenance_requests"],
        "2"
    );
    assert_eq!(
        f.counters(2).await["native"]["unknown_provenance_requests"],
        "1"
    );
    projection::rebuild(&f.owner).await;
    let global:i64=sqlx::query_scalar("SELECT count(*) FROM core_channels.usage_contributions WHERE request_id=$1 AND channel_id=0 AND unknown_requests=1").bind(&spec.id).fetch_one(&f.owner).await.unwrap();
    assert_eq!(global, 1);
    assert_eq!(projection::financial_snapshot(&f.owner).await, financial);

    // Explicit session read-only GET on a separate restricted pool. Recover and
    // reads must succeed without opportunistic baseline/identity/projection writes.
    let read_before = projection_state(&f.owner).await;
    let ro = sqlx::postgres::PgPoolOptions::new()
        .max_connections(1)
        .after_connect(|c, _| {
            Box::pin(async move {
                sqlx::query("SET default_transaction_read_only=on")
                    .execute(c)
                    .await?;
                Ok(())
            })
        })
        .connect_with(f.role.pool.connect_options().as_ref().clone())
        .await
        .unwrap();
    let management = gateway(&ro, f.key_id, None)
        .with_channel_secret_key(&[86; 32])
        .unwrap();
    let manager = ManagedChannels::recover(ro.clone(), management)
        .await
        .unwrap();
    let response = managed_router(ro.clone(), manager)
        .oneshot(
            Request::builder()
                .uri("/core/v1/admin/routing/channels/1/counters")
                .header("authorization", format!("Bearer {}", f.admin))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let _ = to_bytes(response.into_body(), 1 << 20).await.unwrap();
    ro.close().await;
    assert_eq!(projection_state(&f.owner).await, read_before);
    assert_eq!(projection::financial_snapshot(&f.owner).await, financial);

    // An extra unmatched settle command is schema-valid but semantically corrupt.
    // The real validator must reject it and roll back its ready/derived changes.
    mutate(&f.owner,"INSERT INTO core_charging.ledger_commands(operation_id,request_id,command) SELECT repeat('e',64),$1,jsonb_set(command,'{operation_id}',to_jsonb(repeat('e',64))) FROM core_charging.ledger_commands WHERE request_id=$1 AND command->'change'->>'operation'='settle' LIMIT 1",&late).await;
    let damaged = projection_state(&f.owner).await;
    assert!(
        lmm_core::core_schema::rebuild_channel_usage(&f.owner)
            .await
            .is_err(),
        "full validator silently accepted unmatched command"
    );
    assert_eq!(
        projection_state(&f.owner).await,
        damaged,
        "failed rebuild changed readiness/rollup"
    );
    assert_eq!(
        projection::financial_snapshot(&f.owner).await,
        financial,
        "rebuild wrote source money ledger"
    );
    f.server.abort();
    f.role.cleanup().await;
}

//! Display-only copies through real management HTTP, receipts, and restricted PG.
//! Dispatch proof and cached supplier observations are explicitly synthetic seeds;
//! reserve/settle/refund use the real Charging + TransactionalLedger implementation.
use axum::{
    Router,
    body::{Body, to_bytes},
    http::{Request, StatusCode, header},
};
use lmm_core::{
    billing::{
        charging::{Charging, Outcome, Price, Request as ChargeRequest, Usage},
        transactional_ledger::TransactionalLedger,
    },
    channel_http::{ManagedChannels, managed_router},
    model_http::{ChargePolicy, ModelGateway, ProviderPolicy, UpstreamCredential},
    relay::Limits,
    routing::{
        CredentialRef, PriceConfig, PriceSnapshot, ReleaseVersion, Router as ServingRouter,
        RoutingConfig, store::ReleaseDocument,
    },
    usage::{self, UsageStore},
};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use sqlx::PgPool;
use std::{collections::BTreeMap, sync::Arc, time::Duration};
use tower::ServiceExt;
#[path = "common/model_fixture.rs"]
mod model_fixture;
#[path = "common/reservation_contract.rs"]
mod reservation_contract;
#[path = "common/runtime.rs"]
mod runtime;
const WORKER: [u8; 32] = [61; 32];
const COPY: &str = "/core/v1/admin/routing/channels/1/copy";
#[derive(Clone, Copy, Debug, Deserialize, Serialize, PartialEq, Eq)]
struct Version {
    routing: u64,
    pricing: u64,
}
impl From<ReleaseVersion> for Version {
    fn from(value: ReleaseVersion) -> Self {
        Self {
            routing: value.routing,
            pricing: value.pricing,
        }
    }
}
impl Version {
    fn of(document: &ReleaseDocument) -> Self {
        Self {
            routing: document.routing.version,
            pricing: document.pricing.version,
        }
    }
}
fn fixture() -> ReleaseDocument {
    let config:RoutingConfig=serde_json::from_value(json!({"version":1,"groups":[{"name":"default","enabled":true}],
      "upstreams":[{"id":1,"endpoint":"https://provider.example/v1/","protocol":"openai_chat","credential":{"id":771,"revision":1},"enabled":true}],
      "routes":[{"id":1,"model":"model-a","group":"default","enabled":true,"speeds":["standard"],
       "targets":[{"id":1,"upstream_id":1,"upstream_model":"model-a","enabled":true,"priority":0,"weight":1,"speeds":["standard"],"payload":"adapt","timeout_ms":30000}],
       "failover":{"max_attempts":1,"retry_on":[],"allow_replay_after_send":false}}],"aliases":[]})).unwrap();
    let prices:PriceConfig=serde_json::from_value(json!({"version":1,"rules":[{"id":1,"model":"model-a","group":"default","speed":"standard",
      "rates":{"input":2,"output":4,"cache_read":0,"cache_write":0,"request":0},"group_multiplier_ppm":1000000,"speed_multiplier_ppm":1000000}]})).unwrap();
    ReleaseDocument {
        contract_version: 1,
        routing: config,
        pricing: prices,
    }
}
fn gateway(pool: &PgPool) -> ModelGateway {
    gateway_with_loopback(pool, None)
}
fn gateway_with_loopback(pool: &PgPool, loopback: Option<&str>) -> ModelGateway {
    gateway_fixture(pool, loopback, None)
}
fn gateway_fixture(
    pool: &PgPool,
    loopback: Option<&str>,
    missing_binding: Option<bool>,
) -> ModelGateway {
    let document = fixture();
    let mut config = document.routing;
    let prices = document.pricing;
    let mut contexts:Vec<lmm_core::model_http::ExternalModelContext>=serde_json::from_value(json!([{"upstream_id":1,"upstream_model":"model-a","protocol":"openai_chat","endpoint":"https://provider.example/v1/","max_input_tokens":1000,"max_output_tokens":100,"contract_revision":"synthetic-v1","missing_cached_tokens_is_zero":true,"missing_cache_creation_tokens_is_zero":true}])).unwrap();
    if let Some(endpoint) = loopback {
        config.upstreams[0].endpoint =
            lmm_core::routing::Endpoint::local_loopback(endpoint.into()).unwrap();
        contexts.clear();
    }
    if let Some(missing_secret) = missing_binding {
        config.upstreams[0].enabled = false;
        config.routes[0].enabled = false;
        if !missing_secret {
            contexts.clear();
        }
    }
    let credentials = if missing_binding == Some(true) {
        Vec::new()
    } else {
        vec![UpstreamCredential {
            reference: CredentialRef {
                id: 771,
                revision: 1,
            },
            secret: "synthetic-channel-only".into(),
        }]
    };
    let routing = Arc::new(
        ServingRouter::new(config, Arc::new(PriceSnapshot::compile(prices).unwrap())).unwrap(),
    );
    ModelGateway::new_with_provider_policy(
        pool.clone(),
        routing,
        credentials,
        BTreeMap::new(),
        Limits {
            total_timeout: Duration::from_millis(500),
            max_output_tokens: 100,
            ..Limits::default()
        },
        ChargePolicy {
            input_token_ceiling: 1000,
            lease_seconds: 60,
        },
        ProviderPolicy::new(contexts, 1000).unwrap(),
    )
    .unwrap()
}
async fn request(
    app: Router,
    method: &str,
    path: &str,
    secret: &str,
    body: Value,
) -> (StatusCode, Value) {
    let response = app
        .oneshot(
            Request::builder()
                .method(method)
                .uri(path)
                .header(header::AUTHORIZATION, format!("Bearer {secret}"))
                .header(header::CONTENT_TYPE, "application/json")
                .body(Body::from(serde_json::to_vec(&body).unwrap()))
                .unwrap(),
        )
        .await
        .unwrap();
    let status = response.status();
    let bytes = to_bytes(response.into_body(), 1024 * 1024).await.unwrap();
    (
        status,
        serde_json::from_slice(&bytes)
            .unwrap_or_else(|_| json!({"raw":String::from_utf8_lossy(&bytes)})),
    )
}
fn channel_input(expected: Version, id: u64, secret: &str) -> Value {
    json!({"expected_version":expected,"id":id,"name":"Published provider","endpoint":"https://second.example/v1/",
      "protocol":"openai_chat","enabled":true,"secret":secret,
      "contexts":[{"upstream_id":id,"upstream_model":"model-a","protocol":"openai_chat","endpoint":"https://second.example/v1/",
        "max_input_tokens":1000,"max_output_tokens":100,"contract_revision":"synthetic-provider-v2",
        "missing_cached_tokens_is_zero":true,"missing_cache_creation_tokens_is_zero":true}],
      "attachments":[{"route_id":1,"upstream_model":"model-a","priority":0,"weight":1,"speeds":["standard"],"timeout_ms":30000,"payload":"adapt"}]})
}

struct Fixture {
    owner: PgPool,
    role: runtime::RuntimeRole,
    app: Router,
    gateway: ModelGateway,
    charging: Charging,
    key: String,
    admin: String,
    ordinary: String,
}
impl Fixture {
    async fn unpublished(owner: PgPool) -> Self {
        let (identity, _, ordinary, key, _, _) = model_fixture::fixture(&owner, 1_000_000).await;
        let admin = identity.bootstrap_user(8, 6).await.unwrap().secret;
        let role = runtime::RuntimeRole::create(&owner).await;
        role.optional("core_charging").await;
        role.optional("core_model").await;
        let gateway = gateway(&role.pool)
            .with_channel_secret_key(&[83; 32])
            .unwrap();
        let manager = ManagedChannels::recover(role.pool.clone(), gateway.clone())
            .await
            .unwrap();
        let app = managed_router(role.pool.clone(), manager)
            .merge(usage::router(UsageStore::new(role.pool.clone())));
        let charging = Charging::new(role.pool.clone(), Arc::new(TransactionalLedger));
        Self {
            owner,
            role,
            app,
            gateway,
            charging,
            key,
            admin,
            ordinary,
        }
    }
    async fn new(owner: PgPool) -> Self {
        let f = Self::unpublished(owner).await;
        f.ok(
            "PUT",
            "/api/channel/1/status",
            json!({"expected_version":Version::of(&fixture()),"enabled":true}),
        )
        .await;
        // Explicitly model a source with known pre-Native history. No test infers
        // complete coverage merely because today's Native receipts are valid.
        sqlx::query("UPDATE core_channels.counter_baselines SET gross_credits=1000,refunded_credits=50,historical_coverage=TRUE WHERE channel_id=1")
            .execute(&f.owner).await.unwrap();
        sqlx::query("UPDATE core_channels.balance_observations SET balance=-12.125,unit_label='supplier credits',observed_at=to_timestamp(1700000000),kind='provider',credential_id=771,credential_revision=1,recorded_at=to_timestamp(1700000100) WHERE channel_id=1")
            .execute(&f.owner).await.unwrap();
        f
    }
    async fn ok(&self, method: &str, path: &str, body: Value) -> Value {
        let (status, result) = request(self.app.clone(), method, path, &self.admin, body).await;
        assert_eq!(status, StatusCode::OK, "{method} {path}: {result}");
        result["data"].clone()
    }
    async fn version(&self) -> Version {
        let active: Value = sqlx::query_scalar("SELECT public.routing_control_read()")
            .fetch_one(&self.owner)
            .await
            .unwrap();
        Version::of(&serde_json::from_value(active).unwrap())
    }
    async fn counters(&self, id: u64) -> Value {
        self.ok(
            "GET",
            &format!("/core/v1/admin/routing/channels/{id}/counters"),
            Value::Null,
        )
        .await
    }
    async fn copy(&self, legacy: bool, reset: bool) -> u64 {
        let path = if legacy {
            format!("/api/channel/copy/1?reset_balance={reset}&suffix=_legacy")
        } else {
            COPY.into()
        };
        self.ok("POST",&path,if legacy {Value::Null} else {
            json!({"expected_version":self.version().await,"suffix":"_native","reset_balance":reset})
        }).await["id"].as_u64().unwrap()
    }
    async fn stats(&self) -> Value {
        let now: i64 =
            sqlx::query_scalar("SELECT floor(extract(epoch FROM clock_timestamp()))::bigint")
                .fetch_one(&self.owner)
                .await
                .unwrap();
        let data = self
            .ok(
                "GET",
                &format!(
                    "/core/v1/usage/admin/stats?start_timestamp={}&end_timestamp={}",
                    now - 86400,
                    now + 86400
                ),
                Value::Null,
            )
            .await;
        let mut result = json!({});
        for field in [
            "gross_settled_credits",
            "refunded_credits",
            "net_consumption_credits",
            "quota",
        ] {
            assert!(
                !data[field].is_null(),
                "missing stats field {field}: {data}"
            );
            result[field] = data[field].clone();
        }
        result
    }
    async fn settle_source(&self) {
        let spec = ChargeRequest {
            id: "copy-source-charge".into(),
            fingerprint: [62; 32],
            reserve: 41,
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
                version: "copy-source-v1".into(),
                model: "local-model".into(),
                group: "allowed".into(),
                input_per_million: 1_000_000,
                cached_per_million: 0,
                output_per_million: 1_000_000,
            },
        };
        self.charging
            .reserve(&self.key, &spec, &WORKER)
            .await
            .unwrap();
        reservation_contract::install_synthetic_recovery_contract(&self.owner, &spec.id, 40, 1)
            .await;
        let mut dispatch = self.owner.begin().await.unwrap();
        sqlx::query("SET TRANSACTION ISOLATION LEVEL SERIALIZABLE")
            .execute(&mut *dispatch)
            .await
            .unwrap();
        sqlx::query("INSERT INTO core_charging.usage_events(request_id,event_id,detail) SELECT $1,'route-dispatch:1',jsonb_build_object('kind','model_route_dispatch','number',1,'route_id',detail->'binding'->>'route_id','actual_group','allowed','wire_sha256',detail->>'wire_sha256') FROM core_charging.usage_events WHERE request_id=$1 AND event_id='reservation-contract:v1'")
            .bind(&spec.id).execute(&mut *dispatch).await.unwrap();
        dispatch.commit().await.unwrap();
        self.charging.start(&spec.id, &WORKER).await.unwrap();
        self.charging
            .settle(
                &spec.id,
                &WORKER,
                Usage {
                    input: 40,
                    cached: 0,
                    output: 1,
                    ..Default::default()
                },
                Outcome::Completed,
            )
            .await
            .unwrap();
        self.charging
            .refund(&spec.id, "first-refund", 5, "synthetic pre-copy adjustment")
            .await
            .unwrap();
    }
    async fn finish(self) {
        drop(self.app);
        drop(self.gateway);
        drop(self.charging);
        self.role.cleanup().await;
    }
}

// Exact row snapshots detect accidental UPDATEs as well as new journal/charge rows.
// Exclude sequences: failed SQL transactions legitimately consume sequence values.
async fn snapshot(pool: &PgPool, lifecycle: bool) -> Value {
    let tables:Vec<(String,String)>=sqlx::query_as("SELECT schemaname,tablename FROM pg_tables WHERE schemaname IN ('core_billing','core_charging') OR ($1 AND (schemaname='core_channels' OR (schemaname='public' AND (tablename LIKE 'routing_%' OR tablename LIKE 'channel_%')))) ORDER BY schemaname,tablename")
        .bind(lifecycle).fetch_all(pool).await.unwrap();
    let mut result = json!({});
    for (schema, table) in tables {
        let quoted = format!(
            "\"{}\".\"{}\"",
            schema.replace('"', "\"\""),
            table.replace('"', "\"\"")
        );
        let rows:Value=sqlx::query_scalar(&format!("SELECT coalesce(jsonb_agg(to_jsonb(t) ORDER BY to_jsonb(t)::text),'[]'::jsonb) FROM {quoted} t"))
            .fetch_one(pool).await.unwrap();
        result[format!("{schema}.{table}")] = rows;
    }
    if lifecycle {
        result["audit"]=sqlx::query_scalar::<_,Value>("SELECT coalesce(jsonb_agg(to_jsonb(a) ORDER BY id),'[]'::jsonb) FROM core_identity.audit a WHERE action LIKE 'channel.%'")
            .fetch_one(pool).await.unwrap();
    }
    result
}

#[sqlx::test(migrations = false)]
async fn native_and_legacy_copies_snapshot_counters_without_replaying_money(pool: PgPool) {
    let f = Fixture::new(pool).await;
    f.settle_source().await;
    let source = f.counters(1).await;
    assert_eq!(source["native"]["gross_settled_credits"], "41");
    assert_eq!(source["native"]["refunded_credits"], "5");
    assert_eq!(source["combined"]["net_used_credits"], "986");
    assert_eq!(source["complete"], true);
    let money = snapshot(&f.owner, false).await;
    let stats = f.stats().await;
    assert_eq!(stats["net_consumption_credits"], 36);
    let mut copies = Vec::new();
    for (legacy_value, reset) in [
        (None, false),
        (None, true),
        (Some("false"), false),
        (Some("true"), true),
        (Some("0"), false),
        (Some("1"), true),
    ] {
        let id = if let Some(value) = legacy_value {
            f.ok(
                "POST",
                &format!("/api/channel/copy/1?reset_balance={value}&suffix=_legacy"),
                Value::Null,
            )
            .await["id"]
                .as_u64()
                .unwrap()
        } else {
            f.copy(false, reset).await
        };
        let copied = f.counters(id).await;
        assert_eq!(copied["native"]["settled_requests"], "0");
        assert_eq!(copied["native"]["gross_settled_credits"], "0");
        assert_eq!(copied["native"]["refunded_credits"], "0");
        assert_eq!(
            copied["balance"]["balance"],
            if reset { "0" } else { "-12.125" }
        );
        assert_eq!(
            copied["balance"]["kind"],
            if reset { "reset" } else { "copied" }
        );
        assert_eq!(
            copied["balance"]["unit_label"],
            source["balance"]["unit_label"]
        );
        assert_eq!(copied["balance"]["balance_updated_time"], 1700000000_i64);
        assert_eq!(copied["balance"]["source_channel_id"], 1);
        // Attribution belongs to the preserved provider observation, not
        // the new key allocated for this channel copy.
        let raw:(Option<i64>,Option<i64>,Option<f64>,Option<i64>)=sqlx::query_as("SELECT credential_id,credential_revision,balance,floor(extract(epoch FROM observed_at))::bigint FROM core_channels.balance_observations WHERE channel_id=$1")
                .bind(id as i64).fetch_one(&f.owner).await.unwrap();
        assert_eq!(
            raw,
            (
                Some(771),
                Some(1),
                Some(if reset { 0.0 } else { -12.125 }),
                Some(1700000000)
            )
        );

        assert!(copied["balance"]["recorded_at"].as_i64().unwrap() > 1700000100);
        assert_eq!(
            copied["inherited"]["gross_settled_credits"],
            if reset { "0" } else { "1041" }
        );
        assert_eq!(
            copied["inherited"]["refunded_credits"],
            if reset { "0" } else { "55" }
        );
        assert_eq!(
            copied["combined"]["net_used_credits"],
            if reset { "0" } else { "986" }
        );
        assert_eq!(copied["used_quota"], if reset { "0" } else { "986" });
        assert_eq!(copied["complete"], true);
        assert_eq!(
            snapshot(&f.owner, false).await,
            money,
            "copy wrote financial facts"
        );
        assert_eq!(
            f.stats().await,
            stats,
            "display baseline leaked into global stats"
        );
        let audit:Value=sqlx::query_scalar("SELECT target FROM core_identity.audit WHERE action='channel.copy' AND (target#>>'{change,id}')::bigint=$1")
                .bind(id as i64).fetch_one(&f.owner).await.unwrap();
        assert_eq!(
            audit["change"],
            json!({"source_id":1,"id":id,"reset_balance":reset})
        );
        copies.push((id, copied));
    }
    assert_eq!(f.counters(1).await, source);
    // Chaining a copy inherits the combined display once, never replays its
    // source's native requests, and retains the original supplier timestamp.
    let parent = copies[0].0;
    let child = f
        .ok(
            "POST",
            &format!("/core/v1/admin/routing/channels/{parent}/copy"),
            json!({"expected_version":f.version().await,"suffix":"_chain","reset_balance":false}),
        )
        .await["id"]
        .as_u64()
        .unwrap();
    let child_view = f.counters(child).await;
    assert_eq!(child_view["combined"]["net_used_credits"], "986");
    assert_eq!(child_view["native"]["settled_requests"], "0");
    assert_eq!(child_view["balance"]["source_channel_id"], parent);
    assert_eq!(
        child_view["balance"]["balance_updated_time"],
        1700000000_i64
    );
    assert_eq!(snapshot(&f.owner, false).await, money);
    assert_eq!(f.stats().await, stats);
    copies.push((child, child_view));
    f.charging
        .refund(
            "copy-source-charge",
            "later-refund",
            10,
            "synthetic later source adjustment",
        )
        .await
        .unwrap();
    assert_eq!(f.counters(1).await["combined"]["net_used_credits"], "976");
    for (id, copied) in copies {
        assert_eq!(
            f.counters(id).await,
            copied,
            "later source refund changed copy {id}"
        );
    }
    assert_eq!(f.stats().await["net_consumption_credits"], 26);
    f.finish().await;
}

#[sqlx::test(migrations = false)]
async fn deleted_maximum_is_permanent_and_all_create_paths_reject_reuse(pool: PgPool) {
    let f = Fixture::new(pool).await;
    let removed = f.copy(false, true).await;
    let previous = f
        .ok("GET", "/core/v1/admin/routing/configuration", Value::Null)
        .await;
    f.ok(
        "DELETE",
        &format!("/api/channel/{removed}"),
        json!({"expected_version":f.version().await}),
    )
    .await;
    let retired: bool = sqlx::query_scalar(
        "SELECT retired_at IS NOT NULL FROM core_channels.identities WHERE id=$1",
    )
    .bind(removed as i64)
    .fetch_one(&f.owner)
    .await
    .unwrap();
    assert!(retired);
    let before_hint = snapshot(&f.owner, true).await;
    let configuration = f
        .ok("GET", "/core/v1/admin/routing/configuration", Value::Null)
        .await;
    assert_eq!(configuration["channel_id_high_water"], json!(removed));
    assert_eq!(
        snapshot(&f.owner, true).await,
        before_hint,
        "ID hint read changed stored state"
    );
    let next = f.copy(true, true).await;
    assert!(next > removed, "deleted maximum identity was recycled");
    let before = snapshot(&f.owner, true).await;
    let version = f.version().await;
    let (status, result) = request(
        f.app.clone(),
        "POST",
        "/core/v1/admin/routing/channels",
        &f.admin,
        channel_input(version, removed, "synthetic-must-roll-back"),
    )
    .await;
    assert_eq!(status, StatusCode::CONFLICT, "{result}");
    assert_eq!(result["code"], "CHANNEL_IDENTITY_RETIRED");
    assert_eq!(
        snapshot(&f.owner, true).await,
        before,
        "rejected create left key/identity/publication changes"
    );
    // Full configuration publication must enforce the same permanent identity rule.
    let mut input = f
        .ok("GET", "/core/v1/admin/routing/configuration", Value::Null)
        .await;
    let old = previous["routing"]["upstreams"]
        .as_array()
        .unwrap()
        .iter()
        .find(|u| u["id"] == removed)
        .unwrap()
        .clone();
    input["routing"]["upstreams"]
        .as_array_mut()
        .unwrap()
        .push(old);
    input["routing"]["version"] = json!(version.routing + 1);
    let body = json!({"expected_version":version,"routing":input["routing"],"prices":input["prices"],"bindings":input["bindings"]});
    let preview = f
        .ok(
            "POST",
            "/core/v1/admin/routing/configuration/preview",
            body.clone(),
        )
        .await;
    let mut body = body;
    body["confirmation_digest"] = preview["confirmation_digest"].clone();
    let (status, result) = request(
        f.app.clone(),
        "PUT",
        "/core/v1/admin/routing/configuration",
        &f.admin,
        body,
    )
    .await;
    assert_eq!(status, StatusCode::CONFLICT, "{result}");
    assert_eq!(snapshot(&f.owner, true).await, before);
    f.finish().await;
}

#[sqlx::test(migrations = false)]
async fn separate_managers_same_cas_publish_one_copy_and_no_orphans(pool: PgPool) {
    let f = Fixture::new(pool).await;
    let second_gateway = gateway(&f.role.pool)
        .with_channel_secret_key(&[83; 32])
        .unwrap();
    let second = managed_router(
        f.role.pool.clone(),
        ManagedChannels::recover(f.role.pool.clone(), second_gateway.clone())
            .await
            .unwrap(),
    );
    let before = snapshot(&f.owner, false).await;
    let expected = f.version().await;
    let input = json!({"expected_version":expected,"suffix":"_race","reset_balance":false});
    let (a, b) = tokio::join!(
        request(f.app.clone(), "POST", COPY, &f.admin, input.clone()),
        request(second.clone(), "POST", COPY, &f.admin, input)
    );
    assert_eq!(
        [a.0 == StatusCode::OK, b.0 == StatusCode::OK]
            .into_iter()
            .filter(|ok| *ok)
            .count(),
        1,
        "{a:?}, {b:?}"
    );
    let loser = if a.0 == StatusCode::OK { &b } else { &a };
    assert_eq!(loser.0, StatusCode::CONFLICT, "{loser:?}");
    let winner = if a.0 == StatusCode::OK { a.1 } else { b.1 };
    let id = winner["data"]["id"].as_u64().unwrap();
    assert_eq!(f.version().await.routing, expected.routing + 1);
    for table in [
        "core_channels.identities",
        "core_channels.counter_baselines",
        "core_channels.balance_observations",
    ] {
        let count: i64 = sqlx::query_scalar(&format!("SELECT count(*) FROM {table}"))
            .fetch_one(&f.owner)
            .await
            .unwrap();
        assert_eq!(count, 2, "orphan or missing row in {table}");
    }
    let keys: i64 = sqlx::query_scalar("SELECT count(*) FROM public.channel_secret_revisions")
        .fetch_one(&f.owner)
        .await
        .unwrap();
    let audits: i64 =
        sqlx::query_scalar("SELECT count(*) FROM core_identity.audit WHERE action='channel.copy'")
            .fetch_one(&f.owner)
            .await
            .unwrap();
    assert_eq!((keys, audits), (1, 1));
    assert_eq!(f.counters(id).await["balance"]["balance"], "-12.125");
    assert_eq!(snapshot(&f.owner, false).await, before);
    drop(second);
    drop(second_gateway);
    f.finish().await;
}

#[sqlx::test(migrations = false)]
async fn copy_audit_failure_rolls_back_display_identity_key_and_publication(pool: PgPool) {
    let f = Fixture::new(pool).await;
    f.settle_source().await;
    let before = snapshot(&f.owner, true).await;
    let version = f.version().await;
    sqlx::raw_sql("CREATE FUNCTION public.reject_counter_copy_audit() RETURNS trigger LANGUAGE plpgsql AS $$ BEGIN IF NEW.action='channel.copy' THEN RAISE EXCEPTION 'synthetic audit rejection'; END IF; RETURN NEW; END $$; CREATE TRIGGER reject_counter_copy_audit BEFORE INSERT ON core_identity.audit FOR EACH ROW EXECUTE FUNCTION public.reject_counter_copy_audit()")
        .execute(&f.owner).await.unwrap();
    for legacy in [false, true] {
        for reset in [false, true] {
            let path = if legacy {
                format!("/api/channel/copy/1?reset_balance={reset}")
            } else {
                COPY.into()
            };
            let (status, result) = request(
                f.app.clone(),
                "POST",
                &path,
                &f.admin,
                if legacy {
                    Value::Null
                } else {
                    json!({"expected_version":version,"suffix":"_fail","reset_balance":reset})
                },
            )
            .await;
            assert_eq!(status, StatusCode::SERVICE_UNAVAILABLE, "{result}");
            assert_eq!(snapshot(&f.owner, true).await, before);
            assert_eq!(
                Version::from(f.gateway.dashboard_routing().version().unwrap()),
                version
            );
        }
    }
    sqlx::raw_sql("DROP TRIGGER reject_counter_copy_audit ON core_identity.audit; DROP FUNCTION public.reject_counter_copy_audit()")
        .execute(&f.owner).await.unwrap();
    let (status, result) = request(
        f.app.clone(),
        "POST",
        COPY,
        &f.ordinary,
        json!({"expected_version":version,"suffix":"_denied","reset_balance":false}),
    )
    .await;
    assert_eq!(status, StatusCode::FORBIDDEN, "{result}");
    assert_eq!(snapshot(&f.owner, true).await, before);
    let id = f.copy(false, false).await;
    assert_eq!(
        id, 2,
        "rolled-back ID must not become permanently allocated"
    );
    assert_eq!(f.counters(id).await["combined"]["net_used_credits"], "986");
    f.finish().await;
}

#[sqlx::test(migrations = false)]
async fn unpublished_bootstrap_counters_are_read_only_and_unknown(pool: PgPool) {
    let f = Fixture::unpublished(pool).await;
    let registered: bool =
        sqlx::query_scalar("SELECT EXISTS(SELECT FROM core_channels.identities WHERE id=1)")
            .fetch_one(&f.owner)
            .await
            .unwrap();
    assert!(
        !registered,
        "fixture must exercise a bootstrap-only channel"
    );
    let before = snapshot(&f.owner, true).await;
    let (status, body) = request(
        f.app.clone(),
        "GET",
        "/core/v1/admin/routing/channels/1/counters",
        &f.admin,
        Value::Null,
    )
    .await;
    assert_eq!(
        status,
        StatusCode::OK,
        "bootstrap-only counters must remain readable: {body}"
    );
    assert_eq!(
        body["data"]["balance"],
        json!({"balance":null,"unit_label":null,"balance_updated_time":null,
        "kind":"unknown","recorded_at":null,"source_channel_id":null})
    );
    assert_eq!(body["data"]["historical_coverage"], false);
    assert_eq!(body["data"]["complete"], false);
    assert_eq!(body["data"]["used_quota"], Value::Null);
    assert_eq!(
        snapshot(&f.owner, true).await,
        before,
        "GET registered permanent facts"
    );
    f.finish().await;
}

#[sqlx::test(migrations = false)]
async fn definitely_retired_identity_cannot_be_reactivated_as_current_bootstrap(pool: PgPool) {
    let f = Fixture::unpublished(pool).await;
    // Model definite prior retirement, established by deletion evidence or a
    // prior permanent identity. Ordinary receipt-only history without retirement
    // evidence remains nonretired/unknown and is covered separately below.
    // The current in-memory bootstrap deliberately attempts to reuse this ID.
    sqlx::query("INSERT INTO core_channels.identities(id,origin,retired_at) VALUES(1,'migrated',to_timestamp(1700000000))")
        .execute(&f.owner).await.unwrap();
    let before = snapshot(&f.owner, true).await;
    for reset in [false, true] {
        let (status,body)=request(f.app.clone(),"POST",COPY,&f.admin,
            json!({"expected_version":Version::of(&fixture()),"suffix":"_retired","reset_balance":reset})).await;
        assert_eq!(
            status,
            StatusCode::CONFLICT,
            "retired bootstrap identity was accepted as a copy source: {body}"
        );
        assert_eq!(snapshot(&f.owner, true).await, before);
    }
    let (status, body) = request(
        f.app.clone(),
        "PUT",
        "/api/channel/1/status",
        &f.admin,
        json!({"expected_version":Version::of(&fixture()),"enabled":true}),
    )
    .await;
    assert_eq!(
        status,
        StatusCode::CONFLICT,
        "publication preserved a retired identity as current: {body}"
    );
    assert_eq!(snapshot(&f.owner, true).await, before);
    f.finish().await;
}

#[sqlx::test(migrations = false)]
async fn nonretired_migrated_bootstrap_remains_usable_with_unknown_history(pool: PgPool) {
    let f = Fixture::unpublished(pool).await;
    sqlx::query("INSERT INTO core_channels.identities(id,origin) VALUES(1,'migrated')")
        .execute(&f.owner)
        .await
        .unwrap();
    let before = snapshot(&f.owner, true).await;
    let source = f.counters(1).await;
    assert_eq!(source["balance"]["kind"], "unknown");
    assert_eq!(source["balance"]["recorded_at"], Value::Null);
    assert_eq!(source["complete"], false);
    assert_eq!(snapshot(&f.owner, true).await, before);
    let copied=f.ok("POST",COPY,json!({"expected_version":Version::of(&fixture()),"suffix":"_unknown","reset_balance":false})).await["id"].as_u64().unwrap();
    let counters = f.counters(copied).await;
    assert_eq!(counters["balance"]["balance"], Value::Null);
    assert_eq!(counters["balance"]["kind"], "copied");
    assert_eq!(counters["balance"]["balance_updated_time"], Value::Null);
    assert_eq!(counters["complete"], false);
    assert_eq!(counters["used_quota"], Value::Null);
    let reset = f.copy(true, true).await;
    assert_eq!(f.counters(reset).await["combined"]["net_used_credits"], "0");
    assert_eq!(f.counters(reset).await["complete"], true);
    f.finish().await;
}

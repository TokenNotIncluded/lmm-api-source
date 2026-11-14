use super::{SCHEMA, batch, digest, display, exact, record};
// Component46 business acceptance: real loopback HTTP, disposable PG,
// restricted runtime roles and real router handlers. This is not full TLS proof;
// published routing stays HTTPS and only the cfg(test) transport is mapped.
use crate::test_runtime as runtime;
use crate::{
    channel_http::{ManagedChannels, managed_router},
    identity::IdentityStore,
    model_http::{ChargePolicy, ModelGateway, ProviderPolicy, UpstreamCredential},
    relay::Limits,
    routing::{
        CredentialRef, PriceConfig, PriceSnapshot, Router as ServingRouter, RoutingConfig,
        store::ReleaseDocument,
    },
};
use axum::{
    Router,
    body::{Body, to_bytes},
    http::{Request, StatusCode, header},
};
use serde_json::{Value, json};
use sqlx::PgPool;
use std::{collections::BTreeMap, sync::Arc, time::Duration};
use tokio::{net::TcpListener, sync::RwLock};
use tower::ServiceExt;
fn fixture() -> ReleaseDocument {
    let mut config:RoutingConfig=serde_json::from_value(json!({"version":1,"groups":[{"name":"default","enabled":true}],
      "upstreams":[{"id":1,"endpoint":"https://provider.example/v1/","protocol":"openai_chat","credential":{"id":771,"revision":1},"enabled":true}],
      "routes":[{"id":1,"model":"model-a","group":"default","enabled":true,"speeds":["standard"],
       "targets":[{"id":1,"upstream_id":1,"upstream_model":"model-a","enabled":true,"priority":0,"weight":1,"speeds":["standard"],"payload":"adapt","timeout_ms":30000}],
       "failover":{"max_attempts":1,"retry_on":[],"allow_replay_after_send":false}}],"aliases":[]})).unwrap();
    for id in 2..=3 {
        let mut upstream = config.upstreams[0].clone();
        upstream.id = id;
        config.upstreams.push(upstream);
    }
    let prices:PriceConfig=serde_json::from_value(json!({"version":1,"rules":[{"id":1,"model":"model-a","group":"default","speed":"standard",
      "rates":{"input":2,"output":4,"cache_read":0,"cache_write":0,"request":0},"group_multiplier_ppm":1000000,"speed_multiplier_ppm":1000000}]})).unwrap();
    ReleaseDocument {
        contract_version: 1,
        routing: config,
        pricing: prices,
    }
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
    let mut contexts:Vec<crate::model_http::ExternalModelContext>=serde_json::from_value(json!([{"upstream_id":1,"upstream_model":"model-a","protocol":"openai_chat","endpoint":"https://provider.example/v1/","max_input_tokens":1000,"max_output_tokens":100,"contract_revision":"synthetic-v1","missing_cached_tokens_is_zero":true,"missing_cache_creation_tokens_is_zero":true}])).unwrap();
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
    let gateway = ModelGateway::new_with_provider_policy(
        pool.clone(),
        routing,
        credentials,
        BTreeMap::new(),
        Limits {
            total_timeout: Duration::from_secs(5),
            max_output_tokens: 100,
            ..Limits::default()
        },
        ChargePolicy {
            input_token_ceiling: 1000,
            lease_seconds: 60,
        },
        ProviderPolicy::new(contexts, 1000).unwrap(),
    )
    .unwrap();
    match loopback {
        Some(endpoint) => gateway
            .with_local_streaming()
            .with_balance_probe_loopback("https://provider.example", endpoint)
            .unwrap(),
        None => gateway,
    }
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

async fn initialize(pool: &PgPool) {
    crate::core_schema::init(pool).await.unwrap();
    let installed: bool =
        sqlx::query_scalar("SELECT to_regclass('core_channels.catalog_observations') IS NOT NULL")
            .fetch_one(pool)
            .await
            .unwrap();
    // Only the isolated test owner installs the not-yet-registered component46.
    if !installed {
        sqlx::raw_sql(SCHEMA).execute(pool).await.unwrap();
    }
    sqlx::raw_sql("INSERT INTO core_channels.identities(id,origin) SELECT n,'bootstrap' FROM generate_series(1,3) n; INSERT INTO core_channels.counter_baselines(channel_id,historical_coverage) VALUES(1,true),(2,true),(3,false);").execute(pool).await.unwrap();
}
async fn restricted(pool: &PgPool) -> runtime::RuntimeRole {
    let role = runtime::RuntimeRole::create(pool).await;
    role.optional("core_charging").await;
    role.optional("core_model").await;
    sqlx::raw_sql(sqlx::AssertSqlSafe(format!("GRANT EXECUTE ON FUNCTION public.routing_control_balance_lock() TO {0}; GRANT SELECT(channel_id,credential_id,credential_revision,binding_sha256,scope,result,started_at,observed_at,duration_ms), INSERT(channel_id,credential_id,credential_revision,binding_sha256,scope,result,started_at,observed_at,duration_ms), UPDATE(credential_id,credential_revision,binding_sha256,scope,result,started_at,observed_at,duration_ms) ON core_channels.catalog_observations TO {0};", role.name))).execute(pool).await.unwrap();
    let forbidden: bool = sqlx::query_scalar("SELECT has_table_privilege('core_channels.catalog_observations','UPDATE,DELETE,TRUNCATE') OR has_column_privilege('core_channels.catalog_observations','channel_id','UPDATE')").fetch_one(&role.pool).await.unwrap();
    assert!(!forbidden);
    role
}
#[test]
fn exact_display_and_digest_include_generation_but_not_page_number() {
    assert_eq!(
        display(&exact("-1", "3").unwrap_or_else(|_| panic!("rational"))),
        "-0.333333333333333333"
    );
    assert!(exact("1", "0").is_err());
    let document = fixture();
    let bindings = super::Bindings::default();
    let mut stats = BTreeMap::new();
    stats.insert(1, super::Statistics::default());
    let mut params = crate::channel_http::Params::default();
    let first = digest(&document, &bindings, &stats, &params).unwrap_or_else(|_| panic!("digest"));
    params.p = Some(2);
    params.snapshot_digest = Some(first.clone());
    assert_eq!(
        first,
        digest(&document, &bindings, &stats, &params).unwrap_or_else(|_| panic!("digest"))
    );
    stats.get_mut(&1).unwrap().generation = Some("new subsecond observation".into());
    assert_ne!(
        first,
        digest(&document, &bindings, &stats, &params).unwrap_or_else(|_| panic!("digest"))
    );
}
#[sqlx::test(migrations = false)]
async fn batch_exact_sort_units_digest_and_repeatable_read(pool: PgPool) {
    initialize(&pool).await;
    let admin = IdentityStore::from_pool(pool.clone())
        .bootstrap_user(8, 6)
        .await
        .unwrap();
    sqlx::raw_sql("INSERT INTO core_channels.usage_rollups(channel_id,day,bucket,gross_credits) VALUES(1,-1,0,9007199254740993),(2,-1,0,9007199254740994); INSERT INTO core_channels.balance_observations(channel_id,balance,unit_label,observed_at,kind,credential_id,credential_revision) VALUES(1,5,'USD',clock_timestamp(),'provider',771,1),(2,100,'CNY',clock_timestamp(),'provider',771,1);").execute(&pool).await.unwrap();
    let role = restricted(&pool).await;
    let manager =
        ManagedChannels::recover(role.pool.clone(), gateway_with_loopback(&role.pool, None))
            .await
            .unwrap();
    let app = managed_router(role.pool.clone(), manager);
    let first_path = "/api/channel?statistics=true&sort_by=lifetime&sort_order=desc&page_size=1";
    let (status, first) = request(app.clone(), "GET", first_path, &admin.secret, Value::Null).await;
    assert_eq!(status, StatusCode::OK, "{first}");
    assert_eq!(first["data"]["items"][0]["id"], 2);
    assert_eq!(
        first["data"]["items"][0]["lifetime"]["exact_usd_numerator"],
        "9007199254740994"
    );
    assert_eq!(first["data"]["pagination_consistency"], "single_request");
    let digest = first["data"]["snapshot_digest"].as_str().unwrap();
    let next = format!("{first_path}&p=2&snapshot_digest={digest}");
    let (status, second) = request(app.clone(), "GET", &next, &admin.secret, Value::Null).await;
    assert_eq!(status, StatusCode::OK, "{second}");
    assert_eq!(second["data"]["items"][0]["id"], 1);
    assert_eq!(second["data"]["pagination_consistency"], "digest_checked");
    let (_, missing) = request(
        app.clone(),
        "GET",
        "/api/channel?sort_by=balance",
        &admin.secret,
        Value::Null,
    )
    .await;
    assert_eq!(missing["code"], "CHANNEL_BALANCE_UNIT_REQUIRED");
    assert_eq!(missing["data"]["balance_units"], json!(["CNY", "USD"]));
    let (status, selected) = request(
        app.clone(),
        "GET",
        "/api/channel?sort_by=balance&sort_unit=USD&sort_order=desc",
        &admin.secret,
        Value::Null,
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{selected}");
    assert_eq!(selected["data"]["sort_unit"], "USD");
    assert_eq!(selected["data"]["items"][0]["id"], 1);
    assert_eq!(selected["data"]["items"][2]["lifetime"]["complete"], false);
    let mut tx = role.pool.begin().await.unwrap();
    sqlx::query("SET TRANSACTION ISOLATION LEVEL REPEATABLE READ")
        .execute(&mut *tx)
        .await
        .unwrap();
    let routing = fixture().routing;
    let before = batch(&mut tx, &routing)
        .await
        .unwrap_or_else(|_| panic!("batch before"));
    sqlx::query("UPDATE core_channels.balance_observations SET balance=8,observed_at=clock_timestamp() WHERE channel_id=1").execute(&pool).await.unwrap();
    let same = batch(&mut tx, &routing)
        .await
        .unwrap_or_else(|_| panic!("snapshot batch"));
    assert_eq!(
        serde_json::to_value(before).unwrap(),
        serde_json::to_value(same).unwrap()
    );
    tx.commit().await.unwrap();
    let (status, conflict) = request(app.clone(), "GET", &next, &admin.secret, Value::Null).await;
    assert_eq!(status, StatusCode::CONFLICT, "{conflict}");
    assert_eq!(conflict["code"], "CHANNEL_SNAPSHOT_CHANGED");
    // Legacy clients are explicitly only single-request consistent.
    assert_eq!(
        request(
            app.clone(),
            "GET",
            &format!("{first_path}&p=2"),
            &admin.secret,
            Value::Null
        )
        .await
        .0,
        StatusCode::OK
    );
    sqlx::query("UPDATE core_channels.projection_state SET ready=false")
        .execute(&pool)
        .await
        .unwrap();
    assert_eq!(
        request(app, "GET", first_path, &admin.secret, Value::Null)
            .await
            .0,
        StatusCode::SERVICE_UNAVAILABLE
    );
}
#[sqlx::test(migrations = false)]
async fn catalogue_loopback_failure_binding_copy_retirement_and_no_publication(pool: PgPool) {
    initialize(&pool).await;
    let identity = IdentityStore::from_pool(pool.clone());
    let admin = identity.bootstrap_user(8, 6).await.unwrap();
    let ordinary = identity.bootstrap_user(7, 1).await.unwrap();
    let response = Arc::new(RwLock::new(json!({"data":[{"id":"model-a"}]})));
    let body = response.clone();
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let endpoint = format!("http://{}", listener.local_addr().unwrap());
    let provider = Router::new().route(
        "/v1/models",
        axum::routing::get(move || {
            let body = body.clone();
            async move { axum::Json(body.read().await.clone()) }
        }),
    );
    let server = tokio::spawn(async move { axum::serve(listener, provider).await.unwrap() });
    let role = restricted(&pool).await;
    let gateway = gateway_with_loopback(&role.pool, Some(&endpoint))
        .with_channel_secret_key(&[83; 32])
        .unwrap();
    let manager = ManagedChannels::recover(role.pool.clone(), gateway)
        .await
        .unwrap();
    let app = managed_router(role.pool.clone(), manager);
    let release_before: Option<Value> = sqlx::query_scalar("SELECT public.routing_control_read()")
        .fetch_one(&pool)
        .await
        .unwrap();
    let ledger_before: i64 =
        sqlx::query_scalar("SELECT count(*) FROM core_billing.ledger_journals")
            .fetch_one(&pool)
            .await
            .unwrap();
    let probe = "/api/channel/test/1";
    assert_eq!(
        request(app.clone(), "GET", probe, &ordinary.secret, Value::Null)
            .await
            .0,
        StatusCode::FORBIDDEN
    );
    let (status, result) = request(app.clone(), "GET", probe, &admin.secret, Value::Null).await;
    assert_eq!(status, StatusCode::OK, "{result}");
    let mut c = role.pool.acquire().await.unwrap();
    let routing = fixture().routing;
    let stats = batch(&mut c, &routing)
        .await
        .unwrap_or_else(|_| panic!("batch"));
    assert!(stats[&1].response_time.is_some());
    assert_eq!(
        stats[&1].catalog_observation.as_ref().unwrap().scope,
        "model_catalog"
    );
    assert!(
        !stats[&1]
            .catalog_observation
            .as_ref()
            .unwrap()
            .inference_tested
    );
    assert!(
        stats[&2].response_time.is_none(),
        "no imported zero or copied success"
    );
    let mut changed = routing.clone();
    changed.upstreams[0].credential.revision += 1;
    assert!(
        batch(&mut c, &changed)
            .await
            .unwrap_or_else(|_| panic!("key changed"))[&1]
            .catalog_observation
            .is_none()
    );
    changed = routing.clone();
    changed.upstreams[0].endpoint = "https://different.example/v1/"
        .to_string()
        .try_into()
        .unwrap();
    assert!(
        batch(&mut c, &changed)
            .await
            .unwrap_or_else(|_| panic!("endpoint changed"))[&1]
            .catalog_observation
            .is_none()
    );
    *response.write().await = json!({"not_models":true});
    assert!(
        !request(app.clone(), "GET", probe, &admin.secret, Value::Null)
            .await
            .0
            .is_success()
    );
    let failed = batch(&mut c, &routing)
        .await
        .unwrap_or_else(|_| panic!("failed observation"));
    assert!(failed[&1].response_time.is_none());
    assert!(failed[&1].test_time.is_some());
    assert_eq!(
        failed[&1].catalog_observation.as_ref().unwrap().result,
        "failed"
    );
    // An older completion cannot restore the former success.
    record(
        &mut c,
        &routing.upstreams[0],
        chrono::Utc::now() - chrono::Duration::days(1),
        true,
        1,
    )
    .await
    .unwrap_or_else(|_| panic!("old attempt"));
    assert_eq!(
        batch(&mut c, &routing)
            .await
            .unwrap_or_else(|_| panic!("batch"))[&1]
            .catalog_observation
            .as_ref()
            .unwrap()
            .result,
        "failed"
    );
    let release_after: Option<Value> = sqlx::query_scalar("SELECT public.routing_control_read()")
        .fetch_one(&pool)
        .await
        .unwrap();
    let ledger_after: i64 = sqlx::query_scalar("SELECT count(*) FROM core_billing.ledger_journals")
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(ledger_before, ledger_after);
    assert_eq!(
        release_before, release_after,
        "observations must not publish routing or prices"
    );
    let (status, copied) = request(
        app.clone(),
        "POST",
        "/core/v1/admin/routing/channels/1/copy",
        &admin.secret,
        json!({"expected_version":{"routing":1,"pricing":1},"suffix":"_copy","reset_balance":true}),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{copied}");
    let copied_id = copied["data"]["id"].as_i64().unwrap();
    let copied_probe: bool = sqlx::query_scalar(
        "SELECT EXISTS(SELECT FROM core_channels.catalog_observations WHERE channel_id=$1)",
    )
    .bind(copied_id)
    .fetch_one(&pool)
    .await
    .unwrap();
    assert!(
        !copied_probe,
        "copy never invents a fresh catalogue attempt"
    );
    sqlx::query("UPDATE core_channels.identities SET retired_at=clock_timestamp() WHERE id=1")
        .execute(&pool)
        .await
        .unwrap();
    assert!(
        batch(&mut c, &routing)
            .await
            .unwrap_or_else(|_| panic!("retired"))[&1]
            .catalog_observation
            .is_none()
    );
    assert!(
        !request(app, "GET", probe, &admin.secret, Value::Null)
            .await
            .0
            .is_success()
    );
    server.abort();
}

#[sqlx::test(migrations = false)]
async fn reviewed_historical_fractions_add_net_once_and_unknown_invalidates_totals(pool: PgPool) {
    initialize(&pool).await;
    sqlx::raw_sql("INSERT INTO core_channels.opening_sources(source_key,archive_sha256,export_sha256,mapping_sha256,unit_evidence_sha256,source_revision,observed_at,native_overlap,native_request_ids,native_seal_sha256,rows_sha256,plan_sha256) VALUES(repeat('a',64),repeat('b',64),repeat('c',64),repeat('d',64),repeat('e',64),'synthetic-reviewed',123,'disjoint',ARRAY[]::text[],repeat('f',64),repeat('1',64),repeat('2',64)); INSERT INTO core_channels.opening_baselines(channel_id,source_key,source_channel_id,raw_used_quota,credits_per_usd,offset_numerator,offset_denominator,historical_coverage,kind) VALUES(1,repeat('a',64),1,-1,3,-1,3,true,'imported'),(2,repeat('a',64),2,1,3,1,3,true,'imported'),(3,repeat('a',64),3,20,1,20,1,false,'imported'); INSERT INTO core_channels.usage_rollups(channel_id,day,bucket,gross_credits,refunded_credits) VALUES(1,-1,0,500000,100000);").execute(&pool).await.unwrap();
    let role = restricted(&pool).await;
    let mut c = role.pool.acquire().await.unwrap();
    let routing = fixture().routing;
    let stats = batch(&mut c, &routing)
        .await
        .unwrap_or_else(|_| panic!("historical batch"));
    assert_eq!(
        stats[&1].lifetime.total_usd.as_deref(),
        Some("0.466666666666666667")
    );
    assert_eq!(
        stats[&1].lifetime.exact_usd_numerator.as_deref(),
        Some("700000")
    );
    assert_eq!(
        stats[&1].lifetime.exact_usd_denominator.as_deref(),
        Some("1500000")
    );
    assert_eq!(stats[&1].lifetime.historical_observed_at, Some(123));
    assert!(!stats[&3].lifetime.complete);
    assert!(stats[&3].amount.is_none());
    sqlx::query("INSERT INTO core_channels.usage_rollups(channel_id,day,bucket,unknown_requests) VALUES(0,-1,0,1)").execute(&pool).await.unwrap();
    assert!(
        batch(&mut c, &routing)
            .await
            .unwrap_or_else(|_| panic!("global unknown"))
            .values()
            .all(|s| !s.lifetime.complete && s.amount.is_none())
    );
}

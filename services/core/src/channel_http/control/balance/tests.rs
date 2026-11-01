//! Synthetic-provider business acceptance: real loopback HTTP, disposable PG,
//! restricted runtime roles and real router handlers. This is not full TLS proof;
//! published routing stays HTTPS and only the cfg(test) transport is mapped.
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
use std::{
    collections::BTreeMap,
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
    time::Duration,
};
use tokio::{
    net::TcpListener,
    sync::{Notify, RwLock, Semaphore},
};
use tower::ServiceExt;
const BALANCE: &str = "/core/v1/admin/routing/channels/1/balance";
const CONFIG: &str = "/core/v1/admin/routing/channels/1/balance-config";
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

struct Provider {
    body: RwLock<String>,
    blocked: AtomicBool,
    started: Notify,
    release: Semaphore,
}
async fn provider(
    axum::extract::State(state): axum::extract::State<Arc<Provider>>,
    headers: axum::http::HeaderMap,
) -> impl axum::response::IntoResponse {
    assert_eq!(headers["authorization"], "Bearer synthetic-channel-only");
    if state.blocked.load(Ordering::SeqCst) {
        state.started.notify_one();
        state.release.acquire().await.unwrap().forget();
    }
    (
        [(header::CONTENT_TYPE, "application/json")],
        state.body.read().await.clone(),
    )
}
async fn row(pool: &PgPool) -> Value {
    sqlx::query_scalar(
        "SELECT to_jsonb(o) FROM core_channels.balance_observations o WHERE channel_id=1",
    )
    .fetch_one(pool)
    .await
    .unwrap()
}
async fn configure(
    app: &Router,
    admin: &str,
    version: &Value,
    pointer: &str,
    unit: Value,
) -> Value {
    let (status, body) = request(
        app.clone(),
        "PUT",
        CONFIG,
        admin,
        json!({"expected_version":version,
        "probe":{"path":"/balance","json_pointer":pointer,"unit_label":unit}}),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    body["data"]["version"].clone()
}

#[sqlx::test(migrations = false)]
async fn get_balance_preserves_observation_on_provider_auth_and_cas_failure(pool: PgPool) {
    crate::core_schema::init(&pool).await.unwrap();
    let identity = IdentityStore::from_pool(pool.clone());
    let admin = identity.bootstrap_user(8, 6).await.unwrap();
    let ordinary = identity.bootstrap_user(7, 1).await.unwrap();
    let provider_state = Arc::new(Provider {
        body: RwLock::new(json!({"a/b":{"~balance":"12.125"}}).to_string()),
        blocked: AtomicBool::new(false),
        started: Notify::new(),
        release: Semaphore::new(0),
    });
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let endpoint = format!("http://{}", listener.local_addr().unwrap());
    let upstream = Router::new()
        .route("/balance", axum::routing::get(provider))
        .with_state(provider_state.clone());
    let server = tokio::spawn(async move { axum::serve(listener, upstream).await.unwrap() });
    let role = runtime::RuntimeRole::create(&pool).await;
    role.optional("core_charging").await;
    role.optional("core_model").await;
    sqlx::raw_sql(&format!(
        "GRANT EXECUTE ON FUNCTION public.routing_control_balance_lock() TO {}",
        role.name
    ))
    .execute(&pool)
    .await
    .unwrap();
    // Transport mapping requires an explicit host local-mode opt-in and cannot
    // name private-network destinations. Routing configuration cannot enable it.
    assert!(
        gateway_with_loopback(&role.pool, None)
            .with_balance_probe_loopback("https://provider.example", &endpoint)
            .is_err()
    );
    let gateway = gateway_with_loopback(&role.pool, Some(&endpoint));
    let management = ManagedChannels::recover(role.pool.clone(), gateway.clone())
        .await
        .unwrap();
    let app = managed_router(role.pool.clone(), management);
    assert_eq!(
        request(app.clone(), "GET", BALANCE, &admin.secret, Value::Null)
            .await
            .0,
        StatusCode::NOT_IMPLEMENTED
    );
    assert_eq!(
        request(app.clone(), "GET", BALANCE, &ordinary.secret, Value::Null)
            .await
            .0,
        StatusCode::FORBIDDEN
    );
    let mut version = json!({"routing":1,"pricing":1});
    for invalid in [
        json!({"path":"//evil.invalid/balance","json_pointer":"/x"}),
        json!({"path":"https://evil.invalid/balance","json_pointer":"/x"}),
        json!({"path":"/balance?key=bad","json_pointer":"/x"}),
        json!({"path":"/balance#bad","json_pointer":"/x"}),
        json!({"path":"/balance\n","json_pointer":"/x"}),
        json!({"path":"/balance","json_pointer":""}),
        json!({"path":"/balance","json_pointer":"x"}),
        json!({"path":"/balance","json_pointer":"/bad~2"}),
        json!({"path":"/balance","json_pointer":"/bad~"}),
        json!({"path":"/balance","json_pointer":"/x","unit_label":"x".repeat(33)}),
        json!({"path":"/balance","json_pointer":"/x","unit_label":"x\n"}),
    ] {
        let (status, body) = request(
            app.clone(),
            "PUT",
            CONFIG,
            &admin.secret,
            json!({"expected_version":version,"probe":invalid}),
        )
        .await;
        assert_eq!(status, StatusCode::BAD_REQUEST, "{body}");
    }
    version = configure(
        &app,
        &admin.secret,
        &version,
        "/a~1b/~0balance",
        json!("credits"),
    )
    .await;
    let persisted_endpoint:String=sqlx::query_scalar("SELECT u.endpoint FROM public.routing_upstreams u JOIN public.routing_active_release a ON a.config_version=u.version WHERE u.id=1")
        .fetch_one(&pool).await.unwrap();
    assert_eq!(persisted_endpoint, "https://provider.example/v1/");
    for (raw, expected) in [
        (json!("12.125"), "12.125"),
        (json!(0), "0"),
        (json!(-2.75), "-2.75"),
    ] {
        *provider_state.body.write().await = json!({"a/b":{"~balance":raw}}).to_string();
        let (status, body) = request(app.clone(), "GET", BALANCE, &admin.secret, Value::Null).await;
        assert_eq!(status, StatusCode::OK, "{body}");
        assert_eq!(body["data"]["balance"], expected);
        assert_eq!(body["data"]["kind"], "provider");
        assert_eq!(body["data"]["unit_label"], "credits");
        assert!(body["data"]["balance_updated_time"].as_i64().unwrap() > 0);
        assert_eq!(body["data"]["source_channel_id"], Value::Null);
    }
    version = configure(
        &app,
        &admin.secret,
        &version,
        "/a~1b/~0balance",
        Value::Null,
    )
    .await;
    // Durable routing stays HTTPS-only; the explicit synthetic transport is
    // configured again by the host, never recovered from routing JSON.
    let restored = ManagedChannels::recover(
        role.pool.clone(),
        gateway_with_loopback(&role.pool, Some(&endpoint)),
    )
    .await
    .unwrap();
    drop(restored);
    let (status, body) = request(
        app.clone(),
        "GET",
        "/api/channel/update_balance/1",
        &admin.secret,
        Value::Null,
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(body["data"], -2.75);
    assert_eq!(body["observation"]["unit_label"], Value::Null);
    let saved = row(&pool).await;
    for invalid in [
        "not-json".to_owned(),
        json!({"x":1}).to_string(),
        json!({"a/b":{"~balance":"NaN"}}).to_string(),
        json!({"a/b":{"~balance":"1e999"}}).to_string(),
        json!({"a/b":{"~balance":true}}).to_string(),
        json!({"a/b":{"~balance":"synthetic-channel-only"}}).to_string(),
        "x".repeat(2 * 1024 * 1024),
    ] {
        *provider_state.body.write().await = invalid;
        let (status, body) = request(app.clone(), "GET", BALANCE, &admin.secret, Value::Null).await;
        assert_eq!(status, StatusCode::BAD_GATEWAY, "{body}");
        assert!(!body.to_string().contains("synthetic-channel-only"));
        assert_eq!(row(&pool).await, saved);
    }
    let (status, _) = request(
        app.clone(),
        "PUT",
        CONFIG,
        &admin.secret,
        json!({"expected_version":{"routing":1,"pricing":1},"probe":null}),
    )
    .await;
    assert_eq!(status, StatusCode::CONFLICT);
    assert_eq!(row(&pool).await, saved);
    let (status, body) = request(
        app.clone(),
        "PUT",
        CONFIG,
        &admin.secret,
        json!({"expected_version":version,"probe":null}),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    version = body["data"]["version"].clone();
    assert_eq!(
        request(app.clone(), "GET", BALANCE, &admin.secret, Value::Null)
            .await
            .0,
        StatusCode::NOT_IMPLEMENTED
    );
    assert_eq!(row(&pool).await, saved);
    version = configure(
        &app,
        &admin.secret,
        &version,
        "/a~1b/~0balance",
        Value::Null,
    )
    .await;
    *provider_state.body.write().await = json!({"a/b":{"~balance":999}}).to_string();
    provider_state.blocked.store(true, Ordering::SeqCst);
    let waiting_app = app.clone();
    let waiting_secret = admin.secret.clone();
    let waiting = tokio::spawn(async move {
        request(waiting_app, "GET", BALANCE, &waiting_secret, Value::Null).await
    });
    tokio::time::timeout(Duration::from_secs(3), provider_state.started.notified())
        .await
        .unwrap();
    // A configuration publication must complete while the provider GET is blocked.
    version = configure(
        &app,
        &admin.secret,
        &version,
        "/a~1b/~0balance",
        json!("new"),
    )
    .await;
    provider_state.release.add_permits(1);
    assert_eq!(waiting.await.unwrap().0, StatusCode::CONFLICT);
    assert_eq!(row(&pool).await, saved);

    // Reproduce the post-auth / pre-fence race across independent DB sessions.
    // The competing publication owns a ROW EXCLUSIVE table lock before the
    // provider finishes; the balance's final SHARE fence must actually wait.
    let mut publication = pool.begin().await.unwrap();
    sqlx::query("LOCK TABLE public.routing_price_versions IN ROW EXCLUSIVE MODE")
        .execute(&mut *publication)
        .await
        .unwrap();
    let waiting_app = app.clone();
    let waiting_secret = admin.secret.clone();
    let waiting = tokio::spawn(async move {
        request(waiting_app, "GET", BALANCE, &waiting_secret, Value::Null).await
    });
    tokio::time::timeout(Duration::from_secs(3), provider_state.started.notified())
        .await
        .unwrap();
    provider_state.release.add_permits(1);
    tokio::time::timeout(Duration::from_millis(350), async {
        loop {
            let waiting:bool=sqlx::query_scalar("SELECT EXISTS(SELECT FROM pg_locks l JOIN pg_stat_activity a ON a.pid=l.pid WHERE l.relation='public.routing_price_versions'::regclass AND l.mode='ShareLock' AND NOT l.granted AND a.usename=$1)")
                .bind(&role.name).fetch_one(&pool).await.unwrap();
            if waiting { break; }
            tokio::time::sleep(Duration::from_millis(5)).await;
        }
    }).await.expect("balance final transaction must reach the publication fence");
    let active: Value = sqlx::query_scalar("SELECT public.routing_control_read()")
        .fetch_one(&mut *publication)
        .await
        .unwrap();
    let mut bindings: Value = sqlx::query_scalar("SELECT public.routing_control_bindings($1)")
        .bind(version["routing"].as_i64().unwrap())
        .fetch_one(&mut *publication)
        .await
        .unwrap();
    let mut next_routing = active["routing"].clone();
    next_routing["version"] = json!(version["routing"].as_u64().unwrap() + 1);
    bindings["balance_probes"]["1"]["unit_label"] = json!("fence-publication");
    // Use the real durable CAS publication function, with another session's
    // transaction; no in-process publication mutex can serialize this test away.
    sqlx::query("SELECT public.routing_control_publish($1,$2,$3,$4,$5)")
        .bind(&next_routing)
        .bind(&active["pricing"])
        .bind(bindings)
        .bind(version["routing"].as_i64().unwrap())
        .bind(version["pricing"].as_i64().unwrap())
        .execute(&mut *publication)
        .await
        .unwrap();
    publication.commit().await.unwrap();
    version["routing"] = next_routing["version"].clone();
    let (status, body) = waiting.await.unwrap();
    assert_eq!(
        status,
        StatusCode::CONFLICT,
        "old post-auth snapshot passed the table fence: {body}"
    );
    assert_eq!(row(&pool).await, saved);

    // Revoke the requesting credential after initial admission, before the GET finishes.
    let waiting_app = app.clone();
    let waiting_secret = admin.secret.clone();
    let waiting = tokio::spawn(async move {
        request(waiting_app, "GET", BALANCE, &waiting_secret, Value::Null).await
    });
    tokio::time::timeout(Duration::from_secs(3), provider_state.started.notified())
        .await
        .unwrap();
    sqlx::query("UPDATE core_identity.credentials SET revoked_at=clock_timestamp() WHERE id=$1")
        .bind(admin.id)
        .execute(&pool)
        .await
        .unwrap();
    provider_state.release.add_permits(1);
    assert_eq!(waiting.await.unwrap().0, StatusCode::UNAUTHORIZED);
    assert_eq!(row(&pool).await, saved);
    assert!(version["routing"].as_u64().unwrap() > 1);
    server.abort();
    drop(app);
    drop(gateway);
    role.cleanup().await;
}

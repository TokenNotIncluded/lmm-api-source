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
    usage_body: RwLock<String>,
    usage_query: RwLock<Option<String>>,
    api_key_auth: AtomicBool,
    blocked: AtomicBool,
    started: Notify,
    release: Semaphore,
}
async fn provider(
    axum::extract::State(state): axum::extract::State<Arc<Provider>>,
    headers: axum::http::HeaderMap,
    uri: axum::http::Uri,
) -> impl axum::response::IntoResponse {
    if state.api_key_auth.load(Ordering::SeqCst) {
        assert_eq!(headers["api-key"], "synthetic-channel-only");
        assert!(!headers.contains_key("authorization"));
    } else {
        assert_eq!(headers["authorization"], "Bearer synthetic-channel-only");
        assert!(!headers.contains_key("api-key"));
    }
    if state.blocked.load(Ordering::SeqCst) {
        state.started.notify_one();
        state.release.acquire().await.unwrap().forget();
    }
    let body = if uri.path() == "/v1/dashboard/billing/usage" {
        *state.usage_query.write().await = uri.query().map(str::to_owned);
        state.usage_body.read().await.clone()
    } else {
        state.body.read().await.clone()
    };
    ([(header::CONTENT_TYPE, "application/json")], body)
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
        usage_body: RwLock::new(json!({"total_usage":125}).to_string()),
        usage_query: RwLock::new(None),
        api_key_auth: AtomicBool::new(false),
        blocked: AtomicBool::new(false),
        started: Notify::new(),
        release: Semaphore::new(0),
    });
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let endpoint = format!("http://{}", listener.local_addr().unwrap());
    let upstream = Router::new()
        .route("/balance", axum::routing::get(provider))
        .route(
            "/v1/dashboard/billing/subscription",
            axum::routing::get(provider),
        )
        .route("/v1/dashboard/billing/usage", axum::routing::get(provider))
        .with_state(provider_state.clone());
    let server = tokio::spawn(async move { axum::serve(listener, upstream).await.unwrap() });
    let role = runtime::RuntimeRole::create(&pool).await;
    role.optional("core_charging").await;
    role.optional("core_model").await;
    sqlx::raw_sql(sqlx::AssertSqlSafe(format!(
        "GRANT EXECUTE ON FUNCTION public.routing_control_balance_lock() TO {}",
        role.name
    )))
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
        json!({"path":"/balance","json_pointer":"/x","subtract_json_pointer":"bad"}),
        json!({"path":"/balance","json_pointer":"/x","subtract_json_pointer":"/bad~2"}),
        json!({"path":"/balance","json_pointer":"/x","subtract_json_pointer":"/synthetic-channel-only"}),
        json!({"path":"/balance","json_pointer":"/x","require":{"json_pointer":"bad","equals":true}}),
        json!({"path":"/balance","json_pointer":"/x","require":{"json_pointer":"/status","equals":null}}),
        json!({"path":"/balance","json_pointer":"/x","require":{"json_pointer":"/status","equals":{"ok":true}}}),
        json!({"path":"/balance","json_pointer":"/x","require":{"json_pointer":"/status","equals":"x".repeat(257)}}),
        json!({"path":"/balance","json_pointer":"/x","require":{"json_pointer":"/synthetic-channel-only","equals":true}}),
        json!({"path":"/balance","json_pointer":"/x","require":{"json_pointer":"/status","equals":"synthetic-channel-only"}}),
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
    // The explicit subtraction plan covers OpenRouter-shaped credit responses
    // without guessing a provider or changing legacy single-pointer probes.
    let (status, body) = request(
        app.clone(), "PUT", CONFIG, &admin.secret,
        json!({"expected_version":version,"probe":{"path":"/balance","json_pointer":"/data/total_credits","subtract_json_pointer":"/data/total_usage","unit_label":"USD"}}),
    ).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    version = body["data"]["version"].clone();
    *provider_state.body.write().await =
        json!({"data":{"total_credits":100.0,"total_usage":12.25}}).to_string();
    let (status, body) = request(app.clone(), "GET", BALANCE, &admin.secret, Value::Null).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(body["data"]["balance"], "87.75");
    let recovered = ManagedChannels::recover(
        role.pool.clone(),
        gateway_with_loopback(&role.pool, Some(&endpoint)),
    )
    .await
    .unwrap();
    let recovered_app = managed_router(role.pool.clone(), recovered);
    let (status, recovered_body) = request(
        recovered_app.clone(),
        "GET",
        BALANCE,
        &admin.secret,
        Value::Null,
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{recovered_body}");
    assert_eq!(recovered_body["data"]["balance"], "87.75");
    let (status, legacy_body) = request(
        recovered_app,
        "GET",
        "/api/channel/update_balance/1",
        &admin.secret,
        Value::Null,
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{legacy_body}");
    assert_eq!(legacy_body["data"], 87.75);
    assert_eq!(legacy_body["observation"]["unit_label"], "USD");
    *provider_state.body.write().await =
        json!({"data":{"total_credits":1,"total_usage":2}}).to_string();
    let (status, body) = request(app.clone(), "GET", BALANCE, &admin.secret, Value::Null).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(body["data"]["balance"], "-1");
    let subtraction_saved = row(&pool).await;
    for bad in [
        json!({"data":{"total_credits":100}}),
        json!({"data":{"total_credits":100,"total_usage":"NaN"}}),
        json!({"data":{"total_credits":1e308,"total_usage":-1e308}}),
        json!({"data":{"total_credits":100,"total_usage":"synthetic-channel-only"}}),
    ] {
        *provider_state.body.write().await = bad.to_string();
        assert_eq!(
            request(app.clone(), "GET", BALANCE, &admin.secret, Value::Null)
                .await
                .0,
            StatusCode::BAD_GATEWAY
        );
        assert_eq!(row(&pool).await, subtraction_saved);
    }
    // Explicit status guard: reject a provider error even if it includes a balance.
    let (status, body) = request(app.clone(), "PUT", CONFIG, &admin.secret,
        json!({"expected_version":version,"probe":{"path":"/balance","json_pointer":"/data/totalBalance","unit_label":"CNY","require":{"json_pointer":"/code","equals":20000}}})).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    version = body["data"]["version"].clone();
    *provider_state.body.write().await =
        json!({"code":20000,"data":{"totalBalance":"18.75"}}).to_string();
    let recovered = ManagedChannels::recover(
        role.pool.clone(),
        gateway_with_loopback(&role.pool, Some(&endpoint)),
    )
    .await
    .unwrap();
    let (status, body) = request(
        managed_router(role.pool.clone(), recovered),
        "GET",
        BALANCE,
        &admin.secret,
        Value::Null,
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(body["data"]["balance"], "18.75");
    let guarded_saved = row(&pool).await;
    for bad in [
        json!({"code":0,"data":{"totalBalance":"500"}}),
        json!({"code":"20000","data":{"totalBalance":"500"}}),
        json!({"data":{"totalBalance":"500"}}),
    ] {
        *provider_state.body.write().await = bad.to_string();
        assert_eq!(
            request(app.clone(), "GET", BALANCE, &admin.secret, Value::Null)
                .await
                .0,
            StatusCode::BAD_GATEWAY
        );
        assert_eq!(row(&pool).await, guarded_saved);
    }
    // Two explicit Moonshot-shaped status predicates must both match. This
    // checks status handling only; it does not convert the CNY balance.
    for requirements in [
        json!([]),
        json!(vec![json!({"json_pointer":"/code","equals":0}); 9]),
        json!([{"json_pointer":"bad","equals":0}]),
        json!([{"json_pointer":"/code","equals":null}]),
        json!([{"json_pointer":"/code","equals":"synthetic-channel-only"}]),
    ] {
        let (status, _) = request(app.clone(), "PUT", CONFIG, &admin.secret,
            json!({"expected_version":version,"probe":{"path":"/balance","json_pointer":"/data/available_balance","unit_label":"CNY","require_all":requirements}})).await;
        assert_eq!(status, StatusCode::BAD_REQUEST);
        assert_eq!(row(&pool).await, guarded_saved);
    }
    let (status, body) = request(app.clone(), "PUT", CONFIG, &admin.secret,
        json!({"expected_version":version,"probe":{"path":"/balance","json_pointer":"/data/available_balance","unit_label":"CNY","require":{"json_pointer":"/status","equals":true},"require_all":[{"json_pointer":"/code","equals":0}]}})).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    version = body["data"]["version"].clone();
    *provider_state.body.write().await =
        json!({"status":true,"code":0,"data":{"available_balance":42.5}}).to_string();
    let recovered = ManagedChannels::recover(
        role.pool.clone(),
        gateway_with_loopback(&role.pool, Some(&endpoint)),
    )
    .await
    .unwrap();
    let (status, body) = request(
        managed_router(role.pool.clone(), recovered),
        "GET",
        BALANCE,
        &admin.secret,
        Value::Null,
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(body["data"]["balance"], "42.5");
    let both_saved = row(&pool).await;
    for bad in [
        json!({"status":false,"code":0,"data":{"available_balance":500}}),
        json!({"status":true,"code":1,"data":{"available_balance":500}}),
        json!({"status":true,"code":"0","data":{"available_balance":500}}),
        json!({"status":true,"data":{"available_balance":500}}),
    ] {
        *provider_state.body.write().await = bad.to_string();
        assert_eq!(
            request(app.clone(), "GET", BALANCE, &admin.secret, Value::Null)
                .await
                .0,
            StatusCode::BAD_GATEWAY
        );
        assert_eq!(row(&pool).await, both_saved);
    }
    // Explicit operator-supplied CNY/USD rate; no rate is inferred.
    for invalid in [
        json!({"unit_label":"USD","cny_per_usd":"0"}),
        json!({"unit_label":"USD","cny_per_usd":"-1"}),
        json!({"unit_label":"USD","cny_per_usd":"NaN"}),
        json!({"unit_label":"USD","cny_per_usd":" 7"}),
        json!({"unit_label":"CNY","cny_per_usd":"7"}),
        json!({"unit_label":"USD","cny_per_usd":"7","subtract_json_pointer":"/other"}),
    ] {
        let mut probe = json!({"path":"/balance","json_pointer":"/data/available_balance"});
        probe
            .as_object_mut()
            .unwrap()
            .extend(invalid.as_object().unwrap().clone());
        let (status, _) = request(
            app.clone(),
            "PUT",
            CONFIG,
            &admin.secret,
            json!({"expected_version":version,"probe":probe}),
        )
        .await;
        assert_eq!(status, StatusCode::BAD_REQUEST);
        assert_eq!(row(&pool).await, both_saved);
    }
    let (status, body) = request(app.clone(), "PUT", CONFIG, &admin.secret,
        json!({"expected_version":version,"probe":{"path":"/balance","json_pointer":"/data/available_balance","unit_label":"USD","cny_per_usd":"7","require":{"json_pointer":"/status","equals":true},"require_all":[{"json_pointer":"/code","equals":0}]}})).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    version = body["data"]["version"].clone();
    *provider_state.body.write().await =
        json!({"status":true,"code":0,"data":{"available_balance":42}}).to_string();
    let recovered = ManagedChannels::recover(
        role.pool.clone(),
        gateway_with_loopback(&role.pool, Some(&endpoint)),
    )
    .await
    .unwrap();
    let (status, body) = request(
        managed_router(role.pool.clone(), recovered),
        "GET",
        BALANCE,
        &admin.secret,
        Value::Null,
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(body["data"]["balance"], "6");
    assert_eq!(body["data"]["unit_label"], "USD");
    let converted_saved = row(&pool).await;
    *provider_state.body.write().await =
        json!({"status":false,"code":0,"data":{"available_balance":42}}).to_string();
    assert_eq!(
        request(app.clone(), "GET", BALANCE, &admin.secret, Value::Null)
            .await
            .0,
        StatusCode::BAD_GATEWAY
    );
    assert_eq!(row(&pool).await, converted_saved);
    // DeepSeek's first USD wins even when CNY appears earlier; CNY fallback
    // uses only the explicitly configured rate.
    let (status, _) = request(app.clone(), "PUT", CONFIG, &admin.secret,
        json!({"expected_version":version,"probe":{"path":"/balance","json_pointer":"/wrong","unit_label":"USD","deepseek_currency_array":true}})).await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    let (status, body) = request(app.clone(), "PUT", CONFIG, &admin.secret,
        json!({"expected_version":version,"probe":{"path":"/balance","json_pointer":"/balance_infos","unit_label":"USD","cny_per_usd":"7","deepseek_currency_array":true}})).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    version = body["data"]["version"].clone();
    *provider_state.body.write().await = json!({"balance_infos":[{"currency":"CNY","total_balance":"70"},{"currency":" USD ","total_balance":"2.5"}]}).to_string();
    let recovered = ManagedChannels::recover(
        role.pool.clone(),
        gateway_with_loopback(&role.pool, Some(&endpoint)),
    )
    .await
    .unwrap();
    let (status, body) = request(
        managed_router(role.pool.clone(), recovered),
        "GET",
        BALANCE,
        &admin.secret,
        Value::Null,
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(body["data"]["balance"], "2.5");
    *provider_state.body.write().await =
        json!({"balance_infos":[{"currency":"CNY","total_balance":"70"}]}).to_string();
    let (status, body) = request(app.clone(), "GET", BALANCE, &admin.secret, Value::Null).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(body["data"]["balance"], "10");
    let deepseek_saved = row(&pool).await;
    for bad in [
        json!({"balance_infos":[{"currency":"USD","total_balance":"-1"}]}),
        json!({"balance_infos":[{"currency":"USD","total_balance":"bad"},{"currency":"CNY","total_balance":"70"}]}),
        json!({"balance_infos":[{"currency":"EUR","total_balance":"70"}]}),
    ] {
        *provider_state.body.write().await = bad.to_string();
        assert_eq!(
            request(app.clone(), "GET", BALANCE, &admin.secret, Value::Null)
                .await
                .0,
            StatusCode::BAD_GATEWAY
        );
        assert_eq!(row(&pool).await, deepseek_saved);
    }
    // Legacy OpenAI-style plan makes two same-origin bearer GETs and derives
    // usage dates from the subscription's payment flag in local calendar time.
    let (status, _) = request(app.clone(), "PUT", CONFIG, &admin.secret,
        json!({"expected_version":version,"probe":{"path":"/v1/dashboard/billing/subscription","json_pointer":"/hard_limit_usd","unit_label":"USD","openai_subscription":true,"subtract_json_pointer":"/x"}})).await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    let (status, body) = request(app.clone(), "PUT", CONFIG, &admin.secret,
        json!({"expected_version":version,"probe":{"path":"/v1/dashboard/billing/subscription","json_pointer":"/hard_limit_usd","unit_label":"USD","openai_subscription":true}})).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    version = body["data"]["version"].clone();
    for paid in [true, false] {
        *provider_state.body.write().await =
            json!({"has_payment_method":paid,"hard_limit_usd":12}).to_string();
        let (status, body) = request(app.clone(), "GET", BALANCE, &admin.secret, Value::Null).await;
        assert_eq!(status, StatusCode::OK, "{body}");
        assert_eq!(body["data"]["balance"], "10.75");
        let query = provider_state.usage_query.read().await.clone().unwrap();
        let parsed = reqwest::Url::parse(&format!("https://example.invalid/?{query}")).unwrap();
        let pairs: std::collections::HashMap<_, _> = parsed.query_pairs().into_owned().collect();
        let end = chrono::NaiveDate::parse_from_str(&pairs["end_date"], "%Y-%m-%d").unwrap();
        let start = chrono::NaiveDate::parse_from_str(&pairs["start_date"], "%Y-%m-%d").unwrap();
        use chrono::Datelike;
        assert_eq!(
            start,
            if paid {
                end.with_day(1).unwrap()
            } else {
                end.checked_sub_days(chrono::Days::new(100)).unwrap()
            }
        );
        assert!((chrono::Local::now().date_naive() - end).num_days().abs() <= 1);
        assert_eq!(pairs.len(), 2);
    }
    let openai_saved = row(&pool).await;
    *provider_state.usage_body.write().await = json!({"total_usage":"bad"}).to_string();
    assert_eq!(
        request(app.clone(), "GET", BALANCE, &admin.secret, Value::Null)
            .await
            .0,
        StatusCode::BAD_GATEWAY
    );
    assert_eq!(row(&pool).await, openai_saved);
    *provider_state.usage_body.write().await = json!({"total_usage":125}).to_string();
    // AIProxy-shaped plan uses exactly the Api-Key header and success flag.
    let (status, body) = request(app.clone(), "PUT", CONFIG, &admin.secret,
        json!({"expected_version":version,"probe":{"path":"/balance","json_pointer":"/data/totalPoints","unit_label":"points","auth":"api_key","require":{"json_pointer":"/success","equals":true}}})).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    version = body["data"]["version"].clone();
    provider_state.api_key_auth.store(true, Ordering::SeqCst);
    *provider_state.body.write().await =
        json!({"success":true,"data":{"totalPoints":77.5}}).to_string();
    let recovered = ManagedChannels::recover(
        role.pool.clone(),
        gateway_with_loopback(&role.pool, Some(&endpoint)),
    )
    .await
    .unwrap();
    let (status, body) = request(
        managed_router(role.pool.clone(), recovered),
        "GET",
        BALANCE,
        &admin.secret,
        Value::Null,
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(body["data"]["balance"], "77.5");
    let api_key_saved = row(&pool).await;
    *provider_state.body.write().await =
        json!({"success":false,"data":{"totalPoints":100}}).to_string();
    assert_eq!(
        request(app.clone(), "GET", BALANCE, &admin.secret, Value::Null)
            .await
            .0,
        StatusCode::BAD_GATEWAY
    );
    assert_eq!(row(&pool).await, api_key_saved);
    provider_state.api_key_auth.store(false, Ordering::SeqCst);
    // Explicit simple plans matching legacy API2GPT and AIGC2D response shapes.
    for (pointer, response, expected) in [
        (
            "/total_remaining",
            json!({"object":"credit_summary","total_remaining":12.5}),
            "12.5",
        ),
        (
            "/total_available",
            json!({"total_available":34.25}),
            "34.25",
        ),
    ] {
        version = configure(&app, &admin.secret, &version, pointer, json!("credits")).await;
        *provider_state.body.write().await = response.to_string();
        let (status, body) = request(app.clone(), "GET", BALANCE, &admin.secret, Value::Null).await;
        assert_eq!(status, StatusCode::OK, "{body}");
        assert_eq!(body["data"]["balance"], expected);
    }
    version = configure(
        &app,
        &admin.secret,
        &version,
        "/a~1b/~0balance",
        Value::Null,
    )
    .await;
    provider_state.api_key_auth.store(false, Ordering::SeqCst);
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

#[sqlx::test(migrations = false)]
async fn legacy_balance_api_publishes_reloads_overrides_copies_and_disables(pool: PgPool) {
    crate::core_schema::init(&pool).await.unwrap();
    let identity = IdentityStore::from_pool(pool.clone());
    let admin = identity.bootstrap_user(8, 6).await.unwrap();
    let ordinary = identity.bootstrap_user(7, 1).await.unwrap();
    let provider_state = Arc::new(Provider {
        body: RwLock::new(
            json!({"code":0,"status":true,"data":{"available_balance":70}}).to_string(),
        ),
        usage_body: RwLock::new(String::new()),
        usage_query: RwLock::new(None),
        api_key_auth: AtomicBool::new(false),
        blocked: AtomicBool::new(false),
        started: Notify::new(),
        release: Semaphore::new(0),
    });
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let endpoint = format!("http://{}", listener.local_addr().unwrap());
    // Exact automatic path: a wrong mapping gets 404 rather than a mock success.
    let upstream = Router::new()
        .route("/v1/users/me/balance", axum::routing::get(provider))
        .route("/balance", axum::routing::get(provider))
        .with_state(provider_state.clone());
    let server = tokio::spawn(async move { axum::serve(listener, upstream).await.unwrap() });
    let role = runtime::RuntimeRole::create(&pool).await;
    role.optional("core_charging").await;
    role.optional("core_model").await;
    sqlx::raw_sql(sqlx::AssertSqlSafe(format!(
        "GRANT EXECUTE ON FUNCTION public.routing_control_balance_lock() TO {}",
        role.name
    )))
    .execute(&pool)
    .await
    .unwrap();
    let gateway = || {
        gateway_with_loopback(&role.pool, Some(&endpoint))
            .with_channel_secret_key(&[83; 32])
            .unwrap()
    };
    let manager = ManagedChannels::recover(role.pool.clone(), gateway())
        .await
        .unwrap();
    let mut app = managed_router(role.pool.clone(), manager);
    let mut version = json!({"routing":1,"pricing":1});
    let legacy = json!({"channel_type":25,"cny_per_usd":"7"});
    let input = json!({"expected_version":version,"probe":null,"legacy":legacy});
    assert_eq!(
        request(app.clone(), "PUT", CONFIG, &ordinary.secret, input.clone())
            .await
            .0,
        StatusCode::FORBIDDEN
    );
    let (status, saved) = request(app.clone(), "PUT", CONFIG, &admin.secret, input).await;
    assert_eq!(status, StatusCode::OK, "{saved}");
    assert_eq!(saved["data"]["legacy"], legacy);
    assert_eq!(saved["data"]["probe"], Value::Null);
    version = saved["data"]["version"].clone();
    // Publication recovery must carry type and explicit rate, without rebuilding
    // manual probes or calling the provider while saving configuration.
    let manager = ManagedChannels::recover(role.pool.clone(), gateway())
        .await
        .unwrap();
    app = managed_router(role.pool.clone(), manager);
    let (status, config) = request(
        app.clone(),
        "GET",
        "/core/v1/admin/routing/configuration",
        &admin.secret,
        Value::Null,
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{config}");
    assert_eq!(config["data"]["bindings"]["legacy_balance"]["1"], legacy);
    let (status, result) = request(app.clone(), "GET", BALANCE, &admin.secret, Value::Null).await;
    assert_eq!(status, StatusCode::OK, "{result}");
    assert_eq!(result["data"]["balance"], "10");
    assert_eq!(result["data"]["unit_label"], "USD");
    assert!(result["data"]["balance_updated_time"].as_i64().unwrap() > 0);
    let original = row(&pool).await;
    for bad in [
        json!({"status":false,"code":0,"data":{"available_balance":999}}),
        json!({"status":true,"code":0,"data":{"available_balance":"bad"}}),
    ] {
        *provider_state.body.write().await = bad.to_string();
        assert_eq!(
            request(app.clone(), "GET", BALANCE, &admin.secret, Value::Null)
                .await
                .0,
            StatusCode::BAD_GATEWAY
        );
        assert_eq!(row(&pool).await, original);
    }
    for rate in ["0", "NaN", " 7"] {
        let (status, _) = request(app.clone(), "PUT", CONFIG, &admin.secret,
            json!({"expected_version":version,"probe":null,"legacy":{"channel_type":25,"cny_per_usd":rate}})).await;
        assert_eq!(status, StatusCode::BAD_REQUEST);
        assert_eq!(row(&pool).await, original);
    }
    // Old probe-only requests preserve automatic metadata, but manual wins.
    version = configure(&app, &admin.secret, &version, "/manual", json!("points")).await;
    *provider_state.body.write().await = json!({"manual":3,"status":false}).to_string();
    let manager = ManagedChannels::recover(role.pool.clone(), gateway())
        .await
        .unwrap();
    app = managed_router(role.pool.clone(), manager);
    let (status, result) = request(app.clone(), "GET", BALANCE, &admin.secret, Value::Null).await;
    assert_eq!(status, StatusCode::OK, "{result}");
    assert_eq!(result["data"]["balance"], "3");
    assert_eq!(result["data"]["unit_label"], "points");
    let manual_observation = row(&pool).await;
    let (status, config) = request(
        app.clone(),
        "GET",
        "/core/v1/admin/routing/configuration",
        &admin.secret,
        Value::Null,
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{config}");
    assert_eq!(config["data"]["bindings"]["legacy_balance"]["1"], legacy);
    // Explicit automatic save removes the manual override; absent rate retains CNY.
    let (status, saved) = request(
        app.clone(),
        "PUT",
        CONFIG,
        &admin.secret,
        json!({"expected_version":version,"probe":null,"legacy":{"channel_type":25}}),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{saved}");
    assert_eq!(saved["data"]["probe"], Value::Null);
    version = saved["data"]["version"].clone();
    assert_eq!(
        row(&pool).await,
        manual_observation,
        "configuration must not create a fresh observation"
    );
    *provider_state.body.write().await =
        json!({"status":true,"code":0,"data":{"available_balance":70}}).to_string();
    let (status, result) = request(app.clone(), "GET", BALANCE, &admin.secret, Value::Null).await;
    assert_eq!(status, StatusCode::OK, "{result}");
    assert_eq!(result["data"]["unit_label"], "CNY");
    assert_eq!(result["data"]["balance"], "70");
    let before_copy = row(&pool).await;
    let (status, copied) = request(
        app.clone(),
        "POST",
        "/core/v1/admin/routing/channels/1/copy",
        &admin.secret,
        json!({"expected_version":version,"suffix":"_automatic","reset_balance":false}),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{copied}");
    version = copied["data"]["version"].clone();
    let id = copied["data"]["id"].as_u64().unwrap();
    let manager = ManagedChannels::recover(role.pool.clone(), gateway())
        .await
        .unwrap();
    app = managed_router(role.pool.clone(), manager);
    let (status, config) = request(
        app.clone(),
        "GET",
        "/core/v1/admin/routing/configuration",
        &admin.secret,
        Value::Null,
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{config}");
    assert_eq!(
        config["data"]["bindings"]["legacy_balance"][id.to_string()],
        json!({"channel_type":25})
    );
    let copied_row: Value = sqlx::query_scalar(
        "SELECT to_jsonb(o) FROM core_channels.balance_observations o WHERE channel_id=$1",
    )
    .bind(id as i64)
    .fetch_one(&pool)
    .await
    .unwrap();
    assert_eq!(copied_row["observed_at"], before_copy["observed_at"]);
    let (status, result) = request(
        app.clone(),
        "GET",
        &format!("/core/v1/admin/routing/channels/{id}/balance"),
        &admin.secret,
        Value::Null,
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{result}");
    assert_eq!(result["data"]["balance"], "70");
    // Legacy null request disables both plans and never rewrites the last observation.
    let (status, cleared) = request(
        app.clone(),
        "PUT",
        CONFIG,
        &admin.secret,
        json!({"expected_version":version,"probe":null}),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{cleared}");
    assert_eq!(cleared["data"]["legacy"], Value::Null);
    let manager = ManagedChannels::recover(role.pool.clone(), gateway())
        .await
        .unwrap();
    app = managed_router(role.pool.clone(), manager);
    assert_eq!(
        request(app.clone(), "GET", BALANCE, &admin.secret, Value::Null)
            .await
            .0,
        StatusCode::NOT_IMPLEMENTED
    );
    assert_eq!(row(&pool).await, before_copy);
    assert_eq!(
        request(
            app.clone(),
            "GET",
            &format!("/core/v1/admin/routing/channels/{id}/balance"),
            &admin.secret,
            Value::Null
        )
        .await
        .0,
        StatusCode::OK
    );
    server.abort();
    drop(app);
    role.cleanup().await;
}

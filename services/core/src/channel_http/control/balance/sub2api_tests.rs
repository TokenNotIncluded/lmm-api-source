//! Real GET /v1/usage and durable observation coverage, with a synthetic provider.
use super::*;

#[sqlx::test(migrations = false)]
async fn sub2api_usage_wallet_quota_and_failed_refresh_preserve_observations(pool: PgPool) {
    crate::core_schema::init(&pool).await.unwrap();
    let identity = IdentityStore::from_pool(pool.clone());
    let admin = identity.bootstrap_user(8, 6).await.unwrap();
    let ordinary = identity.bootstrap_user(7, 1).await.unwrap();
    let wallet = include_str!("../../../../tests/fixtures/sub2api_wallet_usage.json");
    let quota = include_str!("../../../../tests/fixtures/sub2api_quota_usage.json");
    let state = Arc::new(RwLock::new((StatusCode::OK, wallet.to_owned())));
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let endpoint = format!("http://{}", listener.local_addr().unwrap());
    let upstream = Router::new()
        .route(
            "/v1/usage",
            axum::routing::get(
                |axum::extract::State(state): axum::extract::State<
                    Arc<RwLock<(StatusCode, String)>>,
                >,
                 headers: axum::http::HeaderMap,
                 uri: axum::http::Uri| async move {
                    assert_eq!(headers["authorization"], "Bearer synthetic-channel-only");
                    assert!(!headers.contains_key("x-api-key"));
                    assert!(!headers.contains_key("x-goog-api-key"));
                    assert!(!headers.contains_key("api-key"));
                    assert!(uri.query().is_none());
                    let (status, body) = state.read().await.clone();
                    (status, [(header::CONTENT_TYPE, "application/json")], body)
                },
            ),
        )
        .with_state(state.clone());
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
    // Model traffic uses Anthropic auth; the balance endpoint still requires Bearer.
    let gateway = || {
        gateway_for_protocol(
            &role.pool,
            Some(&endpoint),
            None,
            crate::routing::Protocol::AnthropicMessages,
        )
        .with_channel_secret_key(&[83; 32])
        .unwrap()
    };
    let manager = ManagedChannels::recover(role.pool.clone(), gateway())
        .await
        .unwrap();
    let app = managed_router(role.pool.clone(), manager);
    let (status, configured) = request(
        app.clone(),
        "PUT",
        CONFIG,
        &admin.secret,
        json!({"expected_version":{"routing":1,"pricing":1},"probe":null,"legacy":{"channel_type":59}}),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{configured}");
    assert_eq!(configured["data"]["legacy"]["channel_type"], 59);
    // Recovery must retain the selected adapter without turning it into a scalar pointer.
    let manager = ManagedChannels::recover(role.pool.clone(), gateway())
        .await
        .unwrap();
    let app = managed_router(role.pool.clone(), manager);
    assert_eq!(
        request(app.clone(), "GET", BALANCE, &ordinary.secret, Value::Null)
            .await
            .0,
        StatusCode::FORBIDDEN
    );
    for (body, expected) in [(wallet, "42.5"), (quota, "986.17194888")] {
        *state.write().await = (StatusCode::OK, body.to_owned());
        let (status, result) =
            request(app.clone(), "GET", BALANCE, &admin.secret, Value::Null).await;
        assert_eq!(status, StatusCode::OK, "{result}");
        assert_eq!(result["data"]["balance"], expected);
        assert_eq!(result["data"]["unit_label"], "USD");
        assert_eq!(result["data"]["kind"], "provider");
        assert!(result["data"]["balance_updated_time"].as_i64().unwrap() > 0);
        assert!(!result.to_string().contains("synthetic-channel-only"));
    }
    let original = row(&pool).await;
    let escaped = "synthetic-channel-only"
        .chars()
        .map(|ch| format!("\\u{:04x}", ch as u32))
        .collect::<String>();
    let reflected = format!(
        r#"{{"remaining":99,"unit":"USD","isValid":true,"metadata":{{"private_key":"{escaped}"}}}}"#
    );
    for (status, body) in [
        (StatusCode::OK, "{}".to_owned()),
        (
            StatusCode::OK,
            r#"{"remaining":99,"unit":"USD","isValid":false}"#.to_owned(),
        ),
        (
            StatusCode::OK,
            r#"{"remaining":99,"unit":"CNY","isValid":true}"#.to_owned(),
        ),
        (
            StatusCode::OK,
            r#"{"mode":"quota_limited","rate_limits":[{"remaining":9}]}"#.to_owned(),
        ),
        (
            StatusCode::OK,
            r#"{"remaining":-1,"unit":"USD","subscription":{"daily_limit_usd":null}}"#.to_owned(),
        ),
        (StatusCode::OK, reflected),
        (StatusCode::OK, "{".to_owned()),
        (StatusCode::BAD_GATEWAY, wallet.to_owned()),
        (
            StatusCode::UNAUTHORIZED,
            "synthetic-channel-only".to_owned(),
        ),
    ] {
        *state.write().await = (status, body);
        // Native single requests used by the bounded batch and the compatibility
        // single route must both fail without changing the persisted observation.
        for path in [BALANCE, "/api/channel/update_balance/1"] {
            let (status, failure) =
                request(app.clone(), "GET", path, &admin.secret, Value::Null).await;
            assert_eq!(status, StatusCode::BAD_GATEWAY, "{failure}");
            assert!(!failure.to_string().contains("synthetic-channel-only"));
            assert!(!failure.to_string().contains(&escaped));
            assert_eq!(row(&pool).await, original);
        }
    }
    server.abort();
    role.cleanup().await;
}

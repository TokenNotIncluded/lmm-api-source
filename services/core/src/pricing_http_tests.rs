use super::*;
use crate::test_runtime as runtime;
use crate::{
    identity::IdentityStore,
    model_http::pricing::{PricingSurface, tests::gateway},
};
use axum::{
    body::{Body, to_bytes},
    http::Request,
};
use tower::ServiceExt;
async fn call(
    router: &Router,
    path: &str,
    token: Option<&str>,
    body: Option<Value>,
) -> (StatusCode, Value) {
    let mut req = Request::builder()
        .method(if body.is_some() { "POST" } else { "GET" })
        .uri(path);
    if let Some(token) = token {
        req = req.header(header::AUTHORIZATION, format!("Bearer {token}"))
    }
    if body.is_some() {
        req = req.header(header::CONTENT_TYPE, "application/json")
    }
    let response = router
        .clone()
        .oneshot(
            req.body(body.map_or_else(Body::empty, |v| Body::from(v.to_string())))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(
        response.headers()[header::CACHE_CONTROL],
        "private, no-store"
    );
    let status = response.status();
    let bytes = to_bytes(response.into_body(), 3 * 1024 * 1024)
        .await
        .unwrap();
    (status, serde_json::from_slice(&bytes).unwrap())
}
async fn counts(pool: &PgPool) -> (i64, i64, i64) {
    sqlx::query_as("SELECT (SELECT count(*) FROM core_charging.requests),(SELECT count(*) FROM core_billing.ledger_entries),(SELECT count(*) FROM core_identity.audit)").fetch_one(pool).await.unwrap()
}
async fn setup(
    pool: &PgPool,
) -> (
    IdentityStore,
    crate::identity::IssuedCredential,
    Arc<PricingSurface>,
) {
    crate::core_schema::init(pool).await.unwrap();
    let identity = IdentityStore::from_pool(pool.clone());
    let session = identity.bootstrap_user(7, 1).await.unwrap();
    sqlx::query("INSERT INTO core_identity.user_routing_groups(user_id,group_name,enabled,source_reference) VALUES(7,'allowed',TRUE,'pricing-private-test')").execute(pool).await.unwrap();
    let surface = Arc::new(PricingSurface::from_gateway(&gateway(pool.clone())).unwrap());
    (identity, session, surface)
}
#[sqlx::test(migrations = false)]
async fn pricing_pg_readonly_role_public_session_mark_and_revocation(pool: PgPool) {
    let (identity, session, surface) = setup(&pool).await;
    let role = runtime::RuntimeRole::create(&pool).await;
    role.optional("core_console").await;
    let router = PricingHttp::new(role.pool.clone(), surface).router();
    let before = counts(&pool).await;
    let (status, public) = call(&router, "/core/v1/pricing/public", None, None).await;
    assert_eq!(status, StatusCode::OK, "{public}");
    assert_eq!(public["visibility"], "public");
    assert_eq!(public["empty_reason"], Value::Null);
    assert_eq!(public["public_view_available"], true);
    assert_eq!(public["groups"], json!(["allowed", "private"]));
    assert_eq!(public["routing_version"], 9);
    assert_eq!(public["pricing_version"], 11);
    assert_eq!(public["invocation_authorized"], false);
    let (status, own) = call(
        &router,
        "/core/v1/pricing/session",
        Some(&session.secret),
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{own}");
    assert_eq!(own["visibility"], "session");
    assert_eq!(own["empty_reason"], Value::Null);
    assert_eq!(own["public_view_available"], true);
    assert_eq!(own["groups"], json!(["allowed"]));
    assert!(!own.to_string().contains("provider-model"));
    let key = identity
        .issue_key(
            &session.secret,
            crate::accounts::Account {
                kind: crate::accounts::AccountKind::Personal,
                id: 7,
            },
            crate::funding::FundingPolicy::new_personal_key(),
            3600,
        )
        .await
        .unwrap();
    assert_eq!(
        call(&router, "/core/v1/pricing/session", Some(&key.secret), None)
            .await
            .0,
        StatusCode::FORBIDDEN
    );
    assert_eq!(
        call(
            &router,
            "/core/v1/pricing/session",
            Some("lmm_at_not_a_session"),
            None
        )
        .await
        .0,
        StatusCode::FORBIDDEN
    );
    assert_eq!(
        call(&router, "/core/v1/pricing/session", None, None)
            .await
            .0,
        StatusCode::UNAUTHORIZED
    );
    let policy = json!({"version":1,"policies":[{"id":"marked","name":"pricing policy","mode":"fixed_text","fixed_text":"bounded","public_models":["model"],"clients":{},"limits":{"requests_per_minute_per_user":10,"requests_per_minute_global":10,"max_concurrent_per_user":1,"max_concurrent_global":1,"max_input_bytes":4096,"max_output_bytes":4096,"max_output_tokens":1024,"output_bytes_per_second":0,"total_timeout_seconds":5}}]});
    sqlx::query("UPDATE core_abuse.settings SET revision=1,document=$1 WHERE singleton")
        .bind(policy)
        .execute(&pool)
        .await
        .unwrap();
    sqlx::query("INSERT INTO core_abuse.user_marks(user_id,marked,policy_id,revision) VALUES(7,TRUE,'marked',1)").execute(&pool).await.unwrap();
    let (status, marked) = call(
        &router,
        "/core/v1/pricing/session",
        Some(&session.secret),
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{marked}");
    let models = marked["models"].as_array().unwrap();
    assert_eq!(models.len(), 1);
    assert_eq!(models[0]["model"], "model");
    assert_eq!(models[0]["group"], "allowed");
    let (status, runtime) = call(
        &router,
        "/core/v1/pricing/runtime",
        Some(&session.secret),
        Some(json!({"models":["model","claude","absent"]})),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{runtime}");
    assert_eq!(runtime["data"]["model"]["status"], "unknown");
    assert_eq!(runtime["data"]["claude"]["status"], "no_access");
    assert_eq!(runtime["data"]["absent"]["status"], "no_access");
    // Exclude explicit fixture key issuance, whose audit is a real authorized write.
    let after_fixture = counts(&pool).await;
    assert_eq!((before.0, before.1), (after_fixture.0, after_fixture.1));
    let _ = call(
        &router,
        "/core/v1/pricing/session",
        Some(&session.secret),
        None,
    )
    .await;
    assert_eq!(counts(&pool).await, after_fixture);
    sqlx::query("UPDATE core_identity.user_routing_groups SET enabled=FALSE WHERE user_id=7")
        .execute(&pool)
        .await
        .unwrap();
    assert_eq!(
        call(
            &router,
            "/core/v1/pricing/session",
            Some(&session.secret),
            None
        )
        .await
        .0,
        StatusCode::UNAUTHORIZED
    );
    assert_eq!(counts(&pool).await, after_fixture);
    role.cleanup().await;
}
#[sqlx::test(migrations = false)]
async fn pricing_pg_header_nav_policy_bounds_and_public_runtime_unknown(pool: PgPool) {
    let (_, session, surface) = setup(&pool).await;
    let router = PricingHttp::new(pool.clone(), surface).router();
    let before = counts(&pool).await;
    let unavailable = PricingHttp::unavailable(pool.clone()).router();
    let (status, error) = call(&unavailable, "/core/v1/pricing/public", None, None).await;
    assert_eq!(status, StatusCode::SERVICE_UNAVAILABLE);
    assert_eq!(error["error"]["code"], "pricing_unavailable");

    for (raw, public_code, session_code) in [
        (
            r#"{"pricing":{"enabled":true,"requireAuth":true}}"#,
            StatusCode::FORBIDDEN,
            StatusCode::OK,
        ),
        (
            r#"{"pricing":false}"#,
            StatusCode::FORBIDDEN,
            StatusCode::FORBIDDEN,
        ),
        (r#"{"pricing":true}"#, StatusCode::OK, StatusCode::OK),
    ] {
        sqlx::query("INSERT INTO core_console.options(key,value) VALUES('HeaderNavModules',$1) ON CONFLICT(key) DO UPDATE SET value=EXCLUDED.value").bind(raw).execute(&pool).await.unwrap();
        assert_eq!(
            call(&router, "/core/v1/pricing/public", None, None).await.0,
            public_code
        );
        assert_eq!(
            call(
                &router,
                "/core/v1/pricing/session",
                Some(&session.secret),
                None
            )
            .await
            .0,
            session_code
        );
    }
    let (status, runtime) = call(
        &router,
        "/core/v1/pricing/runtime",
        None,
        Some(json!({"models":["model","unknown-model"]})),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{runtime}");
    assert_eq!(runtime["data"]["model"]["status"], "unknown");
    assert_eq!(runtime["data"]["unknown-model"]["status"], "not_listed");
    assert_eq!(
        call(&router, "/core/v1/pricing/public?group=private", None, None)
            .await
            .0,
        StatusCode::BAD_REQUEST
    );
    assert_eq!(
        call(
            &router,
            "/core/v1/pricing/runtime",
            None,
            Some(json!({"models":["model","model"]}))
        )
        .await
        .0,
        StatusCode::BAD_REQUEST
    );
    assert_eq!(counts(&pool).await, before);
}

#[sqlx::test(migrations = false)]
async fn pricing_pg_empty_session_metadata_uses_only_visible_permissions(pool: PgPool) {
    crate::core_schema::init(&pool).await.unwrap();
    let session = IdentityStore::from_pool(pool.clone())
        .bootstrap_user(7, 1)
        .await
        .unwrap();
    let surface = Arc::new(PricingSurface::from_gateway(&gateway(pool.clone())).unwrap());
    let router = PricingHttp::new(pool.clone(), surface).router();
    // Start without assignments: deleting an existing grant revokes the session.
    // The default group remains valid, but this catalogue has no default routes.
    let before = counts(&pool).await;
    for (raw, public_available) in [
        (r#"{"pricing":true}"#, true),
        (r#"{"pricing":{"enabled":true,"requireAuth":true}}"#, false),
    ] {
        sqlx::query("INSERT INTO core_console.options(key,value) VALUES('HeaderNavModules',$1) ON CONFLICT(key) DO UPDATE SET value=EXCLUDED.value")
            .bind(raw).execute(&pool).await.unwrap();
        let (status, empty) = call(
            &router,
            "/core/v1/pricing/session",
            Some(&session.secret),
            None,
        )
        .await;
        assert_eq!(status, StatusCode::OK, "{empty}");
        assert_eq!(empty["visibility"], "session");
        assert_eq!(empty["empty_reason"], "no_visible_models");
        assert_eq!(empty["public_view_available"], public_available);
        assert_eq!(empty["invocation_authorized"], false);
        assert_eq!(empty["models"], json!([]));
        assert_eq!(empty["groups"], json!([]));
        assert!(!empty.to_string().contains("private"));
        assert!(!empty.to_string().contains("claude"));
        let (status, public) = call(&router, "/core/v1/pricing/public", None, None).await;
        assert_eq!(
            status,
            if public_available {
                StatusCode::OK
            } else {
                StatusCode::FORBIDDEN
            }
        );
        if public_available {
            assert_eq!(public["empty_reason"], Value::Null);
            assert_eq!(public["public_view_available"], true);
            assert!(!public["models"].as_array().unwrap().is_empty());
        } else {
            assert!(public.get("empty_reason").is_none());
            assert!(public.get("public_view_available").is_none());
        }
    }
    // A valid group can still have no visible models after the model restriction.
    sqlx::query("INSERT INTO core_identity.user_routing_groups(user_id,group_name,enabled,source_reference) VALUES(7,'allowed',TRUE,'pricing-empty-model-test')")
        .execute(&pool).await.unwrap();
    let policy = json!({"version":1,"policies":[{"id":"marked","name":"pricing policy","mode":"fixed_text","fixed_text":"bounded","public_models":["synthetic-not-listed"],"clients":{},"limits":{"requests_per_minute_per_user":10,"requests_per_minute_global":10,"max_concurrent_per_user":1,"max_concurrent_global":1,"max_input_bytes":4096,"max_output_bytes":4096,"max_output_tokens":1024,"output_bytes_per_second":0,"total_timeout_seconds":5}}]});
    sqlx::query("UPDATE core_abuse.settings SET revision=1,document=$1 WHERE singleton")
        .bind(policy)
        .execute(&pool)
        .await
        .unwrap();
    sqlx::query("INSERT INTO core_abuse.user_marks(user_id,marked,policy_id,revision) VALUES(7,TRUE,'marked',1)")
        .execute(&pool).await.unwrap();
    let (status, empty) = call(
        &router,
        "/core/v1/pricing/session",
        Some(&session.secret),
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{empty}");
    assert_eq!(empty["empty_reason"], "no_visible_models");
    assert_eq!(empty["models"], json!([]));
    assert_eq!(empty["groups"], json!([]));
    assert!(!empty.to_string().contains("synthetic-not-listed"));
    let (status, error) = call(
        &router,
        "/core/v1/pricing/session",
        Some("lmms_not_a_real_session"),
        None,
    )
    .await;
    assert_eq!(status, StatusCode::UNAUTHORIZED, "{error}");
    assert!(error.get("empty_reason").is_none());
    assert!(error.get("public_view_available").is_none());
    assert_eq!(counts(&pool).await, before);
}

#[sqlx::test(migrations = false)]
async fn pricing_pg_empty_public_catalog_is_distinct_from_an_empty_account(pool: PgPool) {
    use crate::routing::{PriceConfig, PriceSnapshot, RoutingConfig};
    crate::core_schema::init(&pool).await.unwrap();
    let routing = crate::routing::Router::new(
        RoutingConfig {
            channel_overrides: Default::default(),
            global_model_policy: None,
            channel_affinity: None,
            routing_reliability: None,
            version: 1,
            groups: vec![],
            upstreams: vec![],
            routes: vec![],
            aliases: vec![],
        },
        Arc::new(
            PriceSnapshot::compile(PriceConfig {
                version: 1,
                rules: vec![],
            })
            .unwrap(),
        ),
    )
    .unwrap();
    let gateway = crate::model_http::ModelGateway::new(
        pool.clone(),
        Arc::new(routing),
        vec![],
        std::collections::BTreeMap::new(),
        crate::relay::Limits {
            total_timeout: std::time::Duration::from_secs(1),
            hook_timeout: std::time::Duration::from_millis(100),
            ..Default::default()
        },
        crate::model_http::ChargePolicy {
            input_token_ceiling: 65536,
            lease_seconds: 10,
        },
    )
    .unwrap();
    let surface = Arc::new(PricingSurface::from_gateway(&gateway).unwrap());
    let router = PricingHttp::new(pool.clone(), surface).router();
    let before = counts(&pool).await;
    let (status, public) = call(&router, "/core/v1/pricing/public", None, None).await;
    assert_eq!(status, StatusCode::OK, "{public}");
    assert_eq!(public["visibility"], "public");
    assert_eq!(public["empty_reason"], "catalog_empty");
    assert_eq!(public["public_view_available"], true);
    assert_eq!(public["invocation_authorized"], false);
    assert_eq!(public["models"], json!([]));
    assert_eq!(public["groups"], json!([]));
    assert_eq!(counts(&pool).await, before);
}

#[sqlx::test(migrations = false)]
async fn pricing_pg_configured_group_rates_ignore_legacy_trust_state(pool: PgPool) {
    let (_, session, surface) = setup(&pool).await;
    let router = PricingHttp::new(pool.clone(), surface).router();
    let before = counts(&pool).await;
    let (status, public) = call(&router, "/core/v1/pricing/public", None, None).await;
    assert_eq!(status, StatusCode::OK, "{public}");
    assert_eq!(public["rate_basis"], "base_group_rate");
    assert!(public.get("user_price").is_none());
    let now: i64 =
        sqlx::query_scalar("SELECT floor(extract(epoch FROM transaction_timestamp()))::bigint")
            .fetch_one(&pool)
            .await
            .unwrap();
    sqlx::query("INSERT INTO core_trust_pricing.user_bases(user_id,original_created_at,console_activated_at,last_api_activity_at,override_level,projection_known,order_count,revision,source_digest) VALUES(7,$1,0,0,2,TRUE,0,1,decode(repeat('11',32),'hex'))")
        .bind(now).execute(&pool).await.unwrap();
    let (status, own) = call(
        &router,
        "/core/v1/pricing/session",
        Some(&session.secret),
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{own}");
    assert_eq!(own["rate_basis"], "effective_group_rate");
    assert!(own.get("user_price").is_none());
    for row in own["models"].as_array().unwrap() {
        let public_row = public["models"]
            .as_array()
            .unwrap()
            .iter()
            .find(|candidate| candidate["id"] == row["id"])
            .unwrap();
        let mut expected = public_row.clone();
        if !row["token_price"].is_null() {
            expected["token_price"]["rate_basis"] = json!("effective_group_rate");
        }
        if !row["upstream_cost_price"].is_null() {
            expected["upstream_cost_price"]["rate_basis"] = json!("effective_group_rate");
        }
        for image in expected["image_variants"].as_array_mut().unwrap() {
            image["rate_basis"] = json!("effective_group_rate");
        }
        assert_eq!(row, &expected);
    }
    let (_, public_again) = call(&router, "/core/v1/pricing/public", None, None).await;
    assert!(public_again.get("user_price").is_none());
    assert_eq!(public_again["models"], public["models"]);
    sqlx::query("UPDATE core_trust_pricing.policies SET native_user_floor=8")
        .execute(&pool)
        .await
        .unwrap();
    sqlx::query("DELETE FROM core_trust_pricing.user_bases WHERE user_id=7")
        .execute(&pool)
        .await
        .unwrap();
    let (status, own_again) = call(
        &router,
        "/core/v1/pricing/session",
        Some(&session.secret),
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{own_again}");
    assert_eq!(own_again["models"], own["models"]);
    assert!(own_again.get("user_price").is_none());
    assert_eq!(counts(&pool).await, before);
}

#[sqlx::test(migrations = false)]
async fn pricing_pg_upstream_cost_rule_is_readonly_exact_and_session_isolated(pool: PgPool) {
    let (_, session, _) = setup(&pool).await;
    let gateway = crate::model_http::pricing::tests::upstream_cost_gateway(pool.clone(), true);
    let role = runtime::RuntimeRole::create(&pool).await;
    role.optional("core_console").await;
    let router = PricingHttp::from_gateway(role.pool.clone(), &gateway).router();
    let now: i64 =
        sqlx::query_scalar("SELECT floor(extract(epoch FROM transaction_timestamp()))::bigint")
            .fetch_one(&pool)
            .await
            .unwrap();
    sqlx::query("INSERT INTO core_trust_pricing.user_bases(user_id,original_created_at,console_activated_at,last_api_activity_at,override_level,projection_known,order_count,revision,source_digest) VALUES(7,$1,0,0,2,TRUE,0,1,decode(repeat('11',32),'hex'))")
        .bind(now).execute(&pool).await.unwrap();
    let before = counts(&pool).await;
    let (status, public) = call(&router, "/core/v1/pricing/public", None, None).await;
    assert_eq!(status, StatusCode::OK, "{public}");
    assert_eq!(public["credits_per_usd"], 500_000);
    assert_eq!(public["rate_basis"], "base_group_rate");
    assert!(public.get("user_price").is_none());
    assert_eq!(public["groups"], json!(["allowed", "private"]));
    assert_eq!(public["invocation_authorized"], false);
    assert_eq!(public["models"].as_array().unwrap().len(), 6);
    for row in public["models"].as_array().unwrap() {
        assert_eq!(row["billing_status"], "unavailable");
        assert_eq!(
            row["unavailable_reason"],
            "upstream_budget_contract_required"
        );
        assert_eq!(row["token_price"], Value::Null);
        assert_eq!(row["apis"], json!([]));
        let price = &row["upstream_cost_price"];
        assert_eq!(price["currency"], "USD");
        assert_eq!(price["cost_source"], "usage.cost");
        assert_eq!(price["credits_per_usd"], 500_000);
        assert_eq!(price["rounding"], "ceil_total_credits");
        assert_eq!(price["rate_basis"], public["rate_basis"]);
        assert_eq!(
            price["multiplier"],
            json!({
                "numerator": if row["group"] == "private" { "2000000" } else { "1250000" },
                "denominator": "1000000"
            })
        );
    }
    let (status, own) = call(
        &router,
        "/core/v1/pricing/session",
        Some(&session.secret),
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{own}");
    assert_eq!(own["rate_basis"], "effective_group_rate");
    assert_eq!(own["groups"], json!(["allowed"]));
    assert_eq!(own["invocation_authorized"], false);
    assert!(own.get("user_price").is_none());
    assert_eq!(own["models"].as_array().unwrap().len(), 5);
    for row in own["models"].as_array().unwrap() {
        assert_eq!(row["group"], "allowed");
        assert_eq!(row["billing_status"], "unavailable");
        assert_eq!(
            row["unavailable_reason"],
            "upstream_budget_contract_required"
        );
        assert_eq!(row["upstream_cost_price"]["rate_basis"], own["rate_basis"]);
        // Only the configured group multiplier applies, even with legacy trust data.
        assert_eq!(
            row["upstream_cost_price"]["multiplier"],
            json!({"numerator":"1250000","denominator":"1000000"})
        );
    }
    let (status, own_again) = call(
        &router,
        "/core/v1/pricing/session",
        Some(&session.secret),
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(own_again["models"], own["models"]);
    let (status, public_again) = call(&router, "/core/v1/pricing/public", None, None).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(public_again["models"], public["models"]);
    assert!(public_again.get("user_price").is_none());
    let (status, runtime) = call(
        &router,
        "/core/v1/pricing/runtime",
        Some(&session.secret),
        Some(json!({"models":["openrouter/fusion", "hidden-model"]})),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{runtime}");
    assert_eq!(
        runtime["data"]["openrouter/fusion"]["status"],
        "unavailable"
    );
    assert_eq!(runtime["data"]["hidden-model"]["status"], "no_access");
    for forbidden in [
        "private-credential",
        "private-cost-provider",
        "synthetic-context",
        "lmms_",
    ] {
        assert!(!public.to_string().contains(forbidden));
        assert!(!own.to_string().contains(forbidden));
    }
    assert_eq!(counts(&pool).await, before);
    role.cleanup().await;
}

#[sqlx::test(migrations = false)]
async fn pricing_pg_disabled_module_allows_only_root_session(pool: PgPool) {
    let (identity, user, surface) = setup(&pool).await;
    let root = identity.bootstrap_user(8, 6).await.unwrap();
    let admin = identity.bootstrap_user(9, 5).await.unwrap();
    sqlx::query("INSERT INTO core_identity.user_routing_groups(user_id,group_name,enabled,source_reference) VALUES(8,'allowed',TRUE,'pricing-root-test')").execute(&pool).await.unwrap();
    let router = PricingHttp::new(pool.clone(), surface).router();
    let unknown = "lmms_not_a_real_root_session_token";
    for raw in [
        r#"{"pricing":{"enabled":false}}"#,
        r#"{"pricing":{"enabled":false,"requireAuth":true}}"#,
        r#"{"pricing":false}"#,
    ] {
        sqlx::query("INSERT INTO core_console.options(key,value) VALUES('HeaderNavModules',$1) ON CONFLICT(key) DO UPDATE SET value=EXCLUDED.value").bind(raw).execute(&pool).await.unwrap();
        let before = counts(&pool).await;
        let (status, public) = call(&router, "/core/v1/pricing/public", None, None).await;
        assert_eq!(status, StatusCode::FORBIDDEN, "{public}");
        assert_eq!(public["error"]["code"], "pricing_forbidden");
        for (secret, code) in [
            (Some(user.secret.as_str()), StatusCode::FORBIDDEN),
            (Some(admin.secret.as_str()), StatusCode::FORBIDDEN),
            (Some(unknown), StatusCode::UNAUTHORIZED),
        ] {
            let (status, body) = call(&router, "/core/v1/pricing/session", secret, None).await;
            assert_eq!(status, code, "{raw} {body}");
        }
        let (status, own) = call(
            &router,
            "/core/v1/pricing/session",
            Some(&root.secret),
            None,
        )
        .await;
        assert_eq!(status, StatusCode::OK, "{raw} {own}");
        assert_eq!(own["visibility"], "session");
        assert_eq!(own["public_view_available"], false);
        assert_eq!(own["invocation_authorized"], false);
        assert_eq!(own["groups"], json!(["allowed"]));
        let (status, runtime) = call(
            &router,
            "/core/v1/pricing/runtime",
            None,
            Some(json!({"models":["model"]})),
        )
        .await;
        assert_eq!(status, StatusCode::FORBIDDEN, "{runtime}");
        let (status, user_runtime) = call(
            &router,
            "/core/v1/pricing/runtime",
            Some(&user.secret),
            Some(json!({"models":["model"]})),
        )
        .await;
        assert_eq!(status, StatusCode::FORBIDDEN, "{user_runtime}");
        let (status, root_runtime) = call(
            &router,
            "/core/v1/pricing/runtime",
            Some(&root.secret),
            Some(json!({"models":["model"]})),
        )
        .await;
        assert_eq!(status, StatusCode::OK, "{root_runtime}");
        assert_eq!(root_runtime["visibility"], "session");
        assert_eq!(root_runtime["data"]["model"]["status"], "unknown");
        assert_eq!(counts(&pool).await, before);
    }
    let key = identity
        .issue_key(
            &root.secret,
            crate::accounts::Account {
                kind: crate::accounts::AccountKind::Personal,
                id: 8,
            },
            crate::funding::FundingPolicy::new_personal_key(),
            3600,
        )
        .await
        .unwrap();
    assert_eq!(
        call(&router, "/core/v1/pricing/session", Some(&key.secret), None)
            .await
            .0,
        StatusCode::FORBIDDEN
    );
}

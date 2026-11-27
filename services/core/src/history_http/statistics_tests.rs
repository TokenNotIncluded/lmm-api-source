use super::*;
use axum::{body::to_bytes, http::Request};
use serde_json::Value;
use sqlx::PgPool;
use tower::ServiceExt;

async fn request(store: HistoryStore, path: &str, secret: Option<&str>) -> Response {
    let mut request = Request::builder().uri(path);
    if let Some(secret) = secret {
        request = request.header(header::AUTHORIZATION, format!("Bearer {secret}"));
    }
    router(store)
        .oneshot(request.body(axum::body::Body::empty()).unwrap())
        .await
        .unwrap()
}

#[tokio::test]
async fn statistics_require_authentication_before_reporting_source_dependencies() {
    let pool = sqlx::postgres::PgPoolOptions::new()
        .connect_lazy("postgres://unused:unused@127.0.0.1:1/unused")
        .unwrap();
    for path in [
        "/api/finance/site-statistics",
        "/api/finance/site-statistics?view=verified",
        "/api/finance/site-statistics?view=unsupported",
        "/api/finance/site-statistics?view=verified&view=verified",
        "/api/perf-metrics/summary",
    ] {
        let response = request(HistoryStore::new(pool.clone()), path, None).await;
        assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
        assert_eq!(response.headers()[header::CACHE_CONTROL], "no-store");
        let value: Value =
            serde_json::from_slice(&to_bytes(response.into_body(), 8192).await.unwrap()).unwrap();
        assert_eq!(value["code"], "AUTH_UNAUTHORIZED");
        assert!(value.get("missing_sources").is_none());
    }
}

#[sqlx::test(migrations = false)]
async fn site_statistics_enforce_admin_and_report_exact_native_zero_totals(pool: PgPool) {
    crate::core_schema::init(&pool).await.unwrap();
    let identity = crate::identity::IdentityStore::from_pool(pool.clone());
    let user = identity.bootstrap_user(7, 1).await.unwrap();
    let admin = identity.bootstrap_user(8, 5).await.unwrap();
    let store = HistoryStore::new(pool);
    let response = request(
        store.clone(),
        "/api/finance/site-statistics",
        Some(&user.secret),
    )
    .await;
    assert_eq!(response.status(), StatusCode::FORBIDDEN);

    let response = request(store, "/api/finance/site-statistics", Some(&admin.secret)).await;
    assert_eq!(response.status(), StatusCode::OK);
    assert_eq!(response.headers()[header::CACHE_CONTROL], "no-store");
    let value: Value =
        serde_json::from_slice(&to_bytes(response.into_body(), 8192).await.unwrap()).unwrap();
    assert_eq!(value["success"], true);
    assert_eq!(value["data"]["credits_per_usd"], 500_000);
    assert_eq!(value["data"]["total_used_credits"], "0");
    assert_eq!(value["data"]["total_balance_credits"], "0");
    assert_eq!(value["data"]["recharge"]["currencies"], json!([]));
    assert_eq!(value["data"]["recharge"]["virtual_units"], json!([]));
    assert_eq!(value["data"]["recharge"]["confirmed_orders"], 0);
    assert_eq!(value["data"]["recharge"]["unconfirmed_orders"], 0);
    assert_eq!(value["data"]["recharge"]["invalid_orders"], 0);
}

#[sqlx::test(migrations = false)]
async fn empty_native_performance_has_no_invented_measurements(pool: PgPool) {
    crate::core_schema::init(&pool).await.unwrap();
    let identity = crate::identity::IdentityStore::from_pool(pool.clone());
    let user = identity.bootstrap_user(7, 1).await.unwrap();
    let response = request(
        HistoryStore::new(pool),
        "/api/perf-metrics/summary",
        Some(&user.secret),
    )
    .await;
    assert_eq!(response.status(), StatusCode::OK);
    let value: Value =
        serde_json::from_slice(&to_bytes(response.into_body(), 8192).await.unwrap()).unwrap();
    assert_eq!(value["availability"], "partial");
    assert_eq!(value["coverage"]["historical_imports"], false);
    assert_eq!(value["data"]["models"], json!([]));
    assert_eq!(value["data"]["aggregate"]["request_count"], 0);
    assert!(value["data"]["aggregate"]["avg_latency_ms"].is_null());
    assert!(value["data"]["aggregate"]["avg_tps"].is_null());
}

#[sqlx::test(migrations = false)]
async fn unverified_payment_evidence_preserves_verified_credit_metrics(pool: PgPool) {
    crate::core_schema::init(&pool).await.unwrap();
    let identity = crate::identity::IdentityStore::from_pool(pool.clone());
    let admin = identity.bootstrap_user(8, 5).await.unwrap();
    let member = identity.bootstrap_user(7, 1).await.unwrap();
    // The native payment view rejects a non-unit allocator as an invalid payment seal.
    sqlx::query("ALTER SEQUENCE core_payments.intents_id_seq INCREMENT BY 2")
        .execute(&pool)
        .await
        .unwrap();
    let store = HistoryStore::new(pool);
    let path = "/api/finance/site-statistics?view=verified";
    assert_eq!(
        request(store.clone(), path, Some(&member.secret))
            .await
            .status(),
        StatusCode::FORBIDDEN
    );
    assert_eq!(
        request(
            store.clone(),
            "/api/finance/site-statistics",
            Some(&admin.secret)
        )
        .await
        .status(),
        StatusCode::SERVICE_UNAVAILABLE
    );
    let response = request(store, path, Some(&admin.secret)).await;
    assert_eq!(response.status(), StatusCode::OK);
    assert_eq!(response.headers()[header::CACHE_CONTROL], "no-store");
    let body: Value =
        serde_json::from_slice(&to_bytes(response.into_body(), 8192).await.unwrap()).unwrap();
    assert_eq!(body["data"]["total_used_credits"], "0");
    assert_eq!(body["data"]["total_balance_credits"], "0");
    assert_eq!(body["data"].get("recharge"), Some(&Value::Null));
    assert_eq!(
        body["data"]["order_exceptions"],
        json!({"unconfirmed_orders":0,"invalid_orders":0})
    );
}

#[sqlx::test(migrations = false)]
async fn missing_credit_sources_preserve_verified_payment_metrics(pool: PgPool) {
    crate::core_schema::init(&pool).await.unwrap();
    let identity = crate::identity::IdentityStore::from_pool(pool.clone());
    let admin = identity.bootstrap_user(8, 5).await.unwrap();
    // The account has neither native registration evidence nor verified opening totals.
    sqlx::query(
        "WITH account AS (
            INSERT INTO core_identity.accounts(kind) VALUES ('personal') RETURNING id
        )
        INSERT INTO core_identity.users(personal_account_id) SELECT id FROM account",
    )
    .execute(&pool)
    .await
    .unwrap();

    sqlx::raw_sql(
        r#"INSERT INTO core_history.finance_sources
        (source_key, contract, source_archive_sha256, mapping_sha256, tables_sha256,
         normalization_sha256, rows_sha256, observed_at, user_count, topup_count,
         credits_per_usd, complete, native_request_ids)
        VALUES (repeat('a', 64), 'core.site-finance-source.v1', repeat('b', 64),
          repeat('c', 64), repeat('d', 64), repeat('e', 64), repeat('f', 64),
          1, 0, 2, 500000, true, '{}'::text[]);
        INSERT INTO core_history.finance_topups
        (source_key, source_order_id, source_row_sha256, classification, currency,
         gross_amount_micros, refunded_amount_micros)
        VALUES (repeat('a', 64), 1, repeat('1', 64), 'cash', 'CNY', 7000000, 1000000),
               (repeat('a', 64), 2, repeat('2', 64), 'unconfirmed', NULL, NULL, NULL)"#,
    )
    .execute(&pool)
    .await
    .unwrap();

    let store = HistoryStore::new(pool);
    let path = "/api/finance/site-statistics";
    let response = request(store.clone(), path, Some(&admin.secret)).await;
    assert_eq!(response.status(), StatusCode::SERVICE_UNAVAILABLE);
    let response = request(
        store,
        "/api/finance/site-statistics?view=verified",
        Some(&admin.secret),
    )
    .await;
    assert_eq!(response.status(), StatusCode::OK);
    assert_eq!(response.headers()[header::CACHE_CONTROL], "no-store");
    let body: Value =
        serde_json::from_slice(&to_bytes(response.into_body(), 8192).await.unwrap()).unwrap();
    let data = &body["data"];
    assert_eq!(
        data["credit_coverage"],
        json!({"unverified_origins":1,"missing_usage_baselines":0,"incomplete_wallets":0,"invalid_settlements":0})
    );
    assert_eq!(data.get("total_used_credits"), Some(&Value::Null));
    assert_eq!(data.get("total_balance_credits"), Some(&Value::Null));
    assert_eq!(
        data["recharge"]["currencies"],
        json!([{
            "currency": "CNY",
            "gross_amount_micros": "7000000",
            "refunded_amount_micros": "1000000",
            "net_amount_micros": "6000000",
            "orders": 1
        }])
    );
    assert_eq!(data["recharge"]["virtual_units"], json!([]));
    assert_eq!(data["recharge"]["confirmed_orders"], 1);
    assert_eq!(
        data["order_exceptions"],
        json!({"unconfirmed_orders":1,"invalid_orders":0})
    );
}

#[sqlx::test(migrations = false)]
async fn both_views_require_shared_historical_lineage(pool: PgPool) {
    crate::core_schema::init(&pool).await.unwrap();
    let identity = crate::identity::IdentityStore::from_pool(pool.clone());
    let admin = identity.bootstrap_user(8, 5).await.unwrap();
    // The archived population declares one user, yet supplies no matching row.
    sqlx::query(
        r#"INSERT INTO core_history.finance_sources
        (source_key, contract, source_archive_sha256, mapping_sha256, tables_sha256,
         normalization_sha256, rows_sha256, observed_at, user_count, topup_count,
         credits_per_usd, complete, native_request_ids)
        VALUES (repeat('a', 64), 'core.site-finance-source.v1', repeat('b', 64),
          repeat('c', 64), repeat('d', 64), repeat('e', 64), repeat('f', 64),
          1, 1, 0, 500000, true, '{}'::text[])"#,
    )
    .execute(&pool)
    .await
    .unwrap();
    let store = HistoryStore::new(pool);
    for path in [
        "/api/finance/site-statistics",
        "/api/finance/site-statistics?view=verified",
    ] {
        let response = request(store.clone(), path, Some(&admin.secret)).await;
        assert_eq!(response.status(), StatusCode::SERVICE_UNAVAILABLE);
        let value: Value =
            serde_json::from_slice(&to_bytes(response.into_body(), 8192).await.unwrap()).unwrap();
        assert!(value.get("data").is_none());
    }
}

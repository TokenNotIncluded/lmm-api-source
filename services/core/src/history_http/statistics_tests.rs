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
    for path in ["/api/finance/site-statistics", "/api/perf-metrics/summary"] {
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

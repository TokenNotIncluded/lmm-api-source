//! Existing billing generations remain effective after the manual reset API retires.
use super::*;
use axum::{
    body::Body,
    http::{Method, Request as HttpRequest, StatusCode},
};
use tower::ServiceExt;

async fn consumed(pool: &PgPool, id: i64) -> (i64, i64, Option<i64>) {
    sqlx::query_as("SELECT c.used_credits::bigint,c.reserved_credits::bigint,c.reset_event_id FROM core_charging.subscription_window($1,floor(extract(epoch FROM clock_timestamp()))::bigint) w CROSS JOIN LATERAL core_charging.subscription_consumption($1,w.start_at,NULL) c")
        .bind(id).fetch_one(pool).await.unwrap()
}
fn request(id: &str, reserve: i64) -> Request {
    Request {
        id: id.into(),
        fingerprint: [9; 32],
        reserve,
        lease_seconds: 300,
        maximum_seconds: 3600,
        price: Price {
            rounding: None,
            text_request: None,
            tool: None,
            audio: None,
            grounding: None,
            user_trust: None,
            image: None,
            upstream_cost_group_ppm: None,
            tiered: None,
            version: "synthetic-reset-v1".into(),
            model: "test-model".into(),
            group: "default".into(),
            input_per_million: 1_000_000,
            cached_per_million: 500_000,
            output_per_million: 1_000_000,
        },
    }
}
async fn settle(charging: &Charging, id: &str, n: i64) {
    assert!(charging.start(id, &[11; 32]).await.unwrap());
    charging
        .settle(
            id,
            &[11; 32],
            Usage {
                output: n,
                ..Usage::default()
            },
            charging::Outcome::Completed,
        )
        .await
        .unwrap();
}

#[tokio::test]
async fn manual_reset_routes_are_absent() {
    let pool = sqlx::postgres::PgPoolOptions::new()
        .connect_lazy("postgresql://localhost/unused")
        .unwrap();
    let app = subscriptions::router(pool);
    for (method, path) in [
        (Method::GET, "/api/subscription/self/reset-vouchers"),
        (
            Method::POST,
            "/api/subscription/self/reset-vouchers/1/redeem",
        ),
        (Method::GET, "/api/subscription/root/reset-targets"),
        (Method::POST, "/api/subscription/root/reset/preview"),
        (Method::POST, "/api/subscription/root/reset"),
        (Method::POST, "/core/v1/subscriptions/reset-codes/redeem"),
    ] {
        let request = HttpRequest::builder()
            .method(method)
            .uri(path)
            .body(Body::empty())
            .unwrap();
        assert_eq!(
            app.clone().oneshot(request).await.unwrap().status(),
            StatusCode::NOT_FOUND,
            "{path}"
        );
    }
}

#[sqlx::test(migrations = false)]
async fn historical_generation_keeps_late_refunds_and_new_usage_separate(pool: PgPool) {
    let mut f = fixture(&pool, true).await;
    let role = runtime::RuntimeRole::create(&pool).await;
    role.optional("core_charging").await;
    role.optional("core_subscriptions").await;
    f.store = SubscriptionStore::new(role.pool.clone());
    let plan = f
        .store
        .save_plan(&f.admin, None, create("historical_generation_plan"))
        .await
        .unwrap();
    let quote = f
        .store
        .quote(&f.user, quote(plan.id, "historical_generation_quote", None))
        .await
        .unwrap();
    let grant = f
        .store
        .confirm(&f.user, &quote.id)
        .await
        .unwrap()
        .subscription_id
        .unwrap();
    let key = f
        .identity
        .issue_key(
            &f.user,
            Account {
                kind: AccountKind::Personal,
                id: 8302,
            },
            FundingPolicy {
                version: 1,
                account_order: None,
                personal_billing_preference: Some(BillingPreference::SubscriptionOnly),
            },
            3600,
        )
        .await
        .unwrap();
    sqlx::query("INSERT INTO core_identity.user_routing_groups(user_id,group_name,enabled,source_reference) VALUES(8302,'default',true,'synthetic-reset')").execute(&pool).await.unwrap();
    let charging = Charging::new(role.pool.clone(), Arc::new(TransactionalLedger));
    let first = request("before-historical-reset", 40);
    charging
        .reserve(&key.secret, &first, &[11; 32])
        .await
        .unwrap();
    settle(&charging, &first.id, 5).await;
    assert_eq!(consumed(&pool, grant).await, (5, 0, None));

    // Only the fixture owner seeds the event which an earlier version recorded.
    let event: i64 = sqlx::query_scalar(r#"
        INSERT INTO core_subscriptions.reset_events
          (operation_id,mode,actor_user_id,user_id,plan_id,subscription_id,window_start,window_end,
           before_limit,before_unlimited,before_used,after_limit,after_unlimited,restored_quota,created_at)
        SELECT 'prior-owner-event','hard',8301,8302,$2,$1,w.start_at,w.end_at,
               w.limit_credits,w.unlimited,5,w.limit_credits,w.unlimited,5,t.at
        FROM (SELECT floor(extract(epoch FROM clock_timestamp()))::bigint AS at) t
        CROSS JOIN LATERAL core_charging.subscription_window($1,t.at) w RETURNING id
    "#).bind(grant).bind(plan.id).fetch_one(&pool).await.unwrap();
    assert_eq!(consumed(&pool, grant).await, (0, 0, Some(event)));

    let second = request("after-historical-reset", 40);
    charging
        .reserve(&key.secret, &second, &[11; 32])
        .await
        .unwrap();
    settle(&charging, &second.id, 6).await;
    charging
        .refund(&first.id, "historical-refund", 5, "provider refund")
        .await
        .unwrap();
    assert_eq!(consumed(&pool, grant).await, (6, 0, Some(event)));
    assert_eq!(f.store.purchases(&f.user).await.unwrap()[0].used_units, "6");

    let can_write: bool = sqlx::query_scalar("SELECT has_table_privilege('core_subscriptions.reset_events','INSERT') OR has_table_privilege('core_subscriptions.reset_vouchers','INSERT') OR has_sequence_privilege('core_subscriptions.reset_events_id_seq','USAGE')").fetch_one(&role.pool).await.unwrap();
    assert!(!can_write);
    balanced(&pool).await;
    role.cleanup().await;
}

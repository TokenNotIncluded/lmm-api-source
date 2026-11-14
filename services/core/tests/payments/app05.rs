use super::*;
use lmm_core::payments::discounts::{self, Definition};

async fn fixture(pool: &PgPool) -> Fixture {
    let mut f = fresh_with_level(pool, 6).await;
    sqlx::raw_sql(sqlx::AssertSqlSafe(discounts::SCHEMA))
        .execute(pool)
        .await
        .unwrap();
    sqlx::raw_sql(sqlx::AssertSqlSafe(discounts::MANAGEMENT_SCHEMA))
        .execute(pool)
        .await
        .unwrap();
    f.store = quantity_store(pool, quantity_policy(7, 1));
    f
}
fn definition() -> Definition {
    Definition {
        code: " save20 ".into(),
        name: "Platform top-up".into(),
        discount_percent: 20,
        min_credit_units: 500_000,
        max_uses: 1,
        enabled: true,
        starts_at: 0,
        ends_at: 0,
    }
}

#[sqlx::test]
async fn app05_quote_prepare_credit_consumes_atomically(pool: PgPool) {
    let f = fixture(&pool).await;
    let code = f
        .store
        .save_discount(&f.secret, None, definition())
        .await
        .unwrap();
    assert_eq!(code.code, "SAVE20");
    let audit: (i64, serde_json::Value) = sqlx::query_as(
        "SELECT actor_user_id,target FROM core_identity.audit WHERE action='payment_discount.create' AND (target->>'code_id')::bigint=$1",
    ).bind(code.id).fetch_one(&pool).await.unwrap();
    assert_eq!(audit.0, 101);
    assert_eq!(audit.1["discount_percent"], 20);
    assert!(
        audit.1.get("code").is_none(),
        "audit must not copy redeemable codes"
    );
    let quote = f
        .store
        .quote_discount(&f.secret, 5003, 1_000_000, "save20")
        .await
        .unwrap();
    // 20 means 20% OFF, stacked on the configured 99% payable ratio.
    assert_eq!((quote.amount_minor, quote.credit_units), (1109, 1_000_000));
    let count: i64 = sqlx::query_scalar("SELECT count(*) FROM core_payments.discount_reservations")
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(count, 0, "preview never reserves or consumes");
    let request = quantity_request(&f, "app05_prepare_payment_01", 1_000_000);
    let intent = f
        .store
        .prepare_with_discount(&f.secret, request.clone(), "save20")
        .await
        .unwrap();
    assert_eq!(
        (intent.amount_minor, intent.credit_units),
        (1109, 1_000_000)
    );
    assert_eq!(
        f.store
            .prepare_with_discount(&f.secret, request.clone(), " SAVE20 ")
            .await
            .unwrap(),
        intent
    );
    assert_eq!(
        f.store
            .prepare(&f.secret, request.clone())
            .await
            .unwrap_err(),
        Error::Conflict
    );
    let mut edited = definition();
    edited.enabled = false;
    edited.discount_percent = 50;
    f.store
        .save_discount(&f.secret, Some(code.id), edited)
        .await
        .unwrap();
    assert_eq!(
        f.store
            .prepare_with_discount(&f.secret, request, "SAVE20")
            .await
            .unwrap(),
        intent
    );
    let consumed: bool = sqlx::query_scalar(
        "SELECT consumed FROM core_payments.discount_reservations WHERE intent_id=$1",
    )
    .bind(intent.intent_id)
    .fetch_one(&pool)
    .await
    .unwrap();
    assert!(!consumed);
    assert_eq!(journals(&pool).await, 0);
    // A failure in the same credit transaction must roll back the actual ledger.
    sqlx::raw_sql("CREATE FUNCTION core_payments.app05_fail_consume() RETURNS trigger LANGUAGE plpgsql AS $$ BEGIN RAISE EXCEPTION 'synthetic consumption storage failure'; END $$; CREATE TRIGGER app05_fail_consume BEFORE UPDATE ON core_payments.discount_reservations FOR EACH ROW EXECUTE FUNCTION core_payments.app05_fail_consume();")
        .execute(&pool).await.unwrap();
    let credit = credit_request(
        &intent,
        "app05_credit_payment_01",
        quantity_paid_proof(&intent),
    );
    assert!(f.store.credit(credit.clone()).await.is_err());
    assert_eq!(journals(&pool).await, 0);
    let state: (bool, bool) = sqlx::query_as("SELECT i.credit_journal_id IS NULL,r.consumed FROM core_payments.intents i JOIN core_payments.discount_reservations r ON r.intent_id=i.id WHERE i.id=$1")
        .bind(intent.intent_id).fetch_one(&pool).await.unwrap();
    assert_eq!(state, (true, false));
    sqlx::raw_sql("DROP TRIGGER app05_fail_consume ON core_payments.discount_reservations; DROP FUNCTION core_payments.app05_fail_consume();")
        .execute(&pool).await.unwrap();
    let receipt = f.store.credit(credit.clone()).await.unwrap();
    assert_eq!(f.store.credit(credit).await.unwrap(), receipt);
    balance(&f, 1_000_000, 0).await;
    let count: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM core_payments.discount_reservations WHERE consumed",
    )
    .fetch_one(&pool)
    .await
    .unwrap();
    assert_eq!(count, 1);
}

#[sqlx::test]
async fn app05_parallel_limit_and_failed_prepare_rollback(pool: PgPool) {
    let f = fixture(&pool).await;
    f.store
        .save_discount(&f.secret, None, definition())
        .await
        .unwrap();
    let mut wrong = quantity_request(&f, "app05_wrong_preview_01", 1_000_000);
    wrong.amount_minor = 1110;
    assert_eq!(
        f.store
            .prepare_with_discount(&f.secret, wrong, "SAVE20")
            .await
            .unwrap_err(),
        Error::Invalid
    );
    let a = quantity_request(&f, "app05_parallel_one_01", 1_000_000);
    let b = quantity_request(&f, "app05_parallel_two_01", 1_000_000);
    let retry_a = a.clone();
    let retry_b = b.clone();
    let (a, b) = tokio::join!(
        f.store
            .prepare_with_discount(&f.secret, a.clone(), "SAVE20"),
        f.store
            .prepare_with_discount(&f.secret, b.clone(), "SAVE20")
    );
    assert_eq!(usize::from(a.is_ok()) + usize::from(b.is_ok()), 1);
    let (failure, retry) = if let Err(error) = a {
        (error, retry_a)
    } else {
        (b.unwrap_err(), retry_b)
    };
    assert!(matches!(failure, Error::Invalid | Error::Unavailable));
    // Core's existing SERIALIZABLE contract reports whole-call retry on 40001.
    // Once the winner commits, the unchanged losing request must see exhaustion.
    assert_eq!(
        f.store
            .prepare_with_discount(&f.secret, retry, "SAVE20")
            .await
            .unwrap_err(),
        Error::Invalid
    );
    let counts: (i64, i64) = sqlx::query_as(
        "SELECT count(*),count(*) FILTER(WHERE consumed) FROM core_payments.discount_reservations",
    )
    .fetch_one(&pool)
    .await
    .unwrap();
    assert_eq!(
        counts,
        (1, 0),
        "pending is a reservation, never a consumed use"
    );
    assert_eq!(journals(&pool).await, 0);
}

#[sqlx::test]
async fn app05_authority_window_minimum_and_http(pool: PgPool) {
    use axum::{body::Body, http::Request};
    use tower::ServiceExt;
    let f = fixture(&pool).await;
    let ordinary = f.identity.bootstrap_user(102, 0).await.unwrap();
    assert_eq!(
        f.store
            .save_discount(&ordinary.secret, None, definition())
            .await
            .unwrap_err(),
        Error::Denied
    );
    let code = f
        .store
        .save_discount(&f.secret, None, definition())
        .await
        .unwrap();
    assert_eq!(
        f.store
            .quote_discount(&f.secret, 5003, 100_000, "SAVE20")
            .await
            .unwrap_err(),
        Error::Denied
    );
    let mut future = definition();
    future.starts_at = 253402300700;
    f.store
        .save_discount(&f.secret, Some(code.id), future)
        .await
        .unwrap();
    assert_eq!(
        f.store
            .quote_discount(&f.secret, 5003, 1_000_000, "SAVE20")
            .await
            .unwrap_err(),
        Error::Denied
    );
    let mut expired = definition();
    expired.ends_at = 1;
    f.store
        .save_discount(&f.secret, Some(code.id), expired)
        .await
        .unwrap();
    assert_eq!(
        f.store
            .quote_discount(&f.secret, 5003, 1_000_000, "SAVE20")
            .await
            .unwrap_err(),
        Error::Denied
    );
    f.store
        .save_discount(&f.secret, Some(code.id), definition())
        .await
        .unwrap();
    let router = discounts::http::router(f.store.0.clone());
    let response = router.clone().oneshot(Request::builder().method("POST").uri("/core/v1/payment-discounts/quote")
        .header("authorization", format!("Bearer {}", f.secret)).header("content-type", "application/json")
        .body(Body::from(r#"{"payment_channel_id":5003,"credit_units":1000000,"discount_code":"SAVE20"}"#)).unwrap()).await.unwrap();
    assert_eq!(response.status(), 200);
    let value: Value = serde_json::from_slice(
        &axum::body::to_bytes(response.into_body(), 4096)
            .await
            .unwrap(),
    )
    .unwrap();
    assert_eq!(value["data"]["amount_minor"], 1109);
    let response = router.oneshot(Request::builder().method("POST").uri("/core/v1/payment-discounts/quote")
        .header("content-type", "application/json").body(Body::from(r#"{"payment_channel_id":5003,"credit_units":1000000,"discount_code":"SAVE20"}"#)).unwrap()).await.unwrap();
    assert_eq!(response.status(), 403);
}

#[sqlx::test]
async fn app05_personal_wire_discount_and_known_rejection(pool: PgPool) {
    let f = fixture(&pool).await;
    f.store
        .save_discount(&f.secret, None, definition())
        .await
        .unwrap();
    let request = pb::PreparePersonalTopupRequest {
        mutation: mutation("app05_personal_wire_01"),
        payment_channel_id: 5003,
        currency: "CNY".into(),
        amount_minor: 0,
        requested_credit_units: 1_000_000,
        discount_code: " save20 ".into(),
    };
    let intent = f
        .store
        .prepare_personal(&f.secret, request.clone())
        .await
        .unwrap();
    assert_eq!(
        (intent.account_id, intent.amount_minor, intent.credit_units),
        (f.account, 1109, 1_000_000)
    );
    assert_eq!(
        f.store
            .prepare_personal(&f.secret, request.clone())
            .await
            .unwrap(),
        intent
    );
    let mut changed = request.clone();
    changed.discount_code.clear();
    assert_eq!(
        f.store
            .prepare_personal(&f.secret, changed)
            .await
            .unwrap_err(),
        Error::Conflict
    );
    for code in ["MISSING", "SAVE20"] {
        let mut rejected = request.clone();
        rejected.mutation = mutation("app05_known_rejection_01");
        rejected.discount_code = code.into();
        assert_eq!(
            f.store
                .prepare_personal(&f.secret, rejected)
                .await
                .unwrap_err(),
            Error::Invalid
        );
    }
    let count: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM core_payments.intents WHERE prepare_key='app05_known_rejection_01'",
    )
    .fetch_one(&pool)
    .await
    .unwrap();
    assert_eq!(count, 0);
}

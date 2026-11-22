use super::*;
use crate::{
    billing::ledger::Ledger,
    core_schema,
    identity::{BrowserRegistration, BrowserSessionMetadata, IdentityStore},
};
use axum::{
    body::{Body, to_bytes},
    http::Request as HttpRequest,
};
use tower::ServiceExt;

struct Fixture {
    store: Store,
    cookie: String,
    bearer: String,
    browser: BrowserAuth,
    account: i64,
}
async fn fixture(pool: &PgPool, available: i64, affiliate: i64) -> Fixture {
    core_schema::init(pool).await.unwrap();
    sqlx::query("INSERT INTO core_console.options(key,value) VALUES('payment_setting.compliance_confirmed','true'),('payment_setting.compliance_terms_version','v1')").execute(pool).await.unwrap();
    let identity = IdentityStore::from_pool(pool.clone()).with_registration_enabled(true);
    let browser = BrowserAuth::new(identity.clone(), &[43; 32]).unwrap();
    browser
        .register(&BrowserRegistration {
            username: "affiliate_owner",
            password: "synthetic password",
            email: "",
        })
        .await
        .unwrap();
    let bundle = browser
        .login(
            "affiliate_owner",
            "synthetic password",
            &BrowserSessionMetadata::default(),
        )
        .await
        .unwrap();
    let bearer = bundle.data["access_token"].as_str().unwrap().to_owned();
    let account = identity.authorize(&bearer).await.unwrap().owner_account_id;
    let opening = serde_json::json!({"migration_id":"affiliate-test-opening","source_entity_key":"user:1","source_archive_sha256":"a".repeat(64),"mapping_sha256":"b".repeat(64),"source_row_sha256":"c".repeat(64),"observed_at":1700000000,"account_id":account,"available_units":available,"reserved_units":0,"affiliate_units":affiliate});
    let mut tx = pool.begin().await.unwrap();
    sqlx::raw_sql("SET TRANSACTION ISOLATION LEVEL SERIALIZABLE; SET LOCAL synchronous_commit=on")
        .execute(&mut *tx)
        .await
        .unwrap();
    sqlx::query("SELECT core_billing.import_opening_balance($1)")
        .bind(SqlJson(opening))
        .execute(&mut *tx)
        .await
        .unwrap();
    tx.commit().await.unwrap();
    Fixture {
        store: Store::new(pool.clone()),
        cookie: bundle.refresh_token,
        bearer,
        browser,
        account,
    }
}
async fn journals(pool: &PgPool) -> i64 {
    sqlx::query_scalar(
        "SELECT count(*) FROM core_billing.ledger_journals WHERE scope='core-affiliate-claim'",
    )
    .fetch_one(pool)
    .await
    .unwrap()
}

#[sqlx::test(migrations = false)]
async fn partial_claim_is_exact_once_and_repays_debt(pool: PgPool) {
    let f = fixture(&pool, -500_000, 6_000_000).await;
    assert!(matches!(
        f.store.preview(&f.cookie, Some(499_999)).await,
        Err(Error::Invalid)
    ));
    let preview = f.store.preview(&f.cookie, Some(1_500_000)).await.unwrap();
    assert_eq!(journals(&pool).await, 0);
    let receipt = f
        .store
        .confirm(&f.cookie, preview.id, &preview.csrf)
        .await
        .unwrap();
    assert!(matches!(receipt.outcome, Outcome::Posted { .. }));
    let replay = f
        .store
        .confirm(&f.cookie, preview.id, &preview.csrf)
        .await
        .unwrap();
    assert_eq!(
        serde_json::to_value(&receipt).unwrap(),
        serde_json::to_value(&replay).unwrap()
    );
    let balance = Ledger::from_pool(pool.clone())
        .balance(f.account)
        .await
        .unwrap();
    assert_eq!(
        (
            balance.available_units,
            balance.spendable_units,
            balance.debt_units,
            balance.affiliate_units
        ),
        (1_000_000, 1_000_000, 0, 4_500_000)
    );
    assert_eq!(journals(&pool).await, 1);
    let unbalanced:i64=sqlx::query_scalar("SELECT count(*) FROM (SELECT journal_id FROM core_billing.ledger_entries GROUP BY journal_id HAVING sum(delta_units)<>0) x").fetch_one(&pool).await.unwrap();
    assert_eq!(unbalanced, 0);
}
#[sqlx::test(migrations = false)]
async fn competing_claims_cannot_overdraw_and_rejection_is_durable(pool: PgPool) {
    let f = fixture(&pool, 0, 6_000_000).await;
    let a = f.store.preview(&f.cookie, Some(4_000_000)).await.unwrap();
    let b = f.store.preview(&f.cookie, Some(4_000_000)).await.unwrap();
    let (first, second) = tokio::join!(
        f.store.confirm(&f.cookie, a.id, &a.csrf),
        f.store.confirm(&f.cookie, b.id, &b.csrf)
    );
    let outcomes = [first.unwrap(), second.unwrap()];
    assert_eq!(
        outcomes
            .iter()
            .filter(|r| matches!(r.outcome, Outcome::Posted { .. }))
            .count(),
        1
    );
    assert_eq!(
        outcomes
            .iter()
            .filter(|r| matches!(r.outcome, Outcome::Rejected { .. }))
            .count(),
        1
    );
    for preview in [&a, &b] {
        let original = outcomes.iter().find(|r| r.id == preview.id).unwrap();
        let replay = f
            .store
            .confirm(&f.cookie, preview.id, &preview.csrf)
            .await
            .unwrap();
        assert_eq!(
            serde_json::to_value(original).unwrap(),
            serde_json::to_value(replay).unwrap()
        );
    }
    let balance = Ledger::from_pool(pool.clone())
        .balance(f.account)
        .await
        .unwrap();
    assert_eq!(
        (balance.spendable_units, balance.affiliate_units),
        (4_000_000, 2_000_000)
    );
    assert_eq!(journals(&pool).await, 1);
}
#[sqlx::test(migrations = false)]
async fn cookie_session_csrf_and_origin_are_required(pool: PgPool) {
    let f = fixture(&pool, 0, 6_000_000).await;
    let preview = f.store.preview(&f.cookie, Some(1_000_000)).await.unwrap();
    assert!(matches!(
        f.store
            .confirm(&f.cookie, preview.id, &"0".repeat(64))
            .await,
        Err(Error::Forbidden)
    ));
    let other = f
        .browser
        .login(
            "affiliate_owner",
            "synthetic password",
            &BrowserSessionMetadata::default(),
        )
        .await
        .unwrap();
    assert!(matches!(
        f.store
            .confirm(&other.refresh_token, preview.id, &preview.csrf)
            .await,
        Err(Error::Forbidden)
    ));
    let app = router(f.store.clone(), "https://affiliate.test".into(), false).unwrap();
    let response = app
        .clone()
        .oneshot(
            HttpRequest::builder()
                .uri("/core/v1/affiliate/claim?amount_credits=1000000")
                .header("authorization", format!("Bearer {}", f.bearer))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
    let response = app
        .clone()
        .oneshot(
            HttpRequest::builder()
                .uri(format!("/core/v1/affiliate/claim/{}", preview.id))
                .method("POST")
                .header("cookie", format!("new_api_refresh={}", other.refresh_token))
                .header("origin", "https://evil.test")
                .header("content-type", "application/json")
                .body(Body::from(
                    serde_json::json!({"csrf":preview.csrf}).to_string(),
                ))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::FORBIDDEN);
    let response = app
        .clone()
        .oneshot(
            HttpRequest::builder()
                .uri("/core/v1/affiliate/claim?account_id=123")
                .header("cookie", format!("new_api_refresh={}", other.refresh_token))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::BAD_REQUEST);
    let response = app
        .oneshot(
            HttpRequest::builder()
                .uri("/core/v1/affiliate/claim")
                .header("cookie", format!("new_api_refresh={}", other.refresh_token))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    assert_eq!(response.headers()[header::CACHE_CONTROL], "no-store");
    let body = String::from_utf8(
        to_bytes(response.into_body(), 16384)
            .await
            .unwrap()
            .to_vec(),
    )
    .unwrap();
    assert!(body.contains("6000000 CREDIT"));
    assert_eq!(journals(&pool).await, 0);
}
#[sqlx::test(migrations = false)]
async fn negative_entitlement_and_expired_preview_cannot_claim(pool: PgPool) {
    let f = fixture(&pool, 0, -500_000).await;
    assert!(matches!(
        f.store.preview(&f.cookie, None).await,
        Err(Error::Invalid)
    ));
    let mut tx = pool.begin().await.unwrap();
    let actor = BrowserAuth::oauth_identity_in_transaction(&mut tx, &f.cookie)
        .await
        .unwrap();
    let csrf = "a".repeat(64);
    let id:i64=sqlx::query_scalar("INSERT INTO core_wallet.affiliate_claims(owner_user,account_id,session_id,amount,csrf_digest,expires_at) VALUES($1,$2,$3,500000,$4,1) RETURNING id").bind(actor.user).bind(actor.account).bind(actor.sid).bind(Sha256::digest(csrf.as_bytes()).to_vec()).fetch_one(&mut *tx).await.unwrap();
    tx.commit().await.unwrap();
    assert!(matches!(
        f.store.confirm(&f.cookie, id, &csrf).await,
        Err(Error::Forbidden)
    ));
    assert_eq!(journals(&pool).await, 0);
}

#[sqlx::test(migrations = false)]
async fn policy_gate_is_authoritative_and_does_not_hide_completed_receipts(pool: PgPool) {
    let f = fixture(&pool, 0, 6_000_000).await;
    let completed = f.store.preview(&f.cookie, Some(500_000)).await.unwrap();
    let receipt = f
        .store
        .confirm(&f.cookie, completed.id, &completed.csrf)
        .await
        .unwrap();
    let pending = f.store.preview(&f.cookie, Some(500_000)).await.unwrap();
    sqlx::query("UPDATE core_console.options SET value='false' WHERE key='payment_setting.compliance_confirmed'").execute(&pool).await.unwrap();
    assert!(matches!(
        f.store.preview(&f.cookie, None).await,
        Err(Error::Unavailable)
    ));
    assert!(matches!(
        f.store.confirm(&f.cookie, pending.id, &pending.csrf).await,
        Err(Error::Unavailable)
    ));
    let replay = f
        .store
        .confirm(&f.cookie, completed.id, &completed.csrf)
        .await
        .unwrap();
    assert_eq!(
        serde_json::to_value(receipt).unwrap(),
        serde_json::to_value(replay).unwrap()
    );
    sqlx::query("UPDATE core_console.options SET value='true' WHERE key='payment_setting.compliance_confirmed'").execute(&pool).await.unwrap();
    sqlx::query("UPDATE core_console.options SET value='old' WHERE key='payment_setting.compliance_terms_version'").execute(&pool).await.unwrap();
    assert!(matches!(
        f.store.preview(&f.cookie, None).await,
        Err(Error::Unavailable)
    ));
    sqlx::query(
        "DELETE FROM core_console.options WHERE key='payment_setting.compliance_terms_version'",
    )
    .execute(&pool)
    .await
    .unwrap();
    assert!(matches!(
        f.store.preview(&f.cookie, None).await,
        Err(Error::Unavailable)
    ));
    assert_eq!(journals(&pool).await, 1);
}

#[sqlx::test(migrations = false)]
async fn affiliate_code_and_registration_are_persistent_and_atomic(pool: PgPool) {
    let f = fixture(&pool, 0, 1_000_000).await;
    let app = router(f.store.clone(), "https://affiliate.test".into(), false).unwrap();
    let mut codes = Vec::new();
    for _ in 0..2 {
        let response = app
            .clone()
            .oneshot(
                HttpRequest::builder()
                    .uri("/api/user/aff")
                    .header("cookie", format!("new_api_refresh={}", f.cookie))
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::OK);
        let body = to_bytes(response.into_body(), 4096).await.unwrap();
        let value: serde_json::Value = serde_json::from_slice(&body).unwrap();
        codes.push(value["data"].as_str().unwrap().to_owned());
    }
    assert_eq!(codes[0], codes[1]);
    let response = app
        .clone()
        .oneshot(
            HttpRequest::builder()
                .uri("/core/v1/affiliate/preview")
                .method("POST")
                .header("cookie", format!("new_api_refresh={}", f.cookie))
                .header("origin", "https://affiliate.test")
                .header("content-type", "application/json")
                .body(Body::from(r#"{"amount_credits":"500000"}"#))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let value: serde_json::Value =
        serde_json::from_slice(&to_bytes(response.into_body(), 4096).await.unwrap()).unwrap();
    assert_eq!(value["amount"], 500000);
    assert!(value["id"].as_i64().unwrap() > 0);
    assert_eq!(value["csrf"].as_str().unwrap().len(), 64);

    let input = BrowserRegistration {
        username: "referred",
        password: "synthetic password",
        email: "",
    };
    f.browser
        .register_with_affiliate(&input, &codes[0])
        .await
        .unwrap();
    let owner: i64 =
        sqlx::query_scalar("SELECT user_id FROM core_identity.affiliate_codes WHERE code=$1")
            .bind(&codes[0])
            .fetch_one(&pool)
            .await
            .unwrap();
    let bound: i64 = sqlx::query_scalar("SELECT inviter_id FROM core_identity.affiliate_registrations r JOIN core_identity.user_profiles p ON p.user_id=r.user_id WHERE p.username='referred'")
        .fetch_one(&pool).await.unwrap();
    assert_eq!(owner, bound);
    let invalid = BrowserRegistration {
        username: "invalid_referral",
        ..input
    };
    assert!(
        f.browser
            .register_with_affiliate(&invalid, &"0".repeat(32))
            .await
            .is_err()
    );
    let exists: bool = sqlx::query_scalar(
        "SELECT EXISTS(SELECT FROM core_identity.user_profiles WHERE username='invalid_referral')",
    )
    .fetch_one(&pool)
    .await
    .unwrap();
    assert!(
        !exists,
        "invalid referral must roll back the identity transaction"
    );
    assert_eq!(journals(&pool).await, 0, "binding cannot mint funds");
}

#[path = "legacy_alias_tests.rs"]
mod legacy_alias_tests;

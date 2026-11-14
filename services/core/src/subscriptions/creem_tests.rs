//! Real PostgreSQL, original Creem HMAC, shared order/entitlement mutations.
//! Run explicitly with the main agent's isolated lmm_creem_test database.
use super::*;
use crate::{
    identity::IdentityStore,
    payments::{Channel, PaymentsStore, ProviderVerification},
};
use hmac::{Hmac, KeyInit as _, Mac};
use sha2::{Digest, Sha256};
const SECRET: &str = "creem_synthetic_subscription_secret_01";
fn channel() -> Channel {
    Channel {
        id: 18,
        provider: "creem".into(),
        merchant: "merchant_creem_test".into(),
        environment: "test".into(),
        currency: "USD".into(),
        webhook_secrets: vec![SECRET.into()],
        credits_per_minor: 5000,
        credit_quote: None,
        max_amount_minor: 10000,
        verification: ProviderVerification::default(),
    }
}
fn meta(id: &str) -> Value {
    json!({"lmm_subscription_order_id":id,"lmm_merchant":"merchant_creem_test","lmm_price_id":"prod_frozen_monthly"})
}
fn product() -> Value {
    json!({"id":"prod_frozen_monthly","object":"product","mode":"test","price":125,"currency":"USD","billing_type":"recurring","billing_period":"every-month"})
}
fn event(
    id: &str,
    kind: &str,
    order: &str,
    at: i64,
    start: i64,
    end: i64,
    transaction: &str,
) -> Value {
    let date = |n| chrono::DateTime::from_timestamp(n, 0).unwrap().to_rfc3339();
    json!({"id":id,"eventType":kind,"created_at":at,"object":{
 "id":"sub_creem_original","object":"subscription","mode":"test","status":"active","product":product(),"customer":{"id":"cust_original"},
 "collection_method":"charge_automatically","last_transaction_id":transaction,"current_period_start_date":date(start),"current_period_end_date":date(end),"metadata":meta(order)}})
}
fn sign(e: &Value) -> (Vec<u8>, Vec<u8>) {
    let raw = serde_json::to_vec(e).unwrap();
    let mut mac = Hmac::<Sha256>::new_from_slice(SECRET.as_bytes()).unwrap();
    mac.update(&raw);
    let signature = crate::hex_codec::encode(mac.finalize().into_bytes()).into_bytes();
    (raw, signature)
}
#[tokio::test]
#[ignore = "requires the exclusive lmm_creem_test PostgreSQL database and unified Core build"]
async fn creem_original_first_cycle_lifecycle_and_recovery_pg() {
    let url =
        std::env::var("LMM_CREEM_TEST_DATABASE_URL").expect("exclusive Creem test database URL");
    let pool = sqlx::postgres::PgPoolOptions::new()
        .max_connections(5)
        .connect(&url)
        .await
        .unwrap();
    crate::core_schema::init(&pool).await.unwrap();
    let identity = IdentityStore::from_pool(pool.clone());
    let admin = identity.bootstrap_user(8501, 6).await.unwrap().secret;
    let user = identity.bootstrap_user(8502, 0).await.unwrap().secret;
    let other = identity.bootstrap_user(8503, 0).await.unwrap().secret;
    let store = SubscriptionStore::new(pool.clone());
    let mut channels = vec![channel()];
    for (id, currency) in [(19, "CNY"), (20, "EUR")] {
        let mut c = channel();
        c.id = id;
        c.currency = currency.into();
        channels.push(c);
    }
    let payments = PaymentsStore::new(pool.clone(), channels).unwrap();
    let plan = store
        .save_plan(
            &admin,
            None,
            SavePlan {
                operation_key: "creem_plan_fixture_01".into(),
                expected_revision: None,
                terms: PlanTerms {
                    title: "Synthetic monthly Creem".into(),
                    price_units: "1000".into(),
                    duration_seconds: 0,
                    duration_months: 1,
                    reset_period: "grant".into(),
                    reset_seconds: 0,
                    limit_units: "10000".into(),
                    models: vec!["*".into()],
                    groups: vec!["*".into()],
                    max_purchases_per_user: 1,
                    enabled: true,
                    allow_balance_pay: true,
                },
            },
        )
        .await
        .unwrap();
    let price = CreemPrice {
        plan_id: plan.id,
        plan_revision: plan.revision,
        channel_id: 18,
        currency: "USD".into(),
        amount_minor: "125".into(),
        provider_price_id: "prod_frozen_monthly".into(),
        interval: "month".into(),
        interval_count: 1,
        success_url: "https://example.test/subscriptions".into(),
        cancel_url: "https://example.test/subscriptions".into(),
    };
    store
        .save_creem_price(&payments, &admin, "creem_price_fixture_01", &price)
        .await
        .unwrap();
    assert!(matches!(
        store
            .save_creem_price(&payments, &user, "creem_price_user_bad", &price)
            .await,
        Err(Error::Forbidden)
    ));
    let req = EPayPurchaseRequest {
        operation_key: "creem_order_fixture_01".into(),
        plan_id: plan.id,
        channel_id: 18,
    };
    let prepared = store
        .prepare_creem_purchase(&payments, &user, &req)
        .await
        .unwrap();
    let id = &prepared.order.order_id;
    assert_eq!(
        store
            .prepare_creem_purchase(&payments, &user, &req)
            .await
            .unwrap()
            .order
            .order_id,
        *id
    );
    assert!(matches!(
        store.read_creem_purchase(&other, id).await,
        Err(Error::NotFound)
    ));
    let t = now(&mut pool.acquire().await.unwrap()).await.unwrap();
    let at = t * 1000;
    let wallet_result = payments
        .prepare_personal(
            &user,
            crate::internal_rpc::pb::PreparePersonalTopupRequest {
                mutation: Some(crate::internal_rpc::pb::MutationContext {
                    idempotency_key: "creem_sub_only_wallet_denied".into(),
                    ..Default::default()
                }),
                payment_channel_id: 18,
                currency: "USD".into(),
                amount_minor: 125,
                ..Default::default()
            },
        )
        .await;
    assert!(
        matches!(wallet_result, Err(crate::payments::Error::Unavailable)),
        "subscription-only channel must reject new wallet orders"
    );
    let journal_count: i64 =
        sqlx::query_scalar("SELECT count(*) FROM core_billing.ledger_journals")
            .fetch_one(&pool)
            .await
            .unwrap();
    let checkout = json!({"id":"evt_creem_checkout","eventType":"checkout.completed","created_at":at,"object":{
 "id":"ch_original","object":"checkout","mode":"test","status":"completed","request_id":id,"product":product(),"customer":"cust_original","subscription":"sub_creem_original","metadata":meta(id),"order":{"type":"recurring","status":"paid","mode":"test","amount_paid":125,"currency":"USD"}}});
    let (raw, sig) = sign(&checkout);
    assert!(matches!(
        store
            .accept_creem_event(&payments, 18, id, &raw, &sig)
            .await,
        Err(Error::Conflict)
    ));
    store.confirm_creem_purchase(&user, id).await.unwrap();
    let proof = store
        .accept_creem_event(&payments, 18, id, &raw, &sig)
        .await
        .unwrap();
    assert_eq!(proof.kind, "checkout");
    assert_eq!(proof.subscription_id, None);
    assert_eq!(
        store
            .resolve_creem_subscription_order(18, "sub_creem_original")
            .await
            .unwrap(),
        *id
    );
    let paid = event(
        "evt_creem_first",
        "subscription.paid",
        id,
        at + 1,
        t - 30,
        t + 2_592_000,
        "tran_first",
    );
    let (raw, sig) = sign(&paid);
    assert!(
        store
            .accept_creem_event(&payments, 19, id, &raw, &sig)
            .await
            .is_err()
    );
    let accepted = store
        .accept_creem_event(&payments, 18, id, &raw, &sig)
        .await
        .unwrap();
    assert!(accepted.subscription_id.is_some());
    assert_eq!(
        store
            .accept_creem_event(&payments, 18, id, &raw, &sig)
            .await
            .unwrap(),
        accepted
    );
    let restarted = SubscriptionStore::new(pool.clone());
    assert_eq!(
        restarted
            .creem_event(18, id, &accepted.event_id, &Sha256::digest(&raw))
            .await
            .unwrap(),
        accepted
    );
    let mut changed = paid.clone();
    changed["object"]["product"]["price"] = json!(126);
    let (bad, bad_sig) = sign(&changed);
    assert!(matches!(
        store
            .accept_creem_event(&payments, 18, id, &bad, &bad_sig)
            .await,
        Err(Error::Conflict)
    ));
    for (field, value) in [
        ("mode", json!("prod")),
        ("last_transaction_id", json!("pi_wrong_provider")),
    ] {
        let mut bad = paid.clone();
        bad["id"] = json!("evt_creem_rejected");
        bad["object"][field] = value;
        let (raw, sig) = sign(&bad);
        assert!(
            store
                .accept_creem_event(&payments, 18, id, &raw, &sig)
                .await
                .is_err()
        );
    }
    let mut overlap = event(
        "evt_creem_overlap",
        "subscription.paid",
        id,
        at + 2,
        t - 10,
        t + 2_592_000,
        "tran_overlap",
    );
    let (raw, sig) = sign(&overlap);
    assert!(matches!(
        store
            .accept_creem_event(&payments, 18, id, &raw, &sig)
            .await,
        Err(Error::Conflict)
    ));
    overlap["id"] = json!("evt_creem_same_payment_other_delivery");
    overlap["object"]["last_transaction_id"] = json!("tran_first");
    overlap["object"]["current_period_start_date"] =
        paid["object"]["current_period_start_date"].clone();
    let (raw, sig) = sign(&overlap);
    assert_eq!(
        store
            .accept_creem_event(&payments, 18, id, &raw, &sig)
            .await
            .unwrap()
            .subscription_id,
        accepted.subscription_id
    );
    let mut canceled = event(
        "evt_creem_cancel",
        "subscription.canceled",
        id,
        at + 10,
        t - 30,
        t + 2_592_000,
        "tran_first",
    );
    canceled["object"]["status"] = json!("canceled");
    canceled["object"]["metadata"] = json!({});
    let (raw, sig) = sign(&canceled);
    assert_eq!(
        store
            .accept_creem_event(&payments, 18, id, &raw, &sig)
            .await
            .unwrap()
            .state,
        "canceled"
    );
    let active: bool =
        sqlx::query_scalar("SELECT active FROM core_charging.subscriptions WHERE id=$1")
            .bind(accepted.subscription_id)
            .fetch_one(&pool)
            .await
            .unwrap();
    assert!(
        active,
        "renewal cancellation must retain the prepaid period"
    );
    let resumed = event(
        "evt_creem_restore",
        "subscription.update",
        id,
        at + 11,
        t - 30,
        t + 2_592_000,
        "tran_first",
    );
    let (raw, sig) = sign(&resumed);
    assert_eq!(
        store
            .accept_creem_event(&payments, 18, id, &raw, &sig)
            .await
            .unwrap()
            .state,
        "active"
    );
    let renewed = event(
        "evt_creem_renew",
        "subscription.paid",
        id,
        at + 20,
        t + 2_592_000,
        t + 5_184_000,
        "tran_second",
    );
    let (raw, sig) = sign(&renewed);
    let second = store
        .accept_creem_event(&payments, 18, id, &raw, &sig)
        .await
        .unwrap();
    assert_ne!(second.subscription_id, accepted.subscription_id);
    // Old cancellation cannot roll a newer paid state backwards.
    canceled["id"] = json!("evt_creem_old_cancel");
    let (raw, sig) = sign(&canceled);
    assert_eq!(
        store
            .accept_creem_event(&payments, 18, id, &raw, &sig)
            .await
            .unwrap()
            .state,
        "active"
    );
    let cycles: i64 = sqlx::query_scalar("SELECT count(*) FROM core_subscriptions.creem_cycles")
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(cycles, 2);
    let current_journals: i64 =
        sqlx::query_scalar("SELECT count(*) FROM core_billing.ledger_journals")
            .fetch_one(&pool)
            .await
            .unwrap();
    assert_eq!(
        current_journals, journal_count,
        "Creem subscriptions must never credit wallet money"
    );
    assert_eq!(
        restarted
            .read_creem_purchase(&user, id)
            .await
            .unwrap()
            .latest_grant_id,
        second.subscription_id
    );
    for (channel_id, currency, user_id) in [(19, "CNY", 8504), (20, "EUR", 8505)] {
        let buyer = identity.bootstrap_user(user_id, 0).await.unwrap().secret;
        let mut p = price.clone();
        p.channel_id = channel_id;
        p.currency = currency.into();
        store
            .save_creem_price(
                &payments,
                &admin,
                &format!("creem_price_{currency}_fixture"),
                &p,
            )
            .await
            .unwrap();
        let o = store
            .prepare_creem_purchase(
                &payments,
                &buyer,
                &EPayPurchaseRequest {
                    operation_key: format!("creem_order_{currency}_fixture"),
                    plan_id: plan.id,
                    channel_id,
                },
            )
            .await
            .unwrap();
        store
            .confirm_creem_purchase(&buyer, &o.order.order_id)
            .await
            .unwrap();
        let mut paid = event(
            &format!("evt_creem_paid_{currency}"),
            "subscription.paid",
            &o.order.order_id,
            at + 30,
            t - 30,
            t + 2_592_000,
            &format!("tran_paid_{currency}"),
        );
        paid["object"]["id"] = json!(format!("sub_creem_{currency}"));
        paid["object"]["customer"] = json!(format!("cust_{currency}"));
        paid["object"]["product"]["currency"] = json!(currency);
        let (raw, sig) = sign(&paid);
        assert!(
            store
                .accept_creem_event(&payments, channel_id, &o.order.order_id, &raw, &sig)
                .await
                .unwrap()
                .subscription_id
                .is_some()
        );
        paid["id"] = json!(format!("evt_creem_cross_currency_{currency}"));
        paid["object"]["product"]["currency"] = json!("USD");
        let (raw, sig) = sign(&paid);
        assert!(matches!(
            store
                .accept_creem_event(&payments, channel_id, &o.order.order_id, &raw, &sig)
                .await,
            Err(Error::Conflict)
        ));
    }
}

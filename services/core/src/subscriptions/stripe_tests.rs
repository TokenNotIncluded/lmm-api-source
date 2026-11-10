//! Requires Root's frozen Core62 target and an isolated PostgreSQL lease.
//! Source only until that explicit validation window is granted.
use super::*;
use crate::{
    identity::IdentityStore,
    payments::{Channel, PaymentsStore},
};
use hmac::{Hmac, Mac};
use sha2::Sha256;

const SECRET: &str = "whsec_subscription_synthetic_test_01";
struct Fixture {
    store: SubscriptionStore,
    payments: PaymentsStore,
    user: String,
    other: String,
    admin: String,
    order: StripeOrder,
    clock: i64,
}
async fn fixture(pool: &PgPool) -> Fixture {
    crate::core_schema::init(pool).await.unwrap();
    let identity = IdentityStore::from_pool(pool.clone());
    let admin = identity.bootstrap_user(8401, 6).await.unwrap().secret;
    let user = identity.bootstrap_user(8402, 0).await.unwrap().secret;
    let other = identity.bootstrap_user(8403, 0).await.unwrap().secret;
    let payments = PaymentsStore::new(
        pool.clone(),
        vec![Channel::stripe_test(
            8,
            "acct_subscription_test".into(),
            SECRET.into(),
            10000,
        )],
    )
    .unwrap();
    let store = SubscriptionStore::new(pool.clone());
    let p = store
        .save_plan(
            &admin,
            None,
            SavePlan {
                operation_key: "stripe_plan_fixture_01".into(),
                expected_revision: None,
                terms: PlanTerms {
                    title: "Synthetic daily Stripe".into(),
                    price_units: "1000".into(),
                    duration_seconds: 86400,
                    duration_months: 0,
                    reset_period: "grant".into(),
                    reset_seconds: 0,
                    limit_units: "10000".into(),
                    models: vec!["*".into()],
                    groups: vec!["*".into()],
                    max_purchases_per_user: 1,
                    enabled: true,
                },
            },
        )
        .await
        .unwrap();
    store
        .save_stripe_price(
            &payments,
            &admin,
            "stripe_price_fixture_01",
            &StripePrice {
                plan_id: p.id,
                plan_revision: p.revision,
                channel_id: 8,
                currency: "USD".into(),
                amount_minor: "125".into(),
                provider_price_id: "price_frozen_daily".into(),
                interval: "day".into(),
                interval_count: 1,
                success_url: "https://lmm.example.test/subscriptions".into(),
                cancel_url: "https://lmm.example.test/subscriptions".into(),
            },
        )
        .await
        .unwrap();
    let o = store
        .prepare_stripe_purchase(
            &payments,
            &user,
            &EPayPurchaseRequest {
                operation_key: "stripe_order_fixture_01".into(),
                plan_id: p.id,
                channel_id: 8,
            },
        )
        .await
        .unwrap();
    let order = store
        .confirm_stripe_purchase(&user, &o.order.order_id)
        .await
        .unwrap();
    let clock = now(&mut pool.acquire().await.unwrap()).await.unwrap();
    Fixture {
        store,
        payments,
        user,
        other,
        admin,
        order,
        clock,
    }
}
fn meta(f: &Fixture) -> Value {
    json!({"lmm_subscription_order_id":f.order.order.order_id,"lmm_merchant":"acct_subscription_test","lmm_price_id":"price_frozen_daily"})
}
fn invoice(
    f: &Fixture,
    event: &str,
    invoice: &str,
    payment: &str,
    start: i64,
    end: i64,
    paid: bool,
) -> Value {
    json!({"id":event,"type":if paid{"invoice.paid"}else{"invoice.payment_failed"},"created":f.clock,"livemode":false,"api_version":crate::payments::subscription_stripe::STRIPE_SUBSCRIPTION_API_VERSION,"data":{"object":{
        "id":invoice,"object":"invoice","livemode":false,"subscription":"sub_provider_original","customer":"cus_original",
        "currency":"usd","total":125,"amount_due":125,"amount_paid":if paid{125}else{0},"amount_remaining":if paid{0}else{125},
        "paid_out_of_band":false,"status":if paid{"paid"}else{"open"},"payment_intent":payment,"billing_reason":"subscription_cycle",
        "subscription_details":{"metadata":meta(f)},"lines":{"has_more":false,"data":[{"price":{"id":"price_frozen_daily"},"quantity":1,"currency":"usd","amount":125,"proration":false,"period":{"start":start,"end":end}}]}
    }}})
}
fn sign(event: &Value, at: i64) -> (Vec<u8>, Vec<u8>) {
    let raw = serde_json::to_vec(event).unwrap();
    let mut mac = Hmac::<Sha256>::new_from_slice(SECRET.as_bytes()).unwrap();
    mac.update(format!("{at}.").as_bytes());
    mac.update(&raw);
    let hex = mac
        .finalize()
        .into_bytes()
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect::<String>();
    let sig = format!("t={at},v1={hex}").into_bytes();
    (raw, sig)
}
#[sqlx::test(migrations = false)]
async fn stripe_invoices_are_original_once_per_cycle_and_never_wallet_money(pool: PgPool) {
    let f = fixture(&pool).await;
    let id = &f.order.order.order_id;
    assert!(matches!(
        f.store.read_stripe_purchase(&f.other, id).await,
        Err(Error::NotFound)
    ));
    let one = invoice(
        &f,
        "evt_first_invoice",
        "in_first_invoice",
        "pi_original_first",
        f.clock - 60,
        f.clock + 86340,
        true,
    );
    let (raw, sig) = sign(&one, f.clock);
    let accepted = f
        .store
        .accept_stripe_event(&f.payments, id, &raw, &sig)
        .await
        .unwrap();
    assert!(accepted.subscription_id.is_some());
    assert_eq!(
        f.store
            .accept_stripe_event(&f.payments, id, &raw, &sig)
            .await
            .unwrap(),
        accepted
    );
    // Another authentic event for the SAME invoice returns the same grant.
    let mut alias = one.clone();
    alias["id"] = json!("evt_same_invoice_alias");
    alias["type"] = json!("invoice.payment_succeeded");
    let (a, b) = sign(&alias, f.clock);
    assert_eq!(
        f.store
            .accept_stripe_event(&f.payments, id, &a, &b)
            .await
            .unwrap()
            .subscription_id,
        accepted.subscription_id
    );
    let next = invoice(
        &f,
        "evt_next_invoice",
        "in_next_invoice",
        "pi_original_next",
        f.clock + 86340,
        f.clock + 172740,
        true,
    );
    let (a, b) = sign(&next, f.clock);
    let renewal = f
        .store
        .accept_stripe_event(&f.payments, id, &a, &b)
        .await
        .unwrap();
    assert_ne!(renewal.subscription_id, accepted.subscription_id);
    let failed = invoice(
        &f,
        "evt_failed_old",
        "in_failed_old",
        "pi_failed_old",
        f.clock - 60,
        f.clock + 86340,
        false,
    );
    let (a, b) = sign(&failed, f.clock);
    assert_eq!(
        f.store
            .accept_stripe_event(&f.payments, id, &a, &b)
            .await
            .unwrap()
            .state,
        "active"
    );
    let counts:(i64,i64)=sqlx::query_as("SELECT (SELECT count(*) FROM core_subscriptions.stripe_cycles),(SELECT count(*) FROM core_billing.ledger_journals)").fetch_one(&pool).await.unwrap();
    assert_eq!(counts, (2, 0));
    // Receipt recovery survives removal of the active provider configuration.
    let absent = PaymentsStore::new(pool.clone(), vec![]).unwrap();
    assert_eq!(
        f.store
            .accept_stripe_event(&absent, id, &raw, &sig)
            .await
            .unwrap(),
        accepted
    );
    let new = f
        .store
        .prepare_stripe_purchase(
            &f.payments,
            &f.user,
            &EPayPurchaseRequest {
                operation_key: "stripe_second_quote_01".into(),
                plan_id: f.order.order.plan.id,
                channel_id: 8,
            },
        )
        .await
        .unwrap();
    assert!(matches!(
        f.store
            .confirm_stripe_purchase(&f.user, &new.order.order_id)
            .await,
        Err(Error::Conflict)
    ));
}
#[sqlx::test(migrations = false)]
async fn stripe_checkout_does_not_grant_and_terminal_events_do_not_restore_access(pool: PgPool) {
    let f = fixture(&pool).await;
    let id = &f.order.order.order_id;
    let checkout = json!({"id":"evt_checkout_only","type":"checkout.session.completed","created":f.clock+1,"livemode":false,"api_version":crate::payments::subscription_stripe::STRIPE_SUBSCRIPTION_API_VERSION,"data":{"object":{"object":"checkout.session","id":"cs_subscription","mode":"subscription","client_reference_id":id,"status":"complete","subscription":"sub_provider_original","customer":"cus_original","metadata":meta(&f)}}});
    let (raw, sig) = sign(&checkout, f.clock + 1);
    assert!(
        f.store
            .accept_stripe_event(&f.payments, id, &raw, &sig)
            .await
            .unwrap()
            .subscription_id
            .is_none()
    );
    let paid = invoice(
        &f,
        "evt_reordered_paid",
        "in_reordered_paid",
        "pi_reordered",
        f.clock - 60,
        f.clock + 86340,
        true,
    );
    let (raw, sig) = sign(&paid, f.clock);
    assert_eq!(
        f.store
            .accept_stripe_event(&f.payments, id, &raw, &sig)
            .await
            .unwrap()
            .state,
        "active"
    );
    let cancel = json!({"id":"evt_terminal","type":"customer.subscription.deleted","created":f.clock+2,"livemode":false,"api_version":crate::payments::subscription_stripe::STRIPE_SUBSCRIPTION_API_VERSION,"data":{"object":{"object":"subscription","id":"sub_provider_original","customer":"cus_original","status":"canceled","metadata":meta(&f)}}});
    let (raw, sig) = sign(&cancel, f.clock + 2);
    assert_eq!(
        f.store
            .accept_stripe_event(&f.payments, id, &raw, &sig)
            .await
            .unwrap()
            .state,
        "canceled"
    );
    let old = invoice(
        &f,
        "evt_old_invoice_late",
        "in_old_invoice",
        "pi_old_invoice",
        f.clock - 86460,
        f.clock - 60,
        true,
    );
    let (raw, sig) = sign(&old, f.clock);
    assert_eq!(
        f.store
            .accept_stripe_event(&f.payments, id, &raw, &sig)
            .await
            .unwrap()
            .state,
        "canceled"
    );
    let active: i64 =
        sqlx::query_scalar("SELECT count(*) FROM core_charging.subscriptions WHERE active")
            .fetch_one(&pool)
            .await
            .unwrap();
    assert_eq!(active, 0);
    // Admin reads current configured prices; personal users cannot configure.
    assert_eq!(
        f.store
            .stripe_prices(&f.admin, f.order.order.plan.id, true)
            .await
            .unwrap()
            .len(),
        1
    );
}
#[sqlx::test(migrations = false)]
async fn stripe_invalid_candidates_and_temporary_provider_drift_do_not_own_grants(pool: PgPool) {
    let f = fixture(&pool).await;
    let id = &f.order.order.order_id;
    let paid = invoice(
        &f,
        "evt_recover_original",
        "in_recover_original",
        "pi_recover_original",
        f.clock - 60,
        f.clock + 86340,
        true,
    );
    let (raw, sig) = sign(&paid, f.clock);
    assert!(matches!(
        f.store
            .accept_stripe_event(
                &f.payments,
                id,
                &raw,
                b"t=1,v1=0000000000000000000000000000000000000000000000000000000000000000"
            )
            .await,
        Err(Error::Invalid)
    ));
    let mut epay =
        Channel::stripe_test(8, "1001".into(), "synthetic-epay-signing-key".into(), 10000);
    epay.provider = "epay".into();
    epay.currency = "CNY".into();
    epay.verification = crate::payments::ProviderVerification {
        payment_type: "alipay".into(),
        endpoint: "https://epay.example.test".into(),
        notify_url: "https://lmm.example.test/callbacks/8".into(),
        return_url: "https://lmm.example.test/subscriptions".into(),
        ..Default::default()
    };
    let changed = PaymentsStore::new(pool.clone(), vec![epay]).unwrap();
    assert!(matches!(
        f.store.accept_stripe_event(&changed, id, &raw, &sig).await,
        Err(Error::Conflict)
    ));
    let mut unsupported = paid.clone();
    unsupported["api_version"] = json!("2025-03-31.basil");
    let (a, b) = sign(&unsupported, f.clock);
    assert!(matches!(
        f.store.accept_stripe_event(&f.payments, id, &a, &b).await,
        Err(Error::Unavailable)
    ));
    // A real original HMAC remains verifiable after its delivery time window;
    // future-dated signatures do not. Domain replay keys prevent a second grant.
    assert!(
        f.payments
            .verify_stripe_subscription(
                &f.order.binding,
                id,
                f.clock - 1,
                &raw,
                &sig,
                f.clock + 3600
            )
            .is_ok()
    );
    let (future_raw, future_sig) = sign(&paid, f.clock + 3600);
    assert!(
        f.payments
            .verify_stripe_subscription(
                &f.order.binding,
                id,
                f.clock - 1,
                &future_raw,
                &future_sig,
                f.clock
            )
            .is_err()
    );
    assert!(
        f.store
            .accept_stripe_event(&f.payments, id, &raw, &sig)
            .await
            .unwrap()
            .subscription_id
            .is_some()
    );
    let invalid = invoice(
        &f,
        "evt_wrong_price",
        "in_wrong_price",
        "pi_wrong_price",
        f.clock + 86340,
        f.clock + 172740,
        true,
    );
    let mut invalid = invalid;
    invalid["data"]["object"]["lines"]["data"][0]["price"]["id"] = json!("price_other");
    let (raw, sig) = sign(&invalid, f.clock);
    assert!(matches!(
        f.store
            .accept_stripe_event(&f.payments, id, &raw, &sig)
            .await,
        Err(Error::Invalid)
    ));
    let count: i64 = sqlx::query_scalar("SELECT count(*) FROM core_subscriptions.stripe_cycles")
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(count, 1);
}

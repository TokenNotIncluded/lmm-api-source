use super::*;
use crate::{
    identity::IdentityStore,
    payments::{
        Channel, CreditQuotePolicy, CreditQuoteRatio, PaymentsStore, ProviderVerification,
        rpc::{PaymentsRpc, Permissions},
    },
    subscriptions::{ExternalPrice, PlanTerms, SavePlan, SubscriptionStore},
    test_runtime::RuntimeRole,
};
use serde_json::{Value, json};
use std::collections::BTreeMap;

fn channel() -> Channel {
    Channel {
        id: 7,
        provider: "epay".into(),
        merchant: "1001".into(),
        environment: "test".into(),
        currency: "CNY".into(),
        webhook_secrets: vec!["synthetic-plan-offers-private-key".into()],
        credits_per_minor: 1,
        max_amount_minor: 10000,
        credit_quote: Some(CreditQuotePolicy {
            settlement_per_usd_numerator: 7,
            settlement_per_usd_denominator: 1,
            discounts: BTreeMap::from([(
                500000,
                CreditQuoteRatio {
                    numerator: 9,
                    denominator: 10,
                },
            )]),
        }),
        verification: ProviderVerification {
            payment_type: "alipay".into(),
            endpoint: "https://epay.example.test".into(),
            notify_url: "https://lmm.example.test/payments/callbacks/7".into(),
            return_url: "https://lmm.example.test/subscriptions".into(),
            ..Default::default()
        },
    }
}
fn terms() -> PlanTerms {
    PlanTerms {
        title: "Published current plan".into(),
        price_units: "100".into(),
        duration_seconds: 3600,
        duration_months: 0,
        reset_period: "grant".into(),
        reset_seconds: 0,
        limit_units: "1000".into(),
        models: vec!["*".into()],
        groups: vec!["*".into()],
        max_purchases_per_user: 1,
        enabled: true,
        allow_balance_pay: true,
    }
}
async fn unchanged_state(pool: &PgPool) -> Value {
    let mut values = serde_json::Map::new();
    for table in [
        "core_payments.intents",
        "core_subscriptions.quotes",
        "core_subscriptions.external_orders",
        "core_subscriptions.commands",
        "core_billing.ledger_journals",
        "core_identity.credentials",
    ] {
        let value:Value=sqlx::query_scalar(sqlx::AssertSqlSafe(format!(
            "SELECT coalesce(jsonb_agg(to_jsonb(t) ORDER BY to_jsonb(t)::text),'[]'::jsonb) FROM {table} t"
        ))).fetch_one(pool).await.unwrap();
        values.insert(table.to_owned(), value);
    }
    Value::Object(values)
}
#[test]
fn plan_offers_checkout_binding_matches_go_and_separates_purpose() {
    let c = channel();
    assert_eq!(
        crate::payments::offers::binding(&c, false),
        "fecbd0c415edfd7f3aaea21e9a980260e914b1671535e2bfaaabed7f5406c016"
    );
    assert_eq!(
        crate::payments::offers::binding(&c, true),
        "3725b08cc80a10a1d13d3427ce505106a408133638600318d38097e32f52aebb"
    );
    let mut changed = c.clone();
    changed.merchant = "other".into();
    assert_ne!(
        crate::payments::offers::binding(&c, false),
        crate::payments::offers::binding(&changed, false)
    );
}
#[sqlx::test(migrations = false)]
async fn plan_offers_use_runtime_current_terms_and_never_prepare_payment(pool: PgPool) {
    crate::core_schema::init(&pool).await.unwrap();
    let identity = IdentityStore::from_pool(pool.clone());
    let admin = identity.bootstrap_user(93601, 6).await.unwrap().secret;
    let user = identity.bootstrap_user(93602, 0).await.unwrap().secret;
    let actor = identity.authorize(&user).await.unwrap();
    let subscriptions = SubscriptionStore::new(pool.clone());
    let plan = subscriptions
        .save_plan(
            &admin,
            None,
            SavePlan {
                operation_key: "save_plan_offers_01".into(),
                expected_revision: None,
                terms: terms(),
            },
        )
        .await
        .unwrap();
    let payments = PaymentsStore::new(pool.clone(), vec![channel()]).unwrap();
    subscriptions
        .save_epay_price(
            &payments,
            &admin,
            "plan_offers_price_01",
            &ExternalPrice {
                plan_id: plan.id,
                plan_revision: plan.revision,
                channel_id: 7,
                currency: "CNY".into(),
                amount_minor: "125".into(),
            },
        )
        .await
        .unwrap();
    let rpc = PaymentsRpc::new(
        payments,
        &[58; 32],
        Permissions {
            prepare: true,
            ..Default::default()
        },
    )
    .unwrap();
    let runtime = RuntimeRole::create(&pool).await;
    runtime.optional("core_subscriptions").await;
    let reader = PlanOffersStore::new(runtime.pool.clone(), Some(rpc.offer_source(true)))
        .await
        .unwrap();
    let before = unchanged_state(&pool).await;
    let facts = serde_json::to_value(reader.read(&user).await.unwrap()).unwrap();
    assert_eq!(facts["actor_user_id"], actor.user_id);
    assert_eq!(facts["plans"][0]["id"], plan.id);
    assert_eq!(facts["plans"][0]["revision"], 1);
    assert_eq!(
        facts["plans"][0]["external_prices"][0]["amount_minor"],
        "125"
    );
    assert_eq!(facts["plans"][0]["external_prices"][0]["configured"], true);
    assert_eq!(facts["topup_channels"][0]["configured"], true);
    assert_eq!(
        facts["topup_channels"][0]["credit_quote"]["discounts"]["500000"],
        json!({"numerator":9,"denominator":10})
    );
    assert_eq!(unchanged_state(&pool).await, before);
    assert!(
        !facts
            .to_string()
            .contains("synthetic-plan-offers-private-key")
    );
    assert!(!facts.to_string().contains(&user));
    let mut replacement = terms();
    replacement.price_units = "200".into();
    subscriptions
        .save_plan(
            &admin,
            Some(plan.id),
            SavePlan {
                operation_key: "save_plan_offers_02".into(),
                expected_revision: Some(1),
                terms: replacement,
            },
        )
        .await
        .unwrap();
    let before = unchanged_state(&pool).await;
    let facts = serde_json::to_value(reader.read(&user).await.unwrap()).unwrap();
    assert_eq!(facts["plans"][0]["revision"], 2);
    assert_eq!(facts["plans"][0]["terms"]["price_units"], "200");
    assert_eq!(facts["plans"][0]["external_prices"], json!([]));
    assert_eq!(unchanged_state(&pool).await, before);
    sqlx::query("UPDATE core_identity.credentials SET revoked_at=clock_timestamp() WHERE id=$1")
        .bind(actor.credential_id)
        .execute(&pool)
        .await
        .unwrap();
    let before = unchanged_state(&pool).await;
    assert!(matches!(reader.read(&user).await, Err(Error::Unauthorized)));
    assert_eq!(unchanged_state(&pool).await, before);
    drop(reader);
    runtime.cleanup().await;
}
#[sqlx::test(migrations = false)]
async fn plan_offers_require_actual_console_read_authority(pool: PgPool) {
    crate::core_schema::init(&pool).await.unwrap();
    let identity = IdentityStore::from_pool(pool.clone());
    let user = identity.bootstrap_user(93611, 0).await.unwrap().secret;
    let runtime = RuntimeRole::create(&pool).await;
    assert!(
        PlanOffersStore::new(runtime.pool.clone(), None)
            .await
            .is_err()
    );
    runtime.optional("core_subscriptions").await;
    let reader = PlanOffersStore::new(runtime.pool.clone(), None)
        .await
        .unwrap();
    let before = unchanged_state(&pool).await;
    let facts = serde_json::to_value(reader.read(&user).await.unwrap()).unwrap();
    assert_eq!(facts["plans"], json!([]));
    assert_eq!(facts["topup_channels"], json!([]));
    assert_eq!(unchanged_state(&pool).await, before);
    drop(reader);
    runtime.cleanup().await;
}

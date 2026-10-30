//! Synthetic users and PostgreSQL; real Core wallet, subscription grants and
//! charging are used. No external payment provider or production funds.
use lmm_core::{
    accounts::{Account, AccountKind},
    billing::{
        Credits,
        charging::{self, Charging, Price, Request, Source, Usage},
        ledger::{self, Action, Ledger, Outcome, Request as LedgerRequest},
        transactional_ledger::TransactionalLedger,
    },
    funding::{BillingPreference, FundingPolicy},
    identity::IdentityStore,
    subscriptions::{
        self, ArchivePlan, Error, PlanTerms, QuoteRequest, SavePlan, SubscriptionStore,
    },
};
use sqlx::PgPool;
use std::sync::Arc;
#[path = "common/runtime.rs"]
mod runtime;

struct Fixture {
    identity: IdentityStore,
    store: SubscriptionStore,
    ledger: Ledger,
    admin: String,
    user: String,
    other: String,
    account: i64,
}
async fn fixture(pool: &PgPool, full: bool) -> Fixture {
    let identity = IdentityStore::from_pool(pool.clone());
    if full {
        lmm_core::core_schema::init(pool).await.unwrap();
    } else {
        identity.init_database().await.unwrap();
        for schema in [
            ledger::SCHEMA,
            charging::SCHEMA,
            include_str!("../schema/charging_ledger.sql"),
            lmm_core::abuse::SCHEMA,
            subscriptions::SCHEMA,
        ] {
            sqlx::raw_sql(schema).execute(pool).await.unwrap();
        }
    }
    let admin = identity.bootstrap_user(8301, 6).await.unwrap().secret;
    let user = identity.bootstrap_user(8302, 0).await.unwrap().secret;
    let other = identity.bootstrap_user(8303, 0).await.unwrap().secret;
    let account = identity.authorize(&user).await.unwrap().owner_account_id;
    let ledger = Ledger::from_pool(pool.clone());
    ledger.open_wallet(account).await.unwrap();
    let gift = ledger
        .execute(&LedgerRequest {
            scope: "synthetic-subscription-fixture".into(),
            operation_key: "initial-credits".into(),
            actor_user_id: Some(8302),
            reason: "Isolated test opening".into(),
            action: Action::Gift {
                account_id: account,
                amount_units: Credits::try_from(500_000).unwrap(),
            },
        })
        .await
        .unwrap();
    assert!(matches!(gift, Outcome::Posted { .. }));
    Fixture {
        identity,
        store: SubscriptionStore::new(pool.clone()),
        ledger,
        admin,
        user,
        other,
        account,
    }
}
fn terms() -> PlanTerms {
    PlanTerms {
        title: "Synthetic hourly plan".into(),
        price_units: "100000".into(),
        duration_seconds: 3600,
        reset_period: "grant".into(),
        reset_seconds: 0,
        limit_units: "1000".into(),
        models: vec!["*".into()],
        groups: vec!["*".into()],
        max_purchases_per_user: 0,
        enabled: true,
    }
}
fn create(key: &str) -> SavePlan {
    SavePlan {
        operation_key: key.into(),
        expected_revision: None,
        terms: terms(),
    }
}
fn quote(plan: i64, key: &str, renew_from: Option<i64>) -> QuoteRequest {
    QuoteRequest {
        plan_id: plan,
        operation_key: key.into(),
        renew_from,
    }
}
async fn balanced(pool: &PgPool) {
    let count:i64=sqlx::query_scalar("SELECT count(*) FROM (SELECT journal_id FROM core_billing.ledger_entries GROUP BY journal_id HAVING sum(delta_units)<>0) bad").fetch_one(pool).await.unwrap();
    assert_eq!(count, 0);
    let stale:i64=sqlx::query_scalar("SELECT count(*) FROM core_billing.balance_state b LEFT JOIN (SELECT ledger_account_id,sum(delta_units) AS units FROM core_billing.ledger_entries GROUP BY ledger_account_id) e ON e.ledger_account_id=b.ledger_account_id WHERE b.balance_units::numeric<>coalesce(e.units,0)").fetch_one(pool).await.unwrap();
    assert_eq!(stale, 0);
}

#[sqlx::test]
async fn wallet_purchase_renewal_cancellation_and_unused_refund_are_atomic(pool: PgPool) {
    let mut f = fixture(&pool, true).await;
    let runtime = runtime::RuntimeRole::create(&pool).await;
    runtime.optional("core_charging").await;
    runtime.optional("core_subscriptions").await;
    f.store = SubscriptionStore::new(runtime.pool.clone());
    assert_eq!(
        f.store
            .save_plan(&f.user, None, create("ordinary_user_denied"))
            .await,
        Err(Error::Forbidden)
    );
    let input = create("create_hourly_plan_01");
    let plan = f
        .store
        .save_plan(&f.admin, None, input.clone())
        .await
        .unwrap();
    assert_eq!(
        f.store.save_plan(&f.admin, None, input).await.unwrap(),
        plan
    );
    let q = f
        .store
        .quote(&f.user, quote(plan.id, "first_purchase_quote", None))
        .await
        .unwrap();
    assert_eq!(
        f.store
            .quote(&f.user, quote(plan.id, "first_purchase_quote", None))
            .await
            .unwrap(),
        q
    );
    assert_eq!(f.store.confirm(&f.other, &q.id).await, Err(Error::NotFound));
    let first = f.store.confirm(&f.user, &q.id).await.unwrap();
    assert_eq!(first.status, "posted");
    assert_eq!(f.store.confirm(&f.user, &q.id).await.unwrap(), first);
    assert_eq!(
        f.ledger.balance(f.account).await.unwrap().available_units,
        400_000
    );
    let renewal = f
        .store
        .quote(
            &f.user,
            quote(plan.id, "renewal_purchase_quote", first.purchase_id),
        )
        .await
        .unwrap();
    assert_eq!(renewal.starts_at, q.ends_at);
    let second = f.store.confirm(&f.user, &renewal.id).await.unwrap();
    assert_ne!(first.subscription_id, second.subscription_id);
    assert_eq!(
        f.ledger.balance(f.account).await.unwrap().available_units,
        300_000
    );
    let canceled = f
        .store
        .cancel(&f.user, first.purchase_id.unwrap())
        .await
        .unwrap();
    assert!(canceled.renewal_canceled);
    assert!(
        canceled.entitlement_active,
        "cancel removed paid current benefits"
    );
    assert_eq!(
        f.store
            .cancel(&f.user, first.purchase_id.unwrap())
            .await
            .unwrap(),
        canceled
    );
    assert_eq!(
        f.store
            .quote(
                &f.user,
                quote(plan.id, "cannot_renew_canceled", first.purchase_id)
            )
            .await,
        Err(Error::Conflict)
    );
    let preview = f
        .store
        .preview_refund(&f.user, second.purchase_id.unwrap())
        .await
        .unwrap();
    assert_eq!(preview.amount_units, "100000");
    assert_eq!(
        f.store.refund(&f.other, &preview.id).await,
        Err(Error::NotFound)
    );
    let refunded = f.store.refund(&f.user, &preview.id).await.unwrap();
    assert_eq!(refunded.status, "refunded");
    assert_eq!(
        f.store.refund(&f.user, &preview.id).await.unwrap(),
        refunded
    );
    assert_eq!(
        f.ledger.balance(f.account).await.unwrap().available_units,
        400_000
    );
    let active: bool =
        sqlx::query_scalar("SELECT active FROM core_charging.subscriptions WHERE id=$1")
            .bind(second.subscription_id.unwrap())
            .fetch_one(&pool)
            .await
            .unwrap();
    assert!(!active);
    f.store
        .archive_plan(
            &f.admin,
            plan.id,
            ArchivePlan {
                operation_key: "archive_hourly_plan".into(),
                expected_revision: 1,
            },
        )
        .await
        .unwrap();
    assert!(f.store.plans(&f.user, false).await.unwrap().is_empty());
    assert_eq!(
        f.store.confirm(&f.user, &q.id).await.unwrap(),
        first,
        "catalog edit changed durable paid receipt"
    );
    assert_eq!(
        f.store.refund(&f.user, &preview.id).await.unwrap(),
        refunded
    );
    balanced(&pool).await;
    runtime.cleanup().await;
}

#[sqlx::test]
async fn stale_quotes_and_insufficient_funds_never_create_entitlements(pool: PgPool) {
    let f = fixture(&pool, false).await;
    let plan = f
        .store
        .save_plan(&f.admin, None, create("create_stale_plan_01"))
        .await
        .unwrap();
    let q = f
        .store
        .quote(&f.user, quote(plan.id, "stale_purchase_quote", None))
        .await
        .unwrap();
    let mut edited = terms();
    edited.price_units = "200000".into();
    let edit = SavePlan {
        operation_key: "edit_stale_plan_01".into(),
        expected_revision: Some(plan.revision),
        terms: edited,
    };
    let updated = f
        .store
        .save_plan(&f.admin, Some(plan.id), edit.clone())
        .await
        .unwrap();
    assert_eq!(
        f.store
            .save_plan(&f.admin, Some(plan.id), edit)
            .await
            .unwrap(),
        updated
    );
    assert_eq!(f.store.confirm(&f.user, &q.id).await, Err(Error::Conflict));
    let count: i64 = sqlx::query_scalar("SELECT count(*) FROM core_charging.subscriptions")
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(count, 0);
    let empty = f
        .store
        .quote(&f.other, quote(plan.id, "insufficient_quote_01", None))
        .await
        .unwrap();
    let rejected = f.store.confirm(&f.other, &empty.id).await.unwrap();
    assert_eq!(rejected.status, "rejected");
    assert_eq!(rejected.rejection.as_deref(), Some("insufficient_funds"));
    assert_eq!(
        f.store.confirm(&f.other, &empty.id).await.unwrap(),
        rejected
    );
    assert!(f.store.purchases(&f.other).await.unwrap().is_empty());
    assert_eq!(
        f.ledger.balance(f.account).await.unwrap().available_units,
        500_000
    );
    balanced(&pool).await;
}

#[sqlx::test]
async fn real_charging_reservations_and_usage_block_refund_and_revoked_actors_cannot_replay(
    pool: PgPool,
) {
    let f = fixture(&pool, false).await;
    let plan = f
        .store
        .save_plan(&f.admin, None, create("create_usage_plan_01"))
        .await
        .unwrap();
    let q = f
        .store
        .quote(&f.user, quote(plan.id, "usage_purchase_quote", None))
        .await
        .unwrap();
    let receipt = f.store.confirm(&f.user, &q.id).await.unwrap();
    let preview = f
        .store
        .preview_refund(&f.user, receipt.purchase_id.unwrap())
        .await
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
    sqlx::query("INSERT INTO core_identity.user_routing_groups(user_id,group_name,enabled,source_reference) VALUES(8302,'default',true,'synthetic-subscription-usage-fixture')").execute(&pool).await.unwrap();
    let charging = Charging::new(pool.clone(), Arc::new(TransactionalLedger));
    let request = Request {
        id: "subscription-real-consumption".into(),
        fingerprint: [9; 32],
        price: Price {
            text_request: None,
            tool: None,
            grounding: None,
            user_trust: None,
            image: None,
            upstream_cost_group_ppm: None,
            tiered: None,
            version: "synthetic-v1".into(),
            model: "test-model".into(),
            group: "default".into(),
            input_per_million: 1_000_000,
            cached_per_million: 500_000,
            output_per_million: 1_000_000,
        },
        reserve: 40,
        lease_seconds: 300,
        maximum_seconds: 3600,
    };
    let worker = [11; 32];
    let reserved = charging
        .reserve(&key.secret, &request, &worker)
        .await
        .unwrap();
    assert!(
        matches!(reserved.source,Source::Subscription{id,..} if Some(id)==receipt.subscription_id)
    );
    assert_eq!(f.store.refund(&f.user, &preview.id).await, Err(Error::Used));
    assert_eq!(
        f.store
            .preview_refund(&f.user, receipt.purchase_id.unwrap())
            .await,
        Err(Error::Used)
    );
    assert!(charging.start(&request.id, &worker).await.unwrap());
    charging
        .settle(
            &request.id,
            &worker,
            Usage {
                output: 5,
                ..Usage::default()
            },
            charging::Outcome::Completed,
        )
        .await
        .unwrap();
    assert_eq!(f.store.refund(&f.user, &preview.id).await, Err(Error::Used));
    let rows = f.store.purchases(&f.user).await.unwrap();
    assert_eq!(rows[0].used_units, "5");
    assert_eq!(rows[0].reserved_units, "0");
    assert_eq!(
        f.ledger.balance(f.account).await.unwrap().available_units,
        400_000,
        "subscription usage charged wallet again"
    );
    sqlx::query("UPDATE core_identity.users SET active=false WHERE id=8302")
        .execute(&pool)
        .await
        .unwrap();
    assert!(matches!(
        f.store.confirm(&f.user, &q.id).await,
        Err(Error::Unauthorized | Error::Forbidden)
    ));
    assert!(matches!(
        f.store.refund(&f.user, &preview.id).await,
        Err(Error::Unauthorized | Error::Forbidden)
    ));
    balanced(&pool).await;
}

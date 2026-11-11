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
#[path = "subscriptions/admin.rs"]
mod admin;
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
        duration_months: 0,
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
        allow_balance_pay: true,
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
    // Unknown historical fiat valuation must not disable native credit spending.
    let mut connection = runtime.pool.acquire().await.unwrap();
    let wallet = lmm_core::financial_profile::read_wallet(&mut connection, 8302)
        .await
        .unwrap();
    assert!(wallet.financial_projection_available);
    assert_eq!(wallet.wallet_unit, "core_credit");
    assert_eq!(wallet.quota, 500_000);
    assert_eq!(wallet.wallet_money_basis, "unknown");
    assert!(!wallet.wallet_money_available);
    drop(connection);
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
    let mut connection = runtime.pool.acquire().await.unwrap();
    let wallet = lmm_core::financial_profile::read_wallet(&mut connection, 8302)
        .await
        .unwrap();
    assert_eq!(wallet.quota, 400_000);
    assert_eq!(wallet.wallet_money_basis, "unknown");
    assert!(!wallet.wallet_money_available);
    drop(connection);
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
    let f = fixture(&pool, true).await;
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
    // Real charging depends on the complete current Core schema, not just the
    // subscription and ledger tables used by the other narrow fixture.
    let f = fixture(&pool, true).await;
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
    // Target the latter half of a DB second, when a bigint cast used to round
    // the new anchor into the future relative to charging's floor timestamp.
    sqlx::query("SELECT pg_sleep(mod(1.65 - mod(extract(epoch FROM clock_timestamp()), 1), 1)::double precision)")
        .execute(&pool)
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
            rounding: None,
            text_request: None,
            tool: None,
            audio: None,
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

#[sqlx::test(migrations = false)]
async fn calendar_plan_purchase_and_renewal_keep_calendar_terms(pool: PgPool) {
    use chrono::{Datelike, NaiveDate, TimeDelta};
    let f = fixture(&pool, true).await;
    let mut input = create("calendar_plan_create");
    input.terms.duration_seconds = 0;
    input.terms.duration_months = 1;
    let plan = f.store.save_plan(&f.admin, None, input).await.unwrap();
    let first = f
        .store
        .quote(&f.user, quote(plan.id, "calendar_first_quote", None))
        .await
        .unwrap();
    let expected = |start| {
        let d = chrono::DateTime::from_timestamp(start, 0).unwrap();
        let (y, m) = if d.month() == 12 {
            (d.year() + 1, 1)
        } else {
            (d.year(), d.month() + 1)
        };
        (NaiveDate::from_ymd_opt(y, m, 1).unwrap().and_time(d.time())
            + TimeDelta::days(i64::from(d.day0())))
        .and_utc()
        .timestamp()
    };
    assert_eq!(first.ends_at, expected(first.starts_at));
    let receipt = f.store.confirm(&f.user, &first.id).await.unwrap();
    let renewal = f
        .store
        .quote(
            &f.user,
            quote(plan.id, "calendar_renew_quote", receipt.purchase_id),
        )
        .await
        .unwrap();
    assert_eq!(renewal.starts_at, first.ends_at);
    assert_eq!(renewal.ends_at, expected(first.ends_at));
    f.store.confirm(&f.user, &renewal.id).await.unwrap();
    balanced(&pool).await;
}

#[sqlx::test(migrations = false)]
async fn balance_original_quote_read_recovers_receipt_without_reconfirming(pool: PgPool) {
    let f = fixture(&pool, true).await;
    let plan = f
        .store
        .save_plan(&f.admin, None, create("balance-status-create-0001"))
        .await
        .unwrap();
    let q = f
        .store
        .quote(&f.user, quote(plan.id, "balance-status-quote-0001", None))
        .await
        .unwrap();
    let before = f.store.read_balance_purchase(&f.user, &q.id).await.unwrap();
    assert_eq!(before.quote, q);
    assert!(before.receipt.is_none());
    assert!(matches!(
        f.store.read_balance_purchase(&f.other, &q.id).await,
        Err(Error::NotFound)
    ));
    let receipt = f.store.confirm(&f.user, &q.id).await.unwrap();
    let counts:(i64,i64,i64)=sqlx::query_as("SELECT (SELECT count(*) FROM core_billing.ledger_entries),(SELECT count(*) FROM core_subscriptions.purchases),(SELECT count(*) FROM core_charging.subscriptions)").fetch_one(&pool).await.unwrap();
    for _ in 0..2 {
        let original = f.store.read_balance_purchase(&f.user, &q.id).await.unwrap();
        assert_eq!(original.quote, q);
        assert_eq!(original.receipt.as_ref(), Some(&receipt));
    }
    let after:(i64,i64,i64)=sqlx::query_as("SELECT (SELECT count(*) FROM core_billing.ledger_entries),(SELECT count(*) FROM core_subscriptions.purchases),(SELECT count(*) FROM core_charging.subscriptions)").fetch_one(&pool).await.unwrap();
    assert_eq!(counts, after);
}

#[sqlx::test(migrations = false)]
async fn legacy_balance_original_is_read_only_and_counts_purchase_limit(pool: PgPool) {
    let f = fixture(&pool, true).await;
    let mut input = create("legacy-balance-plan-0001");
    input.terms.max_purchases_per_user = 1;
    let plan = f.store.save_plan(&f.admin, None, input).await.unwrap();
    let actor = f.identity.authorize(&f.user).await.unwrap();
    let mut seed = pool.begin().await.unwrap();
    sqlx::query("SET TRANSACTION ISOLATION LEVEL SERIALIZABLE")
        .execute(&mut *seed)
        .await
        .unwrap();
    let grant:i64=sqlx::query_scalar("INSERT INTO core_charging.subscriptions(account_id,grant_reference,period,anchor,ends_at,limit_credits,models,groups,active) VALUES($1,'legacy-sub:test:1','grant',1000,2000,100,ARRAY['*'],ARRAY['*'],false) RETURNING id").bind(f.account).fetch_one(&mut *seed).await.unwrap();
    seed.commit().await.unwrap();
    let history = serde_json::json!({"reference":"BALANCE_OLD_1","plan":plan,"account_id":f.account,"charge_units":"10","subscription_id":grant,"ends_at":2000,"source":{"source_status":"success"}});
    sqlx::query("INSERT INTO core_subscriptions.commands(actor_user_id,operation_key,request,result) VALUES($1,'legacy_balance_original_0001',$2,$3)").bind(actor.user_id).bind(serde_json::json!({"kind":"legacy_balance_original_v1"})).bind(&history).execute(&pool).await.unwrap();
    let before:(i64,i64)=sqlx::query_as("SELECT (SELECT count(*) FROM core_billing.ledger_entries),(SELECT count(*) FROM core_charging.subscriptions)").fetch_one(&pool).await.unwrap();
    assert_eq!(
        f.store
            .read_legacy_balance_purchase(&f.user, "BALANCE_OLD_1")
            .await
            .unwrap(),
        history
    );
    assert_eq!(
        f.store
            .read_legacy_balance_purchase(&f.other, "BALANCE_OLD_1")
            .await,
        Err(Error::NotFound)
    );
    let q = f
        .store
        .quote(&f.user, quote(plan.id, "legacy-balance-next-quote", None))
        .await
        .unwrap();
    assert_eq!(f.store.confirm(&f.user, &q.id).await, Err(Error::Conflict));
    let after:(i64,i64)=sqlx::query_as("SELECT (SELECT count(*) FROM core_billing.ledger_entries),(SELECT count(*) FROM core_charging.subscriptions)").fetch_one(&pool).await.unwrap();
    assert_eq!(before, after);
}

#[sqlx::test]
async fn wallet_payment_disable_blocks_new_spend_and_preserves_posted_replay(pool: PgPool) {
    let f = fixture(&pool, true).await;
    let plan = f
        .store
        .save_plan(&f.admin, None, create("wallet_policy_plan_01"))
        .await
        .unwrap();
    let posted_quote = f
        .store
        .quote(&f.user, quote(plan.id, "wallet_posted_quote_01", None))
        .await
        .unwrap();
    let pending_quote = f
        .store
        .quote(&f.user, quote(plan.id, "wallet_pending_quote_01", None))
        .await
        .unwrap();
    let posted = f.store.confirm(&f.user, &posted_quote.id).await.unwrap();
    assert_eq!(posted.status, "posted");
    let before: (i64, i64, i64) = sqlx::query_as(
        "SELECT (SELECT count(*) FROM core_billing.ledger_journals), (SELECT count(*) FROM core_subscriptions.purchases), (SELECT count(*) FROM core_subscriptions.quotes)"
    ).fetch_one(&pool).await.unwrap();
    let mut disabled = plan.terms.clone();
    disabled.allow_balance_pay = false;
    let current = f
        .store
        .save_plan(
            &f.admin,
            Some(plan.id),
            SavePlan {
                operation_key: "disable_wallet_plan_01".into(),
                expected_revision: Some(plan.revision),
                terms: disabled,
            },
        )
        .await
        .unwrap();
    assert!(!current.terms.allow_balance_pay);
    assert_eq!(
        f.store.confirm(&f.user, &posted_quote.id).await.unwrap(),
        posted
    );
    assert_eq!(
        f.store.confirm(&f.user, &pending_quote.id).await,
        Err(Error::Conflict)
    );
    assert_eq!(
        f.store
            .quote(&f.user, quote(plan.id, "wallet_denied_quote_01", None))
            .await,
        Err(Error::Conflict)
    );
    assert_eq!(
        f.store
            .quote(
                &f.user,
                quote(plan.id, "wallet_denied_renew_01", posted.purchase_id)
            )
            .await,
        Err(Error::Conflict)
    );
    assert_eq!(
        f.store
            .quote(&f.user, quote(plan.id, "wallet_pending_quote_01", None))
            .await
            .unwrap(),
        pending_quote
    );
    let after: (i64, i64, i64) = sqlx::query_as(
        "SELECT (SELECT count(*) FROM core_billing.ledger_journals), (SELECT count(*) FROM core_subscriptions.purchases), (SELECT count(*) FROM core_subscriptions.quotes)"
    ).fetch_one(&pool).await.unwrap();
    assert_eq!(after, before);
    balanced(&pool).await;
}

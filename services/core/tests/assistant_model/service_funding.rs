//! Real Session, PostgreSQL funds, runtime role and counted loopback provider.
use super::*;
use lmm_core::{
    assistant_model::ServiceFundingPolicy,
    billing::{
        Credits,
        charging::{Budget, Charging},
        ledger::{Action, Ledger, Request},
        transactional_ledger::TransactionalLedger,
    },
    identity::{IdentityStore, IssuedCredential},
};

struct Fixture {
    identity: IdentityStore,
    ledger: Ledger,
    customer: IssuedCredential,
    root: IssuedCredential,
    customer_account: i64,
    payer_account: i64,
}

async fn setup(pool: &PgPool) -> Fixture {
    lmm_core::core_schema::init(pool).await.unwrap();
    let identity = IdentityStore::from_pool(pool.clone());
    let customer = identity.bootstrap_user(7, 1).await.unwrap();
    let root = identity.bootstrap_user(90001, 6).await.unwrap();
    let customer_account = identity
        .authorize(&customer.secret)
        .await
        .unwrap()
        .owner_account_id;
    let payer_account = identity
        .authorize(&root.secret)
        .await
        .unwrap()
        .owner_account_id;
    assert_ne!(
        payer_account, 90001,
        "a source user ID is not an account ID"
    );
    let ledger = Ledger::from_pool(pool.clone());
    for account in [customer_account, payer_account] {
        ledger.open_wallet(account).await.unwrap();
    }
    ledger
        .execute(&Request {
            scope: "assistant-service-funding-test".into(),
            operation_key: "synthetic-service-funds".into(),
            actor_user_id: Some(90001),
            reason: "Synthetic platform funds in an isolated test database".into(),
            action: Action::Credit {
                account_id: payer_account,
                amount_units: Credits::try_from(100000).unwrap(),
                payment_reference: "synthetic:assistant-service-funding".into(),
            },
        })
        .await
        .unwrap();
    Fixture {
        identity,
        ledger,
        customer,
        root,
        customer_account,
        payer_account,
    }
}

fn policy(account: i64) -> PersonalTextPolicy {
    PersonalTextPolicy {
        model: "local-model".into(),
        group: "allowed".into(),
        budget_credits: 10000,
        max_output_tokens: 32,
        ttl_seconds: 300,
        gift_tools: false,
        sponsor: None,
        service_funding: Some(ServiceFundingPolicy {
            payer_user_id: 90001,
            payer_account_id: account,
        }),
    }
}

fn funded(pool: &PgPool, provider: &Provider, account: i64) -> AssistantModel {
    AssistantModel::new(pool.clone(), gateway(pool, provider), policy(account)).unwrap()
}

#[sqlx::test]
async fn service_funding_charges_and_refunds_only_the_fixed_platform_wallet(pool: PgPool) {
    let f = setup(&pool).await;
    // The customer has neither developer access nor a routing-group grant.
    let grants: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM core_identity.user_routing_groups WHERE user_id=7",
    )
    .fetch_one(&pool)
    .await
    .unwrap();
    assert_eq!(grants, 0);
    let runtime = runtime_authority::RuntimeRole::create(&pool).await;
    runtime.optional("core_charging").await;
    runtime.optional("core_assistant").await;
    let p = Provider::start(true).await;
    let s = funded(&runtime.pool, &p, f.payer_account);
    let b = binding(&s, &f.customer.secret, "service-customer", "hello").await;
    let op = s
        .reserve_text(&f.customer.secret, &b, "hello")
        .await
        .unwrap();
    assert_eq!(op.state, State::Settled);
    assert_eq!(op.settled_credits, Some(49));
    assert_eq!(
        s.reserve_text(&f.customer.secret, &b, "hello")
            .await
            .unwrap()
            .request_id,
        op.request_id
    );
    assert_eq!(p.hits.load(Ordering::SeqCst), 1);
    let funding: (i64,i64,i64,Option<i64>,Option<i64>) = sqlx::query_as(
        "SELECT actor_user_id,owner_account_id,payer_account_id,assistant_sponsor_account_id,source_subscription_id FROM core_charging.requests WHERE id=$1")
        .bind(&op.request_id).fetch_one(&pool).await.unwrap();
    assert_eq!(
        funding,
        (
            7,
            f.customer_account,
            f.payer_account,
            Some(f.payer_account),
            None
        )
    );
    assert_eq!(
        f.ledger
            .balance(f.customer_account)
            .await
            .unwrap()
            .available_units,
        0
    );
    assert_eq!(
        f.ledger
            .balance(f.payer_account)
            .await
            .unwrap()
            .available_units,
        99951
    );
    // Disabling new platform funding must not strand an already settled charge.
    let stopped = sqlx::query(
        "UPDATE core_identity.users SET active=false,platform_role='admin' WHERE id=90001",
    )
    .execute(&pool)
    .await
    .unwrap();
    assert_eq!(stopped.rows_affected(), 1);
    let charging = Charging::new(runtime.pool.clone(), Arc::new(TransactionalLedger));
    charging
        .refund(
            &op.request_id,
            "service-refund",
            49,
            "synthetic-original-refund-evidence",
        )
        .await
        .unwrap();
    charging
        .refund(
            &op.request_id,
            "service-refund",
            49,
            "synthetic-original-refund-evidence",
        )
        .await
        .unwrap();
    assert_eq!(
        f.ledger
            .balance(f.payer_account)
            .await
            .unwrap()
            .available_units,
        100000
    );
    assert_eq!(
        f.ledger
            .balance(f.customer_account)
            .await
            .unwrap()
            .available_units,
        0
    );
    let gifts: (bool,i64,i64) = sqlx::query_as("SELECT enabled,(SELECT count(*) FROM core_gift.qualifications),(SELECT count(*) FROM core_gift.decisions) FROM core_gift.policy WHERE singleton")
        .fetch_one(&pool).await.unwrap();
    assert_eq!(
        gifts,
        (false, 0, 0),
        "service funding never enables or qualifies gifts"
    );
    runtime.cleanup().await;
}

#[sqlx::test]
async fn service_funding_allows_the_root_customer_without_a_new_user_limit(pool: PgPool) {
    let f = setup(&pool).await;
    let p = Provider::start(true).await;
    let s = funded(&pool, &p, f.payer_account);
    let b = binding(&s, &f.root.secret, "service-root-self", "hello").await;
    let op = s.reserve_text(&f.root.secret, &b, "hello").await.unwrap();
    assert_eq!(op.state, State::Settled);
    let row: (i64,i64,Option<i64>) = sqlx::query_as("SELECT owner_account_id,payer_account_id,assistant_sponsor_account_id FROM core_charging.requests WHERE id=$1")
        .bind(op.request_id).fetch_one(&pool).await.unwrap();
    assert_eq!(
        row,
        (f.payer_account, f.payer_account, Some(f.payer_account))
    );
    assert_eq!(
        f.ledger
            .balance(f.payer_account)
            .await
            .unwrap()
            .available_units,
        99951
    );
    assert_eq!(p.hits.load(Ordering::SeqCst), 1);
}

#[sqlx::test]
async fn service_funding_rechecks_enabled_root_at_grant_and_reservation(pool: PgPool) {
    let f = setup(&pool).await;
    let p = Provider::start(true).await;
    let wrong = funded(&pool, &p, f.customer_account);
    let cv = wrong
        .conversation(&f.customer.secret, "wrong-service-account", true)
        .await
        .unwrap();
    assert_eq!(
        wrong
            .turn(&f.customer.secret, &cv.core_conversation_id, "turn-1", true)
            .await
            .unwrap_err(),
        Error::Forbidden
    );
    let s = funded(&pool, &p, f.payer_account);
    let b = binding(
        &s,
        &f.customer.secret,
        "service-demoted-after-grant",
        "hello",
    )
    .await;
    sqlx::query("UPDATE core_identity.users SET platform_role='admin' WHERE id=90001")
        .execute(&pool)
        .await
        .unwrap();
    assert_eq!(
        s.reserve_text(&f.customer.secret, &b, "hello")
            .await
            .unwrap_err(),
        Error::Forbidden
    );
    let next = s
        .conversation(&f.customer.secret, "service-demoted-before-grant", true)
        .await
        .unwrap();
    assert_eq!(
        s.turn(
            &f.customer.secret,
            &next.core_conversation_id,
            "turn-1",
            true
        )
        .await
        .unwrap_err(),
        Error::Forbidden
    );
    assert_eq!(p.hits.load(Ordering::SeqCst), 0);
    let rows: i64 = sqlx::query_scalar("SELECT count(*) FROM core_charging.requests")
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(rows, 0);
    assert_eq!(
        f.ledger
            .balance(f.payer_account)
            .await
            .unwrap()
            .available_units,
        100000
    );
    assert_eq!(
        f.ledger
            .balance(f.payer_account)
            .await
            .unwrap()
            .reserved_units,
        0
    );
}

#[sqlx::test]
async fn service_funding_preserves_the_real_payer_account_budget(pool: PgPool) {
    let f = setup(&pool).await;
    let charging = Charging::new(pool.clone(), Arc::new(TransactionalLedger));
    charging
        .set_budget(
            &f.root.secret,
            &Budget {
                scope: "account".into(),
                account_id: Some(f.payer_account),
                user_id: None,
                key_id: None,
                period: "day".into(),
                anchor: 0,
                seconds: 0,
                limit: 10,
            },
        )
        .await
        .unwrap();
    let p = Provider::start(true).await;
    let s = funded(&pool, &p, f.payer_account);
    let b = binding(&s, &f.customer.secret, "service-budget-limited", "hello").await;
    assert_eq!(
        s.reserve_text(&f.customer.secret, &b, "hello")
            .await
            .unwrap_err(),
        Error::NoFunds
    );
    assert_eq!(p.hits.load(Ordering::SeqCst), 0);
    let rows: i64 = sqlx::query_scalar("SELECT count(*) FROM core_charging.requests")
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(rows, 0);
    assert_eq!(
        f.ledger
            .balance(f.payer_account)
            .await
            .unwrap()
            .available_units,
        100000
    );
}

#[sqlx::test]
async fn service_funding_requires_a_session_and_retains_single_request_ceiling(pool: PgPool) {
    let f = setup(&pool).await;
    let p = Provider::start(true).await;
    let mut limited = policy(f.payer_account);
    limited.budget_credits = 1;
    let s = AssistantModel::new(pool.clone(), gateway(&pool, &p), limited).unwrap();
    assert!(matches!(
        s.conversation("invalid-session", "invalid", true).await,
        Err(Error::Unauthorized | Error::Invalid)
    ));
    let b = binding(
        &s,
        &f.customer.secret,
        "service-single-request-limit",
        "hello",
    )
    .await;
    assert_eq!(
        s.reserve_text(&f.customer.secret, &b, "hello")
            .await
            .unwrap_err(),
        Error::NoFunds
    );
    assert_eq!(p.hits.load(Ordering::SeqCst), 0);
    assert_eq!(
        f.ledger
            .balance(f.payer_account)
            .await
            .unwrap()
            .reserved_units,
        0
    );
    assert!(f.identity.authorize(&f.customer.secret).await.is_ok());
}

#[path = "service_funding/anthropic.rs"]
mod anthropic;

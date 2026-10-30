//! User-initiated closure and its financial refusal paths on real synthetic PG.
use lmm_core::{
    core_schema,
    identity::{
        AccountClosureOutcome, AccountClosureRequest, BrowserAuth, BrowserBundle,
        BrowserRegistration, BrowserSessionMetadata, IdentityError, IdentityStore,
        admin_users::{AdjustmentRequest, AdminUsersStore},
    },
};
use sqlx::PgPool;

#[path = "common/runtime.rs"]
mod runtime;
const KEY: &str = "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";
const PASSWORD: &str = "closure fixture";
async fn fixture(
    pool: &PgPool,
) -> (
    runtime::RuntimeRole,
    IdentityStore,
    BrowserAuth,
    BrowserBundle,
) {
    core_schema::init(pool).await.unwrap();
    let role = runtime::RuntimeRole::create(pool).await;
    role.optional("web_identity").await;
    let store = IdentityStore::from_pool(role.pool.clone()).with_registration_enabled(true);
    let browser = BrowserAuth::new(store.clone(), &[43; 32]).unwrap();
    browser
        .register(&BrowserRegistration {
            username: "closure_member",
            password: PASSWORD,
            email: "",
        })
        .await
        .unwrap();
    let bundle = browser
        .login(
            "closure_member",
            PASSWORD,
            &BrowserSessionMetadata::default(),
        )
        .await
        .unwrap();
    (role, store, browser, bundle)
}
fn access(bundle: &BrowserBundle) -> &str {
    bundle.data["access_token"].as_str().unwrap()
}
fn user(bundle: &BrowserBundle) -> i64 {
    bundle.data["user"]["id"].as_i64().unwrap()
}
async fn request(browser: &BrowserAuth, bundle: &BrowserBundle) -> AccountClosureRequest {
    let view = browser
        .account_closure_preview(access(bundle))
        .await
        .unwrap();
    AccountClosureRequest {
        expected_auth_version: view.expected_auth_version,
        confirmation: view.username,
        operation_key: KEY.into(),
    }
}
fn gift() -> AdjustmentRequest {
    AdjustmentRequest {
        operation_key: "synthetic-closure-funding-00000001".into(),
        delta_units: "100".into(),
        reason: "Synthetic closure funding".into(),
    }
}

#[sqlx::test(migrations = false)]
async fn explicit_closure_preserves_history_revokes_all_authority_and_recovers_the_same_receipt(
    pool: PgPool,
) {
    let (role, store, browser, original) = fixture(&pool).await;
    let other = browser
        .login(
            "closure_member",
            PASSWORD,
            &BrowserSessionMetadata::default(),
        )
        .await
        .unwrap();
    let mut input = request(&browser, &original).await;
    input.confirmation = "different-account".into();
    assert!(
        browser
            .close_account(access(&original), &input)
            .await
            .is_err()
    );
    input.confirmation = "closure_member".into();
    let AccountClosureOutcome::Closed(receipt) = browser
        .close_account(access(&original), &input)
        .await
        .unwrap()
    else {
        panic!("empty native account unexpectedly blocked")
    };
    assert!(receipt.closed);
    assert_eq!(receipt.operation_key, KEY);
    for token in [access(&original), access(&other)] {
        assert_eq!(
            store.authorize(token).await.err(),
            Some(IdentityError::Unauthorized)
        );
    }
    assert!(
        browser
            .refresh(&original.refresh_token, None)
            .await
            .is_err()
    );
    assert!(
        browser
            .login(
                "closure_member",
                PASSWORD,
                &BrowserSessionMetadata::default()
            )
            .await
            .is_err()
    );
    assert!(
        browser
            .close_account(access(&original), &input)
            .await
            .is_err()
    );
    assert!(
        browser
            .account_closure_receipt(KEY)
            .await
            .unwrap()
            .unwrap()
            .closed
    );
    assert!(
        browser
            .account_closure_receipt(&"b".repeat(64))
            .await
            .unwrap()
            .is_none()
    );
    let facts: (bool,bool,i64,i64) = sqlx::query_as("SELECT u.active,a.active,(SELECT count(*) FROM core_identity.audit WHERE actor_user_id=u.id AND action='account.close'),(SELECT count(*) FROM core_billing.ledger_journals WHERE actor_user_id=u.id) FROM core_identity.users u JOIN core_identity.accounts a ON a.id=u.personal_account_id WHERE u.id=$1")
        .bind(user(&original)).fetch_one(&pool).await.unwrap();
    assert_eq!(facts, (false, false, 1, 0));
    assert!(
        sqlx::query("DELETE FROM core_account_lifecycle.closures")
            .execute(&pool)
            .await
            .is_err()
    );
    role.cleanup().await;
}

#[sqlx::test(migrations = false)]
async fn financial_entitlements_pending_payments_and_team_ownership_are_explicit_refusals(
    pool: PgPool,
) {
    let (role, store, browser, original) = fixture(&pool).await;
    let admin = IdentityStore::from_pool(pool.clone())
        .bootstrap_user(900, 5)
        .await
        .unwrap();
    AdminUsersStore::new(store.clone())
        .adjust_user_balance(&admin.secret, user(&original), &gift())
        .await
        .unwrap();
    store.create_team(access(&original)).await.unwrap();
    let principal = store.authorize(access(&original)).await.unwrap();
    let mut tx = pool.begin().await.unwrap();
    sqlx::query("SET TRANSACTION ISOLATION LEVEL SERIALIZABLE")
        .execute(&mut *tx)
        .await
        .unwrap();
    sqlx::query("INSERT INTO core_charging.subscriptions(account_id,grant_reference,period,anchor,ends_at,limit_credits,models,groups) VALUES($1,'synthetic-close-entitlement','grant',floor(extract(epoch FROM clock_timestamp()))::bigint,floor(extract(epoch FROM clock_timestamp()))::bigint+3600,100,ARRAY['synthetic'],ARRAY['default'])")
        .bind(principal.owner_account_id).execute(&mut *tx).await.unwrap();
    sqlx::query("INSERT INTO core_payments.intents(account_id,created_by_user_id,channel_id,provider,merchant,environment,currency,amount_minor,credit_units,credits_per_minor,prepare_key,prepare_hash) VALUES($1,$2,1,'stripe','synthetic','test','USD',1,5000,5000,'synthetic-closure-prepare',$3)")
        .bind(principal.owner_account_id).bind(user(&original)).bind(vec![3u8;32]).execute(&mut *tx).await.unwrap();
    tx.commit().await.unwrap();
    let view = browser
        .account_closure_preview(access(&original))
        .await
        .unwrap();
    for reason in [
        "unsettled_balance",
        "owned_team",
        "active_subscription",
        "payment_or_refund_pending",
    ] {
        assert!(view.blockers.contains(&reason));
    }
    let input = AccountClosureRequest {
        expected_auth_version: view.expected_auth_version,
        confirmation: view.username,
        operation_key: KEY.into(),
    };
    assert!(matches!(
        browser
            .close_account(access(&original), &input)
            .await
            .unwrap(),
        AccountClosureOutcome::Blocked(_)
    ));
    assert!(store.authorize(access(&original)).await.is_ok());
    assert!(
        browser
            .account_closure_receipt(KEY)
            .await
            .unwrap()
            .is_none()
    );
    // The closure lock function does not require or grant direct balance writes.
    assert!(
        sqlx::query("UPDATE core_billing.balance_state SET balance_units=0")
            .execute(&role.pool)
            .await
            .is_err()
    );
    role.cleanup().await;
}

#[sqlx::test(migrations = false)]
async fn concurrent_credit_and_closure_never_leave_a_closed_funded_account(pool: PgPool) {
    let (role, store, browser, original) = fixture(&pool).await;
    let admin = IdentityStore::from_pool(pool.clone())
        .bootstrap_user(900, 5)
        .await
        .unwrap();
    let manager = AdminUsersStore::new(store.clone());
    let request = request(&browser, &original).await;
    let adjustment = gift();
    let (closed, credited) = tokio::join!(
        browser.close_account(access(&original), &request),
        manager.adjust_user_balance(&admin.secret, user(&original), &adjustment),
    );
    let (active, balance): (bool,String) = sqlx::query_as("SELECT u.active,COALESCE((SELECT sum(b.balance_units::numeric)::text FROM core_billing.ledger_accounts l JOIN core_billing.balance_state b ON b.ledger_account_id=l.id WHERE l.owner_account_id=u.personal_account_id),'0') FROM core_identity.users u WHERE u.id=$1")
        .bind(user(&original)).fetch_one(&pool).await.unwrap();
    if matches!(closed, Ok(AccountClosureOutcome::Closed(_))) {
        assert!(!active);
        assert_eq!(balance, "0");
        assert!(credited.is_err());
    } else {
        assert!(active);
        if let Ok(receipt) = credited {
            assert_eq!(receipt.status, "posted");
            assert_eq!(balance, "100");
        }
    }
    role.cleanup().await;
}

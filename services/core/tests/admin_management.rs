//! Administrator mutations verified through the public Core stores and a
//! restricted PostgreSQL runtime role. No production source is copied here.
use lmm_core::{
    billing::ledger::Ledger,
    identity::{
        self, IdentityStore,
        admin_users::{
            AdjustmentRequest, AdminUsersStore, ProfileError, SecurityAction, SecurityRequest,
        },
    },
};
use sqlx::PgPool;
#[path = "common/runtime.rs"]
mod runtime;
use runtime::RuntimeRole;

const KEY: &str = "synthetic-adjustment-operation-0001";
const SCOPE: &str = "core-admin-user-adjustment";
fn security_request(version: &str, action: SecurityAction) -> SecurityRequest {
    SecurityRequest {
        expected_auth_version: version.to_owned(),
        action,
        reason: "Synthetic administrator recovery".into(),
    }
}
fn adjustment_request(delta: &str, key: &str) -> AdjustmentRequest {
    AdjustmentRequest {
        operation_key: key.into(),
        delta_units: delta.into(),
        reason: "Synthetic correction".into(),
    }
}

#[sqlx::test(migrations = false)]
async fn suspension_enable_and_revocation_use_restricted_runtime_and_fresh_cas(pool: PgPool) {
    lmm_core::core_schema::init(&pool).await.unwrap();
    let owner = IdentityStore::from_pool(pool.clone());
    let admin = owner.bootstrap_user(900, 5).await.unwrap();
    let user = owner.bootstrap_user(10, 0).await.unwrap();
    let role = RuntimeRole::create(&pool).await;
    let store = AdminUsersStore::new(IdentityStore::from_pool(role.pool.clone()));

    let before = store.user_security(&admin.secret, 10).await.unwrap();
    assert!(before.active);
    let suspended = store
        .update_user_security(
            &admin.secret,
            10,
            &security_request(&before.auth_version, SecurityAction::Disable),
        )
        .await
        .unwrap();
    assert!(suspended.changed);
    assert!(!suspended.user.active);
    assert_eq!(
        owner.authorize(&user.secret).await.err(),
        Some(identity::IdentityError::Unauthorized)
    );

    assert!(matches!(
        store
            .update_user_security(
                &admin.secret,
                10,
                &security_request(&before.auth_version, SecurityAction::Enable)
            )
            .await,
        Err(ProfileError::Conflict)
    ));
    assert!(!store.user_security(&admin.secret, 10).await.unwrap().active);
    let enabled = store
        .update_user_security(
            &admin.secret,
            10,
            &security_request(&suspended.user.auth_version, SecurityAction::Enable),
        )
        .await
        .unwrap();
    assert!(enabled.user.active);
    // Re-enabling never resurrects the old bearer.
    assert_eq!(
        owner.authorize(&user.secret).await.err(),
        Some(identity::IdentityError::Unauthorized)
    );
    let actions: Vec<String> = sqlx::query_scalar("SELECT action FROM core_identity.audit WHERE target->>'user_id'='10' AND action LIKE 'admin.user.%' ORDER BY id")
        .fetch_all(&pool).await.unwrap();
    assert_eq!(actions, ["admin.user.disable", "admin.user.enable"]);
    role.cleanup().await;
}

#[sqlx::test(migrations = false)]
async fn management_rejects_self_privileged_targets_and_ordinary_callers(pool: PgPool) {
    lmm_core::core_schema::init(&pool).await.unwrap();
    let owner = IdentityStore::from_pool(pool.clone());
    let admin = owner.bootstrap_user(900, 5).await.unwrap();
    let root = owner.bootstrap_user(901, 6).await.unwrap();
    let user = owner.bootstrap_user(10, 0).await.unwrap();
    let role = RuntimeRole::create(&pool).await;
    let store = AdminUsersStore::new(IdentityStore::from_pool(role.pool.clone()));
    for (secret, id) in [
        (&user.secret, 900),
        (&admin.secret, 900),
        (&admin.secret, 901),
        (&root.secret, 900),
    ] {
        assert!(matches!(
            store
                .update_user_security(secret, id, &security_request("1", SecurityAction::ResetMfa))
                .await,
            Err(ProfileError::Forbidden)
        ));
    }
    let changed: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM core_identity.audit WHERE action LIKE 'admin.user.%'",
    )
    .fetch_one(&pool)
    .await
    .unwrap();
    assert_eq!(changed, 0);
    role.cleanup().await;
}

#[sqlx::test(migrations = false)]
async fn mfa_reset_invalidates_factors_recovery_codes_and_all_credentials(pool: PgPool) {
    lmm_core::core_schema::init(&pool).await.unwrap();
    let owner = IdentityStore::from_pool(pool.clone());
    let admin = owner.bootstrap_user(900, 5).await.unwrap();
    let user = owner.bootstrap_user(10, 0).await.unwrap();
    sqlx::query("INSERT INTO core_identity.mfa_factors(user_id,enabled,key_id,encrypted_secret) VALUES (10,TRUE,'synthetic',decode(repeat('aa',64),'hex'))")
        .execute(&pool).await.unwrap();
    sqlx::query("INSERT INTO core_identity.mfa_backup_codes(user_id,factor_revision,verifier) VALUES(10,1,repeat('x',32))")
        .execute(&pool).await.unwrap();
    sqlx::query(
        "UPDATE core_identity.user_profiles SET second_factor_required=TRUE WHERE user_id=10",
    )
    .execute(&pool)
    .await
    .unwrap();
    let role = RuntimeRole::create(&pool).await;
    let store = AdminUsersStore::new(IdentityStore::from_pool(role.pool.clone()));
    let before = store.user_security(&admin.secret, 10).await.unwrap();
    assert!(before.mfa_enabled);
    let after = store
        .update_user_security(
            &admin.secret,
            10,
            &security_request(&before.auth_version, SecurityAction::ResetMfa),
        )
        .await
        .unwrap();
    assert!(!after.user.mfa_enabled);
    assert_ne!(after.user.auth_version, before.auth_version);
    let (enabled, secret, needed, consumed): (bool, bool, bool, bool) = sqlx::query_as(
        "SELECT f.enabled,f.encrypted_secret IS NOT NULL,p.second_factor_required,
         (SELECT bool_and(consumed_at IS NOT NULL) FROM core_identity.mfa_backup_codes WHERE user_id=10)
         FROM core_identity.mfa_factors f JOIN core_identity.user_profiles p ON p.user_id=f.user_id WHERE f.user_id=10")
        .fetch_one(&pool).await.unwrap();
    assert_eq!(
        (enabled, secret, needed, consumed),
        (false, false, false, true)
    );
    assert_eq!(
        owner.authorize(&user.secret).await.err(),
        Some(identity::IdentityError::Unauthorized)
    );
    role.cleanup().await;
}

#[sqlx::test(migrations = false)]
async fn adjustments_post_once_use_ledger_and_preserve_permanent_business_rejection(pool: PgPool) {
    lmm_core::core_schema::init(&pool).await.unwrap();
    let owner = IdentityStore::from_pool(pool.clone());
    let admin = owner.bootstrap_user(900, 5).await.unwrap();
    owner.bootstrap_user(10, 0).await.unwrap();
    let role = RuntimeRole::create(&pool).await;
    let store = AdminUsersStore::new(IdentityStore::from_pool(role.pool.clone()));
    let ledger = Ledger::from_pool(role.pool.clone());
    let account: i64 =
        sqlx::query_scalar("SELECT personal_account_id FROM core_identity.users WHERE id=10")
            .fetch_one(&pool)
            .await
            .unwrap();
    let credit = adjustment_request("100", KEY);
    let (a, b) = tokio::join!(
        store.adjust_user_balance(&admin.secret, 10, &credit),
        store.adjust_user_balance(&admin.secret, 10, &credit)
    );
    let posted = a.unwrap();
    assert_eq!(posted, b.unwrap());
    assert_eq!(posted.status, "posted");
    assert_eq!(posted.delta_units, "100");
    assert_eq!(ledger.balance(account).await.unwrap().available_units, 100);
    assert_eq!(
        store
            .user_balance_adjustment(&admin.secret, 10, KEY)
            .await
            .unwrap(),
        posted
    );
    assert!(matches!(
        store
            .adjust_user_balance(&admin.secret, 10, &adjustment_request("101", KEY))
            .await,
        Err(ProfileError::Conflict)
    ));
    let rejection = adjustment_request("-101", "synthetic-adjustment-operation-0002");
    let rejected = store
        .adjust_user_balance(&admin.secret, 10, &rejection)
        .await
        .unwrap();
    assert_eq!(rejected.status, "rejected");
    assert_eq!(rejected.rejection.as_deref(), Some("insufficient_funds"));
    store
        .adjust_user_balance(
            &admin.secret,
            10,
            &adjustment_request("100", "synthetic-adjustment-operation-0003"),
        )
        .await
        .unwrap();
    assert_eq!(
        store
            .adjust_user_balance(&admin.secret, 10, &rejection)
            .await
            .unwrap(),
        rejected
    );
    assert_eq!(ledger.balance(account).await.unwrap().available_units, 200);
    store
        .adjust_user_balance(
            &admin.secret,
            10,
            &adjustment_request("-30", "synthetic-adjustment-operation-0004"),
        )
        .await
        .unwrap();
    assert_eq!(ledger.balance(account).await.unwrap().available_units, 170);
    let audits: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM core_identity.audit WHERE action='admin.user.balance.adjust'",
    )
    .fetch_one(&pool)
    .await
    .unwrap();
    assert_eq!(audits, 4);
    let broken: i64 = sqlx::query_scalar("SELECT count(*) FROM (SELECT journal_id FROM core_billing.ledger_entries GROUP BY journal_id HAVING sum(delta_units::numeric)<>0) invalid").fetch_one(&pool).await.unwrap();
    assert_eq!(broken, 0);
    role.cleanup().await;
}

#[sqlx::test(migrations = false)]
async fn adjustments_deny_user_self_privileged_and_unestablished_historical_wallet(pool: PgPool) {
    lmm_core::core_schema::init(&pool).await.unwrap();
    let owner = IdentityStore::from_pool(pool.clone());
    let admin = owner.bootstrap_user(900, 5).await.unwrap();
    let user = owner.bootstrap_user(10, 0).await.unwrap();
    let role = RuntimeRole::create(&pool).await;
    let store = AdminUsersStore::new(IdentityStore::from_pool(role.pool.clone()));
    for (secret, target) in [
        (&user.secret, 10),
        (&user.secret, 900),
        (&admin.secret, 900),
    ] {
        assert!(matches!(
            store
                .adjust_user_balance(secret, target, &adjustment_request("1", KEY))
                .await,
            Err(ProfileError::Forbidden)
        ));
    }
    // A historical-looking identity has no authoritative native zero audit and
    // no imported ledger opening. Neither provisioning nor a journal may occur.
    let account: i64 = sqlx::query_scalar(
        "INSERT INTO core_identity.accounts(kind) VALUES('personal') RETURNING id",
    )
    .fetch_one(&pool)
    .await
    .unwrap();
    sqlx::query("INSERT INTO core_identity.users(id,personal_account_id) VALUES(11,$1)")
        .bind(account)
        .execute(&pool)
        .await
        .unwrap();
    assert!(matches!(
        store
            .adjust_user_balance(&admin.secret, 11, &adjustment_request("1", KEY))
            .await,
        Err(ProfileError::Unavailable)
    ));
    let rows: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM core_billing.ledger_accounts WHERE owner_account_id=$1",
    )
    .bind(account)
    .fetch_one(&pool)
    .await
    .unwrap();
    assert_eq!(rows, 0);
    let operations: i64 =
        sqlx::query_scalar("SELECT count(*) FROM core_billing.ledger_operations WHERE scope=$1")
            .bind(SCOPE)
            .fetch_one(&pool)
            .await
            .unwrap();
    assert_eq!(operations, 0);
    role.cleanup().await;
}

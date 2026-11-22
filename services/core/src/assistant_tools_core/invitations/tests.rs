use super::*;
use crate::{identity::IdentityStore, test_runtime::RuntimeRole};
use serde_json::{Value, json};

async fn runtime(pool: &PgPool) -> RuntimeRole {
    let runtime = RuntimeRole::create(pool).await;
    sqlx::raw_sql(sqlx::AssertSqlSafe(format!(
        "GRANT SELECT ON core_identity.affiliate_codes,core_identity.affiliate_registrations TO {};
         GRANT SELECT(code,user_id) ON core_identity.legacy_affiliate_aliases TO {};
         GRANT USAGE ON SCHEMA core_charging TO {};
         GRANT SELECT ON core_charging.requests TO {}",
        runtime.name, runtime.name, runtime.name, runtime.name
    )))
    .execute(pool)
    .await
    .unwrap();
    runtime
}
async fn state(pool: &PgPool) -> Value {
    let mut result = serde_json::Map::new();
    for table in [
        "core_identity.affiliate_codes",
        "core_identity.legacy_affiliate_aliases",
        "core_identity.affiliate_registrations",
        "core_identity.credentials",
        "core_billing.ledger_journals",
        "core_billing.balance_state",
        "core_billing.account_opening_counters",
    ] {
        let rows:Value=sqlx::query_scalar(sqlx::AssertSqlSafe(format!(
            "SELECT coalesce(jsonb_agg(to_jsonb(t) ORDER BY to_jsonb(t)::text),'[]'::jsonb) FROM {table} t"
        ))).fetch_one(pool).await.unwrap();
        result.insert(table.into(), rows);
    }
    Value::Object(result)
}
#[sqlx::test(migrations = false)]
async fn invitation_facts_read_current_self_without_creating_code_or_money(pool: PgPool) {
    crate::core_schema::init(&pool).await.unwrap();
    let identity = IdentityStore::from_pool(pool.clone());
    let user = identity.bootstrap_user(94501, 0).await.unwrap().secret;
    let other = identity.bootstrap_user(94502, 0).await.unwrap().secret;
    let runtime = runtime(&pool).await;
    check_ready(&runtime.pool).await.unwrap();
    let before = state(&pool).await;
    let empty = read(&runtime.pool, &user).await.unwrap();
    assert_eq!(empty.actor_user_id, 94501);
    assert!(empty.invitation_code.is_none());
    assert!(!empty.automatic_signup_rewards_enabled);
    assert!(!empty.legacy_invited_count_available);
    assert!(empty.legacy_invited_count.is_none());
    assert!(!empty.financial_authority);
    assert_eq!(empty.native_registered_referrals, 0);
    assert_eq!(state(&pool).await, before);
    let code = "a".repeat(32);
    sqlx::query("INSERT INTO core_identity.affiliate_codes(user_id,code) VALUES($1,$2)")
        .bind(94501_i64)
        .bind(&code)
        .execute(&pool)
        .await
        .unwrap();
    let mut tx = pool.begin().await.unwrap();
    crate::affiliate::bind_registration(&mut tx, 94502, &code)
        .await
        .unwrap();
    tx.commit().await.unwrap();
    let before = state(&pool).await;
    let facts = read(&runtime.pool, &user).await.unwrap();
    assert_eq!(facts.invitation_code.as_deref(), Some(code.as_str()));
    assert_eq!(facts.native_registered_referrals, 1);
    assert_eq!(facts.balance_status, "available");
    let balances = facts.balances.unwrap();
    assert_eq!(balances.wallet_signed_credits, 0);
    assert_eq!(balances.affiliate_available_credits, 0);
    assert_eq!(balances.affiliate_debt_credits, 0);
    assert_eq!(balances.opening_affiliate_history_credits, 0);
    let other_facts = read(&runtime.pool, &other).await.unwrap();
    assert_eq!(other_facts.actor_user_id, 94502);
    assert!(other_facts.invitation_code.is_none());
    assert_eq!(other_facts.native_registered_referrals, 0);
    assert_eq!(state(&pool).await, before);
    runtime.cleanup().await;
}
#[sqlx::test(migrations = false)]
async fn invitation_facts_require_actual_alias_columns_and_current_account(pool: PgPool) {
    crate::core_schema::init(&pool).await.unwrap();
    let identity = IdentityStore::from_pool(pool.clone());
    let secret = identity.bootstrap_user(94511, 0).await.unwrap().secret;
    let actor = identity.authorize(&secret).await.unwrap();
    let runtime = runtime(&pool).await;
    check_ready(&runtime.pool).await.unwrap();
    sqlx::raw_sql(sqlx::AssertSqlSafe(format!(
        "REVOKE SELECT ON core_identity.legacy_affiliate_aliases FROM {};
         REVOKE SELECT(code,user_id) ON core_identity.legacy_affiliate_aliases FROM {}",
        runtime.name, runtime.name
    )))
    .execute(&pool)
    .await
    .unwrap();
    assert!(check_ready(&runtime.pool).await.is_err());
    sqlx::raw_sql(sqlx::AssertSqlSafe(format!(
        "GRANT SELECT(code,user_id) ON core_identity.legacy_affiliate_aliases TO {}",
        runtime.name
    )))
    .execute(&pool)
    .await
    .unwrap();
    check_ready(&runtime.pool).await.unwrap();
    sqlx::query("UPDATE core_identity.accounts SET active=false WHERE id=$1")
        .bind(actor.owner_account_id)
        .execute(&pool)
        .await
        .unwrap();
    let before = state(&pool).await;
    assert!(matches!(
        read(&runtime.pool, &secret).await,
        Err(Error::Unauthorized)
    ));
    assert_eq!(state(&pool).await, before);
    runtime.cleanup().await;
}
#[sqlx::test(migrations = false)]
async fn invitation_draft_binds_actual_recipient_own_code_and_session(pool: PgPool) {
    crate::core_schema::init(&pool).await.unwrap();
    let identity = IdentityStore::from_pool(pool.clone());
    let secret = identity.bootstrap_user(94521, 0).await.unwrap().secret;
    let code = "b".repeat(32);
    sqlx::query("INSERT INTO core_identity.affiliate_codes(user_id,code) VALUES($1,$2)")
        .bind(94521_i64)
        .bind(&code)
        .execute(&pool)
        .await
        .unwrap();
    let runtime = runtime(&pool).await;
    let before = state(&pool).await;
    let mut tx = runtime.pool.begin().await.unwrap();
    let actor = context::from_session(&mut tx, &secret).await.unwrap();
    let source = json!({"email":"Recipient+tag@Example.Invalid"});
    let draft = json!({"kind":"invitation_email_v1","email":"recipient+tag@example.invalid","invitation_code":code});
    validate_draft(&mut tx, &actor, &source, &draft)
        .await
        .unwrap();
    for invalid in [
        json!({"kind":"invitation_email_v1","email":"other@example.invalid","invitation_code":code}),
        json!({"kind":"invitation_email_v1","email":"recipient+tag@example.invalid","invitation_code":"c".repeat(32)}),
        json!({"kind":"invitation_email_v1","email":"recipient+tag@example.invalid","invitation_code":code,"actor":94522}),
        json!({"kind":"other","email":"recipient+tag@example.invalid","invitation_code":code}),
    ] {
        assert!(
            validate_draft(&mut tx, &actor, &source, &invalid)
                .await
                .is_err()
        );
    }
    for invalid in [
        json!({"email":"recipient@example.invalid","invitation_code":code}),
        json!({"email":"Display <recipient@example.invalid>"}),
        json!({"email":"recipient@example.invalid\nBcc: other@example.invalid"}),
    ] {
        assert!(
            validate_draft(&mut tx, &actor, &invalid, &draft)
                .await
                .is_err()
        );
    }
    tx.rollback().await.unwrap();
    assert_eq!(state(&pool).await, before);
    runtime.cleanup().await;
}

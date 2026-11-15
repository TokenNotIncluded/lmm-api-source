//! Optional proof storage is verified without granting mutable reward history.
use crate::{
    core_schema,
    runtime_database::{self, RequiredCapabilities, RuntimeDatabaseError},
    test_runtime::RuntimeRole,
};
use sqlx::PgPool;

#[sqlx::test(migrations = false)]
async fn checkin_rewards_require_narrow_grants_and_reject_unsafe_authority(pool: PgPool) {
    core_schema::init(&pool).await.unwrap();
    let role = RuntimeRole::create(&pool).await;
    role.optional("web_identity").await;
    let required = RequiredCapabilities {
        checkin_rewards: true,
        web_identity: true,
        ..Default::default()
    };
    assert_eq!(
        runtime_database::check(&role.pool, required).await,
        Err(RuntimeDatabaseError::MissingPrivileges)
    );
    role.optional("core_checkin_challenges").await;
    runtime_database::check(&role.pool, required).await.unwrap();
    for privilege in [
        "UPDATE(consumed_at) ON core_checkin_challenges.challenges",
        "INSERT ON core_checkin_challenges.receipts",
    ] {
        sqlx::raw_sql(sqlx::AssertSqlSafe(format!(
            "REVOKE {privilege} FROM {}",
            role.name
        )))
        .execute(&pool)
        .await
        .unwrap();
        assert_eq!(
            runtime_database::check(&role.pool, required).await,
            Err(RuntimeDatabaseError::MissingPrivileges)
        );
        sqlx::raw_sql(sqlx::AssertSqlSafe(format!(
            "GRANT {privilege} TO {}",
            role.name
        )))
        .execute(&pool)
        .await
        .unwrap();
    }
    for privilege in [
        "UPDATE(sid) ON core_checkin_challenges.challenges",
        "UPDATE(expires_at) ON core_checkin_challenges.challenges",
        "UPDATE(amount_units) ON core_checkin_challenges.receipts",
        "DELETE ON core_checkin_challenges.receipts",
        "SELECT ON core_meta.checkin_challenges_upgrades",
    ] {
        sqlx::raw_sql(sqlx::AssertSqlSafe(format!(
            "GRANT {privilege} TO {}",
            role.name
        )))
        .execute(&pool)
        .await
        .unwrap();
        assert_eq!(
            runtime_database::check(&role.pool, required).await,
            Err(RuntimeDatabaseError::UnsafePrivileges),
            "{privilege}"
        );
        assert_eq!(
            runtime_database::check(&role.pool, Default::default()).await,
            Err(RuntimeDatabaseError::UnsafePrivileges),
            "{privilege}"
        );
        sqlx::raw_sql(sqlx::AssertSqlSafe(format!(
            "REVOKE {privilege} FROM {}",
            role.name
        )))
        .execute(&pool)
        .await
        .unwrap();
    }
    role.cleanup().await;
}

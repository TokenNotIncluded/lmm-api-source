use super::*;
use crate::{core_schema, test_runtime::RuntimeRole};

#[sqlx::test(migrations = false)]
async fn exact_route_grants_pass_startup_and_excess_authority_is_rejected(pool: sqlx::PgPool) {
    core_schema::init(&pool).await.unwrap();
    let role = RuntimeRole::create(&pool).await;
    let mut connection = role.pool.acquire().await.unwrap();
    // The accounting checker still rejects privileges on unknown core_channels objects.
    super::super::channel_accounting::check(&mut connection, true)
        .await
        .unwrap();
    check(&mut connection, true).await.unwrap();

    for (permission, grantee) in [
        (
            "UPDATE(key_sha256) ON core_channels.affinity_cache",
            role.name.as_str(),
        ),
        (
            "UPDATE(credential_revision) ON core_channels.route_health",
            role.name.as_str(),
        ),
        ("DELETE ON core_channels.route_health", role.name.as_str()),
        (
            "TRUNCATE ON core_channels.affinity_cache",
            role.name.as_str(),
        ),
        (
            "SELECT(upgrade_id) ON core_meta.route_runtime_upgrades",
            role.name.as_str(),
        ),
        (
            "SELECT(channel_id) ON core_channels.affinity_cache",
            "PUBLIC",
        ),
    ] {
        sqlx::raw_sql(sqlx::AssertSqlSafe(format!(
            "GRANT {permission} TO {grantee}"
        )))
        .execute(&pool)
        .await
        .unwrap();
        assert_eq!(
            check(&mut connection, false).await,
            Err(RuntimeDatabaseError::UnsafePrivileges)
        );
        sqlx::raw_sql(sqlx::AssertSqlSafe(format!(
            "REVOKE {permission} FROM {grantee}"
        )))
        .execute(&pool)
        .await
        .unwrap();
        check(&mut connection, true).await.unwrap();
    }
    sqlx::raw_sql(sqlx::AssertSqlSafe(format!(
        "REVOKE UPDATE(generation) ON core_channels.route_health FROM {}",
        role.name
    )))
    .execute(&pool)
    .await
    .unwrap();
    assert_eq!(
        check(&mut connection, true).await,
        Err(RuntimeDatabaseError::MissingPrivileges)
    );
    check(&mut connection, false).await.unwrap();
    drop(connection);
    role.cleanup().await;
}

use super::*;
use crate::{core_schema, test_runtime::RuntimeRole};
#[sqlx::test(migrations = false)]
async fn column_grants_are_sufficient_and_wider_authority_is_rejected(pool: sqlx::PgPool) {
    core_schema::init(&pool).await.unwrap();
    let role = RuntimeRole::create(&pool).await;
    let mut c = role.pool.acquire().await.unwrap();
    check(&mut c, true).await.unwrap();
    for (grant, revoke) in [
        (
            "SELECT ON core_channels.catalog_observations",
            "SELECT ON core_channels.catalog_observations",
        ),
        (
            "UPDATE(channel_id) ON core_channels.catalog_observations",
            "UPDATE(channel_id) ON core_channels.catalog_observations",
        ),
        (
            "DELETE ON core_channels.catalog_observations",
            "DELETE ON core_channels.catalog_observations",
        ),
        (
            "SELECT(upgrade_id) ON core_meta.channel_observations_upgrades",
            "SELECT(upgrade_id) ON core_meta.channel_observations_upgrades",
        ),
    ] {
        sqlx::raw_sql(&format!("GRANT {grant} TO {}", role.name))
            .execute(&pool)
            .await
            .unwrap();
        assert_eq!(
            check(&mut c, false).await,
            Err(RuntimeDatabaseError::UnsafePrivileges)
        );
        sqlx::raw_sql(&format!("REVOKE {revoke} FROM {}", role.name))
            .execute(&pool)
            .await
            .unwrap();
        // REVOKE of a table privilege also removes that privilege's column ACLs.
        let grants = include_str!("../../../deployment/postgres/grant-core-runtime.sql")
            .split_once("-- BEGIN channel-observations runtime grants\n")
            .unwrap()
            .1
            .split_once("-- END channel-observations runtime grants")
            .unwrap()
            .0
            .replace(":\"runtime_role\"", &role.name);
        sqlx::raw_sql(&grants).execute(&pool).await.unwrap();
        check(&mut c, true).await.unwrap();
    }
    sqlx::raw_sql("GRANT SELECT(duration_ms) ON core_channels.catalog_observations TO PUBLIC")
        .execute(&pool)
        .await
        .unwrap();
    assert_eq!(
        check(&mut c, true).await,
        Err(RuntimeDatabaseError::UnsafePrivileges)
    );
    sqlx::raw_sql("REVOKE SELECT(duration_ms) ON core_channels.catalog_observations FROM PUBLIC")
        .execute(&pool)
        .await
        .unwrap();
    sqlx::raw_sql(&format!(
        "REVOKE INSERT(duration_ms) ON core_channels.catalog_observations FROM {}",
        role.name
    ))
    .execute(&pool)
    .await
    .unwrap();
    assert_eq!(
        check(&mut c, true).await,
        Err(RuntimeDatabaseError::MissingPrivileges)
    );
    check(&mut c, false).await.unwrap();
    drop(c);
    role.cleanup().await;
}
#[sqlx::test(migrations = false)]
async fn inherited_and_grant_option_authority_is_rejected(pool: sqlx::PgPool) {
    core_schema::init(&pool).await.unwrap();
    let role = RuntimeRole::create(&pool).await;
    let group = format!("{}_obs", role.name);
    sqlx::raw_sql(&format!("CREATE ROLE {group} NOLOGIN; GRANT DELETE ON core_channels.catalog_observations TO {group}; GRANT {group} TO {}",role.name)).execute(&pool).await.unwrap();
    let mut c = role.pool.acquire().await.unwrap();
    assert_eq!(
        check(&mut c, false).await,
        Err(RuntimeDatabaseError::UnsafePrivileges)
    );
    sqlx::raw_sql(&format!("REVOKE {group} FROM {}; DROP OWNED BY {group}; DROP ROLE {group}; GRANT SELECT(duration_ms) ON core_channels.catalog_observations TO {} WITH GRANT OPTION",role.name,role.name)).execute(&pool).await.unwrap();
    assert_eq!(
        check(&mut c, false).await,
        Err(RuntimeDatabaseError::UnsafePrivileges)
    );
    drop(c);
    role.cleanup().await;
}

//! Display-only persistence and exact runtime column authority.
use lmm_core::{core_schema, runtime_database::RuntimeDatabaseError};
use sqlx::PgPool;
#[path = "../src/runtime_channel_accounting.rs"]
mod accounting;
#[path = "common/runtime.rs"]
mod runtime;

#[sqlx::test(migrations = false)]
async fn accounting_constraints_and_runtime_authority(pool: PgPool) {
    core_schema::init(&pool).await.unwrap();
    let count: i64 = sqlx::query_scalar("SELECT count(*) FROM core_channels.identities")
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(
        count, 0,
        "bootstrap registration belongs to the first authorized operation"
    );
    let role = runtime::RuntimeRole::create(&pool).await;
    sqlx::raw_sql("INSERT INTO core_channels.identities(id,origin) VALUES(1,'bootstrap'),(2,'native'),(3,'copy'),(9007199254740991,'migrated'); INSERT INTO core_channels.counter_baselines(channel_id) VALUES(1); INSERT INTO core_channels.balance_observations(channel_id) VALUES(1)")
        .execute(&role.pool).await.unwrap();
    sqlx::raw_sql("INSERT INTO core_channels.counter_baselines(channel_id,gross_credits,refunded_credits,source_channel_id,source_routing_version) VALUES(3,99999999999999999999999999999999999999,1,1,1)")
        .execute(&role.pool).await.unwrap();
    let unknown: bool = sqlx::query_scalar("SELECT balance IS NULL AND observed_at IS NULL AND kind='unknown' FROM core_channels.balance_observations WHERE channel_id=1")
        .fetch_one(&role.pool).await.unwrap();
    assert!(unknown);
    for statement in [
        "INSERT INTO core_channels.identities(id,origin) VALUES(0,'native')",
        "INSERT INTO core_channels.identities(id,origin) VALUES(9007199254740992,'native')",
        "INSERT INTO core_channels.identities(id,origin) VALUES(4,'other')",
        "INSERT INTO core_channels.counter_baselines(channel_id,gross_credits) VALUES(2,-1)",
        "INSERT INTO core_channels.counter_baselines(channel_id,refunded_credits) VALUES(2,1)",
        "INSERT INTO core_channels.counter_baselines(channel_id,gross_credits) VALUES(2,'NaN')",
        "INSERT INTO core_channels.counter_baselines(channel_id,source_routing_version) VALUES(2,0)",
        "INSERT INTO core_channels.counter_baselines(channel_id,source_channel_id) VALUES(2,42)",
        "INSERT INTO core_channels.counter_baselines(channel_id) VALUES(42)",
        "INSERT INTO core_channels.balance_observations(channel_id,balance) VALUES(2,1)",
        "INSERT INTO core_channels.balance_observations(channel_id,kind,balance) VALUES(2,'provider',1)",
        "INSERT INTO core_channels.balance_observations(channel_id,credential_id) VALUES(2,1)",
        "INSERT INTO core_channels.balance_observations(channel_id,credential_id,credential_revision) VALUES(2,0,1)",
        "INSERT INTO core_channels.balance_observations(channel_id,kind,balance) VALUES(2,'reset',NULL)",
        "INSERT INTO core_channels.balance_observations(channel_id,kind,balance) VALUES(2,'reset',1)",
        "INSERT INTO core_channels.balance_observations(channel_id,kind,balance) VALUES(2,'copied','NaN')",
        "INSERT INTO core_channels.balance_observations(channel_id,kind,balance) VALUES(2,'copied','Infinity')",
        "INSERT INTO core_channels.balance_observations(channel_id,kind,balance) VALUES(2,'copied','-Infinity')",
        "INSERT INTO core_channels.balance_observations(channel_id,unit_label) VALUES(2,repeat('a',33))",
        "INSERT INTO core_channels.balance_observations(channel_id,kind) VALUES(2,'other')",
    ] {
        let error = sqlx::raw_sql(statement)
            .execute(&role.pool)
            .await
            .unwrap_err();
        assert!(
            matches!(
                error.as_database_error().unwrap().code().as_deref(),
                Some("23514" | "23503")
            ),
            "{statement}: {error}"
        );
    }
    sqlx::raw_sql("UPDATE core_channels.identities SET retired_at=clock_timestamp() WHERE id=1; UPDATE core_channels.balance_observations SET kind='provider',balance=-2.5,unit_label='USD',observed_at='2020-01-01 UTC',credential_id=1,credential_revision=2 WHERE channel_id=1; UPDATE core_channels.balance_observations SET kind='reset',balance=0 WHERE channel_id=1; INSERT INTO core_channels.balance_observations(channel_id,kind,balance,observed_at,credential_id,credential_revision,source_channel_id) SELECT 3,'copied',balance,observed_at,credential_id,credential_revision,1 FROM core_channels.balance_observations WHERE channel_id=1")
        .execute(&role.pool).await.unwrap();
    // Copying an unknown source keeps NULLs; resetting it has no probe timestamp.
    sqlx::raw_sql("INSERT INTO core_channels.identities(id,origin) VALUES(4,'copy'),(5,'copy'); INSERT INTO core_channels.balance_observations(channel_id,balance,observed_at,kind,credential_id,credential_revision,source_channel_id) VALUES(4,NULL,NULL,'copied',7,9,2),(5,0,NULL,'reset',7,9,2)")
        .execute(&role.pool).await.unwrap();
    let nullable_copies: Vec<(i64,bool)> = sqlx::query_as("SELECT channel_id,observed_at IS NULL AND credential_id=7 AND credential_revision=9 AND source_channel_id=2 AND ((kind='copied' AND balance IS NULL) OR (kind='reset' AND balance=0)) FROM core_channels.balance_observations WHERE channel_id IN (4,5) ORDER BY channel_id")
        .fetch_all(&role.pool).await.unwrap();
    assert_eq!(nullable_copies, vec![(4, true), (5, true)]);
    let reset: bool = sqlx::query_scalar("SELECT balance=0 AND observed_at='2020-01-01 UTC'::timestamptz AND kind='reset' FROM core_channels.balance_observations WHERE channel_id=1")
        .fetch_one(&role.pool).await.unwrap();
    assert!(reset);
    for statement in [
        "UPDATE core_channels.identities SET id=5 WHERE id=2",
        "UPDATE core_channels.identities SET origin='copy'",
        "UPDATE core_channels.identities SET history_ambiguous=true",
        "UPDATE core_channels.identities SET created_at=clock_timestamp()",
        "UPDATE core_channels.counter_baselines SET gross_credits=1",
        "UPDATE core_channels.counter_baselines SET historical_coverage=true",
        "UPDATE core_channels.balance_observations SET channel_id=2 WHERE channel_id=1",
        "DELETE FROM core_channels.identities",
        "DELETE FROM core_channels.counter_baselines",
        "DELETE FROM core_channels.balance_observations",
        "TRUNCATE core_channels.balance_observations",
        "SELECT * FROM core_meta.channel_accounting_upgrades",
        "UPDATE core_billing.ledger_journals SET created_at=clock_timestamp()",
    ] {
        let error = sqlx::raw_sql(statement)
            .execute(&role.pool)
            .await
            .unwrap_err();
        assert_eq!(
            error.as_database_error().unwrap().code().as_deref(),
            Some("42501"),
            "{statement}: {error}"
        );
    }
    let mut c = role.pool.acquire().await.unwrap();
    accounting::check(&mut c, true).await.unwrap();
    drop(c);
    sqlx::raw_sql(&format!(
        "REVOKE EXECUTE ON FUNCTION public.routing_control_balance_lock() FROM {}",
        role.name
    ))
    .execute(&pool)
    .await
    .unwrap();
    let mut c = role.pool.acquire().await.unwrap();
    assert_eq!(
        accounting::check(&mut c, true).await.unwrap_err(),
        RuntimeDatabaseError::MissingPrivileges
    );
    drop(c);
    sqlx::raw_sql(&format!(
        "GRANT EXECUTE ON FUNCTION public.routing_control_balance_lock() TO {} WITH GRANT OPTION",
        role.name
    ))
    .execute(&pool)
    .await
    .unwrap();
    let mut c = role.pool.acquire().await.unwrap();
    assert_eq!(
        accounting::check(&mut c, false).await.unwrap_err(),
        RuntimeDatabaseError::UnsafePrivileges
    );
    drop(c);
    sqlx::raw_sql(&format!(
        "REVOKE GRANT OPTION FOR EXECUTE ON FUNCTION public.routing_control_balance_lock() FROM {}",
        role.name
    ))
    .execute(&pool)
    .await
    .unwrap();
    sqlx::raw_sql("GRANT EXECUTE ON FUNCTION public.routing_control_balance_lock() TO PUBLIC")
        .execute(&pool)
        .await
        .unwrap();
    let mut c = role.pool.acquire().await.unwrap();
    assert_eq!(
        accounting::check(&mut c, false).await.unwrap_err(),
        RuntimeDatabaseError::UnsafePrivileges
    );
    drop(c);
    sqlx::raw_sql("REVOKE EXECUTE ON FUNCTION public.routing_control_balance_lock() FROM PUBLIC")
        .execute(&pool)
        .await
        .unwrap();
    // Every broad grant is rejected even when model routing is disabled.
    for grant in [
        "UPDATE ON core_channels.identities",
        "UPDATE(gross_credits) ON core_channels.counter_baselines",
        "DELETE ON core_channels.balance_observations",
        "SELECT ON core_meta.channel_accounting_upgrades",
        "UPDATE(channel_id) ON core_channels.balance_observations",
    ] {
        sqlx::raw_sql(&format!("GRANT {grant} TO {}", role.name))
            .execute(&pool)
            .await
            .unwrap();
        let mut c = role.pool.acquire().await.unwrap();
        assert_eq!(
            accounting::check(&mut c, false).await.unwrap_err(),
            RuntimeDatabaseError::UnsafePrivileges
        );
        drop(c);
        sqlx::raw_sql(&format!("REVOKE {grant} FROM {}", role.name))
            .execute(&pool)
            .await
            .unwrap();
    }
    sqlx::raw_sql("GRANT SELECT ON core_meta.channel_accounting_upgrades TO PUBLIC")
        .execute(&pool)
        .await
        .unwrap();
    let mut c = role.pool.acquire().await.unwrap();
    assert_eq!(
        accounting::check(&mut c, false).await.unwrap_err(),
        RuntimeDatabaseError::UnsafePrivileges
    );
    drop(c);
    sqlx::raw_sql("REVOKE SELECT ON core_meta.channel_accounting_upgrades FROM PUBLIC")
        .execute(&pool)
        .await
        .unwrap();
    let inherited = format!("{}_extra", role.name);
    sqlx::raw_sql(&format!("CREATE ROLE {inherited} NOLOGIN; GRANT UPDATE(gross_credits) ON core_channels.counter_baselines TO {inherited}; GRANT {inherited} TO {}",role.name)).execute(&pool).await.unwrap();
    let mut c = role.pool.acquire().await.unwrap();
    assert_eq!(
        accounting::check(&mut c, false).await.unwrap_err(),
        RuntimeDatabaseError::UnsafePrivileges
    );
    drop(c);
    sqlx::raw_sql(&format!("DROP OWNED BY {inherited}; DROP ROLE {inherited}"))
        .execute(&pool)
        .await
        .unwrap();
    sqlx::raw_sql(&format!(
        "REVOKE INSERT ON core_channels.counter_baselines FROM {}",
        role.name
    ))
    .execute(&pool)
    .await
    .unwrap();
    let mut c = role.pool.acquire().await.unwrap();
    assert_eq!(
        accounting::check(&mut c, true).await.unwrap_err(),
        RuntimeDatabaseError::MissingPrivileges
    );
    accounting::check(&mut c, false).await.unwrap();
    drop(c);
    core_schema::check(&pool).await.unwrap();
    role.cleanup().await;
}

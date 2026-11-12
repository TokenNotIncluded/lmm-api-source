//! Component44 derived rows, startup authority, and opening immutability.
use lmm_core::{core_schema, runtime_database::RuntimeDatabaseError};
use sqlx::PgPool;
#[path = "../src/runtime_database/channel_projection.rs"]
mod projection;
#[path = "common/runtime.rs"]
mod runtime;
#[sqlx::test(migrations = false)]
async fn projection_and_opening_runtime_authority(pool: PgPool) {
    core_schema::init(&pool).await.unwrap();
    let ready: bool =
        sqlx::query_scalar("SELECT ready FROM core_channels.projection_state WHERE singleton")
            .fetch_one(&pool)
            .await
            .unwrap();
    assert!(
        ready,
        "fresh empty installation is ready without a startup write"
    );
    assert_eq!(core_schema::rebuild_channel_usage(&pool).await.unwrap(), 0);
    let role = runtime::RuntimeRole::create(&pool).await;
    let mut c = role.pool.acquire().await.unwrap();
    projection::check(&mut c, true).await.unwrap();
    drop(c);
    assert!(matches!(
        core_schema::rebuild_channel_usage(&role.pool).await,
        Err(lmm_core::identity::IdentityError::Forbidden)
    ));
    sqlx::raw_sql("INSERT INTO core_channels.usage_rollups(channel_id,day,bucket,gross_credits,refunded_credits,settled_requests) VALUES(0,-1,0,99999999999999999999999999999999999999,1,2),(9007199254740991,0,63,1,0,1); UPDATE core_channels.usage_rollups SET pending_requests=1 WHERE channel_id=0; DELETE FROM core_channels.usage_rollups")
        .execute(&role.pool).await.unwrap();
    for values in [
        "(-1,-1,0,0,0)",
        "(1,-2,0,0,0)",
        "(1,-1,64,0,0)",
        "(1,-1,0,-1,0)",
        "(1,-1,0,0,1)",
        "(1,-1,0,'NaN',0)",
    ] {
        let error=sqlx::raw_sql(&format!("INSERT INTO core_channels.usage_rollups(channel_id,day,bucket,gross_credits,refunded_credits) VALUES{values}"))
            .execute(&role.pool).await.unwrap_err();
        assert_eq!(
            error.as_database_error().unwrap().code().as_deref(),
            Some("23514"),
            "{values}: {error}"
        );
    }
    for statement in [
        "UPDATE core_channels.projection_state SET ready=false",
        "INSERT INTO core_channels.projection_state VALUES(false,false)",
        "DELETE FROM core_channels.projection_state",
        "TRUNCATE core_channels.usage_rollups",
        "INSERT INTO core_channels.projection_dirty VALUES('unknown',1)",
        "UPDATE core_channels.projection_dirty SET authorized_at=2",
        "INSERT INTO core_channels.opening_sources(source_key) VALUES('x')",
        "UPDATE core_channels.opening_sources SET observed_at=1",
        "DELETE FROM core_channels.opening_sources",
        "UPDATE core_channels.opening_baselines SET raw_used_quota=1",
        "DELETE FROM core_channels.opening_baselines",
        "TRUNCATE core_channels.opening_baselines",
        "SELECT * FROM core_meta.channel_projection_upgrades",
        "SELECT core_channels.mark_projection_dirty()",
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
    for grant in [
        "UPDATE(ready) ON core_channels.projection_state",
        "INSERT(request_id) ON core_channels.projection_dirty",
        "INSERT(source_key) ON core_channels.opening_sources",
        "UPDATE(raw_used_quota) ON core_channels.opening_baselines",
        "SELECT ON core_meta.channel_projection_upgrades",
        "TRUNCATE ON core_channels.usage_contributions",
        "EXECUTE ON FUNCTION core_channels.mark_projection_dirty()",
        "EXECUTE ON FUNCTION core_channels.check_opening_copy()",
    ] {
        sqlx::raw_sql(&format!("GRANT {grant} TO {}", role.name))
            .execute(&pool)
            .await
            .unwrap();
        let mut c = role.pool.acquire().await.unwrap();
        assert_eq!(
            projection::check(&mut c, false).await.unwrap_err(),
            RuntimeDatabaseError::UnsafePrivileges,
            "{grant}"
        );
        drop(c);
        sqlx::raw_sql(&format!("REVOKE {grant} FROM {}", role.name))
            .execute(&pool)
            .await
            .unwrap();
    }
    let mut c = role.pool.acquire().await.unwrap();
    projection::check(&mut c, true).await.unwrap();
    drop(c);
    role.cleanup().await;
}

//! Real PostgreSQL lock ordering for marked-user resource admission.
use lmm_core::{core_schema, identity::IdentityStore};
use sqlx::PgPool;
#[path = "common/abuse_resources.rs"]
mod common;

#[sqlx::test(migrations = false)]
async fn older_serializable_snapshot_cannot_admit_after_real_mark_commits(pool: PgPool) {
    core_schema::init(&pool).await.unwrap();
    let identity = IdentityStore::from_pool(pool.clone());
    let user = identity.bootstrap_user(11, 1).await.unwrap();
    let marker = common::marker(&pool, &identity).await;
    let commit = marker.preview(11, "resource-older-snapshot").await;
    let mut older = pool.begin().await.unwrap();
    sqlx::query("SET TRANSACTION ISOLATION LEVEL SERIALIZABLE")
        .execute(&mut *older)
        .await
        .unwrap();
    let active: bool = sqlx::query_scalar("SELECT active FROM core_identity.users WHERE id=11")
        .fetch_one(&mut *older)
        .await
        .unwrap();
    assert!(active);
    let mark = marker
        .abuse
        .commit_users(&marker.root_secret, &commit)
        .await
        .unwrap();
    assert_eq!(mark["users"][0]["marked"], true);
    // Mark must physically update the canonical user tuple without changing
    // auth_version. A lock alone lets a SERIALIZABLE older snapshot miss a new
    // mark after waiting, even though it acquires SHARE after the commit.
    let result = sqlx::query("SELECT id FROM core_identity.users WHERE id=11 FOR SHARE")
        .fetch_one(&mut *older)
        .await;
    let error = result.expect_err(
        "mark left an older serializable snapshot able to acquire new resource authority",
    );
    assert_eq!(
        error.as_database_error().and_then(|e| e.code()).as_deref(),
        Some("40001")
    );
    older.rollback().await.unwrap();
    let live = identity.authorize(&user.secret).await.unwrap();
    assert_eq!(live.user_id, 11);
    let version: i64 =
        sqlx::query_scalar("SELECT auth_version FROM core_identity.users WHERE id=11")
            .fetch_one(&pool)
            .await
            .unwrap();
    assert_eq!(version, 1);
}

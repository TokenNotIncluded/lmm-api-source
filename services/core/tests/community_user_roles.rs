//! Live identity fields for private analytics consumers; never source snapshots.
use lmm_core::{
    community_read::{CommunityRead, UsersInfoRequest},
    core_schema,
    identity::IdentityStore,
};
use sqlx::PgPool;
#[sqlx::test(migrations = false)]
async fn batch_users_preserve_live_platform_role_and_active_state(pool: PgPool) {
    core_schema::init(&pool).await.unwrap();
    let identity = IdentityStore::from_pool(pool.clone());
    for (id, level) in [(1, 0), (2, 5), (3, 6)] {
        identity.bootstrap_user(id, level).await.unwrap();
    }
    let read = CommunityRead::new(pool.clone()).await.unwrap();
    let request = UsersInfoRequest {
        user_ids: vec![1, 2, 3, 999],
    };
    let rows = read.users(&request).await.unwrap();
    assert_eq!(rows.len(), 3);
    assert_eq!(
        rows.iter()
            .map(|u| u.platform_role.as_str())
            .collect::<Vec<_>>(),
        vec!["user", "admin", "superadmin"]
    );
    sqlx::query("UPDATE core_identity.users SET active=false WHERE id=1")
        .execute(&pool)
        .await
        .unwrap();
    let rows = read.users(&request).await.unwrap();
    assert!(!rows[0].active);
    assert_eq!(rows[0].platform_role, "user");
    assert_eq!(
        serde_json::to_value(&rows[1]).unwrap()["platform_role"],
        "admin"
    );
}

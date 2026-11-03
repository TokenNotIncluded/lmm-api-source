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

#[sqlx::test(migrations = false)]
async fn go_consumer_reads_real_private_core_router(pool: PgPool) {
    use std::os::unix::fs::DirBuilderExt;
    core_schema::init(&pool).await.unwrap();
    let identity = IdentityStore::from_pool(pool.clone());
    for (id, level) in [(1, 0), (2, 5), (3, 6)] {
        identity.bootstrap_user(id, level).await.unwrap();
    }
    sqlx::query("UPDATE core_identity.users SET active=false WHERE id=1")
        .execute(&pool)
        .await
        .unwrap();
    let read = CommunityRead::new(pool).await.unwrap();
    let dir = std::env::temp_dir().join(format!("lmm-core-go-roles-{}", std::process::id()));
    std::fs::DirBuilder::new().mode(0o700).create(&dir).unwrap();
    let socket = dir.join("core.sock");
    let listener = tokio::net::UnixListener::bind(&socket).unwrap();
    let router =
        lmm_core::community_read::http::router(read, b"synthetic-community-purpose-token-20261104")
            .unwrap();
    let server = tokio::spawn(async move {
        axum::serve(listener, router).await.unwrap();
    });
    let output = tokio::task::spawn_blocking(move || {
        std::process::Command::new("go")
            .args([
                "test",
                "./internal/modules/community",
                "-run",
                "^TestCoreUserRolesLive$",
                "-count=1",
            ])
            .current_dir(std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../extensions"))
            .env("LMM_TEST_CORE_USER_SOCKET", socket)
            .output()
    })
    .await
    .unwrap();
    server.abort();
    let _ = server.await;
    std::fs::remove_dir_all(dir).unwrap();
    let output = output.unwrap();
    assert!(
        output.status.success(),
        "Go live consumer failed: {}{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
}

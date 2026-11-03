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
    assert!(rows.iter().all(|u| u.registered_at.is_some_and(|t| t > 0)));
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
    sqlx::query("UPDATE core_identity.accounts SET created_at=to_timestamp(0) WHERE id=(SELECT personal_account_id FROM core_identity.users WHERE id=2)").execute(&pool).await.unwrap();
    let rows = read.users(&request).await.unwrap();
    assert!(rows[0].registered_at.is_some());
    assert!(rows[1].registered_at.is_none());
    sqlx::query("INSERT INTO core_migration.import_runs(migration_id,source_archive_sha256,mapping_sha256,manifest) VALUES('old-time-fixture',repeat('a',64),repeat('b',64),'{}')").execute(&pool).await.unwrap();
    let rows = read.users(&request).await.unwrap();
    assert!(rows.iter().all(|u| u.registered_at.is_none()));
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

#[sqlx::test(migrations = false)]
async fn registration_proof_is_account_scoped_and_ignores_wallet_history(pool: PgPool) {
    use lmm_core::community_read::UserPageRequest;
    core_schema::init(&pool).await.unwrap();
    let identity = IdentityStore::from_pool(pool.clone());
    identity.bootstrap_user(71, 0).await.unwrap();
    let account: i64 =
        sqlx::query_scalar("SELECT personal_account_id FROM core_identity.users WHERE id=71")
            .fetch_one(&pool)
            .await
            .unwrap();
    let read = CommunityRead::new(pool.clone()).await.unwrap();
    let request = UsersInfoRequest { user_ids: vec![71] };
    let registered = read.users(&request).await.unwrap()[0]
        .registered_at
        .unwrap();
    sqlx::query("INSERT INTO core_migration.import_runs(migration_id,source_archive_sha256,mapping_sha256,manifest) VALUES('wallet-history',repeat('a',64),repeat('b',64),'{\"scope\":\"wallet-topup-history.v1\"}')")
        .execute(&pool).await.unwrap();
    assert_eq!(
        read.users(&request).await.unwrap()[0].registered_at,
        Some(registered),
        "a financial archive must not invalidate native registration time"
    );
    let page = UserPageRequest {
        after_id: 0,
        limit: 100,
    };
    assert_eq!(
        read.user_page(&page).await.unwrap().users[0].registered_at,
        Some(registered)
    );
    let opening = serde_json::json!({"migration_id":"opening-proof", "source_entity_key":"user:71",
        "source_archive_sha256":"a".repeat(64), "mapping_sha256":"b".repeat(64),
        "source_row_sha256":"c".repeat(64), "observed_at":1793674367_i64,
        "account_id":account,"available_units":0,"reserved_units":0,"affiliate_units":0});
    let mut tx = pool.begin().await.unwrap();
    sqlx::query("SET TRANSACTION ISOLATION LEVEL SERIALIZABLE")
        .execute(&mut *tx)
        .await
        .unwrap();
    sqlx::query("SET LOCAL synchronous_commit=on")
        .execute(&mut *tx)
        .await
        .unwrap();
    sqlx::query("SELECT core_billing.import_opening_balance($1)")
        .bind(opening)
        .execute(&mut *tx)
        .await
        .unwrap();
    tx.commit().await.unwrap();
    assert!(
        read.users(&request).await.unwrap()[0]
            .registered_at
            .is_none(),
        "legacy openings without registration provenance must remain unknown"
    );
    let mut times = serde_json::Map::new();
    times.insert(account.to_string(), serde_json::json!(registered));
    let manifest =
        serde_json::json!({"scope":"legacy-cutover-identity.v1", "registration_times":times});
    sqlx::query("INSERT INTO core_migration.import_runs(migration_id,source_archive_sha256,mapping_sha256,manifest) VALUES('known-times',repeat('a',64),repeat('b',64),$1)")
        .bind(manifest).execute(&pool).await.unwrap();
    assert_eq!(
        read.users(&request).await.unwrap()[0].registered_at,
        Some(registered)
    );
    assert_eq!(
        read.user_page(&page).await.unwrap().users[0].registered_at,
        Some(registered)
    );
    sqlx::query("UPDATE core_identity.accounts SET created_at=to_timestamp($1) WHERE id=$2")
        .bind(registered + 1)
        .bind(account)
        .execute(&pool)
        .await
        .unwrap();
    assert!(
        read.users(&request).await.unwrap()[0]
            .registered_at
            .is_none(),
        "a receipt for another timestamp cannot establish provenance"
    );
}

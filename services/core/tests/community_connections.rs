use lmm_core::{
    community_read::{CommunityRead, UsersInfoRequest},
    core_schema,
    identity::IdentityStore,
};
use sqlx::PgPool;

#[sqlx::test(migrations = false)]
async fn creation_facts_survive_revocation_and_do_not_guess_imports(pool: PgPool) {
    core_schema::init(&pool).await.unwrap();
    let identity = IdentityStore::from_pool(pool.clone());
    for id in [11, 12, 13, 14] {
        identity.bootstrap_user(id, 0).await.unwrap();
    }
    sqlx::query("INSERT INTO core_identity.credentials(digest,kind,user_id,user_version,owner_account_id,expires_at,revoked_at,created_at,session_started_at) SELECT decode(lpad(to_hex(id),64,'0'),'hex'),'api_key',id,auth_version,personal_account_id,now()+interval '1 day',now(),now()-interval '2 days'+interval '100 seconds',now()-interval '2 days'+interval '100 seconds' FROM core_identity.users WHERE id IN (11,14)").execute(&pool).await.unwrap();
    // Browser grants include revoked authorizations. Foundation is not issued.
    sqlx::query("INSERT INTO core_oauth.families(id,user_id,owner_account_id,user_version,issuer,resource,client_id,issuance_kind,purpose,scopes,created_at,expires_at,revoked_at) SELECT 'lmm_gr_'||lpad(to_hex(id),64,'0'),id,personal_account_id,auth_version,'https://synthetic.example','https://synthetic.example/api/oauth2','lmm','browser','read_only',ARRAY['catalog:read','group:ZGVmYXVsdA'],now()-interval '2 days'+interval '200 seconds',now()+interval '1 day',now() FROM core_identity.users WHERE id IN (12,14)").execute(&pool).await.unwrap();
    sqlx::query("UPDATE core_identity.accounts SET created_at=to_timestamp(floor(extract(epoch FROM now()-interval '2 days'))) WHERE id IN (SELECT personal_account_id FROM core_identity.users WHERE id IN (11,12,13,14))").execute(&pool).await.unwrap();
    let read = CommunityRead::new(pool.clone()).await.unwrap();
    let request = UsersInfoRequest {
        user_ids: vec![11, 12, 999],
    };
    let rows = read.connection_milestones(&request).await.unwrap().users;
    assert_eq!(rows.len(), 2);
    assert!(
        rows[0].history_complete
            && rows[0].api_key_created_at.is_some()
            && rows[0].oauth_authorized_at.is_none()
    );
    assert!(
        rows[1].history_complete
            && rows[1].api_key_created_at.is_none()
            && rows[1].oauth_authorized_at.is_some()
    );
    let socket_dir = std::env::temp_dir().join(format!("lmm-connection-{}", std::process::id()));
    std::fs::create_dir(&socket_dir).unwrap();
    let socket = socket_dir.join("core.sock");
    let listener = tokio::net::UnixListener::bind(&socket).unwrap();
    let router = lmm_core::community_read::http::router(
        read.clone(),
        b"synthetic-community-purpose-token-20261104",
    )
    .unwrap();
    let server = tokio::spawn(async move {
        axum::serve(listener, router).await.unwrap();
    });
    let output = tokio::task::spawn_blocking(move || {
        std::process::Command::new("go")
            .args([
                "test",
                "./internal/modules/acquisition",
                "-run",
                "^TestConnectionFunnelLive$",
                "-v",
                "-count=1",
            ])
            .current_dir(concat!(env!("CARGO_MANIFEST_DIR"), "/../extensions"))
            .env("LMM_TEST_CONNECTION_SOCKET", socket)
            .output()
            .unwrap()
    })
    .await
    .unwrap();
    server.abort();
    let _ = server.await;
    std::fs::remove_dir_all(socket_dir).unwrap();
    assert!(
        output.status.success(),
        "{}{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(String::from_utf8_lossy(&output.stdout).contains("--- PASS: TestConnectionFunnelLive"));
    sqlx::query("INSERT INTO core_migration.import_runs(migration_id,source_archive_sha256,mapping_sha256,manifest) VALUES('connection-fixture',repeat('a',64),repeat('b',64),jsonb_build_object('scope','legacy-cutover-identity.v1','registration_times',jsonb_build_object((SELECT personal_account_id::text FROM core_identity.users WHERE id=11),1700000000)))").execute(&pool).await.unwrap();
    let rows = read.connection_milestones(&request).await.unwrap().users;
    assert!(!rows[0].history_complete);
    assert!(rows[0].api_key_created_at.is_none());
    assert!(rows[1].history_complete);
    assert!(
        read.connection_milestones(&UsersInfoRequest {
            user_ids: vec![1; 101]
        })
        .await
        .is_err()
    );
    assert!(
        read.connection_milestones(&UsersInfoRequest { user_ids: vec![0] })
            .await
            .is_err()
    );
}

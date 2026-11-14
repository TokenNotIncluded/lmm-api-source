//! Synthetic PostgreSQL/Router regression; execution requires an isolated SQLx database.
use super::{router, tests, *};
use crate::{
    accounts::{Account, AccountKind},
    funding::FundingPolicy,
    identity::{self, IdentityStore, permissions},
    test_runtime::RuntimeRole,
};
use sha2::{Digest, Sha256};

#[path = "authorization_test_helpers.rs"]
mod helpers;

use helpers::{check_reads, imported, policy, state};
const CONTRACT: &str = permissions::CONTRACT_VERSION;

#[sqlx::test(migrations = false)]
async fn channel_read_uses_current_policy_and_preserves_read_only_state(pool: PgPool) {
    crate::core_schema::init(&pool).await.unwrap();
    let identity = IdentityStore::from_pool(pool.clone());
    let allowed = identity.bootstrap_user(101, 5).await.unwrap();
    let denied = identity.bootstrap_user(102, 5).await.unwrap();
    let root = identity.bootstrap_user(103, 6).await.unwrap();
    let user = identity.bootstrap_user(104, 1).await.unwrap();
    let revoked = identity.bootstrap_user(105, 5).await.unwrap();
    let changed = identity.bootstrap_user(106, 5).await.unwrap();
    let missing = identity.bootstrap_user(107, 5).await.unwrap();
    let incomplete = identity.bootstrap_user(108, 5).await.unwrap();
    let bad_hash = identity.bootstrap_user(109, 5).await.unwrap();
    let inactive = identity.bootstrap_user(110, 5).await.unwrap();
    let stale = identity.bootstrap_user(111, 5).await.unwrap();
    let root_missing = identity.bootstrap_user(112, 6).await.unwrap();
    let root_bad_hash = identity.bootstrap_user(113, 6).await.unwrap();
    identity.bootstrap_user(114, 5).await.unwrap();
    let bare = "lmms_synthetic_session_without_assurance_114";
    let bare_digest = Sha256::digest(bare.as_bytes()).to_vec();
    sqlx::query("INSERT INTO core_identity.credentials(digest,kind,user_id,user_version,owner_account_id,expires_at) SELECT $1,'session',id,auth_version,personal_account_id,clock_timestamp()+interval '1 hour' FROM core_identity.users WHERE id=114")
        .bind(bare_digest).execute(&pool).await.unwrap();
    let key = identity
        .issue_key(
            &allowed.secret,
            Account {
                kind: AccountKind::Personal,
                id: 101,
            },
            FundingPolicy::new_personal_key(),
            3600,
        )
        .await
        .unwrap();

    for id in [101, 102, 107, 108, 109] {
        imported(&pool, id, "admin").await;
    }
    imported(&pool, 112, "superadmin").await;
    imported(&pool, 113, "superadmin").await;
    policy(&pool, 101, "allow", true, true).await;
    policy(&pool, 102, "deny", true, true).await;
    policy(&pool, 103, "deny", true, true).await;
    policy(&pool, 108, "allow", false, true).await;
    policy(&pool, 109, "allow", true, false).await;
    policy(&pool, 113, "allow", true, false).await;
    sqlx::query("UPDATE core_identity.credentials SET revoked_at=clock_timestamp() WHERE id=$1")
        .bind(revoked.id)
        .execute(&pool)
        .await
        .unwrap();
    sqlx::query("UPDATE core_identity.accounts SET active=FALSE WHERE id=(SELECT personal_account_id FROM core_identity.users WHERE id=110)").execute(&pool).await.unwrap();
    sqlx::query("UPDATE core_identity.users SET auth_version=auth_version+1 WHERE id=111")
        .execute(&pool)
        .await
        .unwrap();

    let role = RuntimeRole::create(&pool).await;
    let (session, current, superuser): (String, String, bool) = sqlx::query_as(
        "SELECT session_user::text,current_user::text,rolsuper FROM pg_roles WHERE rolname=current_user",
    )
    .fetch_one(&role.pool)
    .await
    .unwrap();
    assert_eq!(session, role.name);
    assert_eq!(current, role.name);
    assert!(!superuser);
    let (config, prices) = tests::fixture();
    let app = router(role.pool.clone(), tests::serving(config, prices));
    let before = state(&pool).await;
    check_reads(&app, &allowed.secret, StatusCode::OK).await;
    check_reads(&app, &denied.secret, StatusCode::FORBIDDEN).await;
    check_reads(&app, &root.secret, StatusCode::OK).await;
    check_reads(&app, &user.secret, StatusCode::FORBIDDEN).await;
    check_reads(&app, &key.secret, StatusCode::FORBIDDEN).await;
    check_reads(&app, &revoked.secret, StatusCode::UNAUTHORIZED).await;
    check_reads(&app, &inactive.secret, StatusCode::UNAUTHORIZED).await;
    check_reads(&app, &stale.secret, StatusCode::UNAUTHORIZED).await;
    check_reads(&app, bare, StatusCode::UNAUTHORIZED).await;
    check_reads(&app, &missing.secret, StatusCode::FORBIDDEN).await;
    check_reads(&app, &incomplete.secret, StatusCode::FORBIDDEN).await;
    check_reads(&app, &bad_hash.secret, StatusCode::FORBIDDEN).await;
    check_reads(&app, &root_missing.secret, StatusCode::FORBIDDEN).await;
    check_reads(&app, &root_bad_hash.secret, StatusCode::FORBIDDEN).await;
    check_reads(&app, &changed.secret, StatusCode::OK).await;
    // AdminUsersRead remains allowed while ChannelRead is explicitly denied.
    let mut tx = role.pool.begin().await.unwrap();
    sqlx::query("SET TRANSACTION ISOLATION LEVEL REPEATABLE READ READ ONLY")
        .execute(&mut *tx)
        .await
        .unwrap();
    assert!(
        identity::authorize_admin_read_snapshot(&mut tx, &denied.secret)
            .await
            .is_ok()
    );
    tx.commit().await.unwrap();
    assert_eq!(
        before,
        state(&pool).await,
        "channel reads changed database rows or sequences"
    );

    // A native admin begins with the existing default; the first installed
    // immutable policy denies it on the next request, without changing Session.
    policy(&pool, 106, "deny", true, true).await;
    policy(&pool, 107, "allow", true, true).await;
    let before = state(&pool).await;
    check_reads(&app, &changed.secret, StatusCode::FORBIDDEN).await;
    check_reads(&app, &missing.secret, StatusCode::OK).await;
    for (path, method) in [
        ("/api/channel", "POST"),
        ("/api/channel/", "PUT"),
        ("/api/channel/1", "DELETE"),
    ] {
        let response = tests::request(app.clone(), path, Some(&allowed.secret), method).await;
        assert_eq!(
            response.status(),
            StatusCode::NOT_IMPLEMENTED,
            "{method} {path}"
        );
    }
    assert_eq!(
        before,
        state(&pool).await,
        "channel operations changed database rows or sequences"
    );
    drop(app);
    role.cleanup().await;
}

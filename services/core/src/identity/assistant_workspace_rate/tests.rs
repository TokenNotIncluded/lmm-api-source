use super::*;
use crate::identity::{BrowserSessionMetadata, IdentityStore, LoginRequest};
use serde_json::Value;
use sqlx::postgres::PgPoolOptions;

#[test]
fn workspace_bucket_is_actor_scoped_and_separate_from_login_buckets() {
    assert!(bucket(0).is_err());
    assert!(bucket(-1).is_err());
    assert_ne!(bucket(1).unwrap(), bucket(2).unwrap());
    for scope in ["password", "oauth2_native_peer", "assistant-workspace"] {
        for key in ["1", "user:1", "same-user-login"] {
            let login: [u8; 32] = Sha256::digest(format!("{scope}\0{key}").as_bytes()).into();
            assert_ne!(bucket(1).unwrap(), login);
        }
    }
}

async fn attempts(pool: &PgPool, user: i64) -> i32 {
    sqlx::query_scalar("SELECT attempts FROM core_identity.auth_attempts WHERE bucket=$1")
        .bind(bucket(user).unwrap().to_vec())
        .fetch_one(pool)
        .await
        .unwrap()
}

async fn other_buckets(pool: &PgPool, user: i64, other: i64) -> Value {
    sqlx::query_scalar(
        "SELECT coalesce(jsonb_agg(to_jsonb(a) ORDER BY bucket),'[]'::jsonb)
         FROM core_identity.auth_attempts a WHERE bucket<>$1 AND bucket<>$2",
    )
    .bind(bucket(user).unwrap().to_vec())
    .bind(bucket(other).unwrap().to_vec())
    .fetch_one(pool)
    .await
    .unwrap()
}

async fn session_rows(pool: &PgPool) -> Value {
    sqlx::query_scalar(
        "SELECT jsonb_build_object(
         'credentials',(SELECT jsonb_agg(to_jsonb(c) ORDER BY id) FROM core_identity.credentials c),
         'sessions',(SELECT jsonb_agg(to_jsonb(s) ORDER BY sid) FROM core_identity.web_sessions s))",
    )
    .fetch_one(pool)
    .await
    .unwrap()
}

#[sqlx::test(migrations = false)]
async fn workspace_rate_shares_sessions_counts_failures_and_persists_window(pool: PgPool) {
    let identity = IdentityStore::from_pool(pool.clone()).with_registration_enabled(true);
    identity.init_database().await.unwrap();
    let browser = BrowserAuth::new(identity.clone(), &[46; 32]).unwrap();
    let first = LoginRequest {
        login_name: "workspace-rate-first".into(),
        password: "workspace-rate-123".into(),
    };
    let second = LoginRequest {
        login_name: "workspace-rate-second".into(),
        password: "workspace-rate-456".into(),
    };
    let original = identity.register(&first).await.unwrap();
    let original_other = identity.register(&second).await.unwrap();
    let user = identity.authorize(&original.secret).await.unwrap().user_id;
    let other = identity
        .authorize(&original_other.secret)
        .await
        .unwrap()
        .user_id;
    let one = browser
        .login(
            &first.login_name,
            &first.password,
            &BrowserSessionMetadata::default(),
        )
        .await
        .unwrap();
    let two = browser
        .login(
            &first.login_name,
            &first.password,
            &BrowserSessionMetadata::default(),
        )
        .await
        .unwrap();
    let foreign = browser
        .login(
            &second.login_name,
            &second.password,
            &BrowserSessionMetadata::default(),
        )
        .await
        .unwrap();
    let preserved_buckets = other_buckets(&pool, user, other).await;
    let preserved_sessions = session_rows(&pool).await;
    assert!(take(&pool, &one.refresh_token).await.unwrap().allowed);
    // The following domain transaction fails and rolls back. The independently
    // committed attempt remains; an action error cannot reset the shared budget.
    let mut failed_action = pool.begin().await.unwrap();
    assert!(
        sqlx::query("SELECT 1/0")
            .execute(&mut *failed_action)
            .await
            .is_err()
    );
    failed_action.rollback().await.unwrap();
    assert_eq!(attempts(&pool, user).await, 1);
    for _ in 1..119 {
        assert!(take(&pool, &one.refresh_token).await.unwrap().allowed);
    }
    assert_eq!(attempts(&pool, user).await, 119);
    let (left, right) = tokio::join!(
        take(&pool, &one.refresh_token),
        take(&pool, &two.refresh_token),
    );
    let left = left.unwrap();
    let right = right.unwrap();
    assert_ne!(left.allowed, right.allowed); // Exactly request 120 succeeds.
    assert!((1..=300).contains(&left.retry_after_seconds));
    assert!((1..=300).contains(&right.retry_after_seconds));
    assert_eq!(attempts(&pool, user).await, 121);
    assert!(take(&pool, &foreign.refresh_token).await.unwrap().allowed);
    assert_eq!(attempts(&pool, other).await, 1);
    // A separate pool/service instance has no in-memory reset. The independent
    // SQL/PG driver separately records any actual database restart observation.
    let reopened = PgPoolOptions::new()
        .max_connections(2)
        .connect_with(pool.connect_options().as_ref().clone())
        .await
        .unwrap();
    assert!(!take(&reopened, &one.refresh_token).await.unwrap().allowed);
    reopened.close().await;
    assert_eq!(attempts(&pool, user).await, 121);
    assert!(take(&pool, "invalid-cookie").await.is_err());
    assert_eq!(attempts(&pool, user).await, 121);
    // Only the test owner advances this isolated fixture's old window.
    sqlx::query("UPDATE core_identity.auth_attempts SET started_at=clock_timestamp()-interval '301 seconds' WHERE bucket=$1")
        .bind(bucket(user).unwrap().to_vec())
        .execute(&pool)
        .await
        .unwrap();
    assert!(take(&pool, &two.refresh_token).await.unwrap().allowed);
    assert_eq!(attempts(&pool, user).await, 1);
    assert_eq!(other_buckets(&pool, user, other).await, preserved_buckets);
    assert_eq!(session_rows(&pool).await, preserved_sessions);
    // Revoking an actual browser family blocks the caller before touching the
    // counter, even though that user's other Session remains valid.
    let mut tx = pool.begin().await.unwrap();
    let actor = BrowserAuth::oauth_identity_in_transaction(&mut tx, &one.refresh_token)
        .await
        .unwrap();
    sqlx::query("UPDATE core_identity.web_sessions SET revoked_at=clock_timestamp() WHERE sid=$1")
        .bind(actor.sid)
        .execute(&mut *tx)
        .await
        .unwrap();
    tx.commit().await.unwrap();
    assert!(take(&pool, &one.refresh_token).await.is_err());
    assert_eq!(attempts(&pool, user).await, 1);
    assert!(take(&pool, &two.refresh_token).await.unwrap().allowed);
    assert_eq!(attempts(&pool, user).await, 2);
}

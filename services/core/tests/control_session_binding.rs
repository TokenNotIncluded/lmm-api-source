//! Real PostgreSQL checks for Core-owned interactive Session bindings.
use lmm_core::{
    accounts::{Account, AccountKind},
    funding::FundingPolicy,
    identity::{
        BrowserAuth, BrowserBundle, BrowserRegistration, BrowserSessionMetadata, IdentityError,
        IdentityStore,
    },
};
use sqlx::PgPool;

const PASSWORD: &str = "binding-password";

async fn setup(pool: PgPool) -> (IdentityStore, BrowserAuth) {
    let store = IdentityStore::from_pool(pool.clone())
        .with_registration_enabled(true)
        .with_mfa_keys("test".into(), vec![("test".into(), vec![47; 32])])
        .unwrap();
    store.init_database().await.unwrap();
    sqlx::raw_sql(sqlx::AssertSqlSafe(lmm_core::abuse::SCHEMA))
        .execute(&pool)
        .await
        .unwrap();
    let browser = BrowserAuth::new(store.clone(), &[43; 32]).unwrap();
    (store, browser)
}

fn access(bundle: &BrowserBundle) -> &str {
    bundle.data["access_token"].as_str().unwrap()
}
fn sid(bundle: &BrowserBundle) -> &str {
    bundle.data["session"]["sid"].as_str().unwrap()
}
async fn login(browser: &BrowserAuth) -> BrowserBundle {
    browser
        .login(
            "binding_owner",
            PASSWORD,
            &BrowserSessionMetadata::default(),
        )
        .await
        .unwrap()
}

#[sqlx::test(migrations = false)]
async fn binding_survives_refresh_but_not_other_families_or_authority_changes(pool: PgPool) {
    let (store, browser) = setup(pool.clone()).await;
    browser
        .register(&BrowserRegistration {
            username: "binding_owner",
            password: PASSWORD,
            email: "",
        })
        .await
        .unwrap();
    let first = login(&browser).await;
    let other = login(&browser).await;
    let (actor, initial) = store
        .authorize_with_session_binding(access(&first))
        .await
        .unwrap();
    let (_, other_binding) = store
        .authorize_with_session_binding(access(&other))
        .await
        .unwrap();
    assert!(initial.starts_with("v1:") && initial.len() == 67);
    assert_ne!(initial, other_binding);
    assert!(!initial.contains(sid(&first)));
    assert!(!initial.contains(access(&first)));

    let refreshed = browser
        .refresh(&first.refresh_token, Some(sid(&first)))
        .await
        .unwrap();
    let (renewed_actor, renewed) = store
        .authorize_with_session_binding(access(&refreshed))
        .await
        .unwrap();
    assert_eq!(actor.user_id, renewed_actor.user_id);
    assert_ne!(actor.credential_id, renewed_actor.credential_id);
    assert_ne!(access(&first), access(&refreshed));
    assert_eq!(sid(&first), sid(&refreshed));
    assert_eq!(initial, renewed);
    assert!(matches!(
        store.authorize_with_session_binding(access(&first)).await,
        Err(IdentityError::Unauthorized)
    ));

    let key = store
        .issue_key(
            access(&refreshed),
            Account {
                kind: AccountKind::Personal,
                id: actor.user_id,
            },
            FundingPolicy::new_personal_key(),
            3600,
        )
        .await
        .unwrap();
    assert!(
        store
            .authorize_with_session_binding(&key.secret)
            .await
            .unwrap()
            .1
            .is_empty()
    );

    let expired = login(&browser).await;
    sqlx::query("UPDATE core_identity.web_sessions SET expires_at=clock_timestamp()-interval '1 second' WHERE sid=$1")
        .bind(sid(&expired)).execute(&pool).await.unwrap();
    assert!(matches!(
        store.authorize_with_session_binding(access(&expired)).await,
        Err(IdentityError::Unauthorized)
    ));

    browser
        .logout(Some(&other.refresh_token), None, Some(sid(&other)))
        .await
        .unwrap();
    assert!(matches!(
        store.authorize_with_session_binding(access(&other)).await,
        Err(IdentityError::Unauthorized)
    ));

    let changed = browser
        .change_password(
            access(&refreshed),
            PASSWORD,
            "binding-replaced",
            None,
            Some(sid(&refreshed)),
        )
        .await
        .unwrap();
    assert_eq!(sid(&changed), sid(&refreshed));
    let (_, changed_binding) = store
        .authorize_with_session_binding(access(&changed))
        .await
        .unwrap();
    assert_ne!(changed_binding, initial);
    assert!(matches!(
        store
            .authorize_with_session_binding(access(&refreshed))
            .await,
        Err(IdentityError::Unauthorized)
    ));
    browser
        .logout(None, Some(access(&changed)), Some(sid(&changed)))
        .await
        .unwrap();
    assert!(matches!(
        store.authorize_with_session_binding(access(&changed)).await,
        Err(IdentityError::Unauthorized)
    ));
}

#[sqlx::test(migrations = false)]
async fn native_sessions_remain_credential_bound(pool: PgPool) {
    let (store, _) = setup(pool).await;
    let original = store.bootstrap_user(1, 1).await.unwrap();
    let (_, first) = store
        .authorize_with_session_binding(&original.secret)
        .await
        .unwrap();
    assert_eq!(
        first,
        store
            .authorize_with_session_binding(&original.secret)
            .await
            .unwrap()
            .1
    );
    let replacement = store.refresh_session(&original.secret).await.unwrap();
    let (_, next) = store
        .authorize_with_session_binding(&replacement.secret)
        .await
        .unwrap();
    assert_ne!(first, next);
    assert!(matches!(
        store.authorize_with_session_binding(&original.secret).await,
        Err(IdentityError::Unauthorized)
    ));
}

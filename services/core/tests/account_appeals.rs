//! Suspension review through restricted runtime roles, with real persisted facts.
use lmm_core::identity::{
    self, BrowserAuth, BrowserBundle, BrowserLoginOutcome, BrowserRegistration,
    BrowserSessionMetadata, IdentityStore,
    admin_users::{
        AccountAppealsQuery, AdminUsersStore, AppealReviewAction, AppealReviewRequest,
        ProfileError, SecurityAction, SecurityRequest,
    },
};
use serde_json::Value;
use sqlx::PgPool;
#[path = "common/runtime.rs"]
mod runtime;

const PASSWORD: &str = "appeal-fixture-971";
const PRIVATE: &str = "Private synthetic investigation reference";
const PUBLIC: &str = "Your account was suspended under the synthetic usage rule.";
const NOTE: &str = "Private reviewer evidence, never part of the public receipt";
struct Fixture {
    role: runtime::RuntimeRole,
    browser: BrowserAuth,
    admin: AdminUsersStore,
    admin_secret: String,
    user: i64,
    original: BrowserBundle,
}
async fn fixture(pool: &PgPool, public: bool) -> Fixture {
    lmm_core::core_schema::init(pool).await.unwrap();
    let owner = IdentityStore::from_pool(pool.clone());
    let admin_secret = owner.bootstrap_user(900, 5).await.unwrap().secret;
    let role = runtime::RuntimeRole::create(pool).await;
    role.optional("web_identity").await;
    let store = IdentityStore::from_pool(role.pool.clone()).with_registration_enabled(true);
    let browser = BrowserAuth::new(store.clone(), &[61; 32]).unwrap();
    browser
        .register(&BrowserRegistration {
            username: "appeal-user",
            password: PASSWORD,
            email: "",
        })
        .await
        .unwrap();
    let original = browser
        .login("appeal-user", PASSWORD, &BrowserSessionMetadata::default())
        .await
        .unwrap();
    let user = original.data["user"]["id"].as_i64().unwrap();
    let admin = AdminUsersStore::new(store);
    let before = admin.user_security(&admin_secret, user).await.unwrap();
    admin
        .update_user_security(
            &admin_secret,
            user,
            &SecurityRequest {
                expected_auth_version: before.auth_version,
                action: SecurityAction::Disable,
                reason: PRIVATE.into(),
                public_reason: public.then(|| PUBLIC.into()),
            },
        )
        .await
        .unwrap();
    Fixture {
        role,
        browser,
        admin,
        admin_secret,
        user,
        original,
    }
}
async fn ledger(pool: &PgPool) -> Value {
    sqlx::query_scalar("SELECT COALESCE(jsonb_agg(to_jsonb(e) ORDER BY to_jsonb(e)::text),'[]'::jsonb) FROM core_billing.ledger_entries e")
        .fetch_one(pool).await.unwrap()
}
fn review(revision: &str, version: &str) -> AppealReviewRequest {
    AppealReviewRequest {
        expected_revision: revision.into(),
        expected_auth_version: version.into(),
        note: NOTE.into(),
    }
}

#[sqlx::test(migrations = false)]
async fn restricted_review_restores_only_fresh_login_and_keeps_private_notes_private(pool: PgPool) {
    let f = fixture(&pool, true).await;
    let before_ledger = ledger(&pool).await;
    let original = f.original.data["access_token"].as_str().unwrap();
    let grant = f
        .browser
        .password_account_appeal("appeal-user", PASSWORD)
        .await
        .unwrap();
    assert_eq!(grant.context.public_reason.as_deref(), Some(PUBLIC));
    assert!(grant.context.public_reason_available);
    assert!(
        !serde_json::to_string(&grant.context)
            .unwrap()
            .contains(PRIVATE)
    );
    let store = IdentityStore::from_pool(f.role.pool.clone());
    assert_eq!(
        store.authorize(&grant.token).await.err(),
        Some(identity::IdentityError::Unauthorized)
    );
    let receipt = f
        .browser
        .submit_account_appeal(&grant.token, "Please review my suspension")
        .await
        .unwrap();
    assert_eq!(receipt.status, "pending");
    assert!(!receipt.already_pending);
    let replay = f
        .browser
        .submit_account_appeal(&grant.token, "Please review my suspension")
        .await
        .unwrap();
    assert_eq!(replay.id, receipt.id);
    assert!(replay.already_pending);
    assert_eq!(
        f.browser
            .submit_account_appeal(&grant.token, "Overwrite the original request")
            .await
            .err(),
        Some(identity::IdentityError::Conflict)
    );
    assert!(matches!(
        f.admin
            .account_appeals(&grant.token, &AccountAppealsQuery::default())
            .await,
        Err(ProfileError::Unauthorized)
    ));
    let page = f
        .admin
        .account_appeals(&f.admin_secret, &AccountAppealsQuery::default())
        .await
        .unwrap();
    assert_eq!(page.pagination.total, 1);
    let item = &page.data[0];
    assert!(matches!(
        f.admin
            .review_account_appeal(
                &f.admin_secret,
                item.id,
                AppealReviewAction::Approve,
                &review("2", &item.auth_version)
            )
            .await,
        Err(ProfileError::Conflict)
    ));
    assert!(
        !f.admin
            .user_security(&f.admin_secret, f.user)
            .await
            .unwrap()
            .active
    );
    let input = review(&item.revision, &item.auth_version);
    let approved = f
        .admin
        .review_account_appeal(
            &f.admin_secret,
            item.id,
            AppealReviewAction::Approve,
            &input,
        )
        .await
        .unwrap();
    assert_eq!(approved.status, "approved");
    assert_eq!(approved.revision, "2");
    let replay = f
        .admin
        .review_account_appeal(
            &f.admin_secret,
            item.id,
            AppealReviewAction::Approve,
            &input,
        )
        .await
        .unwrap();
    assert_eq!(replay.id, item.id);
    assert_eq!(replay.revision, "2");
    assert_eq!(
        store.authorize(original).await.err(),
        Some(identity::IdentityError::Unauthorized)
    );
    let context = f
        .browser
        .account_appeal_context(&grant.token)
        .await
        .unwrap();
    assert_eq!(context.request.as_ref().unwrap().status, "approved");
    let public = serde_json::to_string(&context).unwrap();
    assert!(!public.contains(NOTE));
    assert!(!public.contains(PRIVATE));
    let fresh = f
        .browser
        .login("appeal-user", PASSWORD, &BrowserSessionMetadata::default())
        .await
        .unwrap();
    assert!(
        store
            .authorize(fresh.data["access_token"].as_str().unwrap())
            .await
            .is_ok()
    );
    let audits: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM core_identity.audit WHERE action='admin.user.appeal.approve'",
    )
    .fetch_one(&pool)
    .await
    .unwrap();
    assert_eq!(audits, 1);
    assert_eq!(before_ledger, ledger(&pool).await);
    f.role.cleanup().await;
}

#[sqlx::test(migrations = false)]
async fn concurrent_submission_one_pending_and_old_generation_cannot_be_reviewed(pool: PgPool) {
    let f = fixture(&pool, false).await;
    let first = f
        .browser
        .password_account_appeal("appeal-user", PASSWORD)
        .await
        .unwrap();
    let second = f
        .browser
        .password_account_appeal("appeal-user", PASSWORD)
        .await
        .unwrap();
    assert!(!first.context.public_reason_available);
    assert!(first.context.public_reason.is_none());
    let (left, right) = tokio::join!(
        f.browser
            .submit_account_appeal(&first.token, "First evidence for review"),
        f.browser
            .submit_account_appeal(&second.token, "Additional evidence for review")
    );
    let left = left.unwrap();
    let right = right.unwrap();
    assert_eq!(left.id, right.id);
    assert_eq!(left.reason, right.reason);
    assert_ne!(left.already_pending, right.already_pending);
    let pending = f
        .admin
        .account_appeals(&f.admin_secret, &AccountAppealsQuery::default())
        .await
        .unwrap();
    let item = &pending.data[0];
    let old = f
        .admin
        .user_security(&f.admin_secret, f.user)
        .await
        .unwrap();
    f.admin
        .update_user_security(
            &f.admin_secret,
            f.user,
            &SecurityRequest {
                expected_auth_version: old.auth_version,
                action: SecurityAction::RevokeCredentials,
                reason: "Synthetic generation change".into(),
                public_reason: None,
            },
        )
        .await
        .unwrap();
    assert!(matches!(
        f.admin
            .review_account_appeal(
                &f.admin_secret,
                item.id,
                AppealReviewAction::Approve,
                &review(&item.revision, &item.auth_version)
            )
            .await,
        Err(ProfileError::Conflict)
    ));
    let pending = f
        .admin
        .account_appeals(&f.admin_secret, &AccountAppealsQuery::default())
        .await
        .unwrap();
    assert_eq!(pending.pagination.total, 0);
    let all = f
        .admin
        .account_appeals(
            &f.admin_secret,
            &AccountAppealsQuery {
                status: Some("superseded".into()),
                ..Default::default()
            },
        )
        .await
        .unwrap();
    assert_eq!(all.data[0].status, "superseded");
    assert_eq!(
        f.browser
            .account_appeal_context(&first.token)
            .await
            .unwrap()
            .request
            .unwrap()
            .status,
        "superseded"
    );
    f.role.cleanup().await;
}

#[sqlx::test(migrations = false)]
async fn rejected_request_keeps_account_disabled_and_expired_proof_cannot_submit(pool: PgPool) {
    let f = fixture(&pool, false).await;
    let grant = f
        .browser
        .password_account_appeal("appeal-user", PASSWORD)
        .await
        .unwrap();
    let receipt = f
        .browser
        .submit_account_appeal(&grant.token, "Review these synthetic facts")
        .await
        .unwrap();
    let item = f
        .admin
        .account_appeals(&f.admin_secret, &AccountAppealsQuery::default())
        .await
        .unwrap()
        .data
        .remove(0);
    f.admin
        .review_account_appeal(
            &f.admin_secret,
            receipt.id,
            AppealReviewAction::Reject,
            &review(&item.revision, &item.auth_version),
        )
        .await
        .unwrap();
    assert!(
        !f.admin
            .user_security(&f.admin_secret, f.user)
            .await
            .unwrap()
            .active
    );
    assert_eq!(
        f.browser
            .account_appeal_context(&grant.token)
            .await
            .unwrap()
            .request
            .unwrap()
            .status,
        "rejected"
    );
    let fresh = f
        .browser
        .password_account_appeal("appeal-user", PASSWORD)
        .await
        .unwrap();
    sqlx::query(
        "UPDATE core_account_appeals.flows SET expires_at=clock_timestamp()-interval '1 second'",
    )
    .execute(&pool)
    .await
    .unwrap();
    assert_eq!(
        f.browser
            .submit_account_appeal(&fresh.token, "An expired proof is rejected")
            .await
            .err(),
        Some(identity::IdentityError::Unauthorized)
    );
    let outcome = f
        .browser
        .login_with_mfa(
            "appeal-user",
            PASSWORD,
            &BrowserSessionMetadata::default(),
            &"a".repeat(64),
        )
        .await
        .unwrap();
    assert!(matches!(outcome, BrowserLoginOutcome::Disabled(_)));
    f.role.cleanup().await;
}

#[sqlx::test(migrations = false)]
async fn published_explanation_survives_revocation_but_never_follows_a_new_ban(pool: PgPool) {
    let f = fixture(&pool, true).await;
    let before = f
        .admin
        .user_security(&f.admin_secret, f.user)
        .await
        .unwrap();
    f.admin
        .update_user_security(
            &f.admin_secret,
            f.user,
            &SecurityRequest {
                expected_auth_version: before.auth_version,
                action: SecurityAction::RevokeCredentials,
                reason: PRIVATE.into(),
                public_reason: None,
            },
        )
        .await
        .unwrap();
    let grant = f
        .browser
        .password_account_appeal("appeal-user", PASSWORD)
        .await
        .unwrap();
    assert_eq!(grant.context.public_reason.as_deref(), Some(PUBLIC));
    let before = f
        .admin
        .user_security(&f.admin_secret, f.user)
        .await
        .unwrap();
    f.admin
        .update_user_security(
            &f.admin_secret,
            f.user,
            &SecurityRequest {
                expected_auth_version: before.auth_version,
                action: SecurityAction::Enable,
                reason: PRIVATE.into(),
                public_reason: None,
            },
        )
        .await
        .unwrap();
    // An independent owner action may suspend without publishing any reason.
    sqlx::query(
        "UPDATE core_identity.users SET active=FALSE,auth_version=auth_version+1 WHERE id=$1",
    )
    .bind(f.user)
    .execute(&pool)
    .await
    .unwrap();
    let grant = f
        .browser
        .password_account_appeal("appeal-user", PASSWORD)
        .await
        .unwrap();
    assert!(!grant.context.public_reason_available);
    assert!(grant.context.public_reason.is_none());
    f.role.cleanup().await;
}

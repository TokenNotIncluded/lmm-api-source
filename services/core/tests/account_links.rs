//! Native binding ownership, revocation and browser-flow boundaries on real PG.
//! Provider token exchange remains the existing fixed-provider implementation;
//! these tests never bypass it or publish a constructor for verified identities.
use axum::{
    Router,
    body::{Body, to_bytes},
    http::{HeaderMap, Request, StatusCode},
};
use lmm_core::{
    core_schema,
    identity::{
        AccountBindFinish, AccountLinkProvider, AccountUnlinkRequest, BrowserAuth, BrowserBundle,
        BrowserRegistration, BrowserSessionMetadata, ExternalOAuth, ExternalProvider, GoogleOAuth,
        IdentityError, IdentityStore,
    },
    legacy_auth::{self, LegacyAuthConfig},
};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use sqlx::PgPool;
use tower::ServiceExt;

#[path = "common/runtime.rs"]
mod runtime;
const ORIGIN: &str = "https://bindings.example.test";
const PASSWORD: &str = "account-link fixture";
const GOOGLE: &str = "https://accounts.google.com";
const GITHUB: &str = "https://github.com";
const BINDING: &str = "1111111111111111111111111111111111111111111111111111111111111111";
const OTHER: &str = "2222222222222222222222222222222222222222222222222222222222222222";
struct Fixture {
    role: runtime::RuntimeRole,
    store: IdentityStore,
    browser: BrowserAuth,
    app: Router,
    original: BrowserBundle,
}
async fn fixture(pool: &PgPool) -> Fixture {
    core_schema::init(pool).await.unwrap();
    let role = runtime::RuntimeRole::create(pool).await;
    role.optional("web_identity").await;
    let store = IdentityStore::from_pool(role.pool.clone())
        .with_registration_enabled(true)
        .with_account_links(true)
        .with_google_oauth(
            GoogleOAuth::new(
                "synthetic-google".into(),
                "synthetic-client-secret".into(),
                format!("{ORIGIN}/api/oauth/google/callback"),
            )
            .unwrap(),
        )
        .with_external_oauth(
            ExternalOAuth::new(
                ExternalProvider::Github,
                "synthetic-github",
                "synthetic-client-secret",
                &format!("{ORIGIN}/api/oauth/github"),
            )
            .unwrap(),
        )
        .unwrap();
    let browser = BrowserAuth::new(store.clone(), &[43; 32]).unwrap();
    browser
        .register(&BrowserRegistration {
            username: "link_member",
            password: PASSWORD,
            email: "",
        })
        .await
        .unwrap();
    let original = browser
        .login("link_member", PASSWORD, &BrowserSessionMetadata::default())
        .await
        .unwrap();
    let app = legacy_auth::router(
        store.clone(),
        LegacyAuthConfig::new(ORIGIN, &[43; 32]).unwrap(),
    );
    Fixture {
        role,
        store,
        browser,
        app,
        original,
    }
}
fn access(bundle: &BrowserBundle) -> &str {
    bundle.data["access_token"].as_str().unwrap()
}
fn user(bundle: &BrowserBundle) -> i64 {
    bundle.data["user"]["id"].as_i64().unwrap()
}
fn sid(bundle: &BrowserBundle) -> &str {
    bundle.data["session"]["sid"].as_str().unwrap()
}
async fn add_link(pool: &PgPool, user: i64, issuer: &str, subject: &str) {
    sqlx::query(
        "INSERT INTO core_identity.oauth_identities(issuer,subject,user_id) VALUES($1,$2,$3)",
    )
    .bind(issuer)
    .bind(subject)
    .bind(user)
    .execute(pool)
    .await
    .unwrap();
}
async fn unlink_request(
    browser: &BrowserAuth,
    access: &str,
    issuer: &str,
    subject: &str,
) -> AccountUnlinkRequest {
    let view = browser.account_links(access).await.unwrap();
    let link = view["links"]
        .as_array()
        .unwrap()
        .iter()
        .find(|link| link["issuer"] == issuer && link["subject"] == subject)
        .unwrap();
    AccountUnlinkRequest {
        issuer: issuer.into(),
        subject: subject.into(),
        expected_revision: link["revision"].as_str().unwrap().into(),
        expected_auth_version: view["expected_auth_version"].as_str().unwrap().into(),
    }
}
async fn request(
    app: &Router,
    method: &str,
    path: &str,
    token: Option<&str>,
    origin: Option<&str>,
    cookie: Option<&str>,
    body: Value,
) -> (StatusCode, HeaderMap, Value) {
    let mut request = Request::builder()
        .method(method)
        .uri(path)
        .header("content-type", "application/json");
    if let Some(token) = token {
        request = request.header("authorization", format!("Bearer {token}"));
    }
    if let Some(origin) = origin {
        request = request.header("origin", origin);
    }
    if let Some(cookie) = cookie {
        request = request.header("cookie", cookie);
    }
    let response = app
        .clone()
        .oneshot(request.body(Body::from(body.to_string())).unwrap())
        .await
        .unwrap();
    let status = response.status();
    let headers = response.headers().clone();
    let value = serde_json::from_slice(&to_bytes(response.into_body(), 65536).await.unwrap())
        .unwrap_or(Value::Null);
    (status, headers, value)
}

#[sqlx::test(migrations = false)]
async fn unlink_keeps_identity_ownership_uses_cas_and_rotates_all_old_authority(pool: PgPool) {
    let f = fixture(&pool).await;
    add_link(
        &pool,
        user(&f.original),
        GOOGLE,
        "retained-provider-subject",
    )
    .await;
    let other = f
        .browser
        .login("link_member", PASSWORD, &BrowserSessionMetadata::default())
        .await
        .unwrap();
    let before:Value=sqlx::query_scalar("SELECT jsonb_build_array(created_at,expires_at,weekly_logout) FROM core_identity.web_sessions WHERE sid=$1").bind(sid(&f.original)).fetch_one(&pool).await.unwrap();
    let input = unlink_request(
        &f.browser,
        access(&f.original),
        GOOGLE,
        "retained-provider-subject",
    )
    .await;
    assert_eq!(input.expected_revision, "0");
    let current = f
        .browser
        .unlink_account(access(&f.original), &input, None)
        .await
        .unwrap();
    assert_eq!(sid(&current), sid(&f.original));
    let after:Value=sqlx::query_scalar("SELECT jsonb_build_array(created_at,expires_at,weekly_logout) FROM core_identity.web_sessions WHERE sid=$1").bind(sid(&current)).fetch_one(&pool).await.unwrap();
    assert_eq!(before, after);
    assert!(matches!(
        f.store.authorize(access(&f.original)).await,
        Err(IdentityError::Unauthorized)
    ));
    assert!(f.store.authorize(access(&other)).await.is_err());
    assert!(f.store.authorize(access(&current)).await.is_ok());
    let stored:(i64,i64,bool)=sqlx::query_as("SELECT i.user_id,s.revision,s.revoked_at IS NOT NULL FROM core_identity.oauth_identities i JOIN core_account_links.oauth_states s USING(issuer,subject) WHERE i.issuer=$1 AND i.subject=$2")
        .bind(GOOGLE).bind("retained-provider-subject").fetch_one(&pool).await.unwrap();
    assert_eq!(stored, (user(&current), 1, true));
    let view = f.browser.account_links(access(&current)).await.unwrap();
    assert_eq!(view["links"][0]["active"], false);
    let stale = AccountUnlinkRequest {
        expected_auth_version: view["expected_auth_version"].as_str().unwrap().into(),
        ..input
    };
    assert!(
        f.browser
            .unlink_account(access(&current), &stale, None)
            .await
            .is_err()
    );
    assert!(
        sqlx::query("DELETE FROM core_account_links.oauth_states")
            .execute(&f.role.pool)
            .await
            .is_err()
    );
    assert!(
        sqlx::query("UPDATE core_account_links.oauth_states SET user_id=user_id+1")
            .execute(&f.role.pool)
            .await
            .is_err()
    );
    assert_eq!(sqlx::query_scalar::<_,i64>("SELECT count(*) FROM core_identity.audit WHERE actor_user_id=$1 AND action='identity.account_unlink'").bind(user(&current)).fetch_one(&pool).await.unwrap(),1);
    f.role.cleanup().await;
}

/// Append valid, synthetic primary evidence after an owner-only policy fixture
/// change; never disable immutable guards or expose an unverified OAuth proof.
async fn synthetic_session(pool: &PgPool, user: i64, old: bool) -> String {
    let secret = format!("lmms_{}", "a".repeat(64));
    let sid = "b".repeat(64);
    let offset = if old { 660.0_f64 } else { 0.0_f64 };
    let (id,version):(i64,i64)=sqlx::query_as("INSERT INTO core_identity.credentials(digest,kind,user_id,user_version,owner_account_id,expires_at,session_started_at) SELECT $1,'session',id,auth_version,personal_account_id,clock_timestamp()+interval '1 hour',clock_timestamp()-make_interval(secs=>$3::double precision) FROM core_identity.users WHERE id=$2 RETURNING id,user_version")
        .bind(Sha256::digest(secret.as_bytes()).to_vec()).bind(user).bind(offset).fetch_one(pool).await.unwrap();
    sqlx::query("INSERT INTO core_identity.session_authentication(credential_id,user_id,user_version,factor_revision,factor_level,method,authenticated_at) VALUES($1,$2,$3,0,1,'primary',clock_timestamp()-make_interval(secs=>$4::double precision))")
        .bind(id).bind(user).bind(version).bind(offset).execute(pool).await.unwrap();
    sqlx::query("INSERT INTO core_identity.web_sessions(sid,user_id,user_version,current_credential_id,refresh_digest,ip,user_agent,created_at,expires_at) VALUES($1,$2,$3,$4,$5,'','synthetic account-link fixture',clock_timestamp()-make_interval(secs=>$6::double precision),clock_timestamp()+interval '1 hour')")
        .bind(&sid).bind(user).bind(version).bind(id).bind(vec![42_u8;32]).bind(offset).execute(pool).await.unwrap();
    sqlx::query(
        "INSERT INTO core_identity.web_session_credentials(credential_id,sid) VALUES($1,$2)",
    )
    .bind(id)
    .bind(&sid)
    .execute(pool)
    .await
    .unwrap();
    secret
}

#[sqlx::test(migrations = false)]
async fn last_configured_login_cannot_be_unlinked_and_history_cannot_stand_in_for_a_login(
    pool: PgPool,
) {
    let f = fixture(&pool).await;
    add_link(&pool, user(&f.original), GOOGLE, "only-usable-login").await;
    add_link(&pool, user(&f.original), GITHUB, "revoked-history").await;
    sqlx::query("INSERT INTO core_account_links.oauth_states(issuer,subject,user_id,revoked_at) VALUES($1,'revoked-history',$2,clock_timestamp())").bind(GITHUB).bind(user(&f.original)).execute(&pool).await.unwrap();
    sqlx::query(
        "UPDATE core_identity.user_profiles SET password_login_allowed=false WHERE user_id=$1",
    )
    .bind(user(&f.original))
    .execute(&pool)
    .await
    .unwrap();
    let current = synthetic_session(&pool, user(&f.original), false).await;
    let input = unlink_request(&f.browser, &current, GOOGLE, "only-usable-login").await;
    assert!(
        f.browser
            .unlink_account(&current, &input, None)
            .await
            .is_err()
    );
    assert!(f.store.authorize(&current).await.is_ok());
    assert_eq!(
        sqlx::query_scalar::<_, i64>(
            "SELECT count(*) FROM core_account_links.oauth_states WHERE issuer=$1"
        )
        .bind(GOOGLE)
        .fetch_one(&pool)
        .await
        .unwrap(),
        0
    );
    // Re-enable the second identity as an owner fixture, but do not configure
    // that provider in this browser. It still cannot serve as a usable fallback.
    sqlx::query("UPDATE core_account_links.oauth_states SET revoked_at=NULL,revision=revision+1 WHERE issuer=$1").bind(GITHUB).execute(&pool).await.unwrap();
    let unavailable = BrowserAuth::new(
        IdentityStore::from_pool(f.role.pool.clone()).with_account_links(true),
        &[43; 32],
    )
    .unwrap();
    assert!(
        unavailable
            .unlink_account(&current, &input, None)
            .await
            .is_err()
    );
    f.role.cleanup().await;
}

#[sqlx::test(migrations = false)]
async fn binding_is_cookie_provider_family_bound_and_api_callbacks_only_relay(pool: PgPool) {
    let f = fixture(&pool).await;
    let version =
        f.browser.account_links(access(&f.original)).await.unwrap()["expected_auth_version"]
            .as_str()
            .unwrap()
            .to_owned();
    let (status, _, _) = request(
        &f.app,
        "POST",
        "/api/user/account-links/google/begin",
        Some(access(&f.original)),
        Some("https://different.example.test"),
        None,
        json!({"expected_auth_version":version}),
    )
    .await;
    assert_eq!(status, StatusCode::FORBIDDEN);
    let (status, headers, body) = request(
        &f.app,
        "POST",
        "/api/user/account-links/google/begin",
        Some(access(&f.original)),
        Some(ORIGIN),
        None,
        json!({"expected_auth_version":version}),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    let cookie = headers
        .get_all("set-cookie")
        .iter()
        .find_map(|value| {
            value
                .to_str()
                .ok()
                .filter(|value| value.starts_with("__Host-lmm_account_bind_google="))
        })
        .unwrap();
    assert!(
        cookie.contains("HttpOnly") && cookie.contains("Secure") && cookie.contains("SameSite=Lax")
    );
    let state = body["data"]["state"].as_str().unwrap();
    let url = reqwest::Url::parse(body["data"]["authorization_url"].as_str().unwrap()).unwrap();
    assert_eq!(url.host_str(), Some("accounts.google.com"));
    assert_eq!(
        url.query_pairs().find(|(key, _)| key == "state").unwrap().1,
        state
    );
    assert!(
        f.browser
            .is_account_bind_state(AccountLinkProvider::Google, state)
            .await
            .unwrap()
    );
    assert!(
        !f.browser
            .is_account_bind_state(
                AccountLinkProvider::External(ExternalProvider::Github),
                state
            )
            .await
            .unwrap()
    );
    let input = AccountBindFinish {
        state: state.into(),
        code: "not-exchanged".into(),
        iss: None,
    };
    assert!(
        f.browser
            .finish_account_bind(
                access(&f.original),
                AccountLinkProvider::Google,
                OTHER,
                &input
            )
            .await
            .is_err()
    );
    assert!(
        f.browser
            .is_account_bind_state(AccountLinkProvider::Google, state)
            .await
            .unwrap()
    );
    let (status, headers, _) = request(
        &f.app,
        "GET",
        &format!("/api/oauth/google/callback?state={state}&code=not-exchanged"),
        None,
        None,
        None,
        Value::Null,
    )
    .await;
    assert_eq!(status, StatusCode::SEE_OTHER);
    let redirect = reqwest::Url::parse(headers["location"].to_str().unwrap()).unwrap();
    assert_eq!(redirect.origin().ascii_serialization(), ORIGIN);
    assert_eq!(redirect.path(), "/oauth/google");
    assert_eq!(
        redirect
            .query_pairs()
            .find(|(key, _)| key == "account_bind")
            .unwrap()
            .1,
        "1"
    );
    assert!(!headers.contains_key("set-cookie"));
    assert!(
        f.browser
            .is_account_bind_state(AccountLinkProvider::Google, state)
            .await
            .unwrap()
    );
    // Correct bound flow with invalid code is terminal, before any network I/O.
    let cookie = cookie.split(';').next().unwrap();
    assert_eq!(
        request(
            &f.app,
            "POST",
            "/api/user/account-links/google/finish",
            Some(access(&f.original)),
            Some(ORIGIN),
            Some(cookie),
            json!({"state":state,"code":""})
        )
        .await
        .0,
        StatusCode::UNAUTHORIZED
    );
    assert!(
        !f.browser
            .is_account_bind_state(AccountLinkProvider::Google, state)
            .await
            .unwrap()
    );
    assert_eq!(
        sqlx::query_scalar::<_, i64>(
            "SELECT count(*) FROM core_identity.oauth_identities WHERE user_id=$1"
        )
        .bind(user(&f.original))
        .fetch_one(&pool)
        .await
        .unwrap(),
        0
    );
    f.role.cleanup().await;
}

#[sqlx::test(migrations = false)]
async fn stale_sessions_disabled_component_and_missing_tables_never_grant_a_write(pool: PgPool) {
    let f = fixture(&pool).await;
    add_link(&pool, user(&f.original), GOOGLE, "stale-proof-target").await;
    let stale = synthetic_session(&pool, user(&f.original), true).await;
    let input = unlink_request(&f.browser, &stale, GOOGLE, "stale-proof-target").await;
    assert!(f.store.authorize(&stale).await.is_ok());
    assert!(
        f.browser
            .unlink_account(&stale, &input, None)
            .await
            .is_err()
    );
    assert!(
        f.browser
            .begin_account_bind(
                &stale,
                AccountLinkProvider::Google,
                BINDING,
                &input.expected_auth_version
            )
            .await
            .is_err()
    );
    let disabled = BrowserAuth::new(f.store.clone().with_account_links(false), &[43; 32]).unwrap();
    assert!(
        disabled
            .unlink_account(access(&f.original), &input, None)
            .await
            .is_err()
    );
    sqlx::query("ALTER TABLE core_account_links.oauth_states RENAME TO unavailable_states")
        .execute(&pool)
        .await
        .unwrap();
    assert!(f.browser.account_links(access(&f.original)).await.is_err());
    assert!(
        f.browser
            .unlink_account(access(&f.original), &input, None)
            .await
            .is_err()
    );
    f.role.cleanup().await;
}

//! Real HTTP envelopes and Cookie boundaries against an isolated complete Core DB.
use axum::{
    Router,
    body::{Body, to_bytes},
    http::{HeaderMap, Request, StatusCode},
};
use lmm_core::{
    identity::{
        BrowserAuth, BrowserRegistration, BrowserSessionMetadata, IdentityError, IdentityStore,
    },
    legacy_auth::{self, LegacyAuthConfig},
};
use serde_json::{Value, json};
use sqlx::PgPool;
use tower::ServiceExt;

const ORIGIN: &str = "https://appeal.example";
const PATH: &str = "/api/user/account-action-requests/appeal";
const PASSWORD: &str = "appeal-test-971";

async fn fixture(pool: &PgPool) -> (BrowserAuth, Router, i64) {
    lmm_core::core_schema::init(pool).await.unwrap();
    let store = IdentityStore::from_pool(pool.clone()).with_registration_enabled(true);
    let browser = BrowserAuth::new(store.clone(), &[71; 32]).unwrap();
    browser
        .register(&BrowserRegistration {
            username: "appeal-owner",
            password: PASSWORD,
            email: "",
        })
        .await
        .unwrap();
    let id: i64 = sqlx::query_scalar(
        "SELECT user_id FROM core_identity.user_profiles WHERE username='appeal-owner'",
    )
    .fetch_one(pool)
    .await
    .unwrap();
    let app = legacy_auth::router(store, LegacyAuthConfig::new(ORIGIN, &[71; 32]).unwrap());
    (browser, app, id)
}

async fn call(
    app: &Router,
    method: &str,
    path: &str,
    body: Option<Value>,
    headers: &[(&str, &str)],
) -> (StatusCode, HeaderMap, Value) {
    let mut request = Request::builder().method(method).uri(path);
    for (name, value) in headers {
        request = request.header(*name, *value);
    }
    let body = if let Some(body) = body {
        request = request.header("content-type", "application/json");
        Body::from(body.to_string())
    } else {
        Body::empty()
    };
    let response = app
        .clone()
        .oneshot(request.body(body).unwrap())
        .await
        .unwrap();
    let status = response.status();
    let headers = response.headers().clone();
    let bytes = to_bytes(response.into_body(), 65536).await.unwrap();
    (
        status,
        headers,
        serde_json::from_slice(&bytes).unwrap_or(Value::Null),
    )
}

fn appeal_cookie(headers: &HeaderMap) -> String {
    headers
        .get_all("set-cookie")
        .iter()
        .map(|v| v.to_str().unwrap())
        .find(|v| v.starts_with("lmm_account_appeal="))
        .unwrap()
        .split(';')
        .next()
        .unwrap()
        .to_owned()
}

async fn authority_counts(pool: &PgPool) -> (i64, i64, i64) {
    sqlx::query_as("SELECT (SELECT count(*) FROM core_identity.credentials),(SELECT count(*) FROM core_identity.web_sessions),(SELECT count(*) FROM core_identity.mfa_challenges)").fetch_one(pool).await.unwrap()
}

#[sqlx::test(migrations = false)]
async fn disabled_login_and_cookie_submission_never_issue_normal_authority(pool: PgPool) {
    let (_, app, user) = fixture(&pool).await;
    sqlx::query(
        "UPDATE core_identity.users SET active=FALSE,auth_version=auth_version+1 WHERE id=$1",
    )
    .bind(user)
    .execute(&pool)
    .await
    .unwrap();
    // Primary disabled ownership works even when a second factor is required.
    sqlx::query(
        "UPDATE core_identity.user_profiles SET second_factor_required=TRUE WHERE user_id=$1",
    )
    .bind(user)
    .execute(&pool)
    .await
    .unwrap();
    let before = authority_counts(&pool).await;
    let (status, headers, body) = call(
        &app,
        "POST",
        "/api/user/login",
        Some(json!({"username":"appeal-owner","password":PASSWORD})),
        &[("origin", ORIGIN)],
    )
    .await;
    assert_eq!(status, StatusCode::FORBIDDEN, "{body}");
    assert_eq!(body["code"], "AUTH_USER_DISABLED");
    assert_eq!(body["data"]["appeal_available"], true);
    assert_eq!(body["data"]["username"], "appeal-owner");
    assert!(body["data"].get("token").is_none());
    assert!(body["data"].get("access_token").is_none());
    assert_eq!(headers["cache-control"], "no-store");
    let cookie = appeal_cookie(&headers);
    let token = cookie.strip_prefix("lmm_account_appeal=").unwrap();
    assert!(!body.to_string().contains(token));
    assert!(headers.get_all("set-cookie").iter().any(|h| {
        let h = h.to_str().unwrap();
        h.starts_with("new_api_refresh=;") && h.contains("Max-Age=0")
    }));
    let (status, _, context) = call(&app, "GET", PATH, None, &[("cookie", &cookie)]).await;
    assert_eq!(status, StatusCode::OK, "{context}");
    assert_eq!(context["success"], true);
    assert!(context["data"]["request"].is_null());
    let reason = json!({"reason":"Please review this suspension"});
    let (status, _, receipt) = call(
        &app,
        "POST",
        PATH,
        Some(reason.clone()),
        &[("origin", ORIGIN), ("cookie", &cookie)],
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{receipt}");
    assert_eq!(receipt["data"]["status"], "pending");
    let (_, _, repeat) = call(
        &app,
        "POST",
        PATH,
        Some(reason),
        &[("origin", ORIGIN), ("cookie", &cookie)],
    )
    .await;
    assert_eq!(repeat["data"]["id"], receipt["data"]["id"]);
    assert_eq!(repeat["data"]["already_pending"], true);
    let (status, _, context) = call(&app, "GET", PATH, None, &[("cookie", &cookie)]).await;
    assert_eq!(status, StatusCode::OK, "{context}");
    assert_eq!(context["data"]["request"]["id"], receipt["data"]["id"]);
    assert_eq!(authority_counts(&pool).await, before);
}

#[sqlx::test(migrations = false)]
async fn password_submission_rejects_active_and_disallowed_password_without_sessions(pool: PgPool) {
    let (browser, app, user) = fixture(&pool).await;
    let before = authority_counts(&pool).await;
    assert!(matches!(
        browser
            .password_account_appeal("appeal-owner", PASSWORD)
            .await,
        Err(IdentityError::Forbidden)
    ));
    let input =
        json!({"username":"appeal-owner","password":PASSWORD,"reason":"Please review suspension"});
    let (status, headers, body) = call(
        &app,
        "POST",
        PATH,
        Some(input.clone()),
        &[("origin", ORIGIN)],
    )
    .await;
    assert_eq!(status, StatusCode::FORBIDDEN, "{body}");
    assert!(!headers.contains_key("set-cookie"));
    assert_eq!(authority_counts(&pool).await, before);
    sqlx::query(
        "UPDATE core_identity.users SET active=FALSE,auth_version=auth_version+1 WHERE id=$1",
    )
    .bind(user)
    .execute(&pool)
    .await
    .unwrap();
    sqlx::query(
        "UPDATE core_identity.user_profiles SET password_login_allowed=FALSE WHERE user_id=$1",
    )
    .bind(user)
    .execute(&pool)
    .await
    .unwrap();
    let (status, headers, _) = call(
        &app,
        "POST",
        PATH,
        Some(input.clone()),
        &[("origin", ORIGIN)],
    )
    .await;
    assert_eq!(status, StatusCode::UNAUTHORIZED);
    assert!(!headers.contains_key("set-cookie"));
    sqlx::query(
        "UPDATE core_identity.user_profiles SET password_login_allowed=TRUE WHERE user_id=$1",
    )
    .bind(user)
    .execute(&pool)
    .await
    .unwrap();
    let (status, headers, body) =
        call(&app, "POST", PATH, Some(input), &[("origin", ORIGIN)]).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(body["data"]["status"], "pending");
    let cookie = appeal_cookie(&headers);
    let (status, _, context) = call(&app, "GET", PATH, None, &[("cookie", &cookie)]).await;
    assert_eq!(status, StatusCode::OK, "{context}");
    assert_eq!(context["data"]["request"]["id"], body["data"]["id"]);
    assert_eq!(authority_counts(&pool).await, before);
}

#[sqlx::test(migrations = false)]
async fn http_requires_exact_origin_json_and_cookie_ownership(pool: PgPool) {
    let (browser, app, _) = fixture(&pool).await;
    let normal = browser
        .login("appeal-owner", PASSWORD, &BrowserSessionMetadata::default())
        .await
        .unwrap();
    let bearer = format!("Bearer {}", normal.data["access_token"].as_str().unwrap());
    let body = json!({"reason":"Please review suspension"});
    for method in ["GET", "POST"] {
        let (status, _, result) = call(
            &app,
            method,
            PATH,
            if method == "POST" {
                Some(body.clone())
            } else {
                None
            },
            &[("origin", ORIGIN), ("authorization", &bearer)],
        )
        .await;
        assert_eq!(status, StatusCode::UNAUTHORIZED, "{result}");
        assert_eq!(result["code"], "AUTH_ACCOUNT_APPEAL_PROOF_INVALID");
    }
    for headers in [
        vec![("referer", "https://appeal.example/sign-in")],
        vec![("origin", "https://evil.example")],
        vec![("origin", ORIGIN), ("origin", ORIGIN)],
    ] {
        let (status, headers, _) = call(&app, "POST", PATH, Some(body.clone()), &headers).await;
        assert_eq!(status, StatusCode::FORBIDDEN);
        assert!(!headers.contains_key("set-cookie"));
    }
    let (status, _, _) = call(&app, "POST", PATH, None, &[("origin", ORIGIN)]).await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    let (status, _, _) = call(
        &app,
        "POST",
        PATH,
        Some(json!({"reason":"Please review", "token":"forged"})),
        &[("origin", ORIGIN)],
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    let (status, _, _) = call(
        &app,
        "POST",
        PATH,
        Some(json!({"reason":"Please review", "username":"appeal-owner"})),
        &[("origin", ORIGIN)],
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
}

#[sqlx::test(migrations = false)]
async fn normal_login_and_logout_clear_previous_account_appeal_cookie(pool: PgPool) {
    let (browser, app, user) = fixture(&pool).await;
    sqlx::query(
        "UPDATE core_identity.users SET active=FALSE,auth_version=auth_version+1 WHERE id=$1",
    )
    .bind(user)
    .execute(&pool)
    .await
    .unwrap();
    let (status, headers, _) = call(
        &app,
        "POST",
        "/api/user/login",
        Some(json!({"username":"appeal-owner","password":PASSWORD})),
        &[("origin", ORIGIN)],
    )
    .await;
    assert_eq!(status, StatusCode::FORBIDDEN);
    let old_proof = appeal_cookie(&headers);
    browser
        .register(&BrowserRegistration {
            username: "second-owner",
            password: PASSWORD,
            email: "",
        })
        .await
        .unwrap();
    let (status, headers, body) = call(
        &app,
        "POST",
        "/api/user/login",
        Some(json!({"username":"second-owner","password":PASSWORD})),
        &[("origin", ORIGIN), ("cookie", &old_proof)],
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(body["data"]["user"]["username"], "second-owner");
    let cleared = "lmm_account_appeal=; Path=/api/user/account-action-requests; Secure; HttpOnly; SameSite=Lax; Max-Age=0";
    assert!(
        headers
            .get_all("set-cookie")
            .iter()
            .any(|v| v.to_str().unwrap() == cleared)
    );
    let refresh = headers
        .get_all("set-cookie")
        .iter()
        .map(|v| v.to_str().unwrap())
        .find(|v| v.starts_with("new_api_refresh="))
        .unwrap()
        .split(';')
        .next()
        .unwrap()
        .to_owned();
    // Apply the response's Cookie deletion: the shared browser now sends only B's refresh Cookie.
    let (status, _, _) = call(&app, "GET", PATH, None, &[("cookie", &refresh)]).await;
    assert_eq!(status, StatusCode::UNAUTHORIZED);
    let stale_browser_cookies = format!("{refresh}; {old_proof}");
    let (status, headers, body) = call(
        &app,
        "POST",
        "/api/user/auth/logout",
        None,
        &[("origin", ORIGIN), ("cookie", &stale_browser_cookies)],
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert!(
        headers
            .get_all("set-cookie")
            .iter()
            .any(|v| v.to_str().unwrap() == cleared)
    );
}

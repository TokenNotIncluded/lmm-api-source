//! Isolated PostgreSQL and private policy-file coverage for HeroSMS settings.
use axum::{
    Router,
    body::{Body, to_bytes},
    http::{Request, StatusCode},
};
use lmm_core::{
    core_schema,
    funding::FundingPolicy,
    hero_sms::{self, PolicyFile, Service},
    identity::IdentityStore,
};
use serde_json::{Value, json};
use sqlx::PgPool;
use std::{fs, os::unix::fs::PermissionsExt};
use tower::ServiceExt;
#[path = "common/runtime.rs"]
mod runtime;

async fn call(
    app: &Router,
    action: &str,
    host: &str,
    user: Option<&str>,
    body: Value,
) -> (StatusCode, Value) {
    let mut request = Request::builder()
        .method("POST")
        .uri(format!("/internal/hero-sms/v1/{action}"))
        .header("content-type", "application/json")
        .header("authorization", format!("Bearer {host}"))
        .header("x-lmm-protocol", "1");
    if let Some(user) = user {
        request = request.header("x-lmm-user-credential", user);
    }
    let response = app
        .clone()
        .oneshot(request.body(Body::from(body.to_string())).unwrap())
        .await
        .unwrap();
    let status = response.status();
    assert_eq!(response.headers()["cache-control"], "no-store");
    let bytes = to_bytes(response.into_body(), 65536).await.unwrap();
    (status, serde_json::from_slice(&bytes).unwrap())
}

#[sqlx::test(migrations = false)]
async fn settings_require_current_root_session_and_persist_without_database_writes(pool: PgPool) {
    core_schema::init(&pool).await.unwrap();
    let identity = IdentityStore::from_pool(pool.clone());
    let root = identity.bootstrap_user(1, 6).await.unwrap();
    let admin = identity.bootstrap_user(2, 5).await.unwrap();
    let user = identity.bootstrap_user(3, 0).await.unwrap();
    let actor = identity.authorize(&root.secret).await.unwrap();
    let key = identity
        .issue_key(
            &root.secret,
            actor.owner,
            FundingPolicy::new_personal_key(),
            3600,
        )
        .await
        .unwrap();
    let role = runtime::RuntimeRole::create(&pool).await;
    let directory =
        std::env::temp_dir().join(format!("lmm-hero-runtime-{:016x}", rand::random::<u64>()));
    fs::create_dir(&directory).unwrap();
    fs::set_permissions(&directory, fs::Permissions::from_mode(0o700)).unwrap();
    let path = directory.join("policy.json");
    fs::write(&path, r#"{"revision":0,"enabled":false,"sms_enabled":false,"email_enabled":false,"price_multiplier":"1"}"#).unwrap();
    fs::set_permissions(&path, fs::Permissions::from_mode(0o600)).unwrap();
    let host = "synthetic-hero-host-111111111111111111";
    let app = hero_sms::http::router(
        Service::new(
            role.pool.clone(),
            PolicyFile::open(path.clone()).unwrap(),
            None,
        )
        .unwrap(),
        host.as_bytes(),
    )
    .unwrap();
    assert_eq!(
        call(
            &app,
            "ready",
            "incorrect-host-token-111111111111111",
            None,
            json!({})
        )
        .await
        .0,
        StatusCode::UNAUTHORIZED
    );
    let (status, ready) = call(&app, "ready", host, None, json!({})).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(ready["purchases_available"], false);
    assert_eq!(
        call(&app, "settings", host, None, json!({})).await.0,
        StatusCode::UNAUTHORIZED
    );
    for denied in [&admin.secret, &user.secret, &key.secret] {
        assert_eq!(
            call(&app, "settings", host, Some(denied), json!({}))
                .await
                .0,
            StatusCode::FORBIDDEN
        );
    }
    let update = json!({"revision":0,"enabled":false,"sms_enabled":true,"email_enabled":false,"price_multiplier":"2.000001"});
    let (status, saved) = call(
        &app,
        "save-policy",
        host,
        Some(&root.secret),
        update.clone(),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(saved["revision"], 1);
    assert_eq!(saved["price_multiplier"], "2.000001");
    assert_eq!(saved["purchases_available"], false);
    assert_eq!(
        call(&app, "save-policy", host, Some(&root.secret), update)
            .await
            .0,
        StatusCode::CONFLICT
    );
    let reopened = hero_sms::http::router(
        Service::new(
            role.pool.clone(),
            PolicyFile::open(path.clone()).unwrap(),
            None,
        )
        .unwrap(),
        host.as_bytes(),
    )
    .unwrap();
    let (status, current) = call(&reopened, "settings", host, Some(&root.secret), json!({})).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(current, saved);
    assert_eq!(
        call(&reopened, "test", host, Some(&root.secret), json!({}))
            .await
            .1["error"],
        "CREDENTIAL_NOT_CONFIGURED"
    );
    assert_eq!(call(&reopened, "save-policy", host, Some(&root.secret), json!({"revision":1,"enabled":true,"sms_enabled":true,"email_enabled":true,"price_multiplier":"1"})).await.1["error"], "CREDENTIAL_NOT_CONFIGURED");
    assert_eq!(
        call(&reopened, "buy", host, Some(&user.secret), json!({}))
            .await
            .1["error"],
        "SERVICE_DISABLED"
    );
    sqlx::query("UPDATE core_identity.users SET platform_role='admin',auth_version=auth_version+1 WHERE id=1")
        .execute(&pool).await.unwrap();
    assert_eq!(
        call(&reopened, "settings", host, Some(&root.secret), json!({}))
            .await
            .0,
        StatusCode::UNAUTHORIZED
    );
    drop(reopened);
    drop(app);
    role.cleanup().await;
    fs::remove_dir_all(directory).unwrap();
}

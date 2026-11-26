use axum::{
    Router,
    body::{Body, to_bytes},
    http::{Request, StatusCode},
    response::Response,
};
use lmm_core::{http, identity::IdentityStore, identity_http};
use serde_json::{Value, json};
use sqlx::PgPool;
use tower::ServiceExt;

async fn call(
    app: &Router,
    method: &str,
    path: &str,
    token: Option<&str>,
    body: Option<Value>,
) -> Response {
    let mut request = Request::builder().method(method).uri(path);
    if let Some(token) = token {
        request = request.header("authorization", format!("Bearer {token}"));
    }
    let body = match body {
        Some(value) => {
            request = request.header("content-type", "application/json");
            Body::from(value.to_string())
        }
        None => Body::empty(),
    };
    app.clone()
        .oneshot(request.body(body).unwrap())
        .await
        .unwrap()
}
async fn json_body(response: Response) -> Value {
    serde_json::from_slice(&to_bytes(response.into_body(), 65536).await.unwrap()).unwrap()
}

#[sqlx::test(migrations = false)]
async fn create_team_persists_the_requested_name_and_preserves_unnamed_creation(pool: PgPool) {
    let store = IdentityStore::from_pool(pool);
    store.init_database().await.unwrap();
    let app = identity_http::router(store.clone());
    for (uid, input, expected) in [
        (
            21,
            Some(json!({"name":"  Fixture Team A  "})),
            "Fixture Team A",
        ),
        (22, None, "Team"),
        (23, Some(json!({})), "Team"),
    ] {
        let session = store.bootstrap_user(uid, 1).await.unwrap();
        let credential = Some(session.secret.as_str());
        let created = call(&app, "POST", "/core/v1/teams", credential, input).await;
        assert_eq!(created.status(), StatusCode::CREATED);
        let id = json_body(created).await["id"].as_i64().unwrap();
        let listed = call(&app, "GET", "/core/v1/teams", credential, None).await;
        assert_eq!(listed.status(), StatusCode::OK);
        let listed = json_body(listed).await;
        let teams = listed.as_array().unwrap();
        assert_eq!(teams.len(), 1);
        assert_eq!(teams[0]["id"], id);
        assert_eq!(teams[0]["name"], expected);
        assert_eq!(teams[0]["version"], 1);
    }

    let session = store.bootstrap_user(24, 1).await.unwrap();
    for name in [" ".to_string(), "a\u{0001}b".to_string(), "界".repeat(101)] {
        let response = call(
            &app,
            "POST",
            "/core/v1/teams",
            Some(&session.secret),
            Some(json!({"name":name})),
        )
        .await;
        assert_eq!(response.status(), StatusCode::BAD_REQUEST);
    }
    for (media_type, contents, status) in [
        (
            "text/plain",
            r#"{"name":"ignored"}"#.into(),
            StatusCode::UNSUPPORTED_MEDIA_TYPE,
        ),
        ("application/json", "{".into(), StatusCode::BAD_REQUEST),
        (
            "application/json",
            r#"{"name":3}"#.into(),
            StatusCode::UNPROCESSABLE_ENTITY,
        ),
        (
            "application/json",
            "x".repeat(16 * 1024 + 1),
            StatusCode::PAYLOAD_TOO_LARGE,
        ),
    ] {
        let request = Request::post("/core/v1/teams")
            .header("authorization", format!("Bearer {}", session.secret))
            .header("content-type", media_type)
            .body(Body::from(contents))
            .unwrap();
        assert_eq!(app.clone().oneshot(request).await.unwrap().status(), status);
    }
    let empty = call(&app, "GET", "/core/v1/teams", Some(&session.secret), None).await;
    assert_eq!(json_body(empty).await, json!([]));
    let request = Request::post("/core/v1/teams")
        .header("authorization", format!("Bearer {}", session.secret))
        .header("content-type", "application/json")
        .body(Body::empty())
        .unwrap();
    assert_eq!(
        app.oneshot(request).await.unwrap().status(),
        StatusCode::CREATED
    );
}

#[sqlx::test(migrations = false)]
async fn native_http_flow_keeps_model_traffic_closed(pool: PgPool) {
    let store = IdentityStore::from_pool(pool.clone());
    store.init_database().await.unwrap();
    // Explicit installer-owned fixture for the key-management/user projection dependency.
    sqlx::raw_sql(sqlx::AssertSqlSafe(lmm_core::abuse::SCHEMA))
        .execute(&pool)
        .await
        .unwrap();
    let session = store.bootstrap_user(11, 1).await.unwrap();
    let app = http::router().merge(identity_http::router(store));
    let denied = call(&app, "GET", "/core/v1/identity", None, None).await;
    assert_eq!(denied.status(), StatusCode::UNAUTHORIZED);
    assert_eq!(denied.headers()["cache-control"], "no-store");
    assert_eq!(denied.headers()["www-authenticate"], "Bearer");
    let response = call(
        &app,
        "GET",
        "/core/v1/identity",
        Some(&session.secret),
        None,
    )
    .await;
    assert_eq!(response.status(), StatusCode::OK);
    let identity = json_body(response).await;
    assert_eq!(identity["user_id"], 11);
    assert!(identity["owner_account_id"].as_i64().unwrap() > 0);
    assert!(identity.get("secret").is_none());
    assert!(identity.get("user_version").is_none());
    let response = call(
        &app,
        "POST",
        "/core/v1/keys",
        Some(&session.secret),
        Some(json!({"owner":{"kind":"personal","id":11},"ttl_seconds":3600})),
    )
    .await;
    assert_eq!(response.status(), StatusCode::CREATED);
    assert_eq!(response.headers()["cache-control"], "no-store");
    let key = json_body(response).await;
    let token = key["secret"].as_str().unwrap();
    assert_eq!(
        call(&app, "GET", "/core/v1/identity", Some(token), None)
            .await
            .status(),
        StatusCode::OK
    );
    assert_eq!(
        call(&app, "POST", "/core/v1/teams", Some(token), None)
            .await
            .status(),
        StatusCode::FORBIDDEN
    );
    assert_eq!(
        call(&app, "POST", "/core/v1/teams", Some(&session.secret), None)
            .await
            .status(),
        StatusCode::CREATED
    );
    let teams = call(&app, "GET", "/core/v1/teams", Some(&session.secret), None).await;
    assert_eq!(json_body(teams).await.as_array().unwrap().len(), 1);
    let forged = json!({"owner":{"kind":"personal","id":11},"ttl_seconds":3600,"platform_level":6});
    assert_eq!(
        call(
            &app,
            "POST",
            "/core/v1/keys",
            Some(&session.secret),
            Some(forged)
        )
        .await
        .status(),
        StatusCode::UNPROCESSABLE_ENTITY
    );
    let large = json!({"padding":"x".repeat(17000)});
    assert_eq!(
        call(
            &app,
            "POST",
            "/core/v1/keys",
            Some(&session.secret),
            Some(large)
        )
        .await
        .status(),
        StatusCode::PAYLOAD_TOO_LARGE
    );
    for (method, path) in [
        ("GET", "/health/ready"),
        ("GET", "/v1/models"),
        ("POST", "/v1/responses"),
    ] {
        assert_eq!(
            call(&app, method, path, Some(token), None).await.status(),
            StatusCode::SERVICE_UNAVAILABLE
        );
    }
    let path = format!("/core/v1/credentials/{}", key["id"].as_i64().unwrap());
    assert_eq!(
        call(&app, "DELETE", &path, Some(&session.secret), None)
            .await
            .status(),
        StatusCode::NO_CONTENT
    );
    assert_eq!(
        call(&app, "GET", "/core/v1/identity", Some(token), None)
            .await
            .status(),
        StatusCode::UNAUTHORIZED
    );
}

#[sqlx::test(migrations = false)]
async fn database_failure_never_grants_access_or_calls_go(pool: PgPool) {
    let store = IdentityStore::from_pool(pool.clone());
    store.init_database().await.unwrap();
    let session = store.bootstrap_user(1, 1).await.unwrap();
    let app = http::router().merge(identity_http::router(store));
    pool.close().await;
    let response = call(
        &app,
        "GET",
        "/core/v1/identity",
        Some(&session.secret),
        None,
    )
    .await;
    assert_eq!(response.status(), StatusCode::SERVICE_UNAVAILABLE);
    assert_eq!(
        json_body(response).await,
        json!({"error":{"code":"identity_unavailable"}})
    );
    assert_eq!(
        call(&app, "GET", "/health/live", None, None).await.status(),
        StatusCode::OK
    );
    assert_eq!(
        call(&app, "POST", "/v1/responses", Some(&session.secret), None)
            .await
            .status(),
        StatusCode::SERVICE_UNAVAILABLE
    );
}

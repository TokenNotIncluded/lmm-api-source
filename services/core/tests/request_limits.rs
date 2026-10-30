//! Real PostgreSQL admission and administrator HTTP contracts.
use axum::{
    Router,
    body::{Body, to_bytes},
    http::{Request, StatusCode},
};
use lmm_core::{
    accounts::{Account, AccountKind},
    core_schema,
    funding::FundingPolicy,
    identity::IdentityStore,
    request_limits::{self, AdmissionError},
};
use serde_json::{Value, json};
use sqlx::{PgPool, Row};
use tower::ServiceExt;

async fn fixture(pool: &PgPool) -> (IdentityStore, String) {
    core_schema::init(pool).await.unwrap();
    let identity = IdentityStore::from_pool(pool.clone());
    let root = identity.bootstrap_user(1, 6).await.unwrap().secret;
    (identity, root)
}
async fn policy(pool: &PgPool, user: Option<i32>, groups: Value) {
    let revision: i64 = sqlx::query_scalar("SELECT revision FROM core_request_limits.policy")
        .fetch_one(pool)
        .await
        .unwrap();
    let result: Option<i64> =
        sqlx::query_scalar("SELECT core_request_limits.write_policy($1,TRUE,86400,$2,$3,1)")
            .bind(revision)
            .bind(user)
            .bind(groups)
            .fetch_one(pool)
            .await
            .unwrap();
    assert_eq!(result, Some(revision + 1));
}
async fn used(pool: &PgPool, scope: i16, subject: &str) -> i64 {
    sqlx::query_scalar("SELECT COALESCE(sum(used),0)::bigint FROM core_request_limits.buckets WHERE scope=$1 AND subject=$2")
        .bind(scope).bind(subject).fetch_one(pool).await.unwrap()
}
async fn call(
    app: &Router,
    method: &str,
    path: &str,
    secret: Option<&str>,
    body: Option<Value>,
) -> (StatusCode, Value) {
    let mut request = Request::builder().method(method).uri(path);
    if let Some(secret) = secret {
        request = request.header("authorization", format!("Bearer {secret}"));
    }
    let body = if let Some(value) = body {
        request = request.header("content-type", "application/json");
        Body::from(value.to_string())
    } else {
        Body::empty()
    };
    let response = app
        .clone()
        .oneshot(request.body(body).unwrap())
        .await
        .unwrap();
    assert_eq!(response.headers()["cache-control"], "private, no-store");
    let status = response.status();
    let body = to_bytes(response.into_body(), 65536).await.unwrap();
    (status, serde_json::from_slice(&body).unwrap_or(Value::Null))
}

#[sqlx::test(migrations = false)]
async fn request_limits_admin_policy_revision_and_role_boundaries(pool: PgPool) {
    let (identity, root) = fixture(&pool).await;
    let admin = identity.bootstrap_user(2, 5).await.unwrap().secret;
    let paid = identity.bootstrap_user(3, 4).await.unwrap().secret;
    let key = identity
        .issue_key(
            &root,
            Account {
                kind: AccountKind::Personal,
                id: 1,
            },
            FundingPolicy::new_personal_key(),
            3600,
        )
        .await
        .unwrap()
        .secret;
    let app = request_limits::router(pool.clone());
    let path = "/core/v1/admin/request-limits";
    let body = json!({"expected_revision":1,"policy":{"enabled":true,"window_seconds":60,"user_limit":2,"group_limits":{"default":5}}});
    for secret in [
        Some(admin.as_str()),
        Some(paid.as_str()),
        Some(key.as_str()),
        None,
    ] {
        let expected = if secret.is_some() {
            StatusCode::FORBIDDEN
        } else {
            StatusCode::UNAUTHORIZED
        };
        assert_eq!(call(&app, "GET", path, secret, None).await.0, expected);
        assert_eq!(
            call(&app, "PUT", path, secret, Some(body.clone())).await.0,
            expected
        );
    }
    let before = call(&app, "GET", path, Some(&root), None).await;
    assert_eq!(before.0, StatusCode::OK);
    assert_eq!(before.1["policy"]["enabled"], false);
    let saved = call(&app, "PUT", path, Some(&root), Some(body.clone())).await;
    assert_eq!(saved.0, StatusCode::OK);
    assert_eq!(saved.1["revision"], 2);
    assert_eq!(saved.1["policy"], body["policy"]);
    assert_eq!(
        call(&app, "PUT", path, Some(&root), Some(body.clone()))
            .await
            .0,
        StatusCode::CONFLICT
    );
    let mut invalid = body;
    invalid["expected_revision"] = json!(2);
    invalid["policy"]["window_seconds"] = json!(0);
    assert_eq!(
        call(&app, "PUT", path, Some(&root), Some(invalid)).await.0,
        StatusCode::BAD_REQUEST
    );

    let auto = "/core/v1/admin/routing/auto-groups";
    let order = json!({"expected_revision":1,"groups":["private","default","国产"]});
    assert_eq!(
        call(&app, "PUT", auto, Some(&admin), Some(order.clone()))
            .await
            .0,
        StatusCode::FORBIDDEN
    );
    let saved = call(&app, "PUT", auto, Some(&root), Some(order.clone())).await;
    assert_eq!(saved.0, StatusCode::OK);
    assert_eq!(saved.1["groups"], order["groups"]);
    assert_eq!(
        call(&app, "PUT", auto, Some(&root), Some(order)).await.0,
        StatusCode::CONFLICT
    );
    assert_eq!(
        call(
            &app,
            "PUT",
            auto,
            Some(&root),
            Some(json!({"expected_revision":2,"groups":["default","default"]}))
        )
        .await
        .0,
        StatusCode::BAD_REQUEST
    );
    assert_eq!(
        call(
            &app,
            "PUT",
            auto,
            Some(&root),
            Some(json!({"expected_revision":2,"groups":[]}))
        )
        .await
        .0,
        StatusCode::OK
    );
    let events: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM core_request_limits.policy_events WHERE actor_user_id=1",
    )
    .fetch_one(&pool)
    .await
    .unwrap();
    assert_eq!(events, 3);
}

#[sqlx::test(migrations = false)]
async fn request_limits_aggregate_keys_and_share_groups_across_connections(pool: PgPool) {
    let (identity, root) = fixture(&pool).await;
    let first = identity
        .issue_key(
            &root,
            Account {
                kind: AccountKind::Personal,
                id: 1,
            },
            FundingPolicy::new_personal_key(),
            3600,
        )
        .await
        .unwrap();
    let second = identity
        .issue_key(
            &root,
            Account {
                kind: AccountKind::Personal,
                id: 1,
            },
            FundingPolicy::new_personal_key(),
            3600,
        )
        .await
        .unwrap();
    let users = [
        identity.authorize(&first.secret).await.unwrap().user_id,
        identity.authorize(&second.secret).await.unwrap().user_id,
    ];
    assert_eq!(users, [1, 1]);
    policy(&pool, Some(5), json!({"shared":7})).await;
    let mut tasks = tokio::task::JoinSet::new();
    for n in 0..20 {
        let pool = pool.clone();
        let user = users[n % 2];
        tasks.spawn(async move {
            request_limits::admit(&pool, user, "shared", &format!("key-{}-{n}", n % 2)).await
        });
    }
    let mut allowed = 0;
    while let Some(result) = tasks.join_next().await {
        match result.unwrap() {
            Ok(()) => allowed += 1,
            Err(AdmissionError::Limited {
                retry_after_seconds,
            }) => assert!((1..=86400).contains(&retry_after_seconds)),
            other => panic!("unexpected admission {other:?}"),
        }
    }
    assert_eq!(allowed, 5);
    assert_eq!(used(&pool, 1, "1").await, 5);
    assert_eq!(used(&pool, 2, "shared").await, 5);
    let mut tasks = tokio::task::JoinSet::new();
    for user in 2..12 {
        let pool = pool.clone();
        tasks.spawn(async move {
            request_limits::admit(&pool, user, "shared", &format!("other-{user}")).await
        });
    }
    let mut allowed = 0;
    while let Some(result) = tasks.join_next().await {
        if result.unwrap().is_ok() {
            allowed += 1;
        }
    }
    assert_eq!(allowed, 2);
    assert_eq!(used(&pool, 2, "shared").await, 7);
    let total: i64 = sqlx::query_scalar(
        "SELECT sum(used)::bigint FROM core_request_limits.buckets WHERE scope=1",
    )
    .fetch_one(&pool)
    .await
    .unwrap();
    assert_eq!(total, 7, "group rejection must not consume a user slot");
}

#[sqlx::test(migrations = false)]
async fn request_limits_logical_retry_dedup_and_atomic_rejection(pool: PgPool) {
    fixture(&pool).await;
    policy(&pool, Some(2), json!({"first":1,"next":10})).await;
    request_limits::admit(&pool, 1, "first", "logical-a")
        .await
        .unwrap();
    request_limits::admit(&pool, 1, "first", "logical-a")
        .await
        .unwrap();
    request_limits::admit(&pool, 1, "next", "logical-a")
        .await
        .unwrap();
    assert_eq!(used(&pool, 1, "1").await, 1);
    assert_eq!(used(&pool, 2, "first").await, 1);
    assert_eq!(used(&pool, 2, "next").await, 1);
    assert!(matches!(
        request_limits::admit(&pool, 2, "first", "logical-a").await,
        Err(AdmissionError::Limited { .. })
    ));
    assert_eq!(
        used(&pool, 1, "2").await,
        0,
        "same ID from another user is not a replay"
    );
    request_limits::admit(&pool, 2, "next", "logical-b")
        .await
        .unwrap();
    request_limits::admit(&pool, 2, "next", "logical-c")
        .await
        .unwrap();
    assert!(matches!(
        request_limits::admit(&pool, 2, "next", "logical-d").await,
        Err(AdmissionError::Limited { .. })
    ));
    assert_eq!(
        used(&pool, 2, "next").await,
        3,
        "user rejection must not consume a group slot"
    );
}

#[sqlx::test(migrations = false)]
async fn request_limits_expired_window_disabled_and_storage_failure(pool: PgPool) {
    fixture(&pool).await;
    request_limits::admit(&pool, 1, "default", "disabled")
        .await
        .unwrap();
    assert_eq!(used(&pool, 1, "1").await, 0);
    policy(&pool, Some(1), json!({})).await;
    sqlx::query("INSERT INTO core_request_limits.buckets(scope,subject,window_seconds,window_started,used) VALUES (1,'1',86400,(floor(extract(epoch FROM clock_timestamp()))::bigint / 86400 - 1)*86400,1)")
        .execute(&pool).await.unwrap();
    request_limits::admit(&pool, 1, "default", "new-window")
        .await
        .unwrap();
    let row = sqlx::query("SELECT count(*)::bigint AS buckets,sum(used)::bigint AS used FROM core_request_limits.buckets WHERE scope=1 AND subject='1'")
        .fetch_one(&pool).await.unwrap();
    assert_eq!(row.get::<i64, _>("buckets"), 1);
    assert_eq!(row.get::<i64, _>("used"), 1);
    assert_eq!(
        request_limits::admit(&pool, 0, "default", "invalid").await,
        Err(AdmissionError::Invalid)
    );
    sqlx::query("ALTER TABLE core_request_limits.policy RENAME TO unavailable_policy")
        .execute(&pool)
        .await
        .unwrap();
    assert_eq!(
        request_limits::admit(&pool, 1, "default", "cannot-admit").await,
        Err(AdmissionError::Storage)
    );
}

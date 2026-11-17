//! Native network-policy settings with a disposable database and restricted runtime.
use axum::{
    Router,
    body::{Body, to_bytes},
    extract::ConnectInfo,
    http::{Request, StatusCode, header},
    routing::get,
};
use lmm_core::{
    core_schema,
    funding::FundingPolicy,
    identity::IdentityStore,
    network_access::{NetworkAccess, TypedConfig},
    options_http::{self, OptionsHttp, RuntimeOptions},
    runtime_database::{self, RequiredCapabilities, RuntimeDatabaseError},
};
use serde_json::{Value, json};
use sqlx::PgPool;
use std::net::SocketAddr;
use tower::ServiceExt;

#[path = "common/runtime.rs"]
mod runtime;

fn settings(pool: &PgPool) -> Router {
    options_http::router(OptionsHttp::new(
        pool.clone(),
        IdentityStore::from_pool(pool.clone()),
        RuntimeOptions {
            server_address: "https://console.example.test".into(),
            registration_enabled: false,
            password_register_enabled: false,
            oauth_register_enabled: false,
            registration_disabled_methods: vec![],
        },
    ))
}

async fn call(
    app: &Router,
    method: &str,
    path: &str,
    secret: &str,
    body: Value,
) -> (StatusCode, Value) {
    let response = app
        .clone()
        .oneshot(
            Request::builder()
                .method(method)
                .uri(path)
                .header(header::AUTHORIZATION, format!("Bearer {secret}"))
                .header(header::CONTENT_TYPE, "application/json")
                .body(Body::from(body.to_string()))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.headers()[header::CACHE_CONTROL], "no-store");
    let status = response.status();
    let bytes = to_bytes(response.into_body(), 1 << 20).await.unwrap();
    (status, serde_json::from_slice(&bytes).unwrap())
}

async fn setup(owner: &PgPool) -> (runtime::RuntimeRole, IdentityStore, String) {
    core_schema::init(owner).await.unwrap();
    let identity = IdentityStore::from_pool(owner.clone());
    let root = identity.bootstrap_user(1, 6).await.unwrap();
    let runtime = runtime::RuntimeRole::create(owner).await;
    runtime.optional("core_console").await;
    runtime_database::check(
        &runtime.pool,
        RequiredCapabilities {
            console: true,
            ..Default::default()
        },
    )
    .await
    .unwrap();
    (runtime, identity, root.secret)
}

async fn snapshot(pool: &PgPool) -> (Vec<(String, String, i64)>, Vec<Value>) {
    let options =
        sqlx::query_as("SELECT key,value,revision FROM core_console.options ORDER BY key")
            .fetch_all(pool)
            .await
            .unwrap();
    let audits = sqlx::query_scalar(
        "SELECT target FROM core_identity.audit WHERE action='console.options.update' ORDER BY id",
    )
    .fetch_all(pool)
    .await
    .unwrap();
    (options, audits)
}

async fn probe(app: &Router) -> StatusCode {
    let mut request = Request::builder()
        .uri("/network-probe")
        .body(Body::empty())
        .unwrap();
    request.extensions_mut().insert(ConnectInfo(
        "192.0.2.7:43210".parse::<SocketAddr>().unwrap(),
    ));
    app.clone().oneshot(request).await.unwrap().status()
}

#[sqlx::test(migrations = false)]
async fn network_access_restricted_runtime_validates_saves_and_roundtrips(owner: PgPool) {
    let (runtime, identity, secret) = setup(&owner).await;
    let app = settings(&runtime.pool);
    let ingress = NetworkAccess::new(Some(runtime.pool.clone()), TypedConfig::default())
        .unwrap()
        .protect(Router::new().route("/network-probe", get(|| async { StatusCode::NO_CONTENT })));
    assert_eq!(probe(&ingress).await, StatusCode::NO_CONTENT);
    let actor = identity.authorize(&secret).await.unwrap();
    let key = identity
        .issue_key(
            &secret,
            actor.owner,
            FundingPolicy::new_personal_key(),
            3600,
        )
        .await
        .unwrap();
    let admin = identity.bootstrap_user(2, 5).await.unwrap();
    let rules = "unknown-country: reject\nip(geoip:CN) -> reject\nfallback: direct";
    let before = snapshot(&owner).await;
    let (_, initial) = call(&app, "GET", "/api/option/", &secret, json!(null)).await;
    assert!(
        initial["capabilities"]["writable_keys"]
            .as_array()
            .unwrap()
            .contains(&json!("IPAccessRoutingRules"))
    );
    assert!(
        initial["data"]
            .as_array()
            .unwrap()
            .iter()
            .any(|item| item["key"] == "IPAccessRoutingRules" && item["value"] == "")
    );

    for denied in [&key.secret, &admin.secret] {
        for (method, path, payload) in [
            (
                "PUT",
                "/api/option/",
                json!({"key":"IPAccessRoutingRules","value":rules}),
            ),
            (
                "POST",
                "/api/option/validate",
                json!({"values":{"IPAccessRoutingRules":rules}}),
            ),
            (
                "POST",
                "/api/option/bulk",
                json!({"values":{"IPAccessRoutingRules":rules}}),
            ),
        ] {
            assert_eq!(
                call(&app, method, path, denied, payload).await.0,
                StatusCode::FORBIDDEN
            );
        }
    }
    for invalid in [
        "".to_owned(),
        "fallback: proxy".to_owned(),
        "ip(geoip:USA) -> reject\nfallback: direct".to_owned(),
        format!("#{}\nfallback: direct", "x".repeat(1025)),
        "fallback: direct\n".repeat(1025),
    ] {
        let (status, body) = call(
            &app,
            "POST",
            "/api/option/bulk",
            &secret,
            json!({"values":{"IPAccessRoutingRules":invalid,"Notice":"must not save"}}),
        )
        .await;
        assert_eq!(status, StatusCode::BAD_REQUEST);
        assert_eq!(body["code"], "SYSTEM_OPTIONS_INVALID");
    }
    let (status,valid) = call(&app,"POST","/api/option/validate",&secret,
        json!({"values":{"IPAccessRoutingRules":rules},"expected_values":{"IPAccessRoutingRules":""}})).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(valid["saved"], false);
    assert_eq!(snapshot(&owner).await, before);

    let payload = json!({"key":"IPAccessRoutingRules","value":rules});
    for path in ["/api/option", "/api/option/"] {
        assert_eq!(
            call(&app, "PUT", path, &secret, payload.clone()).await.0,
            StatusCode::OK
        );
    }
    let saved = snapshot(&owner).await;
    assert_eq!(
        saved.0,
        vec![("IPAccessRoutingRules".into(), rules.into(), 1)]
    );
    assert_eq!(saved.1.len(), 1);
    assert_eq!(saved.1[0]["changes"][0]["key"], "IPAccessRoutingRules");
    assert_eq!(saved.1[0]["changes"][0]["after"], json!([rules, 1]));
    // The already-created public ingress sees the committed rules on its next request.
    assert_eq!(probe(&ingress).await, StatusCode::FORBIDDEN);
    let reloaded = settings(&runtime.pool);
    let (_, options) = call(&reloaded, "GET", "/api/option/", &secret, json!(null)).await;
    assert!(
        options["data"]
            .as_array()
            .unwrap()
            .iter()
            .any(|item| item["key"] == "IPAccessRoutingRules" && item["value"] == rules)
    );
    for (country, action) in [
        (Some("CN"), "reject"),
        (Some("US"), "direct"),
        (None, "reject"),
    ] {
        let (status, preview) = call(
            &app,
            "POST",
            "/api/option/ip-access/preview",
            &secret,
            json!({"rules":rules,"ip":"192.0.2.7","country":country}),
        )
        .await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(preview["data"]["action"], action);
    }

    for statement in [
        "UPDATE core_console.options SET value='fallback: reject'",
        "INSERT INTO core_console.options(key,value) VALUES ('OAuthSecret','denied')",
        "DELETE FROM core_console.options",
        "TRUNCATE core_console.options",
        "SELECT * FROM core_meta.network_access_upgrades",
    ] {
        let error = sqlx::query(statement)
            .execute(&runtime.pool)
            .await
            .unwrap_err();
        assert_eq!(
            error.as_database_error().unwrap().code().as_deref(),
            Some("42501")
        );
    }
    for statement in [
        "SELECT core_console.write_public_option('IPAccessRoutingRules','fallback: direct')",
        "SELECT core_console.write_ip_access_rules('')",
        "SELECT core_console.write_ip_access_rules(repeat('x',16385))",
    ] {
        let error = sqlx::query(statement)
            .execute(&runtime.pool)
            .await
            .unwrap_err();
        assert_eq!(
            error.as_database_error().unwrap().code().as_deref(),
            Some("22023")
        );
    }
    assert_eq!(snapshot(&owner).await, saved);
    sqlx::raw_sql(sqlx::AssertSqlSafe(format!(
        "REVOKE EXECUTE ON FUNCTION core_console.write_ip_access_rules(text) FROM {}",
        runtime.name
    )))
    .execute(&owner)
    .await
    .unwrap();
    assert!(matches!(
        runtime_database::check(
            &runtime.pool,
            RequiredCapabilities {
                console: true,
                ..Default::default()
            }
        )
        .await,
        Err(RuntimeDatabaseError::MissingPrivileges)
    ));
    runtime.cleanup().await;
}

#[sqlx::test(migrations = false)]
async fn network_access_bulk_cas_is_atomic_and_audit_failure_rolls_back(owner: PgPool) {
    let (runtime, identity, secret) = setup(&owner).await;
    let second = identity.bootstrap_user(2, 6).await.unwrap();
    let app = settings(&runtime.pool);
    let attempt_a = call(
        &app,
        "POST",
        "/api/option/bulk",
        &secret,
        json!({"values":{"IPAccessRoutingRules":"fallback: reject","Notice":"first"},
            "expected_values":{"IPAccessRoutingRules":"","Notice":""}}),
    );
    let attempt_b = call(
        &app,
        "POST",
        "/api/option/bulk",
        &second.secret,
        json!({"values":{"IPAccessRoutingRules":"fallback: direct","Notice":"second"},
            "expected_values":{"IPAccessRoutingRules":"","Notice":""}}),
    );
    let (a, b) = tokio::join!(attempt_a, attempt_b);
    let mut statuses = [a.0.as_u16(), b.0.as_u16()];
    statuses.sort();
    assert_eq!(statuses, [200, 409]);
    let saved = snapshot(&owner).await;
    assert_eq!(saved.0.len(), 2);
    assert!(saved.0.iter().all(|row| row.2 == 1));
    assert_eq!(saved.1.len(), 1);
    assert_eq!(saved.1[0]["changes"].as_array().unwrap().len(), 2);
    let policy = &saved
        .0
        .iter()
        .find(|row| row.0 == "IPAccessRoutingRules")
        .unwrap()
        .1;
    let notice = &saved.0.iter().find(|row| row.0 == "Notice").unwrap().1;
    assert!(
        policy == "fallback: reject" && notice == "first"
            || policy == "fallback: direct" && notice == "second"
    );
    let (status, body) = call(
        &app,
        "POST",
        "/api/option/bulk",
        &secret,
        json!({"values":{"IPAccessRoutingRules":"fallback: direct","Notice":"stale"},
            "expected_values":{"IPAccessRoutingRules":""}}),
    )
    .await;
    assert_eq!(status, StatusCode::CONFLICT);
    assert_eq!(body["code"], "SYSTEM_OPTIONS_CONFLICT");
    assert_eq!(snapshot(&owner).await, saved);

    sqlx::query("ALTER TABLE core_identity.audit ADD CONSTRAINT reject_network_audit CHECK (action <> 'console.options.update') NOT VALID")
        .execute(&owner).await.unwrap();
    let changed = "ip(192.0.2.0/24) -> direct\nfallback: reject";
    assert_eq!(
        call(
            &app,
            "POST",
            "/api/option/bulk",
            &secret,
            json!({"values":{"IPAccessRoutingRules":changed,"Notice":"roll back"},
            "expected_values":{"IPAccessRoutingRules":policy,"Notice":notice}})
        )
        .await
        .0,
        StatusCode::SERVICE_UNAVAILABLE
    );
    assert_eq!(snapshot(&owner).await, saved);
    runtime.cleanup().await;
}

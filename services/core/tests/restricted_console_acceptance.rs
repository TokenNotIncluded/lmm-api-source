//! Synthetic fresh-install acceptance with a restricted runtime role.
//! Run via scripts/test-console-runtime.sh; never use an existing database.
use std::{env, path::PathBuf, process::Command};

use lmm_core::{
    core_schema,
    funding::FundingPolicy,
    identity::IdentityStore,
    options_http::{self, OptionsHttp, RuntimeOptions},
    runtime_database::{self, RequiredCapabilities},
};
use serde_json::{Value, json};
use sqlx::{
    PgPool,
    postgres::{PgConnectOptions, PgPoolOptions},
};

async fn pool(socket: &str, port: u16, user: &str, role: bool) -> PgPool {
    PgPoolOptions::new()
        .max_connections(6)
        .after_connect(move |connection, _| {
            Box::pin(async move {
                if role {
                    sqlx::query("SET ROLE lmm_core_owner")
                        .execute(connection)
                        .await?;
                }
                Ok(())
            })
        })
        .connect_with(
            PgConnectOptions::new_without_pgpass()
                .socket(socket)
                .port(port)
                .username(user)
                .database("lmm_acceptance"),
        )
        .await
        .expect("connect to private fixture")
}

async fn snapshot(
    owner: &PgPool,
) -> (
    Vec<(String, String, i64, String)>,
    Vec<(i64, i64, Option<i64>, Value)>,
) {
    let options = sqlx::query_as(
        "SELECT key,value,revision,updated_at::text FROM core_console.options ORDER BY key",
    )
    .fetch_all(owner)
    .await
    .unwrap();
    let audit = sqlx::query_as("SELECT id,actor_user_id,credential_id,target FROM core_identity.audit WHERE action='console.options.update' ORDER BY id")
        .fetch_all(owner).await.unwrap();
    (options, audit)
}

async fn request(
    client: &reqwest::Client,
    base: &str,
    method: reqwest::Method,
    path: &str,
    token: Option<&str>,
    body: Value,
) -> (u16, Value) {
    let mut req = client.request(method, format!("{base}{path}")).json(&body);
    if let Some(token) = token {
        req = req.bearer_auth(token);
    }
    let response = req.send().await.expect("actual TCP request");
    assert_eq!(response.headers().get("cache-control").unwrap(), "no-store");
    let status = response.status().as_u16();
    (status, response.json().await.expect("JSON response"))
}

async fn denied_sql(pool: &PgPool, sql: &str, expected: &str) {
    let err = sqlx::query(sql)
        .execute(pool)
        .await
        .expect_err("statement must be denied");
    let code = err.as_database_error().and_then(|e| e.code());
    assert_eq!(
        code.as_deref(),
        Some(expected),
        "unexpected SQLSTATE for {sql}"
    );
}

#[tokio::test]
#[ignore = "requires the isolated PostgreSQL fixture created by scripts/test-console-runtime.sh"]
async fn restricted_console_over_tcp() {
    let fixture = PathBuf::from(env::var("ACCEPT_FIXTURE").expect("ACCEPT_FIXTURE"));
    let fixture = fixture.canonicalize().expect("fixture directory");
    let socket = fixture.join("socket");
    let socket = socket.to_str().expect("UTF-8 socket path");
    let port: u16 = env::var("ACCEPT_PORT").unwrap().parse().unwrap();
    let expected_system_id = env::var("ACCEPT_SYSTEM_ID").unwrap();
    let admin = pool(socket, port, "lmm_fixture_admin", false).await;
    let (version, data_dir, system_id): (i32, String, String) = sqlx::query_as(
        "SELECT current_setting('server_version_num')::int, current_setting('data_directory'), system_identifier::text FROM pg_control_system()")
        .fetch_one(&admin).await.unwrap();
    assert!((180000..190000).contains(&version));
    assert_eq!(
        PathBuf::from(data_dir).canonicalize().unwrap(),
        fixture.join("data").canonicalize().unwrap()
    );
    assert_eq!(system_id, expected_system_id);
    let admin_identity: (String, String) = sqlx::query_as("SELECT current_database(),current_user")
        .fetch_one(&admin)
        .await
        .unwrap();
    assert_eq!(
        admin_identity,
        ("lmm_acceptance".into(), "lmm_fixture_admin".into())
    );

    let owner = pool(socket, port, "lmm_installer", true).await;
    let ids: (String, String) = sqlx::query_as("SELECT session_user,current_user")
        .fetch_one(&owner)
        .await
        .unwrap();
    assert_eq!(ids, ("lmm_installer".into(), "lmm_core_owner".into()));
    core_schema::init(&owner)
        .await
        .expect("actual fresh core init");
    core_schema::check(&owner)
        .await
        .expect("fresh catalog contract");
    let owner_identity = IdentityStore::from_pool(owner.clone());
    let root = owner_identity.bootstrap_user(1, 6).await.unwrap();
    let admin_user = owner_identity.bootstrap_user(2, 5).await.unwrap();
    let actor = owner_identity.authorize(&root.secret).await.unwrap();
    let api_key = owner_identity
        .issue_key(
            &root.secret,
            actor.owner,
            FundingPolicy::new_personal_key(),
            3600,
        )
        .await
        .unwrap();
    sqlx::query(
        "INSERT INTO core_console.options(key,value) VALUES ('OAuthSecret','synthetic-hidden')",
    )
    .execute(&owner)
    .await
    .unwrap();

    let status = Command::new(env::var_os("ACCEPT_PSQL").expect("absolute psql path"))
        .env_clear()
        .env("PGOPTIONS", "-c role=lmm_core_owner")
        .arg("-X")
        .arg("-h")
        .arg(socket)
        .arg("-p")
        .arg(port.to_string())
        .arg("-U")
        .arg("lmm_installer")
        .arg("-d")
        .arg("lmm_acceptance")
        .arg("-v")
        .arg("ON_ERROR_STOP=1")
        .arg("-v")
        .arg("runtime_role=lmm_core_runtime")
        .arg("-v")
        .arg("enable_console=true")
        .arg("-f")
        .arg(env::var_os("ACCEPT_GRANT_SQL").expect("frozen official grant script"))
        .status()
        .expect("run official grant script");
    assert!(status.success(), "official grant script failed");
    let runtime = pool(socket, port, "lmm_core_runtime", false).await;
    let ids: (String, String) = sqlx::query_as("SELECT session_user,current_user")
        .fetch_one(&runtime)
        .await
        .unwrap();
    assert_eq!(ids, ("lmm_core_runtime".into(), "lmm_core_runtime".into()));
    runtime_database::check(
        &runtime,
        RequiredCapabilities {
            console: true,
            ..Default::default()
        },
    )
    .await
    .expect("restricted startup authority");

    for sql in [
        "INSERT INTO core_console.options(key,value) VALUES ('Notice','denied')",
        "UPDATE core_console.options SET value='denied' WHERE key='OAuthSecret'",
        "DELETE FROM core_console.options WHERE key='OAuthSecret'",
        "TRUNCATE core_console.options",
        "CREATE TABLE core_console.denied(x int)",
    ] {
        denied_sql(&runtime, sql, "42501").await;
    }
    let before = snapshot(&owner).await;
    denied_sql(
        &runtime,
        "SELECT core_console.write_public_option('OAuthSecret','denied')",
        "22023",
    )
    .await;
    assert_eq!(snapshot(&owner).await, before);

    let app = options_http::router(OptionsHttp::new(
        runtime.clone(),
        IdentityStore::from_pool(runtime.clone()),
        RuntimeOptions {
            server_address: "https://console.example.test".into(),
            registration_enabled: false,
            password_register_enabled: false,
            oauth_register_enabled: false,
            registration_disabled_methods: vec![],
        },
    ));
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let base = format!("http://{}", listener.local_addr().unwrap());
    let server = tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
    let client = reqwest::Client::builder().no_proxy().build().unwrap();
    for token in [&admin_user.secret, &api_key.secret] {
        let (status, _) = request(
            &client,
            &base,
            reqwest::Method::PUT,
            "/api/option",
            Some(token),
            json!({"key":"Notice","value":"denied"}),
        )
        .await;
        assert_eq!(status, 403);
    }
    assert_eq!(
        request(
            &client,
            &base,
            reqwest::Method::GET,
            "/api/option",
            None,
            json!(null)
        )
        .await
        .0,
        401
    );
    assert_eq!(
        request(
            &client,
            &base,
            reqwest::Method::PUT,
            "/api/option",
            Some(&root.secret),
            json!({"key":"OAuthSecret","value":"denied"})
        )
        .await
        .0,
        409
    );
    assert_eq!(snapshot(&owner).await, before);

    let (status, result) = request(
        &client,
        &base,
        reqwest::Method::PUT,
        "/api/option/",
        Some(&root.secret),
        json!({"key":"Notice","value":"hello"}),
    )
    .await;
    assert_eq!(status, 200);
    assert_eq!(result["saved"], true);
    let saved = snapshot(&owner).await;
    assert!(
        saved
            .0
            .iter()
            .any(|r| r.0 == "Notice" && r.1 == "hello" && r.2 == 1)
    );
    assert_eq!(saved.1.len(), 1);
    assert_eq!(saved.1[0].1, 1);
    assert_eq!(saved.1[0].2, Some(root.id));
    assert_eq!(saved.1[0].3["changes"][0]["key"], "Notice");
    assert_eq!(saved.1[0].3["changes"][0]["after"], json!(["hello", 1]));
    assert_eq!(
        request(
            &client,
            &base,
            reqwest::Method::PUT,
            "/api/option",
            Some(&root.secret),
            json!({"key":"Notice","value":"hello"})
        )
        .await
        .0,
        200
    );
    assert_eq!(
        snapshot(&owner).await,
        saved,
        "no-op changed revision, timestamp or audit"
    );
    let announcements =
        json!([{"id":1,"content":"synthetic","publishDate":"2026-10-27T00:00:00Z"}]).to_string();
    assert_eq!(
        request(
            &client,
            &base,
            reqwest::Method::PUT,
            "/api/option",
            Some(&root.secret),
            json!({"key":"Announcements","value":announcements})
        )
        .await
        .0,
        200
    );
    let aliases = snapshot(&owner).await;
    for key in ["Announcements", "console_setting.announcements"] {
        assert!(
            aliases
                .0
                .iter()
                .any(|r| r.0 == key && r.1 == announcements && r.2 == 1)
        );
    }
    for path in ["/api/option", "/api/option/"] {
        let (status, body) = request(
            &client,
            &base,
            reqwest::Method::GET,
            path,
            Some(&root.secret),
            json!(null),
        )
        .await;
        assert_eq!(status, 200);
        assert_eq!(body["capabilities"]["read_only"], false);
        let data = body["data"].as_array().unwrap();
        assert!(
            data.iter()
                .any(|v| v["key"] == "Notice" && v["value"] == "hello")
        );
        for key in ["Announcements", "console_setting.announcements"] {
            assert!(
                data.iter()
                    .any(|v| v["key"] == key && v["value"] == announcements)
            );
        }
        assert!(!body.to_string().contains("synthetic-hidden"));
    }
    assert_eq!(
        request(
            &client,
            &base,
            reqwest::Method::POST,
            "/api/option/validate",
            Some(&root.secret),
            json!({"values":{"Notice":"validate-only"}})
        )
        .await
        .0,
        200
    );
    assert_eq!(snapshot(&owner).await, aliases);
    assert_eq!(
        request(
            &client,
            &base,
            reqwest::Method::POST,
            "/api/option/bulk",
            Some(&root.secret),
            json!({"values":{"Notice":"partial","OAuthSecret":"forbidden"}})
        )
        .await
        .0,
        409
    );
    assert_eq!(snapshot(&owner).await, aliases);
    assert_eq!(
        request(
            &client,
            &base,
            reqwest::Method::POST,
            "/api/option/bulk",
            Some(&root.secret),
            json!({"values":{"Notice":"stale"},"expected_values":{"Notice":"wrong"}})
        )
        .await
        .0,
        409
    );
    assert_eq!(snapshot(&owner).await, aliases);

    // Competing compare-and-save requests start from the same observed value.
    let attempt_a = request(
        &client,
        &base,
        reqwest::Method::POST,
        "/api/option/bulk",
        Some(&root.secret),
        json!({"values":{"Notice":"race-a"},"expected_values":{"Notice":"hello"}}),
    );
    let attempt_b = request(
        &client,
        &base,
        reqwest::Method::POST,
        "/api/option/bulk",
        Some(&root.secret),
        json!({"values":{"Notice":"race-b"},"expected_values":{"Notice":"hello"}}),
    );
    let (a, b) = tokio::join!(attempt_a, attempt_b);
    let mut statuses = [a.0, b.0];
    statuses.sort();
    assert_eq!(statuses, [200, 409]);
    let after_race = snapshot(&owner).await;
    assert_eq!(after_race.1.len(), aliases.1.len() + 1);
    assert!(
        after_race
            .0
            .iter()
            .any(|r| r.0 == "Notice" && (r.1 == "race-a" || r.1 == "race-b") && r.2 == 2)
    );

    // Inject failure at the audit boundary after option writes. Revoke is fixture-only.
    sqlx::query("REVOKE INSERT ON core_identity.audit FROM lmm_core_runtime")
        .execute(&owner)
        .await
        .unwrap();
    let status = request(
        &client,
        &base,
        reqwest::Method::POST,
        "/api/option/bulk",
        Some(&root.secret),
        json!({"values":{"Notice":"rollback", "Announcements":"[]"}}),
    )
    .await
    .0;
    assert_eq!(status, 503);
    assert_eq!(
        snapshot(&owner).await,
        after_race,
        "failed audit must roll back all option rows"
    );
    sqlx::query("GRANT INSERT ON core_identity.audit TO lmm_core_runtime")
        .execute(&owner)
        .await
        .unwrap();
    runtime_database::check(
        &runtime,
        RequiredCapabilities {
            console: true,
            ..Default::default()
        },
    )
    .await
    .unwrap();
    server.abort();
    runtime.close().await;
    owner.close().await;
    admin.close().await;
}

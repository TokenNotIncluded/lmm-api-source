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

async fn denied_sql(pool: &PgPool, sql: &'static str, expected: &str) {
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
    // Exercise the real file reader and its HTTP projection, including precedence
    // over an invalid legacy database rate. This fixture never opens the file URL.
    let db_reference = fixture.join("unused-database-url");
    std::fs::write(&db_reference, "postgres://127.0.0.1:1/unused").unwrap();
    let config_path = fixture.join("core.toml");
    std::fs::write(&config_path, format!(
        "version=1\n[database]\nurl_file='{}'\n[console]\nenabled=true\nnotice='From file'\nusd_exchange_rate='7.2'\n",
        db_reference.display()
    )).unwrap();
    lmm_core::config::initialize(Some(&config_path)).unwrap();
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
        "INSERT INTO core_console.options(key,value) VALUES ('OAuthSecret','synthetic-hidden'),('Notice','Legacy notice'),('USDExchangeRate','invalid')",
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
    // Navigation was an optional predecessor component; fresh installations may
    // omit its setter. If installed, it must still refuse unrelated options.
    let navigation_setter: bool = sqlx::query_scalar(
        "SELECT to_regprocedure('core_console.write_navigation_option(text,text)') IS NOT NULL",
    )
    .fetch_one(&runtime)
    .await
    .unwrap();
    if navigation_setter {
        denied_sql(
            &runtime,
            "SELECT core_console.write_navigation_option('OAuthSecret','denied')",
            "22023",
        )
        .await;
    }
    denied_sql(
        &runtime,
        "SELECT core_console.write_public_option('HeaderNavModules','{}')",
        "22023",
    )
    .await;
    assert_eq!(snapshot(&owner).await, before);

    let app = options_http::router(OptionsHttp::new(
        runtime.clone(),
        IdentityStore::from_pool(runtime.clone()),
        RuntimeOptions {
            server_address: "https://console.example.test".into(),
            password_login_enabled: false,
            email_verification_enabled: false,
            passkey_login_enabled: false,
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
    for token in [
        None,
        Some(admin_user.secret.as_str()),
        Some(api_key.secret.as_str()),
        Some(root.secret.as_str()),
    ] {
        for (method, path, payload) in [
            (
                reqwest::Method::PUT,
                "/api/option",
                json!({"key":"Notice","value":"changed"}),
            ),
            (
                reqwest::Method::PUT,
                "/api/option/",
                json!({"key":"OAuthSecret","value":"changed"}),
            ),
            (
                reqwest::Method::POST,
                "/api/option/bulk",
                json!({"values":{"Notice":"changed","HeaderNavModules":"{}","USDExchangeRate":"7.2"}}),
            ),
            (
                reqwest::Method::POST,
                "/api/option/validate",
                json!({"values":{"Notice":"changed"}}),
            ),
        ] {
            let (status, body) = request(&client, &base, method, path, token, payload).await;
            assert_eq!(status, 405, "{path}");
            assert_eq!(body["code"], "SYSTEM_OPTIONS_READ_ONLY");
        }
    }
    for (token, expected) in [
        (None, 401),
        (Some(admin_user.secret.as_str()), 403),
        (Some(api_key.secret.as_str()), 403),
        (Some(root.secret.as_str()), 200),
    ] {
        for path in ["/api/option", "/api/option/"] {
            let (status, body) = request(
                &client,
                &base,
                reqwest::Method::GET,
                path,
                token,
                json!(null),
            )
            .await;
            assert_eq!(status, expected);
            if status == 200 {
                assert_eq!(body["capabilities"]["read_only"], true);
                assert_eq!(body["capabilities"]["writable_keys"], json!([]));
                let data = body["data"].as_array().unwrap();
                for (key, value) in [("Notice", "From file"), ("USDExchangeRate", "7.2")] {
                    assert!(
                        data.iter()
                            .any(|item| item["key"] == key && item["value"] == value)
                    );
                }
                assert!(!body.to_string().contains("synthetic-hidden"));
                assert!(!body.to_string().contains("unused-database-url"));
            }
        }
    }
    assert_eq!(snapshot(&owner).await, before);
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

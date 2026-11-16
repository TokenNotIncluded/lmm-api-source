use super::*;
use axum::{
    body::Body,
    http::{Request, StatusCode},
};
use lmm_core::http::{Capabilities, configured_router};
use sqlx::postgres::PgPoolOptions;
use std::{
    fs,
    os::unix::fs::PermissionsExt,
    process::{Command, Stdio},
};
use tower::ServiceExt;

fn core_binary() -> std::path::PathBuf {
    use sha2::{Digest, Sha256};
    use std::path::PathBuf;

    let explicit = std::env::var_os("LMM_CHECKIN_CORE_BIN");
    let path = explicit
        .clone()
        .or_else(|| std::env::var_os("CARGO_BIN_EXE_lmm-core"))
        .or_else(|| option_env!("CARGO_BIN_EXE_lmm-core").map(Into::into))
        .map(PathBuf::from)
        .unwrap_or_else(|| {
            // Cargo may place unit tests under either deps/ or build/.../out/.
            // Search only ancestors of this Cargo artifact, never PATH.
            let executable = std::env::current_exe().unwrap();
            executable
                .ancestors()
                .skip(1)
                .map(|directory| {
                    directory.join(format!("lmm-core{}", std::env::consts::EXE_SUFFIX))
                })
                .find(|candidate| candidate.is_file())
                .expect("build lmm-core first or supply LMM_CHECKIN_CORE_BIN and its SHA256")
        });
    assert!(path.is_absolute() && path.is_file());
    assert_eq!(
        path.canonicalize().unwrap(),
        path,
        "Core binary must not use a symlink"
    );
    if explicit.is_some() {
        let expected = std::env::var("LMM_CHECKIN_CORE_SHA256")
            .expect("an explicitly selected Core binary requires its frozen SHA256");
        assert_eq!(expected.len(), 64);
        assert!(
            expected
                .bytes()
                .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
        );
        assert_eq!(
            crate::hex_codec::encode(Sha256::digest(fs::read(&path).unwrap())),
            expected,
            "frozen Core binary changed before spawn"
        );
    }
    let output = Command::new(&path)
        .arg("schema-manifest")
        .env_clear()
        .stdin(Stdio::null())
        .output()
        .unwrap();
    assert!(output.status.success() && output.stderr.is_empty());
    let mut expected = serde_json::to_vec(&lmm_core::core_schema::schema_manifest()).unwrap();
    expected.push(b'\n');
    assert_eq!(
        output.stdout, expected,
        "selected Core binary has a different schema contract"
    );
    path
}

async fn get(app: &axum::Router, path: &str, secret: Option<&str>) -> (StatusCode, Value) {
    let mut request = Request::builder().uri(path);
    if let Some(secret) = secret {
        request = request.header("authorization", format!("Bearer {secret}"));
    }
    let response = app
        .clone()
        .oneshot(request.body(Body::empty()).unwrap())
        .await
        .unwrap();
    let status = response.status();
    assert!(
        response.headers()["cache-control"]
            .to_str()
            .unwrap()
            .contains("no-store")
    );
    let bytes = axum::body::to_bytes(response.into_body(), 65_536)
        .await
        .unwrap();
    (status, serde_json::from_slice(&bytes).unwrap())
}

#[sqlx::test(migrations = false)]
async fn fresh_binary_refuses_historical_owner_commands_without_mutating_policy(pool: PgPool) {
    let suffix = format!("{:016x}", rand::random::<u64>());
    let owner = format!("checkin_owner_{suffix}");
    let installer = format!("checkin_installer_{suffix}");
    let password = format!("synthetic-{:032x}", rand::random::<u128>());
    let database = pool.connect_options().get_database().unwrap().to_owned();
    assert!(
        database
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'_')
    );
    sqlx::raw_sql(sqlx::AssertSqlSafe(format!("CREATE ROLE {owner} NOLOGIN; CREATE ROLE {installer} LOGIN NOINHERIT PASSWORD '{password}'; GRANT {owner} TO {installer}; ALTER DATABASE \"{database}\" OWNER TO {owner};")))
        .execute(&pool).await.unwrap();
    let role = format!("SET ROLE {owner}");
    let owner_pool = PgPoolOptions::new()
        .max_connections(3)
        .after_connect(move |c, _| {
            let role = role.clone();
            Box::pin(async move {
                sqlx::query(sqlx::AssertSqlSafe::<&str>(&role))
                    .execute(c)
                    .await?;
                Ok(())
            })
        })
        .connect_with(
            pool.connect_options()
                .as_ref()
                .clone()
                .username(&installer)
                .password(&password),
        )
        .await
        .unwrap();
    let f = fixture(&owner_pool, 1, 1, false).await;
    let app = configured_router(
        owner_pool.clone(),
        Capabilities {
            identity: true,
            checkins: true,
            ..Capabilities::default()
        },
    );
    let (status, body) = get(&app, "/api/status", None).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body["data"]["checkin_enabled"], true);
    let (status, body) = get(&app, "/health/ready", None).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body["checkin_enabled"], true);
    let (status, _) = get(&app, "/api/user/checkin", Some(&f.user)).await;
    assert_eq!(status, StatusCode::OK);
    let (status, _) = get(&app, "/api/user/checkin", None).await;
    assert_eq!(status, StatusCode::UNAUTHORIZED);

    let dir = std::env::temp_dir().join(format!("lmm-checkin-owner-{suffix}"));
    fs::create_dir(&dir).unwrap();
    fs::set_permissions(&dir, fs::Permissions::from_mode(0o700)).unwrap();
    let mut url = reqwest::Url::parse(&std::env::var("DATABASE_URL").unwrap()).unwrap();
    url.set_path(&format!("/{database}"));
    url.set_username(&installer).unwrap();
    url.set_password(Some(&password)).unwrap();
    url.query_pairs_mut()
        .append_pair("options", &format!("-c role={owner}"));
    let url_file = dir.join("database-url");
    fs::write(&url_file, url.as_str()).unwrap();
    fs::set_permissions(&url_file, fs::Permissions::from_mode(0o600)).unwrap();
    let config = dir.join("owner.toml");
    fs::write(
        &config,
        format!(
            "version=1\n[database]\nurl_file = {}\n",
            serde_json::to_string(url_file.to_str().unwrap()).unwrap()
        ),
    )
    .unwrap();
    fs::set_permissions(&config, fs::Permissions::from_mode(0o600)).unwrap();
    let binary = core_binary();
    let invoke = |command: &str, input: &Value| {
        let mut child = Command::new(&binary)
            .args(["--config", config.to_str().unwrap(), command])
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .unwrap();
        use std::io::Write;
        child
            .stdin
            .take()
            .unwrap()
            .write_all(&serde_json::to_vec(input).unwrap())
            .unwrap();
        let output = child.wait_with_output().unwrap();
        assert!(!output.status.success(), "fresh binary accepted {command}");
        assert!(output.stdout.is_empty());
        let error = String::from_utf8_lossy(&output.stderr);
        assert!(
            error.contains("Usage: lmm-core"),
            "unexpected rejection: {error}"
        );
    };
    let before = journals(&owner_pool).await;
    invoke(
        "seed-checkin-history",
        &serde_json::to_value(&f.input).unwrap(),
    );
    let Json(settings): Json<BTreeMap<String, String>> =
        sqlx::query_scalar("SELECT settings FROM core_checkins.seed_receipts WHERE seed_id=$1")
            .bind(&f.seed.seed_id)
            .fetch_one(&owner_pool)
            .await
            .unwrap();
    let mut settings = settings;
    settings.insert("checkin_setting.enabled".into(), "false".into());
    let mut policy = source_policy(1, 1, &f.seed);
    policy.enabled = false;
    policy.source_options_sha256 = sha(&serde_json::to_vec(&settings).unwrap());
    let request = json!({"operation_key":"runtime_disable_policy_0002","expected_revision":1,"policy":policy,"source_options":settings});
    invoke("publish-checkin-policy", &request);
    let (status, body) = get(&app, "/api/status", None).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body["data"]["checkin_enabled"], true);
    let revision: i64 =
        sqlx::query_scalar("SELECT revision FROM core_checkins.active_policy WHERE singleton")
            .fetch_one(&owner_pool)
            .await
            .unwrap();
    assert_eq!(revision, 1);
    assert_eq!(journals(&owner_pool).await, before);
    owner_pool.close().await;
    fs::remove_dir_all(dir).unwrap();
}

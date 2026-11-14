//! Restricted HTTP fixture for real browser authorization acceptance.
//! Runs only when explicitly selected with an owned database and isolated network namespace.
use crate::{
    http::{Capabilities, PublicStatus},
    identity::IdentityStore,
    legacy_auth::LegacyAuthConfig,
};
use serde::Deserialize;
use serde_json::json;
use std::{
    fs::{self, OpenOptions},
    io::Write,
    net::SocketAddr,
    os::unix::fs::{OpenOptionsExt, PermissionsExt},
    path::{Path, PathBuf},
    time::Duration,
};

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Input {
    nonce: String,
    output: PathBuf,
    runtime_url_file: PathBuf,
    expected_database: String,
    expected_socket_dir: PathBuf,
    listen: SocketAddr,
    origin: String,
    refresh_key_file: PathBuf,
    provider_key_file: PathBuf,
    replay_key_file: PathBuf,
    // None exercises the default unmodified production trust boundary.
    fixture_ca_pem_file: Option<PathBuf>,
    expected_network_namespace: String,
    timeout_seconds: u64,
}
fn private(path: &Path) -> Vec<u8> {
    let meta = fs::symlink_metadata(path).expect("private input metadata");
    assert!(
        meta.is_file() && !meta.file_type().is_symlink() && meta.permissions().mode() & 0o077 == 0
    );
    fs::read(path).expect("private input read")
}
fn key(path: &Path) -> Vec<u8> {
    let b = private(path);
    assert_eq!(b.len(), 32);
    b
}
fn receipt(path: &Path, v: serde_json::Value) {
    let mut f = OpenOptions::new()
        .create_new(true)
        .write(true)
        .mode(0o600)
        .open(path)
        .expect("exclusive receipt");
    f.write_all(serde_json::to_string_pretty(&v).unwrap().as_bytes())
        .unwrap();
    f.sync_all().unwrap();
}
#[tokio::test]
#[ignore = "independent frozen Auth HTTP worker; fresh owned PG and network namespace only"]
async fn independent_auth_http_worker() {
    let input =
        PathBuf::from(std::env::var("LMM_AUTH_HTTP_WORKER_INPUT").expect("private worker input"));
    let cfg: Input = serde_json::from_slice(&private(&input)).expect("worker config");
    assert!(
        (16..=64).contains(&cfg.nonce.len()) && cfg.nonce.bytes().all(|b| b.is_ascii_hexdigit())
    );
    assert!((30..=1800).contains(&cfg.timeout_seconds));
    assert!(cfg.listen.ip().is_loopback());
    assert!(cfg.listen.port() > 1024);
    assert_eq!(cfg.expected_database, format!("auth01_{}", cfg.nonce));
    assert_eq!(
        fs::read_link("/proc/self/ns/net")
            .unwrap()
            .to_str()
            .unwrap(),
        cfg.expected_network_namespace
    );
    let routes = std::process::Command::new("ip")
        .args(["route", "show", "default"])
        .output()
        .unwrap();
    assert!(routes.status.success() && routes.stdout.is_empty());
    let routes6 = std::process::Command::new("ip")
        .args(["-6", "route", "show", "default"])
        .output()
        .unwrap();
    assert!(routes6.status.success() && routes6.stdout.is_empty());
    let meta = fs::symlink_metadata(&cfg.output).unwrap();
    assert!(meta.is_dir() && meta.permissions().mode() & 0o077 == 0);
    let tag = format!("worker-{}", std::process::id());
    let ready = cfg.output.join(format!("{tag}.ready.json"));
    let stop = cfg.output.join(format!("{tag}.stop"));
    assert!(!stop.exists() && !ready.exists());
    let raw = private(&cfg.runtime_url_file);
    let url = std::str::from_utf8(&raw).unwrap().trim();
    // Outer runner must bind this DSN to its newly created owned cluster before launch.
    let parsed = reqwest::Url::parse(url).expect("runtime URL");
    assert!(matches!(parsed.scheme(), "postgres" | "postgresql"));
    assert_eq!(parsed.path(), format!("/{}", cfg.expected_database));
    assert!(matches!(
        parsed.host_str(),
        Some("127.0.0.1" | "localhost" | "::1")
    ));
    assert!(
        cfg.expected_socket_dir.is_absolute() && cfg.expected_socket_dir.starts_with(&cfg.output)
    );
    let sockets: Vec<_> = parsed
        .query_pairs()
        .filter(|(k, _)| k == "host")
        .map(|(_, v)| v.into_owned())
        .collect();
    assert_eq!(
        sockets,
        vec![cfg.expected_socket_dir.to_str().unwrap().to_owned()]
    );
    let socket_meta = fs::symlink_metadata(&cfg.expected_socket_dir).unwrap();
    assert!(
        socket_meta.is_dir()
            && !socket_meta.file_type().is_symlink()
            && socket_meta.permissions().mode() & 0o077 == 0
    );
    let pool = sqlx::postgres::PgPoolOptions::new()
        .max_connections(4)
        .connect(url)
        .await
        .expect("owned runtime DB");
    let db: String = sqlx::query_scalar("SELECT current_database()")
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(db, cfg.expected_database);
    let privileged:bool=sqlx::query_scalar("SELECT rolsuper OR rolcreatedb OR rolcreaterole OR rolreplication OR rolbypassrls FROM pg_roles WHERE rolname=current_user").fetch_one(&pool).await.unwrap();
    assert!(!privileged);
    crate::runtime_database::check(
        &pool,
        crate::runtime_database::RequiredCapabilities {
            web_identity: true,
            federated_auth: true,
            console: true,
            ..Default::default()
        },
    )
    .await
    .expect("restricted production runtime authority");
    crate::core_schema::check(&pool)
        .await
        .expect("preinstalled exact frozen full schema; no DDL in worker");
    let mut store = IdentityStore::from_pool(pool.clone())
        .with_registration_enabled(true)
        .with_password_register_enabled(true)
        .with_oauth_register_enabled(true)
        .with_registration_email_verification(false)
        .with_account_links(true)
        .with_financial_profile_enabled(true)
        .with_federated_keys(
            "fixture".into(),
            vec![("fixture".into(), key(&cfg.provider_key_file))],
            key(&cfg.replay_key_file),
        )
        .unwrap();
    let has_test_ca = cfg.fixture_ca_pem_file.is_some();
    if let Some(pem) = &cfg.fixture_ca_pem_file {
        store = store
            .with_federated_test_ca(
                reqwest::Certificate::from_pem(&private(pem)).expect("explicit fixture CA"),
            )
            .unwrap();
    }
    store.check_federated_auth_ready().await.unwrap();
    let mut auth = LegacyAuthConfig::new(&cfg.origin, &key(&cfg.refresh_key_file))
        .unwrap()
        .with_legal_acceptance_required(false);
    if cfg.origin.starts_with("http:") {
        auth = auth
            .with_insecure_local_http()
            .expect("only explicit local app origin, never IdP transport");
    }
    let caps = Capabilities {
        identity: true,
        legacy_web_auth: true,
        console: true,
        ..Default::default()
    };
    let status = PublicStatus {
        server_address: Some(cfg.origin.clone()),
        password_login_enabled: true,
        native_account_bindings: true,
        registration_enabled: true,
        password_register_enabled: true,
        oauth_register_enabled: true,
        federated_auth: true,
        ..Default::default()
    };
    // Public status uses production DB queries; no fake provider/API responses.
    let console = crate::console_compat::ConsoleCompat::new(pool.clone(), store.clone());
    let app =
        crate::http::configured_router_with_runtime_status(pool.clone(), caps, None, None, status)
            .merge(crate::legacy_auth::router(store, auth))
            .merge(crate::console_compat::router(console));
    let listener = tokio::net::TcpListener::bind(cfg.listen).await.unwrap();
    receipt(
        &ready,
        json!({"nonce":cfg.nonce,"pid":std::process::id(),"address":listener.local_addr().unwrap().to_string(),
        "database":db,"origin":cfg.origin,"network_namespace":cfg.expected_network_namespace,
        "explicit_test_ca":has_test_ca,"provider_local_mode":false,"console_enabled":true,"go_enabled":false,"ddl_performed":false}),
    );
    let duration = Duration::from_secs(cfg.timeout_seconds);
    let nonce = cfg.nonce.clone();
    let stop_path = stop.clone();
    let requested_stop = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
    let observed_stop = requested_stop.clone();
    axum::serve(
        listener,
        app.into_make_service_with_connect_info::<SocketAddr>(),
    )
    .with_graceful_shutdown(async move {
        let deadline = tokio::time::Instant::now() + duration;
        loop {
            if tokio::time::Instant::now() >= deadline {
                break;
            }
            if stop_path.exists() {
                assert_eq!(
                    String::from_utf8(private(&stop_path)).unwrap().trim(),
                    nonce
                );
                observed_stop.store(true, std::sync::atomic::Ordering::SeqCst);
                break;
            }
            tokio::time::sleep(Duration::from_millis(100)).await;
        }
    })
    .await
    .unwrap();
    let counts:serde_json::Value=sqlx::query_scalar("SELECT json_build_object('users',(SELECT count(*) FROM core_identity.users),'providers',(SELECT count(*) FROM core_federated_auth.providers),'identities',(SELECT count(*) FROM core_federated_auth.identities),'flows',(SELECT count(*) FROM core_federated_auth.flows))").fetch_one(&pool).await.unwrap();
    receipt(
        &cfg.output.join(format!("{tag}.stopped.json")),
        json!({"nonce":cfg.nonce,"pid":std::process::id(),"counts":counts,
            "requested_stop":requested_stop.load(std::sync::atomic::Ordering::SeqCst)}),
    );
    pool.close().await;
    assert!(
        requested_stop.load(std::sync::atomic::Ordering::SeqCst),
        "worker deadline expired without an explicit stop"
    );
}

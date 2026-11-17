use super::*;
use axum::{Router, extract::Query, routing::get};
use std::{
    collections::HashMap,
    os::unix::fs::PermissionsExt,
    sync::atomic::{AtomicUsize, Ordering},
};

struct Fixture(PathBuf);
impl Fixture {
    fn new() -> Self {
        let path =
            std::env::temp_dir().join(format!("lmm-hero-policy-{:016x}", rand::random::<u64>()));
        fs::create_dir(&path).unwrap();
        fs::set_permissions(&path, fs::Permissions::from_mode(0o700)).unwrap();
        Self(path)
    }
    fn private(&self, name: &str, value: &str) -> PathBuf {
        let path = self.0.join(name);
        fs::write(&path, value).unwrap();
        fs::set_permissions(&path, fs::Permissions::from_mode(0o600)).unwrap();
        path
    }
    fn policy(&self) -> PathBuf {
        self.private("policy.json", r#"{"revision":0,"enabled":false,"sms_enabled":false,"email_enabled":false,"price_multiplier":"1"}"#)
    }
}
impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}
fn save(revision: i64, value: &str) -> SavePolicy {
    SavePolicy {
        revision,
        enabled: false,
        sms_enabled: true,
        email_enabled: false,
        price_multiplier: value.into(),
    }
}

#[test]
fn multiplier_retains_six_decimal_places_and_rejects_invalid_input() {
    for value in ["0.000001", "2.000001", "1000", "1000.000000", "1"] {
        assert!(multiplier_valid(value), "{value}");
    }
    for value in [
        "0",
        "-1",
        "1e2",
        "1000.000001",
        ".1",
        "1.",
        "1.1234567",
        "NaN",
        "1.2.3",
        " 2",
    ] {
        assert!(!multiplier_valid(value), "{value}");
    }
}

#[tokio::test]
async fn policy_save_survives_restart_and_conflicting_writers_preserve_exact_value() {
    let fixture = Fixture::new();
    let path = fixture.policy();
    let file = PolicyFile::open(path.clone()).unwrap();
    let (a, b) = tokio::join!(
        file.save(save(0, "2.000001"), 7),
        file.save(save(0, "4.5"), 8)
    );
    assert_eq!(usize::from(a.is_ok()) + usize::from(b.is_ok()), 1);
    assert_eq!(
        a.as_ref().err().or(b.as_ref().err()),
        Some(&Error::StateConflict)
    );
    let expected = a.or(b).unwrap();
    let reopened = PolicyFile::open(path.clone()).unwrap();
    assert_eq!(reopened.read().await.unwrap(), expected);
    assert_eq!(expected.revision, 1);
    assert!(expected.updated_at.unwrap() > 0);
    assert!(matches!(
        expected.price_multiplier.as_str(),
        "2.000001" | "4.5"
    ));
    assert_eq!(
        fs::metadata(path).unwrap().permissions().mode() & 0o777,
        0o600
    );
    assert_eq!(
        file.save(save(1, "1000.000001"), 7).await,
        Err(Error::Invalid)
    );
    assert_eq!(reopened.read().await.unwrap(), expected);
}

#[tokio::test]
async fn invalid_or_unwritable_policy_never_reports_saved() {
    let fixture = Fixture::new();
    let path = fixture.policy();
    let file = PolicyFile::open(path.clone()).unwrap();
    fs::write(&path, "invalid").unwrap();
    assert_eq!(file.save(save(0, "2"), 7).await, Err(Error::Unavailable));
    assert_eq!(fs::read_to_string(path).unwrap(), "invalid");
}

#[tokio::test]
async fn connection_test_uses_read_only_balance_and_honors_key_rotation() {
    let fixture = Fixture::new();
    let policy = PolicyFile::open(fixture.policy()).unwrap();
    let credential = fixture.private("key", "synthetic-hero-key-11111111");
    let pool = sqlx::postgres::PgPoolOptions::new()
        .connect_lazy("postgres://unused@127.0.0.1:1/unused")
        .unwrap();
    let service = Service::new(pool, policy, Some(credential.clone())).unwrap();
    let calls = Arc::new(AtomicUsize::new(0));
    let count = calls.clone();
    let app = Router::new().route(
        "/balance",
        get(move |Query(query): Query<HashMap<String, String>>| {
            let count = count.clone();
            async move {
                assert_eq!(query["action"], "getBalance");
                count.fetch_add(1, Ordering::SeqCst);
                match query["api_key"].as_str() {
                    "synthetic-hero-key-11111111" => "ACCESS_BALANCE:0.000001",
                    "synthetic-hero-key-22222222" => "BAD_KEY",
                    _ => "not a balance",
                }
            }
        }),
    );
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let endpoint = format!("http://{}/balance", listener.local_addr().unwrap());
    let server = tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
    assert_eq!(service.test_connection_at(&endpoint).await, Ok(()));
    fs::write(&credential, "synthetic-hero-key-22222222").unwrap();
    assert_eq!(
        service.test_connection_at(&endpoint).await,
        Err(Error::CredentialRejected)
    );
    fs::write(&credential, "synthetic-hero-key-33333333").unwrap();
    assert_eq!(
        service.test_connection_at(&endpoint).await,
        Err(Error::Unavailable)
    );
    assert_eq!(calls.load(Ordering::SeqCst), 3);
    server.abort();
}

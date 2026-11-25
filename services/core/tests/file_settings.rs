#![cfg(unix)]
use lmm_core::config;
use std::{
    fs,
    os::unix::fs::{PermissionsExt, symlink},
    path::PathBuf,
};

struct Fixture(PathBuf);
impl Fixture {
    fn new() -> Self {
        let path = std::env::temp_dir().join(format!(
            "lmm-settings-{}-{}",
            std::process::id(),
            rand::random::<u64>()
        ));
        fs::create_dir(&path).unwrap();
        fs::set_permissions(&path, fs::Permissions::from_mode(0o700)).unwrap();
        Self(path)
    }
}
impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

// A separate integration-test executable has its own process-wide Core configuration.
#[test]
fn toml_load_drop_ins_before_installing_the_effective_startup_configuration() {
    let root = Fixture::new();
    let base = root.0.join("core.toml");
    let database = root.0.join("database-url");
    fs::write(&database, "postgres://test:test@127.0.0.1:1/unused").unwrap();
    fs::set_permissions(&database, fs::Permissions::from_mode(0o600)).unwrap();
    fs::write(&base, format!("version=1\n[server]\nworker_threads=2\n[identity]\nregistration_disabled_methods=['github','discord']\n[database]\nurl_file={database:?}\n[console]\nenabled=true\nnotice='base'\n")).unwrap();
    let dir = root.0.join("core.toml.d");
    fs::create_dir(&dir).unwrap();
    fs::write(
        dir.join("90-local.toml"),
        "[identity]\nregistration_disabled_methods=[]\n[console]\nnotice='last'\nannouncements='[]'\n",
    )
    .unwrap();
    fs::write(dir.join("10-site.toml"), "[server]\nlisten='127.0.0.1:9021'\n[console]\nnotice='first'\nusd_exchange_rate='7.25'\nip_access_routing_rules='fallback: reject'").unwrap();
    fs::write(dir.join("invalid.toml.bak"), "bad = [").unwrap();
    fs::write(dir.join(".pending.toml"), "bad = [").unwrap();
    let invalid = dir.join("99-invalid.toml");
    for fields in [
        "bad = [",
        "[console]\nunknown=true",
        "[console]\nusd_exchange_rate='nan'",
        "[console]\nannouncements='[{}]'",
        "[console]\nip_access_routing_rules='invalid'",
    ] {
        fs::write(&invalid, fields).unwrap();
        assert!(config::initialize(Some(&base)).is_err(), "{fields}");
    }
    fs::remove_file(&invalid).unwrap();
    symlink(&base, &invalid).unwrap();
    assert!(config::initialize(Some(&base)).is_err());
    fs::remove_file(invalid).unwrap();
    config::initialize(Some(&base)).unwrap();
    assert_eq!(
        config::setting("LMM_CORE_LISTEN").unwrap(),
        "127.0.0.1:9021"
    );
    assert_eq!(config::setting("LMM_CORE_WORKER_THREADS").unwrap(), "2");
    assert_eq!(
        config::setting("LMM_CORE_REGISTRATION_DISABLED_METHODS").unwrap(),
        "[]"
    );
    for (key, expected) in [
        ("Notice", "last"),
        ("Announcements", "[]"),
        ("console_setting.announcements", "[]"),
        ("USDExchangeRate", "7.25"),
        ("IPAccessRoutingRules", "fallback: reject"),
    ] {
        assert_eq!(config::console_option(key).as_deref(), Some(expected));
    }
    assert!(config::console_option("LMM_CORE_DATABASE_URL_FILE").is_none());
}

#[test]
fn toml_directory_binds_the_listener_from_the_last_fragment() {
    use std::{
        io::{Read, Write},
        net::{TcpListener, TcpStream},
        process::{Command, Stdio},
        time::{Duration, Instant},
    };
    let root = Fixture::new();
    // An attempted bind to the first address would fail while this socket is open.
    let occupied = TcpListener::bind("127.0.0.1:0").unwrap();
    let available = TcpListener::bind("127.0.0.1:0").unwrap();
    let expected = available.local_addr().unwrap();
    fs::write(
        root.0.join("00-base.toml"),
        format!(
            "version=1\n[server]\nlisten='{}'",
            occupied.local_addr().unwrap()
        ),
    )
    .unwrap();
    fs::write(
        root.0.join("90-bind.toml"),
        format!("[server]\nlisten='{expected}'"),
    )
    .unwrap();
    drop(available);
    let mut child = Command::new(env!("CARGO_BIN_EXE_lmm-core"))
        .arg("--config")
        .arg(&root.0)
        .arg("serve")
        .env_clear()
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .spawn()
        .unwrap();
    let deadline = Instant::now() + Duration::from_secs(5);
    let result = loop {
        if let Some(status) = child.try_wait().unwrap() {
            break Err(format!("Core exited: {status}"));
        }
        if let Ok(mut socket) = TcpStream::connect_timeout(&expected, Duration::from_millis(100)) {
            let response = (|| -> std::io::Result<String> {
                socket.set_read_timeout(Some(Duration::from_secs(2)))?;
                socket.write_all(
                    b"GET /health/live HTTP/1.1\r\nHost: localhost\r\nConnection: close\r\n\r\n",
                )?;
                let mut response = String::new();
                socket.read_to_string(&mut response)?;
                Ok(response)
            })();
            break response.map_err(|err| err.to_string());
        }
        if Instant::now() >= deadline {
            break Err("Core did not bind the configured listener".to_owned());
        }
        std::thread::sleep(Duration::from_millis(20));
    };
    let _ = child.kill();
    child.wait().unwrap();
    assert!(result.unwrap().starts_with("HTTP/1.1 200"));
}

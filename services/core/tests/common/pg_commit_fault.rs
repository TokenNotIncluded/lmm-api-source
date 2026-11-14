//! Actual wire ACK loss using the existing reviewed PG proxy, not fake success.
use std::{
    fs,
    io::Write,
    os::unix::fs::{OpenOptionsExt, PermissionsExt},
    path::PathBuf,
    process::{Child, Command, Stdio},
    time::Duration,
};
pub(crate) struct PgCommitFault {
    child: Child,
    pub url: String,
    events: PathBuf,
    stop: PathBuf,
    socket_dir: PathBuf,
}
impl PgCommitFault {
    pub async fn start(database_url: &str, writer: &str) -> Self {
        let url = reqwest::Url::parse(database_url).unwrap();
        assert!(matches!(url.host_str(), Some("127.0.0.1" | "localhost")));
        let nonce = format!("{:016x}", rand::random::<u64>());
        let root = PathBuf::from(
            std::env::var("LMM_CONTINUITY_DEV_ARTIFACTS")
                .unwrap_or_else(|_| std::env::temp_dir().to_string_lossy().into_owned()),
        )
        .join(format!("pg-ack-{nonce}"));
        fs::create_dir_all(&root).unwrap();
        fs::set_permissions(&root, fs::Permissions::from_mode(0o700)).unwrap();
        let socket_dir = std::env::temp_dir().join(format!("lmm-ack-{nonce}"));
        fs::create_dir(&socket_dir).unwrap();
        fs::set_permissions(&socket_dir, fs::Permissions::from_mode(0o700)).unwrap();
        let ready = root.join("ready");
        let events = root.join("events.json");
        let stop = root.join("stop");
        let config = root.join("config.json");
        let repo = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..");
        let value = serde_json::json!({"upstream_port":url.port().unwrap(),"user":url.username(),"database":url.path().trim_start_matches('/'),
            "writer":writer,"socket_dir":socket_dir,"proxy_module":repo.join("tests/console/independent_process/pg_fault.py"),"ready":ready,"events":events,"stop":stop});
        let mut f = fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .mode(0o600)
            .open(&config)
            .unwrap();
        f.write_all(&serde_json::to_vec(&value).unwrap()).unwrap();
        drop(f);
        let log = fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .mode(0o600)
            .open(root.join("proxy.log"))
            .unwrap();
        let child = Command::new("python")
            .arg(PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/common/pg_commit_fault.py"))
            .arg(config)
            .stdout(Stdio::from(log.try_clone().unwrap()))
            .stderr(Stdio::from(log))
            .spawn()
            .unwrap();
        let mut this = Self {
            child,
            url: String::new(),
            events,
            stop,
            socket_dir,
        };
        for _ in 0..250 {
            if let Ok(port) = fs::read_to_string(&ready) {
                let mut proxied = url.clone();
                proxied.set_port(Some(port.parse().unwrap())).unwrap();
                let query: Vec<_> = proxied
                    .query_pairs()
                    .filter(|(k, _)| k != "sslmode")
                    .map(|(k, v)| (k.into_owned(), v.into_owned()))
                    .collect();
                proxied.set_query(None);
                for (k, v) in query {
                    proxied.query_pairs_mut().append_pair(&k, &v);
                }
                proxied.query_pairs_mut().append_pair("sslmode", "disable");
                this.url = proxied.to_string();
                return this;
            }
            assert!(
                this.child.try_wait().unwrap().is_none(),
                "PG ACK proxy exited; inspect private log"
            );
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
        panic!("PG proxy startup timeout")
    }
    pub async fn assert_commit_ack_dropped(&self) {
        for _ in 0..250 {
            if let Ok(bytes) = fs::read(&self.events) {
                let value: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
                if value["dropped"] == true {
                    assert_eq!(value["errors"], serde_json::json!([]));
                    let events = value["events"].as_array().unwrap();
                    assert!(events.contains(&serde_json::json!(
                        "upstream_command_complete_commit_suppressed"
                    )));
                    assert!(events.contains(&serde_json::json!(
                        "upstream_ready_idle_suppressed_connection_closed"
                    )));
                    return;
                }
            }
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
        panic!("real upstream COMMIT acknowledgement loss not observed")
    }
}
impl Drop for PgCommitFault {
    fn drop(&mut self) {
        let _ = fs::write(&self.stop, "");
        for _ in 0..100 {
            if self.child.try_wait().ok().flatten().is_some() {
                let _ = fs::remove_dir(&self.socket_dir);
                return;
            }
            std::thread::sleep(Duration::from_millis(20));
        }
        let _ = self.child.kill();
        let _ = self.child.wait();
        let _ = fs::remove_dir_all(&self.socket_dir);
    }
}

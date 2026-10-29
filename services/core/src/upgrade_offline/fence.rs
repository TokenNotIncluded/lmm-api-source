//! Persistent target admission control, outside the target's physical restore.
use super::{Error, Request, Result, digest, engine, require, secret};
use serde::{Deserialize, Serialize};
use sqlx::{Connection, PgConnection, Row, postgres::PgConnectOptions};
use std::{
    fs,
    io::Write,
    os::unix::fs::{MetadataExt, OpenOptionsExt, PermissionsExt},
    path::PathBuf,
    str::FromStr,
    time::Duration,
};

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ServiceScope {
    System,
    User,
}

/// Explicit deployment units; their file masks persist independently of PGDATA.
#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Service {
    pub name: String,
    pub scope: ServiceScope,
    pub executable_sha256: String,
}
impl Service {
    async fn systemctl(&self, args: &[&str]) -> Result<String> {
        require(
            self.name.starts_with("lmm-")
                && self.name.ends_with(".service")
                && self
                    .name
                    .bytes()
                    .all(|b| b.is_ascii_alphanumeric() || b"-_.@".contains(&b)),
            "OFFLINE_SERVICE_NAME_INVALID",
        )?;
        let name = self.name.clone();
        let user = matches!(self.scope, ServiceScope::User);
        let args = args.iter().map(|a| a.to_string()).collect::<Vec<_>>();
        let output = tokio::task::spawn_blocking(move || -> Result<std::process::Output> {
            let mut c = std::process::Command::new("systemctl");
            c.env("SYSTEMD_PAGER", "cat").env("SYSTEMD_COLORS", "0");
            if user {
                c.arg("--user");
            }
            c.args(&args)
                .arg(&name)
                .stdout(std::process::Stdio::piped())
                .stderr(std::process::Stdio::null());
            let mut child = c
                .spawn()
                .map_err(|_| Error::Rejected("OFFLINE_SERVICE_CONTROL_FAILED"))?;
            let deadline = std::time::Instant::now() + Duration::from_secs(300);
            while child
                .try_wait()
                .map_err(|_| Error::Rejected("OFFLINE_SERVICE_CONTROL_FAILED"))?
                .is_none()
            {
                if std::time::Instant::now() >= deadline {
                    let _ = child.kill();
                    let _ = child.wait();
                    return Err(Error::Rejected("OFFLINE_SERVICE_CONTROL_TIMEOUT"));
                }
                std::thread::sleep(Duration::from_millis(20));
            }
            child
                .wait_with_output()
                .map_err(|_| Error::Rejected("OFFLINE_SERVICE_CONTROL_FAILED"))
        })
        .await
        .map_err(|_| Error::Rejected("OFFLINE_SERVICE_CONTROL_FAILED"))??;
        require(output.status.success(), "OFFLINE_SERVICE_CONTROL_FAILED")?;
        String::from_utf8(output.stdout).map_err(|_| Error::Storage)
    }
    pub(super) async fn stop(&self) -> Result<()> {
        let pid = self
            .systemctl(&["show", "--property=MainPID", "--value"])
            .await?;
        let pid = pid.trim().parse::<u32>().map_err(|_| Error::Storage)?;
        if pid > 0 {
            require(pid != std::process::id(), "OFFLINE_CANNOT_STOP_CONTROLLER")?;
            let executable = fs::read(format!("/proc/{pid}/exe")).map_err(|_| Error::Storage)?;
            require(
                digest(executable) == self.executable_sha256,
                "OFFLINE_SERVICE_ARTIFACT_MISMATCH",
            )?;
        }
        self.systemctl(&["mask"]).await?;
        self.systemctl(&["stop"]).await?;
        self.verify().await
    }
    pub(super) async fn verify(&self) -> Result<()> {
        let state = self
            .systemctl(&[
                "show",
                "--property=ActiveState",
                "--property=MainPID",
                "--property=UnitFileState",
            ])
            .await?;
        let facts = state
            .lines()
            .filter_map(|s| s.split_once('='))
            .collect::<std::collections::BTreeMap<_, _>>();
        require(
            facts.get("MainPID") == Some(&"0")
                && matches!(facts.get("ActiveState"), Some(&"inactive" | &"failed"))
                && facts.get("UnitFileState") == Some(&"masked"),
            "OFFLINE_SERVICE_NOT_PERSISTENTLY_STOPPED",
        )
    }
}

pub(super) struct Fence {
    management: PgConnection,
    management_url: String,
    target_url: String,
    probe_url: String,
    hba: PathBuf,
    prefix: String,
    _lock: fs::File,
}
impl Fence {
    pub(super) async fn connect(target_url: &str, r: &Request, request_sha: &str) -> Result<Self> {
        let management_url = secret(&r.management_url_file)?;
        let probe_url = secret(&r.runtime_probe_url_file)?;
        for url in [target_url, management_url.as_str(), probe_url.as_str()] {
            let options = PgConnectOptions::from_str(url).map_err(|_| Error::Invalid)?;
            require(
                options.get_socket().is_some(),
                "OFFLINE_LOCAL_POSTGRES_SOCKET_REQUIRED",
            )?;
        }
        let mut management = PgConnection::connect(&management_url).await?;
        let row=sqlx::query("SELECT system_identifier::text AS system,(SELECT oid::bigint FROM pg_database WHERE datname=$1) AS oid,current_database() AS management_database,(SELECT rolsuper FROM pg_roles WHERE rolname=session_user) AS super,current_setting('hba_file') AS hba,current_setting('data_directory') AS data,pg_is_in_recovery() AS replica FROM pg_control_system()")
            .bind(&r.bridge.database_name).fetch_one(&mut management).await?;
        require(
            row.get::<String, _>("system") == r.bridge.system_identifier
                && row.get::<Option<i64>, _>("oid") == Some(i64::from(r.bridge.database_oid))
                && row.get::<bool, _>("super")
                && !row.get::<bool, _>("replica")
                && row.get::<String, _>("management_database") != r.bridge.database_name,
            "OFFLINE_MANAGEMENT_IDENTITY_MISMATCH",
        )?;
        let hba = fs::canonicalize(&r.hba_file).map_err(|_| Error::Storage)?;
        let actual = fs::canonicalize(row.get::<String, _>("hba")).map_err(|_| Error::Storage)?;
        let data = fs::canonicalize(row.get::<String, _>("data")).map_err(|_| Error::Storage)?;
        require(
            hba == r.hba_file && hba == actual && !hba.starts_with(data),
            "OFFLINE_HBA_MUST_BE_OUTSIDE_TARGET_RESTORE",
        )?;
        let meta = fs::symlink_metadata(&hba).map_err(|_| Error::Storage)?;
        require(
            meta.is_file() && meta.permissions().mode() & 0o022 == 0,
            "OFFLINE_HBA_NOT_PROTECTED",
        )?;
        let lockpath = hba.with_extension("lmm-offline.lock");
        if lockpath.exists() {
            require(
                fs::symlink_metadata(&lockpath)
                    .map_err(|_| Error::Storage)?
                    .is_file(),
                "OFFLINE_LOCK_NOT_REGULAR",
            )?;
        }
        let lock = fs::OpenOptions::new()
            .read(true)
            .write(true)
            .create(true)
            .truncate(false)
            .mode(0o600)
            .open(&lockpath)
            .map_err(|_| Error::Storage)?;
        lock.lock().map_err(|_| Error::Storage)?;
        let prefix = format!(
            "# lmm offline v3 {request_sha}\nlocal {} {} scram-sha-256\nlocal {} all reject\nhost {} all 0.0.0.0/0 reject\nhost {} all ::/0 reject\n# end lmm offline v3\n",
            engine::quote(&r.bridge.database_name),
            engine::quote(&r.bridge.installer),
            engine::quote(&r.bridge.database_name),
            engine::quote(&r.bridge.database_name),
            engine::quote(&r.bridge.database_name)
        );
        Ok(Self {
            management,
            management_url,
            target_url: target_url.into(),
            probe_url,
            hba,
            prefix,
            _lock: lock,
        })
    }
    pub(super) async fn close_admission(&mut self, r: &Request) -> Result<()> {
        let old = fs::read_to_string(&self.hba).map_err(|_| Error::Storage)?;
        if !old.starts_with(&self.prefix) {
            require(
                !old.starts_with("# lmm offline v3 "),
                "OFFLINE_ANOTHER_OPERATION_HOLDS_FENCE",
            )?;
            let meta = fs::metadata(&self.hba).map_err(|_| Error::Storage)?;
            let temporary = self
                .hba
                .with_extension(format!("lmm-{}.new", std::process::id()));
            let mut file = fs::OpenOptions::new()
                .create_new(true)
                .write(true)
                .mode(0o600)
                .open(&temporary)
                .map_err(|_| Error::Storage)?;
            file.write_all(format!("{}{old}", self.prefix).as_bytes())
                .map_err(|_| Error::Storage)?;
            std::os::unix::fs::chown(&temporary, Some(meta.uid()), Some(meta.gid()))
                .map_err(|_| Error::Storage)?;
            file.set_permissions(fs::Permissions::from_mode(
                meta.permissions().mode() & 0o777,
            ))
            .map_err(|_| Error::Storage)?;
            file.sync_all().map_err(|_| Error::Storage)?;
            fs::rename(&temporary, &self.hba).map_err(|_| Error::Storage)?;
            fs::File::open(self.hba.parent().ok_or(Error::Invalid)?)
                .map_err(|_| Error::Storage)?
                .sync_all()
                .map_err(|_| Error::Storage)?;
        }
        let loaded: String = sqlx::query_scalar("SELECT pg_conf_load_time()::text")
            .fetch_one(&mut self.management)
            .await?;
        let reloaded: bool = sqlx::query_scalar("SELECT pg_reload_conf()")
            .fetch_one(&mut self.management)
            .await?;
        require(reloaded, "OFFLINE_HBA_RELOAD_FAILED")?;
        let mut changed = false;
        for _ in 0..100 {
            let now: String = sqlx::query_scalar("SELECT pg_conf_load_time()::text")
                .fetch_one(&mut self.management)
                .await?;
            if now != loaded {
                changed = true;
                break;
            }
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
        require(changed, "OFFLINE_HBA_RELOAD_NOT_OBSERVED")?;
        let valid: bool = sqlx::query_scalar(
            "SELECT NOT EXISTS(SELECT FROM pg_hba_file_rules WHERE error IS NOT NULL)",
        )
        .fetch_one(&mut self.management)
        .await?;
        require(valid, "OFFLINE_HBA_PARSE_FAILED")?;
        self.allow(r, false).await
    }
    async fn allow(&mut self, r: &Request, allow: bool) -> Result<()> {
        sqlx::query(&format!(
            "ALTER DATABASE {} ALLOW_CONNECTIONS {allow}",
            engine::quote(&r.bridge.database_name)
        ))
        .execute(&mut self.management)
        .await?;
        let actual: bool =
            sqlx::query_scalar("SELECT datallowconn FROM pg_database WHERE oid=$1::bigint::oid")
                .bind(i64::from(r.bridge.database_oid))
                .fetch_one(&mut self.management)
                .await?;
        require(actual == allow, "OFFLINE_DATABASE_ADMISSION_UNKNOWN")
    }
    /// HBA remains closed to applications during this maintenance-only window.
    pub(super) async fn target(&mut self, r: &Request) -> Result<PgConnection> {
        require(
            fs::read_to_string(&self.hba)
                .map_err(|_| Error::Storage)?
                .starts_with(&self.prefix),
            "OFFLINE_HBA_FENCE_CHANGED",
        )?;
        self.allow(r, true).await?;
        let opened = PgConnection::connect(&self.target_url).await;
        // A fresh management connection owns closure even if target auth failed.
        let reclosed = async {
            let mut fresh = PgConnection::connect(&self.management_url).await?;
            let id: String =
                sqlx::query_scalar("SELECT system_identifier::text FROM pg_control_system()")
                    .fetch_one(&mut fresh)
                    .await?;
            require(
                id == r.bridge.system_identifier,
                "OFFLINE_RECLOSE_IDENTITY_MISMATCH",
            )?;
            sqlx::query(&format!(
                "ALTER DATABASE {} ALLOW_CONNECTIONS false",
                engine::quote(&r.bridge.database_name)
            ))
            .execute(&mut fresh)
            .await?;
            let closed: bool = sqlx::query_scalar(
                "SELECT NOT datallowconn FROM pg_database WHERE oid=$1::bigint::oid",
            )
            .bind(i64::from(r.bridge.database_oid))
            .fetch_one(&mut fresh)
            .await?;
            require(closed, "OFFLINE_RECLOSE_UNKNOWN")
        }
        .await;
        reclosed?;
        let mut target = opened?;
        sqlx::query(&format!("SET ROLE {}", engine::quote(&r.bridge.owner)))
            .execute(&mut target)
            .await?;
        let valid:bool=sqlx::query_scalar("SELECT current_database()=$1 AND (SELECT oid::bigint FROM pg_database WHERE datname=current_database())=$2 AND session_user=$3 AND (SELECT system_identifier::text FROM pg_control_system())=$4")
            .bind(&r.bridge.database_name).bind(i64::from(r.bridge.database_oid)).bind(&r.bridge.installer).bind(&r.bridge.system_identifier).fetch_one(&mut target).await?;
        require(valid, "OFFLINE_TARGET_IDENTITY_MISMATCH")?;
        Ok(target)
    }
    pub(super) async fn prepare(&mut self, r: &Request) -> Result<PgConnection> {
        self.close_admission(r).await?;
        for service in &r.services {
            service.stop().await?;
        }
        let scram:bool=sqlx::query_scalar("SELECT coalesce(rolpassword LIKE 'SCRAM-SHA-256$%',false) FROM pg_authid WHERE rolname=$1").bind(&r.bridge.installer).fetch_one(&mut self.management).await?;
        require(scram, "OFFLINE_INSTALLER_SCRAM_REQUIRED")?;
        // Revoke admission before taking conservation snapshots; no runtime
        // role is reenabled by an apply or recovery result.
        sqlx::query(&format!(
            "ALTER ROLE {} NOLOGIN",
            engine::quote(&r.bridge.runtime)
        ))
        .execute(&mut self.management)
        .await?;
        sqlx::query(&format!(
            "REVOKE CONNECT ON DATABASE {} FROM PUBLIC,{},{}",
            engine::quote(&r.bridge.database_name),
            engine::quote(&r.bridge.runtime),
            engine::quote(&r.bridge.extensions)
        ))
        .execute(&mut self.management)
        .await?;
        self.target(r).await
    }
    pub(super) async fn verify(&mut self, r: &Request, pid: i32) -> Result<String> {
        require(
            fs::read_to_string(&self.hba)
                .map_err(|_| Error::Storage)?
                .starts_with(&self.prefix),
            "OFFLINE_HBA_FENCE_CHANGED",
        )?;
        sqlx::query("SELECT pg_stat_clear_snapshot()")
            .execute(&mut self.management)
            .await?;
        let clean:bool=sqlx::query_scalar("SELECT NOT datallowconn AND NOT EXISTS(SELECT FROM pg_stat_activity WHERE datid=d.oid AND pid<>$2) AND NOT EXISTS(SELECT FROM pg_prepared_xacts WHERE database=d.datname) AND NOT EXISTS(SELECT FROM pg_subscription WHERE subdbid=d.oid) FROM pg_database d WHERE d.oid=$1::bigint::oid")
            .bind(i64::from(r.bridge.database_oid)).bind(pid).fetch_one(&mut self.management).await?;
        require(clean, "OFFLINE_TARGET_WRITERS_NOT_DRAINED")?;
        for s in &r.services {
            s.verify().await?;
        }
        Ok(digest(fs::read(&self.hba).map_err(|_| Error::Storage)?))
    }
    /// Probe the loaded HBA, not merely the on-disk pg_hba_file_rules view.
    pub(super) async fn probe(&mut self, r: &Request) -> Result<()> {
        self.allow(r, true).await?;
        let result = PgConnection::connect(&self.probe_url).await;
        self.allow(r, false).await?;
        match result {
            Err(sqlx::Error::Database(error))
                if error.code().as_deref() == Some("28000")
                    && error.message().contains("pg_hba.conf") =>
            {
                Ok(())
            }
            Ok(c) => {
                c.close().await?;
                Err(Error::Rejected("OFFLINE_RUNTIME_HBA_PROBE_ADMITTED"))
            }
            _ => Err(Error::Rejected("OFFLINE_RUNTIME_HBA_PROBE_UNDETERMINED")),
        }
    }
}

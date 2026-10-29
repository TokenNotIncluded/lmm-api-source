//! Explicit native schema upgrades with external history and persistent fencing.
//!
//! A committed schema upgrade does not authorize reopening a site. Financial
//! migration, deployment-wide reconciliation and finalization remain separate.
mod engine;
mod fence;
use crate::upgrade_history::{Binding, History, Phase};
pub use engine::{BridgeRequest, Predecessor};
pub use fence::{Service, ServiceScope};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use sqlx::{Connection, PgConnection};
use std::{
    fmt, fs,
    io::Read,
    path::{Path, PathBuf},
};

/// No raw database errors, configuration paths or secrets enter public errors.
#[derive(Debug)]
pub enum Error {
    Invalid,
    Storage,
    Rejected(&'static str),
    CommitUnknown,
}
impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::Invalid => "OFFLINE_INVALID_REQUEST",
            Self::Storage => "OFFLINE_STORAGE_UNAVAILABLE",
            Self::Rejected(code) => code,
            Self::CommitUnknown => "OFFLINE_COMMIT_UNKNOWN_USE_RECOVER",
        })
    }
}
impl std::error::Error for Error {}
impl From<sqlx::Error> for Error {
    fn from(_: sqlx::Error) -> Self {
        Self::Storage
    }
}
impl From<engine::BridgeError> for Error {
    fn from(e: engine::BridgeError) -> Self {
        match e {
            engine::BridgeError::Rejected(code) => Self::Rejected(code),
            engine::BridgeError::Database(inner) => {
                drop(inner);
                Self::Storage
            }
        }
    }
}
impl From<crate::upgrade_history::HistoryError> for Error {
    fn from(e: crate::upgrade_history::HistoryError) -> Self {
        use crate::upgrade_history::HistoryError as H;
        match e {
            H::CommitUnknown => Self::Rejected("OFFLINE_HISTORY_COMMIT_UNKNOWN"),
            H::Conflict => Self::Rejected("OFFLINE_HISTORY_CONFLICT"),
            H::Identity => Self::Rejected("OFFLINE_HISTORY_AUTHORITY_MISMATCH"),
            _ => Self::Storage,
        }
    }
}
type Result<T> = std::result::Result<T, Error>;
fn require(ok: bool, code: &'static str) -> Result<()> {
    if ok {
        Ok(())
    } else {
        Err(Error::Rejected(code))
    }
}
fn digest(bytes: impl AsRef<[u8]>) -> String {
    format!("{:x}", Sha256::digest(bytes))
}
fn secret(path: &Path) -> Result<String> {
    let bytes = crate::config::private_file(path, 16384).map_err(|_| Error::Invalid)?;
    String::from_utf8(bytes)
        .map(|s| s.trim().into())
        .map_err(|_| Error::Invalid)
}

#[derive(Clone, Copy, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum Action {
    Apply,
    Recover,
}
/// Only new runtime permissions; previously granted capabilities are preserved.
#[derive(Clone, Debug, Default, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct RuntimeDelta {
    pub console: bool,
    pub assistant_agent: bool,
}

/// Operator-owned inputs. URL file paths are never persisted as request facts.
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Request {
    pub version: u32,
    pub action: Action,
    pub operation_id: String,
    pub candidate_sha256: String,
    pub authority_id: String,
    pub history_url_file: PathBuf,
    pub management_url_file: PathBuf,
    pub runtime_probe_url_file: PathBuf,
    pub hba_file: PathBuf,
    pub bridge: BridgeRequest,
    pub runtime: RuntimeDelta,
    pub services: Vec<Service>,
}
impl Request {
    /// Bound private stdin, with no automatic source, URL or flag fallback.
    pub fn read(input: impl Read) -> Result<Self> {
        let mut bytes = Vec::new();
        input
            .take(65537)
            .read_to_end(&mut bytes)
            .map_err(|_| Error::Invalid)?;
        if bytes.len() > 65536 {
            return Err(Error::Invalid);
        }
        serde_json::from_slice(&bytes).map_err(|_| Error::Invalid)
    }
    fn binding(&self) -> Result<Binding> {
        require(self.version == 3, "OFFLINE_VERSION_INVALID")?;
        let site_id = digest(
            serde_json::to_vec(&(
                "core.offline.site.v3",
                &self.bridge.system_identifier,
                self.bridge.database_oid,
            ))
            .map_err(|_| Error::Invalid)?,
        );
        require(
            self.bridge.version == 3
                && self.bridge.procedure == engine::PROCEDURE
                && [
                    &self.bridge.owner,
                    &self.bridge.installer,
                    &self.bridge.runtime,
                    &self.bridge.extensions,
                ]
                .iter()
                .all(|n| !n.is_empty() && n.len() <= 63 && !n.chars().any(char::is_control)),
            "OFFLINE_ROLE_OR_PROCEDURE_INVALID",
        )?;
        let request_sha256 = digest(
            serde_json::to_vec(&(
                "core.offline.request.v3",
                &self.bridge,
                &self.runtime,
                &self.services,
                &self.hba_file,
                &self.authority_id,
            ))
            .map_err(|_| Error::Invalid)?,
        );
        Ok(Binding {
            site_id,
            operation_id: self.operation_id.clone(),
            request_sha256,
            candidate_sha256: self.candidate_sha256.clone(),
            target_system_identifier: self.bridge.system_identifier.clone(),
            target_database: self.bridge.database_name.clone(),
            target_database_oid: self.bridge.database_oid,
        })
    }
}

async fn grant_delta(c: &mut PgConnection, r: &Request) -> Result<()> {
    let role = engine::quote(&r.bridge.runtime);
    if r.runtime.console {
        sqlx::raw_sql(&format!("GRANT EXECUTE ON FUNCTION core_console.lock_public_options(),core_console.write_public_option(text,text) TO {role}"))
            .execute(&mut *c).await?;
    }
    let canonical = include_str!("../../../deployment/postgres/grant-core-runtime.sql");
    {
        let sql = canonical
            .split_once(
                "-- Email authority and delivery: only the columns changed by the writers.\n",
            )
            .ok_or(Error::Invalid)?
            .1
            .split_once("\\if :enable_oauth2\n")
            .ok_or(Error::Invalid)?
            .0
            .replace(":\"runtime_role\"", &role);
        sqlx::raw_sql(&sql).execute(&mut *c).await?;
    }
    if r.runtime.assistant_agent {
        let sql = canonical
            .split_once("-- BEGIN assistant-agent runtime grants\n")
            .ok_or(Error::Invalid)?
            .1
            .split_once("-- END assistant-agent runtime grants")
            .ok_or(Error::Invalid)?
            .0
            .replace(":\"runtime_role\"", &role);
        sqlx::raw_sql(&sql).execute(&mut *c).await?;
    }
    Ok(())
}

async fn receipt(c: &mut PgConnection, b: &Binding) -> Result<Option<(Value, String)>> {
    let exists: bool =
        sqlx::query_scalar("SELECT to_regclass('core_meta.offline_upgrades') IS NOT NULL")
            .fetch_one(&mut *c)
            .await?;
    if !exists {
        return Ok(None);
    }
    let text: Option<String> = sqlx::query_scalar(
        "SELECT to_jsonb(r)::text FROM core_meta.offline_upgrades r WHERE operation_id=$1",
    )
    .bind(&b.operation_id)
    .fetch_optional(&mut *c)
    .await?;
    text.map(|text| {
        let value: Value = serde_json::from_str(&text).map_err(|_| Error::Storage)?;
        require(
            value["binding"] == serde_json::to_value(b).map_err(|_| Error::Invalid)?
                && value["procedure"] == "core-offline-upgrade-v3",
            "OFFLINE_RECEIPT_BINDING_MISMATCH",
        )?;
        Ok((value, digest(text)))
    })
    .transpose()
}

/// The same dedicated session is fenced and locked before its fresh snapshot.
pub async fn execute(target_url: &str, r: Request) -> Result<Value> {
    let binding = r.binding()?;
    let compiled =
        serde_json::to_value(crate::core_schema::schema_manifest()).map_err(|_| Error::Invalid)?;
    let plan = if r.action == Action::Apply {
        Some(engine::BridgePlan::new(
            serde_json::from_value(serde_json::to_value(&r.bridge).map_err(|_| Error::Invalid)?)
                .map_err(|_| Error::Invalid)?,
            &compiled,
        )?)
    } else {
        None
    };
    if r.action == Action::Apply {
        let actual = fs::read(std::env::current_exe().map_err(|_| Error::Storage)?)
            .map_err(|_| Error::Storage)?;
        require(
            digest(actual) == r.candidate_sha256,
            "OFFLINE_EXECUTABLE_MISMATCH",
        )?;
    }
    let mut history = History::connect(
        &secret(&r.history_url_file)?,
        &r.authority_id,
        &r.bridge.system_identifier,
    )
    .await?;
    let previous = history.snapshot(&binding).await?;
    if r.action == Action::Apply {
        require(
            !previous
                .as_ref()
                .is_some_and(|s| matches!(s.phase, Phase::Started | Phase::Committed)),
            "OFFLINE_STARTED_RECOVER_ONLY",
        )?;
    } else {
        require(previous.is_some(), "OFFLINE_HISTORY_MISSING_UNKNOWN")?;
    }
    let mut fence = fence::Fence::connect(target_url, &r, &binding.request_sha256).await?;
    let current = if r.action == Action::Apply {
        history.reserve(&binding).await?
    } else {
        previous.ok_or(Error::Invalid)?
    };
    let mut c = fence.prepare(&r).await?;
    sqlx::raw_sql("SET search_path=pg_catalog; SET timezone='UTC'; SET statement_timeout='30s'; SET lock_timeout='3s'; SELECT pg_advisory_lock(741803120)").execute(&mut c).await?;
    let pid: i32 = sqlx::query_scalar("SELECT pg_backend_pid()")
        .fetch_one(&mut c)
        .await?;
    let fence_sha = fence.verify(&r, pid).await?;
    fence.probe(&r).await?;
    if r.action == Action::Recover {
        let observed = receipt(&mut c, &binding).await?;
        let contract_matches = if let Some((value, _)) = &observed {
            let actual: Vec<String> =
                sqlx::query_scalar(crate::core_schema::schema_manifest().catalog.query_utf8)
                    .fetch_all(&mut c)
                    .await?;
            let saved:String=sqlx::query_scalar("SELECT encode(catalog_fingerprint,'hex') FROM core_meta.runtime_contract WHERE singleton").fetch_one(&mut c).await?;
            engine::framed(&actual) == saved && value["successor_catalog_sha256"] == saved
        } else {
            false
        };
        let status = match (&current.phase, &observed) {
            (Phase::Committed, Some((_, sha))) if current.receipt_sha256.as_ref() == Some(sha) => {
                "committed_history_observed"
            }
            (Phase::Committed, _) => "database_diverged_from_committed_history",
            (Phase::Started, Some((_, sha))) if contract_matches => {
                history.committed(&binding, sha).await?;
                "committed_receipt_recovered"
            }
            _ => "unknown_history",
        };
        c.close().await?;
        return Ok(
            json!({"version":3,"status":status,"operation_id":r.operation_id,"observed_receipt":observed.is_some(),"target_contract_matches":contract_matches,"site_open_allowed":false,"retry_allowed":false}),
        );
    }
    let plan = plan.ok_or(Error::Invalid)?;
    engine::offline(&mut c, &plan).await?;
    let rows = engine::OldRows::capture(&mut c).await?;
    if current.phase == Phase::Closing {
        history.held(&binding, &fence_sha).await?;
    } else {
        require(
            current.phase == Phase::Held && current.fence_sha256.as_ref() == Some(&fence_sha),
            "OFFLINE_HELD_FENCE_MISMATCH",
        )?;
    }
    history.start(&binding).await?;
    let mut tx = c.begin().await?;
    sqlx::raw_sql("SET TRANSACTION ISOLATION LEVEL SERIALIZABLE; SET LOCAL synchronous_commit=on; SET LOCAL work_mem='4MB'; SET LOCAL max_parallel_workers_per_gather=0").execute(&mut *tx).await?;
    for table in rows.columns.keys() {
        sqlx::query(&format!("LOCK TABLE {table} IN ACCESS EXCLUSIVE MODE"))
            .execute(&mut *tx)
            .await?;
    }
    engine::offline(&mut tx, &plan).await?;
    let mut report = engine::stage(&mut tx, &plan, &rows).await?;
    grant_delta(&mut tx, &r).await?;
    let permissions = engine::framed(&engine::permissions(&mut tx).await?);
    sqlx::query("INSERT INTO core_meta.offline_upgrades(operation_id,procedure,binding,predecessor_catalog_sha256,successor_catalog_sha256,preserved_data_sha256,preserved_permissions_sha256,runtime_permissions_sha256) VALUES($1,'core-offline-upgrade-v3',$2,$3,$4,$5,$6,$7)")
        .bind(&r.operation_id).bind(serde_json::to_value(&binding).map_err(|_|Error::Invalid)?).bind(&report.predecessor_catalog_sha256).bind(&report.successor_catalog_sha256).bind(&report.preserved_data_sha256).bind(&report.preserved_permissions_sha256).bind(permissions).execute(&mut *tx).await?;
    fence.verify(&r, pid).await?;
    tx.commit().await.map_err(|_| Error::CommitUnknown)?;
    let (_, sha) = receipt(&mut c, &binding).await?.ok_or(Error::Storage)?;
    history.committed(&binding, &sha).await?;
    report.status = "committed";
    c.close().await?;
    Ok(
        json!({"version":3,"status":"committed","operation_id":r.operation_id,"schema":report,"receipt_sha256":sha,"fence_scope":"target_postgres_and_named_units","site_open_allowed":false,"retry_allowed":false}),
    )
}

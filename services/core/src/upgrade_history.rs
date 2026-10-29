//! Durable upgrade history in a separately administered PostgreSQL authority.
//!
//! This store supplies original-operation tombstones and compare-and-set. It
//! does not attest a writer fence, authorize target DDL or reopen a site. The
//! authority and its administration must remain outside target restores.
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use sqlx::{Connection, PgConnection, Row};
use std::{fmt, io::Read};

const SCHEMA: &str = include_str!("../schema/upgrade_history.sql");

/// Secret-free errors; failed mutation acknowledgements require observation.
#[derive(Debug)]
pub enum HistoryError {
    Invalid,
    Identity,
    Conflict,
    Storage,
    CommitUnknown,
}
impl fmt::Display for HistoryError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::Invalid => "UPGRADE_HISTORY_INVALID_REQUEST",
            Self::Identity => "UPGRADE_HISTORY_AUTHORITY_MISMATCH",
            Self::Conflict => "UPGRADE_HISTORY_CAS_OR_BINDING_CONFLICT",
            Self::Storage => "UPGRADE_HISTORY_UNAVAILABLE",
            Self::CommitUnknown => "UPGRADE_HISTORY_COMMIT_UNKNOWN",
        })
    }
}
impl std::error::Error for HistoryError {}
impl From<sqlx::Error> for HistoryError {
    fn from(error: sqlx::Error) -> Self {
        if error.as_database_error().is_some_and(|e| {
            matches!(
                e.message(),
                "UPGRADE_HISTORY_BINDING_CONFLICT" | "UPGRADE_HISTORY_CAS_CONFLICT"
            )
        }) {
            Self::Conflict
        } else {
            Self::Storage
        }
    }
}
type Result<T> = std::result::Result<T, HistoryError>;
fn hash(text: &str) -> bool {
    text.len() == 64
        && text
            .bytes()
            .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
}
fn identifier(text: &str) -> bool {
    !text.is_empty()
        && text.len() <= 63
        && text
            .bytes()
            .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'_')
        && text.as_bytes()[0].is_ascii_lowercase()
}
fn contract() -> String {
    format!("{:x}", Sha256::digest(SCHEMA))
}

/// Complete immutable binding of one original operation to one physical target.
#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct Binding {
    pub site_id: String,
    pub operation_id: String,
    pub request_sha256: String,
    pub candidate_sha256: String,
    pub target_system_identifier: String,
    pub target_database: String,
    pub target_database_oid: u32,
}
impl Binding {
    fn validate(&self) -> Result<()> {
        if [&self.site_id, &self.operation_id].iter().any(|s| {
            !(16..=128).contains(&s.len())
                || !s
                    .bytes()
                    .all(|b| b.is_ascii_alphanumeric() || b"-_.".contains(&b))
        }) || !hash(&self.request_sha256)
            || !hash(&self.candidate_sha256)
            || self
                .target_system_identifier
                .parse::<u64>()
                .ok()
                .is_none_or(|n| n == 0)
            || !identifier(&self.target_database)
            || self.target_database_oid == 0
        {
            return Err(HistoryError::Invalid);
        }
        Ok(())
    }
}

/// Recorded phase; none of these phases alone permits target execution.
#[derive(Clone, Copy, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum Phase {
    Closing,
    Held,
    Started,
    Committed,
}

/// Current authoritative row; absence never proves an operation did not commit.
#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
pub struct Snapshot {
    pub site_id: String,
    pub operation_id: String,
    pub binding: Binding,
    pub sequence: i64,
    pub phase: Phase,
    pub fence_sha256: Option<String>,
    pub receipt_sha256: Option<String>,
}

/// A fresh authenticated connection to an already installed external authority.
pub struct History {
    connection: PgConnection,
    authority_id: String,
    target_system_identifier: String,
}
impl fmt::Debug for History {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("History")
            .field("authority_id", &self.authority_id)
            .finish_non_exhaustive()
    }
}
impl History {
    /// Connect without initializing state. Reject the physical target cluster.
    pub async fn connect(
        url: &str,
        authority_id: &str,
        target_system_identifier: &str,
    ) -> Result<Self> {
        if !hash(authority_id)
            || target_system_identifier
                .parse::<u64>()
                .ok()
                .is_none_or(|n| n == 0)
        {
            return Err(HistoryError::Invalid);
        }
        let mut connection = PgConnection::connect(url).await?;
        let identity: Option<Value> = sqlx::query_scalar("SELECT lmm_upgrade_history.identity()")
            .fetch_one(&mut connection)
            .await?;
        let identity = identity.ok_or(HistoryError::Identity)?;
        if identity["authority_id"] != authority_id
            || identity["contract_sha256"] != contract()
            || identity["system_identifier"] == target_system_identifier
            || identity["primary"] != true
            || identity["fsync"] != "on"
            || identity["full_page_writes"] != "on"
        {
            return Err(HistoryError::Identity);
        }
        let safe:bool=sqlx::query_scalar("SELECT NOT(rolsuper OR rolcreatedb OR rolcreaterole OR rolreplication OR rolbypassrls) AND current_user=session_user AND NOT pg_has_role(current_user,$1,'MEMBER') AND NOT pg_has_role(current_user,'pg_write_all_data','MEMBER') AND NOT pg_has_role(current_user,'pg_read_all_data','MEMBER') FROM pg_roles WHERE rolname=current_user")
            .bind(identity["owner"].as_str().ok_or(HistoryError::Identity)?).fetch_one(&mut connection).await?;
        let direct:bool=sqlx::query_scalar("SELECT has_table_privilege('lmm_upgrade_history.operations','INSERT,UPDATE,DELETE,TRUNCATE') OR has_table_privilege('lmm_upgrade_history.installation','INSERT,UPDATE,DELETE,TRUNCATE') OR has_schema_privilege('lmm_upgrade_history','CREATE')")
            .fetch_one(&mut connection).await?;
        if !safe || direct {
            return Err(HistoryError::Identity);
        }
        Ok(Self {
            connection,
            authority_id: authority_id.into(),
            target_system_identifier: target_system_identifier.into(),
        })
    }
    /// Read current history without creating missing state or clearing a slot.
    pub async fn snapshot(&mut self, binding: &Binding) -> Result<Option<Snapshot>> {
        binding.validate()?;
        if binding.target_system_identifier != self.target_system_identifier {
            return Err(HistoryError::Identity);
        }
        let value: Option<Value> = sqlx::query_scalar("SELECT lmm_upgrade_history.snapshot($1)")
            .bind(&binding.site_id)
            .fetch_one(&mut self.connection)
            .await?;
        value.map(|v| self.decode(v, binding)).transpose()
    }
    fn decode(&self, value: Value, binding: &Binding) -> Result<Snapshot> {
        let snapshot: Snapshot =
            serde_json::from_value(value).map_err(|_| HistoryError::Identity)?;
        if snapshot.binding != *binding
            || snapshot.site_id != binding.site_id
            || snapshot.operation_id != binding.operation_id
        {
            return Err(HistoryError::Conflict);
        }
        Ok(snapshot)
    }
    /// Reserve exactly one original operation; identical calls only observe it.
    pub async fn reserve(&mut self, binding: &Binding) -> Result<Snapshot> {
        self.mutate(binding, None, None).await
    }
    /// Record a separately verified closed writer fence. This store does not verify it.
    pub async fn held(&mut self, binding: &Binding, fence_sha256: &str) -> Result<Snapshot> {
        if !hash(fence_sha256) {
            return Err(HistoryError::Invalid);
        }
        self.mutate(binding, Some(1), Some(fence_sha256)).await
    }
    /// Consume the original held operation once; no lease or reconnect resets it.
    pub async fn start(&mut self, binding: &Binding) -> Result<Snapshot> {
        self.mutate(binding, Some(2), None).await
    }
    /// Record an independently verified target receipt after its actual commit.
    pub async fn committed(&mut self, binding: &Binding, receipt_sha256: &str) -> Result<Snapshot> {
        if !hash(receipt_sha256) {
            return Err(HistoryError::Invalid);
        }
        self.mutate(binding, Some(3), Some(receipt_sha256)).await
    }
    async fn mutate(
        &mut self,
        binding: &Binding,
        sequence: Option<i64>,
        fact: Option<&str>,
    ) -> Result<Snapshot> {
        binding.validate()?;
        if binding.target_system_identifier != self.target_system_identifier {
            return Err(HistoryError::Identity);
        }
        let mut tx = self.connection.begin().await?;
        sqlx::query("SET LOCAL synchronous_commit=on")
            .execute(&mut *tx)
            .await?;
        let value = serde_json::to_value(binding).map_err(|_| HistoryError::Invalid)?;
        let value: Value = if let Some(sequence) = sequence {
            sqlx::query_scalar("SELECT lmm_upgrade_history.advance($1,$2,$3)")
                .bind(value)
                .bind(sequence)
                .bind(fact)
                .fetch_one(&mut *tx)
                .await?
        } else {
            sqlx::query_scalar("SELECT lmm_upgrade_history.reserve($1)")
                .bind(value)
                .fetch_one(&mut *tx)
                .await?
        };
        tx.commit().await.map_err(|_| HistoryError::CommitUnknown)?;
        self.decode(value, binding)
    }
}

/// Offline CLI management: explicit installation or read-only inspection only.
#[derive(Debug, Deserialize)]
#[serde(tag = "action", rename_all = "snake_case", deny_unknown_fields)]
pub enum Command {
    Install {
        authority_id: String,
        owner: String,
        writer: String,
    },
    Inspect {
        authority_id: String,
        binding: Binding,
    },
}
impl Command {
    /// Read a bounded request without accepting DSNs or authority assertions.
    pub fn read(input: impl Read) -> Result<Self> {
        let mut bytes = Vec::new();
        input
            .take(16385)
            .read_to_end(&mut bytes)
            .map_err(|_| HistoryError::Invalid)?;
        if bytes.len() > 16384 {
            return Err(HistoryError::Invalid);
        }
        serde_json::from_slice(&bytes).map_err(|_| HistoryError::Invalid)
    }
    /// Execute against the explicit offline configuration's database URL.
    pub async fn run(self, url: &str) -> Result<Value> {
        let result = match self {
            Self::Install {
                authority_id,
                owner,
                writer,
            } => install(url, &authority_id, &owner, &writer).await?,
            Self::Inspect {
                authority_id,
                binding,
            } => {
                let mut history =
                    History::connect(url, &authority_id, &binding.target_system_identifier).await?;
                json!({"status":"observed_history_only","operation":history.snapshot(&binding).await?})
            }
        };
        Ok(
            json!({"version":1,"protocol":"core.upgrade-history.v1","result":result,"retry_allowed":false,"site_open_allowed":false}),
        )
    }
}

async fn install(url: &str, authority_id: &str, owner: &str, writer: &str) -> Result<Value> {
    if !hash(authority_id) || !identifier(owner) || !identifier(writer) || owner == writer {
        return Err(HistoryError::Invalid);
    }
    let mut connection = PgConnection::connect(url).await?;
    let mut tx = connection.begin().await?;
    sqlx::query("SET LOCAL synchronous_commit=on")
        .execute(&mut *tx)
        .await?;
    let roles=sqlx::query("SELECT rolname::text,rolcanlogin,rolsuper OR rolcreatedb OR rolcreaterole OR rolreplication OR rolbypassrls AS privileged FROM pg_roles WHERE rolname=$1 OR rolname=$2")
        .bind(owner).bind(writer).fetch_all(&mut *tx).await?;
    if roles.len() != 2
        || roles.iter().any(|r| {
            r.get::<bool, _>("privileged")
                || r.get::<bool, _>("rolcanlogin") != (r.get::<String, _>("rolname") == writer)
        })
    {
        return Err(HistoryError::Identity);
    }
    let separated:bool=sqlx::query_scalar("SELECT NOT pg_has_role($1,$2,'MEMBER') AND session_user<>$1 AND session_user<>$2 AND NOT EXISTS(SELECT FROM pg_namespace WHERE nspname NOT IN ('public','information_schema') AND nspname !~ '^pg_') AND NOT EXISTS(SELECT FROM pg_class WHERE relnamespace='public'::regnamespace) AND NOT EXISTS(SELECT FROM pg_proc WHERE pronamespace='public'::regnamespace)")
        .bind(writer).bind(owner).fetch_one(&mut *tx).await?;
    if !separated {
        return Err(HistoryError::Identity);
    }
    sqlx::query(&format!("SET LOCAL ROLE \"{owner}\" "))
        .execute(&mut *tx)
        .await?;
    sqlx::raw_sql(SCHEMA).execute(&mut *tx).await?;
    sqlx::query("INSERT INTO lmm_upgrade_history.installation(authority_id,contract_sha256,writer) VALUES($1,$2,$3::name)")
        .bind(authority_id).bind(contract()).bind(writer).execute(&mut *tx).await?;
    sqlx::raw_sql(&format!("GRANT USAGE ON SCHEMA lmm_upgrade_history TO \"{writer}\"; GRANT EXECUTE ON FUNCTION lmm_upgrade_history.identity(),lmm_upgrade_history.snapshot(text),lmm_upgrade_history.reserve(jsonb),lmm_upgrade_history.advance(jsonb,bigint,text) TO \"{writer}\""))
        .execute(&mut *tx).await?;
    tx.commit().await.map_err(|_| HistoryError::CommitUnknown)?;
    Ok(json!({"status":"installed","authority_id":authority_id}))
}

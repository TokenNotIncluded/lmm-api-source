//! Durable operational jobs and process heartbeats. Financial facts stay intact.
use crate::identity::{self, CredentialKind, IdentityError};
use axum::{
    Json, Router,
    extract::{Path, Query, State},
    http::{HeaderMap, StatusCode, header},
    response::{IntoResponse, Response},
    routing::{get, post},
};
use serde::Deserialize;
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use sqlx::{PgConnection, PgPool, Row, postgres::PgRow};
use std::{
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
    time::Duration,
};

pub const SCHEMA: &str = include_str!("../schema/operations.sql");
const STALE_SECONDS: i64 = 45;
const BATCH_SIZE: i32 = 500;

#[derive(Clone)]
pub struct OperationsStore {
    pool: PgPool,
}

#[derive(Debug)]
pub enum OperationsError {
    Unauthorized,
    Forbidden,
    Invalid,
    Conflict,
    NotFound,
    Unavailable,
}
impl std::fmt::Display for OperationsError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "operations unavailable: {self:?}")
    }
}
impl std::error::Error for OperationsError {}
impl From<sqlx::Error> for OperationsError {
    fn from(_: sqlx::Error) -> Self {
        Self::Unavailable
    }
}
impl From<IdentityError> for OperationsError {
    fn from(value: IdentityError) -> Self {
        match value {
            IdentityError::Unauthorized => Self::Unauthorized,
            IdentityError::Forbidden => Self::Forbidden,
            _ => Self::Unavailable,
        }
    }
}
type Result<T> = std::result::Result<T, OperationsError>;

async fn authorize(c: &mut PgConnection, secret: &str, level: i16) -> Result<i64> {
    let actor = identity::authorize_in_transaction(c, secret).await?;
    if actor.credential_kind != CredentialKind::Session || actor.platform_level < level {
        return Err(OperationsError::Forbidden);
    }
    let role: String =
        sqlx::query_scalar("SELECT platform_role FROM core_identity.users WHERE id=$1")
            .bind(actor.user_id)
            .fetch_one(c)
            .await?;
    if role != "superadmin" && (level >= 6 || role != "admin") {
        return Err(OperationsError::Forbidden);
    }
    Ok(actor.user_id)
}

fn task(row: &PgRow) -> Result<Value> {
    let total: i64 = row.try_get("total")?;
    let processed: i64 = row.try_get("processed")?;
    let status: String = row.try_get("status")?;
    Ok(json!({
        "id":row.try_get::<i64,_>("id")?, "task_id":row.try_get::<String,_>("task_id")?,
        "type":"log_cleanup", "status":status, "scope":"historical_logs",
        "payload":{"target_timestamp":row.try_get::<i64,_>("target_timestamp")?,
            "batch_size":row.try_get::<i32,_>("batch_size")?},
        "state":{"total":total,"processed":processed,"remaining":total-processed,
            "progress":if total==0 {100.0} else {processed as f64/total as f64*100.0}},
        "result":(status=="succeeded").then(||json!({"deleted_count":processed})),
        "created_at":row.try_get::<i64,_>("created_unix")?,
        "updated_at":row.try_get::<i64,_>("updated_unix")?
    }))
}
const TASK_COLUMNS: &str = "*,floor(extract(epoch FROM created_at))::bigint AS created_unix,floor(extract(epoch FROM updated_at))::bigint AS updated_unix";

impl OperationsStore {
    pub fn new(pool: PgPool) -> Self {
        Self { pool }
    }

    pub async fn start_cleanup(&self, secret: &str, target: i64, key: &str) -> Result<Value> {
        if target <= 0
            || target > 253402300799
            || key.is_empty()
            || key.len() > 128
            || !key.bytes().all(|b| b.is_ascii_graphic())
        {
            return Err(OperationsError::Invalid);
        }
        let mut tx = self.pool.begin().await?;
        sqlx::query("SET LOCAL statement_timeout='5s'")
            .execute(&mut *tx)
            .await?;
        let user = authorize(&mut tx, secret, 6).await?;
        let id = format!(
            "{:x}",
            Sha256::digest(format!("log-cleanup-v1:{user}:{key}"))
        );
        sqlx::query("SELECT pg_advisory_xact_lock(741803151)")
            .execute(&mut *tx)
            .await?;
        if let Some(row) = sqlx::query(&format!(
            "SELECT {TASK_COLUMNS} FROM core_operations.log_cleanup_jobs WHERE task_id=$1"
        ))
        .bind(&id)
        .fetch_optional(&mut *tx)
        .await?
        {
            if row.try_get::<i64, _>("target_timestamp")? != target {
                return Err(OperationsError::Conflict);
            }
            let result = task(&row)?;
            tx.commit().await?;
            return Ok(result);
        }
        let now: i64 =
            sqlx::query_scalar("SELECT floor(extract(epoch FROM clock_timestamp()))::bigint")
                .fetch_one(&mut *tx)
                .await?;
        if target > now {
            return Err(OperationsError::Invalid);
        }
        let busy: bool = sqlx::query_scalar("SELECT EXISTS(SELECT FROM core_operations.log_cleanup_jobs WHERE status IN ('pending','running'))")
            .fetch_one(&mut *tx).await?;
        if busy {
            return Err(OperationsError::Conflict);
        }
        let bound: i64 =
            sqlx::query_scalar("SELECT coalesce(max(source_id),0) FROM core_history.sources")
                .fetch_one(&mut *tx)
                .await?;
        let total: i64 = sqlx::query_scalar(
            "SELECT count(*) FROM core_history.log_records WHERE created_at<$1 AND source_id<=$2",
        )
        .bind(target)
        .bind(bound)
        .fetch_one(&mut *tx)
        .await?;
        let row = sqlx::query(&format!("INSERT INTO core_operations.log_cleanup_jobs(task_id,created_by,target_timestamp,max_source_id,batch_size,total,status) VALUES($1,$2,$3,$4,$5,$6,$7) RETURNING {TASK_COLUMNS}"))
            .bind(&id).bind(user).bind(target).bind(bound).bind(BATCH_SIZE).bind(total)
            .bind(if total==0 {"succeeded"} else {"pending"}).fetch_one(&mut *tx).await?;
        let result = task(&row)?;
        tx.commit().await?;
        Ok(result)
    }

    pub async fn tasks(&self, secret: &str, id: Option<&str>, limit: i64) -> Result<Vec<Value>> {
        if !(1..=100).contains(&limit)
            || id.is_some_and(|id| id.len() != 64 || !id.bytes().all(|b| b.is_ascii_hexdigit()))
        {
            return Err(OperationsError::Invalid);
        }
        let mut tx = self.pool.begin().await?;
        authorize(&mut tx, secret, 6).await?;
        let rows=sqlx::query(&format!("SELECT {TASK_COLUMNS} FROM core_operations.log_cleanup_jobs WHERE ($1::text IS NULL OR task_id=$1) ORDER BY id DESC LIMIT $2"))
            .bind(id).bind(limit).fetch_all(&mut *tx).await?;
        let result = rows.iter().map(task).collect::<Result<Vec<_>>>()?;
        tx.commit().await?;
        Ok(result)
    }

    pub async fn cleanup_batch(&self) -> Result<Option<String>> {
        let mut tx = self.pool.begin().await?;
        sqlx::query("SET LOCAL statement_timeout='5s'")
            .execute(&mut *tx)
            .await?;
        let result = sqlx::query_scalar("SELECT core_operations.run_log_cleanup_batch()")
            .fetch_one(&mut *tx)
            .await?;
        tx.commit().await?;
        Ok(result)
    }

    pub async fn instances(&self, secret: &str) -> Result<Value> {
        let mut tx = self.pool.begin().await?;
        authorize(&mut tx, secret, 5).await?;
        let rows=sqlx::query("SELECT boot_id,instance_name,version,phase,in_flight,models_ready,rpc_ready, floor(extract(epoch FROM started_at))::bigint AS started_at, floor(extract(epoch FROM heartbeat_at))::bigint AS heartbeat_at, heartbeat_at>clock_timestamp()-make_interval(secs=>$1) AND phase<>'stopped' AS online FROM core_operations.instances ORDER BY heartbeat_at DESC,boot_id LIMIT 201")
            .bind(STALE_SECONDS as f64).fetch_all(&mut *tx).await?;
        let items=rows.iter().take(200).map(|row|Ok(json!({
            "boot_id":row.try_get::<String,_>("boot_id")?, "instance_name":row.try_get::<String,_>("instance_name")?,
            "version":row.try_get::<String,_>("version")?, "phase":row.try_get::<String,_>("phase")?,
            "in_flight":row.try_get::<i64,_>("in_flight")?, "models_ready":row.try_get::<bool,_>("models_ready")?,
            "rpc_ready":row.try_get::<bool,_>("rpc_ready")?, "started_at":row.try_get::<i64,_>("started_at")?,
            "heartbeat_at":row.try_get::<i64,_>("heartbeat_at")?, "online":row.try_get::<bool,_>("online")?
        }))).collect::<std::result::Result<Vec<Value>,sqlx::Error>>()?;
        tx.commit().await?;
        Ok(
            json!({"items":items,"truncated":rows.len()>200,"stale_after_seconds":STALE_SECONDS,"retention_days":7}),
        )
    }
}

/// Each process has its own boot ID, even when instance labels are reused.
pub struct Heartbeat {
    store: OperationsStore,
    boot: String,
    name: String,
}
impl Heartbeat {
    pub fn new(pool: PgPool, name: Option<String>) -> Result<Self> {
        let boot = format!("{:032x}", rand::random::<u128>());
        let name = name.unwrap_or_else(|| format!("core-{}", &boot[..12]));
        if name.is_empty() || name.len() > 128 || name.chars().any(char::is_control) {
            return Err(OperationsError::Invalid);
        }
        Ok(Self {
            store: OperationsStore::new(pool),
            boot,
            name,
        })
    }
    pub async fn pulse(&self, phase: &str, active: usize, models: bool, rpc: bool) -> Result<()> {
        sqlx::query("INSERT INTO core_operations.instances(boot_id,instance_name,version,phase,in_flight,models_ready,rpc_ready) VALUES($1,$2,$3,$4,$5,$6,$7) ON CONFLICT(boot_id) DO UPDATE SET heartbeat_at=clock_timestamp(),phase=EXCLUDED.phase,in_flight=EXCLUDED.in_flight,models_ready=EXCLUDED.models_ready,rpc_ready=EXCLUDED.rpc_ready")
            .bind(&self.boot).bind(&self.name).bind(env!("CARGO_PKG_VERSION")).bind(phase).bind(active as i64).bind(models).bind(rpc)
            .execute(&self.store.pool).await?;
        Ok(())
    }
    pub async fn run(
        self,
        lifecycle: crate::lifecycle::Lifecycle,
        models: Arc<AtomicBool>,
        rpc: Arc<AtomicBool>,
        mut stopped: tokio::sync::watch::Receiver<bool>,
    ) {
        let mut ticker = tokio::time::interval(Duration::from_secs(1));
        ticker.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
        let mut ticks = 0u32;
        loop {
            tokio::select! { _=ticker.tick()=>{}, _=stopped.changed()=>break }
            if *stopped.borrow() {
                break;
            }
            if ticks % 10 == 0 {
                let (phase, active) = lifecycle.snapshot();
                let phase = match phase {
                    crate::lifecycle::Phase::Preparing => "preparing",
                    crate::lifecycle::Phase::Ready => "ready",
                    crate::lifecycle::Phase::Draining => "draining",
                };
                if self
                    .pulse(
                        phase,
                        active,
                        models.load(Ordering::Acquire),
                        rpc.load(Ordering::Acquire),
                    )
                    .await
                    .is_err()
                {
                    eprintln!("instance heartbeat unavailable");
                }
            }
            if ticks % 3600 == 0 {
                if sqlx::query("DELETE FROM core_operations.instances WHERE heartbeat_at<clock_timestamp()-interval '7 days'").execute(&self.store.pool).await.is_err() {
                    eprintln!("old instance heartbeat cleanup unavailable");
                }
            }
            if self.store.cleanup_batch().await.is_err() {
                eprintln!("log cleanup batch not confirmed; durable progress will resume");
            }
            ticks = ticks.wrapping_add(1);
        }
        let (_, active) = lifecycle.snapshot();
        let _ = self.pulse("stopped", active, false, false).await;
    }
}

impl IntoResponse for OperationsError {
    fn into_response(self) -> Response {
        let (status, code) = match self {
            Self::Unauthorized => (StatusCode::UNAUTHORIZED, "OPERATIONS_UNAUTHORIZED"),
            Self::Forbidden => (StatusCode::FORBIDDEN, "OPERATIONS_FORBIDDEN"),
            Self::Invalid => (StatusCode::BAD_REQUEST, "OPERATIONS_INVALID"),
            Self::Conflict => (StatusCode::CONFLICT, "LOG_CLEANUP_CONFLICT"),
            Self::NotFound => (StatusCode::NOT_FOUND, "OPERATIONS_NOT_FOUND"),
            Self::Unavailable => (StatusCode::SERVICE_UNAVAILABLE, "OPERATIONS_UNAVAILABLE"),
        };
        (
            status,
            Json(json!({"success":false,"message":code,"error":{"code":code}})),
        )
            .into_response()
    }
}
fn bearer(headers: &HeaderMap) -> Result<&str> {
    let mut values = headers.get_all(header::AUTHORIZATION).iter();
    let value = values
        .next()
        .and_then(|v| v.to_str().ok())
        .ok_or(OperationsError::Unauthorized)?;
    if values.next().is_some() {
        return Err(OperationsError::Unauthorized);
    }
    let (scheme, secret) = value.split_once(' ').ok_or(OperationsError::Unauthorized)?;
    if !scheme.eq_ignore_ascii_case("Bearer") || secret.is_empty() {
        return Err(OperationsError::Unauthorized);
    }
    Ok(secret)
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct StartQuery {
    target_timestamp: i64,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct CurrentQuery {
    r#type: String,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct ListQuery {
    limit: Option<i64>,
}
async fn start(
    State(s): State<OperationsStore>,
    h: HeaderMap,
    Query(q): Query<StartQuery>,
) -> Result<Json<Value>> {
    let keys = h.get_all("idempotency-key").iter().collect::<Vec<_>>();
    if keys.len() != 1 {
        return Err(OperationsError::Invalid);
    }
    let key = keys[0].to_str().map_err(|_| OperationsError::Invalid)?;
    Ok(Json(
        json!({"success":true,"data":s.start_cleanup(bearer(&h)?,q.target_timestamp,key).await?}),
    ))
}
async fn current(
    State(s): State<OperationsStore>,
    h: HeaderMap,
    Query(q): Query<CurrentQuery>,
) -> Result<Json<Value>> {
    if q.r#type != "log_cleanup" {
        return Err(OperationsError::Invalid);
    }
    Ok(Json(
        json!({"success":true,"data":s.tasks(bearer(&h)?,None,1).await?.pop()}),
    ))
}
async fn list(
    State(s): State<OperationsStore>,
    h: HeaderMap,
    Query(q): Query<ListQuery>,
) -> Result<Json<Value>> {
    Ok(Json(
        json!({"success":true,"data":s.tasks(bearer(&h)?,None,q.limit.unwrap_or(20)).await?}),
    ))
}
async fn get_task(
    State(s): State<OperationsStore>,
    h: HeaderMap,
    Path(id): Path<String>,
) -> Result<Json<Value>> {
    let item = s
        .tasks(bearer(&h)?, Some(&id), 1)
        .await?
        .pop()
        .ok_or(OperationsError::NotFound)?;
    Ok(Json(json!({"success":true,"data":item})))
}
async fn instances(State(s): State<OperationsStore>, h: HeaderMap) -> Result<Json<Value>> {
    Ok(Json(s.instances(bearer(&h)?).await?))
}
pub fn router(store: OperationsStore) -> Router {
    Router::new()
        .route("/api/system-task/log-cleanup", post(start))
        .route("/api/system-task/current", get(current))
        .route("/api/system-task/list", get(list))
        .route("/api/system-task/{id}", get(get_task))
        .route("/core/v1/admin/instances", get(instances))
        .with_state(store)
        .layer(axum::middleware::from_fn(
            |request: axum::extract::Request, next: axum::middleware::Next| async move {
                let mut response = next.run(request).await;
                response
                    .headers_mut()
                    .insert(header::CACHE_CONTROL, "private, no-store".parse().unwrap());
                response
                    .headers_mut()
                    .insert(header::VARY, "Authorization".parse().unwrap());
                response
            },
        ))
}

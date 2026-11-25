//! Database-authoritative fixed-window request limits, independent of funds.
use crate::identity::{self, CredentialKind, IdentityError};
use axum::{
    Json, Router,
    extract::State,
    http::{HeaderMap, StatusCode, header},
    response::{IntoResponse, Response},
    routing::get,
};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use sqlx::{PgConnection, PgPool, Row};
use std::collections::BTreeMap;

pub const SCHEMA: &str = include_str!("../schema/request_limits.sql");

#[derive(Debug, PartialEq, Eq)]
pub enum AdmissionError {
    Limited { retry_after_seconds: u64 },
    Storage,
    Invalid,
}
impl std::fmt::Display for AdmissionError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "request admission: {self:?}")
    }
}
impl std::error::Error for AdmissionError {}

mod file;
pub use file::RoutingPolicyFile;

/// Call before precharge/upstream, and again before switching to another actual
/// routing group. The same user/request ID is charged once per scope/window.
/// Database failure is fail-closed; no admission can be inferred from a timeout.
pub async fn admit(
    pool: &PgPool,
    user_id: i64,
    group: &str,
    request_id: &str,
) -> Result<(), AdmissionError> {
    let mut connection = pool.acquire().await.map_err(|_| AdmissionError::Storage)?;
    admit_in_transaction(&mut connection, user_id, group, request_id).await
}

/// The assistant already holds identity/funds locks. Join that transaction so
/// admission and its durable worker reservation commit together.
pub(crate) async fn admit_in_transaction(
    connection: &mut PgConnection,
    user_id: i64,
    group: &str,
    request_id: &str,
) -> Result<(), AdmissionError> {
    if user_id <= 0
        || !crate::routing::valid_group_name(group)
        || request_id.is_empty()
        || request_id.len() > 128
        || !request_id.bytes().all(|b| b.is_ascii_graphic())
    {
        return Err(AdmissionError::Invalid);
    }
    let row =
        sqlx::query("SELECT allowed,retry_after_seconds FROM core_request_limits.admit($1,$2,$3)")
            .bind(user_id)
            .bind(group)
            .bind(request_id)
            .fetch_one(connection)
            .await
            .map_err(|_| AdmissionError::Storage)?;
    if row
        .try_get::<bool, _>("allowed")
        .map_err(|_| AdmissionError::Storage)?
    {
        Ok(())
    } else {
        let seconds: i64 = row
            .try_get("retry_after_seconds")
            .map_err(|_| AdmissionError::Storage)?;
        Err(AdmissionError::Limited {
            retry_after_seconds: seconds.try_into().map_err(|_| AdmissionError::Storage)?,
        })
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct Policy {
    pub enabled: bool,
    pub window_seconds: u32,
    pub user_limit: Option<u32>,
    pub group_limits: BTreeMap<String, u32>,
}
#[derive(Debug)]
enum SettingsError {
    Unauthorized,
    Forbidden,
    Storage,
}
impl From<sqlx::Error> for SettingsError {
    fn from(_: sqlx::Error) -> Self {
        Self::Storage
    }
}
impl From<IdentityError> for SettingsError {
    fn from(value: IdentityError) -> Self {
        match value {
            IdentityError::Unauthorized => Self::Unauthorized,
            IdentityError::Forbidden => Self::Forbidden,
            _ => Self::Storage,
        }
    }
}
impl IntoResponse for SettingsError {
    fn into_response(self) -> Response {
        let (status, code) = match self {
            Self::Unauthorized => (StatusCode::UNAUTHORIZED, "REQUEST_LIMITS_UNAUTHORIZED"),
            Self::Forbidden => (StatusCode::FORBIDDEN, "REQUEST_LIMITS_FORBIDDEN"),
            Self::Storage => (
                StatusCode::SERVICE_UNAVAILABLE,
                "REQUEST_LIMITS_UNAVAILABLE",
            ),
        };
        (status, Json(json!({"error":{"code":code}}))).into_response()
    }
}

fn bearer(headers: &HeaderMap) -> Result<&str, SettingsError> {
    let mut values = headers.get_all(header::AUTHORIZATION).iter();
    let value = values
        .next()
        .and_then(|v| v.to_str().ok())
        .ok_or(SettingsError::Unauthorized)?;
    if values.next().is_some() {
        return Err(SettingsError::Unauthorized);
    }
    let (scheme, secret) = value.split_once(' ').ok_or(SettingsError::Unauthorized)?;
    if !scheme.eq_ignore_ascii_case("Bearer") || secret.is_empty() {
        return Err(SettingsError::Unauthorized);
    }
    Ok(secret)
}
async fn administrator(c: &mut PgConnection, secret: &str) -> Result<i64, SettingsError> {
    let actor = identity::authorize_in_transaction(c, secret).await?;
    if actor.credential_kind != CredentialKind::Session {
        return Err(SettingsError::Forbidden);
    }
    let role: String = sqlx::query_scalar(
        "SELECT platform_role FROM core_identity.users WHERE id=$1 AND active FOR SHARE",
    )
    .bind(actor.user_id)
    .fetch_one(c)
    .await?;
    if role != "superadmin" {
        return Err(SettingsError::Forbidden);
    }
    Ok(actor.user_id)
}
async fn policy_snapshot(c: &mut PgConnection) -> Result<Value, SettingsError> {
    let row = sqlx::query("SELECT revision,enabled,window_seconds,user_limit,group_limits FROM core_request_limits.policy WHERE singleton")
        .fetch_one(c).await?;
    Ok(
        json!({"version":1,"revision":row.try_get::<i64,_>("revision")?,"policy":{
            "enabled":row.try_get::<bool,_>("enabled")?,
            "window_seconds":row.try_get::<i32,_>("window_seconds")?,
            "user_limit":row.try_get::<Option<i32>,_>("user_limit")?,
            "group_limits":row.try_get::<Value,_>("group_limits")?
        }}),
    )
}
pub(crate) async fn global_auto_groups(c: &mut PgConnection) -> Result<Vec<String>, sqlx::Error> {
    sqlx::query_scalar("SELECT groups FROM core_request_limits.auto_policy WHERE singleton")
        .fetch_one(c)
        .await
}
async fn auto_snapshot(c: &mut PgConnection) -> Result<Value, SettingsError> {
    let row =
        sqlx::query("SELECT revision,groups FROM core_request_limits.auto_policy WHERE singleton")
            .fetch_one(c)
            .await?;
    Ok(
        json!({"version":1,"revision":row.try_get::<i64,_>("revision")?,
        "groups":row.try_get::<Vec<String>,_>("groups")?}),
    )
}
async fn get_policy(
    State(pool): State<PgPool>,
    headers: HeaderMap,
) -> Result<Json<Value>, SettingsError> {
    let mut tx = pool.begin().await?;
    administrator(&mut tx, bearer(&headers)?).await?;
    let snapshot = policy_snapshot(&mut tx).await?;
    tx.commit().await?;
    Ok(Json(snapshot))
}
async fn get_auto(
    State(pool): State<PgPool>,
    headers: HeaderMap,
) -> Result<Json<Value>, SettingsError> {
    let mut tx = pool.begin().await?;
    administrator(&mut tx, bearer(&headers)?).await?;
    let snapshot = auto_snapshot(&mut tx).await?;
    tx.commit().await?;
    Ok(Json(snapshot))
}
pub fn router(pool: PgPool) -> Router {
    Router::new()
        .route("/core/v1/admin/request-limits", get(get_policy))
        .route("/core/v1/admin/routing/auto-groups", get(get_auto))
        .with_state(pool)
        .layer(axum::middleware::from_fn(
            |request: axum::extract::Request, next: axum::middleware::Next| async move {
                let mut response = next.run(request).await;
                response
                    .headers_mut()
                    .insert(header::CACHE_CONTROL, "private, no-store".parse().unwrap());
                response
            },
        ))
}

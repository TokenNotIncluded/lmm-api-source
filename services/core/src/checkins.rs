//! Historical offline policy/import tools. Fresh installations use Go checkins.
mod owner;
mod types;
pub use types::*;

use crate::identity;
#[cfg(test)]
use chrono::NaiveDate;
use chrono::{DateTime, Utc};
use serde_json::{Value, json};
use sqlx::{PgConnection, PgPool, Postgres, Row, Transaction, types::Json};

pub const SCHEMA: &str = include_str!("../schema/checkins.sql");

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Error {
    Invalid,
    Unauthorized,
    Forbidden,
    NotFound,
    Conflict,
    Unavailable,
    Retry,
}
pub type Result<T> = std::result::Result<T, Error>;

impl std::fmt::Display for Error {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(formatter, "checkin {self:?}")
    }
}
impl std::error::Error for Error {}

impl From<sqlx::Error> for Error {
    fn from(error: sqlx::Error) -> Self {
        match error.as_database_error().and_then(|e| e.code()).as_deref() {
            Some("40001" | "40P01" | "55P03" | "23505") => Self::Retry,
            _ => Self::Unavailable,
        }
    }
}
impl From<identity::IdentityError> for Error {
    fn from(error: identity::IdentityError) -> Self {
        match error {
            identity::IdentityError::Invalid => Self::Invalid,
            identity::IdentityError::Unauthorized => Self::Unauthorized,
            identity::IdentityError::Forbidden => Self::Forbidden,
            identity::IdentityError::Conflict => Self::Conflict,
            identity::IdentityError::Storage => Self::Unavailable,
        }
    }
}

#[derive(Clone)]
pub struct CheckinStore {
    pool: PgPool,
}

impl CheckinStore {
    pub fn new(pool: PgPool) -> Self {
        Self { pool }
    }

    async fn begin(&self) -> Result<Transaction<'_, Postgres>> {
        let mut tx = self.pool.begin().await?;
        sqlx::query("SET TRANSACTION ISOLATION LEVEL SERIALIZABLE")
            .execute(&mut *tx)
            .await?;
        sqlx::query("SET LOCAL synchronous_commit=on")
            .execute(&mut *tx)
            .await?;
        Ok(tx)
    }
}

async fn now(c: &mut PgConnection) -> Result<i64> {
    Ok(
        sqlx::query_scalar("SELECT floor(extract(epoch FROM transaction_timestamp()))::bigint")
            .fetch_one(c)
            .await?,
    )
}
pub fn calendar_day(timestamp: i64) -> Result<String> {
    let shifted = timestamp
        .checked_add(i64::from(CALENDAR_OFFSET_SECONDS))
        .ok_or(Error::Invalid)?;
    Ok(DateTime::<Utc>::from_timestamp(shifted, 0)
        .ok_or(Error::Invalid)?
        .format("%Y-%m-%d")
        .to_string())
}
fn valid_key(key: &str) -> Result<()> {
    if !(16..=80).contains(&key.len())
        || !key
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b"_.:-".contains(&b))
    {
        return Err(Error::Invalid);
    }
    Ok(())
}
fn integer(value: &str) -> Result<i64> {
    let parsed = value.parse::<i64>().map_err(|_| Error::Invalid)?;
    if parsed.to_string() != value {
        return Err(Error::Invalid);
    }
    Ok(parsed)
}
async fn policy(c: &mut PgConnection) -> Result<Option<PublishedPolicy>> {
    let row = sqlx::query("SELECT r.revision,r.policy FROM core_checkins.active_policy a JOIN core_checkins.policy_revisions r ON r.revision=a.revision WHERE a.singleton")
        .fetch_optional(c).await?;
    row.map(|r| {
        Ok(PublishedPolicy {
            revision: r.try_get("revision")?,
            policy: r.try_get::<Json<Policy>, _>("policy")?.0,
        })
    })
    .transpose()
}
#[cfg(test)]
mod history_runtime;
#[cfg(test)]
#[path = "checkins/history_tests.rs"]
mod history_tests;

// Read-only historical import collision check; never issues rewards.
async fn daily(c: &mut PgConnection, account: i64, day: &str) -> Result<Option<DailyRecord>> {
    Ok(sqlx::query_scalar::<_, Json<DailyRecord>>(
        "SELECT result FROM core_checkins.days WHERE account_id=$1 AND checkin_date=$2::text::date",
    )
    .bind(account)
    .bind(day)
    .fetch_optional(c)
    .await?
    .map(|v| v.0))
}

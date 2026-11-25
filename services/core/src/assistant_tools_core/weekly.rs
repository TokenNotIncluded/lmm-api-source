//! One durable decision per UTC week. A status read never evaluates eligibility.
use super::{Error, context::VerifiedToolContext};
use serde_json::{Value, json};
use sqlx::{PgConnection, Row, postgres::PgRow};

#[cfg(test)]
pub(crate) const STATUS_TOOL_NAME: &str = "get_weekly_discount_status";
#[cfg(test)]
pub(crate) const STATUS_TOOL_SCHEMA: &str =
    r#"{"additionalProperties":false,"properties":{},"type":"object"}"#;

pub(crate) mod agent;
pub(super) mod browser;
mod intent;

pub(super) fn week_start(stamp: i64) -> i64 {
    (stamp - 345_600).div_euclid(604_800) * 604_800 + 345_600
}
pub(super) async fn now(c: &mut PgConnection) -> Result<i64, Error> {
    Ok(
        sqlx::query_scalar("SELECT floor(extract(epoch FROM clock_timestamp()))::bigint")
            .fetch_one(c)
            .await?,
    )
}
pub(super) async fn lock_owner(c: &mut PgConnection, user: i64) -> Result<(), Error> {
    sqlx::query("SELECT pg_advisory_xact_lock(hashtextextended('lmm.weekly.'||$1::text,0))")
        .bind(user)
        .execute(c)
        .await?;
    Ok(())
}
pub(super) async fn policy(c: &mut PgConnection) -> Result<(bool, i16), Error> {
    Ok(sqlx::query_as(
        "SELECT enabled,maximum_percent FROM core_weekly.lock_weekly_discount_policy()",
    )
    .fetch_one(c)
    .await?)
}
pub(super) fn projection(row: &PgRow) -> Result<Value, Error> {
    let id: String = row.try_get("id")?;
    let week: i64 = row.try_get("week_start")?;
    Ok(json!({
        "id":id, "week_start":week, "expires_at":week+604_800,
        "status":row.try_get::<String,_>("status")?,
        "discount_percent":row.try_get::<i16,_>("discount_percent")?,
        "created_at":row.try_get::<i64,_>("created_at")?,
        "claimed_at":row.try_get::<Option<i64>,_>("claimed_at")?,
        "browser_path":format!("/api/user/auth/assistant/weekly-discounts/{id}")
    }))
}
pub(super) async fn status_in_transaction(
    c: &mut PgConnection,
    context: &VerifiedToolContext,
) -> Result<Value, Error> {
    let actor = context.revalidate(c).await?;
    let week = week_start(now(c).await?);
    let row = sqlx::query(
        "SELECT * FROM core_weekly.weekly_discounts WHERE user_id=$1 AND week_start=$2",
    )
    .bind(actor.user)
    .bind(week)
    .fetch_optional(c)
    .await?;
    match row {
        Some(row) => Ok(
            json!({"ok":true,"read_only":true,"decision_used":true,"current_utc_week":week,"status":row.try_get::<String,_>("status")?,"decision":projection(&row)?}),
        ),
        None => Ok(
            json!({"ok":true,"read_only":true,"decision_used":false,"current_utc_week":week,"status":"none","week_start":week,"decision":null}),
        ),
    }
}
pub(crate) async fn check_ready(c: &mut PgConnection) -> Result<(), Error> {
    sqlx::query("SELECT d.id,d.origin,d.week_start,d.operation_id,d.web_session_sid,d.claim_nonce_sha256,d.claim_nonce_expires_at FROM core_weekly.weekly_discounts d WHERE FALSE").execute(&mut *c).await?;
    let ready: bool = sqlx::query_scalar(
        "SELECT count(*)=2 FROM pg_trigger WHERE tgenabled='O' AND tgdeferrable AND tginitdeferred
           AND ((tgrelid='core_weekly.weekly_discounts'::regclass AND tgname='weekly_discount_binding')
             OR (tgrelid='core_assistant.agent_tool_operations'::regclass AND tgname='weekly_discount_tool_binding'))")
        .fetch_one(&mut *c).await?;
    let policy_rows: i64 =
        sqlx::query_scalar("SELECT count(*) FROM core_weekly.weekly_discount_policy")
            .fetch_one(c)
            .await?;
    if !ready || policy_rows != 1 {
        return Err(Error::Unavailable);
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn utc_week_uses_monday_and_never_local_timezone() {
        assert_eq!(week_start(345_600), 345_600);
        assert_eq!(week_start(345_600 + 604_799), 345_600);
        assert_eq!(week_start(345_600 + 604_800), 950_400);
        assert_eq!(week_start(345_599), -259_200);
    }
}

#[cfg(test)]
mod registry_tests;

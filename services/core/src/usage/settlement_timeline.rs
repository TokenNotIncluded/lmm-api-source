//! Read the original transactional charging commands only for an authorized detail view.
use super::{Error, ValidatedRequest, safe};
use crate::billing::charging::Change;
use serde_json::{Value, json};
use sqlx::{PgConnection, Row};

pub(super) async fn read(
    db: &mut PgConnection,
    request: &ValidatedRequest,
    now: i64,
) -> Result<Value, Error> {
    // The surrounding transaction validated these same command identities and amounts.
    // Preserve repeated reserve operations as absolute ceilings, never additive costs.
    let mut rows = sqlx::query(
        "SELECT command->'change' AS change, floor(extract(epoch FROM created_at))::bigint AS at
         FROM core_charging.ledger_commands WHERE request_id=$1
         ORDER BY created_at DESC, operation_id DESC LIMIT 201",
    )
    .bind(&request.id)
    .fetch_all(&mut *db)
    .await?;
    let truncated = rows.len() > 200;
    rows.truncate(200);
    let mut events = Vec::new();
    for row in rows.into_iter().rev() {
        let at: i64 = safe(row.try_get("at")?)?;
        let change: Change =
            serde_json::from_value(row.try_get("change")?).map_err(|_| Error::Storage)?;
        let mut record = |kind: &str, amount: i64| -> Result<(), Error> {
            events.push(json!({"kind":kind,"at":at,"credits":safe(amount)?}));
            Ok(())
        };
        match change {
            Change::Reserve { total } => record("reserve_total", total)?,
            Change::Settle { amount } => {
                record("captured", amount)?;
                let unused = request
                    .reserved
                    .checked_sub(amount)
                    .filter(|n| *n >= 0)
                    .ok_or(Error::Storage)?;
                if unused > 0 {
                    record("released", unused)?;
                }
            }
            Change::Release => record("released", request.reserved)?,
            Change::Refund { amount } => record("refunded", amount)?,
        }
    }
    let pending = request.held();
    let last_hold_check: Option<i64> = if pending {
        sqlx::query_scalar(
            "SELECT floor(extract(epoch FROM max(created_at)))::bigint FROM core_charging.usage_events
             WHERE request_id=$1 AND event_id IN ('unknown','recovery')
               AND detail @> '{\"kind\":\"unknown_provider_usage\",\"hold_retained\":true}'::jsonb",
        ).bind(&request.id).fetch_one(db).await?
    } else {
        None
    };
    Ok(json!({
        "events":events, "truncated":truncated,
        "wait_seconds_at_read":pending.then(|| now.saturating_sub(request.at).max(0)),
        "hold_retained_at":last_hold_check,
    }))
}

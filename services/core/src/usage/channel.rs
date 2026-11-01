//! Native channel consumption, read from one SQL snapshot without ledger writes.
use super::{Error, MAX_REQUESTS, ValidatedRequest};
use crate::{billing::charging::contract, routing::DeliveryState};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use sqlx::{PgConnection, Row};
use std::collections::BTreeMap;
use tokio_stream::StreamExt;

#[derive(Debug, Serialize)]
pub(crate) struct ChannelUsage {
    pub scope: &'static str,
    pub start_timestamp: i64,
    pub end_timestamp: i64,
    pub gross_settled_credits: String,
    pub refunded_credits: String,
    pub net_used_credits: String,
    pub settled_requests: String,
    pub pending_potential_requests: String,
    pub unknown_provenance_requests: String,
    pub credits_per_usd: &'static str,
    pub historical_coverage: bool,
}

#[derive(Debug)]
pub(crate) enum ChannelUsageError {
    Storage,
    InvalidReceipt,
    TooManyRequests,
    TooManyReceipts,
    HistoryAmbiguous,
}
impl From<sqlx::Error> for ChannelUsageError {
    fn from(_: sqlx::Error) -> Self {
        Self::Storage
    }
}
impl From<Error> for ChannelUsageError {
    fn from(error: Error) -> Self {
        match error {
            Error::TooManyBuckets => Self::TooManyReceipts,
            _ => Self::InvalidReceipt,
        }
    }
}

// All window requests are scanned: without provenance a request cannot safely
// be excluded merely because the requested channel is absent from today's config.
// Each collection has an extra row to make overflow an error, not partial success.
const SELECT: &str = r#"
 SELECT r.*, a.kind AS payer_kind, coalesce(k.name,r.oauth_source->>'client_id','') AS token_name,
   e.detail AS final_event, p.detail AS provenance, m.detail AS admission,
   (SELECT coalesce(jsonb_agg(to_jsonb(c)), '[]'::jsonb) FROM
      (SELECT operation_id,command FROM core_charging.ledger_commands
       WHERE request_id=r.id ORDER BY operation_id LIMIT 2049) c) AS commands,
   (SELECT coalesce(jsonb_agg(to_jsonb(f)), '[]'::jsonb) FROM
      (SELECT refund_id,amount FROM core_charging.refunds
       WHERE request_id=r.id ORDER BY refund_id LIMIT 2049) f) AS refunds,
   (SELECT coalesce(jsonb_agg(to_jsonb(t)), '[]'::jsonb) FROM
      (SELECT event_id,detail FROM core_charging.usage_events WHERE request_id=r.id
       AND (event_id LIKE 'route-dispatch:%' OR event_id LIKE 'route-failure:%')
       ORDER BY event_id LIMIT 17) t) AS attempts
 FROM core_charging.requests r
 JOIN core_identity.accounts a ON a.id=r.payer_account_id
 LEFT JOIN core_identity.api_key_settings k ON k.credential_id=r.key_id
 LEFT JOIN core_charging.usage_events e ON e.request_id=r.id AND e.event_id='final'
 LEFT JOIN core_charging.usage_events p ON p.request_id=r.id AND p.event_id='reservation-contract:v1'
 LEFT JOIN core_charging.usage_events m ON m.request_id=r.id AND m.event_id='reservation-admission:v1'
 WHERE ($3 OR r.authorized_at BETWEEN $1 AND $2)
 ORDER BY r.authorized_at DESC,r.id DESC LIMIT 100001
"#;

#[derive(Deserialize)]
struct AttemptEvent {
    event_id: String,
    detail: Value,
}
#[derive(Deserialize)]
struct Dispatch {
    kind: String,
    number: u8,
    route_id: String,
    wire_sha256: String,
}
#[derive(Deserialize)]
struct Failure {
    kind: String,
    number: u8,
    route_id: String,
    failure: crate::relay::AttemptFailure,
}

/// A dispatch is written before send; earlier attempts must have durable proof
/// of no acceptance before a later attempt can own the logical settlement.
fn final_channel(proof: &contract::ReservationContract, events: &[AttemptEvent]) -> Option<u64> {
    let mut dispatches = BTreeMap::new();
    let mut failures = BTreeMap::new();
    for event in events {
        if event.event_id.starts_with("route-dispatch:") {
            let d: Dispatch = serde_json::from_value(event.detail.clone()).ok()?;
            if d.kind != "model_route_dispatch"
                || !(1..=8).contains(&d.number)
                || event.event_id != format!("route-dispatch:{}", d.number)
                || proof
                    .dispatched_channel(&d.route_id, &d.wire_sha256)
                    .is_none()
            {
                return None;
            }
            dispatches.insert(d.number, d);
        } else {
            let f: Failure = serde_json::from_value(event.detail.clone()).ok()?;
            if f.kind != "model_route_failure"
                || event.event_id != format!("route-failure:{}", f.number)
            {
                return None;
            }
            failures.insert(f.number, f);
        }
    }
    let last = *dispatches.last_key_value()?.0;
    if dispatches.len() != usize::from(last)
        || failures
            .iter()
            .any(|(n, f)| dispatches.get(n).is_none_or(|d| d.route_id != f.route_id))
    {
        return None;
    }
    for number in 1..last {
        let f = failures.get(&number)?;
        if !matches!(
            f.failure.failure.delivery,
            DeliveryState::NotSent | DeliveryState::Rejected
        ) || (f.failure.failure.delivery == DeliveryState::Rejected
            && (f.failure.http_status.is_none() || f.failure.response_sha256.is_none()))
        {
            return None;
        }
    }
    let d = dispatches.get(&last)?;
    proof.dispatched_channel(&d.route_id, &d.wire_sha256)
}

/// Caller validates the 31-day window and owns the read-only snapshot. No
/// archived logs, live provider calls, current routes or wallet balances enter it.
pub(crate) async fn channel_usage(
    c: &mut PgConnection,
    channel_id: u64,
    start: i64,
    end: i64,
) -> Result<ChannelUsage, ChannelUsageError> {
    aggregate(c, channel_id, start, end, "native_channel_window").await
}

/// Own Native consumption across all stored facts visible in this snapshot,
/// without a transaction-start or 31-day cutoff. The timestamps describe the
/// full supported time domain; the caller adds any inheritance baseline.
pub(crate) async fn channel_totals(
    c: &mut PgConnection,
    channel_id: u64,
) -> Result<ChannelUsage, ChannelUsageError> {
    aggregate(c, channel_id, 0, 253402300799, "native_channel_totals").await
}

async fn aggregate(
    c: &mut PgConnection,
    channel_id: u64,
    start: i64,
    end: i64,
    scope: &'static str,
) -> Result<ChannelUsage, ChannelUsageError> {
    let ambiguous: Option<bool> =
        sqlx::query_scalar("SELECT history_ambiguous OR retired_at IS NOT NULL FROM core_channels.identities WHERE id=$1")
            .bind(channel_id as i64)
            .fetch_optional(&mut *c)
            .await?;
    if ambiguous == Some(true) {
        return Err(ChannelUsageError::HistoryAmbiguous);
    }
    let mut gross = 0_u128;
    let mut refunded = 0_u128;
    let mut settled = 0_u64;
    let mut pending = 0_u64;
    let mut unknown = 0_u64;
    let mut count = 0;
    let mut rows = sqlx::query(SELECT)
        .bind(start)
        .bind(end)
        .bind(scope == "native_channel_totals")
        .fetch(c);
    while let Some(row) = rows.next().await {
        count += 1;
        if count > MAX_REQUESTS {
            return Err(ChannelUsageError::TooManyRequests);
        }
        let row = row?;
        let value = row.try_get("provenance")?;
        let marker = row.try_get("admission")?;
        let fingerprint: Vec<u8> = row.try_get("fingerprint")?;
        let spec: Vec<u8> = row.try_get("spec_digest")?;
        let events: Vec<AttemptEvent> = serde_json::from_value(row.try_get("attempts")?)
            .map_err(|_| ChannelUsageError::InvalidReceipt)?;
        if events.len() > 16 {
            return Err(ChannelUsageError::TooManyReceipts);
        }
        let r = ValidatedRequest::from_row(row)?;
        if !r.settled() && !r.held() {
            continue;
        }
        let (owner, potential) = if let Some(image) = &r.price.image {
            // Image tariff restoration validates its independent upstream binding.
            let quote = image
                .clone()
                .restore()
                .map_err(|_| ChannelUsageError::InvalidReceipt)?;
            let owner = quote.binding().upstream_id;
            (Some(owner), owner == channel_id)
        } else {
            match contract::read_attribution(value, marker, &r.price, &fingerprint, &spec) {
                Ok(Some(proof)) => (
                    final_channel(&proof, &events),
                    proof.potential_channel(channel_id),
                ),
                Ok(None) | Err(_) => {
                    unknown += 1;
                    continue;
                }
            }
        };
        if !potential {
            continue;
        }
        if r.settled() {
            match owner {
                Some(owner) if owner == channel_id => {
                    gross += r.amounts.gross as u128;
                    refunded += r.amounts.refunded as u128;
                    settled += 1;
                }
                Some(_) => (),
                None => unknown += 1,
            }
        } else if r.held() {
            pending += 1;
        }
    }
    // MAX_REQUESTS * JS_SAFE fits u128; each nonnegative amount and each refund
    // bound has already passed the shared receipt validator.
    Ok(ChannelUsage {
        scope,
        start_timestamp: start,
        end_timestamp: end,
        gross_settled_credits: gross.to_string(),
        refunded_credits: refunded.to_string(),
        net_used_credits: (gross - refunded).to_string(),
        settled_requests: settled.to_string(),
        pending_potential_requests: pending.to_string(),
        unknown_provenance_requests: unknown.to_string(),
        credits_per_usd: "500000",
        historical_coverage: false,
    })
}

//! Derived Native channel contributions. Financial receipts remain authoritative.
use super::channel::{ChannelUsage, ChannelUsageError};
use super::receipts::{InvalidReceipt, ReceiptValidation, Refund, StoredCommand};
use crate::{
    billing::charging::{Charge, Price, contract},
    routing::DeliveryState,
};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use sha2::{Digest, Sha256};
use sqlx::{PgConnection, Row};
use std::collections::{BTreeMap, BTreeSet};
use tokio_stream::StreamExt;

#[cfg(test)]
#[path = "../../tests/common/channel_projection_image.rs"]
mod image_acceptance;

#[derive(Debug)]
pub(crate) enum ProjectionError {
    Database(sqlx::Error),
    Charging(crate::billing::charging::Error),
    InvalidReceipt,
}
impl From<sqlx::Error> for ProjectionError {
    fn from(error: sqlx::Error) -> Self {
        Self::Database(error)
    }
}
impl From<InvalidReceipt> for ProjectionError {
    fn from(_: InvalidReceipt) -> Self {
        Self::InvalidReceipt
    }
}
impl From<ProjectionError> for ChannelUsageError {
    fn from(error: ProjectionError) -> Self {
        match error {
            ProjectionError::Database(_) => Self::Storage,
            ProjectionError::Charging(_) => Self::Storage,
            ProjectionError::InvalidReceipt => Self::InvalidReceipt,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, sqlx::FromRow)]
struct Contribution {
    request_id: String,
    channel_id: i64,
    authorized_at: i64,
    gross_credits: i64,
    refunded_credits: i64,
    settled_requests: i64,
    pending_requests: i64,
    unknown_requests: i64,
}

struct RequestFacts<'a> {
    id: &'a str,
    at: i64,
    state: &'a str,
    price: &'a Price,
    gross: i64,
    refunded: i64,
}

fn derive_contributions(
    facts: RequestFacts<'_>,
    proof: Option<&contract::ReservationContract>,
    attempts: &[AttemptEvent],
) -> Result<Vec<Contribution>, ProjectionError> {
    let RequestFacts {
        id,
        at,
        state,
        price,
        gross,
        refunded,
    } = facts;
    if !matches!(state, "settled" | "reserved" | "streaming" | "reconcile") {
        return Ok(Vec::new());
    }
    let settled = state == "settled";
    let (owner, candidates) = if let Some(image) = &price.image {
        let quote = image
            .clone()
            .restore()
            .map_err(|_| ProjectionError::InvalidReceipt)?;
        let id = quote.binding().upstream_id;
        (Some(id), BTreeSet::from([id]))
    } else if let Some(proof) = proof {
        (
            final_channel(proof, attempts),
            proof.channel_ids().collect(),
        )
    } else {
        (None, BTreeSet::from([0]))
    };
    let mut contributions = Vec::new();
    for channel in candidates {
        if settled && owner.is_some_and(|owner| owner != channel) {
            continue;
        }
        let known = settled && owner == Some(channel);
        let unknown = channel == 0 || (settled && owner.is_none());
        contributions.push(Contribution {
            request_id: id.to_owned(),
            channel_id: channel as i64,
            authorized_at: at,
            gross_credits: if known { gross } else { 0 },
            refunded_credits: if known { refunded } else { 0 },
            settled_requests: i64::from(known),
            pending_requests: i64::from(!settled && !unknown),
            unknown_requests: i64::from(unknown),
        });
    }
    Ok(contributions)
}

async fn attempts(c: &mut PgConnection, id: &str) -> Result<Vec<AttemptEvent>, ProjectionError> {
    let rows = sqlx::query("SELECT event_id,detail FROM core_charging.usage_events WHERE request_id=$1 AND (event_id LIKE 'route-dispatch:%' OR event_id LIKE 'route-failure:%') ORDER BY event_id")
        .bind(id).fetch_all(c).await?;
    rows.into_iter()
        .map(|row| {
            Ok(AttemptEvent {
                event_id: row.try_get("event_id")?,
                detail: row.try_get("detail")?,
            })
        })
        .collect()
}

/// Called only after the charging engine successfully writes its typed result.
/// No money receipt re-read, replay, charge calculation or transaction ownership.
pub(crate) async fn record_charge(
    c: &mut PgConnection,
    charge: &Charge,
) -> Result<(), ProjectionError> {
    if charge.state == "released" {
        return replace_contributions(c, &charge.id, Vec::new()).await;
    }
    let proof = if charge.price.image.is_none() {
        match contract::load(c, charge).await {
            Ok(proof) => proof,
            Err(
                crate::billing::charging::Error::Invalid
                | crate::billing::charging::Error::Conflict,
            ) => None,
            Err(error) => return Err(ProjectionError::Charging(error)),
        }
    } else {
        None
    };
    let attempts = if proof.is_some() {
        attempts(c, &charge.id).await?
    } else {
        Vec::new()
    };
    let new = derive_contributions(
        RequestFacts {
            id: &charge.id,
            at: charge.authorized_at,
            state: &charge.state,
            price: &charge.price,
            gross: charge.settled,
            refunded: charge.refunded,
        },
        proof.as_ref(),
        &attempts,
    )?;
    replace_contributions(c, &charge.id, new).await
}

struct AttributionEvidence {
    provenance: Option<Value>,
    admission: Option<Value>,
    fingerprint: Vec<u8>,
    spec: Vec<u8>,
    attempts: Vec<AttemptEvent>,
}

/// Receipts are streamed for a bounded batch of requests, with no external-page
/// receipt cap. The exact same validator consumes bounded HTTP rows above.
async fn validated_batch(
    c: &mut PgConnection,
    ids: &[String],
) -> Result<Vec<Contribution>, ProjectionError> {
    let rows = sqlx::query("SELECT r.*,(SELECT pc.detail FROM core_charging.usage_events pc WHERE pc.request_id=r.id AND pc.event_id='reservation-contract:v1') AS pricing_contract,    (SELECT pa.detail FROM core_charging.usage_events pa WHERE pa.request_id=r.id AND pa.event_id='reservation-admission:v1') AS pricing_admission,    (SELECT po.detail FROM core_charging.usage_events po WHERE po.request_id=r.id AND po.event_id='provider-observation:terminal') AS pricing_observation,a.kind AS payer_kind,coalesce(k.name,r.oauth_source->>'client_id','') AS token_name,e.detail AS final_event,p.detail AS provenance,m.detail AS admission FROM core_charging.requests r JOIN core_identity.accounts a ON a.id=r.payer_account_id LEFT JOIN core_identity.api_key_settings k ON k.credential_id=r.key_id LEFT JOIN core_charging.usage_events e ON e.request_id=r.id AND e.event_id='final' LEFT JOIN core_charging.usage_events p ON p.request_id=r.id AND p.event_id='reservation-contract:v1' LEFT JOIN core_charging.usage_events m ON m.request_id=r.id AND m.event_id='reservation-admission:v1' WHERE r.id=ANY($1) ORDER BY r.id COLLATE \"C\"")
        .bind(ids).fetch_all(&mut *c).await?;
    let mut requests = BTreeMap::new();
    for row in rows {
        let id: String = row.try_get("id")?;
        let evidence = AttributionEvidence {
            provenance: row.try_get("provenance")?,
            admission: row.try_get("admission")?,
            fingerprint: row.try_get("fingerprint")?,
            spec: row.try_get("spec_digest")?,
            attempts: Vec::new(),
        };
        requests.insert(id, (ReceiptValidation::new(&row)?, evidence));
    }
    if requests.len() != ids.len() {
        return Err(ProjectionError::InvalidReceipt);
    }
    {
        let mut commands = sqlx::query("SELECT request_id,operation_id,command FROM core_charging.ledger_commands WHERE request_id=ANY($1) ORDER BY request_id COLLATE \"C\",operation_id COLLATE \"C\"")
            .bind(ids).fetch(&mut *c);
        while let Some(row) = commands.next().await {
            let row = row?;
            let id: String = row.try_get("request_id")?;
            let (validator, _) = requests
                .get_mut(&id)
                .ok_or(ProjectionError::InvalidReceipt)?;
            validator.command(StoredCommand {
                operation_id: row.try_get("operation_id")?,
                command: row.try_get("command")?,
            })?;
        }
    }
    {
        let mut refunds = sqlx::query("SELECT request_id,refund_id,amount FROM core_charging.refunds WHERE request_id=ANY($1) ORDER BY request_id COLLATE \"C\",refund_id COLLATE \"C\"")
            .bind(ids).fetch(&mut *c);
        while let Some(row) = refunds.next().await {
            let row = row?;
            let id: String = row.try_get("request_id")?;
            let (validator, _) = requests
                .get_mut(&id)
                .ok_or(ProjectionError::InvalidReceipt)?;
            validator.refund(Refund {
                refund_id: row.try_get("refund_id")?,
                amount: row.try_get("amount")?,
            })?;
        }
    }
    {
        let mut attempts = sqlx::query("SELECT request_id,event_id,detail FROM core_charging.usage_events WHERE request_id=ANY($1) AND (event_id LIKE 'route-dispatch:%' OR event_id LIKE 'route-failure:%') ORDER BY request_id COLLATE \"C\",event_id COLLATE \"C\"")
            .bind(ids).fetch(&mut *c);
        while let Some(row) = attempts.next().await {
            let row = row?;
            let id: String = row.try_get("request_id")?;
            let (_, evidence) = requests
                .get_mut(&id)
                .ok_or(ProjectionError::InvalidReceipt)?;
            evidence.attempts.push(AttemptEvent {
                event_id: row.try_get("event_id")?,
                detail: row.try_get("detail")?,
            });
        }
    }
    let mut contributions = Vec::new();
    for (validation, evidence) in requests.into_values() {
        let request = validation.finish()?;
        let proof = contract::read_attribution(
            evidence.provenance,
            evidence.admission,
            &request.price,
            &evidence.fingerprint,
            &evidence.spec,
        )
        .ok()
        .flatten();
        contributions.extend(derive_contributions(
            RequestFacts {
                id: &request.id,
                at: request.at,
                state: &request.state,
                price: &request.price,
                gross: request.amounts.gross,
                refunded: request.amounts.refunded,
            },
            proof.as_ref(),
            &evidence.attempts,
        )?);
    }
    Ok(contributions)
}

/// Explicit owner operation only. The offline caller holds source and
/// projection write fences and owns commit/rollback. Never called by startup.
pub(crate) async fn backfill_channel_usage(c: &mut PgConnection) -> Result<u64, ProjectionError> {
    sqlx::query("UPDATE core_channels.projection_state SET ready=FALSE WHERE singleton")
        .execute(&mut *c)
        .await?;
    sqlx::query("DELETE FROM core_channels.usage_rollups")
        .execute(&mut *c)
        .await?;
    sqlx::query("DELETE FROM core_channels.usage_contributions")
        .execute(&mut *c)
        .await?;
    let mut after: Option<String> = None;
    let mut count = 0_u64;
    loop {
        let ids: Vec<String> = sqlx::query_scalar("SELECT id FROM core_charging.requests WHERE $1::text IS NULL OR id COLLATE \"C\" > $1 COLLATE \"C\" ORDER BY id COLLATE \"C\" LIMIT 1000")
            .bind(&after).fetch_all(&mut *c).await?;
        if ids.is_empty() {
            break;
        }
        let contributions = validated_batch(c, &ids).await?;
        insert_contributions(c, &contributions).await?;
        count += ids.len() as u64;
        after = ids.last().cloned();
    }
    sqlx::query("INSERT INTO core_channels.usage_rollups(channel_id,day,bucket,gross_credits,refunded_credits,settled_requests,pending_requests,unknown_requests) SELECT channel_id,d.day,(get_byte(sha256(convert_to(request_id,'UTF8')),0)&63)::smallint,sum(gross_credits),sum(refunded_credits),sum(settled_requests),sum(pending_requests),sum(unknown_requests) FROM core_channels.usage_contributions CROSS JOIN LATERAL (VALUES(-1::bigint),(authorized_at/86400)) AS d(day) GROUP BY channel_id,d.day,(get_byte(sha256(convert_to(request_id,'UTF8')),0)&63)::smallint")
        .execute(&mut *c).await?;
    sqlx::query("DELETE FROM core_channels.projection_dirty")
        .execute(&mut *c)
        .await?;
    sqlx::query("UPDATE core_channels.projection_state SET ready=TRUE WHERE singleton")
        .execute(c)
        .await?;
    Ok(count)
}

async fn insert_contributions(
    c: &mut PgConnection,
    rows: &[Contribution],
) -> Result<(), ProjectionError> {
    if rows.is_empty() {
        return Ok(());
    }
    let json = serde_json::to_value(rows).map_err(|_| ProjectionError::InvalidReceipt)?;
    sqlx::query("INSERT INTO core_channels.usage_contributions(request_id,channel_id,authorized_at,gross_credits,refunded_credits,settled_requests,pending_requests,unknown_requests) SELECT request_id,channel_id,authorized_at,gross_credits,refunded_credits,settled_requests,pending_requests,unknown_requests FROM jsonb_to_recordset($1) AS r(request_id text,channel_id bigint,authorized_at bigint,gross_credits numeric,refunded_credits numeric,settled_requests bigint,pending_requests bigint,unknown_requests bigint)")
        .bind(json).execute(c).await?;
    Ok(())
}

#[derive(Default)]
struct Delta {
    gross: i128,
    refunded: i128,
    settled: i64,
    pending: i64,
    unknown: i64,
}
impl Delta {
    fn add(&mut self, contribution: &Contribution, sign: i64) {
        self.gross += i128::from(contribution.gross_credits) * i128::from(sign);
        self.refunded += i128::from(contribution.refunded_credits) * i128::from(sign);
        self.settled += contribution.settled_requests * sign;
        self.pending += contribution.pending_requests * sign;
        self.unknown += contribution.unknown_requests * sign;
    }
    fn zero(&self) -> bool {
        self.gross == 0
            && self.refunded == 0
            && self.settled == 0
            && self.pending == 0
            && self.unknown == 0
    }
}

async fn replace_contributions(
    c: &mut PgConnection,
    id: &str,
    new: Vec<Contribution>,
) -> Result<(), ProjectionError> {
    // The caller already holds the original request lock. All shared rollup
    // locks use deterministic channel/day/bucket order, after financial writes.
    let old = sqlx::query_as::<_, Contribution>("SELECT request_id,channel_id,authorized_at,gross_credits::bigint AS gross_credits,refunded_credits::bigint AS refunded_credits,settled_requests,pending_requests,unknown_requests FROM core_channels.usage_contributions WHERE request_id=$1 ORDER BY channel_id FOR UPDATE")
        .bind(id).fetch_all(&mut *c).await?;
    if old != new {
        let bucket = i16::from(Sha256::digest(id.as_bytes())[0] & 63);
        let mut deltas = BTreeMap::<(i64, i64, i16), Delta>::new();
        for (rows, sign) in [(&old, -1), (&new, 1)] {
            for row in rows {
                for day in [-1, row.authorized_at / 86400] {
                    deltas
                        .entry((row.channel_id, day, bucket))
                        .or_default()
                        .add(row, sign);
                }
            }
        }
        for ((channel, day, bucket), delta) in deltas {
            if delta.zero() {
                continue;
            }
            // Insert a valid zero row first; negative deltas must not be
            // proposed as INSERT values before ON CONFLICT checks constraints.
            sqlx::query("INSERT INTO core_channels.usage_rollups(channel_id,day,bucket,gross_credits,refunded_credits,settled_requests,pending_requests,unknown_requests) VALUES($1,$2,$3,0,0,0,0,0) ON CONFLICT(channel_id,day,bucket) DO NOTHING")
                .bind(channel).bind(day).bind(bucket).execute(&mut *c).await?;
            sqlx::query("UPDATE core_channels.usage_rollups SET gross_credits=gross_credits+$4::numeric,refunded_credits=refunded_credits+$5::numeric,settled_requests=settled_requests+$6,pending_requests=pending_requests+$7,unknown_requests=unknown_requests+$8 WHERE channel_id=$1 AND day=$2 AND bucket=$3")
                .bind(channel).bind(day).bind(bucket).bind(delta.gross.to_string()).bind(delta.refunded.to_string())
                .bind(delta.settled).bind(delta.pending).bind(delta.unknown).execute(&mut *c).await?;
        }
        sqlx::query("DELETE FROM core_channels.usage_contributions WHERE request_id=$1")
            .bind(id)
            .execute(&mut *c)
            .await?;
        insert_contributions(c, &new).await?;
    }
    sqlx::query("DELETE FROM core_channels.projection_dirty WHERE request_id=$1")
        .bind(id)
        .execute(c)
        .await?;
    Ok(())
}

pub(super) async fn read_usage(
    c: &mut PgConnection,
    channel: u64,
    start: i64,
    end: i64,
    scope: &'static str,
) -> Result<ChannelUsage, ChannelUsageError> {
    let lifetime = scope == "native_channel_totals";
    let ready: Option<bool> =
        sqlx::query_scalar("SELECT ready FROM core_channels.projection_state WHERE singleton")
            .fetch_optional(&mut *c)
            .await?;
    if ready != Some(true) {
        return Err(ChannelUsageError::Unavailable);
    }
    let dirty: bool = sqlx::query_scalar("SELECT EXISTS(SELECT FROM core_channels.projection_dirty WHERE $1 OR authorized_at BETWEEN $2 AND $3)")
        .bind(lifetime).bind(start).bind(end).fetch_one(&mut *c).await?;
    if dirty {
        return Err(ChannelUsageError::Unavailable);
    }
    let first_full = start / 86400 + i64::from(start % 86400 != 0);
    let after_full = (end + 1) / 86400;
    let row = sqlx::query("SELECT coalesce(sum(gross_credits),0)::text AS gross,coalesce(sum(refunded_credits),0)::text AS refunded,(coalesce(sum(gross_credits),0)-coalesce(sum(refunded_credits),0))::text AS net,coalesce(sum(settled_requests),0)::text AS settled,coalesce(sum(pending_requests),0)::text AS pending,coalesce(sum(unknown_requests),0)::text AS unknown FROM (SELECT gross_credits,refunded_credits,settled_requests,pending_requests,unknown_requests FROM core_channels.usage_rollups WHERE channel_id=ANY($1) AND (($2 AND day=-1) OR (NOT $2 AND day >= $5 AND day < $6)) UNION ALL SELECT gross_credits,refunded_credits,settled_requests,pending_requests,unknown_requests FROM core_channels.usage_contributions WHERE NOT $2 AND channel_id=ANY($1) AND authorized_at BETWEEN $3 AND $4 AND (authorized_at < $7 OR authorized_at >= $8)) amounts")
        .bind(vec![0_i64,channel as i64]).bind(lifetime).bind(start).bind(end)
        .bind(first_full).bind(after_full).bind(first_full*86400).bind(after_full*86400)
        .fetch_one(c).await?;
    Ok(ChannelUsage {
        scope,
        start_timestamp: start,
        end_timestamp: end,
        gross_settled_credits: row.try_get("gross")?,
        refunded_credits: row.try_get("refunded")?,
        net_used_credits: row.try_get("net")?,
        settled_requests: row.try_get("settled")?,
        pending_potential_requests: row.try_get("pending")?,
        unknown_provenance_requests: row.try_get("unknown")?,
        credits_per_usd: "500000",
        historical_coverage: false,
    })
}

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

//! Included under charging::engine: audio evidence uses the existing transaction,
//! ledger, authorization, budget and reconciliation machinery.
use super::*;
use crate::model_http::audio::{AudioMeteredUsage, AudioUsageFormat};
use serde::Deserialize;

const TERMINAL: &str = "audio-terminal:v1";
const USAGE_EVENT: &str = "usage:audio-terminal:v1";

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Terminal {
    version: u32,
    request_id: String,
    route_id: String,
    wire_sha256: String,
    price_sha256: String,
    raw_usage: String,
    usage: AudioMeteredUsage,
}

pub(super) fn restore_usage(base: Usage, evidence: Option<Value>) -> Result<Usage> {
    let has_audio = evidence
        .as_ref()
        .and_then(Value::as_array)
        .is_some_and(|events| events.iter().any(|e| e["usage"].get("audio").is_some()));
    if !has_audio {
        return cache_evidence::restore(base, evidence);
    }
    if base != Usage::default() {
        return Err(Error::Storage);
    }
    let events = evidence
        .as_ref()
        .and_then(Value::as_array)
        .ok_or(Error::Storage)?;
    let mut actual = None;
    for event in events {
        if !matches!(event["event_id"].as_str(), Some(USAGE_EVENT | "final")) {
            return Err(Error::Storage);
        }
        let usage: Usage =
            serde_json::from_value(event["usage"].clone()).map_err(|_| Error::Storage)?;
        if !usage.valid() || usage.audio.is_none() || actual.is_some_and(|old| old != usage) {
            return Err(Error::Storage);
        }
        actual = Some(usage);
    }
    actual.ok_or(Error::Storage)
}

fn verify_terminal(charge: &Charge, terminal: &Terminal, dispatch: &Value) -> Result<Usage> {
    let price = charge.price.audio.as_ref().ok_or(Error::Invalid)?;
    if terminal.version != 1
        || terminal.request_id != charge.id
        || terminal.price_sha256 != contract::typed_digest(&charge.price)?
        || dispatch["kind"] != "model_route_dispatch"
        || dispatch["number"] != 1
        || dispatch["route_id"].as_str() != Some(terminal.route_id.as_str())
        || dispatch["wire_sha256"].as_str() != Some(terminal.wire_sha256.as_str())
        || dispatch["actual_group"].as_str() != Some(charge.price.group.as_str())
        || price.usage_format != AudioUsageFormat::OpenAiChatV1
    {
        return Err(Error::Invalid);
    }
    let counts = price
        .usage_format
        .parse(terminal.raw_usage.as_bytes())
        .map_err(|_| Error::Invalid)?;
    let raw_hash: [u8; 32] = Sha256::digest(terminal.raw_usage.as_bytes()).into();
    if counts != terminal.usage.counts || raw_hash != terminal.usage.raw_usage_sha256 {
        return Err(Error::Invalid);
    }
    let usage = Usage {
        audio: Some(terminal.usage),
        ..Usage::default()
    };
    if !usage.valid() || charge.price.cost(usage)? > charge.reserved {
        return Err(Error::Invalid);
    }
    Ok(usage)
}

pub(super) fn validate_loaded(charge: &Charge, row: &PgRow) -> Result<()> {
    let terminal: Option<Value> = row.try_get("audio_terminal")?;
    if let Some(value) = terminal {
        let terminal: Terminal = serde_json::from_value(value).map_err(|_| Error::Storage)?;
        let dispatch: Value = row
            .try_get::<Option<Value>, _>("audio_dispatch")?
            .ok_or(Error::Storage)?;
        if verify_terminal(charge, &terminal, &dispatch).map_err(|_| Error::Storage)?
            != charge.usage
            || (charge.state == "settled"
                && charge
                    .price
                    .cost(charge.usage)
                    .map_err(|_| Error::Storage)?
                    != charge.settled)
        {
            return Err(Error::Storage);
        }
    } else if charge.usage.audio.is_some()
        || (charge.state == "settled" && charge.price.audio.is_some())
    {
        return Err(Error::Storage);
    }
    Ok(())
}

pub(super) async fn verify_usage(
    c: &mut PgConnection,
    charge: &Charge,
    usage: Usage,
) -> Result<()> {
    if charge.price.audio.is_none() && usage.audio.is_none() {
        return Ok(());
    }
    let value: Value = sqlx::query_scalar(
        "SELECT detail FROM core_charging.usage_events WHERE request_id=$1 AND event_id=$2",
    )
    .bind(&charge.id)
    .bind(TERMINAL)
    .fetch_optional(&mut *c)
    .await?
    .ok_or(Error::Invalid)?;
    let terminal: Terminal = serde_json::from_value(value).map_err(|_| Error::Invalid)?;
    let dispatch: Value = sqlx::query_scalar("SELECT detail FROM core_charging.usage_events WHERE request_id=$1 AND event_id='route-dispatch:1'")
        .bind(&charge.id).fetch_optional(&mut *c).await?.ok_or(Error::Invalid)?;
    let proof = contract::load(c, charge).await?.ok_or(Error::Invalid)?;
    if !proof.permits_dispatch(&terminal.route_id, &terminal.wire_sha256)
        || verify_terminal(charge, &terminal, &dispatch)? != usage
    {
        return Err(Error::Invalid);
    }
    Ok(())
}

impl Charging {
    /// Only the audio HTTP worker calls this after validating a terminal success.
    /// Raw bytes and normalized counters are durably bound BEFORE exposing output.
    pub(crate) async fn checkpoint_audio(
        &self,
        id: &str,
        worker: &[u8; 32],
        raw_usage: &str,
        response_sha256: [u8; 32],
    ) -> Result<Charge> {
        let mut tx = self.begin().await?;
        lock(&mut tx, id).await?;
        let stored = load(&mut tx, id).await?;
        worker_matches(&stored, worker)?;
        let price = stored.charge.price.audio.as_ref().ok_or(Error::Invalid)?;
        let counts = price
            .usage_format
            .parse(raw_usage.as_bytes())
            .map_err(|_| Error::Invalid)?;
        let metered = AudioMeteredUsage {
            counts,
            raw_usage_sha256: Sha256::digest(raw_usage.as_bytes()).into(),
            response_sha256,
        };
        let dispatch: Value = sqlx::query_scalar("SELECT detail FROM core_charging.usage_events WHERE request_id=$1 AND event_id='route-dispatch:1'")
            .bind(id).fetch_optional(&mut *tx).await?.ok_or(Error::Invalid)?;
        let terminal = Terminal {
            version: 1,
            request_id: id.into(),
            route_id: dispatch["route_id"].as_str().ok_or(Error::Invalid)?.into(),
            wire_sha256: dispatch["wire_sha256"]
                .as_str()
                .ok_or(Error::Invalid)?
                .into(),
            price_sha256: contract::typed_digest(&stored.charge.price)?,
            raw_usage: raw_usage.into(),
            usage: metered,
        };
        let usage = verify_terminal(&stored.charge, &terminal, &dispatch)?;
        let detail = json(&terminal)?;
        let previous: Option<Value> = sqlx::query_scalar(
            "SELECT detail FROM core_charging.usage_events WHERE request_id=$1 AND event_id=$2",
        )
        .bind(id)
        .bind(TERMINAL)
        .fetch_optional(&mut *tx)
        .await?;
        if let Some(previous) = previous {
            if previous != detail || stored.charge.usage != usage {
                return Err(Error::Conflict);
            }
            commit(tx).await?;
            return Ok(stored.charge);
        }
        live(&stored.charge, now(&mut tx).await?)?;
        if stored.charge.state != "streaming" {
            return Err(Error::WrongState);
        }
        let proof = contract::load(&mut tx, &stored.charge)
            .await?
            .ok_or(Error::Invalid)?;
        if !proof.permits_dispatch(&terminal.route_id, &terminal.wire_sha256) {
            return Err(Error::Invalid);
        }
        proof.check_usage(&stored.charge, usage)?;
        Self::check_usage(&stored.charge, usage)?;
        event(&mut tx, id, TERMINAL, detail).await?;
        event(
            &mut tx,
            id,
            USAGE_EVENT,
            json!({"kind":"usage","usage":usage}),
        )
        .await?;
        sqlx::query("UPDATE core_charging.requests SET revision=revision+1 WHERE id=$1")
            .bind(id)
            .execute(&mut *tx)
            .await?;
        let charge = load(&mut tx, id).await?.charge;
        commit(tx).await?;
        Ok(charge)
    }

    /// Called after recover_expired. No terminal evidence means no settlement.
    pub(crate) async fn reconcile_audio_receipts(&self, limit: i64) -> Result<usize> {
        if !(1..=1000).contains(&limit) {
            return Err(Error::Invalid);
        }
        let ids: Vec<String> = sqlx::query_scalar("SELECT r.id FROM core_charging.requests r WHERE r.state='reconcile' AND r.price ? 'audio' AND EXISTS (SELECT 1 FROM core_charging.usage_events e WHERE e.request_id=r.id AND e.event_id='audio-terminal:v1') ORDER BY r.id LIMIT $1")
            .bind(limit).fetch_all(&self.pool).await?;
        let mut settled = 0;
        for id in ids {
            let Some(charge) = self.resolve(&id).await? else {
                continue;
            };
            if charge.state != "reconcile" {
                continue;
            }
            // finish rechecks the raw receipt + original dispatch inside its transaction.
            match self
                .reconcile(
                    &id,
                    charge.revision,
                    charge.usage,
                    Outcome::Completed,
                    "audio-terminal:v1",
                )
                .await
            {
                Ok(_) => settled += 1,
                Err(Error::WrongState | Error::Conflict | Error::Retry | Error::CommitUnknown) => {
                    ()
                }
                Err(error) => return Err(error),
            }
        }
        Ok(settled)
    }
}

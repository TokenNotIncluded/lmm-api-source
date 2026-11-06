//! Included under charging::engine: audio evidence uses the existing transaction,
//! ledger, authorization, budget and reconciliation machinery.
use super::*;
use crate::billing::audio::{
    AudioMeteredUsage, AudioUsageFormat,
    journal::{
        JournalState, MAX_PACKET_BYTES, MAX_PACKETS, MAX_STREAM_BYTES, journal_step, journal_usage,
    },
};
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
        || !matches!(
            price.usage_format,
            AudioUsageFormat::OpenAiChatV1 | AudioUsageFormat::OpenAiChatV2
        )
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
    if let Some(raw) = verify_packet_tail(c, charge, &proof, terminal.usage.response_sha256).await?
    {
        if raw != terminal.raw_usage {
            return Err(Error::Invalid);
        }
    }
    if !proof.permits_dispatch(&terminal.route_id, &terminal.wire_sha256)
        || verify_terminal(charge, &terminal, &dispatch)? != usage
    {
        return Err(Error::Invalid);
    }
    Ok(())
}

async fn verify_packet_tail(
    c: &mut PgConnection,
    charge: &Charge,
    proof: &contract::ReservationContract,
    response_sha256: [u8; 32],
) -> Result<Option<String>> {
    let rows:Vec<Value>=sqlx::query_scalar("SELECT detail FROM core_charging.usage_events WHERE request_id=$1 AND event_id LIKE 'audio-packet:v1:%' ORDER BY event_id")
        .bind(&charge.id).fetch_all(c).await?;
    if !proof.audio_stream() {
        return if rows.is_empty() {
            Ok(None)
        } else {
            Err(Error::Invalid)
        };
    }
    if rows.is_empty() || rows.len() > MAX_PACKETS as usize {
        return Err(Error::Invalid);
    }
    let price_sha = contract::typed_digest(&charge.price)?;
    let mut previous = JournalState::default();
    let mut chain = "00".repeat(32);
    let mut total = 0u64;
    let mut raw = None;
    for (i, row) in rows.iter().enumerate() {
        let mut detail = row.clone();
        let saved = detail
            .as_object_mut()
            .ok_or(Error::Invalid)?
            .remove("chain_sha256")
            .ok_or(Error::Invalid)?;
        if !matches!(row["journal_version"].as_u64(), Some(3 | 4))
            || saved != contract::typed_digest(&detail)?
            || row["previous_chain"] != chain
            || row.get("packet").is_some()
        {
            return Err(Error::Invalid);
        }
        chain = saved.as_str().ok_or(Error::Invalid)?.into();
        let bytes = row["bytes"].as_u64().ok_or(Error::Invalid)?;
        total = total.checked_add(bytes).ok_or(Error::Invalid)?;
        if bytes == 0
            || bytes > MAX_PACKET_BYTES as u64
            || total > MAX_STREAM_BYTES as u64
            || row["total_bytes"].as_u64() != Some(total)
            || row["sequence"].as_u64() != Some((i + 1) as u64)
            || row["price_sha256"] != price_sha
        {
            return Err(Error::Invalid);
        }
        let state: JournalState =
            serde_json::from_value(row["state"].clone()).map_err(|_| Error::Invalid)?;
        let last = i + 1 == rows.len();
        if previous.done
            || previous
                .id
                .as_ref()
                .is_some_and(|id| state.id.as_ref() != Some(id))
            || (previous.finished && !state.finished)
            || row["terminal"] != last
            || row["http_eof"] != last
            || state.done != last
        {
            return Err(Error::Invalid);
        }
        if let Some(packet) = row["usage_packet"].as_str() {
            if row["journal_version"] != 3
                || raw.is_some()
                || packet.len() as u64 != bytes
                || row["packet_sha256"] != contract::digest(packet.as_bytes())
            {
                return Err(Error::Invalid);
            }
            let (expected, retained) = journal_step(
                &previous,
                packet.as_bytes(),
                proof.audio_stream_model(),
                charge
                    .price
                    .audio
                    .as_ref()
                    .ok_or(Error::Invalid)?
                    .usage_format,
            )
            .map_err(|_| Error::Invalid)?;
            if expected != state || retained.is_none() {
                return Err(Error::Invalid);
            }
            raw = Some(
                journal_usage(
                    packet,
                    proof.audio_stream_model(),
                    charge
                        .price
                        .audio
                        .as_ref()
                        .ok_or(Error::Invalid)?
                        .usage_format,
                )
                .map_err(|_| Error::Invalid)?,
            );
        } else if let Some(usage) = row["raw_usage"].as_str() {
            if row["journal_version"] != 4
                || raw.is_some()
                || !previous.finished
                || previous.has_usage
                || !state.has_usage
                || state.done
                || !state.finished
                || state.id != previous.id
                || row["raw_usage_sha256"] != contract::digest(usage.as_bytes())
            {
                return Err(Error::Invalid);
            }
            charge
                .price
                .audio
                .as_ref()
                .ok_or(Error::Invalid)?
                .usage_format
                .parse(usage.as_bytes())
                .map_err(|_| Error::Invalid)?;
            raw = Some(usage.to_owned());
        } else if state.has_usage != previous.has_usage {
            return Err(Error::Invalid);
        }
        if last
            && (!previous.finished
                || !previous.has_usage
                || row["stream_sha256"] != contract::hex_digest(&response_sha256))
        {
            return Err(Error::Invalid);
        }
        previous = state;
    }
    raw.map(Some).ok_or(Error::Invalid)
}

impl Charging {
    /// Immutable output journal, not a usage update or authority to spend.
    pub(crate) async fn checkpoint_audio_packet(
        &self,
        id: &str,
        worker: &[u8; 32],
        sequence: u64,
        packet: &[u8],
        stream_sha256: [u8; 32],
        terminal: bool,
    ) -> Result<()> {
        if terminal {
            return Err(Error::Invalid);
        }
        let mut tx = self.begin().await?;
        lock(&mut tx, id).await?;
        let stored = load(&mut tx, id).await?;
        worker_matches(&stored, worker)?;
        append_packet(&mut tx, &stored, id, sequence, packet, stream_sha256, false).await?;
        commit(tx).await
    }

    /// HTTP worker calls only after clean EOF. Final packet, EOF attestation,
    /// raw receipt and normalized usage are one serializable transaction.
    pub(crate) async fn checkpoint_audio_stream(
        &self,
        id: &str,
        worker: &[u8; 32],
        sequence: u64,
        packet: &[u8],
        response_sha256: [u8; 32],
    ) -> Result<Charge> {
        let mut tx = self.begin().await?;
        lock(&mut tx, id).await?;
        let stored = load(&mut tx, id).await?;
        worker_matches(&stored, worker)?;
        append_packet(
            &mut tx,
            &stored,
            id,
            sequence,
            packet,
            response_sha256,
            true,
        )
        .await?;
        let proof = contract::load(&mut tx, &stored.charge)
            .await?
            .ok_or(Error::Invalid)?;
        let raw = verify_packet_tail(&mut tx, &stored.charge, &proof, response_sha256)
            .await?
            .ok_or(Error::Invalid)?;
        let charge = save_receipt(&mut tx, &stored, id, &raw, response_sha256).await?;
        commit(tx).await?;
        Ok(charge)
    }

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
        let charge = save_receipt(&mut tx, &stored, id, raw_usage, response_sha256).await?;
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

async fn append_packet(
    c: &mut PgConnection,
    stored: &Stored,
    id: &str,
    sequence: u64,
    packet: &[u8],
    stream_sha256: [u8; 32],
    terminal: bool,
) -> Result<()> {
    if !(1..=MAX_PACKETS).contains(&sequence)
        || packet.is_empty()
        || packet.len() > MAX_PACKET_BYTES
    {
        return Err(Error::Invalid);
    }
    if stored.charge.price.audio.is_none() {
        return Err(Error::Invalid);
    }
    let event_id = format!("audio-packet:v1:{sequence:04}");
    let proof = contract::load(c, &stored.charge)
        .await?
        .ok_or(Error::Invalid)?;
    let previous:Option<Value>=sqlx::query_scalar("SELECT detail FROM core_charging.usage_events WHERE request_id=$1 AND event_id LIKE 'audio-packet:v1:%' AND event_id<$2 ORDER BY event_id DESC LIMIT 1")
        .bind(id).bind(&event_id).fetch_optional(&mut *c).await?;
    let (state, prior_chain, prior_bytes) = if let Some(p) = &previous {
        if p["sequence"].as_u64() != Some(sequence - 1) || p["terminal"] != false {
            return Err(Error::Conflict);
        }
        (
            serde_json::from_value::<JournalState>(p["state"].clone())
                .map_err(|_| Error::Invalid)?,
            p["chain_sha256"].as_str().ok_or(Error::Invalid)?.to_owned(),
            p["total_bytes"].as_u64().ok_or(Error::Invalid)?,
        )
    } else {
        if sequence != 1 {
            return Err(Error::Conflict);
        }
        (JournalState::default(), "00".repeat(32), 0)
    };
    let (next, usage_packet) = journal_step(
        &state,
        packet,
        proof.audio_stream_model(),
        stored
            .charge
            .price
            .audio
            .as_ref()
            .ok_or(Error::Invalid)?
            .usage_format,
    )
    .map_err(|_| Error::Invalid)?;
    if next.done != terminal {
        return Err(Error::Invalid);
    }
    let total = prior_bytes
        .checked_add(packet.len() as u64)
        .ok_or(Error::Invalid)?;
    if total > MAX_STREAM_BYTES as u64 {
        return Err(Error::Invalid);
    }
    let mut detail = json!({"kind":"audio_output_packet","journal_version":4,"sequence":sequence,
        "packet_sha256":contract::digest(packet),"bytes":packet.len(),"total_bytes":total,
        "stream_sha256":contract::hex_digest(&stream_sha256),"terminal":terminal,"http_eof":terminal,
        "state":next,"raw_usage_sha256":usage_packet.as_ref().map(|s|contract::digest(s.as_bytes())),"raw_usage":usage_packet,"previous_chain":prior_chain,
        "price_sha256":contract::typed_digest(&stored.charge.price)?});
    detail["chain_sha256"] = json!(contract::typed_digest(&detail)?);
    let old: Option<Value> = sqlx::query_scalar(
        "SELECT detail FROM core_charging.usage_events WHERE request_id=$1 AND event_id=$2",
    )
    .bind(id)
    .bind(&event_id)
    .fetch_optional(&mut *c)
    .await?;
    if let Some(old) = old {
        if old != detail {
            return Err(Error::Conflict);
        }
        return Ok(());
    }
    live(&stored.charge, now(c).await?)?;
    if stored.charge.state != "streaming" || stored.charge.usage.audio.is_some() {
        return Err(Error::WrongState);
    }
    let dispatch: Value = sqlx::query_scalar("SELECT detail FROM core_charging.usage_events WHERE request_id=$1 AND event_id='route-dispatch:1'")
            .bind(id).fetch_optional(&mut *c).await?.ok_or(Error::Invalid)?;
    let proof = contract::load(c, &stored.charge)
        .await?
        .ok_or(Error::Invalid)?;
    if !proof.audio_stream()
        || !proof.permits_dispatch(
            dispatch["route_id"].as_str().ok_or(Error::Invalid)?,
            dispatch["wire_sha256"].as_str().ok_or(Error::Invalid)?,
        )
    {
        return Err(Error::Invalid);
    }
    event(c, id, &event_id, detail).await?;
    Ok(())
}

async fn save_receipt(
    c: &mut PgConnection,
    stored: &Stored,
    id: &str,
    raw_usage: &str,
    response_sha256: [u8; 32],
) -> Result<Charge> {
    let proof = contract::load(c, &stored.charge)
        .await?
        .ok_or(Error::Invalid)?;
    if let Some(raw) = verify_packet_tail(c, &stored.charge, &proof, response_sha256).await? {
        if raw != raw_usage {
            return Err(Error::Invalid);
        }
    }
    let price = stored.charge.price.audio.as_ref().ok_or(Error::Invalid)?;
    price
        .usage_format
        .validate_retained_chat_usage(raw_usage.as_bytes())
        .map_err(|_| Error::Invalid)?;
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
            .bind(id).fetch_optional(&mut *c).await?.ok_or(Error::Invalid)?;
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
    .fetch_optional(&mut *c)
    .await?;
    if let Some(previous) = previous {
        if previous != detail || stored.charge.usage != usage {
            return Err(Error::Conflict);
        }
        return Ok(stored.charge.clone());
    }
    live(&stored.charge, now(c).await?)?;
    if stored.charge.state != "streaming" {
        return Err(Error::WrongState);
    }
    if !proof.permits_dispatch(&terminal.route_id, &terminal.wire_sha256) {
        return Err(Error::Invalid);
    }
    proof.check_usage(&stored.charge, usage)?;
    Charging::check_usage(&stored.charge, usage)?;
    event(c, id, TERMINAL, detail).await?;
    event(c, id, USAGE_EVENT, json!({"kind":"usage","usage":usage})).await?;
    sqlx::query("UPDATE core_charging.requests SET revision=revision+1 WHERE id=$1")
        .bind(id)
        .execute(&mut *c)
        .await?;
    let charge = load(c, id).await?.charge;
    Ok(charge)
}

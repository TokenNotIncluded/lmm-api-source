//! Audio SSE only: no text-usage normalization, estimates or retry after dispatch.
use super::*;
use crate::relay::provider_network::ProviderConnection;
use tokio::sync::mpsc;
use tokio_stream::wrappers::ReceiverStream;

type Result<T> = std::result::Result<T, RelayError>;
pub(crate) const MAX_STREAM_BYTES: usize = 16 * 1024 * 1024;
const MAX_PACKETS: u64 = 4096;

pub(super) fn launch(
    gateway: ModelGateway,
    id: String,
    worker: [u8; 32],
    price: charging::Price,
    model: String,
    secret: String,
    connection: ProviderConnection,
    upstream: reqwest::Response,
    permit: OwnedSemaphorePermit,
) -> Result<Response> {
    if !connection.matches_peer(upstream.remote_addr())
        || upstream.headers().contains_key(header::CONTENT_ENCODING)
        || !upstream
            .headers()
            .get(header::CONTENT_TYPE)
            .and_then(|h| h.to_str().ok())
            .is_some_and(|h| {
                h.split(';')
                    .next()
                    .is_some_and(|v| v.trim().eq_ignore_ascii_case("text/event-stream"))
            })
    {
        return Err(RelayError::SettlementPending);
    }
    let (tx, rx) = mpsc::channel::<std::result::Result<Bytes, std::io::Error>>(
        gateway.limits.queue_chunks.max(1),
    );
    tokio::spawn(async move {
        let _permit = permit;
        let _connection = connection;
        let result = tokio::time::timeout(
            gateway.limits.total_timeout,
            pump(
                &gateway, &id, &worker, &price, &model, &secret, upstream, &tx,
            ),
        )
        .await;
        if !matches!(result, Ok(Ok(()))) {
            let _ = billing::retry(|| gateway.bridge.charging.mark_unknown(&id, &worker)).await;
            let _ = tokio::time::timeout(
                gateway.limits.write_timeout,
                tx.send(Err(std::io::Error::other("audio settlement pending"))),
            )
            .await;
        }
    });
    let mut response = Response::new(Body::from_stream(ReceiverStream::new(rx)));
    response.headers_mut().insert(
        header::CONTENT_TYPE,
        HeaderValue::from_static("text/event-stream"),
    );
    response
        .headers_mut()
        .insert(header::CACHE_CONTROL, HeaderValue::from_static("no-store"));
    Ok(response)
}

#[derive(Default)]
pub(crate) struct TerminalState {
    id: Option<String>,
    finished: bool,
    usage: Option<String>,
    usage_packet: Option<Vec<u8>>,
    done: bool,
}
impl TerminalState {
    // Returns true only for a verified [DONE] following finish + complete usage.
    pub(crate) fn observe(
        &mut self,
        packet: &[u8],
        model: &str,
        format: AudioUsageFormat,
    ) -> Result<bool> {
        if self.done {
            return Err(RelayError::SettlementPending);
        }
        let text = std::str::from_utf8(packet).map_err(|_| RelayError::SettlementPending)?;
        let mut data = Vec::new();
        for line in text.lines() {
            if line.is_empty() || line.starts_with(':') {
                continue;
            }
            if let Some(value) = line.strip_prefix("data:") {
                data.push(value.strip_prefix(' ').unwrap_or(value));
            } else {
                return Err(RelayError::SettlementPending);
            }
        }
        if data.is_empty() {
            return Ok(false);
        }
        let raw = data.join("\n");
        if raw == "[DONE]" {
            if !self.finished || self.usage.is_none() {
                return Err(RelayError::SettlementPending);
            }
            self.done = true;
            return Ok(true);
        }
        if self.usage.is_some() {
            return Err(RelayError::SettlementPending);
        }
        validate_envelope(&raw)?;
        let value: Value = serde_json::from_str(&raw).map_err(|_| RelayError::SettlementPending)?;
        let id = value["id"]
            .as_str()
            .filter(|s| !s.is_empty())
            .ok_or(RelayError::SettlementPending)?;
        if value["object"] != "chat.completion.chunk"
            || value["model"] != model
            || id.len() > 128
            || !id
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || b"-_:./".contains(&b))
            || self
                .id
                .as_ref()
                .is_some_and(|old| old != &charging::contract::digest(id.as_bytes()))
        {
            return Err(RelayError::SettlementPending);
        }
        if value.get("error").is_some_and(|v| !v.is_null()) {
            return Err(RelayError::SettlementPending);
        }
        self.id = Some(charging::contract::digest(id.as_bytes()));
        let choices = value["choices"]
            .as_array()
            .ok_or(RelayError::SettlementPending)?;
        if value.get("usage").is_some_and(|u| !u.is_null()) {
            if !self.finished
                || !choices.is_empty()
                || ["delta", "body", "audio", "content"]
                    .iter()
                    .any(|k| value.get(*k).is_some())
            {
                return Err(RelayError::SettlementPending);
            }
            #[derive(Deserialize)]
            struct RawUsage {
                usage: Box<RawValue>,
            }
            let receipt: RawUsage =
                serde_json::from_str(&raw).map_err(|_| RelayError::SettlementPending)?;
            super::super::receipt::validate_chat_usage_retention(receipt.usage.get().as_bytes())
                .map_err(|_| RelayError::SettlementPending)?;
            format
                .parse(receipt.usage.get().as_bytes())
                .map_err(|_| RelayError::SettlementPending)?;
            self.usage = Some(receipt.usage.get().into());
            self.usage_packet = Some(packet.into());
            return Ok(false);
        }
        if self.finished || choices.len() != 1 || choices[0]["index"] != 0 {
            return Err(RelayError::SettlementPending);
        }
        let delta = choices[0]["delta"]
            .as_object()
            .ok_or(RelayError::SettlementPending)?;
        for (key, value) in delta {
            let valid = match key.as_str() {
                "role" => value == "assistant",
                "content" => value.is_null() || value.is_string(),
                "audio" => value.as_object().is_some_and(|a| {
                    a.iter().all(|(k, v)| match k.as_str() {
                        "id" | "data" | "transcript" => v.is_string(),
                        "expires_at" => v.as_u64().is_some(),
                        _ => false,
                    })
                }),
                _ => false,
            };
            if !valid {
                return Err(RelayError::SettlementPending);
            }
        }
        match choices[0]["finish_reason"].as_str() {
            Some("stop" | "length") => self.finished = true,
            None if choices[0]["finish_reason"].is_null() => (),
            _ => return Err(RelayError::SettlementPending),
        }
        Ok(false)
    }
}

pub(crate) fn frame_end(bytes: &[u8]) -> Option<usize> {
    (0..bytes.len()).find_map(|i| {
        if bytes[i..].starts_with(b"\n\n") {
            Some(i + 2)
        } else if bytes[i..].starts_with(b"\r\n\r\n") {
            Some(i + 4)
        } else {
            None
        }
    })
}
async fn send(
    tx: &mpsc::Sender<std::result::Result<Bytes, std::io::Error>>,
    bytes: Vec<u8>,
    limits: &Limits,
) -> Result<()> {
    tokio::time::timeout(limits.write_timeout, tx.send(Ok(Bytes::from(bytes))))
        .await
        .map_err(|_| RelayError::SettlementPending)?
        .map_err(|_| RelayError::SettlementPending)
}
async fn pump(
    gateway: &ModelGateway,
    id: &str,
    worker: &[u8; 32],
    price: &charging::Price,
    model: &str,
    secret: &str,
    mut upstream: reqwest::Response,
    tx: &mpsc::Sender<std::result::Result<Bytes, std::io::Error>>,
) -> Result<()> {
    let format = price
        .audio
        .as_ref()
        .ok_or(RelayError::SettlementPending)?
        .usage_format;
    let limits = &gateway.limits;
    let cap = limits
        .upstream_bytes
        .min(limits.output_bytes)
        .min(MAX_STREAM_BYTES);
    let mut buffer = Vec::new();
    let mut total = 0usize;
    let mut sequence = 0u64;
    let mut digest = Sha256::new();
    let mut secret_tail = Vec::new();
    let mut state = TerminalState::default();
    let mut done_packet = None;
    loop {
        let chunk = tokio::time::timeout(limits.read_timeout, upstream.chunk())
            .await
            .map_err(|_| RelayError::SettlementPending)?
            .map_err(|_| RelayError::SettlementPending)?;
        let Some(chunk) = chunk else {
            break;
        };
        total = total
            .checked_add(chunk.len())
            .ok_or(RelayError::SettlementPending)?;
        if total > cap {
            return Err(RelayError::SettlementPending);
        }
        secret_tail.extend_from_slice(&chunk);
        if !secret.is_empty()
            && secret_tail
                .windows(secret.len())
                .any(|w| w == secret.as_bytes())
        {
            return Err(RelayError::SettlementPending);
        }
        let keep = secret.len().saturating_sub(1).min(secret_tail.len());
        secret_tail.drain(..secret_tail.len() - keep);
        buffer.extend_from_slice(&chunk);
        while let Some(end) = frame_end(&buffer) {
            if end > limits.event_bytes {
                return Err(RelayError::SettlementPending);
            }
            let packet: Vec<u8> = buffer.drain(..end).collect();
            sequence += 1;
            if sequence > MAX_PACKETS {
                return Err(RelayError::SettlementPending);
            }
            let terminal = state.observe(&packet, model, format)?;
            digest.update(&packet);
            let stream_sha: [u8; 32] = digest.clone().finalize().into();
            if terminal {
                // DONE is not HTTP EOF. Never persist final authority until the
                // bounded transport confirms EOF with no later bytes or errors.
                done_packet = Some(packet);
            } else {
                billing::retry(|| {
                    gateway
                        .bridge
                        .charging
                        .checkpoint_audio_packet(id, worker, sequence, &packet, stream_sha, false)
                })
                .await
                .map_err(billing::error)?;
                if state.usage.is_none() {
                    send(tx, packet, limits).await?;
                }
            }
        }
        if buffer.len() > limits.event_bytes {
            return Err(RelayError::SettlementPending);
        }
    }
    if !buffer.is_empty() || !state.done {
        return Err(RelayError::SettlementPending);
    }
    let packet = done_packet.ok_or(RelayError::SettlementPending)?;
    let stream_sha: [u8; 32] = digest.finalize().into();
    // DONE+EOF evidence and the raw usage receipt commit in one ledger transaction.
    let checkpoint = billing::retry(|| {
        gateway
            .bridge
            .charging
            .checkpoint_audio_stream(id, worker, sequence, &packet, stream_sha)
    })
    .await
    .map_err(billing::error)?;
    let settled = billing::retry(|| {
        gateway
            .bridge
            .charging
            .settle(id, worker, checkpoint.usage, Outcome::Completed)
    })
    .await;
    let settled = match settled {
        Ok(c) => Some(c),
        Err(charging::Error::CommitUnknown) => gateway
            .bridge
            .charging
            .resolve(id)
            .await
            .map_err(billing::error)?,
        _ => return Err(RelayError::SettlementPending),
    };
    if !settled.as_ref().is_some_and(|c| {
        billing::confirmed_settlement(c, id, price, checkpoint.usage, Outcome::Completed)
    }) {
        return Err(RelayError::SettlementPending);
    }
    send(
        tx,
        state
            .usage_packet
            .take()
            .ok_or(RelayError::SettlementPending)?,
        limits,
    )
    .await?;
    send(tx, packet, limits).await
}

/// Complete-packet protocol fixture helper; production persists semantic checkpoints.
#[cfg(test)]
pub(crate) fn replay_packets<'a>(
    packets: impl IntoIterator<Item = &'a [u8]>,
    model: &str,
    format: AudioUsageFormat,
) -> Result<String> {
    let mut state = TerminalState::default();
    for packet in packets {
        if frame_end(packet) != Some(packet.len()) {
            return Err(RelayError::SettlementPending);
        }
        state.observe(packet, model, format)?;
    }
    if !state.done {
        return Err(RelayError::SettlementPending);
    }
    state.usage.ok_or(RelayError::SettlementPending)
}

#[cfg(test)]
#[path = "stream_tests.rs"]
mod tests;

// Metadata is type-checked in memory, then discarded. Unknown envelope
// metadata is not accounting authority; raw usage gets its own strict parser.
fn validate_envelope(raw: &str) -> Result<()> {
    super::super::receipt::validate_unique_json(raw.as_bytes())
        .map_err(|_| RelayError::SettlementPending)?;
    #[derive(Deserialize)]
    #[allow(dead_code)]
    struct Envelope {
        id: String,
        object: String,
        model: String,
        choices: Vec<Value>,
        usage: Option<Box<RawValue>>,
        created: Option<u64>,
        system_fingerprint: Option<String>,
        service_tier: Option<String>,
        obfuscation: Option<String>,
    }
    let _: Envelope = serde_json::from_str(raw).map_err(|_| RelayError::SettlementPending)?;
    Ok(())
}

/// Persist only bounded semantic state; IDs are hashed and no output delta is retained.
#[derive(Default, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct JournalState {
    pub id: Option<String>,
    pub finished: bool,
    pub has_usage: bool,
    pub done: bool,
}
pub(crate) fn journal_step(
    previous: &JournalState,
    packet: &[u8],
    model: &str,
    format: AudioUsageFormat,
) -> Result<(JournalState, Option<String>)> {
    if frame_end(packet) != Some(packet.len()) {
        return Err(RelayError::SettlementPending);
    }
    let mut state = TerminalState {
        id: previous.id.clone(),
        finished: previous.finished,
        usage: previous.has_usage.then(String::new),
        usage_packet: None,
        done: previous.done,
    };
    state.observe(packet, model, format)?;
    let retained = if previous.has_usage {
        None
    } else {
        state.usage.clone()
    };
    Ok((
        JournalState {
            id: state.id,
            finished: state.finished,
            has_usage: state.usage.is_some(),
            done: state.done,
        },
        retained,
    ))
}
pub(crate) fn journal_usage(packet: &str, model: &str, format: AudioUsageFormat) -> Result<String> {
    let value = packet
        .lines()
        .filter_map(|l| {
            l.strip_prefix("data:")
                .map(|s| s.strip_prefix(' ').unwrap_or(s))
        })
        .collect::<Vec<_>>()
        .join("\n");
    let v: Value = serde_json::from_str(&value).map_err(|_| RelayError::SettlementPending)?;
    validate_envelope(&value)?;
    if v["model"] != model || v["choices"] != serde_json::json!([]) {
        return Err(RelayError::SettlementPending);
    }
    #[derive(Deserialize)]
    struct Raw {
        usage: Box<RawValue>,
    }
    let raw: Raw = serde_json::from_str(&value).map_err(|_| RelayError::SettlementPending)?;
    format
        .parse(raw.usage.get().as_bytes())
        .map_err(|_| RelayError::SettlementPending)?;
    Ok(raw.usage.get().into())
}

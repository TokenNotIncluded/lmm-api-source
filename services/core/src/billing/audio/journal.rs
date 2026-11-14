//! Pure audio stream validation shared by live forwarding and durable replay.
use super::{AudioError, AudioUsageFormat, price::Result, receipt};
use serde::Deserialize;
use serde_json::{Value, value::RawValue};
use sha2::{Digest, Sha256};

pub(crate) const MAX_STREAM_BYTES: usize = 16 * 1024 * 1024;
pub(crate) const MAX_PACKETS: u64 = 4096;
pub(crate) const MAX_PACKET_BYTES: usize = 262_144;

#[derive(Default)]
pub(crate) struct TerminalState {
    id: Option<String>,
    finished: bool,
    usage: Option<String>,
    usage_packet: Option<Vec<u8>>,
    done: bool,
}
impl TerminalState {
    pub(crate) fn has_usage(&self) -> bool {
        self.usage.is_some()
    }
    pub(crate) fn is_done(&self) -> bool {
        self.done
    }
    pub(crate) fn into_usage_packet(self) -> Option<Vec<u8>> {
        self.usage_packet
    }
    // Returns true only for a verified [DONE] following finish + complete usage.
    pub(crate) fn observe(
        &mut self,
        packet: &[u8],
        model: &str,
        format: AudioUsageFormat,
    ) -> Result<bool> {
        if self.done {
            return Err(AudioError::InvalidUsage);
        }
        let text = std::str::from_utf8(packet).map_err(|_| AudioError::InvalidUsage)?;
        let mut data = Vec::new();
        for line in text.lines() {
            if line.is_empty() || line.starts_with(':') {
                continue;
            }
            if let Some(value) = line.strip_prefix("data:") {
                data.push(value.strip_prefix(' ').unwrap_or(value));
            } else {
                return Err(AudioError::InvalidUsage);
            }
        }
        if data.is_empty() {
            return Ok(false);
        }
        let raw = data.join("\n");
        if raw == "[DONE]" {
            if !self.finished || self.usage.is_none() {
                return Err(AudioError::InvalidUsage);
            }
            self.done = true;
            return Ok(true);
        }
        if self.usage.is_some() {
            return Err(AudioError::InvalidUsage);
        }
        validate_envelope(&raw)?;
        let value: Value = serde_json::from_str(&raw).map_err(|_| AudioError::InvalidUsage)?;
        let id = value["id"]
            .as_str()
            .filter(|s| !s.is_empty())
            .ok_or(AudioError::InvalidUsage)?;
        if value["object"] != "chat.completion.chunk"
            || value["model"] != model
            || id.len() > 128
            || !id
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || b"-_:./".contains(&b))
            || self
                .id
                .as_ref()
                .is_some_and(|old| old != &crate::hex_codec::encode(Sha256::digest(id.as_bytes())))
        {
            return Err(AudioError::InvalidUsage);
        }
        if value.get("error").is_some_and(|v| !v.is_null()) {
            return Err(AudioError::InvalidUsage);
        }
        self.id = Some(crate::hex_codec::encode(Sha256::digest(id.as_bytes())));
        let choices = value["choices"]
            .as_array()
            .ok_or(AudioError::InvalidUsage)?;
        if value.get("usage").is_some_and(|u| !u.is_null()) {
            if !self.finished
                || !choices.is_empty()
                || ["delta", "body", "audio", "content"]
                    .iter()
                    .any(|k| value.get(*k).is_some())
            {
                return Err(AudioError::InvalidUsage);
            }
            #[derive(Deserialize)]
            struct RawUsage {
                usage: Box<RawValue>,
            }
            let receipt: RawUsage =
                serde_json::from_str(&raw).map_err(|_| AudioError::InvalidUsage)?;
            receipt::validate_chat_usage_retention(receipt.usage.get().as_bytes())
                .map_err(|_| AudioError::InvalidUsage)?;
            format
                .parse(receipt.usage.get().as_bytes())
                .map_err(|_| AudioError::InvalidUsage)?;
            self.usage = Some(receipt.usage.get().into());
            self.usage_packet = Some(packet.into());
            return Ok(false);
        }
        if self.finished || choices.len() != 1 || choices[0]["index"] != 0 {
            return Err(AudioError::InvalidUsage);
        }
        let delta = choices[0]["delta"]
            .as_object()
            .ok_or(AudioError::InvalidUsage)?;
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
                return Err(AudioError::InvalidUsage);
            }
        }
        match choices[0]["finish_reason"].as_str() {
            Some("stop" | "length") => self.finished = true,
            None if choices[0]["finish_reason"].is_null() => (),
            _ => return Err(AudioError::InvalidUsage),
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
            return Err(AudioError::InvalidUsage);
        }
        state.observe(packet, model, format)?;
    }
    if !state.done {
        return Err(AudioError::InvalidUsage);
    }
    state.usage.ok_or(AudioError::InvalidUsage)
}

#[cfg(test)]
#[path = "journal_tests.rs"]
mod tests;

// Metadata is type-checked in memory, then discarded. Unknown envelope
// metadata is not accounting authority; raw usage gets its own strict parser.
fn validate_envelope(raw: &str) -> Result<()> {
    receipt::validate_unique_json(raw.as_bytes()).map_err(|_| AudioError::InvalidUsage)?;
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
    let _: Envelope = serde_json::from_str(raw).map_err(|_| AudioError::InvalidUsage)?;
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
        return Err(AudioError::InvalidUsage);
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
    let v: Value = serde_json::from_str(&value).map_err(|_| AudioError::InvalidUsage)?;
    validate_envelope(&value)?;
    if v["model"] != model || v["choices"] != serde_json::json!([]) {
        return Err(AudioError::InvalidUsage);
    }
    #[derive(Deserialize)]
    struct Raw {
        usage: Box<RawValue>,
    }
    let raw: Raw = serde_json::from_str(&value).map_err(|_| AudioError::InvalidUsage)?;
    format
        .parse(raw.usage.get().as_bytes())
        .map_err(|_| AudioError::InvalidUsage)?;
    Ok(raw.usage.get().into())
}

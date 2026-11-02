use super::price::Result;
use super::{AudioError, AudioQuote, AudioUsage};
use crate::billing::Credits;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use sha2::{Digest, Sha256};

const MAX_USAGE_BYTES: usize = 65_536;

/// Frozen provider projection. Chat/realtime V1 require explicit modality counts;
/// a provider omitting text counts needs a separately reviewed adapter contract.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AudioUsageFormat {
    FourCountersV1,
    OpenAiChatV1,
    /// Reviewed text/audio-only request projection; explicit audio, optional text.
    OpenAiChatV2,
    OpenAiRealtimeV1,
}
impl AudioUsageFormat {
    pub(crate) fn validate_retained_chat_usage(self, raw: &[u8]) -> Result<()> {
        if !matches!(self, Self::OpenAiChatV1 | Self::OpenAiChatV2) {
            return Err(AudioError::InvalidUsage);
        }
        validate_chat_usage_retention(raw)
    }

    pub(crate) fn parse(self, raw: &[u8]) -> Result<AudioUsage> {
        if raw.is_empty() {
            return Err(AudioError::UnknownUsage);
        }
        if raw.len() > MAX_USAGE_BYTES {
            return Err(AudioError::InvalidUsage);
        }
        validate_unique_json(raw)?;
        if self == Self::OpenAiChatV2 {
            return parse_chat_v2(raw);
        }
        if self == Self::FourCountersV1 {
            // Required counters; zero must be explicitly present in the receipt.
            let value: Value = serde_json::from_slice(raw).map_err(|_| AudioError::InvalidUsage)?;
            for field in ["text_input", "text_output", "audio_input", "audio_output"] {
                if value.get(field).is_none_or(Value::is_null) {
                    return Err(AudioError::UnknownUsage);
                }
            }
            return serde_json::from_slice(raw).map_err(|_| AudioError::InvalidUsage);
        }
        let value: Value = serde_json::from_slice(raw).map_err(|_| AudioError::InvalidUsage)?;
        let (input, output, total_in, total_out) = match self {
            Self::OpenAiChatV1 => (
                "prompt_tokens_details",
                "completion_tokens_details",
                "prompt_tokens",
                "completion_tokens",
            ),
            Self::OpenAiRealtimeV1 => (
                "input_token_details",
                "output_token_details",
                "input_tokens",
                "output_tokens",
            ),
            Self::FourCountersV1 | Self::OpenAiChatV2 => unreachable!(),
        };
        let count = |path: &str| -> Result<u64> {
            let n = value
                .pointer(path)
                .filter(|v| !v.is_null())
                .ok_or(AudioError::UnknownUsage)?;
            n.as_u64().ok_or(AudioError::InvalidUsage)
        };
        let usage = AudioUsage {
            text_input: count(&format!("/{input}/text_tokens"))?,
            text_output: count(&format!("/{output}/text_tokens"))?,
            audio_input: count(&format!("/{input}/audio_tokens"))?,
            audio_output: count(&format!("/{output}/audio_tokens"))?,
        };
        if usage.input()? != count(&format!("/{total_in}"))?
            || usage.output()? != count(&format!("/{total_out}"))?
        {
            return Err(AudioError::InvalidUsage);
        }
        // Extra modalities cannot be silently priced as text/audio. Cached
        // subsets do not alter this v1 tariff, which prices all modality tokens.
        for details in [input, output] {
            if let Some(n) = value.pointer(&format!("/{details}/image_tokens")) {
                if n.as_u64() != Some(0) {
                    return Err(AudioError::InvalidUsage);
                }
            }
        }
        if let Some(total) = value.get("total_tokens") {
            if total.as_u64()
                != Some(
                    usage
                        .input()?
                        .checked_add(usage.output()?)
                        .ok_or(AudioError::Overflow)?,
                )
            {
                return Err(AudioError::InvalidUsage);
            }
        }
        Ok(usage)
    }
}

/// Taken from the durable, authenticated dispatch record, never provider JSON.
/// One logical request/quote can have attempts, but the ledger must settle once.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AudioDispatch {
    pub request_id: String,
    pub attempt_id: String,
    pub route_id: String,
    pub wire_sha256: [u8; 32],
    pub quote_sha256: [u8; 32],
}

/// Store atomically before settlement. Raw bytes are the upstream usage object,
/// not a reserialized object, response body, audio payload or credentials.
/// Deserialization alone is not validation: recovery must call replay().
#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AudioSettlement {
    version: u32,
    dispatch: AudioDispatch,
    raw_usage: Vec<u8>,
    raw_usage_sha256: [u8; 32],
    usage: AudioUsage,
    credits: Credits,
}
impl AudioSettlement {
    pub fn credits(&self) -> Credits {
        self.credits
    }
    pub fn usage(&self) -> AudioUsage {
        self.usage
    }
    pub fn dispatch(&self) -> &AudioDispatch {
        &self.dispatch
    }
    pub fn raw_usage_sha256(&self) -> [u8; 32] {
        self.raw_usage_sha256
    }
    pub fn raw_usage(&self) -> &[u8] {
        &self.raw_usage
    }
}

impl AudioQuote {
    /// Call only for a terminal successful response from the authenticated
    /// supplier attempt. false/absent/malformed usage must keep its hold.
    /// This computes a receipt; it does not grant spending authority.
    pub fn settle(
        &self,
        dispatch: &AudioDispatch,
        raw_usage: &[u8],
        final_usage: bool,
    ) -> Result<AudioSettlement> {
        if dispatch.quote_sha256 != self.digest()
            || dispatch.request_id != self.spec.binding.request_id
            || dispatch.route_id != self.spec.binding.route_id
            || dispatch.attempt_id.is_empty()
            || dispatch.attempt_id.len() > 256
        {
            return Err(AudioError::BindingMismatch);
        }
        if !final_usage {
            return Err(AudioError::UnknownUsage);
        }
        let usage = self.spec.usage_format.parse(raw_usage)?;
        let credits = self.cost(usage)?;
        Ok(AudioSettlement {
            version: 1,
            dispatch: dispatch.clone(),
            raw_usage: raw_usage.to_vec(),
            raw_usage_sha256: Sha256::digest(raw_usage).into(),
            usage,
            credits,
        })
    }
    /// Bind persisted evidence to the actual durable attempt and recompute using
    /// the original quote. Never select a new tariff during retry or recovery.
    pub fn replay(&self, dispatch: &AudioDispatch, receipt: &AudioSettlement) -> Result<Credits> {
        let expected = self.settle(dispatch, &receipt.raw_usage, true)?;
        if &expected != receipt {
            return Err(AudioError::BindingMismatch);
        }
        Ok(expected.credits)
    }
}

/// Optional means absent, not null or an invalid numeric value.
#[derive(Default)]
struct OptionalCount(Option<u64>);
impl<'de> Deserialize<'de> for OptionalCount {
    fn deserialize<D: serde::Deserializer<'de>>(d: D) -> std::result::Result<Self, D::Error> {
        u64::deserialize(d).map(|n| Self(Some(n)))
    }
}
fn parse_chat_v2(raw: &[u8]) -> Result<AudioUsage> {
    #[derive(Deserialize)]
    #[serde(deny_unknown_fields)]
    struct Details {
        audio_tokens: u64,
        #[serde(default)]
        text_tokens: OptionalCount,
        #[serde(default)]
        image_tokens: OptionalCount,
        #[serde(default)]
        cached_tokens: OptionalCount,
        #[serde(default)]
        cache_write_tokens: OptionalCount,
        #[serde(default)]
        reasoning_tokens: OptionalCount,
        #[serde(default)]
        accepted_prediction_tokens: OptionalCount,
        #[serde(default)]
        rejected_prediction_tokens: OptionalCount,
    }
    impl Details {
        fn text(&self, total: u64) -> Result<u64> {
            if self.image_tokens.0.is_some_and(|n| n != 0) {
                return Err(AudioError::InvalidUsage);
            }
            let text = total
                .checked_sub(self.audio_tokens)
                .ok_or(AudioError::InvalidUsage)?;
            if self.text_tokens.0.is_some_and(|n| n != text) {
                return Err(AudioError::InvalidUsage);
            }
            for n in [
                &self.cached_tokens,
                &self.cache_write_tokens,
                &self.reasoning_tokens,
                &self.accepted_prediction_tokens,
                &self.rejected_prediction_tokens,
            ] {
                if n.0.is_some_and(|n| n > total) {
                    return Err(AudioError::InvalidUsage);
                }
            }
            Ok(text)
        }
    }
    #[derive(Deserialize)]
    #[serde(deny_unknown_fields)]
    struct Receipt {
        prompt_tokens: u64,
        completion_tokens: u64,
        total_tokens: u64,
        prompt_tokens_details: Details,
        completion_tokens_details: Details,
    }
    let r: Receipt = serde_json::from_slice(raw).map_err(|_| AudioError::InvalidUsage)?;
    if r.prompt_tokens.checked_add(r.completion_tokens) != Some(r.total_tokens) {
        return Err(AudioError::InvalidUsage);
    }
    Ok(AudioUsage {
        text_input: r.prompt_tokens_details.text(r.prompt_tokens)?,
        text_output: r.completion_tokens_details.text(r.completion_tokens)?,
        audio_input: r.prompt_tokens_details.audio_tokens,
        audio_output: r.completion_tokens_details.audio_tokens,
    })
}

/// Reject ambiguous authoritative JSON before any Value map can overwrite keys.
pub(crate) fn validate_unique_json(raw: &[u8]) -> Result<()> {
    struct Checked;
    impl<'de> Deserialize<'de> for Checked {
        fn deserialize<D: serde::Deserializer<'de>>(d: D) -> std::result::Result<Self, D::Error> {
            struct Visitor;
            impl<'de> serde::de::Visitor<'de> for Visitor {
                type Value = Checked;
                fn expecting(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
                    f.write_str("unambiguous JSON")
                }
                fn visit_map<M: serde::de::MapAccess<'de>>(
                    self,
                    mut m: M,
                ) -> std::result::Result<Checked, M::Error> {
                    let mut keys = std::collections::HashSet::new();
                    while let Some(key) = m.next_key::<String>()? {
                        if !keys.insert(key) {
                            return Err(serde::de::Error::custom("duplicate JSON key"));
                        }
                        m.next_value::<Checked>()?;
                    }
                    Ok(Checked)
                }
                fn visit_seq<S: serde::de::SeqAccess<'de>>(
                    self,
                    mut s: S,
                ) -> std::result::Result<Checked, S::Error> {
                    while s.next_element::<Checked>()?.is_some() {}
                    Ok(Checked)
                }
                fn visit_bool<E: serde::de::Error>(
                    self,
                    _: bool,
                ) -> std::result::Result<Checked, E> {
                    Ok(Checked)
                }
                fn visit_u64<E: serde::de::Error>(self, _: u64) -> std::result::Result<Checked, E> {
                    Ok(Checked)
                }
                fn visit_i64<E: serde::de::Error>(self, _: i64) -> std::result::Result<Checked, E> {
                    Ok(Checked)
                }
                fn visit_f64<E: serde::de::Error>(self, _: f64) -> std::result::Result<Checked, E> {
                    Ok(Checked)
                }
                fn visit_str<E: serde::de::Error>(
                    self,
                    _: &str,
                ) -> std::result::Result<Checked, E> {
                    Ok(Checked)
                }
                fn visit_unit<E: serde::de::Error>(self) -> std::result::Result<Checked, E> {
                    Ok(Checked)
                }
            }
            d.deserialize_any(Visitor)
        }
    }
    serde_json::from_slice::<Checked>(raw)
        .map(|_| ())
        .map_err(|_| AudioError::InvalidUsage)
}

/// Fresh Chat evidence admits counters only; historical V1 receipt replay keeps
/// its original projection. Never persist provider body hidden inside usage.
pub(crate) fn validate_chat_usage_retention(raw: &[u8]) -> Result<()> {
    validate_unique_json(raw)?;
    let value: Value = serde_json::from_slice(raw).map_err(|_| AudioError::InvalidUsage)?;
    let object = value.as_object().ok_or(AudioError::InvalidUsage)?;
    for (key, value) in object {
        match key.as_str() {
            "prompt_tokens" | "completion_tokens" | "total_tokens" if value.as_u64().is_some() => {}
            "prompt_tokens_details" | "completion_tokens_details" => {
                let details = value.as_object().ok_or(AudioError::InvalidUsage)?;
                if details.iter().any(|(key, value)| {
                    !matches!(
                        key.as_str(),
                        "text_tokens"
                            | "audio_tokens"
                            | "image_tokens"
                            | "cached_tokens"
                            | "cache_write_tokens"
                            | "reasoning_tokens"
                            | "accepted_prediction_tokens"
                            | "rejected_prediction_tokens"
                    ) || value.as_u64().is_none()
                }) {
                    return Err(AudioError::InvalidUsage);
                }
            }
            _ => return Err(AudioError::InvalidUsage),
        }
    }
    Ok(())
}

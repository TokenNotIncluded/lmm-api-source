//! Provider decoders share a bounded block model. Usage counters are cumulative.
use super::sse::Frame;
use super::{
    BlockKind, CacheCreationDetails, Event, FinishReason, Limits, OpenAiCacheWriteDetails,
    RelayError, UpstreamProtocol, Usage,
};
use serde_json::Value;
use std::collections::BTreeMap;

use crate::routing::CacheCreationMode;

#[path = "decode/zconv.rs"]
mod zconv_bridge;

type Key = (u64, u64);
struct Slot {
    index: usize,
    kind: BlockKind,
    ended: bool,
    has_text: bool,
    default_object: bool,
}
pub(crate) struct Decoder {
    protocol: UpstreamProtocol,
    cache_creation_mode: Option<CacheCreationMode>,
    legacy_openrouter_usage: bool,
    started: bool,
    source_id: Option<String>,
    done: bool,
    finish: Option<FinishReason>,
    blocks: BTreeMap<Key, Slot>,
    max_blocks: usize,
    image_count: usize,
    event_bytes: usize,
    gemini_calls: u64,
    gemini_grounded_requests: Option<u8>,
    continuation_token: Option<String>,
}
fn text(v: &Value) -> Result<&str, RelayError> {
    v.as_str()
        .ok_or(RelayError::Protocol("expected upstream string"))
}
fn counter(v: &Value, key: &str) -> Result<Option<u64>, RelayError> {
    // A missing details object is unknown. A present scalar or array is an
    // invalid receipt, not evidence that its billable counters are absent.
    if !v.is_null() && !v.is_object() {
        return Err(RelayError::Protocol("invalid usage details object"));
    }
    v.get(key)
        .filter(|v| !v.is_null())
        .map(|v| {
            v.as_u64()
                .ok_or(RelayError::Protocol("invalid usage counter"))
        })
        .transpose()
}

fn chat_cached_input(v: &Value, input: Option<u64>) -> Result<Option<u64>, RelayError> {
    let canonical = counter(&v["prompt_tokens_details"], "cached_tokens")?;
    // DeepSeek reports the same cache-read subset at the top level. Preserve
    // actual counters; neither a model name nor a missing field proves zero.
    let hit = counter(v, "prompt_cache_hit_tokens")?;
    if matches!((canonical, hit), (Some(a), Some(b)) if a != b) {
        return Err(RelayError::Protocol("inconsistent cache read usage"));
    }
    let cached = canonical.or(hit);
    if let Some(missed) = counter(v, "prompt_cache_miss_tokens")? {
        if input.is_some_and(|total| missed > total) {
            return Err(RelayError::Protocol("cache miss usage exceeds input total"));
        }
        if let Some(cached) = cached {
            let total = cached
                .checked_add(missed)
                .ok_or(RelayError::Protocol("usage overflow"))?;
            if input.is_some_and(|input| total != input) {
                return Err(RelayError::Protocol("inconsistent cache partition"));
            }
        }
    }
    Ok(cached)
}

fn add(a: Option<u64>, b: Option<u64>, c: Option<u64>) -> Result<Option<u64>, RelayError> {
    a.map(|a| {
        a.checked_add(b.unwrap_or(0))
            .and_then(|n| n.checked_add(c.unwrap_or(0)))
            .ok_or(RelayError::Protocol("usage overflow"))
    })
    .transpose()
}
fn cache_creation_details(v: &Value) -> Result<Option<CacheCreationDetails>, RelayError> {
    if v.is_null() {
        return Ok(None);
    }
    if !v.is_object() {
        return Err(RelayError::Protocol("invalid cache creation details"));
    }
    serde_json::from_value(v.clone())
        .map(Some)
        .map_err(|_| RelayError::Protocol("invalid cache creation details"))
}

fn openai_cache_write_details(v: &Value) -> Result<Option<OpenAiCacheWriteDetails>, RelayError> {
    if v.is_null() {
        return Ok(None);
    }
    if !v.is_object() {
        return Err(RelayError::Protocol("invalid OpenAI cache write details"));
    }
    serde_json::from_value(v.clone())
        .map(Some)
        .map_err(|_| RelayError::Protocol("invalid OpenAI cache write details"))
}

fn cache_creation_usage(u: &mut Usage, v: &Value, details_key: &str) -> Result<(), RelayError> {
    u.cache_creation_input_tokens = counter(v, "cache_creation_input_tokens")?;
    u.cache_creation_details = cache_creation_details(&v[details_key])?;
    if u.cache_creation_mode.is_none()
        && u.cache_creation_input_tokens.is_none()
        && let Some(details) = u.cache_creation_details
    {
        u.cache_creation_input_tokens = details.total()?;
    }
    Ok(())
}

#[cfg_attr(not(test), allow(dead_code))] // Re-exported for tests; production passes a cache-creation mode.
pub(crate) fn usage(protocol: UpstreamProtocol, v: &Value) -> Result<Usage, RelayError> {
    usage_with_cache_creation_mode(protocol, v, None)
}

pub(crate) fn usage_with_cache_creation_mode(
    protocol: UpstreamProtocol,
    v: &Value,
    mode: Option<CacheCreationMode>,
) -> Result<Usage, RelayError> {
    if v.is_null() {
        return Ok(Usage::default());
    }
    if !v.is_object() {
        return Err(RelayError::Protocol("invalid usage object"));
    }
    let mut u = Usage {
        cache_creation_mode: mode,
        ..Usage::default()
    };
    if matches!(
        protocol,
        UpstreamProtocol::Chat | UpstreamProtocol::Responses
    ) && let Some(value) = v.get("cost").filter(|v| !v.is_null())
    {
        let number = value
            .as_number()
            .ok_or(RelayError::Protocol("invalid upstream cost"))?;
        u.upstream_cost_atto_usd = Some(
            crate::upstream_cost::parse_usd(&number.to_string())
                .ok_or(RelayError::Protocol("invalid upstream cost"))?,
        );
    }
    match protocol {
        UpstreamProtocol::Chat => {
            u.input_tokens = counter(v, "prompt_tokens")?;
            u.output_tokens = counter(v, "completion_tokens")?;
            u.cached_input_tokens = chat_cached_input(v, u.input_tokens)?;
            u.reasoning_output_tokens =
                counter(&v["completion_tokens_details"], "reasoning_tokens")?;
        }
        UpstreamProtocol::Responses => {
            u.input_tokens = counter(v, "input_tokens")?;
            u.output_tokens = counter(v, "output_tokens")?;
            u.cached_input_tokens = counter(&v["input_tokens_details"], "cached_tokens")?;
            u.reasoning_output_tokens = counter(&v["output_tokens_details"], "reasoning_tokens")?;
        }
        UpstreamProtocol::Anthropic => {
            u.cached_input_tokens = counter(v, "cache_read_input_tokens")?;
            cache_creation_usage(&mut u, v, "cache_creation")?;
            let uncached = counter(v, "input_tokens")?;
            if u.cache_creation_mode.is_some() {
                u.uncached_input_tokens = uncached;
                if let Some(created) = u.effective_cache_creation_input_tokens()? {
                    u.input_tokens = add(uncached, u.cached_input_tokens, Some(created))?;
                } else {
                    // A raw aggregate is required even if both TTL counters
                    // exist: they cannot prove no generic remainder exists.
                    let known = u
                        .cache_creation_details
                        .map(|details| details.known_total())
                        .transpose()?
                        .unwrap_or(0);
                    let _ = add(uncached, u.cached_input_tokens, Some(known))?;
                }
            } else {
                // A partial duration breakdown establishes a lower bound, not the
                // complete cache write total needed to normalize disjoint input.
                if u.cache_creation_details.is_none() || u.cache_creation_input_tokens.is_some() {
                    u.input_tokens = add(
                        uncached,
                        u.cached_input_tokens,
                        u.cache_creation_input_tokens,
                    )?;
                } else if let Some(details) = u.cache_creation_details {
                    // Known disjoint counters must still fit even when they do not
                    // establish a complete inclusive input total.
                    let _ = add(
                        uncached,
                        u.cached_input_tokens,
                        Some(details.known_total()?),
                    )?;
                }
            }
            u.output_tokens = counter(v, "output_tokens")?;
        }
        UpstreamProtocol::Gemini => {
            u.input_tokens = counter(v, "promptTokenCount")?;
            u.cached_input_tokens = counter(v, "cachedContentTokenCount")?;
            u.reasoning_output_tokens = counter(v, "thoughtsTokenCount")?;
            u.tool_use_prompt_tokens = counter(v, "toolUsePromptTokenCount")?;
            u.output_tokens = add(
                counter(v, "candidatesTokenCount")?,
                u.reasoning_output_tokens,
                None,
            )?;
            if let Some(total) = counter(v, "totalTokenCount")? {
                let known = u
                    .input_tokens
                    .unwrap_or(0)
                    .checked_add(u.output_tokens.unwrap_or(0))
                    .and_then(|tokens| tokens.checked_add(u.tool_use_prompt_tokens.unwrap_or(0)))
                    .ok_or(RelayError::Protocol("usage overflow"))?;
                if known > total
                    || (u.input_tokens.is_some()
                        && u.output_tokens.is_some()
                        && u.tool_use_prompt_tokens.is_some()
                        && known != total)
                {
                    return Err(RelayError::Protocol("inconsistent Gemini usage total"));
                }
            }
        }
    }
    if matches!(
        protocol,
        UpstreamProtocol::Chat | UpstreamProtocol::Responses
    ) {
        // Preserve the extension emitted by Usage::wire. Discarding it here
        // would bypass the charging adapter's unsupported-cache-write guard.
        cache_creation_usage(&mut u, v, "cache_creation_details")?;
        let input_details = match protocol {
            UpstreamProtocol::Chat => &v["prompt_tokens_details"],
            UpstreamProtocol::Responses => &v["input_tokens_details"],
            _ => unreachable!(),
        };
        // OpenAI-compatible providers report these aliases for the same
        // cache writes. Their maximum avoids counting those tokens twice.
        let alias = counter(input_details, "cached_creation_tokens")?
            .max(counter(input_details, "cache_write_tokens")?)
            .max(counter(input_details, "cache_creation_input_tokens")?);
        if let Some(alias) = alias {
            if u.cache_creation_input_tokens
                .is_some_and(|canonical| canonical != alias)
            {
                return Err(RelayError::Protocol("inconsistent cache creation usage"));
            }
            u.cache_creation_input_tokens = Some(alias);
        }
        // Lifetimes are provider observations, not a consequence of requested
        // cache retention. Missing aggregate or lifetime fields remain unknown.
        u.openai_cache_write_details =
            openai_cache_write_details(&input_details["cache_write_tokens_details"])?;
        if let Some(total) = counter(v, "total_tokens")? {
            let known = u
                .input_tokens
                .unwrap_or(0)
                .checked_add(u.output_tokens.unwrap_or(0))
                .ok_or(RelayError::Protocol("usage overflow"))?;
            if known > total
                || (u.input_tokens.is_some() && u.output_tokens.is_some() && known != total)
            {
                return Err(RelayError::Protocol("inconsistent usage total"));
            }
            // A reported total cannot fill in a missing input/output counter.
        }
    }
    u.validate()?;
    Ok(u)
}
fn finish_reason(
    protocol: UpstreamProtocol,
    v: &str,
    tools: bool,
) -> Result<FinishReason, RelayError> {
    match (protocol, v) {
        (UpstreamProtocol::Chat, "stop")
        | (UpstreamProtocol::Anthropic, "end_turn" | "stop_sequence") => Ok(FinishReason::Stop),
        (UpstreamProtocol::Chat, "tool_calls" | "function_call")
        | (UpstreamProtocol::Anthropic, "tool_use") => Ok(FinishReason::ToolCalls),
        (UpstreamProtocol::Chat, "length")
        | (UpstreamProtocol::Anthropic, "max_tokens")
        | (UpstreamProtocol::Gemini, "MAX_TOKENS") => Ok(FinishReason::Length),
        // Long Decode stopped at the provider hop limit. OpenAI `length` is the
        // compatible signal; native Gemini re-emits CONTINUATION with the token.
        (UpstreamProtocol::Gemini, "CONTINUATION") => Ok(FinishReason::Continuation),
        (UpstreamProtocol::Chat, "content_filter")
        | (UpstreamProtocol::Anthropic, "refusal")
        | (
            UpstreamProtocol::Gemini,
            "SAFETY" | "RECITATION" | "BLOCKLIST" | "PROHIBITED_CONTENT" | "SPII" | "IMAGE_SAFETY",
        ) => Ok(FinishReason::ContentFilter),
        (UpstreamProtocol::Gemini, "STOP") => Ok(if tools {
            FinishReason::ToolCalls
        } else {
            FinishReason::Stop
        }),
        _ => Err(RelayError::Protocol("unsupported upstream finish reason")),
    }
}
fn plain_response_part(part: &Value) -> Result<(), RelayError> {
    if part
        .get("annotations")
        .is_some_and(|a| !a.is_null() && a.as_array().is_none_or(|a| !a.is_empty()))
    {
        return Err(RelayError::Protocol(
            "annotated Responses output is not supported",
        ));
    }
    Ok(())
}
fn plain_response_item(item: &Value) -> Result<(), RelayError> {
    if item.get("encrypted_content").is_some_and(|v| !v.is_null()) {
        return Err(RelayError::Protocol(
            "encrypted Responses output is not supported",
        ));
    }
    if let Some(parts) = item.get("content").filter(|v| !v.is_null()) {
        for part in parts
            .as_array()
            .ok_or(RelayError::Protocol("invalid Responses content"))?
        {
            plain_response_part(part)?;
        }
    }
    Ok(())
}
fn plain_response(response: &Value) -> Result<(), RelayError> {
    if let Some(items) = response.get("output").filter(|v| !v.is_null()) {
        for item in items
            .as_array()
            .ok_or(RelayError::Protocol("invalid Responses output"))?
        {
            plain_response_item(item)?;
        }
    }
    Ok(())
}

impl Decoder {
    pub fn new(protocol: UpstreamProtocol, limits: &Limits) -> Self {
        Self::new_with_cache_creation_mode(protocol, limits, None)
    }

    pub(crate) fn new_with_cache_creation_mode(
        protocol: UpstreamProtocol,
        limits: &Limits,
        cache_creation_mode: Option<CacheCreationMode>,
    ) -> Self {
        Self {
            protocol,
            cache_creation_mode,
            legacy_openrouter_usage: false,
            started: false,
            source_id: None,
            done: false,
            finish: None,
            blocks: BTreeMap::new(),
            max_blocks: limits.max_blocks,
            image_count: 0,
            event_bytes: limits.event_bytes,
            gemini_calls: 0,
            gemini_grounded_requests: None,
            continuation_token: None,
        }
    }

    pub(crate) fn with_legacy_openrouter_usage(mut self, enabled: bool) -> Self {
        self.legacy_openrouter_usage = enabled;
        self
    }

    fn usage(&self, value: &Value) -> Result<Usage, RelayError> {
        if self.legacy_openrouter_usage {
            if self.protocol != UpstreamProtocol::Chat
                || self.cache_creation_mode != Some(CacheCreationMode::GenericRemainderV1)
            {
                return Err(RelayError::Invalid("legacy OpenRouter usage contract"));
            }
            return super::legacy_openrouter_usage::project(value);
        }
        usage_with_cache_creation_mode(self.protocol, value, self.cache_creation_mode)
    }
    pub fn is_done(&self) -> bool {
        self.done
    }
    fn start(&mut self, id: Option<&str>, out: &mut Vec<Event>) -> Result<(), RelayError> {
        if let Some(id) = id {
            if id.is_empty() || id.len() > 256 || id.chars().any(char::is_control) {
                return Err(RelayError::Protocol("invalid upstream ID"));
            }
            if self.source_id.as_deref().is_some_and(|old| old != id) {
                return Err(RelayError::Protocol("upstream response ID changed"));
            }
            self.source_id = Some(id.to_owned());
        }
        if !self.started {
            self.started = true;
            out.push(Event::Start(self.source_id.clone().unwrap_or_default()));
        }
        Ok(())
    }
    fn tools(&self) -> bool {
        self.blocks
            .values()
            .any(|s| matches!(s.kind, BlockKind::Tool { .. }))
    }
    fn block(
        &mut self,
        key: Key,
        kind: BlockKind,
        out: &mut Vec<Event>,
    ) -> Result<usize, RelayError> {
        if self.finish.is_some() {
            return Err(RelayError::Protocol("content after finish"));
        }
        if let Some(s) = self.blocks.get(&key) {
            let compatible = match (&s.kind, &kind) {
                (BlockKind::Text, BlockKind::Text)
                | (BlockKind::Thinking, BlockKind::Thinking)
                | (BlockKind::Refusal, BlockKind::Refusal) => true,
                (BlockKind::Tool { id: a, name: b }, BlockKind::Tool { id: c, name: d }) => {
                    a == c && b == d
                }
                _ => false,
            };
            if s.ended || !compatible {
                return Err(RelayError::Protocol("invalid block transition"));
            }
            return Ok(s.index);
        }
        if self.blocks.len() + self.image_count >= self.max_blocks {
            return Err(RelayError::Limit("output block count"));
        }
        if let BlockKind::Tool { id, name } = &kind {
            if id.is_empty() || name.is_empty() || id.len() > 256 || name.len() > 128 {
                return Err(RelayError::Protocol("invalid tool identity"));
            }
            if self
                .blocks
                .values()
                .any(|s| matches!(&s.kind, BlockKind::Tool { id: old, .. } if old == id))
            {
                return Err(RelayError::Protocol("duplicate upstream tool ID"));
            }
        }
        let index = self.blocks.len();
        out.push(Event::Block {
            index,
            kind: kind.clone(),
        });
        self.blocks.insert(
            key,
            Slot {
                index,
                kind,
                ended: false,
                has_text: false,
                default_object: false,
            },
        );
        Ok(index)
    }
    fn delta(&mut self, key: Key, value: &str, out: &mut Vec<Event>) -> Result<(), RelayError> {
        let slot = self
            .blocks
            .get_mut(&key)
            .ok_or(RelayError::Protocol("delta without a block"))?;
        if slot.ended || self.finish.is_some() {
            return Err(RelayError::Protocol("delta after block end"));
        }
        if !value.is_empty() {
            slot.has_text = true;
            out.push(Event::Delta {
                index: slot.index,
                text: value.to_owned(),
            });
        }
        Ok(())
    }
    fn simple(
        &mut self,
        key: Key,
        kind: BlockKind,
        value: &str,
        out: &mut Vec<Event>,
    ) -> Result<(), RelayError> {
        self.block(key, kind, out)?;
        self.delta(key, value, out)
    }
    fn continuation(&mut self, token: &str, out: &mut Vec<Event>) -> Result<(), RelayError> {
        if token.is_empty() {
            return Ok(());
        }
        if token.len() > self.event_bytes || token.chars().any(char::is_control) {
            return Err(RelayError::Protocol("invalid Gemini continuation token"));
        }
        match &self.continuation_token {
            Some(existing) if existing == token => Ok(()),
            Some(_) => Err(RelayError::Protocol(
                "conflicting Gemini continuation token",
            )),
            None => {
                self.continuation_token = Some(token.to_owned());
                out.push(Event::ContinuationToken(token.to_owned()));
                Ok(())
            }
        }
    }
    fn signature(&self, key: Key, value: &str, out: &mut Vec<Event>) -> Result<(), RelayError> {
        let s = self
            .blocks
            .get(&key)
            .ok_or(RelayError::Protocol("signature without a block"))?;
        if s.ended {
            return Err(RelayError::Protocol("signature after block end"));
        }
        out.push(Event::Signature {
            index: s.index,
            provider: self.protocol,
            value: value.to_owned(),
        });
        Ok(())
    }
    fn close(&mut self, key: Key, out: &mut Vec<Event>) -> Result<(), RelayError> {
        let s = self
            .blocks
            .get_mut(&key)
            .ok_or(RelayError::Protocol("unknown block end"))?;
        if !s.ended {
            if s.default_object && !s.has_text {
                out.push(Event::Delta {
                    index: s.index,
                    text: "{}".into(),
                });
            }
            s.ended = true;
            out.push(Event::EndBlock(s.index));
        }
        Ok(())
    }
    fn close_all(&mut self, out: &mut Vec<Event>) -> Result<(), RelayError> {
        let keys: Vec<_> = self.blocks.keys().copied().collect();
        for key in keys {
            self.close(key, out)?;
        }
        Ok(())
    }
    fn finish(&mut self, reason: FinishReason, out: &mut Vec<Event>) -> Result<(), RelayError> {
        if self.finish.is_some() {
            return Err(RelayError::Protocol("duplicate finish"));
        }
        self.close_all(out)?;
        self.finish = Some(reason);
        out.push(Event::Finish(reason));
        Ok(())
    }
    fn done(&mut self, out: &mut Vec<Event>) -> Result<(), RelayError> {
        if self.finish.is_none() || self.done {
            return Err(RelayError::Truncated);
        }
        self.done = true;
        out.push(Event::Done);
        Ok(())
    }
    fn admit(&self, value: &Value, full: bool) -> Result<(), RelayError> {
        match self.protocol {
            UpstreamProtocol::Chat => {
                let choices = value["choices"]
                    .as_array()
                    .ok_or(RelayError::Protocol("missing choices"))?;
                if choices.len() > 1 || (full && choices.len() != 1) {
                    return Err(RelayError::Protocol(
                        "only one upstream candidate is supported",
                    ));
                }
                for choice in choices {
                    let message = &choice[if full { "message" } else { "delta" }];
                    if !message.is_object() {
                        return Err(RelayError::Protocol("missing message delta"));
                    }
                    if message.get("function_call").is_some() {
                        return Err(RelayError::Protocol(
                            "legacy function_call output is unsupported",
                        ));
                    }
                    for field in ["content", "reasoning_content", "refusal"] {
                        if let Some(value) = message.get(field).filter(|v| !v.is_null()) {
                            text(value)?;
                        }
                    }
                    if let Some(reason) = choice.get("finish_reason").filter(|v| !v.is_null()) {
                        finish_reason(self.protocol, text(reason)?, self.tools())?;
                    }
                }
            }
            UpstreamProtocol::Responses => {
                let response = if full {
                    Some(value)
                } else {
                    value.get("response")
                };
                if let Some(response) = response {
                    plain_response(response)?;
                    if full
                        && !matches!(
                            response["status"].as_str(),
                            Some("completed" | "incomplete")
                        )
                    {
                        return Err(RelayError::Protocol("upstream response is not complete"));
                    }
                    if response["status"] == "incomplete"
                        && !matches!(
                            response["incomplete_details"]["reason"].as_str(),
                            Some("max_output_tokens" | "content_filter")
                        )
                    {
                        return Err(RelayError::Protocol("unknown incomplete response"));
                    }
                }
                if let Some(item) = value.get("item") {
                    plain_response_item(item)?;
                }
                if let Some(part) = value.get("part") {
                    plain_response_part(part)?;
                }
            }
            UpstreamProtocol::Anthropic => {
                let delta = if full { value } else { &value["delta"] };
                if let Some(reason) = delta.get("stop_reason").filter(|v| !v.is_null()) {
                    let reason = text(reason)?;
                    finish_reason(self.protocol, reason, self.tools())?;
                    if reason == "stop_sequence" {
                        let sequence = text(&delta["stop_sequence"])?;
                        if sequence.is_empty() || sequence.len() > 1024 {
                            return Err(RelayError::Protocol("invalid stop sequence receipt"));
                        }
                    } else if delta.get("stop_sequence").is_some_and(|v| !v.is_null()) {
                        return Err(RelayError::Protocol(
                            "stop sequence without matching finish reason",
                        ));
                    }
                }
            }
            UpstreamProtocol::Gemini => {
                if let Some(candidates) = value.get("candidates") {
                    let candidates = candidates
                        .as_array()
                        .ok_or(RelayError::Protocol("invalid Gemini candidates"))?;
                    if candidates.len() > 1 {
                        return Err(RelayError::Protocol("multiple Gemini candidates"));
                    }
                    for candidate in candidates {
                        if let Some(reason) = candidate.get("finishReason") {
                            finish_reason(self.protocol, text(reason)?, self.tools())?;
                        }
                    }
                }
            }
        }
        Ok(())
    }
    pub fn frame(&mut self, frame: Frame) -> Result<Vec<Event>, RelayError> {
        if self.done {
            return Err(RelayError::Protocol("event after terminal event"));
        }
        let mut out = Vec::new();
        if frame.data.is_empty() {
            return Ok(out);
        }
        if frame.data == "[DONE]" {
            self.decode_frame(&frame, &Value::Null, &mut out)?;
            return Ok(out);
        }
        let value: Value = serde_json::from_str(&frame.data)
            .map_err(|_| RelayError::Protocol("invalid SSE JSON"))?;
        if !value.is_object() || value.get("error").is_some_and(|e| !e.is_null()) {
            return Err(RelayError::Protocol("invalid upstream event"));
        }
        match value["type"].as_str() {
            Some("message_start") if self.started => {
                return Err(RelayError::Protocol("duplicate message start"));
            }
            Some("content_block_start" | "response.output_item.added") if !self.started => {
                return Err(RelayError::Protocol("content before response start"));
            }
            _ => {}
        }
        self.admit(&value, false)?;
        self.decode_frame(&frame, &value, &mut out)?;
        Ok(out)
    }
    pub fn eof(&mut self) -> Result<Vec<Event>, RelayError> {
        let mut out = Vec::new();
        // Gemini ends with HTTP EOF, but still must have a semantic finish reason.
        if !self.done && self.protocol == UpstreamProtocol::Gemini {
            self.done(&mut out)?;
        }
        if !self.done {
            return Err(RelayError::Truncated);
        }
        Ok(out)
    }
    pub fn json(&mut self, value: &Value) -> Result<Vec<Event>, RelayError> {
        if !value.is_object() || value.get("error").is_some_and(|e| !e.is_null()) {
            return Err(RelayError::Protocol("invalid upstream JSON response"));
        }
        self.admit(value, true)?;
        let mut out = Vec::new();
        self.decode_response(value, &mut out)?;
        Ok(out)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn deepseek_receipt() -> Value {
        serde_json::json!({
            "prompt_tokens":100,
            "prompt_cache_hit_tokens":80,
            "prompt_cache_miss_tokens":20,
            "completion_tokens":10,
            "completion_tokens_details":{"reasoning_tokens":7},
            "total_tokens":110
        })
    }

    #[test]
    fn deepseek_cache_alias_normalizes_without_counting_reasoning_twice() {
        let mut receipt = deepseek_receipt();
        let observed = usage(UpstreamProtocol::Chat, &receipt).unwrap();
        assert_eq!(observed.input_tokens, Some(100));
        assert_eq!(observed.cached_input_tokens, Some(80));
        assert_eq!(observed.output_tokens, Some(10));
        assert_eq!(observed.reasoning_output_tokens, Some(7));

        receipt["prompt_tokens_details"] = serde_json::json!({"cached_tokens":80});
        assert_eq!(usage(UpstreamProtocol::Chat, &receipt).unwrap(), observed);
        receipt["prompt_tokens_details"]["cached_tokens"] = serde_json::json!(79);
        assert!(usage(UpstreamProtocol::Chat, &receipt).is_err());
    }

    #[test]
    fn deepseek_cache_alias_requires_consistent_provider_evidence() {
        for (field, invalid) in [
            ("prompt_cache_hit_tokens", serde_json::json!(-1)),
            ("prompt_cache_hit_tokens", serde_json::json!(101)),
            ("prompt_cache_hit_tokens", serde_json::json!("80")),
            ("prompt_cache_miss_tokens", serde_json::json!(-1)),
            ("prompt_cache_miss_tokens", serde_json::json!(19)),
            ("prompt_cache_miss_tokens", serde_json::json!(101)),
            ("prompt_cache_miss_tokens", serde_json::json!(0.5)),
        ] {
            let mut receipt = deepseek_receipt();
            receipt[field] = invalid;
            assert!(
                usage(UpstreamProtocol::Chat, &receipt).is_err(),
                "{receipt}"
            );
        }
        assert!(
            usage(
                UpstreamProtocol::Chat,
                &serde_json::json!({
                    "prompt_cache_hit_tokens":u64::MAX,"prompt_cache_miss_tokens":1
                })
            )
            .is_err()
        );

        let mut partial = deepseek_receipt();
        partial
            .as_object_mut()
            .unwrap()
            .remove("prompt_cache_hit_tokens");
        assert_eq!(
            usage(UpstreamProtocol::Chat, &partial)
                .unwrap()
                .cached_input_tokens,
            None
        );
        partial["prompt_cache_hit_tokens"] = Value::Null;
        assert_eq!(
            usage(UpstreamProtocol::Chat, &partial)
                .unwrap()
                .cached_input_tokens,
            None
        );
        partial["prompt_cache_hit_tokens"] = serde_json::json!(0);
        partial["prompt_cache_miss_tokens"] = serde_json::json!(100);
        assert_eq!(
            usage(UpstreamProtocol::Chat, &partial)
                .unwrap()
                .cached_input_tokens,
            Some(0)
        );
    }

    #[test]
    fn deepseek_cache_alias_survives_json_and_repeated_stream_usage() {
        let expected = usage(UpstreamProtocol::Chat, &deepseek_receipt()).unwrap();
        let mut decoder = Decoder::new(UpstreamProtocol::Chat, &Limits::default());
        let events = decoder
            .json(&completion(UpstreamProtocol::Chat, deepseek_receipt()))
            .unwrap();
        assert!(
            events
                .iter()
                .any(|event| matches!(event, Event::Usage(value) if *value == expected))
        );
        assert!(decoder.is_done());

        let mut decoder = Decoder::new(UpstreamProtocol::Chat, &Limits::default());
        let mut observed = Usage::default();
        for chunk in [
            serde_json::json!({"id":"synthetic-response","choices":[{"index":0,"delta":{"role":"assistant","content":"ok"},"finish_reason":null}]}),
            serde_json::json!({"id":"synthetic-response","choices":[{"index":0,"delta":{},"finish_reason":"stop"}]}),
            serde_json::json!({"id":"synthetic-response","choices":[],"usage":deepseek_receipt()}),
            serde_json::json!({"id":"synthetic-response","choices":[],"usage":deepseek_receipt()}),
        ] {
            for event in decoder
                .frame(Frame {
                    event: None,
                    data: chunk.to_string(),
                })
                .unwrap()
            {
                if let Event::Usage(value) = event {
                    observed.merge(value.clone()).unwrap();
                }
            }
        }
        decoder
            .frame(Frame {
                event: None,
                data: "[DONE]".into(),
            })
            .unwrap();
        assert!(decoder.is_done());
        assert_eq!(observed, expected);
    }

    #[test]
    fn usage_is_not_zero_when_absent_and_cache_is_normalized() {
        assert_eq!(
            usage(UpstreamProtocol::Chat, &Value::Null).unwrap(),
            Usage::default()
        );
        let u = usage(UpstreamProtocol::Anthropic,&serde_json::json!({"input_tokens":10,"cache_read_input_tokens":20,"cache_creation_input_tokens":30,"output_tokens":4})).unwrap();
        assert_eq!(u.input_tokens, Some(60));
        assert_eq!(u.output_tokens, Some(4));
        let g = usage(UpstreamProtocol::Gemini,&serde_json::json!({"promptTokenCount":5,"candidatesTokenCount":7,"thoughtsTokenCount":3})).unwrap();
        assert_eq!(g.output_tokens, Some(10));
        assert_eq!(g.reasoning_output_tokens, Some(3));
    }
    #[test]
    fn usage_overflow_and_negative_counters_are_errors() {
        assert!(
            usage(
                UpstreamProtocol::Chat,
                &serde_json::json!({"prompt_tokens":-1})
            )
            .is_err()
        );
        assert!(
            usage(
                UpstreamProtocol::Anthropic,
                &serde_json::json!({"input_tokens":u64::MAX,"cache_read_input_tokens":1})
            )
            .is_err()
        );
    }

    #[test]
    fn anthropic_cache_creation_split_normalizes_only_complete_evidence() {
        let decoded = usage(
            UpstreamProtocol::Anthropic,
            &serde_json::json!({
                "input_tokens":10,
                "cache_read_input_tokens":20,
                "cache_creation":{
                    "ephemeral_5m_input_tokens":30,
                    "ephemeral_1h_input_tokens":40
                }
            }),
        )
        .unwrap();
        assert_eq!(decoded.cache_creation_input_tokens, Some(70));
        assert_eq!(decoded.input_tokens, Some(100));
        assert_eq!(
            decoded.cache_creation_details,
            Some(CacheCreationDetails {
                ephemeral_5m_input_tokens: Some(30),
                ephemeral_1h_input_tokens: Some(40),
            })
        );
        let partial = usage(
            UpstreamProtocol::Anthropic,
            &serde_json::json!({
                "input_tokens":10,"cache_read_input_tokens":20,
                "cache_creation":{"ephemeral_5m_input_tokens":30}
            }),
        )
        .unwrap();
        assert_eq!(partial.cache_creation_input_tokens, None);
        assert_eq!(partial.input_tokens, None);
        assert_eq!(partial.cached_input_tokens, Some(20));
        for invalid_input in [serde_json::json!(-1), serde_json::json!(u64::MAX)] {
            assert!(
                usage(
                    UpstreamProtocol::Anthropic,
                    &serde_json::json!({
                        "input_tokens":invalid_input,"cache_read_input_tokens":1,
                        "cache_creation":{"ephemeral_5m_input_tokens":0}
                    }),
                )
                .is_err()
            );
        }
        for detail in [
            serde_json::json!({"ephemeral_5m_input_tokens":0}),
            serde_json::json!({"ephemeral_1h_input_tokens":0}),
            serde_json::json!({}),
        ] {
            let decoded = usage(
                UpstreamProtocol::Anthropic,
                &serde_json::json!({"cache_creation":detail}),
            )
            .unwrap();
            assert_eq!(decoded.cache_creation_input_tokens, None);
            assert_eq!(decoded.input_tokens, None);
            assert_eq!(
                serde_json::to_value(decoded.cache_creation_details.unwrap()).unwrap(),
                detail
            );
        }
        for details in [Value::Null, serde_json::json!({})] {
            let decoded = usage(
                UpstreamProtocol::Anthropic,
                &serde_json::json!({"cache_creation":details}),
            )
            .unwrap();
            assert_eq!(
                decoded.cache_creation_details.is_some(),
                details.is_object()
            );
        }
    }

    #[test]
    fn cache_creation_details_reject_invalid_shapes_counters_and_totals() {
        for (protocol, key) in [
            (UpstreamProtocol::Anthropic, "cache_creation"),
            (UpstreamProtocol::Chat, "cache_creation_details"),
            (UpstreamProtocol::Responses, "cache_creation_details"),
        ] {
            for invalid in [
                serde_json::json!(1),
                serde_json::json!([]),
                serde_json::json!({"ephemeral_5m_input_tokens":-1}),
                serde_json::json!({"ephemeral_1h_input_tokens":1.5}),
                serde_json::json!({"ephemeral_5m_input_tokens":true}),
                serde_json::json!({"ephemeral_1h_input_tokens":"1"}),
                serde_json::json!({"unknown_duration":1}),
                serde_json::json!({"ephemeral_5m_input_tokens":u64::MAX,"ephemeral_1h_input_tokens":1}),
            ] {
                let mut receipt = serde_json::json!({});
                receipt[key] = invalid;
                assert!(usage(protocol, &receipt).is_err(), "{receipt}");
            }
            for invalid in [
                serde_json::json!({"ephemeral_5m_input_tokens":2,"ephemeral_1h_input_tokens":2}),
                serde_json::json!({"ephemeral_5m_input_tokens":6}),
                serde_json::json!({"ephemeral_1h_input_tokens":6}),
            ] {
                let mut receipt = serde_json::json!({"cache_creation_input_tokens":5});
                receipt[key] = invalid;
                assert!(usage(protocol, &receipt).is_err(), "{receipt}");
            }
            let mut receipt = serde_json::json!({"cache_creation_input_tokens":5});
            receipt[key] = serde_json::json!({"ephemeral_5m_input_tokens":2});
            assert!(usage(protocol, &receipt).is_ok());
        }
    }

    #[test]
    fn cache_creation_split_wire_round_trip_preserves_partial_and_explicit_zero() {
        for details in [
            CacheCreationDetails {
                ephemeral_5m_input_tokens: Some(2),
                ephemeral_1h_input_tokens: Some(3),
            },
            CacheCreationDetails {
                ephemeral_5m_input_tokens: Some(0),
                ephemeral_1h_input_tokens: None,
            },
            CacheCreationDetails::default(),
        ] {
            let expected = Usage {
                input_tokens: Some(10),
                output_tokens: Some(4),
                cache_creation_input_tokens: Some(5),
                cache_creation_details: Some(details),
                ..Usage::default()
            };
            for (protocol, client) in [
                (UpstreamProtocol::Chat, crate::relay::ClientProtocol::Chat),
                (
                    UpstreamProtocol::Responses,
                    crate::relay::ClientProtocol::Responses,
                ),
            ] {
                let wire = expected.wire(client);
                assert_eq!(wire["cache_creation_details"], serde_json::json!(details));
                assert_eq!(usage(protocol, &wire).unwrap(), expected);
            }
        }
    }

    fn openai_receipt(protocol: UpstreamProtocol) -> Value {
        match protocol {
            UpstreamProtocol::Chat => serde_json::json!({
                "prompt_tokens": 10,
                "completion_tokens": 4,
                "total_tokens": 14,
                "prompt_tokens_details": {"cached_tokens": 3},
                "completion_tokens_details": {"reasoning_tokens": 1},
                "cache_creation_input_tokens": 2
            }),
            UpstreamProtocol::Responses => serde_json::json!({
                "input_tokens": 10,
                "output_tokens": 4,
                "total_tokens": 14,
                "input_tokens_details": {"cached_tokens": 3},
                "output_tokens_details": {"reasoning_tokens": 1},
                "cache_creation_input_tokens": 2
            }),
            _ => unreachable!(),
        }
    }

    #[test]
    fn openai_cache_creation_aliases_use_max_and_preserve_unknown_and_zero() {
        for (protocol, field) in [
            (UpstreamProtocol::Chat, "prompt_tokens_details"),
            (UpstreamProtocol::Responses, "input_tokens_details"),
        ] {
            for (created, written, nested, expected) in [
                (None, None, None, None),
                (Some(0), None, None, Some(0)),
                (None, Some(0), None, Some(0)),
                (None, None, Some(0), Some(0)),
                (None, None, Some(5), Some(5)),
                (Some(2), Some(5), None, Some(5)),
                (Some(5), Some(2), None, Some(5)),
                (Some(5), Some(5), None, Some(5)),
                (Some(2), Some(5), Some(7), Some(7)),
                (Some(7), Some(5), Some(2), Some(7)),
                (Some(2), Some(7), Some(5), Some(7)),
                (Some(5), Some(5), Some(5), Some(5)),
                (
                    Some(u64::MAX),
                    Some(u64::MAX),
                    Some(u64::MAX),
                    Some(u64::MAX),
                ),
            ] {
                let mut receipt = serde_json::json!({});
                receipt[field] = serde_json::json!({});
                if let Some(value) = created {
                    receipt[field]["cached_creation_tokens"] = serde_json::json!(value);
                }
                if let Some(value) = written {
                    receipt[field]["cache_write_tokens"] = serde_json::json!(value);
                }
                if let Some(value) = nested {
                    receipt[field]["cache_creation_input_tokens"] = serde_json::json!(value);
                }
                let decoded = usage(protocol, &receipt).unwrap();
                assert_eq!(decoded.cache_creation_input_tokens, expected, "{receipt}");
                assert_eq!(decoded.cache_creation_details, None);
            }
            let mut receipt = serde_json::json!({});
            receipt[field] = serde_json::json!({
                "cached_creation_tokens":null,"cache_write_tokens":null,
                "cache_creation_input_tokens":null
            });
            assert_eq!(
                usage(protocol, &receipt)
                    .unwrap()
                    .cache_creation_input_tokens,
                None
            );
        }
    }

    #[test]
    fn nested_cache_creation_alias_preserves_inclusive_input_and_cache_bounds() {
        for (protocol, input, output, field) in [
            (
                UpstreamProtocol::Chat,
                "prompt_tokens",
                "completion_tokens",
                "prompt_tokens_details",
            ),
            (
                UpstreamProtocol::Responses,
                "input_tokens",
                "output_tokens",
                "input_tokens_details",
            ),
        ] {
            let mut receipt = serde_json::json!({});
            receipt[input] = serde_json::json!(100);
            receipt[output] = serde_json::json!(5);
            receipt[field] = serde_json::json!({
                "cached_tokens":20,"cache_write_tokens":10,
                "cache_creation_input_tokens":30
            });
            let decoded = usage(protocol, &receipt).unwrap();
            assert_eq!(decoded.input_tokens, Some(100), "{receipt}");
            assert_eq!(decoded.output_tokens, Some(5), "{receipt}");
            assert_eq!(decoded.cached_input_tokens, Some(20), "{receipt}");
            assert_eq!(decoded.cache_creation_input_tokens, Some(30), "{receipt}");

            receipt["cache_creation_input_tokens"] = serde_json::json!(20);
            assert!(usage(protocol, &receipt).is_err(), "{receipt}");
            receipt
                .as_object_mut()
                .unwrap()
                .remove("cache_creation_input_tokens");
            receipt[field]["cached_tokens"] = serde_json::json!(71);
            assert!(usage(protocol, &receipt).is_err(), "{receipt}");
        }
    }

    #[test]
    fn openai_cache_creation_aliases_must_match_canonical_and_ttl_totals() {
        for (protocol, field) in [
            (UpstreamProtocol::Chat, "prompt_tokens_details"),
            (UpstreamProtocol::Responses, "input_tokens_details"),
        ] {
            let mut receipt = serde_json::json!({"cache_creation_input_tokens":5});
            receipt[field] = serde_json::json!({
                "cached_creation_tokens":2,"cache_write_tokens":3,
                "cache_creation_input_tokens":5
            });
            assert_eq!(
                usage(protocol, &receipt)
                    .unwrap()
                    .cache_creation_input_tokens,
                Some(5)
            );
            for canonical in [0, 4, 6] {
                receipt["cache_creation_input_tokens"] = serde_json::json!(canonical);
                assert!(usage(protocol, &receipt).is_err(), "{receipt}");
            }
            receipt
                .as_object_mut()
                .unwrap()
                .remove("cache_creation_input_tokens");
            receipt["cache_creation_details"] = serde_json::json!({
                "ephemeral_5m_input_tokens":2,"ephemeral_1h_input_tokens":3
            });
            assert_eq!(
                usage(protocol, &receipt)
                    .unwrap()
                    .cache_creation_input_tokens,
                Some(5)
            );
            receipt[field]["cache_write_tokens"] = serde_json::json!(6);
            assert!(usage(protocol, &receipt).is_err());
            receipt["cache_creation_details"] = serde_json::json!({"ephemeral_5m_input_tokens":7});
            assert!(usage(protocol, &receipt).is_err());
            receipt["cache_creation_details"] = Value::Null;
            receipt[match protocol {
                UpstreamProtocol::Chat => "prompt_tokens",
                UpstreamProtocol::Responses => "input_tokens",
                _ => unreachable!(),
            }] = serde_json::json!(5);
            assert!(usage(protocol, &receipt).is_err());
        }
    }

    #[test]
    fn openai_cache_creation_aliases_reject_invalid_counters_before_taking_max() {
        for (protocol, field) in [
            (UpstreamProtocol::Chat, "prompt_tokens_details"),
            (UpstreamProtocol::Responses, "input_tokens_details"),
        ] {
            for alias in [
                "cached_creation_tokens",
                "cache_write_tokens",
                "cache_creation_input_tokens",
            ] {
                for invalid in [
                    serde_json::json!(-1),
                    serde_json::json!(1.0),
                    serde_json::json!(true),
                    serde_json::json!("2"),
                    serde_json::json!([]),
                    serde_json::json!({}),
                    serde_json::from_str::<Value>("18446744073709551616").unwrap(),
                ] {
                    let mut receipt = serde_json::json!({"cache_creation_input_tokens":5});
                    receipt[field] = serde_json::json!({
                        "cached_creation_tokens":5,"cache_write_tokens":5,
                        "cache_creation_input_tokens":5
                    });
                    receipt[field][alias] = invalid;
                    assert!(usage(protocol, &receipt).is_err(), "{receipt}");
                }
            }
        }
    }

    #[test]
    fn a07_usage_wire_round_trip_preserves_cache_creation() {
        let expected = Usage {
            tool: None,
            google_search_grounded_requests: None,
            tool_use_prompt_tokens: None,

            upstream_cost_atto_usd: None,
            uncached_input_tokens: None,
            input_tokens: Some(10),
            output_tokens: Some(4),
            cached_input_tokens: Some(3),
            cache_creation_input_tokens: Some(2),
            cache_creation_details: None,
            openai_cache_write_details: None,
            cache_creation_mode: None,
            legacy_openrouter_raw: None,
            reasoning_output_tokens: Some(1),
        };
        for (protocol, client) in [
            (UpstreamProtocol::Chat, crate::relay::ClientProtocol::Chat),
            (
                UpstreamProtocol::Responses,
                crate::relay::ClientProtocol::Responses,
            ),
        ] {
            assert_eq!(
                usage(protocol, &expected.wire(client)),
                Ok(expected.clone())
            );
            assert_eq!(
                usage(protocol, &openai_receipt(protocol)),
                Ok(expected.clone())
            );
        }
    }

    #[test]
    fn a07_present_non_object_details_are_not_missing_usage() {
        for (protocol, fields) in [
            (
                UpstreamProtocol::Chat,
                ["prompt_tokens_details", "completion_tokens_details"],
            ),
            (
                UpstreamProtocol::Responses,
                ["input_tokens_details", "output_tokens_details"],
            ),
        ] {
            for field in fields {
                for invalid in [
                    serde_json::json!(0),
                    serde_json::json!(1.5),
                    serde_json::json!(false),
                    serde_json::json!("invalid"),
                    serde_json::json!([]),
                ] {
                    let mut receipt = openai_receipt(protocol);
                    receipt[field] = invalid;
                    assert!(matches!(
                        usage(protocol, &receipt),
                        Err(RelayError::Protocol(_))
                    ));
                }
            }
        }
    }

    #[test]
    fn a07_absent_null_and_empty_details_keep_counters_unknown() {
        for (protocol, fields) in [
            (
                UpstreamProtocol::Chat,
                ["prompt_tokens_details", "completion_tokens_details"],
            ),
            (
                UpstreamProtocol::Responses,
                ["input_tokens_details", "output_tokens_details"],
            ),
        ] {
            for value in [Value::Null, serde_json::json!({})] {
                let mut receipt = openai_receipt(protocol);
                for field in fields {
                    receipt[field] = value.clone();
                }
                let observed = usage(protocol, &receipt).unwrap();
                assert_eq!(observed.cached_input_tokens, None);
                assert_eq!(observed.reasoning_output_tokens, None);
                assert_eq!(observed.cache_creation_input_tokens, Some(2));
            }
            assert_eq!(
                usage(protocol, &serde_json::json!({})),
                Ok(Usage::default())
            );
        }
    }

    #[test]
    fn a07_cache_write_and_total_counters_must_be_unsigned_integers() {
        for protocol in [UpstreamProtocol::Chat, UpstreamProtocol::Responses] {
            for field in ["cache_creation_input_tokens", "total_tokens"] {
                for invalid in [
                    serde_json::json!(-1),
                    serde_json::json!(1.5),
                    serde_json::json!(true),
                    serde_json::json!("2"),
                    serde_json::json!([]),
                    serde_json::json!({}),
                ] {
                    let mut receipt = openai_receipt(protocol);
                    receipt[field] = invalid;
                    assert!(usage(protocol, &receipt).is_err());
                }
            }
        }
    }

    #[test]
    fn a07_reported_total_must_match_known_input_and_output() {
        for protocol in [UpstreamProtocol::Chat, UpstreamProtocol::Responses] {
            for total in [0, 13, 15, u64::MAX] {
                let mut receipt = openai_receipt(protocol);
                receipt["total_tokens"] = serde_json::json!(total);
                assert!(usage(protocol, &receipt).is_err());
            }
            assert!(usage(protocol, &openai_receipt(protocol)).is_ok());
        }
    }

    #[test]
    fn a07_total_does_not_create_missing_input_or_output() {
        for (protocol, input, output) in [
            (UpstreamProtocol::Chat, "prompt_tokens", "completion_tokens"),
            (UpstreamProtocol::Responses, "input_tokens", "output_tokens"),
        ] {
            let only_total = serde_json::json!({"total_tokens": 14});
            assert_eq!(usage(protocol, &only_total), Ok(Usage::default()));
            for known_field in [input, output] {
                let mut receipt = only_total.clone();
                receipt[known_field] = serde_json::json!(10);
                let observed = usage(protocol, &receipt).unwrap();
                assert_eq!(observed.input_tokens, (known_field == input).then_some(10));
                assert_eq!(
                    observed.output_tokens,
                    (known_field == output).then_some(10)
                );
                receipt["total_tokens"] = serde_json::json!(9);
                assert!(usage(protocol, &receipt).is_err());
            }
        }
    }

    #[test]
    fn a07_cache_and_reasoning_subsets_are_checked_during_decode() {
        for (protocol, reasoning_details) in [
            (UpstreamProtocol::Chat, "completion_tokens_details"),
            (UpstreamProtocol::Responses, "output_tokens_details"),
        ] {
            let mut receipt = openai_receipt(protocol);
            receipt["cache_creation_input_tokens"] = serde_json::json!(8);
            assert!(usage(protocol, &receipt).is_err());
            let mut receipt = openai_receipt(protocol);
            receipt[reasoning_details]["reasoning_tokens"] = serde_json::json!(5);
            assert!(usage(protocol, &receipt).is_err());
        }
        assert!(
            usage(
                UpstreamProtocol::Gemini,
                &serde_json::json!({
                    "promptTokenCount": 3, "cachedContentTokenCount": 4
                })
            )
            .is_err()
        );
    }

    #[test]
    fn a07_total_overflow_is_rejected_before_emitting_usage() {
        for (protocol, receipt) in [
            (
                UpstreamProtocol::Chat,
                serde_json::json!({
                    "prompt_tokens": u64::MAX, "completion_tokens": 1
                }),
            ),
            (
                UpstreamProtocol::Responses,
                serde_json::json!({
                    "input_tokens": u64::MAX, "output_tokens": 1
                }),
            ),
            (
                UpstreamProtocol::Anthropic,
                serde_json::json!({
                    "input_tokens": u64::MAX, "output_tokens": 1
                }),
            ),
            (
                UpstreamProtocol::Gemini,
                serde_json::json!({
                    "promptTokenCount": u64::MAX, "candidatesTokenCount": 1
                }),
            ),
        ] {
            assert!(usage(protocol, &receipt).is_err());
        }
    }

    #[test]
    fn a07_explicit_zero_cache_creation_is_not_lost() {
        for protocol in [UpstreamProtocol::Chat, UpstreamProtocol::Responses] {
            let mut receipt = openai_receipt(protocol);
            receipt["cache_creation_input_tokens"] = serde_json::json!(0);
            assert_eq!(
                usage(protocol, &receipt)
                    .unwrap()
                    .cache_creation_input_tokens,
                Some(0)
            );
            receipt["cache_creation_input_tokens"] = Value::Null;
            assert_eq!(
                usage(protocol, &receipt)
                    .unwrap()
                    .cache_creation_input_tokens,
                None
            );
            receipt
                .as_object_mut()
                .unwrap()
                .remove("cache_creation_input_tokens");
            assert_eq!(
                usage(protocol, &receipt)
                    .unwrap()
                    .cache_creation_input_tokens,
                None
            );
        }
    }

    fn completion(protocol: UpstreamProtocol, receipt: Value) -> Value {
        match protocol {
            UpstreamProtocol::Chat => serde_json::json!({
                "id": "synthetic-response",
                "choices": [{"index": 0, "message": {"content": "ok"}, "finish_reason": "stop"}],
                "usage": receipt
            }),
            UpstreamProtocol::Responses => serde_json::json!({
                "id": "synthetic-response", "status": "completed", "output": [], "usage": receipt
            }),
            _ => unreachable!(),
        }
    }

    #[test]
    fn a07_non_streaming_decoder_preserves_billable_evidence() {
        for protocol in [UpstreamProtocol::Chat, UpstreamProtocol::Responses] {
            let mut decoder = Decoder::new(protocol, &Limits::default());
            let events = decoder
                .json(&completion(protocol, openai_receipt(protocol)))
                .unwrap();
            assert!(events.iter().any(|event| matches!(
                event,
                Event::Usage(Usage {
                    cache_creation_input_tokens: Some(2),
                    ..
                })
            )));
            assert!(decoder.is_done());
            let mut decoder = Decoder::new(protocol, &Limits::default());
            let mut bad = openai_receipt(protocol);
            bad["cache_creation_input_tokens"] = serde_json::json!("invalid");
            assert!(decoder.json(&completion(protocol, bad)).is_err());
            assert!(!decoder.is_done());
        }
    }

    #[test]
    fn a07_malformed_stream_receipt_cannot_complete_the_decoder() {
        for protocol in [UpstreamProtocol::Chat, UpstreamProtocol::Responses] {
            let mut decoder = Decoder::new(protocol, &Limits::default());
            let mut bad = openai_receipt(protocol);
            bad["cache_creation_input_tokens"] = serde_json::json!("invalid");
            let terminal = match protocol {
                UpstreamProtocol::Chat => serde_json::json!({"choices": [], "usage": bad}),
                UpstreamProtocol::Responses => serde_json::json!({
                    "type": "response.completed", "response": completion(protocol, bad)
                }),
                _ => unreachable!(),
            };
            assert!(
                decoder
                    .frame(Frame {
                        event: None,
                        data: terminal.to_string()
                    })
                    .is_err()
            );
            assert!(!decoder.is_done());
            assert_eq!(decoder.eof().unwrap_err(), RelayError::Truncated);
        }
    }

    #[test]
    fn a07_responses_final_receipt_cannot_change_response_identity() {
        for wrong_id in [
            serde_json::json!("different-response"),
            serde_json::json!(""),
            serde_json::json!("invalid\nidentity"),
            serde_json::json!(7),
            serde_json::json!(false),
            serde_json::json!([]),
            serde_json::json!({}),
        ] {
            let mut decoder = Decoder::new(UpstreamProtocol::Responses, &Limits::default());
            decoder
                .frame(Frame {
                    event: Some("response.created".into()),
                    data: serde_json::json!({
                        "type": "response.created", "response": {"id": "synthetic-response"}
                    })
                    .to_string(),
                })
                .unwrap();
            let mut response = completion(
                UpstreamProtocol::Responses,
                openai_receipt(UpstreamProtocol::Responses),
            );
            response["id"] = wrong_id;
            let result = decoder.frame(Frame {
                event: Some("response.completed".into()),
                data: serde_json::json!({
                    "type": "response.completed", "response": response
                })
                .to_string(),
            });
            assert!(matches!(result, Err(RelayError::Protocol(_))));
            assert!(!decoder.is_done());
        }
    }

    #[test]
    fn a07_matching_responses_terminal_retains_cache_write_evidence() {
        let mut decoder = Decoder::new(UpstreamProtocol::Responses, &Limits::default());
        decoder
            .frame(Frame {
                event: Some("response.created".into()),
                data: serde_json::json!({
                    "type": "response.created", "response": {"id": "synthetic-response"}
                })
                .to_string(),
            })
            .unwrap();
        let events = decoder
            .frame(Frame {
                event: Some("response.completed".into()),
                data: serde_json::json!({
                    "type": "response.completed", "response": completion(
                        UpstreamProtocol::Responses, openai_receipt(UpstreamProtocol::Responses)
                    )
                })
                .to_string(),
            })
            .unwrap();
        assert!(events.iter().any(|event| matches!(
            event,
            Event::Usage(Usage {
                cache_creation_input_tokens: Some(2),
                ..
            })
        )));
        assert!(events.iter().any(|event| matches!(event, Event::Done)));
        assert!(!events.iter().any(|event| matches!(event, Event::Start(_))));
        assert!(decoder.is_done());
    }

    #[test]
    fn live_shaped_responses_completed_json_decodes() {
        // Live gpt-6.1-sol / OpenAI Responses receipts include phase and
        // access_programs; decoder must accept them as ignored metadata.
        let value = serde_json::json!({
            "id": "resp_live_shaped",
            "object": "response",
            "status": "completed",
            "access_programs": {"cyber": "standard"},
            "billing": {"payer": "openai"},
            "image": {"context_compression": "disabled"},
            "output": [{
                "id": "msg_live",
                "type": "message",
                "status": "completed",
                "role": "assistant",
                "phase": "final_answer",
                "content": [{"type": "output_text", "text": "OK", "annotations": [], "logprobs": []}]
            }],
            "usage": {
                "input_tokens": 11,
                "output_tokens": 5,
                "total_tokens": 16,
                "input_tokens_details": {"cached_tokens": 0, "cache_write_tokens": 0},
                "output_tokens_details": {"reasoning_tokens": 0}
            }
        });
        let mut decoder = Decoder::new(UpstreamProtocol::Responses, &Limits::default());
        let events = decoder
            .json(&value)
            .expect("live-shaped Responses JSON should decode");
        assert!(events.iter().any(|e| matches!(e, Event::Done)));
        assert!(events.iter().any(|e| matches!(e, Event::Usage(_))));
        assert!(decoder.is_done());
    }
}

#[cfg(test)]
mod openai_cache_write_tests {
    use super::*;
    use serde_json::json;

    fn details_key(protocol: UpstreamProtocol) -> &'static str {
        match protocol {
            UpstreamProtocol::Chat => "prompt_tokens_details",
            UpstreamProtocol::Responses => "input_tokens_details",
            _ => unreachable!(),
        }
    }

    fn receipt(protocol: UpstreamProtocol) -> Value {
        let (input, output) = match protocol {
            UpstreamProtocol::Chat => ("prompt_tokens", "completion_tokens"),
            UpstreamProtocol::Responses => ("input_tokens", "output_tokens"),
            _ => unreachable!(),
        };
        let details = details_key(protocol);
        json!({input:12000,output:10,details:{
            "cached_tokens":4000,"cache_write_tokens":7000,
            "cache_write_tokens_details":{"ttl_30m_tokens":3000,"ttl_12h_tokens":4000}
        }})
    }

    fn response(protocol: UpstreamProtocol, receipt: Value, stream: bool) -> Value {
        match protocol {
            UpstreamProtocol::Chat if stream => json!({
                "id":"cache-write-response","choices":[{
                    "index":0,"delta":{"content":"ok"},"finish_reason":"stop"
                }],"usage":receipt
            }),
            UpstreamProtocol::Chat => json!({
                "id":"cache-write-response","choices":[{
                    "index":0,"message":{"content":"ok"},"finish_reason":"stop"
                }],"usage":receipt
            }),
            UpstreamProtocol::Responses => {
                let value = json!({
                    "id":"cache-write-response","status":"completed","output":[],"usage":receipt
                });
                if stream {
                    json!({"type":"response.completed","response":value})
                } else {
                    value
                }
            }
            _ => unreachable!(),
        }
    }

    #[test]
    fn both_openai_protocols_preserve_the_reported_lifetime_partition() {
        for protocol in [UpstreamProtocol::Chat, UpstreamProtocol::Responses] {
            let value = usage(protocol, &receipt(protocol)).unwrap();
            assert_eq!(value.input_tokens, Some(12000));
            assert_eq!(value.cached_input_tokens, Some(4000));
            assert_eq!(value.cache_creation_input_tokens, Some(7000));
            assert_eq!(
                value.openai_cache_write_details,
                Some(OpenAiCacheWriteDetails {
                    ttl_30m_tokens: Some(3000),
                    ttl_12h_tokens: Some(4000),
                })
            );
            assert_eq!(value.cache_creation_details, None);
            for (output_protocol, client) in [
                (UpstreamProtocol::Chat, crate::relay::ClientProtocol::Chat),
                (
                    UpstreamProtocol::Responses,
                    crate::relay::ClientProtocol::Responses,
                ),
            ] {
                assert_eq!(usage(output_protocol, &value.wire(client)).unwrap(), value);
            }
        }
    }

    #[test]
    fn missing_lifetimes_and_aggregate_are_never_invented() {
        for protocol in [UpstreamProtocol::Chat, UpstreamProtocol::Responses] {
            let key = details_key(protocol);
            for breakdown in [Value::Null, json!({}), json!({"ttl_12h_tokens":null})] {
                let mut body = receipt(protocol);
                body[key]["cache_write_tokens_details"] = breakdown.clone();
                let value = usage(protocol, &body).unwrap();
                assert_eq!(
                    value
                        .openai_cache_write_details
                        .and_then(|v| v.ttl_12h_tokens),
                    None
                );
                assert_eq!(value.cache_creation_input_tokens, Some(7000));
            }
            let mut body = receipt(protocol);
            body[key]["cache_write_tokens_details"] = json!({"ttl_12h_tokens":0});
            assert_eq!(
                usage(protocol, &body)
                    .unwrap()
                    .openai_cache_write_details
                    .unwrap()
                    .ttl_12h_tokens,
                Some(0)
            );
            body[key]
                .as_object_mut()
                .unwrap()
                .remove("cache_write_tokens");
            body[key]["cache_write_tokens_details"] =
                json!({"ttl_30m_tokens":3000,"ttl_12h_tokens":4000});
            let value = usage(protocol, &body).unwrap();
            assert_eq!(value.cache_creation_input_tokens, None);
            assert_eq!(value.effective_cache_creation_input_tokens().unwrap(), None);
            assert_eq!(
                value.openai_cache_write_details.unwrap().ttl_12h_tokens,
                Some(4000)
            );
        }
    }

    #[test]
    fn invalid_lifetime_objects_counters_and_partitions_are_rejected() {
        for protocol in [UpstreamProtocol::Chat, UpstreamProtocol::Responses] {
            let key = details_key(protocol);
            for invalid in [
                json!(false),
                json!(1),
                json!("1"),
                json!([]),
                json!([3000, 4000]),
                json!({"ttl_12h_tokens":-1}),
                json!({"ttl_12h_tokens":1.5}),
                json!({"ttl_12h_tokens":"4000"}),
                json!({"ttl_12h_tokens":true}),
                json!({"ttl_12h_tokens":[]}),
                json!({"ttl_12h_tokens":{}}),
                json!({"ttl_12h_tokens":7001}),
                json!({"ttl_30m_tokens":3000,"ttl_12h_tokens":3999}),
                json!({"ttl_30m_tokens":3000,"ttl_12h_tokens":4001}),
                json!({"ttl_30m_tokens":u64::MAX,"ttl_12h_tokens":1}),
                json!({"ttl_12h_tokens":4000,"unreviewed_lifetime":3000}),
            ] {
                let mut body = receipt(protocol);
                body[key]["cache_write_tokens_details"] = invalid.clone();
                assert!(usage(protocol, &body).is_err(), "{protocol:?}: {invalid}");
            }
            let mut body = receipt(protocol);
            body["cache_creation_input_tokens"] = json!(6999);
            assert!(usage(protocol, &body).is_err());
            body["cache_creation_input_tokens"] = json!(7000);
            body["cache_creation_details"] = json!({"ephemeral_5m_input_tokens":1});
            assert!(usage(protocol, &body).is_err());
        }
    }

    #[test]
    fn json_and_sse_terminals_produce_identical_cache_write_evidence() {
        for protocol in [UpstreamProtocol::Chat, UpstreamProtocol::Responses] {
            let expected = usage(protocol, &receipt(protocol)).unwrap();
            for stream in [false, true] {
                let mut decoder = Decoder::new(protocol, &Limits::default());
                let body = response(protocol, receipt(protocol), stream);
                let mut events = if stream {
                    decoder
                        .frame(Frame {
                            event: None,
                            data: body.to_string(),
                        })
                        .unwrap()
                } else {
                    decoder.json(&body).unwrap()
                };
                if stream && protocol == UpstreamProtocol::Chat {
                    events.extend(
                        decoder
                            .frame(Frame {
                                event: None,
                                data: "[DONE]".into(),
                            })
                            .unwrap(),
                    );
                }
                let mut combined = Usage::default();
                for event in events {
                    if let Event::Usage(value) = event {
                        combined.merge(value.clone()).unwrap();
                    }
                }
                assert_eq!(combined, expected);
                assert!(decoder.is_done());
            }
        }
    }

    #[test]
    fn invalid_sse_lifetimes_cannot_form_a_successful_terminal() {
        for protocol in [UpstreamProtocol::Chat, UpstreamProtocol::Responses] {
            let mut body = receipt(protocol);
            body[details_key(protocol)]["cache_write_tokens_details"]["ttl_12h_tokens"] =
                json!(7001);
            let mut decoder = Decoder::new(protocol, &Limits::default());
            assert!(
                decoder
                    .frame(Frame {
                        event: None,
                        data: response(protocol, body, true).to_string(),
                    })
                    .is_err()
            );
            assert!(!decoder.is_done());
            assert_eq!(decoder.eof().unwrap_err(), RelayError::Truncated);
        }
    }
}

#[cfg(test)]
mod upstream_cost_tests {
    use super::*;
    #[test]
    fn upstream_cost_is_exact_and_distinguishes_zero_missing_and_invalid() {
        for protocol in [UpstreamProtocol::Chat, UpstreamProtocol::Responses] {
            let value: Value = serde_json::from_str(
                r#"{"cost":0.000002000000000001,"cost_details":{"upstream_inference_cost":99}}"#,
            )
            .unwrap();
            let actual = usage(protocol, &value).unwrap();
            assert_eq!(actual.upstream_cost_atto_usd, Some(2_000_000_000_001));
            assert_eq!(
                usage(protocol, &serde_json::json!({"cost":0}))
                    .unwrap()
                    .upstream_cost_atto_usd,
                Some(0)
            );
            assert_eq!(
                usage(protocol, &serde_json::json!({}))
                    .unwrap()
                    .upstream_cost_atto_usd,
                None
            );
            let mut initial = usage(protocol, &serde_json::json!({"cost":0})).unwrap();
            initial
                .merge(
                    usage(
                        protocol,
                        &serde_json::json!({"prompt_tokens":0,"completion_tokens":0}),
                    )
                    .unwrap(),
                )
                .unwrap();
            assert_eq!(initial.upstream_cost_atto_usd, None);
            for invalid in [
                r#"{"cost":-1}"#,
                r#"{"cost":"0.1"}"#,
                r#"{"cost":{}}"#,
                r#"{"cost":1e-19}"#,
            ] {
                let value: Value = serde_json::from_str(invalid).unwrap();
                assert!(usage(protocol, &value).is_err());
            }
        }
    }
}

#[cfg(test)]
mod generic_creation_tests {
    use super::*;
    use serde_json::json;

    fn decode(protocol: UpstreamProtocol, value: &Value) -> Result<Usage, RelayError> {
        usage_with_cache_creation_mode(protocol, value, Some(CacheCreationMode::GenericRemainderV1))
    }

    #[test]
    fn generic_anthropic_preserves_aggregate_and_normalizes_with_effective_total() {
        for (aggregate, five, hour, expected) in [(2, 3, 4, 37), (50, 2, 3, 80), (0, 3, 4, 37)] {
            let value = json!({"input_tokens":10,"output_tokens":1,"cache_read_input_tokens":20,"cache_creation_input_tokens":aggregate,"cache_creation":{"ephemeral_5m_input_tokens":five,"ephemeral_1h_input_tokens":hour}});
            let actual = decode(UpstreamProtocol::Anthropic, &value).unwrap();
            assert_eq!(actual.input_tokens, Some(expected));
            assert_eq!(actual.uncached_input_tokens, Some(10));
            assert_eq!(actual.cache_creation_input_tokens, Some(aggregate));
            assert_eq!(
                actual
                    .cache_creation_details
                    .unwrap()
                    .ephemeral_5m_input_tokens,
                Some(five)
            );
            assert!(usage(UpstreamProtocol::Anthropic, &value).is_err());
        }
        let actual = decode(UpstreamProtocol::Anthropic, &json!({"input_tokens":10,"output_tokens":1,"cache_read_input_tokens":20,"cache_creation_input_tokens":10})).unwrap();
        assert_eq!(actual.input_tokens, Some(40));
        assert_eq!(actual.cache_creation_details, None);
    }

    #[test]
    fn generic_missing_aggregate_is_unknown_even_with_both_ttls() {
        for details in [
            json!({"ephemeral_5m_input_tokens":0,"ephemeral_1h_input_tokens":0}),
            json!({"ephemeral_5m_input_tokens":2,"ephemeral_1h_input_tokens":3}),
            json!({"ephemeral_5m_input_tokens":2}),
            json!({}),
        ] {
            let value = json!({"input_tokens":10,"output_tokens":1,"cache_read_input_tokens":20,"cache_creation":details});
            let actual = decode(UpstreamProtocol::Anthropic, &value).unwrap();
            assert_eq!(actual.cache_creation_input_tokens, None);
            assert_eq!(actual.uncached_input_tokens, Some(10));
            assert_eq!(actual.input_tokens, None);
        }
        let actual = decode(
            UpstreamProtocol::Anthropic,
            &json!({"input_tokens":10,"output_tokens":1,"cache_read_input_tokens":20}),
        )
        .unwrap();
        assert_eq!(actual.cache_creation_input_tokens, None);
        assert_eq!(actual.input_tokens, None);
    }

    #[test]
    fn generic_openai_aliases_are_compared_with_raw_not_effective_total() {
        for (protocol, input, output, details) in [
            (
                UpstreamProtocol::Chat,
                "prompt_tokens",
                "completion_tokens",
                "prompt_tokens_details",
            ),
            (
                UpstreamProtocol::Responses,
                "input_tokens",
                "output_tokens",
                "input_tokens_details",
            ),
        ] {
            let mut value = json!({input:100,output:1,"cache_creation_input_tokens":5,"cache_creation_details":{"ephemeral_5m_input_tokens":6,"ephemeral_1h_input_tokens":4},details:{"cached_creation_tokens":3,"cache_write_tokens":5}});
            let actual = decode(protocol, &value).unwrap();
            assert_eq!(actual.cache_creation_input_tokens, Some(5));
            assert_eq!(
                actual.effective_cache_creation_input_tokens().unwrap(),
                Some(10)
            );
            assert_eq!(actual.input_tokens, Some(100));
            value[details]["cache_write_tokens"] = json!(4);
            assert!(decode(protocol, &value).is_err());
            value
                .as_object_mut()
                .unwrap()
                .remove("cache_creation_input_tokens");
            assert_eq!(
                decode(protocol, &value)
                    .unwrap()
                    .cache_creation_input_tokens,
                Some(4)
            );
            value.as_object_mut().unwrap().remove(details);
            let actual = decode(protocol, &value).unwrap();
            assert_eq!(actual.cache_creation_input_tokens, None);
            assert_eq!(actual.uncached_input_tokens, None);
            assert_eq!(
                actual.effective_cache_creation_input_tokens().unwrap(),
                None
            );
        }
    }

    #[test]
    fn generic_counters_overflow_and_inclusive_cache_overlap_are_rejected() {
        for value in [
            json!({"input_tokens":u64::MAX,"cache_creation_input_tokens":1}),
            json!({"input_tokens":0,"cache_creation_input_tokens":0,"cache_creation":{"ephemeral_5m_input_tokens":u64::MAX,"ephemeral_1h_input_tokens":1}}),
            json!({"input_tokens":u64::MAX,"cache_creation":{"ephemeral_5m_input_tokens":1}}),
            json!({"input_tokens":0,"cache_creation_input_tokens":-1}),
            json!({"input_tokens":0,"cache_creation_input_tokens":1.5}),
            json!({"input_tokens":0,"cache_creation_input_tokens":"1"}),
        ] {
            assert!(
                decode(UpstreamProtocol::Anthropic, &value).is_err(),
                "{value}"
            );
        }
        assert!(decode(UpstreamProtocol::Chat,&json!({"prompt_tokens":26,"prompt_tokens_details":{"cached_tokens":20},"cache_creation_input_tokens":2,"cache_creation_details":{"ephemeral_5m_input_tokens":3,"ephemeral_1h_input_tokens":4}})).is_err());
    }

    #[test]
    fn provider_cannot_opt_into_generic_mode_and_null_is_not_zero() {
        let value = json!({"input_tokens":10,"cache_creation_input_tokens":2,"cache_creation":{"ephemeral_5m_input_tokens":3,"ephemeral_1h_input_tokens":4},"cache_creation_mode":"generic_remainder_v1"});
        assert!(usage(UpstreamProtocol::Anthropic, &value).is_err());
        assert!(decode(UpstreamProtocol::Anthropic, &value).is_ok());
        assert_eq!(
            decode(UpstreamProtocol::Anthropic, &Value::Null).unwrap(),
            Usage::default()
        );
    }

    #[test]
    fn generic_stream_keeps_fixed_input_cache_from_message_start() {
        let mut decoder = Decoder::new_with_cache_creation_mode(
            UpstreamProtocol::Anthropic,
            &Limits::default(),
            Some(CacheCreationMode::GenericRemainderV1),
        );
        let start = json!({"type":"message_start","message":{"id":"msg_generic","usage":{"input_tokens":10,"output_tokens":0,"cache_read_input_tokens":20,"cache_creation_input_tokens":2,"cache_creation":{"ephemeral_5m_input_tokens":3,"ephemeral_1h_input_tokens":4}}}});
        let mut events = Vec::new();
        for value in [
            start,
            json!({"type":"message_delta","delta":{"stop_reason":"end_turn"},"usage":{"output_tokens":1}}),
            json!({"type":"message_stop"}),
        ] {
            events.extend(
                decoder
                    .frame(Frame {
                        event: None,
                        data: value.to_string(),
                    })
                    .unwrap(),
            );
        }
        let mut combined = Usage::default();
        for event in &events {
            if let Event::Usage(value) = event {
                combined.merge(value.clone()).unwrap();
            }
        }
        assert_eq!(combined.input_tokens, Some(37));
        assert_eq!(combined.cache_creation_input_tokens, Some(2));
        assert_eq!(combined.output_tokens, Some(1));
        assert!(events.iter().any(|event| matches!(event, Event::Done)));
    }
}

//! Library IR to bounded Core lifecycle events. Receipts remain Core-owned.
use super::*;
use zconv::ir::{Content, Data, Presence, ReasoningKind, Record, Text};
use zconv::stream::{BlockKind as Kind, EventKind, EventText};

fn owned(value: Text<'_>) -> Result<String, RelayError> {
    match value {
        Text::Utf8(value) => Ok(value.to_owned()),
        Text::Json(value) => serde_json::from_slice(value.raw())
            .map_err(|_| RelayError::Protocol("invalid upstream string")),
    }
}
fn span(payload: &[u8], value: EventText) -> Result<Option<String>, RelayError> {
    if !value.present {
        return Ok(None);
    }
    let raw = payload
        .get(value.offset..value.offset.saturating_add(value.length))
        .ok_or(RelayError::Protocol("invalid upstream event span"))?;
    if value.quoted {
        serde_json::from_slice(raw)
            .map(Some)
            .map_err(|_| RelayError::Protocol("invalid upstream string"))
    } else {
        std::str::from_utf8(raw)
            .map(|v| Some(v.to_owned()))
            .map_err(|_| RelayError::Protocol("invalid upstream string"))
    }
}
fn raw(payload: &[u8], value: EventText) -> Result<Value, RelayError> {
    let value =
        span(payload, value)?.ok_or(RelayError::Protocol("missing upstream event value"))?;
    serde_json::from_str(&value).map_err(|_| RelayError::Protocol("invalid upstream event value"))
}
fn required(payload: &[u8], value: EventText) -> Result<String, RelayError> {
    span(payload, value)?.ok_or(RelayError::Protocol("missing upstream string"))
}
fn count(value: &Value) -> usize {
    match value {
        Value::Array(values) => 1 + values.iter().map(count).sum::<usize>(),
        Value::Object(values) => 1 + values.values().map(|v| 1 + count(v)).sum::<usize>(),
        _ => 1,
    }
}
fn kind(value: Kind, id: Option<String>, name: Option<String>) -> Result<BlockKind, RelayError> {
    Ok(match value {
        Kind::Text => BlockKind::Text,
        Kind::Reasoning => BlockKind::Thinking,
        Kind::ReasoningSummary => BlockKind::ReasoningSummary,
        Kind::Refusal => BlockKind::Refusal,
        Kind::Tool => BlockKind::Tool {
            id: id.ok_or(RelayError::Protocol("missing tool ID"))?,
            name: name.ok_or(RelayError::Protocol("missing tool name"))?,
        },
    })
}
fn finish(value: zconv::stream::FinishReason) -> Result<FinishReason, RelayError> {
    use zconv::stream::FinishReason as F;
    Ok(match value {
        F::Stop => FinishReason::Stop,
        F::Length => FinishReason::Length,
        F::ToolCalls => FinishReason::ToolCalls,
        F::Safety => FinishReason::ContentFilter,
        F::Continuation => FinishReason::Continuation,
        F::Other => return Err(RelayError::Protocol("unsupported upstream finish reason")),
    })
}
impl Decoder {
    fn format(&self) -> zconv::Format {
        let protocol = match self.protocol {
            UpstreamProtocol::Chat => zconv::Protocol::OpenAiChat,
            UpstreamProtocol::Responses => zconv::Protocol::OpenAiResponses,
            UpstreamProtocol::Anthropic => zconv::Protocol::AnthropicMessages,
            UpstreamProtocol::Gemini => zconv::Protocol::GeminiGenerateContent,
        };
        zconv::Format {
            profile: if self.protocol == UpstreamProtocol::Chat {
                zconv::Profile::OpenAiCompatible("lmm")
            } else {
                zconv::Profile::Standard
            },
            ..protocol.into()
        }
    }
    fn key(&self, event: &zconv::stream::Event) -> Key {
        let key = event.key;
        let block = event.block;
        match self.protocol {
            UpstreamProtocol::Chat => match block {
                Kind::Text => (0, 0),
                Kind::Reasoning => (1, 0),
                Kind::ReasoningSummary => (4, 0),
                Kind::Refusal => (2, 0),
                Kind::Tool => (3, u64::from(key.index)),
            },
            UpstreamProtocol::Anthropic => (0, u64::from(key.index)),
            UpstreamProtocol::Responses => (
                u64::from(key.index),
                if block == Kind::Tool {
                    u64::MAX
                } else if block == Kind::ReasoningSummary {
                    // Responses content_index and summary_index are separate
                    // namespaces, and both may be zero on one reasoning item.
                    (1 << 32) | u64::from(key.part)
                } else {
                    u64::from(key.part)
                },
            ),
            UpstreamProtocol::Gemini => {
                let next = (self.blocks.len() as u64, 0);
                if !matches!(event.kind, EventKind::Delta | EventKind::Signature) {
                    return next;
                }
                // Only the leading part can continue the previous frame. An
                // attached signature always refers to the part just decoded.
                if (key.part == 0 || event.kind == EventKind::Signature)
                    && let Some((&last, slot)) = self.blocks.last_key_value()
                    && !slot.ended
                    && matches!(
                        (&slot.kind, block),
                        (BlockKind::Text, Kind::Text)
                            | (BlockKind::ReasoningSummary, Kind::ReasoningSummary)
                    )
                {
                    last
                } else {
                    next
                }
            }
        }
    }
    fn images(&mut self, value: Value, out: &mut Vec<Event>) -> Result<(), RelayError> {
        let Value::Array(images) = value else {
            return Err(RelayError::Protocol("invalid Chat images"));
        };
        for image in &images {
            if image["type"].as_str() != Some("image_url")
                || image["image_url"]["url"]
                    .as_str()
                    .is_none_or(|url| url.is_empty() || url.chars().any(char::is_control))
                || image.get("index").is_some_and(|v| v.as_u64().is_none())
            {
                return Err(RelayError::Protocol("invalid Chat image"));
            }
        }
        if self.finish.is_some() {
            return Err(RelayError::Protocol("images after finish"));
        }
        if images.len()
            > self
                .max_blocks
                .saturating_sub(self.blocks.len() + self.image_count)
        {
            return Err(RelayError::Limit("output block count"));
        }
        self.image_count += images.len();
        if !images.is_empty() {
            out.push(Event::Images(images));
        }
        Ok(())
    }
    fn grounding(&mut self, metadata: Value, out: &mut Vec<Event>) -> Result<(), RelayError> {
        if !metadata.is_object() {
            return Err(RelayError::Protocol("invalid grounding metadata"));
        }
        if let Some(queries) = metadata.get("webSearchQueries") {
            for query in queries
                .as_array()
                .ok_or(RelayError::Protocol("invalid search queries"))?
            {
                text(query)?;
            }
        }
        if let Some(chunks) = metadata.get("groundingChunks") {
            for chunk in chunks
                .as_array()
                .ok_or(RelayError::Protocol("invalid grounding chunks"))?
            {
                if let Some(web) = chunk.get("web") {
                    text(&web["uri"])?;
                }
            }
        }
        // The provider observation is library-decoded. Only billable-search
        // evidence interpretation belongs here, not protocol conversion.
        let queries = metadata
            .get("webSearchQueries")
            .and_then(Value::as_array)
            .map(|queries| {
                queries
                    .iter()
                    .any(|v| v.as_str().is_some_and(|v| !v.trim().is_empty()))
            });
        let web = metadata["groundingChunks"]
            .as_array()
            .is_some_and(|chunks| {
                chunks.iter().any(|chunk| {
                    chunk["web"]["uri"]
                        .as_str()
                        .is_some_and(|uri| !uri.trim().is_empty())
                })
            });
        if let Some(grounded) = queries.or(web.then_some(true)) {
            self.gemini_grounded_requests = Some(
                self.gemini_grounded_requests
                    .unwrap_or(0)
                    .max(u8::from(grounded || web)),
            );
        }
        out.push(Event::GroundingMetadata(metadata));
        Ok(())
    }
    fn receipt(&self, value: &Value, out: &mut Vec<Event>) -> Result<(), RelayError> {
        let receipt = match self.protocol {
            UpstreamProtocol::Chat => value.get("usage").filter(|v| !v.is_null()),
            UpstreamProtocol::Responses => value.get("response").unwrap_or(value).get("usage"),
            UpstreamProtocol::Anthropic => value.get("message").unwrap_or(value).get("usage"),
            UpstreamProtocol::Gemini => value.get("usageMetadata"),
        };
        if let Some(receipt) = receipt {
            let mut usage = self.usage(receipt)?;
            if self.protocol == UpstreamProtocol::Gemini {
                usage.google_search_grounded_requests = self.gemini_grounded_requests;
            }
            if self.protocol == UpstreamProtocol::Anthropic
                && let Some(input) = super::counter(receipt, "input_tokens")?
            {
                out.push(Event::AnthropicInputTokens(input));
            }
            out.push(Event::Usage(usage));
        } else if self.protocol == UpstreamProtocol::Gemini
            && self.gemini_grounded_requests.is_some()
        {
            out.push(Event::Usage(Usage {
                google_search_grounded_requests: self.gemini_grounded_requests,
                ..Usage::default()
            }));
        }
        Ok(())
    }
    fn project_openrouter_stream(&self, content: &mut Value) -> Result<(), RelayError> {
        let object = content.as_object_mut().unwrap();
        // OpenRouter adds provider metadata outside the normalized Chat fields.
        if object
            .remove("provider")
            .is_some_and(|value| !value.is_string() && !value.is_null())
        {
            return Err(RelayError::Protocol("invalid OpenRouter provider metadata"));
        }
        if object
            .remove("openrouter_metadata")
            .is_some_and(|value| !value.is_object() && !value.is_null())
        {
            return Err(RelayError::Protocol("invalid OpenRouter routing metadata"));
        }
        object.retain(|key, value| {
            !matches!(key.as_str(), "error" | "alignment" | "safeguard_results") || !value.is_null()
        });
        let usage = object.get_mut("usage").and_then(Value::as_object_mut);
        let has_usage = usage.is_some();
        if let Some(usage) = usage {
            if usage
                .remove("is_byok")
                .is_some_and(|value| !value.is_boolean() && !value.is_null())
            {
                return Err(RelayError::Protocol("invalid OpenRouter BYOK metadata"));
            }
            if usage
                .remove("server_tool_use_details")
                .is_some_and(|value| {
                    !value.is_null()
                        && !value.as_object().is_some_and(|details| {
                            details.iter().all(|(key, count)| {
                                matches!(
                                    key.as_str(),
                                    "tool_calls_requested"
                                        | "tool_calls_executed"
                                        | "web_search_requests"
                                ) && (count.is_null() || count.as_u64() == Some(0))
                            })
                        })
                })
                || usage
                    .remove("deferred_tools_details")
                    .is_some_and(|value| !value.is_null())
            {
                return Err(RelayError::Unsupported("OpenRouter server tool usage"));
            }
            // OpenRouter may include zero-valued counters for modalities and
            // prediction fields absent from a text response. Nonzero counts
            // still need an explicit usage contract.
            for (group, counters) in [
                (
                    "prompt_tokens_details",
                    &["audio_tokens", "video_tokens"][..],
                ),
                (
                    "completion_tokens_details",
                    &[
                        "audio_tokens",
                        "image_tokens",
                        "accepted_prediction_tokens",
                        "rejected_prediction_tokens",
                    ][..],
                ),
            ] {
                if let Some(details) = usage.get_mut(group).and_then(Value::as_object_mut) {
                    details.retain(|key, value| {
                        !counters.contains(&key.as_str())
                            || !(value.is_null() || value.as_u64() == Some(0))
                    });
                }
            }
        }
        let Some(choices) = object.get_mut("choices").and_then(Value::as_array_mut) else {
            return Ok(());
        };
        let [choice] = choices.as_mut_slice() else {
            return Ok(());
        };
        let Some(choice) = choice.as_object_mut() else {
            return Ok(());
        };
        if choice
            .remove("native_finish_reason")
            .is_some_and(|value| !value.is_string() && !value.is_null())
        {
            return Err(RelayError::Protocol(
                "invalid OpenRouter native finish reason",
            ));
        }
        if choice.get("error").is_some_and(Value::is_null) {
            choice.remove("error");
        }
        if let Some(delta) = choice.get_mut("delta").and_then(Value::as_object_mut) {
            delta.retain(|key, value| match key.as_str() {
                "audio" | "role" | "reasoning" | "reasoning_content" => !value.is_null(),
                "reasoning_details" => {
                    !value.is_null() && !value.as_array().is_some_and(Vec::is_empty)
                }
                _ => true,
            });
        }
        // The documented final usage frame repeats the already observed finish.
        // Treat that exact content-free choice as usage-only. Contradictions,
        // content, tool calls, or extra choice fields still reach validation.
        if has_usage
            && let Some(previous) = self.finish
            && choice.get("index").and_then(Value::as_u64) == Some(0)
            && choice
                .get("finish_reason")
                .and_then(Value::as_str)
                .is_some_and(|reason| {
                    finish_reason(self.protocol, reason, self.tools()).ok() == Some(previous)
                })
            && choice.keys().all(|key| {
                matches!(
                    key.as_str(),
                    "index" | "delta" | "finish_reason" | "logprobs"
                )
            })
            && choice.get("logprobs").is_none_or(Value::is_null)
            && choice
                .get("delta")
                .and_then(Value::as_object)
                .is_some_and(|delta| {
                    delta.iter().all(|(key, value)| {
                        key == "content" && (value.is_null() || value.as_str() == Some(""))
                    })
                })
        {
            choices.clear();
        }
        Ok(())
    }

    pub(super) fn decode_frame(
        &mut self,
        frame: &Frame,
        value: &Value,
        out: &mut Vec<Event>,
    ) -> Result<(), RelayError> {
        let usage_path = if self.protocol == UpstreamProtocol::Responses
            && value.get("response").is_some()
        {
            "/response/usage"
        } else if self.protocol == UpstreamProtocol::Anthropic && value.get("message").is_some() {
            "/message/usage"
        } else {
            "/usage"
        };
        let anthropic_usage = self.protocol == UpstreamProtocol::Anthropic
            && value.pointer(usage_path).is_some_and(Value::is_object);
        let gemini_grade = self.protocol == UpstreamProtocol::Gemini
            && self.expected_speed.is_some()
            && value.pointer("/usageMetadata/serviceTier").is_some();
        let stop_details_parent = match value.get("type").and_then(Value::as_str) {
            Some("message_start") if self.protocol == UpstreamProtocol::Anthropic => "message",
            Some("message_delta") if self.protocol == UpstreamProtocol::Anthropic => "delta",
            _ => "",
        };
        let inactive_stop_details = value[stop_details_parent]
            .get("stop_details")
            .is_some_and(Value::is_null);
        let responses_usage_metadata = self.protocol == UpstreamProtocol::Responses
            && value.pointer(usage_path).is_some_and(|usage| {
                ["attribution", "linapi_cache_billing"]
                    .iter()
                    .any(|key| usage.get(key).is_some_and(Value::is_object))
                    || usage.get("x_details").is_some_and(Value::is_array)
            });
        let legacy_receipt = self.legacy_openrouter_usage && value.get("usage").is_some();
        let cost_details = matches!(
            self.protocol,
            UpstreamProtocol::Chat | UpstreamProtocol::Responses
        ) && value
            .pointer(usage_path)
            .is_some_and(|usage| usage.get("cost_details").is_some());
        let project_openrouter = self.openrouter_stream && value.is_object();
        let payload = if legacy_receipt
            || cost_details
            || project_openrouter
            || anthropic_usage
            || gemini_grade
            || inactive_stop_details
            || responses_usage_metadata
        {
            let mut content = value.clone();
            // admit() checked these provider fields. receipt() still reads the original.
            if anthropic_usage {
                // Core owns validation and settlement for the untouched receipt.
                // The content converter must not interpret provider billing metadata.
                if let Some(usage) = content.pointer_mut(usage_path) {
                    *usage = Value::Null;
                }
            }
            if inactive_stop_details {
                content[stop_details_parent]
                    .as_object_mut()
                    .unwrap()
                    .remove("stop_details");
            }
            if responses_usage_metadata {
                content
                    .pointer_mut(usage_path)
                    .and_then(Value::as_object_mut)
                    .unwrap()
                    .retain(|key, value| {
                        match key.as_str() {
                            "attribution" | "linapi_cache_billing" => !value.is_object(),
                            // A provider audit breakdown repeats counters from the
                            // authoritative top-level receipt. Core bills that receipt.
                            "x_details" => !value.is_array(),
                            _ => true,
                        }
                    });
            }
            if gemini_grade {
                content["usageMetadata"]
                    .as_object_mut()
                    .unwrap()
                    .remove("serviceTier");
            }
            if project_openrouter {
                self.project_openrouter_stream(&mut content)?;
            }
            if legacy_receipt {
                // This owner-selected billing DTO is Core business evidence, not a
                // second content parser or a fallback after library conversion.
                content.as_object_mut().unwrap().remove("usage");
            } else if cost_details {
                // Only usage.cost is the account charge. Supplemental cost_details
                // must not block content conversion or become a billing fallback.
                // receipt() below still reads the untouched authoritative value.
                content
                    .pointer_mut(usage_path)
                    .and_then(Value::as_object_mut)
                    .unwrap()
                    .remove("cost_details");
            }
            serde_json::to_vec(&content)
                .map_err(|_| RelayError::Protocol("invalid upstream receipt"))?
        } else {
            frame.data.as_bytes().to_vec()
        };
        let capacity = count(value).saturating_add(1);
        let mut tokens = vec![zconv::json::Token::EMPTY; capacity];
        let mut events =
            vec![zconv::stream::Event::EMPTY; capacity.saturating_mul(2).saturating_add(2)];
        let used = zconv::stream::decode_frame_format(
            self.format(),
            frame.event.as_deref().unwrap_or("message").as_bytes(),
            &payload,
            &mut tokens,
            &mut events,
            zconv::json::Limits {
                max_input: payload.len(),
                max_tokens: capacity,
                ..Default::default()
            },
        )
        .map_err(|error| {
            self.last_converter_error = Some(error.into());
            match error {
                zconv::Error::LimitExceeded { .. } => RelayError::Limit("output block count"),
                _ => RelayError::Protocol("invalid upstream stream event"),
            }
        })?;
        let mut receipt = false;
        for event in &events[..used] {
            if event.key.choice != 0 {
                return Err(RelayError::Protocol("invalid candidate index"));
            }
            let key = self.key(event);
            if matches!(event.kind, EventKind::ChoiceEnd | EventKind::Finish) && !receipt {
                self.receipt(value, out)?;
                receipt = true;
            }
            match event.kind {
                EventKind::Begin => {
                    let id = span(&payload, event.a)?;
                    self.start(id.as_deref(), out)?;
                }
                EventKind::BlockStart => {
                    let id = span(&payload, event.a)?;
                    let name = span(&payload, event.b)?;
                    if let Some(slot) = self.blocks.get(&key) {
                        if self.protocol != UpstreamProtocol::Chat {
                            return Err(RelayError::Protocol("duplicate content block"));
                        }
                        if let BlockKind::Tool {
                            id: old,
                            name: old_name,
                        } = &slot.kind
                            && (id.as_ref().is_some_and(|v| !v.is_empty() && v != old)
                                || name
                                    .as_ref()
                                    .is_some_and(|v| !v.is_empty() && v != old_name))
                        {
                            return Err(RelayError::Protocol("tool identity changed"));
                        }
                    } else {
                        self.block(key, kind(event.block, id, name)?, out)?;
                        if self.protocol == UpstreamProtocol::Anthropic && event.block == Kind::Tool
                        {
                            self.blocks.get_mut(&key).unwrap().default_object = true;
                        }
                    }
                }
                EventKind::Delta => {
                    if let Some(slot) = self.blocks.get(&key)
                        && !matches!(
                            (&slot.kind, event.block),
                            (BlockKind::Text, Kind::Text)
                                | (BlockKind::Thinking, Kind::Reasoning)
                                | (BlockKind::ReasoningSummary, Kind::ReasoningSummary)
                                | (BlockKind::Refusal, Kind::Refusal)
                                | (BlockKind::Tool { .. }, Kind::Tool)
                        )
                    {
                        return Err(RelayError::Protocol("invalid block transition"));
                    }
                    let text = required(&payload, event.a)?;
                    if matches!(
                        self.protocol,
                        UpstreamProtocol::Chat | UpstreamProtocol::Gemini
                    ) && event.block != Kind::Tool
                    {
                        self.simple(key, kind(event.block, None, None)?, &text, out)?;
                    } else {
                        self.delta(key, &text, out)?;
                    }
                }
                EventKind::Signature => {
                    // Gemini also signs plain text parts; the block must exist
                    // and be open, but need not be a thinking block.
                    self.signature(key, &required(&payload, event.a)?, out)?;
                }
                EventKind::ToolComplete => {
                    let id = span(&payload, event.a)?
                        .unwrap_or_else(|| format!("call_gemini_{}", self.gemini_calls));
                    self.gemini_calls += 1;
                    self.block(
                        key,
                        BlockKind::Tool {
                            id,
                            name: required(&payload, event.b)?,
                        },
                        out,
                    )?;
                    self.delta(key, &required(&payload, event.c)?, out)?;
                    if let Some(signature) = span(&payload, event.signature)? {
                        self.signature(key, &signature, out)?;
                    }
                }
                EventKind::BlockEnd
                    if self.protocol == UpstreamProtocol::Responses
                        && value["type"] == "response.output_item.done" =>
                {
                    let keys: Vec<_> = self
                        .blocks
                        .keys()
                        .filter(|(i, _)| *i == u64::from(event.key.index))
                        .copied()
                        .collect();
                    for key in keys {
                        self.close(key, out)?;
                    }
                }
                EventKind::BlockEnd => self.close(key, out)?,
                EventKind::ChoiceEnd => {
                    let mut reason = finish(event.reason)?;
                    if reason == FinishReason::Stop
                        && self.tools()
                        && matches!(
                            self.protocol,
                            UpstreamProtocol::Gemini | UpstreamProtocol::Responses
                        )
                    {
                        reason = FinishReason::ToolCalls;
                    }
                    self.finish(reason, out)?;
                }
                EventKind::Finish => self.done(out)?,
                EventKind::Images => self.images(raw(&payload, event.a)?, out)?,
                EventKind::Grounding => self.grounding(raw(&payload, event.a)?, out)?,
                EventKind::Continuation => self.continuation(&required(&payload, event.a)?, out)?,
                EventKind::StopSequence => {
                    out.push(Event::StopSequence(required(&payload, event.a)?))
                }
                EventKind::Snapshot
                    if event.block == Kind::Tool
                        && self.blocks.get(&key).is_some_and(|s| !s.has_text) =>
                {
                    self.delta(key, &required(&payload, event.a)?, out)?
                }
                EventKind::Failure => return Err(RelayError::Protocol("upstream stream failed")),
                // These are provider-owned content or receipt fields. Core
                // cannot represent them or authorize their billing semantics;
                // never silently discard them or turn them into local tools.
                EventKind::ProviderBlock | EventKind::ProviderDelta => {
                    return Err(RelayError::Unsupported("provider-owned output content"));
                }
                EventKind::ProviderUsage => {
                    return Err(RelayError::Unsupported("provider-owned usage details"));
                }
                // Core previously ignored benign provider metadata and repeated
                // final snapshots. They are not appended as new content.
                EventKind::Usage
                | EventKind::Extension
                | EventKind::Snapshot
                | EventKind::BlockIncomplete => {}
            }
        }
        if !receipt {
            self.receipt(value, out)?;
        }
        Ok(())
    }
    pub(super) fn decode_response(
        &mut self,
        value: &Value,
        out: &mut Vec<Event>,
    ) -> Result<(), RelayError> {
        // Billing has a richer authoritative representation and is deliberately
        // consumed from the original receipt. Content has one decoder.
        let mut content = value.clone();
        content
            .as_object_mut()
            .unwrap()
            .remove(if self.protocol == UpstreamProtocol::Gemini {
                "usageMetadata"
            } else {
                "usage"
            });
        let payload = serde_json::to_vec(&content)
            .map_err(|_| RelayError::Protocol("invalid upstream response"))?;
        let capacity = count(&content).saturating_add(1);
        let mut tokens = vec![zconv::json::Token::EMPTY; capacity];
        let document = zconv::json::parse(
            &payload,
            &mut tokens,
            zconv::json::Limits {
                max_input: payload.len(),
                max_tokens: capacity,
                ..Default::default()
            },
        )
        .map_err(|_| RelayError::Protocol("invalid upstream response"))?;
        let mut records = vec![Record::EMPTY; capacity];
        let response =
            zconv::protocol::decode_response(self.format(), document.root(), &mut records)
                .map_err(|_| RelayError::Protocol("invalid upstream response"))?;
        let id = response.id.value().map(owned).transpose()?;
        self.start(id.as_deref(), out)?;
        if response
            .records
            .iter()
            .filter(|r| matches!(r, Record::Choice(_)))
            .count()
            != 1
        {
            return Err(RelayError::Protocol(
                "only one upstream candidate is supported",
            ));
        }
        let mut raw_reason = None;
        let mut ordinal = 0u64;
        let mut calls = 0u64;
        let mut reason = None;
        for record in response.records.iter() {
            match record {
                Record::Choice(choice) => {
                    if choice.index != 0 {
                        return Err(RelayError::Protocol("multiple upstream candidates"));
                    }
                    reason = Some(choice.finish);
                    raw_reason = choice.raw_finish.value().map(owned).transpose()?;
                }
                Record::Block(block) => {
                    let (kind, text, signature) = match block.content {
                        Content::Text(text) => (BlockKind::Text, owned(text)?, None),
                        Content::Refusal(text) => (
                            if self.protocol == UpstreamProtocol::Anthropic {
                                BlockKind::Text
                            } else {
                                BlockKind::Refusal
                            },
                            owned(text)?,
                            None,
                        ),
                        Content::Reasoning {
                            kind,
                            text: Presence::Value(text),
                            signature,
                        } => (
                            match kind {
                                ReasoningKind::Thought => BlockKind::Thinking,
                                ReasoningKind::Summary => BlockKind::ReasoningSummary,
                            },
                            owned(text)?,
                            signature.value().map(owned).transpose()?,
                        ),
                        Content::Reasoning {
                            text: Presence::Missing | Presence::Null,
                            signature: Presence::Missing | Presence::Null,
                            ..
                        } if self.protocol == UpstreamProtocol::Responses => continue,
                        Content::Reasoning {
                            text: Presence::Missing | Presence::Null,
                            ..
                        } => return Err(RelayError::Unsupported("provider-owned reasoning")),
                        Content::ToolCall {
                            id,
                            name,
                            arguments,
                            legacy: false,
                        } => {
                            let id = id
                                .value()
                                .map(owned)
                                .transpose()?
                                .or_else(|| {
                                    (self.protocol == UpstreamProtocol::Gemini)
                                        .then(|| format!("call_gemini_{}", self.gemini_calls))
                                })
                                .ok_or(RelayError::Protocol("missing tool ID"))?;
                            let arguments = match arguments {
                                Data::JsonText(text) => owned(text)?,
                                Data::Json(raw) if raw.kind() == zconv::json::Kind::Object => {
                                    let value: Value = serde_json::from_slice(raw.raw())
                                        .map_err(|_| RelayError::Protocol("invalid tool input"))?;
                                    value.to_string()
                                }
                                _ => return Err(RelayError::Protocol("invalid tool input")),
                            };
                            (
                                BlockKind::Tool {
                                    id,
                                    name: owned(name)?,
                                },
                                arguments,
                                None,
                            )
                        }
                        Content::Opaque { .. } => {
                            return Err(RelayError::Unsupported("provider-owned output content"));
                        }
                        _ => {
                            return Err(RelayError::Protocol(
                                "unsupported upstream output content",
                            ));
                        }
                    };
                    let key = match self.protocol {
                        UpstreamProtocol::Chat => match kind {
                            BlockKind::Text => (0, 0),
                            BlockKind::Thinking => (1, 0),
                            BlockKind::ReasoningSummary => (4, 0),
                            BlockKind::Refusal => (2, 0),
                            BlockKind::Tool { .. } => {
                                let key = (3, calls);
                                calls += 1;
                                key
                            }
                        },
                        UpstreamProtocol::Gemini => match kind {
                            // A complete response has ordered parts with independent signatures.
                            BlockKind::Text | BlockKind::ReasoningSummary => (0, ordinal),
                            BlockKind::Tool { .. } => {
                                let key = (2, self.gemini_calls);
                                self.gemini_calls += 1;
                                key
                            }
                            _ => return Err(RelayError::Protocol("unsupported Gemini part")),
                        },
                        UpstreamProtocol::Responses => (ordinal, 0),
                        UpstreamProtocol::Anthropic => (0, ordinal),
                    };
                    self.simple(key, kind, &text, out)?;
                    let signature = block
                        .signature
                        .value()
                        .map(|s| owned(s.value))
                        .transpose()?
                        .or(signature);
                    if let Some(signature) = signature {
                        self.signature(key, &signature, out)?;
                    }
                    ordinal += 1;
                }
                Record::Extension(extension)
                    if extension.key.eq_str("images")
                        && self.protocol == UpstreamProtocol::Chat =>
                {
                    if extension.value.is_null() {
                        continue;
                    }
                    self.images(
                        serde_json::from_slice(extension.value.raw())
                            .map_err(|_| RelayError::Protocol("invalid Chat images"))?,
                        out,
                    )?;
                }
                Record::Extension(extension)
                    if extension.key.eq_str("groundingMetadata")
                        && self.protocol == UpstreamProtocol::Gemini =>
                {
                    self.grounding(
                        serde_json::from_slice(extension.value.raw())
                            .map_err(|_| RelayError::Protocol("invalid grounding metadata"))?,
                        out,
                    )?;
                }
                Record::Extension(extension)
                    if extension.key.eq_str("continuationToken")
                        && self.protocol == UpstreamProtocol::Gemini =>
                {
                    if extension.value.is_null() {
                        continue;
                    }
                    let token = extension
                        .value
                        .as_string()
                        .ok_or(RelayError::Protocol("invalid continuation token"))?;
                    self.continuation(&owned(Text::Json(token))?, out)?;
                }
                Record::Extension(extension)
                    if self.protocol == UpstreamProtocol::Anthropic
                        && extension.key.eq_str("citations")
                        && !extension.value.is_null() =>
                {
                    return Err(RelayError::Unsupported("provider-owned output citations"));
                }
                _ => {}
            }
        }
        if let Some(sequence) = response.stop_sequence.value() {
            out.push(Event::StopSequence(owned(sequence)?));
        }
        self.receipt(value, out)?;
        use zconv::ir::Finish as F;
        let reason = if self.protocol != UpstreamProtocol::Responses {
            finish_reason(
                self.protocol,
                raw_reason.as_deref().ok_or(RelayError::Truncated)?,
                self.tools(),
            )?
        } else {
            match reason.unwrap_or(response.status) {
                F::Stop if self.tools() => FinishReason::ToolCalls,
                F::Stop => FinishReason::Stop,
                F::ToolCalls => FinishReason::ToolCalls,
                F::Length => FinishReason::Length,
                F::Safety | F::Refusal => FinishReason::ContentFilter,
                _ => return Err(RelayError::Protocol("upstream response is not complete")),
            }
        };
        self.finish(reason, out)?;
        self.done(out)
    }
}

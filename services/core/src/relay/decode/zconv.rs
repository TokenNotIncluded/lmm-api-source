//! Library IR to bounded Core lifecycle events. Receipts remain Core-owned.
use super::*;
use zconv::ir::{Content, Data, Presence, Record, Text};
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
    fn key(&self, key: zconv::stream::Key, block: Kind) -> Key {
        match self.protocol {
            UpstreamProtocol::Chat => match block {
                Kind::Text => (0, 0),
                Kind::Reasoning => (1, 0),
                Kind::Refusal => (2, 0),
                Kind::Tool => (3, u64::from(key.index)),
            },
            UpstreamProtocol::Anthropic => (0, u64::from(key.index)),
            UpstreamProtocol::Responses => (
                u64::from(key.index),
                if block == Kind::Tool {
                    u64::MAX
                } else {
                    u64::from(key.part)
                },
            ),
            UpstreamProtocol::Gemini => (u64::from(block == Kind::Reasoning), 0),
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
    pub(super) fn decode_frame(
        &mut self,
        frame: &Frame,
        value: &Value,
        out: &mut Vec<Event>,
    ) -> Result<(), RelayError> {
        let payload = if self.legacy_openrouter_usage && value.get("usage").is_some() {
            // This owner-selected billing DTO is Core business evidence, not a
            // second content parser or a fallback after library conversion.
            let mut legacy = value.clone();
            legacy.as_object_mut().unwrap().remove("usage");
            serde_json::to_vec(&legacy)
                .map_err(|_| RelayError::Protocol("invalid legacy receipt"))?
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
        .map_err(|e| match e {
            zconv::Error::LimitExceeded { .. } => RelayError::Limit("output block count"),
            _ => RelayError::Protocol("invalid upstream stream event"),
        })?;
        let mut receipt = false;
        for event in &events[..used] {
            if event.key.choice != 0 {
                return Err(RelayError::Protocol("invalid candidate index"));
            }
            let key = self.key(event.key, event.block);
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
                    let key = (2, self.gemini_calls);
                    self.gemini_calls += 1;
                    let id = span(&payload, event.a)?
                        .unwrap_or_else(|| format!("call_gemini_{}", key.1));
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
                EventKind::BlockEnd if self.protocol == UpstreamProtocol::Responses => {
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
                            text: Presence::Value(text),
                            signature,
                            ..
                        } => (
                            BlockKind::Thinking,
                            owned(text)?,
                            signature.value().map(owned).transpose()?,
                        ),
                        Content::Reasoning {
                            text: Presence::Missing | Presence::Null,
                            signature: Presence::Missing | Presence::Null,
                            ..
                        } if self.protocol == UpstreamProtocol::Responses => continue,
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
                            BlockKind::Refusal => (2, 0),
                            BlockKind::Tool { .. } => {
                                let key = (3, calls);
                                calls += 1;
                                key
                            }
                        },
                        UpstreamProtocol::Gemini => match kind {
                            BlockKind::Text => (0, 0),
                            BlockKind::Thinking => (1, 0),
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

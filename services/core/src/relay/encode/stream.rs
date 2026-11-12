//! Borrow buffers per zconv call so capacity retries never borrow the owner.
use super::*;
use zconv::{
    json::Token,
    stream::{
        self as ir, ActiveItem, BufferKind, EventEncoderBuffers, EventEncoderState, EventKind,
        EventText, Status, StreamConfig, StreamContext,
    },
};

pub(super) struct Writer {
    state: EventEncoderState,
    written: usize,
    tokens: Vec<Token>,
    items: Vec<ActiveItem>,
    checkpoints: Vec<ActiveItem>,
    aggregate: Vec<u8>,
}

fn text(payload: &mut Vec<u8>, value: &str) -> EventText {
    let offset = payload.len();
    payload.extend_from_slice(value.as_bytes());
    EventText {
        offset,
        length: value.len(),
        quoted: false,
        present: true,
    }
}
fn block_kind(kind: &BlockKind) -> ir::BlockKind {
    match kind {
        BlockKind::Text => ir::BlockKind::Text,
        BlockKind::Thinking => ir::BlockKind::Reasoning,
        BlockKind::Refusal => ir::BlockKind::Refusal,
        BlockKind::Tool { .. } => ir::BlockKind::Tool,
    }
}
fn key(index: usize) -> Result<ir::Key, RelayError> {
    Ok(ir::Key {
        choice: 0,
        index: index
            .try_into()
            .map_err(|_| RelayError::Limit("output block count"))?,
        part: 0,
    })
}
fn event(kind: EventKind) -> ir::Event {
    ir::Event {
        kind,
        ..ir::Event::EMPTY
    }
}
pub(super) fn grow<T: Clone>(
    buffer: &mut Vec<T>,
    needed: usize,
    limit: usize,
    empty: T,
) -> Result<(), RelayError> {
    let length = needed
        .max(buffer.len().saturating_mul(2))
        .max(64)
        .min(limit);
    if length < needed || length <= buffer.len() {
        return Err(RelayError::Limit("response encoder workspace"));
    }
    buffer.resize(length, empty);
    Ok(())
}
impl Writer {
    pub(super) fn new(encoder: &Encoder) -> Result<Self, RelayError> {
        let target = format(encoder.client);
        let mut config = StreamConfig::new(target.protocol, target.protocol);
        config.source = target;
        config.target = target;
        config.allow_partial_usage = true;
        config.json_limits.max_input = encoder.wire_limit();
        Ok(Self {
            state: EventEncoderState::new(config).map_err(library_error)?,
            written: 0,
            tokens: vec![Token::EMPTY; 64],
            items: Vec::new(),
            checkpoints: Vec::new(),
            aggregate: Vec::new(),
        })
    }
    fn stage(
        &mut self,
        encoder: &Encoder,
        payload: &[u8],
        events: &[ir::Event],
        preview: bool,
    ) -> Result<Vec<Vec<u8>>, RelayError> {
        let context = StreamContext {
            response_id: Some(&encoder.id),
            model: Some(&encoder.model),
            created: Some(encoder.created),
            item_id_prefix: Some(&encoder.id),
            ..StreamContext::default()
        };
        let max = encoder.wire_limit();
        let mut output = vec![0; 4096.min(max)];
        loop {
            let buffers = EventEncoderBuffers {
                tokens: &mut self.tokens,
                items: &mut self.items,
                checkpoints: &mut self.checkpoints,
                aggregate: &mut self.aggregate,
            };
            let result = if preview {
                self.state
                    .preview(context, payload, events, buffers, &mut output)
            } else {
                self.state
                    .encode(context, payload, events, buffers, &mut output)
            };
            match result {
                Ok(written) => {
                    let limit = if matches!(
                        encoder.client,
                        ClientProtocol::Anthropic | ClientProtocol::Gemini
                    ) {
                        max
                    } else {
                        usize::MAX
                    };
                    if preview {
                        if self
                            .written
                            .checked_add(written)
                            .is_none_or(|total| total > limit)
                        {
                            return Err(RelayError::Limit("encoded output"));
                        }
                    } else {
                        self.account_output(written, limit)?;
                    }
                    output.truncate(written);
                    return Ok(if output.is_empty() {
                        Vec::new()
                    } else {
                        vec![output]
                    });
                }
                Err(Status::NeedOutput) => {
                    let needed = output.len().saturating_add(1);
                    grow(&mut output, needed, max, 0)?;
                }
                Err(Status::NeedBuffer { buffer, needed, .. }) => match buffer {
                    BufferKind::Tokens => grow(&mut self.tokens, needed, max, Token::EMPTY)?,
                    BufferKind::Items => {
                        grow(
                            &mut self.items,
                            needed,
                            encoder.limits.max_blocks.saturating_add(1),
                            ActiveItem::EMPTY,
                        )?;
                        self.checkpoints.resize(self.items.len(), ActiveItem::EMPTY);
                    }
                    BufferKind::Aggregate => grow(
                        &mut self.aggregate,
                        needed,
                        encoder
                            .limits
                            .output_bytes
                            .saturating_mul(32)
                            .saturating_add(encoder.limits.max_blocks.saturating_mul(4096)),
                        0,
                    )?,
                    _ => return Err(RelayError::Protocol("unexpected response buffer")),
                },
                Err(Status::Failed(error)) => return Err(library_error(error)),
                _ => return Err(RelayError::Protocol("unexpected response encoder status")),
            }
        }
    }
    #[cfg(test)]
    pub(super) fn is_finished(&self) -> bool {
        self.state.is_finished()
    }
    pub(super) fn account_output(&mut self, bytes: usize, limit: usize) -> Result<(), RelayError> {
        self.written = self
            .written
            .checked_add(bytes)
            .ok_or(RelayError::Limit("encoded output"))?;
        if self.written > limit {
            return Err(RelayError::Limit("encoded output"));
        }
        Ok(())
    }
    pub(super) fn consume(
        &mut self,
        encoder: &Encoder,
        input: &Event,
    ) -> Result<Vec<Vec<u8>>, RelayError> {
        let mut payload = Vec::new();
        let mut e = ir::Event::EMPTY;
        match input {
            Event::Start(_) => e.kind = EventKind::Begin,
            Event::Block { index, kind } => {
                if encoder.client == ClientProtocol::Gemini && !matches!(kind, BlockKind::Text) {
                    return Ok(Vec::new());
                }
                e.kind = EventKind::BlockStart;
                e.key = key(*index)?;
                e.block = block_kind(kind);
                if encoder.client == ClientProtocol::Responses {
                    e.item_id = text(&mut payload, &encoder.block_id(*index));
                }
                if let BlockKind::Tool { id, name } = kind {
                    e.a = text(&mut payload, id);
                    e.b = text(&mut payload, name);
                }
            }
            Event::Delta { index, text: value } => {
                let kind = &encoder.blocks[*index].kind;
                if value.is_empty()
                    || (encoder.client == ClientProtocol::Gemini
                        && !matches!(kind, BlockKind::Text))
                {
                    return Ok(Vec::new());
                }
                e.kind = EventKind::Delta;
                e.key = key(*index)?;
                e.block = block_kind(kind);
                e.a = text(&mut payload, value);
            }
            Event::Signature {
                index,
                provider,
                value,
            } => {
                if encoder.client == ClientProtocol::Gemini {
                    return Ok(Vec::new());
                }
                e.kind = EventKind::Signature;
                e.key = key(*index)?;
                e.a = text(&mut payload, value);
                e.b = text(
                    &mut payload,
                    match provider {
                        UpstreamProtocol::Anthropic => "anthropic",
                        UpstreamProtocol::Gemini => "gemini",
                        UpstreamProtocol::Chat => "chat",
                        UpstreamProtocol::Responses => "responses",
                    },
                );
            }
            Event::Images(images) => {
                e.kind = EventKind::Images;
                let raw = serde_json::to_string(images)
                    .map_err(|_| RelayError::Protocol("invalid Chat images"))?;
                e.a = text(&mut payload, &raw);
            }
            // Ends and native final metadata wait for terminal validation. In
            // particular, closing a truncated tool before ChoiceEnd would ask
            // zconv to validate a partial JSON object as a successful tool.
            Event::EndBlock(_)
            | Event::Usage(_)
            | Event::Finish(_)
            | Event::Done
            | Event::GroundingMetadata(_)
            | Event::ContinuationToken(_)
            | Event::StopSequence(_) => return Ok(Vec::new()),
        }
        self.stage(encoder, &payload, &[e], false)
    }
    pub(super) fn terminal(&mut self, encoder: &Encoder) -> Result<Vec<Vec<u8>>, RelayError> {
        let mut events = Vec::new();
        let mut payload = Vec::new();
        for (index, block) in encoder.blocks.iter().enumerate() {
            if encoder.client == ClientProtocol::Gemini && !matches!(block.kind, BlockKind::Text) {
                if let BlockKind::Tool { id, name } = &block.kind {
                    let mut e = event(EventKind::ToolComplete);
                    e.key = key(index)?;
                    e.a = text(&mut payload, id);
                    e.b = text(&mut payload, name);
                    e.c = text(&mut payload, &block.text);
                    if let Some((_, value)) = &block.signature {
                        e.signature = text(&mut payload, value);
                    }
                    events.push(e);
                } else {
                    let mut e = event(EventKind::BlockStart);
                    e.key = key(index)?;
                    e.block = block_kind(&block.kind);
                    events.push(e);
                    e.kind = EventKind::Delta;
                    e.a = text(&mut payload, &block.text);
                    events.push(e);
                    if let Some((_, value)) = &block.signature {
                        e.kind = EventKind::Signature;
                        e.a = text(&mut payload, value);
                        events.push(e);
                    }
                }
            }
        }
        for (kind, value) in [
            (
                EventKind::Grounding,
                encoder
                    .grounding
                    .as_ref()
                    .map(serde_json::to_string)
                    .transpose()
                    .map_err(|_| RelayError::Protocol("invalid grounding metadata"))?,
            ),
            (EventKind::Continuation, encoder.continuation.clone()),
            (EventKind::StopSequence, encoder.stop_sequence.clone()),
        ] {
            if let Some(value) = value {
                let mut e = event(kind);
                e.a = text(&mut payload, &value);
                events.push(e);
            }
        }
        if encoder.client != ClientProtocol::Chat || encoder.include_usage {
            let mut e = event(EventKind::Usage);
            e.usage = encoder.stream_usage()?;
            events.push(e);
        }
        let mut choice = event(EventKind::ChoiceEnd);
        choice.reason = match encoder.finish.ok_or(RelayError::Truncated)? {
            FinishReason::Stop => ir::FinishReason::Stop,
            FinishReason::ToolCalls => ir::FinishReason::ToolCalls,
            FinishReason::Length => ir::FinishReason::Length,
            FinishReason::Continuation if encoder.client == ClientProtocol::Gemini => {
                ir::FinishReason::Continuation
            }
            FinishReason::Continuation => ir::FinishReason::Length,
            FinishReason::ContentFilter => ir::FinishReason::Safety,
        };
        // ChoiceEnd closes all blocks with the final truncation status known.
        events.push(choice);
        events.push(event(EventKind::Finish));
        self.stage(encoder, &payload, &events, true)
    }
    pub(super) fn error(
        &mut self,
        encoder: &Encoder,
        error: &RelayError,
    ) -> Result<Vec<u8>, RelayError> {
        let mut payload = Vec::new();
        let mut e = event(EventKind::Failure);
        e.a = text(&mut payload, error.code());
        e.b = text(&mut payload, &error.to_string());
        Ok(self.stage(encoder, &payload, &[e], false)?.concat())
    }
}

impl Encoder {
    fn stream_usage(&self) -> Result<ir::Usage, RelayError> {
        let usage = &self.usage;
        let mut wire = ir::Usage {
            input: usage.input_tokens,
            uncached_input: usage.uncached_input_tokens,
            output: usage.output_tokens,
            cached: usage.cached_input_tokens,
            reasoning: usage.reasoning_output_tokens,
            cache_write: usage.cache_creation_input_tokens,
            anthropic_cache_details: usage.cache_creation_details.is_some(),
            openai_cache_details: usage.openai_cache_write_details.is_some(),
            ..ir::Usage::default()
        };
        if let Some(details) = usage.cache_creation_details {
            wire.cache_write_5m = details.ephemeral_5m_input_tokens;
            wire.cache_write_1h = details.ephemeral_1h_input_tokens;
        }
        if let Some(details) = usage.openai_cache_write_details {
            wire.cache_write_30m = details.ttl_30m_tokens;
            wire.cache_write_12h = details.ttl_12h_tokens;
        }
        if self.client == ClientProtocol::Anthropic {
            self.native_usage()?;
            // Native Anthropic has no separate reasoning counter.
            wire.reasoning = None;
        }
        if self.client == ClientProtocol::Gemini {
            self.native_usage()?;
            // Native Gemini has no cache-write field; known zero is harmless.
            wire.cache_write = None;
            wire.cache_write_5m = None;
            wire.cache_write_1h = None;
            wire.cache_write_30m = None;
            wire.cache_write_12h = None;
            wire.anthropic_cache_details = false;
            wire.openai_cache_details = false;
            // Missing thoughts are unknown, not zero visible tokens. The Core
            // receipt is untouched; only omit the unprovable wire counter.
            if usage.reasoning_output_tokens.is_none() {
                wire.output = None;
            }
            wire.tool_use_prompt = usage.tool_use_prompt_tokens;
            if usage.google_search_grounded_requests.is_none()
                || usage.tool_use_prompt_tokens.is_some()
            {
                wire.total = usage
                    .input_tokens
                    .zip(usage.output_tokens)
                    .and_then(|(i, o)| i.checked_add(o))
                    .and_then(|n| n.checked_add(usage.tool_use_prompt_tokens.unwrap_or(0)));
            }
        } else {
            wire.total = usage
                .input_tokens
                .zip(usage.output_tokens)
                .and_then(|(i, o)| i.checked_add(o));
        }
        Ok(wire)
    }
}

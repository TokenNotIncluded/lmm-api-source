//! zconv response writers; Core owns lifecycle, limits and settlement authority.
use super::{
    BlockKind, ClientProtocol, Event, FinishReason, Limits, RelayError, Request, UpstreamProtocol,
    Usage,
};
use serde_json::{Value, json};
use zconv::{Format, Profile, Protocol};

mod complete;
mod stream;

struct Block {
    kind: BlockKind,
    text: String,
    signature: Option<(UpstreamProtocol, String)>,
    ended: bool,
}

pub(crate) struct Encoder {
    client: ClientProtocol,
    stream: bool,
    include_usage: bool,
    id: String,
    model: String,
    created: u64,
    writer: Option<stream::Writer>,
    started: bool,
    done: bool,
    emitted: bool,
    blocks: Vec<Block>,
    images: Vec<Value>,
    grounding: Option<Value>,
    continuation: Option<String>,
    stop_sequence: Option<String>,
    usage: Usage,
    finish: Option<FinishReason>,
    bytes: usize,
    limits: Limits,
}

fn format(client: ClientProtocol) -> Format {
    let protocol = match client {
        ClientProtocol::Chat => Protocol::OpenAiChat,
        ClientProtocol::Responses => Protocol::OpenAiResponses,
        ClientProtocol::Anthropic => Protocol::AnthropicMessages,
        ClientProtocol::Gemini => Protocol::GeminiGenerateContent,
    };
    let mut format = Format::new(protocol);
    if matches!(client, ClientProtocol::Chat | ClientProtocol::Responses) {
        format.profile = Profile::OpenAiCompatible("lmm");
    }
    format
}

fn library_error(error: zconv::Error) -> RelayError {
    match error {
        zconv::Error::Unsupported { feature } | zconv::Error::PolicyViolation { feature } => {
            RelayError::Unsupported(feature)
        }
        zconv::Error::InvalidInput { reason, .. } => RelayError::Protocol(reason),
        zconv::Error::LimitExceeded { kind, .. } => RelayError::Limit(kind),
        _ => RelayError::Protocol("response encoding failed"),
    }
}

impl Encoder {
    pub fn new(request: &Request, request_id: &str, limits: &Limits) -> Self {
        let prefix = match request.client {
            ClientProtocol::Chat => "chatcmpl",
            ClientProtocol::Responses => "resp",
            ClientProtocol::Anthropic => "msg",
            ClientProtocol::Gemini => "gemini",
        };
        Self {
            client: request.client,
            stream: request.stream,
            include_usage: request.include_usage,
            id: format!("{prefix}_{request_id}"),
            model: request.model.clone(),
            created: std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap_or_default()
                .as_secs(),
            writer: None,
            started: false,
            done: false,
            emitted: false,
            blocks: Vec::new(),
            images: Vec::new(),
            grounding: None,
            continuation: None,
            stop_sequence: None,
            usage: Usage::default(),
            finish: None,
            bytes: 0,
            limits: limits.clone(),
        }
    }
    pub fn usage(&self) -> Usage {
        self.usage.clone()
    }
    pub fn finish_reason(&self) -> Option<FinishReason> {
        self.finish
    }
    fn block_id(&self, index: usize) -> String {
        let prefix = match self.blocks[index].kind {
            BlockKind::Tool { .. } => "fc",
            BlockKind::Thinking => "rs",
            _ => "msg",
        };
        format!("{prefix}_{}_{index}", self.id)
    }
    fn account(&mut self, bytes: usize) -> Result<(), RelayError> {
        self.bytes = self
            .bytes
            .checked_add(bytes)
            .ok_or(RelayError::Limit("aggregate output"))?;
        if self.bytes > self.limits.output_bytes {
            return Err(RelayError::Limit("aggregate output"));
        }
        Ok(())
    }
    fn wire_limit(&self) -> usize {
        if matches!(
            self.client,
            ClientProtocol::Anthropic | ClientProtocol::Gemini
        ) {
            return self.limits.output_bytes;
        }
        self.limits
            .output_bytes
            .saturating_mul(8)
            .saturating_add(self.limits.max_blocks.saturating_mul(4096))
    }
    pub fn consume(&mut self, event: Event) -> Result<Vec<Vec<u8>>, RelayError> {
        if self.done {
            return Err(RelayError::Protocol("event after done"));
        }
        if !self.started && !matches!(event, Event::Start(_)) {
            return Err(RelayError::Protocol("event before start"));
        }
        match &event {
            Event::Start(id) => {
                if self.started {
                    return Err(RelayError::Protocol("duplicate response start"));
                }
                if matches!(
                    self.client,
                    ClientProtocol::Chat | ClientProtocol::Responses
                ) {
                    self.account(id.len())?;
                }
                self.started = true;
            }
            Event::Block { index, kind } => {
                if self.finish.is_some()
                    || *index != self.blocks.len()
                    || index + self.images.len() >= self.limits.max_blocks
                {
                    return Err(RelayError::Protocol("invalid output block index"));
                }
                if self.client == ClientProtocol::Anthropic && self.blocks.iter().any(|b| !b.ended)
                {
                    return Err(RelayError::Protocol("invalid block order"));
                }
                if matches!(
                    self.client,
                    ClientProtocol::Anthropic | ClientProtocol::Gemini
                ) && matches!(kind, BlockKind::Refusal)
                {
                    return Err(RelayError::Unsupported("native refusal block"));
                }
                if let BlockKind::Tool { id, name } = kind {
                    if id.is_empty()
                        || name.is_empty()
                        || id.chars().chain(name.chars()).any(char::is_control)
                    {
                        return Err(RelayError::Protocol("invalid tool header"));
                    }
                    if self.client == ClientProtocol::Anthropic
                        && [id, name].iter().any(|s| {
                            s.len() > 128
                                || !s
                                    .bytes()
                                    .all(|b| b.is_ascii_alphanumeric() || b"-_".contains(&b))
                        })
                    {
                        return Err(RelayError::Protocol("invalid tool identity"));
                    }
                    if self
                        .blocks
                        .iter()
                        .any(|b| matches!(&b.kind, BlockKind::Tool { id: old, .. } if old == id))
                    {
                        return Err(RelayError::Protocol("duplicate tool ID"));
                    }
                    self.account(id.len().saturating_add(name.len()))?;
                }
                self.blocks.push(Block {
                    kind: kind.clone(),
                    text: String::new(),
                    signature: None,
                    ended: false,
                });
            }
            Event::Delta { index, text } => {
                self.account(text.len())?;
                let b = self
                    .blocks
                    .get_mut(*index)
                    .ok_or(RelayError::Protocol("unknown output block"))?;
                if b.ended
                    || self.finish.is_some()
                    || (self.client == ClientProtocol::Anthropic && b.signature.is_some())
                {
                    return Err(RelayError::Protocol("output after block end"));
                }
                if matches!(b.kind, BlockKind::Tool { .. })
                    && b.text.len().saturating_add(text.len()) > self.limits.tool_argument_bytes
                {
                    return Err(RelayError::Limit("tool arguments"));
                }
                b.text.push_str(text);
            }
            Event::Signature {
                index,
                provider,
                value,
            } => {
                self.account(value.len())?;
                let b = self
                    .blocks
                    .get_mut(*index)
                    .ok_or(RelayError::Protocol("unknown signature block"))?;
                if b.ended || value.is_empty() {
                    return Err(RelayError::Protocol("signature after output end"));
                }
                if self.client == ClientProtocol::Anthropic
                    && (*provider != UpstreamProtocol::Anthropic
                        || !matches!(b.kind, BlockKind::Thinking))
                {
                    return Err(RelayError::Unsupported("foreign thinking signature"));
                }
                if self.client == ClientProtocol::Gemini && *provider != UpstreamProtocol::Gemini {
                    return Err(RelayError::Unsupported("foreign Gemini signature"));
                }
                if self.stream
                    && self.client == ClientProtocol::Gemini
                    && matches!(b.kind, BlockKind::Text)
                {
                    return Err(RelayError::Unsupported(
                        "streaming ordinary text signatures cannot be preserved",
                    ));
                }
                match &mut b.signature {
                    Some((p, s)) if *p == *provider && *provider == UpstreamProtocol::Anthropic => {
                        s.push_str(value)
                    }
                    Some((p, s))
                        if *p == *provider
                            && s == value
                            && self.client != ClientProtocol::Gemini => {}
                    None => b.signature = Some((*provider, value.clone())),
                    _ => return Err(RelayError::Protocol("conflicting provider signature")),
                }
            }
            Event::EndBlock(index) => {
                let b = self
                    .blocks
                    .get_mut(*index)
                    .ok_or(RelayError::Protocol("unknown output end"))?;
                if b.ended {
                    return Err(RelayError::Protocol("duplicate block end"));
                }
                if self.client == ClientProtocol::Anthropic
                    && matches!(b.kind, BlockKind::Thinking)
                    && b.signature.is_none()
                {
                    return Err(RelayError::Unsupported("unsigned native thinking"));
                }
                b.ended = true;
            }
            Event::Usage(usage) => {
                self.usage.merge(usage.clone())?;
                if self
                    .usage
                    .output_tokens
                    .is_some_and(|n| n > u64::from(self.limits.max_output_tokens))
                {
                    return Err(RelayError::Limit("output tokens"));
                }
            }
            Event::Images(images) => {
                if self.client != ClientProtocol::Chat {
                    return Err(RelayError::Unsupported(
                        "Chat image output requires a Chat client",
                    ));
                }
                if self.finish.is_some() {
                    return Err(RelayError::Protocol("invalid Chat image transition"));
                }
                if images.len()
                    > self
                        .limits
                        .max_blocks
                        .saturating_sub(self.blocks.len() + self.images.len())
                {
                    return Err(RelayError::Limit("output block count"));
                }
                self.account(
                    serde_json::to_vec(images)
                        .map_err(|_| RelayError::Protocol("invalid Chat images"))?
                        .len(),
                )?;
                self.images.extend(images.iter().cloned());
            }
            Event::GroundingMetadata(metadata) => {
                if self.client != ClientProtocol::Gemini {
                    return Err(RelayError::Unsupported(
                        "grounding metadata requires native Gemini",
                    ));
                }
                let n = serde_json::to_vec(metadata)
                    .map_err(|_| RelayError::Protocol("invalid grounding metadata"))?
                    .len();
                if n > self.limits.event_bytes {
                    return Err(RelayError::Limit("grounding metadata"));
                }
                // Only the latest grounding snapshot is emitted. Its event
                // bound and the native encoded-output cap remain authoritative.
                self.grounding = Some(metadata.clone());
            }
            Event::ContinuationToken(token) => {
                if token.is_empty()
                    || token.len() > self.limits.event_bytes
                    || token.chars().any(char::is_control)
                {
                    return Err(RelayError::Protocol("invalid Gemini continuation token"));
                }
                if self.client == ClientProtocol::Gemini {
                    match &self.continuation {
                        Some(old) if old != token => {
                            return Err(RelayError::Protocol(
                                "conflicting Gemini continuation token",
                            ));
                        }
                        Some(_) => {}
                        None => {
                            self.account(token.len())?;
                            self.continuation = Some(token.clone());
                        }
                    }
                }
            }
            Event::StopSequence(sequence) => {
                if self.finish.is_some()
                    || sequence.is_empty()
                    || sequence.len() > 1024
                    || (self.client == ClientProtocol::Anthropic && self.stop_sequence.is_some())
                {
                    return Err(RelayError::Protocol("invalid stop sequence receipt"));
                }
                self.account(sequence.len())?;
                if self.client == ClientProtocol::Anthropic {
                    self.stop_sequence = Some(sequence.clone());
                }
            }
            Event::Finish(reason) => {
                if self.finish.is_some() {
                    return Err(RelayError::Protocol("duplicate finish reason"));
                }
                if self.client == ClientProtocol::Anthropic
                    && (*reason == FinishReason::ContentFilter
                        || self.blocks.iter().any(|b| !b.ended))
                {
                    return Err(RelayError::Unsupported(
                        "native finish reason or block order",
                    ));
                }
                if self.stop_sequence.is_some() && *reason != FinishReason::Stop {
                    return Err(RelayError::Protocol("stop sequence requires stop finish"));
                }
                self.finish = Some(*reason);
            }
            Event::Done => {
                if self.finish.is_none() || self.blocks.iter().any(|b| !b.ended) {
                    return Err(RelayError::Truncated);
                }
                self.done = true;
            }
        }
        if !self.stream {
            return Ok(Vec::new());
        }
        if self.writer.is_none() {
            self.writer = Some(stream::Writer::new(self)?);
        }
        let mut writer = self
            .writer
            .take()
            .ok_or(RelayError::Protocol("stream writer missing"))?;
        let result = writer.consume(self, &event);
        self.writer = Some(writer);
        result
    }
    fn validate_complete(&self) -> Result<(), RelayError> {
        if !self.done
            || !self.started
            || self.finish.is_none()
            || self.blocks.iter().any(|b| !b.ended)
        {
            return Err(RelayError::Truncated);
        }
        if self.finish == Some(FinishReason::ToolCalls)
            && !self
                .blocks
                .iter()
                .any(|b| matches!(b.kind, BlockKind::Tool { .. }))
        {
            return Err(RelayError::Protocol("tool finish without tools"));
        }
        self.usage.validate()?;
        for b in &self.blocks {
            if matches!(b.kind, BlockKind::Tool { .. })
                && (matches!(
                    self.client,
                    ClientProtocol::Anthropic | ClientProtocol::Gemini
                ) || !self.finish.is_some_and(FinishReason::truncates_output))
            {
                // zconv validates syntax/duplicate keys too, including stream
                // aggregates. Core additionally requires the object contract.
                let value: Value = serde_json::from_str(&b.text)
                    .map_err(|_| RelayError::Protocol("incomplete tool arguments"))?;
                if !value.is_object() {
                    return Err(RelayError::Protocol("tool arguments are not an object"));
                }
            }
        }
        Ok(())
    }
    /// Construct terminal bytes before the caller's settlement/checkpoint gate;
    /// only the caller may publish them after that gate succeeds.
    pub fn terminal(&mut self) -> Result<Vec<Vec<u8>>, RelayError> {
        if self.emitted {
            return Err(RelayError::Protocol("duplicate terminal boundary"));
        }
        self.validate_complete()?;
        let packets = if self.stream {
            let mut writer = self.writer.take().ok_or(RelayError::Truncated)?;
            let result = writer.terminal(self);
            self.writer = Some(writer);
            result?
        } else {
            vec![self.complete()?]
        };
        self.emitted = true;
        Ok(packets)
    }
    pub fn error_packet(&mut self, error: &RelayError) -> Result<Vec<u8>, RelayError> {
        if self.stream && self.client == ClientProtocol::Responses {
            if self.writer.is_none() {
                self.writer = Some(stream::Writer::new(self)?);
            }
            let mut writer = self
                .writer
                .take()
                .ok_or(RelayError::Protocol("stream writer missing"))?;
            let result = writer.error(self, error);
            self.writer = Some(writer);
            return result;
        }
        // Error status/code mappings belong to Core. zconv handles JSON escaping
        // and bounded writing, without requiring a fabricated successful Begin.
        let message = error.to_string();
        let mut output = vec![
            0;
            message
                .len()
                .saturating_mul(6)
                .saturating_add(512)
                .min(self.wire_limit())
        ];
        let mut wire = zconv::json::Writer::new(&mut output);
        if self.stream {
            if self.client == ClientProtocol::Anthropic {
                wire.raw(b"event: error\n");
            }
            wire.raw(b"data: ");
        }
        match self.client {
            ClientProtocol::Anthropic => {
                let kind = match error.status() {
                    401 => "authentication_error",
                    403 => "permission_error",
                    404 => "not_found_error",
                    413 => "request_too_large",
                    429 => "rate_limit_error",
                    400 => "invalid_request_error",
                    _ => "api_error",
                };
                wire.raw(b"{\"type\":\"error\",\"error\":{\"type\":");
                wire.string(kind.as_bytes());
                wire.raw(b",\"message\":");
            }
            ClientProtocol::Gemini => {
                let status = match error.status() {
                    400 => "INVALID_ARGUMENT",
                    401 => "UNAUTHENTICATED",
                    402 | 403 => "PERMISSION_DENIED",
                    409 => "ABORTED",
                    413 | 429 => "RESOURCE_EXHAUSTED",
                    499 => "CANCELLED",
                    503 => "UNAVAILABLE",
                    504 => "DEADLINE_EXCEEDED",
                    _ => "INTERNAL",
                };
                wire.raw(b"{\"error\":{\"code\":");
                wire.raw(error.status().to_string().as_bytes());
                wire.raw(b",\"status\":");
                wire.string(status.as_bytes());
                wire.raw(b",\"message\":");
            }
            ClientProtocol::Chat | ClientProtocol::Responses => {
                wire.raw(b"{\"error\":{\"type\":");
                wire.string(error.code().as_bytes());
                wire.raw(b",\"code\":");
                wire.string(error.code().as_bytes());
                wire.raw(b",\"message\":");
            }
        }
        wire.string(message.as_bytes());
        wire.raw(b"}}");
        if self.stream {
            wire.raw(b"\n\n");
        }
        let written = wire.finish().map_err(library_error)?;
        if matches!(
            self.client,
            ClientProtocol::Anthropic | ClientProtocol::Gemini
        ) && let Some(writer) = &mut self.writer
        {
            writer.account_output(written, self.limits.output_bytes)?;
        }
        output.truncate(written);
        Ok(output)
    }
}
#[cfg(test)]
mod cache_creation_tests {
    use super::*;

    #[test]
    fn complete_chat_encodes_split_reasoning_refusal_and_partial_tool_arguments() {
        let limits = Limits::default();
        let request = Request::parse(
            ClientProtocol::Chat,
            br#"{"model":"m","messages":[{"role":"user","content":"hello"}]}"#,
            &limits,
        )
        .unwrap();
        let mut encoder = Encoder::new(&request, "zconv", &limits);
        encoder.consume(Event::Start("up".into())).unwrap();
        for (index, (kind, text)) in [
            (BlockKind::Thinking, "think"),
            (BlockKind::Text, "hello"),
            (BlockKind::Thinking, " more"),
            (BlockKind::Refusal, "no"),
            (BlockKind::Refusal, " thanks"),
            (
                BlockKind::Tool {
                    id: "call".into(),
                    name: "f".into(),
                },
                "{\"x\":",
            ),
            (BlockKind::Text, " world"),
        ]
        .into_iter()
        .enumerate()
        {
            encoder.consume(Event::Block { index, kind }).unwrap();
            encoder
                .consume(Event::Delta {
                    index,
                    text: text.into(),
                })
                .unwrap();
            encoder.consume(Event::EndBlock(index)).unwrap();
        }
        encoder
            .consume(Event::Finish(FinishReason::Length))
            .unwrap();
        assert!(encoder.consume(Event::Done).unwrap().is_empty());
        let value: Value = serde_json::from_slice(&encoder.terminal().unwrap().concat()).unwrap();
        let message = &value["choices"][0]["message"];
        assert_eq!(message["content"], "hello world");
        assert_eq!(message["reasoning_content"], "think more");
        assert_eq!(message["refusal"], "no thanks");
        assert_eq!(message["tool_calls"][0]["function"]["arguments"], "{\"x\":");
        assert_eq!(value["choices"][0]["finish_reason"], "length");
    }

    #[test]
    fn openai_terminals_keep_the_native_thirty_minute_and_twelve_hour_details() {
        for client in [ClientProtocol::Chat, ClientProtocol::Responses] {
            for stream in [false, true] {
                let mut body = match client {
                    ClientProtocol::Chat => json!({
                        "model":"m","messages":[{"role":"user","content":"hello"}]
                    }),
                    ClientProtocol::Responses => json!({"model":"m","input":"hello"}),
                    _ => unreachable!(),
                };
                body["stream"] = json!(stream);
                if client == ClientProtocol::Chat && stream {
                    body["stream_options"] = json!({"include_usage":true});
                }
                let limits = Limits::default();
                let request =
                    Request::parse(client, &serde_json::to_vec(&body).unwrap(), &limits).unwrap();
                let mut encoder = Encoder::new(&request, "cache-write-test", &limits);
                for event in [
                    Event::Start("up".into()),
                    Event::Usage(Usage {
                        input_tokens: Some(10),
                        output_tokens: Some(0),
                        cache_creation_input_tokens: Some(3),
                        openai_cache_write_details: Some(super::super::OpenAiCacheWriteDetails {
                            ttl_30m_tokens: Some(1),
                            ttl_12h_tokens: Some(2),
                        }),
                        ..Usage::default()
                    }),
                    Event::Finish(FinishReason::Stop),
                    Event::Done,
                ] {
                    encoder.consume(event).unwrap();
                }
                let bytes = encoder.terminal().unwrap().concat();
                let wire = std::str::from_utf8(&bytes).unwrap();
                assert!(wire.contains("\"cache_write_tokens\":3"));
                assert!(wire.contains("\"ttl_30m_tokens\":1"));
                assert!(wire.contains("\"ttl_12h_tokens\":2"));
                assert!(!wire.contains("openai_cache_write_details"));
            }
        }
    }

    #[test]
    fn openai_terminal_usage_preserves_cache_creation_details() {
        for client in [ClientProtocol::Chat, ClientProtocol::Responses] {
            for stream in [false, true] {
                let mut body = match client {
                    ClientProtocol::Chat => json!({
                        "model":"m","messages":[{"role":"user","content":"hello"}]
                    }),
                    ClientProtocol::Responses => json!({"model":"m","input":"hello"}),
                    _ => unreachable!(),
                };
                body["stream"] = json!(stream);
                if client == ClientProtocol::Chat && stream {
                    body["stream_options"] = json!({"include_usage":true});
                }
                let limits = Limits::default();
                let request =
                    Request::parse(client, &serde_json::to_vec(&body).unwrap(), &limits).unwrap();
                let mut encoder = Encoder::new(&request, "cache-test", &limits);
                for event in [
                    Event::Start("up".into()),
                    Event::Block {
                        index: 0,
                        kind: BlockKind::Text,
                    },
                    Event::Delta {
                        index: 0,
                        text: "ok".into(),
                    },
                    Event::EndBlock(0),
                    Event::Usage(Usage {
                        input_tokens: Some(10),
                        output_tokens: Some(1),
                        cache_creation_input_tokens: Some(3),
                        cache_creation_details: Some(super::super::CacheCreationDetails {
                            ephemeral_5m_input_tokens: Some(0),
                            ephemeral_1h_input_tokens: None,
                        }),
                        ..Usage::default()
                    }),
                    Event::Finish(FinishReason::Stop),
                    Event::Done,
                ] {
                    encoder.consume(event).unwrap();
                }
                let terminal = encoder.terminal().unwrap().concat();
                let wire = std::str::from_utf8(&terminal).unwrap();
                assert!(
                    wire.contains("\"cache_creation_details\":{\"ephemeral_5m_input_tokens\":0}")
                );
                assert!(!wire.contains("ephemeral_1h_input_tokens"));
            }
        }
    }
}

#[cfg(test)]
mod writer_contract_tests {
    use super::*;

    fn encoder(client: ClientProtocol, stream: bool) -> Encoder {
        let limits = Limits::default();
        let mut request = Request::parse(
            ClientProtocol::Chat,
            br#"{"model":"m","messages":[{"role":"user","content":"hello"}]}"#,
            &limits,
        )
        .unwrap();
        // Exercise the encoder independently of native request parsing.
        request.client = client;
        request.stream = stream;
        request.include_usage = true;
        Encoder::new(&request, "contract", &limits)
    }
    fn finish(encoder: &mut Encoder, reason: FinishReason) -> Vec<u8> {
        assert!(encoder.consume(Event::Finish(reason)).unwrap().is_empty());
        assert!(encoder.consume(Event::Done).unwrap().is_empty());
        encoder.terminal().unwrap().concat()
    }
    #[test]
    fn complete_native_signatures_stay_on_their_original_parts() {
        for client in [ClientProtocol::Anthropic, ClientProtocol::Gemini] {
            let mut e = encoder(client, false);
            let provider = if client == ClientProtocol::Anthropic {
                UpstreamProtocol::Anthropic
            } else {
                UpstreamProtocol::Gemini
            };
            for event in [
                Event::Start("up".into()),
                Event::Block {
                    index: 0,
                    kind: BlockKind::Thinking,
                },
                Event::Delta {
                    index: 0,
                    text: "thought".into(),
                },
                Event::Signature {
                    index: 0,
                    provider,
                    value: "signed".into(),
                },
                Event::EndBlock(0),
                Event::Usage(Usage {
                    input_tokens: Some(10),
                    output_tokens: Some(2),
                    cached_input_tokens: Some(0),
                    cache_creation_input_tokens: Some(0),
                    reasoning_output_tokens: Some(1),
                    ..Usage::default()
                }),
            ] {
                assert!(e.consume(event).unwrap().is_empty());
            }
            let response: Value =
                serde_json::from_slice(&finish(&mut e, FinishReason::Stop)).unwrap();
            if client == ClientProtocol::Anthropic {
                assert_eq!(response["content"][0]["thinking"], "thought");
                assert_eq!(response["content"][0]["signature"], "signed");
            } else {
                assert_eq!(
                    response["candidates"][0]["content"]["parts"][0]["text"],
                    "thought"
                );
                assert_eq!(
                    response["candidates"][0]["content"]["parts"][0]["thoughtSignature"],
                    "signed"
                );
            }
        }
    }
    #[test]
    fn complete_responses_reasoning_signature_is_item_metadata() {
        let mut e = encoder(ClientProtocol::Responses, false);
        for event in [
            Event::Start("up".into()),
            Event::Block {
                index: 0,
                kind: BlockKind::Thinking,
            },
            Event::Delta {
                index: 0,
                text: "thought".into(),
            },
            Event::Signature {
                index: 0,
                provider: UpstreamProtocol::Gemini,
                value: "signed".into(),
            },
            Event::EndBlock(0),
        ] {
            e.consume(event).unwrap();
        }
        let response: Value = serde_json::from_slice(&finish(&mut e, FinishReason::Stop)).unwrap();
        assert_eq!(response["output"][0]["summary"][0]["text"], "thought");
        assert_eq!(
            response["output"][0]["provider_metadata"]["protocol"],
            "gemini"
        );
        assert_eq!(
            response["output"][0]["provider_metadata"]["signature"],
            "signed"
        );
    }
    #[test]
    fn native_generic_creation_keeps_reported_total_and_raw_input() {
        for stream in [false, true] {
            let mut e = encoder(ClientProtocol::Anthropic, stream);
            e.consume(Event::Start("up".into())).unwrap();
            e.consume(Event::Usage(Usage {
                input_tokens: Some(37),
                uncached_input_tokens: Some(10),
                output_tokens: Some(1),
                cached_input_tokens: Some(20),
                cache_creation_input_tokens: Some(2),
                cache_creation_details: Some(super::super::CacheCreationDetails {
                    ephemeral_5m_input_tokens: Some(3),
                    ephemeral_1h_input_tokens: Some(4),
                }),
                cache_creation_mode: Some(crate::routing::CacheCreationMode::GenericRemainderV1),
                ..Usage::default()
            }))
            .unwrap();
            let bytes = finish(&mut e, FinishReason::Stop);
            let wire = std::str::from_utf8(&bytes).unwrap();
            assert!(wire.contains("\"input_tokens\":10"));
            assert!(wire.contains("\"cache_creation_input_tokens\":2"));
            assert!(wire.contains("\"ephemeral_5m_input_tokens\":3"));
            assert!(wire.contains("\"ephemeral_1h_input_tokens\":4"));
            assert_eq!(e.usage().input_tokens, Some(37));
        }
    }
    #[test]
    fn native_error_envelopes_preserve_core_status_mapping() {
        for stream in [false, true] {
            for client in [ClientProtocol::Anthropic, ClientProtocol::Gemini] {
                let mut e = encoder(client, stream);
                let bytes = e.error_packet(&RelayError::Unauthorized).unwrap();
                let raw = std::str::from_utf8(&bytes).unwrap();
                let json = if stream {
                    raw.lines()
                        .find_map(|line| line.strip_prefix("data: "))
                        .unwrap()
                } else {
                    raw
                };
                let value: Value = serde_json::from_str(json).unwrap();
                if client == ClientProtocol::Anthropic {
                    assert_eq!(value["type"], "error");
                    assert_eq!(value["error"]["type"], "authentication_error");
                } else {
                    assert_eq!(value["error"]["code"], 401);
                    assert_eq!(value["error"]["status"], "UNAUTHENTICATED");
                }
            }
        }
    }
    #[test]
    fn complete_images_grounding_continuation_and_stop_sequence_survive() {
        let mut e = encoder(ClientProtocol::Chat, false);
        e.consume(Event::Start("up".into())).unwrap();
        e.consume(Event::Images(vec![
            json!({"type":"image_url","image_url":{"url":"https://example.com/result.png"}}),
        ]))
        .unwrap();
        let response: Value = serde_json::from_slice(&finish(&mut e, FinishReason::Stop)).unwrap();
        assert_eq!(
            response["choices"][0]["message"]["images"][0]["image_url"]["url"],
            "https://example.com/result.png"
        );
        let mut e = encoder(ClientProtocol::Gemini, false);
        for event in [
            Event::Start("up".into()),
            Event::GroundingMetadata(json!({"webSearchQueries":["q"]})),
            Event::ContinuationToken("token".into()),
        ] {
            e.consume(event).unwrap();
        }
        let response: Value =
            serde_json::from_slice(&finish(&mut e, FinishReason::Continuation)).unwrap();
        assert_eq!(
            response["candidates"][0]["groundingMetadata"]["webSearchQueries"],
            json!(["q"])
        );
        assert_eq!(response["candidates"][0]["finishReason"], "CONTINUATION");
        assert_eq!(response["candidates"][0]["continuationToken"], "token");
        let mut e = encoder(ClientProtocol::Anthropic, false);
        for event in [
            Event::Start("up".into()),
            Event::StopSequence("END".into()),
            Event::Usage(Usage {
                input_tokens: Some(2),
                output_tokens: Some(1),
                cached_input_tokens: Some(0),
                cache_creation_input_tokens: Some(0),
                ..Usage::default()
            }),
        ] {
            e.consume(event).unwrap();
        }
        let response: Value = serde_json::from_slice(&finish(&mut e, FinishReason::Stop)).unwrap();
        assert_eq!(response["stop_reason"], "stop_sequence");
        assert_eq!(response["stop_sequence"], "END");
    }
    #[test]
    fn native_text_signatures_survive_both_openai_encoders_and_modes() {
        for client in [ClientProtocol::Chat, ClientProtocol::Responses] {
            for stream in [false, true] {
                let mut e = encoder(client, stream);
                let mut bytes = Vec::new();
                for event in [
                    Event::Start("up".into()),
                    Event::Block {
                        index: 0,
                        kind: BlockKind::Text,
                    },
                    Event::Delta {
                        index: 0,
                        text: "answer".into(),
                    },
                    Event::Signature {
                        index: 0,
                        provider: UpstreamProtocol::Gemini,
                        value: "opaque".into(),
                    },
                    Event::EndBlock(0),
                ] {
                    bytes.extend(e.consume(event).unwrap().concat());
                }
                bytes.extend(finish(&mut e, FinishReason::Stop));
                let wire = std::str::from_utf8(&bytes).unwrap();
                let packets: Vec<Value> = if stream {
                    wire.lines()
                        .filter_map(|line| line.strip_prefix("data: "))
                        .filter(|value| *value != "[DONE]")
                        .map(|value| serde_json::from_str(value).unwrap())
                        .collect()
                } else {
                    vec![serde_json::from_slice(&bytes).unwrap()]
                };
                match client {
                    ClientProtocol::Chat if stream => assert!(packets.iter().any(|p| p["choices"]
                        [0]["delta"]["content_details"][0]["signature"]
                        == "opaque")),
                    ClientProtocol::Chat => assert_eq!(
                        packets[0]["choices"][0]["message"]["content_details"][0]["signature"],
                        "opaque"
                    ),
                    ClientProtocol::Responses => {
                        let response = if stream {
                            &packets.last().unwrap()["response"]
                        } else {
                            &packets[0]
                        };
                        assert_eq!(
                            response["output"][0]["provider_metadata"]["protocol"],
                            "gemini"
                        );
                        assert_eq!(
                            response["output"][0]["provider_metadata"]["signature"],
                            "opaque"
                        );
                        assert_eq!(response["store"], false);
                    }
                    _ => unreachable!(),
                }
            }
        }
    }
    #[test]
    fn native_gemini_stream_binds_thought_signature_and_continuation() {
        let mut e = encoder(ClientProtocol::Gemini, true);
        let mut bytes = Vec::new();
        for event in [
            Event::Start("up".into()),
            Event::Block {
                index: 0,
                kind: BlockKind::Thinking,
            },
            Event::Delta {
                index: 0,
                text: "thought".into(),
            },
            Event::Signature {
                index: 0,
                provider: UpstreamProtocol::Gemini,
                value: "opaque".into(),
            },
            Event::EndBlock(0),
            Event::ContinuationToken("next".into()),
        ] {
            bytes.extend(e.consume(event).unwrap().concat());
        }
        bytes.extend(finish(&mut e, FinishReason::Continuation));
        let packets: Vec<Value> = std::str::from_utf8(&bytes)
            .unwrap()
            .lines()
            .filter_map(|line| line.strip_prefix("data: "))
            .map(|line| serde_json::from_str(line).unwrap())
            .collect();
        assert!(
            packets.iter().any(
                |p| p["candidates"][0]["content"]["parts"][0]["text"] == "thought"
                    && p["candidates"][0]["content"]["parts"][0]["thoughtSignature"] == "opaque"
            )
        );
        let terminal = packets
            .iter()
            .find(|p| p["candidates"][0]["finishReason"] == "CONTINUATION")
            .unwrap();
        assert_eq!(terminal["candidates"][0]["continuationToken"], "next");
    }
    #[test]
    fn responses_stream_reasoning_keeps_lifecycle_ids_and_sequence() {
        for finish_reason in [FinishReason::Stop, FinishReason::Length] {
            let mut e = encoder(ClientProtocol::Responses, true);
            let mut bytes = Vec::new();
            for event in [
                Event::Start("up".into()),
                Event::Block {
                    index: 0,
                    kind: BlockKind::Thinking,
                },
                Event::Delta {
                    index: 0,
                    text: "think ".into(),
                },
                Event::Delta {
                    index: 0,
                    text: "carefully".into(),
                },
                Event::EndBlock(0),
            ] {
                bytes.extend(e.consume(event).unwrap().concat());
            }
            bytes.extend(finish(&mut e, finish_reason));
            let packets: Vec<Value> = std::str::from_utf8(&bytes)
                .unwrap()
                .lines()
                .filter_map(|line| line.strip_prefix("data: "))
                .map(|line| serde_json::from_str(line).unwrap())
                .collect();
            assert_eq!(packets[0]["type"], "response.created");
            assert_eq!(packets[1]["type"], "response.in_progress");
            for (index, p) in packets.iter().enumerate() {
                assert_eq!(p["sequence_number"], index);
            }
            let response = &packets.last().unwrap()["response"];
            assert_eq!(
                response["output"][0]["summary"][0]["text"],
                "think carefully"
            );
            assert_eq!(response["output"][0]["id"], "rs_resp_contract_0");
            assert_eq!(
                response["status"],
                if finish_reason == FinishReason::Length {
                    "incomplete"
                } else {
                    "completed"
                }
            );
            let lifecycle: Vec<_> = packets
                .iter()
                .filter(|p| {
                    p["item"]["type"] == "reasoning"
                        || p["type"]
                            .as_str()
                            .is_some_and(|kind| kind.starts_with("response.reasoning_summary_"))
                })
                .collect();
            assert_eq!(
                lifecycle
                    .iter()
                    .map(|p| p["type"].as_str().unwrap())
                    .collect::<Vec<_>>(),
                [
                    "response.output_item.added",
                    "response.reasoning_summary_part.added",
                    "response.reasoning_summary_text.delta",
                    "response.reasoning_summary_text.delta",
                    "response.reasoning_summary_text.done",
                    "response.reasoning_summary_part.done",
                    "response.output_item.done"
                ]
            );
            assert_eq!(lifecycle[0]["item"]["id"], response["output"][0]["id"]);
        }
    }
    #[test]
    fn previewed_responses_terminal_does_not_block_settlement_error() {
        let mut e = encoder(ClientProtocol::Responses, true);
        let mut bytes = Vec::new();
        for event in [
            Event::Start("up".into()),
            Event::Block {
                index: 0,
                kind: BlockKind::Text,
            },
            Event::Delta {
                index: 0,
                text: "answer".into(),
            },
            Event::EndBlock(0),
        ] {
            bytes.extend(e.consume(event).unwrap().concat());
        }
        let before: Vec<Value> = std::str::from_utf8(&bytes)
            .unwrap()
            .lines()
            .filter_map(|line| line.strip_prefix("data: "))
            .map(|line| serde_json::from_str(line).unwrap())
            .collect();
        let prepared = finish(&mut e, FinishReason::Stop);
        assert!(
            std::str::from_utf8(&prepared)
                .unwrap()
                .contains("response.completed")
        );
        assert!(
            e.writer
                .as_ref()
                .is_some_and(|writer| !writer.is_finished())
        );
        // Settlement failed: the privately prepared terminal is discarded.
        let failure = e.error_packet(&RelayError::Unauthorized).unwrap();
        let failure: Value = serde_json::from_str(
            std::str::from_utf8(&failure)
                .unwrap()
                .lines()
                .find_map(|line| line.strip_prefix("data: "))
                .unwrap(),
        )
        .unwrap();
        assert_eq!(failure["type"], "error");
        assert_eq!(failure["sequence_number"], before.len());
        assert!(e.writer.as_ref().is_some_and(|writer| writer.is_finished()));
        assert!(e.error_packet(&RelayError::Unauthorized).is_err());
    }
    #[test]
    fn complete_signed_tools_keep_metadata_on_the_call() {
        for client in [ClientProtocol::Chat, ClientProtocol::Responses] {
            let mut e = encoder(client, false);
            for event in [
                Event::Start("up".into()),
                Event::Block {
                    index: 0,
                    kind: BlockKind::Tool {
                        id: "call".into(),
                        name: "f".into(),
                    },
                },
                Event::Delta {
                    index: 0,
                    text: "{}".into(),
                },
                Event::Signature {
                    index: 0,
                    provider: UpstreamProtocol::Gemini,
                    value: "signed".into(),
                },
                Event::EndBlock(0),
            ] {
                e.consume(event).unwrap();
            }
            let response: Value =
                serde_json::from_slice(&finish(&mut e, FinishReason::ToolCalls)).unwrap();
            let call = if client == ClientProtocol::Chat {
                &response["choices"][0]["message"]["tool_calls"][0]
            } else {
                &response["output"][0]
            };
            assert_eq!(call["provider_metadata"]["protocol"], "gemini");
            assert_eq!(call["provider_metadata"]["signature"], "signed");
        }
    }
    #[test]
    fn chat_without_include_usage_keeps_core_metering_and_partial_tool_terminal() {
        let mut e = encoder(ClientProtocol::Chat, true);
        e.include_usage = false;
        for event in [
            Event::Start("up".into()),
            Event::Block {
                index: 0,
                kind: BlockKind::Tool {
                    id: "call".into(),
                    name: "f".into(),
                },
            },
            Event::Delta {
                index: 0,
                text: "{\"x\":".into(),
            },
            Event::EndBlock(0),
            Event::Usage(Usage {
                input_tokens: Some(10),
                output_tokens: Some(2),
                ..Usage::default()
            }),
        ] {
            e.consume(event).unwrap();
        }
        assert_eq!(e.usage().input_tokens, Some(10));
        assert_eq!(e.usage().output_tokens, Some(2));
        let bytes = finish(&mut e, FinishReason::Length);
        let wire = std::str::from_utf8(&bytes).unwrap();
        assert!(wire.contains("\"finish_reason\":\"length\""));
        assert!(wire.ends_with("data: [DONE]\n\n"));
        assert!(!wire.contains("\"usage\""));
    }
}

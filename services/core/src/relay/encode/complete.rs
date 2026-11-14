//! Complete response IR: text borrows Core blocks; only metadata is JSON.
use super::*;
use zconv::{
    ir::{self, Content, Presence, Record, Span, Text},
    json::{self as wire, Token},
};

fn extras<'a>(
    records: &mut Vec<Record<'a>>,
    object: Option<wire::Value<'a, '_>>,
    source: Format,
    location: &'static str,
    owner: usize,
) -> Span {
    let start = records.len();
    if let Some(object) = object {
        for (key, value) in object.object() {
            records.push(Record::Extension(ir::Extension {
                source,
                key,
                value: value.raw_value(),
                location,
                owner,
            }));
        }
    }
    if records.len() == start {
        Span::EMPTY
    } else {
        Span {
            start,
            end: records.len(),
        }
    }
}
// Size the JSON token tape from the already-owned metadata tree, rather than
// allocating one Token for every byte of a potentially large signature string.
fn token_count(value: &Value) -> usize {
    1 + match value {
        Value::Array(values) => values.iter().map(token_count).sum(),
        Value::Object(values) => values.values().map(|value| 1 + token_count(value)).sum(),
        _ => 0,
    }
}
fn message(end: usize, extensions: Span) -> ir::Message<'static> {
    ir::Message {
        role: ir::Role::Assistant,
        legacy_function: false,
        system_instruction: false,
        tool_calls: Presence::Missing,
        function_call: Presence::Missing,
        refusal: Presence::Missing,
        reasoning: Presence::Missing,
        name: Presence::Missing,
        shape: ir::ContentShape::Null,
        end,
        extensions,
        item_id: Presence::Missing,
        item_status: Presence::Missing,
    }
}
fn block(content: Content<'_>) -> ir::Block<'_> {
    ir::Block {
        content,
        signature: Presence::Missing,
        extensions: Span::EMPTY,
        end: 0,
        item_id: Presence::Missing,
        item_status: Presence::Missing,
    }
}
impl Encoder {
    /// Core receipts are the authority, including lifetime accounting. Native
    /// projections retain unknown fields rather than inventing zero counters.
    pub(super) fn native_usage(&self) -> Result<Value, RelayError> {
        let usage = &self.usage;
        usage.validate()?;
        if self.client == ClientProtocol::Anthropic {
            let input = if usage.cache_creation_mode
                == Some(crate::routing::CacheCreationMode::GenericRemainderV1)
            {
                usage
                    .uncached_input_tokens
                    .ok_or(RelayError::Protocol("missing uncached input usage"))?
            } else {
                let write = usage
                    .effective_cache_creation_input_tokens()?
                    .ok_or(RelayError::Protocol("missing cache creation usage"))?;
                usage
                    .input_tokens
                    .ok_or(RelayError::Protocol("missing input usage"))?
                    .checked_sub(
                        usage
                            .cached_input_tokens
                            .ok_or(RelayError::Protocol("missing cache read usage"))?,
                    )
                    .and_then(|n| n.checked_sub(write))
                    .ok_or(RelayError::Protocol("invalid cache usage"))?
            };
            let mut value = json!({"input_tokens":input, "output_tokens":usage.output_tokens.ok_or(RelayError::Protocol("missing output usage"))?});
            if let Some(n) = usage.cached_input_tokens {
                value["cache_read_input_tokens"] = json!(n);
            }
            if let Some(n) = usage.cache_creation_input_tokens {
                value["cache_creation_input_tokens"] = json!(n);
            }
            if let Some(details) = usage.cache_creation_details {
                value["cache_creation"] = json!(details);
            }
            return Ok(value);
        }
        if usage.cache_creation_input_tokens.is_some_and(|n| n != 0)
            || usage.cache_creation_details.is_some_and(|d| {
                d.ephemeral_5m_input_tokens.is_some_and(|n| n != 0)
                    || d.ephemeral_1h_input_tokens.is_some_and(|n| n != 0)
            })
            || usage.openai_cache_write_details.is_some_and(|d| {
                d.ttl_30m_tokens.is_some_and(|n| n != 0) || d.ttl_12h_tokens.is_some_and(|n| n != 0)
            })
        {
            return Err(RelayError::Unsupported("Gemini cache write usage"));
        }
        let mut value = serde_json::Map::new();
        for (key, count) in [
            ("promptTokenCount", usage.input_tokens),
            ("cachedContentTokenCount", usage.cached_input_tokens),
            ("thoughtsTokenCount", usage.reasoning_output_tokens),
            ("toolUsePromptTokenCount", usage.tool_use_prompt_tokens),
        ] {
            if let Some(n) = count {
                value.insert(key.into(), json!(n));
            }
        }
        if let (Some(o), Some(r)) = (usage.output_tokens, usage.reasoning_output_tokens) {
            value.insert(
                "candidatesTokenCount".into(),
                json!(
                    o.checked_sub(r)
                        .ok_or(RelayError::Protocol("invalid reasoning usage"))?
                ),
            );
        }
        if let (Some(i), Some(o)) = (usage.input_tokens, usage.output_tokens)
            && (usage.google_search_grounded_requests.is_none()
                || usage.tool_use_prompt_tokens.is_some())
        {
            value.insert(
                "totalTokenCount".into(),
                json!(
                    i.checked_add(o)
                        .and_then(|n| n.checked_add(usage.tool_use_prompt_tokens.unwrap_or(0)))
                        .ok_or(RelayError::Protocol("usage overflow"))?
                ),
            );
        }
        Ok(Value::Object(value))
    }
    pub(super) fn complete(&self) -> Result<Vec<u8>, RelayError> {
        let target = format(self.client);
        let finish = match self.finish.ok_or(RelayError::Truncated)? {
            FinishReason::Stop => ir::Finish::Stop,
            FinishReason::ToolCalls => ir::Finish::ToolCalls,
            FinishReason::Length | FinishReason::Continuation => ir::Finish::Length,
            FinishReason::ContentFilter => ir::Finish::Safety,
        };
        let usage = match self.client {
            ClientProtocol::Chat | ClientProtocol::Responses => self.usage.wire(self.client),
            _ => self.native_usage()?,
        };
        let mut metadata = json!({"usage":usage,"choice":{},"message":{},"blocks":[]});
        if self.client == ClientProtocol::Responses {
            metadata["response"] = json!({"store":false});
        }
        if self.client == ClientProtocol::Gemini {
            if let Some(v) = &self.grounding {
                metadata["choice"]["groundingMetadata"] = v.clone();
            }
            if let Some(v) = &self.continuation {
                metadata["choice"]["continuationToken"] = json!(v);
            }
        }
        if !self.images.is_empty() {
            metadata["message"]["images"] = json!(self.images);
        }
        let mut content_details = Vec::new();
        let mut reasoning_details = Vec::new();
        for (index, b) in self.blocks.iter().enumerate() {
            let mut item = json!({});
            if let Some((provider, signature)) = &b.signature {
                if self.client == ClientProtocol::Chat {
                    match b.kind {
                        BlockKind::Tool { .. } => item["provider_metadata"] = json!({"protocol":provider,"signature":signature}),
                        BlockKind::Thinking => reasoning_details.push(json!({"index":index,"text":b.text,"protocol":provider,"signature":signature})),
                        _ => content_details.push(json!({"index":index,"protocol":provider,"signature":signature})),
                    }
                } else if self.client == ClientProtocol::Responses {
                    item["provider_metadata"] = json!({"protocol":provider,"signature":signature});
                }
            }
            metadata["blocks"]
                .as_array_mut()
                .ok_or(RelayError::Protocol("metadata encoding failed"))?
                .push(item);
        }
        if !content_details.is_empty() {
            metadata["message"]["content_details"] = json!(content_details);
        }
        if reasoning_details.len() == 1 {
            metadata["message"]["provider_metadata"] = json!({"protocol":reasoning_details[0]["protocol"],"signature":reasoning_details[0]["signature"]});
        } else if !reasoning_details.is_empty() {
            metadata["message"]["reasoning_details"] = json!(reasoning_details);
        }
        let bytes = serde_json::to_vec(&metadata)
            .map_err(|_| RelayError::Protocol("metadata encoding failed"))?;
        if bytes.len() > self.wire_limit() {
            return Err(RelayError::Limit("encoded response"));
        }
        let mut metadata_tokens = vec![Token::EMPTY; token_count(&metadata)];
        let document = wire::parse(
            &bytes,
            &mut metadata_tokens,
            wire::Limits {
                max_input: self.wire_limit(),
                ..wire::Limits::default()
            },
        )
        .map_err(library_error)?;
        let root = document.root();
        let mut records = vec![Record::EMPTY];
        let choice_extras = extras(&mut records, root.get("choice"), target, "choice", 0);
        let ids: Vec<_> = (0..self.blocks.len()).map(|i| self.block_id(i)).collect();
        let mut ordered: Vec<_> = self.blocks.iter().enumerate().collect();
        if self.client == ClientProtocol::Chat {
            // Chat has one text field and one tool array, unlike ordered items.
            ordered.sort_by_key(|(_, b)| matches!(b.kind, BlockKind::Tool { .. }));
        }
        let shared_message = if self.client != ClientProtocol::Responses {
            let at = records.len();
            records.push(Record::EMPTY);
            let span = extras(&mut records, root.get("message"), target, "message", at);
            Some((at, span))
        } else {
            None
        };
        let block_meta: Vec<_> = root
            .get("blocks")
            .ok_or(RelayError::Protocol("metadata encoding failed"))?
            .array()
            .collect();
        for (index, b) in ordered {
            let parent = if self.client == ClientProtocol::Responses {
                let at = records.len();
                records.push(Record::EMPTY);
                Some(at)
            } else {
                None
            };
            let is_text = matches!(b.kind, BlockKind::Text | BlockKind::Refusal);
            let message_extras = if is_text {
                parent
                    .map(|at| {
                        extras(
                            &mut records,
                            block_meta.get(index).copied(),
                            target,
                            "message",
                            at,
                        )
                    })
                    .unwrap_or(Span::EMPTY)
            } else {
                Span::EMPTY
            };
            let content = match &b.kind {
                BlockKind::Text => Content::Text(Text::Utf8(&b.text)),
                BlockKind::Refusal => Content::Refusal(Text::Utf8(&b.text)),
                BlockKind::Thinking => Content::Reasoning {
                    kind: if self.client == ClientProtocol::Responses {
                        ir::ReasoningKind::Summary
                    } else {
                        ir::ReasoningKind::Thought
                    },
                    text: Presence::Value(Text::Utf8(&b.text)),
                    signature: Presence::Missing,
                },
                BlockKind::Tool { id, name } => Content::ToolCall {
                    id: Presence::Value(Text::Utf8(id)),
                    name: Text::Utf8(name),
                    arguments: ir::Data::JsonText(Text::Utf8(&b.text)),
                    legacy: false,
                },
            };
            let at = records.len();
            records.push(Record::EMPTY);
            let mut entry = block(content);
            if matches!(
                self.client,
                ClientProtocol::Anthropic | ClientProtocol::Gemini
            ) && let Some((_, value)) = &b.signature
            {
                entry.signature = Presence::Value(ir::ProviderSignature {
                    source: target,
                    value: Text::Utf8(value),
                });
            }
            if self.client == ClientProtocol::Responses {
                entry.item_id = Presence::Value(Text::Utf8(&ids[index]));
                entry.item_status = Presence::Value(Text::Utf8(
                    if self.finish.is_some_and(FinishReason::truncates_output) {
                        "incomplete"
                    } else {
                        "completed"
                    },
                ));
            }
            if !is_text {
                entry.extensions = extras(
                    &mut records,
                    block_meta.get(index).copied(),
                    target,
                    if self.client == ClientProtocol::Chat
                        && matches!(b.kind, BlockKind::Tool { .. })
                    {
                        "tool_call"
                    } else {
                        "content"
                    },
                    at,
                );
            }
            entry.end = records.len();
            records[at] = Record::Block(entry);
            if self.client == ClientProtocol::Responses
                && matches!(b.kind, BlockKind::Thinking)
                && entry.extensions.start != entry.extensions.end
            {
                // zconv represents item metadata on a reasoning header with
                // absent text, separately from its summary content records.
                let text = entry.content;
                entry.content = Content::Reasoning {
                    kind: ir::ReasoningKind::Summary,
                    text: Presence::Missing,
                    signature: Presence::Missing,
                };
                records[at] = Record::Block(entry);
                let mut summary = block(text);
                summary.end = records.len() + 1;
                records.push(Record::Block(summary));
            }
            if let Some(at) = parent {
                let mut m = message(records.len(), message_extras);
                if is_text {
                    m.item_id = Presence::Value(Text::Utf8(&ids[index]));
                    m.item_status = entry.item_status;
                }
                records[at] = Record::Message(m);
            }
        }
        if let Some((at, span)) = shared_message {
            records[at] = Record::Message(message(records.len(), span));
        }
        records[0] = Record::Choice(ir::Choice {
            index: 0,
            finish,
            raw_finish: match self.finish {
                Some(FinishReason::Continuation) if self.client == ClientProtocol::Gemini => {
                    Presence::Value(Text::Utf8("CONTINUATION"))
                }
                Some(FinishReason::Stop) if self.stop_sequence.is_some() => {
                    Presence::Value(Text::Utf8("stop_sequence"))
                }
                _ => Presence::Missing,
            },
            end: records.len(),
            extensions: choice_extras,
        });
        // Usage extensions carry exactly Core's authorized wire counters. They
        // never feed the local accounting or the settlement receipt back.
        let usage_extras = extras(&mut records, root.get("usage"), target, "usage", usize::MAX);
        let response_extras = extras(
            &mut records,
            root.get("response"),
            target,
            "response",
            usize::MAX,
        );
        let incomplete_bytes = serde_json::to_vec(
            &json!({"reason":self.finish.map(FinishReason::responses_incomplete_reason)}),
        )
        .map_err(|_| RelayError::Protocol("incomplete encoding failed"))?;
        let mut incomplete_tokens = [Token::EMPTY; 8];
        let incomplete = wire::parse(
            &incomplete_bytes,
            &mut incomplete_tokens,
            wire::Limits::default(),
        )
        .map_err(library_error)?;
        let response = ir::Response {
            source: target,
            id: Presence::Value(Text::Utf8(&self.id)),
            model: Presence::Value(Text::Utf8(&self.model)),
            created: if matches!(
                self.client,
                ClientProtocol::Chat | ClientProtocol::Responses
            ) {
                Presence::Value(ir::Number::Unsigned(self.created))
            } else {
                Presence::Missing
            },
            created_iso: Presence::Missing,
            updated_iso: Presence::Missing,
            status: finish,
            usage: ir::Usage {
                present: !usage.is_object() || !usage.as_object().is_none_or(|v| v.is_empty()),
                is_null: usage.is_null(),
                extensions: usage_extras,
                ..ir::Usage::default()
            },
            error: if self.client == ClientProtocol::Responses {
                Presence::Null
            } else {
                Presence::Missing
            },
            stop_sequence: if self.client == ClientProtocol::Anthropic {
                self.stop_sequence
                    .as_deref()
                    .map(|s| Presence::Value(Text::Utf8(s)))
                    .unwrap_or(Presence::Null)
            } else {
                Presence::Missing
            },
            incomplete_details: if self.client == ClientProtocol::Responses {
                if self.finish.is_some_and(FinishReason::truncates_output) {
                    Presence::Value(incomplete.root().raw_value())
                } else {
                    Presence::Null
                }
            } else {
                Presence::Missing
            },
            extensions: response_extras,
            records: &mut records,
        };
        let mut output = vec![0; 4096.min(self.wire_limit())];
        let mut scratch = vec![0; 256.min(self.wire_limit())];
        let mut tokens = vec![Token::EMPTY; 64];
        loop {
            match zconv::protocol::encode_response(
                &response,
                target,
                &ir::Context::default(),
                ir::Policy::default(),
                &mut output,
                &mut scratch,
                &mut tokens,
            ) {
                Ok(report) => {
                    output.truncate(report.written);
                    return Ok(output);
                }
                Err(zconv::Error::OutputTooSmall { needed }) => {
                    stream::grow(&mut output, needed, self.wire_limit(), 0)?
                }
                Err(zconv::Error::ScratchTooSmall { needed }) => {
                    stream::grow(&mut scratch, needed, self.wire_limit(), 0)?
                }
                Err(zconv::Error::WorkspaceTooSmall { needed }) => {
                    stream::grow(&mut tokens, needed, self.wire_limit(), Token::EMPTY)?
                }
                Err(error) => return Err(library_error(error)),
            }
        }
    }
}

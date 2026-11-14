//! Borrow admitted Core records into zconv IR. Protocol projection lives in zconv.
use super::request::{InputBlock, Request, Role, ToolChoice};
use super::{RelayError, UpstreamProtocol};
use serde_json::Value;
use zconv::{
    Protocol,
    ir::*,
    json::{self, RawValue, Token},
};

const MAX_BYTES: usize = 16 * 1024 * 1024;
const MAX_TOKENS: usize = 262_144;
fn protocol(p: UpstreamProtocol) -> Protocol {
    match p {
        UpstreamProtocol::Chat => Protocol::OpenAiChat,
        UpstreamProtocol::Responses => Protocol::OpenAiResponses,
        UpstreamProtocol::Anthropic => Protocol::AnthropicMessages,
        UpstreamProtocol::Gemini => Protocol::GeminiGenerateContent,
    }
}
fn error(e: zconv::Error) -> RelayError {
    #[cfg(test)]
    eprintln!("zconv request conversion failed: {e:?}");
    match e {
        zconv::Error::InputTooLarge { .. }
        | zconv::Error::WorkspaceTooSmall { .. }
        | zconv::Error::IrTooSmall { .. }
        | zconv::Error::OutputTooSmall { .. }
        | zconv::Error::ScratchTooSmall { .. }
        | zconv::Error::DepthLimitExceeded { .. }
        | zconv::Error::LimitExceeded { .. } => RelayError::Limit("protocol conversion workspace"),
        _ => RelayError::Unsupported("request cannot be converted without semantic loss"),
    }
}
fn raw<'a>(text: &'a str, tokens: &mut [Token]) -> Result<RawValue<'a>, RelayError> {
    json::parse(text.as_bytes(), tokens, json::Limits::default())
        .map(|doc| doc.root().raw_value())
        .map_err(error)
}
fn text(value: &str) -> Text<'_> {
    Text::Utf8(value)
}
fn present(value: Option<&str>) -> Presence<Text<'_>> {
    value
        .map(text)
        .map(Presence::Value)
        .unwrap_or(Presence::Missing)
}
fn block(content: Content<'_>, end: usize) -> Record<'_> {
    Record::Block(Block {
        content,
        signature: Presence::Missing,
        extensions: Span::EMPTY,
        end,
        item_id: Presence::Missing,
        item_status: Presence::Missing,
    })
}

pub(super) fn encode(
    request: &Request,
    target: UpstreamProtocol,
    model: &str,
) -> Result<Value, RelayError> {
    // ImageInput's fields are private to vision.rs. This is an individual media
    // accessor, never a complete source request or another protocol writer.
    let images = request
        .messages
        .iter()
        .flat_map(|m| &m.content)
        .map(|b| {
            if let InputBlock::Image(image) = b {
                image.wire(UpstreamProtocol::Chat).map(Some)
            } else {
                Ok(None)
            }
        })
        .collect::<Result<Vec<_>, _>>()?;
    let schemas = request
        .tools
        .iter()
        .map(|t| {
            if serde_json::from_str::<Value>(&t.ordered_parameters)
                .ok()
                .as_ref()
                == Some(&t.parameters)
            {
                t.ordered_parameters.clone()
            } else {
                t.parameters.to_string()
            }
        })
        .collect::<Vec<_>>();
    let stop = serde_json::to_string(&request.stop).expect("string array serialization");
    let size = model.len()
        + stop.len()
        + schemas.iter().map(String::len).sum::<usize>()
        + request
            .tools
            .iter()
            .map(|t| t.name.len() + t.description.as_ref().map_or(0, String::len))
            .sum::<usize>()
        + request
            .messages
            .iter()
            .flat_map(|m| &m.content)
            .map(|b| match b {
                InputBlock::Text(s) | InputBlock::Refusal(s) => s.len(),
                InputBlock::Image(i) => i.encoded_len(),
                InputBlock::Thinking { text, signature } => {
                    text.len() + signature.as_ref().map_or(0, |(_, s)| s.len())
                }
                InputBlock::ToolCall {
                    id,
                    name,
                    arguments,
                    signature,
                } => {
                    id.len()
                        + name.len()
                        + arguments.len()
                        + signature.as_ref().map_or(0, |(_, s)| s.len())
                }
                InputBlock::ToolResult { id, content } => id.len() + content.len(),
            })
            .sum::<usize>();
    let count = request
        .messages
        .iter()
        .map(|m| m.content.len() + 2)
        .sum::<usize>()
        + request.tools.len();
    if size > MAX_BYTES || count > MAX_TOKENS {
        return Err(RelayError::Limit("protocol conversion input"));
    }
    let capacity = size
        .saturating_mul(6)
        .saturating_add(count.saturating_mul(512))
        .saturating_add(4096)
        .min(MAX_BYTES);
    let mut tokens = vec![Token::EMPTY; size.saturating_add(64).min(MAX_TOKENS)];
    let mut records = Vec::with_capacity(count);
    let mut ordinal = 0;
    for m in &request.messages {
        if m.role == Role::System && m.content.is_empty() {
            continue;
        }
        let role = match m.role {
            Role::System => zconv::ir::Role::System,
            Role::Developer => zconv::ir::Role::Developer,
            Role::User => zconv::ir::Role::User,
            Role::Assistant => zconv::ir::Role::Assistant,
            Role::Tool => zconv::ir::Role::Tool,
        };
        let start = records.len();
        records.push(Record::Message(zconv::ir::Message {
            role,
            legacy_function: false,
            system_instruction: false,
            tool_calls: Presence::Missing,
            function_call: Presence::Missing,
            refusal: Presence::Missing,
            reasoning: Presence::Missing,
            name: Presence::Missing,
            shape: if m.content.iter().any(|b| matches!(b, InputBlock::Image(_))) {
                ContentShape::Array
            } else if m.content.iter().any(|b| matches!(b, InputBlock::Text(_)))
                || m.role != Role::Assistant
            {
                ContentShape::String
            } else {
                ContentShape::Null
            },
            end: 0,
            extensions: Span::EMPTY,
            item_id: Presence::Missing,
            item_status: Presence::Missing,
        }));
        for b in &m.content {
            let content = match b {
                InputBlock::Text(s) => Content::Text(text(s)),
                InputBlock::Refusal(s) => Content::Refusal(text(s)),
                InputBlock::Thinking { text: s, signature } => Content::Reasoning {
                    kind: ReasoningKind::Thought,
                    text: Presence::Value(text(s)),
                    signature: present(signature.as_ref().map(|(_, s)| s.as_str())),
                },
                InputBlock::Image(_) => {
                    let image = &images[ordinal].as_ref().expect("prepared image")["image_url"];
                    let url = image["url"].as_str().expect("validated image URL");
                    Content::Media(Media {
                        kind: MediaKind::Image,
                        mime_type: Presence::Missing,
                        source: if url.starts_with("data:") {
                            MediaSource::DataUrl(text(url))
                        } else {
                            MediaSource::Url(text(url))
                        },
                        detail: present(image["detail"].as_str()),
                        filename: Presence::Missing,
                    })
                }
                InputBlock::ToolCall {
                    id,
                    name,
                    arguments,
                    ..
                } => Content::ToolCall {
                    id: Presence::Value(text(id)),
                    name: text(name),
                    arguments: Data::JsonText(text(arguments)),
                    legacy: false,
                },
                InputBlock::ToolResult { id, content } => Content::ToolResult {
                    id: Presence::Value(text(id)),
                    name: Presence::Missing,
                    result: if target == UpstreamProtocol::Gemini {
                        Data::ObjectOrText {
                            text: text(content),
                            key: text("output"),
                        }
                    } else {
                        Data::Text(text(content))
                    },
                    is_error: Presence::Missing,
                },
            };
            let i = records.len();
            records.push(block(content, i + 1));
            if let InputBlock::Thinking {
                signature: Some((provider, signature)),
                ..
            }
            | InputBlock::ToolCall {
                signature: Some((provider, signature)),
                ..
            } = b
                && let Record::Block(b) = &mut records[i]
            {
                b.signature = Presence::Value(ProviderSignature {
                    source: protocol(*provider).into(),
                    value: text(signature),
                });
            }
            ordinal += 1;
        }
        if records.len() == start + 1
            && target == UpstreamProtocol::Chat
            && m.role != Role::Assistant
        {
            let end = records.len() + 1;
            records.push(block(Content::Text(text("")), end));
        }
        let end = records.len();
        if let Record::Message(m) = &mut records[start] {
            m.end = end;
        }
    }
    let source = match request.client {
        super::ClientProtocol::Chat => UpstreamProtocol::Chat,
        super::ClientProtocol::Responses => UpstreamProtocol::Responses,
        super::ClientProtocol::Anthropic => UpstreamProtocol::Anthropic,
        super::ClientProtocol::Gemini => UpstreamProtocol::Gemini,
    };
    for (t, schema) in request.tools.iter().zip(&schemas) {
        // Responses' service-selected implicit strict mode is a route policy,
        // not something the converter can infer from a missing field.
        let strict =
            if source == UpstreamProtocol::Responses && target != source && t.strict.is_none() {
                Some(false)
            } else {
                t.strict
            };
        records.push(Record::Tool(Tool {
            name: text(&t.name),
            description: present(t.description.as_deref()),
            parameters: Presence::Value(raw(schema, &mut tokens)?),
            schema_dialect: SchemaDialect::JsonSchema,
            strict: strict.map(Presence::Value).unwrap_or(Presence::Missing),
            extensions: Span::EMPTY,
            legacy: false,
        }));
    }
    let mut source_format: zconv::Format = protocol(source).into();
    let mut target_format: zconv::Format = protocol(target).into();
    if target == UpstreamProtocol::Chat
        && request
            .messages
            .iter()
            .flat_map(|m| &m.content)
            .any(|b| matches!(b, InputBlock::Thinking { .. }))
    {
        target_format.profile = zconv::Profile::OpenAiCompatible("core-reasoning");
        if source == target {
            source_format = target_format;
        }
    }
    let max_tokens = request.max_output_tokens.to_string();
    let stream = request.stream.to_string();
    let temperature = request.temperature.map(|n| n.to_string());
    let top_p = request.top_p.map(|n| n.to_string());
    // Core's Anthropic contract ignores concurrency when no tools are present.
    let parallel = request
        .parallel_tool_calls
        .filter(|_| target != UpstreamProtocol::Anthropic || !request.tools.is_empty())
        .map(|v| v.to_string());
    let parameters = Parameters {
        max_output_tokens: Presence::Value(raw(&max_tokens, &mut tokens)?),
        stream: Presence::Value(raw(&stream, &mut tokens)?),
        temperature: temperature
            .as_deref()
            .map(|s| raw(s, &mut tokens))
            .transpose()?
            .map(Presence::Value)
            .unwrap_or(Presence::Missing),
        top_p: top_p
            .as_deref()
            .map(|s| raw(s, &mut tokens))
            .transpose()?
            .map(Presence::Value)
            .unwrap_or(Presence::Missing),
        stop: if request.stop.is_empty() {
            Presence::Missing
        } else {
            Presence::Value(raw(&stop, &mut tokens)?)
        },
        n: if target == UpstreamProtocol::Gemini {
            Presence::Value(raw("1", &mut tokens)?)
        } else {
            Presence::Missing
        },
        reasoning_effort: present(request.reasoning_effort.as_deref()),
        parallel_tool_calls: parallel
            .as_deref()
            .map(|s| raw(s, &mut tokens))
            .transpose()?
            .map(Presence::Value)
            .unwrap_or(Presence::Missing),
        ..Default::default()
    };
    let ir = zconv::ir::Request {
        source: source_format,
        model: Presence::Value(text(model)),
        tools: if request.tools.is_empty() {
            Presence::Missing
        } else {
            Presence::Value(())
        },
        functions: Presence::Missing,
        instruction_presence: Presence::Missing,
        parameters,
        tool_choice: if request.tools.is_empty() {
            zconv::ir::ToolChoice::Missing
        } else {
            match &request.tool_choice {
                ToolChoice::Auto => zconv::ir::ToolChoice::Auto,
                ToolChoice::None => zconv::ir::ToolChoice::None,
                ToolChoice::Required => zconv::ir::ToolChoice::Required,
                ToolChoice::Named(n) => zconv::ir::ToolChoice::Named(text(n)),
            }
        },
        context: Presence::Missing,
        output_constraint: Presence::Missing,
        extensions: Span::EMPTY,
        records: &mut records,
    };
    let mut output = vec![0; capacity];
    let mut scratch = vec![0; capacity];
    let report = zconv::protocol::encode_request(
        &ir,
        target_format,
        &Context {
            max_tokens: Some(u64::from(request.max_output_tokens)),
            ..Default::default()
        },
        Policy::default(),
        &mut output,
        &mut scratch,
        &mut tokens,
    )
    .map_err(error)?;
    serde_json::from_slice(&output[..report.written])
        .map_err(|_| RelayError::Protocol("invalid converted request"))
}

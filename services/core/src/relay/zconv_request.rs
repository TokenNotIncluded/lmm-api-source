//! Bounded request conversion only. Admission, provider profiles, transport and
//! authoritative usage stay in Core. zconv never receives credentials or prices.
use super::{RelayError, UpstreamProtocol};
use serde_json::Value;
use zconv::{
    Operation, Options, Protocol, Workspace,
    ir::{Context, Record},
    json::Token,
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
fn nodes(value: &Value) -> usize {
    1 + match value {
        Value::Array(a) => a.iter().map(nodes).sum(),
        Value::Object(o) => o.len() + o.values().map(nodes).sum::<usize>(),
        _ => 0,
    }
}
fn argument_nodes(value: &Value) -> usize {
    match value {
        Value::Array(a) => a.iter().map(argument_nodes).max().unwrap_or(0),
        Value::Object(o) => o
            .iter()
            .map(|(key, v)| {
                if key == "arguments" {
                    v.as_str()
                        .and_then(|s| serde_json::from_str::<Value>(s).ok())
                        .as_ref()
                        .map(nodes)
                        .unwrap_or(0)
                } else {
                    argument_nodes(v)
                }
            })
            .max()
            .unwrap_or(0),
        _ => 0,
    }
}
// Core's target image validation has already rejected non-auto detail for
// native targets. Explicit auto is equivalent to their only supported default.
fn native_image_defaults(value: &mut Value) {
    // Only actual Chat message image blocks, never schemas/defaults/examples
    // or arbitrary tool arguments with similarly named business fields.
    if let Some(messages) = value.get_mut("messages").and_then(Value::as_array_mut) {
        for message in messages {
            if let Some(parts) = message.get_mut("content").and_then(Value::as_array_mut) {
                for part in parts {
                    if part.get("type").and_then(Value::as_str) == Some("image_url") {
                        if let Some(image) =
                            part.get_mut("image_url").and_then(Value::as_object_mut)
                        {
                            if image.get("detail").and_then(Value::as_str) == Some("auto") {
                                image.remove("detail");
                            }
                        }
                    }
                }
            }
        }
    }
}

fn error(e: zconv::Error) -> RelayError {
    #[cfg(test)]
    eprintln!("ZCONV_ERROR {e:?}");
    match e {
        zconv::Error::InputTooLarge { .. }
        | zconv::Error::WorkspaceTooSmall { .. }
        | zconv::Error::IrTooSmall { .. }
        | zconv::Error::OutputTooSmall { .. }
        | zconv::Error::DepthLimitExceeded { .. }
        | zconv::Error::LimitExceeded { .. } => RelayError::Limit("protocol conversion workspace"),
        _ => RelayError::Unsupported("request cannot be converted without semantic loss"),
    }
}
pub(super) fn convert(
    source: UpstreamProtocol,
    target: UpstreamProtocol,
    mut input: Value,
    model: &str,
    max_tokens: u32,
    record_capacity: usize,
) -> Result<Value, RelayError> {
    // Added by Core's serializer, not user extensions: retention and usage-tail
    // controls are reattached by Core only on the applicable OpenAI target.
    let object = input
        .as_object_mut()
        .ok_or(RelayError::Invalid("conversion request"))?;
    object.remove("store");
    object.remove("stream_options");
    // Core's Anthropic encoder emits [] when no system instruction exists.
    // Do not turn that absence into an extra empty Chat system message.
    if source == UpstreamProtocol::Anthropic
        && object.get("system").and_then(Value::as_array).is_some_and(Vec::is_empty)
    {
        object.remove("system");
    }
    if matches!(
        target,
        UpstreamProtocol::Anthropic | UpstreamProtocol::Gemini
    ) {
        native_image_defaults(&mut input);
    }
    let count = nodes(&input) + 8;
    let nested_count = count.max(argument_nodes(&input) + 8);
    if count > MAX_TOKENS || nested_count > MAX_TOKENS || record_capacity > MAX_TOKENS {
        return Err(RelayError::Limit("protocol conversion workspace"));
    }
    let bytes =
        serde_json::to_vec(&input).map_err(|_| RelayError::Invalid("conversion request"))?;
    if bytes.len() > MAX_BYTES {
        return Err(RelayError::Limit("protocol conversion input"));
    }
    let capacity = bytes
        .len()
        .saturating_mul(4)
        .saturating_add(4096)
        .min(MAX_BYTES);
    let mut output = vec![0; capacity];
    let mut tokens = vec![Token::EMPTY; count];
    let mut nested = vec![Token::EMPTY; nested_count];
    let mut records = vec![Record::EMPTY; record_capacity];
    let mut scratch = vec![0; capacity];
    let options = Options {
        context: Context {
            model: Some(model),
            max_tokens: Some(u64::from(max_tokens)),
            ..Default::default()
        },
        ..Default::default()
    };
    let converted = zconv::convert(
        protocol(source).into(),
        protocol(target).into(),
        Operation::Request,
        &bytes,
        &mut output,
        options,
        Workspace {
            tokens: &mut tokens,
            records: &mut records,
            scratch: &mut scratch,
            nested_tokens: &mut nested,
        },
    )
    .map_err(error)?;
    serde_json::from_slice(&output[..converted.written()])
        .map_err(|_| RelayError::Protocol("invalid converted request"))
}

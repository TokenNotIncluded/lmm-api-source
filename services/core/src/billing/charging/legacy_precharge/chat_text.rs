//! Fixed cc52 GeneralOpenAIRequest.GetTokenCountMeta + EstimateRequestToken.
//! Supports string/null message content, text-only content arrays, string/string-array
//! prompt and input, and static/dynamic function tools. Media content is rejected.
//! Tool-call arguments, reasoning and legacy `functions` are omitted by the source.
//! Tool parameters use Go fmt's decoded map representation, not raw JSON spacing.
use serde::Deserialize;
use serde_json::value::RawValue;

use super::{Error, Estimate, SourceOptions, openai_tokens, ordinary_text};

pub(crate) fn estimate_ordinary(
    options: SourceOptions,
    request: &crate::relay::Request,
    body: &[u8],
    model_bits: u64,
    effective_group_bits: u64,
    source_free_model: bool,
) -> Result<Estimate, Error> {
    if request.client != crate::relay::ClientProtocol::Chat {
        return Err(Error::Invalid);
    }
    let (text, framing) = projection(body)?;
    let prompt = if options.count_token {
        openai_tokens::count_text(&request.model, &text)?
            .checked_add(framing)
            .ok_or(Error::QuotaRange)?
    } else {
        0
    };
    ordinary_text(
        options,
        prompt,
        request.explicit_max_output_tokens,
        model_bits,
        effective_group_bits,
        source_free_model,
    )
}

#[derive(Deserialize)]
struct Chat<'a> {
    #[serde(borrow, default)]
    messages: Option<Vec<Message<'a>>>,
    #[serde(default)]
    prompt: serde_json::Value,
    #[serde(default)]
    input: serde_json::Value,
    #[serde(borrow, default)]
    tools: Option<Vec<Tool<'a>>>,
}

#[derive(Deserialize)]
struct Message<'a> {
    #[serde(default)]
    role: Option<String>,
    #[serde(default)]
    name: Option<String>,
    #[serde(default)]
    content: serde_json::Value,
    #[serde(borrow, default)]
    tools: Option<&'a RawValue>,
}

#[derive(Deserialize)]
struct Tool<'a> {
    #[serde(borrow, default)]
    function: Function<'a>,
}

#[derive(Default, Deserialize)]
struct Function<'a> {
    #[serde(default)]
    name: Option<String>,
    #[serde(default)]
    description: Option<String>,
    #[serde(borrow, default)]
    parameters: Option<&'a RawValue>,
}

fn string_input(value: &serde_json::Value, pieces: &mut Vec<String>) -> Result<(), Error> {
    match value {
        serde_json::Value::Null => (),
        serde_json::Value::String(text) => pieces.push(text.clone()),
        serde_json::Value::Array(items) => {
            // The source ignores non-string array entries.
            pieces.extend(
                items
                    .iter()
                    .filter_map(|item| item.as_str().map(str::to_owned)),
            );
        }
        _ => return Err(Error::Invalid),
    }
    Ok(())
}

fn projection(body: &[u8]) -> Result<(String, u32), Error> {
    let chat: Chat<'_> = serde_json::from_slice(body).map_err(|_| Error::Invalid)?;
    let mut pieces = Vec::new();
    string_input(&chat.prompt, &mut pieces)?;
    string_input(&chat.input, &mut pieces)?;
    let mut framing = 3_u64;
    let mut dynamic_tools = Vec::new();
    for message in chat.messages.into_iter().flatten() {
        framing += 3;
        pieces.push(message.role.unwrap_or_default());
        if let Some(raw) = message.tools {
            // Source silently ignores invalid dynamic tool declarations.
            if let Ok(tools) = serde_json::from_str::<Option<Vec<Tool<'_>>>>(raw.get()) {
                dynamic_tools.extend(tools.unwrap_or_default());
            }
        }
        if !message.content.is_null() {
            if let Some(name) = message.name {
                framing += 3;
                pieces.push(name);
            }
            match message.content {
                serde_json::Value::String(text) => pieces.push(text),
                serde_json::Value::Array(parts) => {
                    for part in parts {
                        if part.get("type").and_then(|v| v.as_str()) != Some("text") {
                            return Err(Error::Invalid);
                        }
                        if let Some(text) = part.get("text").and_then(|v| v.as_str()) {
                            pieces.push(text.to_owned());
                        }
                    }
                }
                _ => return Err(Error::Invalid),
            }
        }
    }
    append_tools(dynamic_tools, &mut pieces, &mut framing)?;
    append_tools(chat.tools.unwrap_or_default(), &mut pieces, &mut framing)?;
    Ok((
        pieces.join("\n"),
        u32::try_from(framing).map_err(|_| Error::QuotaRange)?,
    ))
}

fn append_tools(
    tools: Vec<Tool<'_>>,
    pieces: &mut Vec<String>,
    framing: &mut u64,
) -> Result<(), Error> {
    for tool in tools {
        *framing += 8;
        pieces.push(tool.function.name.unwrap_or_default());
        if let Some(description) = tool.function.description.filter(|s| !s.is_empty()) {
            pieces.push(description);
        }
        if let Some(parameters) = tool.function.parameters {
            pieces.push(go_value(parameters.get())?);
        }
    }
    Ok(())
}

// FunctionRequest.UnmarshalJSON uses UseNumber. fmt sorts map keys and emits
// unquoted string values, preserving json.Number's spelling and precision.
fn go_value(raw: &str) -> Result<String, Error> {
    if raw.starts_with('{') {
        let values: std::collections::BTreeMap<String, &RawValue> =
            serde_json::from_str(raw).map_err(|_| Error::Invalid)?;
        let fields = values
            .into_iter()
            .map(|(key, value)| Ok(format!("{key}:{}", go_value(value.get())?)))
            .collect::<Result<Vec<_>, Error>>()?;
        return Ok(format!("map[{}]", fields.join(" ")));
    }
    if raw.starts_with('[') {
        let values: Vec<&RawValue> = serde_json::from_str(raw).map_err(|_| Error::Invalid)?;
        let items = values
            .into_iter()
            .map(|v| go_value(v.get()))
            .collect::<Result<Vec<_>, _>>()?;
        return Ok(format!("[{}]", items.join(" ")));
    }
    if raw.starts_with('"') {
        return serde_json::from_str(raw).map_err(|_| Error::Invalid);
    }
    Ok(if raw == "null" { "<nil>" } else { raw }.to_owned())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn source_names_framing_and_tool_text() {
        let body = br#"{"messages":[{"role":"user","name":"","content":[{"type":"text","text":"hello"}]},{"role":"assistant","name":"ignored","content":null,"tool_calls":[{"function":{"arguments":"ignored"}}]}],"tools":[{"type":"function","function":{"name":"f","description":"desc","parameters":{ "z":9007199254740993,"a":{"type":"string"}}}}]}"#;
        assert_eq!(
            projection(body).unwrap(),
            (
                "user\n\nhello\nassistant\nf\ndesc\nmap[a:map[type:string] z:9007199254740993]"
                    .into(),
                20
            )
        );
    }

    #[test]
    fn source_count_and_ordinary_hold() {
        let body = br#"{"model":"gpt-4o","messages":[{"role":"user","content":"hello"}],"max_tokens":100}"#;
        let request = crate::relay::Request::parse(
            crate::relay::ClientProtocol::Chat,
            body,
            &crate::relay::Limits::default(),
        )
        .unwrap();
        let options = SourceOptions {
            preconsumed_tokens: 5000,
            count_token: true,
            get_media_token: false,
            get_media_token_not_stream: false,
        };
        let estimate = estimate_ordinary(
            options,
            &request,
            body,
            2_f64.to_bits(),
            0.04_f64.to_bits(),
            false,
        )
        .unwrap();
        assert_eq!(estimate.prompt_tokens, 9); // BPE(user\nhello)=3, framing=6.
        assert_eq!(estimate.completion_tokens, 100);
        assert_eq!(estimate.hold, 408);
    }

    #[test]
    fn dynamic_tools_precede_top_level_and_preserve_number_spelling() {
        let body = br#"{"prompt":["p",7],"input":"i","messages":[{"role":"system","tools":[{"function":{"name":"dynamic","parameters":{"v":1e+03}}}]}],"tools":[{"function":{"name":"static","parameters":null}}]}"#;
        assert_eq!(
            projection(body).unwrap(),
            ("p\ni\nsystem\ndynamic\nmap[v:1e+03]\nstatic".into(), 22)
        );
    }

    #[test]
    fn media_is_rejected() {
        assert_eq!(projection(br#"{"messages":[{"role":"user","content":[{"type":"image_url","image_url":"x"}]}]}"#), Err(Error::Invalid));
        assert_eq!(
            go_value(r#"{"z":1e+3, "a":[true,null,"hi"]}"#).unwrap(),
            "map[a:[true <nil> hi] z:1e+3]"
        );
    }
}

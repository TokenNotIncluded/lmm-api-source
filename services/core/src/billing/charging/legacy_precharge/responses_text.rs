//! Text portion of the fixed cc52 Responses DTO's precharge input.
//! Media must be counted separately before this projection can authorize a hold.
use serde_json::value::RawValue;

use super::{Error, Estimate, SourceOptions, openai_tokens, ordinary_text};

/// Estimate the fixed source's ordinary-ratio hold from the original body
/// and its validated request. The selected group ratio already includes the
/// source actor's trust discount. A tiered or per-request price must use its
/// own calculation.
pub(crate) fn estimate_ordinary(
    options: SourceOptions,
    request: &crate::relay::Request,
    body: &[u8],
    model_ratio_bits: u64,
    effective_group_ratio_bits: u64,
    source_free_model: bool,
) -> Result<Estimate, Error> {
    if request.client != crate::relay::ClientProtocol::Responses {
        return Err(Error::Invalid);
    }
    let prompt = if options.count_token {
        count(&request.model, body)?.ok_or(Error::Invalid)?
    } else {
        0
    };
    ordinary_text(
        options,
        prompt,
        request.explicit_max_output_tokens,
        model_ratio_bits,
        effective_group_ratio_bits,
        source_free_model,
    )
}

#[derive(serde::Deserialize)]
struct Request<'a> {
    #[serde(borrow, default, deserialize_with = "present_raw")]
    input: Option<&'a RawValue>,
    #[serde(borrow, default, deserialize_with = "present_raw")]
    instructions: Option<&'a RawValue>,
    #[serde(borrow, default, deserialize_with = "present_raw")]
    metadata: Option<&'a RawValue>,
    #[serde(borrow, default, deserialize_with = "present_raw")]
    text: Option<&'a RawValue>,
    #[serde(borrow, default, deserialize_with = "present_raw")]
    tool_choice: Option<&'a RawValue>,
    #[serde(borrow, default, deserialize_with = "present_raw")]
    prompt: Option<&'a RawValue>,
    #[serde(borrow, default, deserialize_with = "present_raw")]
    tools: Option<&'a RawValue>,
}

#[derive(serde::Deserialize)]
struct Item<'a> {
    #[serde(default, rename = "type")]
    kind: Option<String>,
    #[serde(default)]
    role: Option<String>,
    #[serde(borrow, default, deserialize_with = "present_raw")]
    content: Option<&'a RawValue>,
    #[serde(default)]
    text: Option<String>,
}

#[derive(serde::Deserialize)]
struct Reasoning {
    #[serde(default)]
    summary: Option<Vec<ReasoningPart>>,
    #[serde(default)]
    content: Option<Vec<ReasoningPart>>,
}

#[derive(serde::Deserialize)]
struct ReasoningPart {
    #[serde(default)]
    text: Option<String>,
}

fn present_raw<'de, D: serde::Deserializer<'de>>(de: D) -> Result<Option<&'de RawValue>, D::Error> {
    serde::Deserialize::deserialize(de).map(Some)
}

/// Count from the original validated body, preserving the tool JSON bytes
/// that the source DTO counted. The caller handles other model estimators.
fn count(original_model: &str, body: &[u8]) -> Result<Option<u32>, Error> {
    openai_tokens::count(original_model, &combined_text(body)?)
}

fn combined_text(body: &[u8]) -> Result<String, Error> {
    let request: Request<'_> = serde_json::from_slice(body).map_err(|_| Error::Invalid)?;
    let mut pieces = Vec::new();
    let mut pending: Vec<_> = request.input.into_iter().collect();
    while let Some(raw) = pending.pop() {
        let value = raw.get();
        if value.starts_with('[') {
            let items: Vec<&RawValue> = serde_json::from_str(value).map_err(|_| Error::Invalid)?;
            pending.extend(items.into_iter().rev());
        } else if let Ok(text) = serde_json::from_str::<Option<String>>(value) {
            pieces.push(text.unwrap_or_default());
        } else if let Ok(item) = serde_json::from_str::<Item<'_>>(value) {
            let kind = item.kind.as_deref().unwrap_or_default();
            if matches!(kind, "" | "message")
                && let (Some(role), Some(content)) =
                    (item.role.filter(|role| !role.is_empty()), item.content)
            {
                pieces.push(role);
                pending.push(content);
                continue;
            }
            match kind {
                "input_text" | "output_text" => pieces.push(item.text.unwrap_or_default()),
                "input_image" | "input_file" => return Err(Error::Invalid),
                "reasoning" => {
                    if let Ok(reasoning) = serde_json::from_str::<Reasoning>(value) {
                        pieces.extend(
                            reasoning
                                .summary
                                .into_iter()
                                .flatten()
                                .chain(reasoning.content.into_iter().flatten())
                                .map(|part| part.text.unwrap_or_default()),
                        );
                    }
                }
                _ => pieces.push(normalize_raw_json(value)),
            }
        } else {
            pieces.push(normalize_raw_json(value));
        }
    }
    pieces.extend(
        [
            request.instructions,
            request.metadata,
            request.text,
            request.tool_choice,
            request.prompt,
            request.tools,
        ]
        .into_iter()
        .flatten()
        .map(|raw| normalize_raw_json(raw.get())),
    );
    Ok(pieces.join("\n"))
}

// Go decodes Unicode escapes in raw JSON without reserializing the payload.
// Semantic input strings have already been decoded by serde.
fn normalize_raw_json(raw: &str) -> String {
    let mut normalized = String::with_capacity(raw.len());
    let mut rest = raw;
    let mut quoted = false;
    while let Some(index) = rest.find(['"', '\\']) {
        normalized.push_str(&rest[..index]);
        rest = &rest[index..];
        if rest.starts_with('"') {
            quoted = !quoted;
            normalized.push('"');
            rest = &rest[1..];
            continue;
        }
        if quoted && let Some((scalar, len)) = unicode_escape(rest) {
            normalized.push(scalar);
            rest = &rest[len..];
            continue;
        }
        let len = if quoted { 2 } else { 1 };
        let Some(prefix) = rest.get(..len) else { break };
        normalized.push_str(prefix);
        rest = &rest[len..];
    }
    normalized.push_str(rest);
    normalized
}

fn unicode_escape(raw: &str) -> Option<(char, usize)> {
    let unit = |text: &str| u16::from_str_radix(text.strip_prefix("\\u")?.get(..4)?, 16).ok();
    let first = unit(raw)?;
    if let Some(scalar) = char::from_u32(u32::from(first)) {
        return Some((scalar, 6));
    }
    let second = unit(raw.get(6..)?)?;
    char::decode_utf16([first, second])
        .next()?
        .ok()
        .map(|scalar| (scalar, 12))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn estimate_uses_source_output_limit_and_source_token_count() {
        use crate::relay::{ClientProtocol, Limits, Request};
        let options = SourceOptions {
            preconsumed_tokens: 5000,
            count_token: true,
            get_media_token: false,
            get_media_token_not_stream: false,
        };
        let implicit = br#"{"model":"gpt-6.1-sol","input":"hello"}"#;
        let explicit = br#"{"model":"gpt-6.1-sol","input":"hello","max_output_tokens":100}"#;
        for (body, completion, hold) in [
            (implicit.as_slice(), 0, 400),
            (explicit.as_slice(), 100, 408),
        ] {
            let request =
                Request::parse(ClientProtocol::Responses, body, &Limits::default()).unwrap();
            let estimate = estimate_ordinary(
                options,
                &request,
                body,
                2_f64.to_bits(),
                0.04_f64.to_bits(),
                false,
            )
            .unwrap();
            assert_eq!(estimate.prompt_tokens, 1);
            assert_eq!(estimate.completion_tokens, completion);
            assert_eq!(estimate.hold, hold);
        }
    }

    #[test]
    fn source_text_preserves_raw_tool_spacing_and_excludes_encrypted_reasoning() {
        let body = br#"{
          "input": [
            {"role":"user","content":[{"type":"input_text","text":"\u4f60\u597d"}]},
            {"type":"reasoning","encrypted_content":"opaque","summary":[{"text":"visible"}]},
            {"type":"function_call_output", "call_id":"x","output":"\uD83D\uDE42"}
          ],
          "instructions":"test",
          "tool_choice":null,
          "tools":[ {"description":"\u5929\u6c14", "name":"f"} ]
        }"#;
        let projected = concat!(
            "user\n你好\nvisible\n",
            r#"{"type":"function_call_output", "call_id":"x","output":"🙂"}"#,
            "\n\"test\"\nnull\n",
            r#"[ {"description":"天气", "name":"f"} ]"#,
        );
        assert_eq!(combined_text(body).unwrap(), projected);
        assert_eq!(
            count("gpt-6.1-sol", body),
            openai_tokens::count("gpt-6.1-sol", projected)
        );
    }

    #[test]
    fn raw_unicode_normalization_leaves_escaped_literals_and_lone_surrogates() {
        assert_eq!(
            normalize_raw_json(r#""\\u4f60 \"\u4f60 \uD800""#),
            r#""\\u4f60 \"你 \uD800""#,
        );
        assert_eq!(
            combined_text(
                br#"{"input":[{"type":"input_image","image_url":"https://example.invalid"}]}"#
            ),
            Err(Error::Invalid)
        );
    }
}

//! Only a complete, reviewed provider error can authorize another dispatch.
use super::UpstreamProtocol;
use serde::Deserialize;
use serde_json::value::RawValue;

#[derive(Default, Deserialize)]
enum EnvelopeKind {
    #[default]
    #[serde(skip)]
    Absent,
    #[serde(rename = "error")]
    AnthropicError,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Envelope<'a> {
    #[serde(borrow)]
    error: &'a RawValue,
    #[serde(rename = "request_id")]
    _request_id: Option<String>,
    #[serde(default, rename = "type")]
    kind: EnvelopeKind,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct ErrorDetail {
    #[serde(rename = "message")]
    _message: String,
    #[serde(rename = "type")]
    _kind: Option<String>,
    #[serde(rename = "param")]
    _param: Option<String>,
    #[serde(rename = "status")]
    _status: Option<String>,
    #[serde(default)]
    code: serde_json::Value,
}

pub(super) fn rejection_envelope(protocol: UpstreamProtocol, bytes: &[u8]) -> bool {
    if !bytes.trim_ascii_start().starts_with(b"{") {
        return false;
    }
    let Ok(envelope) = serde_json::from_slice::<Envelope>(bytes) else {
        return false;
    };
    if matches!(envelope.kind, EnvelopeKind::AnthropicError)
        && protocol != UpstreamProtocol::Anthropic
    {
        return false;
    }
    let detail = envelope.error.get();
    if !detail.starts_with('{') {
        return false;
    }
    let Ok(error) = serde_json::from_str::<ErrorDetail>(detail) else {
        return false;
    };
    // OpenAI-compatible errors carry string codes and Gemini can send a number.
    // Objects and arrays may contain usage, output, or tool receipts.
    matches!(
        error.code,
        serde_json::Value::Null | serde_json::Value::Number(_) | serde_json::Value::String(_)
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pure_provider_errors_remain_eligible_for_configured_failover() {
        for (protocol, payload) in [
            (
                UpstreamProtocol::Chat,
                r#"{"error":{"message":"busy","type":"rate_limit_error","param":null,"code":"rate_limit_exceeded"}}"#,
            ),
            (
                UpstreamProtocol::Responses,
                r#"{"error":{"message":"busy"},"request_id":"req-1"}"#,
            ),
            (
                UpstreamProtocol::Anthropic,
                r#"{"type":"error","error":{"type":"overloaded_error","message":"busy"}}"#,
            ),
            (
                UpstreamProtocol::Gemini,
                r#"{"error":{"code":503,"status":"UNAVAILABLE","message":"busy"}}"#,
            ),
        ] {
            assert!(
                rejection_envelope(protocol, payload.as_bytes()),
                "{payload}"
            );
        }
    }

    #[test]
    fn ambiguous_errors_never_authorize_redispatch() {
        for payload in [
            r#"{"error":{"message":"failed","usage":{"input_tokens":4}}}"#,
            r#"{"error":{"message":"failed","metadata":{"tool_calls":[{}]}}}"#,
            r#"{"error":{"message":"failed","code":{"output":"partial"}}}"#,
            r#"{"error":{"message":"failed"},"request_id":{"usage":1}}"#,
            r#"{"error":{"message":"first","tool_calls":[{}]},"error":{"message":"busy"}}"#,
            r#"{"error":{"message":"failed","code":{"output":"partial"},"code":null}}"#,
            r#"{"type":"error","error":{"message":"busy"}}"#,
            r#"{"error":["busy",null,null,null,null]}"#,
            r#"[{"message":"busy"},null]"#,
        ] {
            assert!(
                !rejection_envelope(UpstreamProtocol::Chat, payload.as_bytes()),
                "{payload}"
            );
        }
    }
}

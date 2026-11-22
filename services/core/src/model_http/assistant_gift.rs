//! Strict decoding of the only promotional model tools. No general tool runner.
//! This module consumes the actual bounded provider response inside Core. It
//! never deserializes a decision from an extension or a public request.
use crate::relay::RelayError;
use serde::Deserialize;
use serde_json::{Value, json, value::RawValue};
use sha2::{Digest, Sha256};

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct GiftArguments {
    pub amount_credits: i64,
    #[serde(default)]
    pub reason: String,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct EmptyArguments {}
pub(crate) enum GiftAction {
    Prepare(GiftArguments),
    Status,
}
/// Fields are private to the model transport. A backend consumer can read the
/// decision, but Go cannot construct this provenance or choose its operation.
pub(crate) struct GiftToolCall {
    id: String,
    response_sha256: [u8; 32],
    action: GiftAction,
}
impl GiftToolCall {
    pub(crate) fn action(&self) -> &GiftAction {
        &self.action
    }
    pub(crate) fn id(&self) -> &str {
        &self.id
    }
    pub(crate) fn response_sha256(&self) -> [u8; 32] {
        self.response_sha256
    }
}
pub(super) fn tool_schemas(exclusive_cap: i64) -> Value {
    json!([
     {"type":"function","function":{"name":"prepare_new_user_gift","description":"Ask Core to check and immediately credit a new-user promotional gift. Core alone enforces verified identity, one-time eligibility, abuse controls and the exclusive amount cap. Zero means no gift.","strict":true,"parameters":{"type":"object","properties":{"amount_credits":{"type":"integer","minimum":0,"maximum":exclusive_cap.saturating_sub(1)},"reason":{"type":"string","maxLength":240}},"required":["amount_credits","reason"],"additionalProperties":false}}},
     {"type":"function","function":{"name":"get_new_user_gift_status","description":"Read this conversation owner's persisted Core gift status without creating or changing a gift.","strict":true,"parameters":{"type":"object","properties":{},"required":[],"additionalProperties":false}}}
    ])
}
/// Anthropic receives ordinary native tools. Core still applies the same strict
/// argument decoder and gift policy to every result; no public relay request is
/// downgraded and no model-supplied field gains funding authority.
pub(super) fn anthropic_tool_schemas(exclusive_cap: i64) -> Value {
    let mut schemas = tool_schemas(exclusive_cap);
    for tool in schemas.as_array_mut().expect("fixed Core gift schemas") {
        tool["function"]
            .as_object_mut()
            .expect("fixed Core gift function")
            .remove("strict");
    }
    schemas
}
/// The caller enables the fixed schemas using a Core-owned grant policy. This
/// decoder is called only after reading the original bounded response bytes.
pub(super) fn parse(
    response: &Value,
    raw_response: &[u8],
) -> Result<Option<GiftToolCall>, RelayError> {
    let choices = response
        .get("choices")
        .and_then(Value::as_array)
        .filter(|v| v.len() == 1)
        .ok_or(RelayError::SettlementPending)?;
    let choice = &choices[0];
    let Some(calls) = choice
        .pointer("/message/tool_calls")
        .filter(|v| !v.is_null())
    else {
        return Ok(None);
    };
    let calls = calls.as_array().ok_or(RelayError::SettlementPending)?;
    if calls.is_empty() {
        return Ok(None);
    }
    if calls.len() != 1
        || choice.get("finish_reason").and_then(Value::as_str) != Some("tool_calls")
        || choice
            .pointer("/message/function_call")
            .is_some_and(|v| !v.is_null())
    {
        return Err(RelayError::SettlementPending);
    }
    let call = &calls[0];
    if call.get("type").and_then(Value::as_str) != Some("function") {
        return Err(RelayError::SettlementPending);
    }
    let id = call
        .get("id")
        .and_then(Value::as_str)
        .ok_or(RelayError::SettlementPending)?;
    let args = call
        .pointer("/function/arguments")
        .and_then(Value::as_str)
        .ok_or(RelayError::SettlementPending)?;
    let name = call
        .pointer("/function/name")
        .and_then(Value::as_str)
        .ok_or(RelayError::SettlementPending)?;
    decode_call(id, name, args, raw_response).map(Some)
}

// RawValue retains the exact input object: decoding through Value first would
// erase duplicate argument keys before the strict gift argument parser sees it.
#[derive(Deserialize)]
struct AnthropicMessage {
    #[serde(rename = "type")]
    kind: String,
    role: String,
    content: Vec<AnthropicBlock>,
    stop_reason: String,
}
#[derive(Deserialize)]
struct AnthropicBlock {
    #[serde(rename = "type")]
    kind: String,
    text: Option<String>,
    id: Option<String>,
    name: Option<String>,
    input: Option<Box<RawValue>>,
}

pub(super) fn parse_anthropic(raw_response: &[u8]) -> Result<Option<GiftToolCall>, RelayError> {
    let message: AnthropicMessage =
        serde_json::from_slice(raw_response).map_err(|_| RelayError::SettlementPending)?;
    if message.kind != "message" || message.role != "assistant" {
        return Err(RelayError::SettlementPending);
    }
    let mut call = None;
    for block in message.content {
        match block.kind.as_str() {
            "text"
                if block.text.is_some()
                    && block.id.is_none()
                    && block.name.is_none()
                    && block.input.is_none() => {}
            "tool_use"
                if block.text.is_none() && call.is_none() && message.stop_reason == "tool_use" =>
            {
                call = Some(decode_call(
                    block.id.as_deref().ok_or(RelayError::SettlementPending)?,
                    block.name.as_deref().ok_or(RelayError::SettlementPending)?,
                    block
                        .input
                        .as_deref()
                        .ok_or(RelayError::SettlementPending)?
                        .get(),
                    raw_response,
                )?);
            }
            _ => return Err(RelayError::SettlementPending),
        }
    }
    if message.stop_reason == "tool_use" && call.is_none() {
        return Err(RelayError::SettlementPending);
    }
    Ok(call)
}

fn decode_call(
    id: &str,
    name: &str,
    args: &str,
    raw_response: &[u8],
) -> Result<GiftToolCall, RelayError> {
    if id.is_empty()
        || id.len() > 128
        || id.chars().any(|c| c.is_control() || c.is_whitespace())
        || args.len() > 4096
    {
        return Err(RelayError::SettlementPending);
    }
    let action = match name {
        "prepare_new_user_gift" => {
            let a: GiftArguments =
                serde_json::from_str(args).map_err(|_| RelayError::SettlementPending)?;
            if a.amount_credits < 0
                || a.amount_credits > 9_007_199_254_740_991
                || a.reason.chars().count() > 240
                || a.reason.chars().any(char::is_control)
            {
                return Err(RelayError::SettlementPending);
            }
            GiftAction::Prepare(a)
        }
        "get_new_user_gift_status" => {
            let _: EmptyArguments =
                serde_json::from_str(args).map_err(|_| RelayError::SettlementPending)?;
            GiftAction::Status
        }
        _ => return Err(RelayError::SettlementPending),
    };
    Ok(GiftToolCall {
        id: id.to_owned(),
        response_sha256: Sha256::digest(raw_response).into(),
        action,
    })
}
#[cfg(test)]
mod tests {
    use super::*;
    fn response(name: &str, args: &str) -> Value {
        json!({"choices":[{"message":{"tool_calls":[{"id":"call_1","type":"function","function":{"name":name,"arguments":args}}]},"finish_reason":"tool_calls"}]})
    }
    #[test]
    fn only_fixed_typed_calls_are_decisions() {
        assert!(matches!(
            parse(
                &response(
                    "prepare_new_user_gift",
                    r#"{"amount_credits":7,"reason":"welcome"}"#
                ),
                b"provider"
            )
            .unwrap()
            .unwrap()
            .action(),
            GiftAction::Prepare(_)
        ));
        assert!(matches!(
            parse(&response("get_new_user_gift_status", "{}"), b"provider")
                .unwrap()
                .unwrap()
                .action(),
            GiftAction::Status
        ));
        for (name, args) in [
            ("arbitrary_tool", "{}"),
            (
                "prepare_new_user_gift",
                r#"{"amount_credits":-1,"reason":"x"}"#,
            ),
            (
                "prepare_new_user_gift",
                r#"{"amount_credits":1.5,"reason":"x"}"#,
            ),
            (
                "prepare_new_user_gift",
                r#"{"amount_credits":1,"reason":"x","payer":99}"#,
            ),
            (
                "prepare_new_user_gift",
                r#"{"amount_credits":1,"amount_credits":2,"reason":"x"}"#,
            ),
            ("get_new_user_gift_status", r#"{"user_id":9}"#),
        ] {
            assert!(parse(&response(name, args), b"provider").is_err());
        }
    }
    #[test]
    fn no_parallel_legacy_or_incomplete_tools() {
        let mut value = response("get_new_user_gift_status", "{}");
        value["choices"][0]["finish_reason"] = json!("length");
        assert!(parse(&value, b"x").is_err());
        let mut value = response("get_new_user_gift_status", "{}");
        let second = value["choices"][0]["message"]["tool_calls"][0].clone();
        value["choices"][0]["message"]["tool_calls"]
            .as_array_mut()
            .unwrap()
            .push(second);
        assert!(parse(&value, b"x").is_err());
        let mut value = response("get_new_user_gift_status", "{}");
        value["choices"][0]["message"]["function_call"] = json!({"name":"legacy"});
        assert!(parse(&value, b"x").is_err());
    }

    fn anthropic(name: &str, args: &str) -> Vec<u8> {
        format!(r#"{{"id":"msg_original","type":"message","role":"assistant","content":[{{"type":"text","text":"Checking eligibility."}},{{"type":"tool_use","id":"toolu_original","name":"{name}","input":{args}}}],"stop_reason":"tool_use","usage":{{"input_tokens":10,"output_tokens":4}}}}"#).into_bytes()
    }

    #[test]
    fn anthropic_decision_keeps_original_id_raw_bytes_and_zero_amount() {
        let raw = anthropic(
            "prepare_new_user_gift",
            r#"{"amount_credits":0,"reason":"not eligible"}"#,
        );
        let call = parse_anthropic(&raw).unwrap().unwrap();
        assert_eq!(call.id(), "toolu_original");
        assert_eq!(
            call.response_sha256(),
            <[u8; 32]>::from(Sha256::digest(&raw))
        );
        assert!(matches!(
            call.action(),
            GiftAction::Prepare(GiftArguments {
                amount_credits: 0,
                ..
            })
        ));
        let status = parse_anthropic(&anthropic("get_new_user_gift_status", "{}"))
            .unwrap()
            .unwrap();
        assert!(matches!(status.action(), GiftAction::Status));
        assert!(parse_anthropic(br#"{"type":"message","role":"assistant","content":[{"type":"text","text":"Hello"}],"stop_reason":"end_turn"}"#).unwrap().is_none());
    }

    #[test]
    fn anthropic_rejects_raw_duplicate_arguments_injected_owners_and_parallel_calls() {
        for (name, args) in [
            (
                "prepare_new_user_gift",
                r#"{"amount_credits":1,"amount_credits":2,"reason":"x"}"#,
            ),
            (
                "prepare_new_user_gift",
                r#"{"amount_credits":1,"reason":"x","payer":99}"#,
            ),
            (
                "prepare_new_user_gift",
                r#"{"amount_credits":-1,"reason":"x"}"#,
            ),
            (
                "prepare_new_user_gift",
                r#"{"amount_credits":1.5,"reason":"x"}"#,
            ),
            ("get_new_user_gift_status", r#"{"user_id":9}"#),
            ("arbitrary_tool", "{}"),
        ] {
            assert!(parse_anthropic(&anthropic(name, args)).is_err());
        }
        let original: Value =
            serde_json::from_slice(&anthropic("get_new_user_gift_status", "{}")).unwrap();
        for stop in ["max_tokens", "end_turn", "refusal"] {
            let mut value = original.clone();
            value["stop_reason"] = json!(stop);
            assert!(parse_anthropic(&serde_json::to_vec(&value).unwrap()).is_err());
        }
        let mut value = original.clone();
        let second = value["content"][1].clone();
        value["content"].as_array_mut().unwrap().push(second);
        assert!(parse_anthropic(&serde_json::to_vec(&value).unwrap()).is_err());
        for kind in ["thinking", "server_tool_use", "redacted_thinking"] {
            let mut value = original.clone();
            value["content"][0]["type"] = json!(kind);
            assert!(parse_anthropic(&serde_json::to_vec(&value).unwrap()).is_err());
        }
        let mut value = original;
        value["content"] = json!([]);
        assert!(parse_anthropic(&serde_json::to_vec(&value).unwrap()).is_err());
    }
}

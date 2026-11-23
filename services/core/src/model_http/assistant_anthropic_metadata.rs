//! Assistant-only evidence for standard, global, token-priced Anthropic calls.
//! The generic relay contract remains unchanged. Untouched objects stay raw so
//! projection cannot erase duplicate tool argument keys before strict decoding.
use crate::relay::RelayError;
use serde::{
    Deserialize, Serialize,
    de::{MapAccess, Visitor},
};
use serde_json::value::RawValue;
use std::{collections::BTreeMap, fmt};

#[derive(Serialize)]
#[serde(transparent)]
struct RawObject(BTreeMap<String, Box<RawValue>>);

impl<'de> Deserialize<'de> for RawObject {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        struct ObjectVisitor;
        impl<'de> Visitor<'de> for ObjectVisitor {
            type Value = RawObject;
            fn expecting(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                f.write_str("an object with unique keys")
            }
            fn visit_map<M: MapAccess<'de>>(self, mut map: M) -> Result<RawObject, M::Error> {
                let mut fields = BTreeMap::new();
                while let Some((key, value)) = map.next_entry::<String, Box<RawValue>>()? {
                    if fields.insert(key, value).is_some() {
                        return Err(serde::de::Error::custom("duplicate assistant response key"));
                    }
                }
                Ok(RawObject(fields))
            }
        }
        deserializer.deserialize_map(ObjectVisitor)
    }
}

fn object(raw: &str) -> Result<RawObject, RelayError> {
    serde_json::from_str(raw).map_err(|_| RelayError::SettlementPending)
}
fn null(raw: &RawValue) -> bool {
    raw.get().trim() == "null"
}
fn string_is(raw: &RawValue, expected: &str) -> bool {
    serde_json::from_str::<String>(raw.get()).is_ok_and(|value| value == expected)
}
fn encoded(value: &RawObject) -> Result<Box<RawValue>, RelayError> {
    serde_json::value::to_raw_value(value).map_err(|_| RelayError::SettlementPending)
}

fn project_usage(raw: &RawValue, final_output: bool) -> Result<Box<RawValue>, RelayError> {
    let mut usage = object(raw.get())?;
    // A message_start output count is preliminary. A terminal message_delta
    // must carry its own cumulative output receipt, including an explicit zero.
    if final_output
        && !usage
            .0
            .get("output_tokens")
            .is_some_and(|value| serde_json::from_str::<u64>(value.get()).is_ok())
    {
        return Err(RelayError::SettlementPending);
    }
    if usage.0.contains_key("iterations")
        || usage
            .0
            .get("service_tier")
            .is_some_and(|v| !null(v) && !string_is(v, "standard"))
        || usage
            .0
            .get("inference_geo")
            .is_some_and(|v| !null(v) && !string_is(v, "global"))
    {
        return Err(RelayError::Unsupported(
            "assistant provider billing dimensions",
        ));
    }
    if let Some(value) = usage.0.get("server_tool_use").filter(|v| !null(v)) {
        let counts = object(value.get())?;
        if counts.0.is_empty()
            || counts.0.iter().any(|(key, value)| {
                !matches!(key.as_str(), "web_search_requests" | "web_fetch_requests")
                    || !serde_json::from_str::<u64>(value.get()).is_ok_and(|count| count == 0)
            })
        {
            return Err(RelayError::Unsupported("assistant provider tool usage"));
        }
    }
    for key in ["service_tier", "inference_geo", "server_tool_use"] {
        usage.0.remove(key);
    }
    encoded(&usage)
}

/// Project only validated non-billable metadata for the existing decoder.
/// Callers retain original response bytes and read original token usage first.
pub(super) fn project(raw: &str) -> Result<String, RelayError> {
    let mut response = object(raw)?;
    let is_delta = response
        .0
        .get("type")
        .is_some_and(|v| string_is(v, "message_delta"));
    let final_output = if is_delta {
        let delta = object(
            response
                .0
                .get("delta")
                .ok_or(RelayError::SettlementPending)?
                .get(),
        )?;
        delta.0.get("stop_reason").is_some_and(|v| !null(v))
    } else {
        false
    };
    if let Some(value) = response.0.get_mut("usage") {
        *value = project_usage(value, final_output)?;
    } else if final_output {
        return Err(RelayError::SettlementPending);
    }
    if let Some(value) = response.0.get_mut("message") {
        let mut message = object(value.get())?;
        if let Some(usage) = message.0.get_mut("usage") {
            *usage = project_usage(usage, false)?;
        }
        *value = encoded(&message)?;
    }
    serde_json::to_string(&response).map_err(|_| RelayError::SettlementPending)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::{Value, json};

    fn receipt(extra: Value) -> String {
        let mut value = json!({"type":"message","role":"assistant","content":[],"usage":{
            "input_tokens":11,"output_tokens":5,"cache_read_input_tokens":7,
            "cache_creation_input_tokens":3,"cache_creation":{
                "ephemeral_5m_input_tokens":2,"ephemeral_1h_input_tokens":1}}});
        value["usage"]
            .as_object_mut()
            .unwrap()
            .extend(extra.as_object().unwrap().clone());
        serde_json::to_string(&value).unwrap()
    }

    #[test]
    fn only_standard_global_zero_tool_metadata_is_projected() {
        let original = receipt(json!({"service_tier":"standard","inference_geo":"global",
            "server_tool_use":{"web_search_requests":0,"web_fetch_requests":0}}));
        let original_value: Value = serde_json::from_str(&original).unwrap();
        let projected: Value = serde_json::from_str(&project(&original).unwrap()).unwrap();
        let mut expected = original_value;
        for key in ["service_tier", "inference_geo", "server_tool_use"] {
            expected["usage"].as_object_mut().unwrap().remove(key);
        }
        assert_eq!(projected, expected);
        assert!(
            project(&receipt(
                json!({"service_tier":null,"inference_geo":null,"server_tool_use":null})
            ))
            .is_ok()
        );
        for extra in [
            json!({"service_tier":"priority"}),
            json!({"service_tier":"batch"}),
            json!({"service_tier":0}),
            json!({"inference_geo":"us"}),
            json!({"inference_geo":"unknown"}),
            json!({"iterations":null}),
            json!({"server_tool_use":{"web_search_requests":1}}),
            json!({"server_tool_use":{"web_search_requests":-1}}),
            json!({"server_tool_use":{"web_search_requests":0.0}}),
            json!({"server_tool_use":{"web_search_requests":false}}),
            json!({"server_tool_use":{"new_billable_counter":0}}),
            json!({"server_tool_use":{}}),
            json!({"server_tool_use":[]}),
        ] {
            assert!(project(&receipt(extra.clone())).is_err(), "{extra}");
        }
    }

    #[test]
    fn duplicate_envelope_and_usage_keys_are_not_erased() {
        for raw in [
            r#"{"type":"message","type":"message","usage":{}}"#,
            r#"{"usage":{},"usage":{}}"#,
            r#"{"message":{"usage":{},"usage":{}}}"#,
            r#"{"usage":{"input_tokens":1,"input_tokens":2}}"#,
            r#"{"usage":{"service_tier":null,"service_tier":"standard"}}"#,
            r#"{"usage":{"service_tier":"standard","service_\u0074ier":"priority"}}"#,
            r#"{"usage":{"server_tool_use":{"web_search_requests":0,"web_search_requests":1}}}"#,
        ] {
            assert!(project(raw).is_err(), "{raw}");
        }
    }

    #[test]
    fn content_and_duplicate_argument_bytes_survive_projection() {
        let content = r#"[{ "type":"tool_use", "id":"toolu_raw", "name":"prepare_new_user_gift", "input": {"amount_credits":1, "amount_credits":2, "reason":"x"} }]"#;
        let raw = format!(
            r#"{{"type":"message","role":"assistant","content":{content},"stop_reason":"tool_use","usage":{{"input_tokens":1,"output_tokens":2,"service_tier":"standard"}}}}"#
        );
        let projected = project(&raw).unwrap();
        let result = object(&projected).unwrap();
        assert_eq!(result.0["content"].get(), content);
        assert!(crate::model_http::assistant_gift::parse_anthropic(raw.as_bytes()).is_err());
        assert!(crate::model_http::assistant_gift::parse_anthropic(projected.as_bytes()).is_err());
    }

    #[test]
    fn terminal_delta_needs_its_own_cumulative_output_receipt() {
        let base =
            json!({"type":"message_delta","delta":{"stop_reason":"end_turn","stop_sequence":null}});
        assert!(project(&base.to_string()).is_err());
        for usage in [
            Value::Null,
            json!({}),
            json!({"output_tokens":null}),
            json!({"output_tokens":-1}),
            json!({"output_tokens":1.0}),
            json!({"output_tokens":"5"}),
        ] {
            let mut value = base.clone();
            value["usage"] = usage;
            assert!(project(&value.to_string()).is_err());
        }
        for output in [0, 5] {
            let mut value = base.clone();
            value["usage"] = json!({"output_tokens":output});
            assert!(project(&value.to_string()).is_ok());
        }
    }
}

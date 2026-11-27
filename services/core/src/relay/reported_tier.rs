//! Verify OpenAI's reported processing grade against the dispatched, priced attempt.
use super::RelayError;
use crate::routing::Speed;
use serde_json::Value;

pub(crate) fn validate_openai_reported_tier(
    expected: Speed,
    reported: Option<&Value>,
) -> Result<(), RelayError> {
    let Some(value) = reported.filter(|value| !value.is_null()) else {
        return Ok(());
    };
    let actual = match value.as_str() {
        Some("default" | "standard") => Speed::Standard,
        Some("priority" | "fast") => Speed::Fast,
        Some("ultrafast") => Speed::Ultrafast,
        _ => {
            return Err(RelayError::Protocol(
                "unrecognized OpenAI service tier receipt",
            ));
        }
    };
    if actual != expected {
        return Err(RelayError::Protocol(
            "OpenAI service tier differs from selected speed",
        ));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn openai_tier_receipts_match_the_frozen_attempt() {
        for (tier, actual) in [
            ("default", Speed::Standard),
            ("standard", Speed::Standard),
            ("priority", Speed::Fast),
            ("fast", Speed::Fast),
            ("ultrafast", Speed::Ultrafast),
        ] {
            for expected in [Speed::Standard, Speed::Fast, Speed::Ultrafast] {
                assert_eq!(
                    validate_openai_reported_tier(expected, Some(&json!(tier))).is_ok(),
                    expected == actual,
                    "receipt {tier} for {expected:?}"
                );
            }
        }
        assert!(validate_openai_reported_tier(Speed::Fast, None).is_ok());
        assert!(validate_openai_reported_tier(Speed::Fast, Some(&Value::Null)).is_ok());
        for value in [json!("auto"), json!("flex"), json!("unpriced"), json!(1)] {
            assert!(validate_openai_reported_tier(Speed::Fast, Some(&value)).is_err());
        }
    }

    #[test]
    fn mismatched_json_and_stream_receipts_never_complete_the_openai_decoder() {
        use crate::relay::{Limits, UpstreamProtocol, decode::Decoder, sse::Frame};
        let usage = json!({"prompt_tokens": 3, "completion_tokens": 2, "total_tokens": 5});
        let chat = json!({
            "id":"one","object":"chat.completion","service_tier":"default",
            "choices":[{"index":0,"message":{"role":"assistant","content":"hello"},"finish_reason":"stop"}],
            "usage":usage
        });
        let response = json!({
            "id":"two","object":"response","status":"completed","output":[],"service_tier":"default",
            "usage":{"input_tokens":3,"output_tokens":2,"total_tokens":5}
        });
        for (protocol, json_body, stream_event) in [
            (
                UpstreamProtocol::Chat,
                chat,
                json!({
                    "id":"one","object":"chat.completion.chunk","choices":[],"usage":usage,"service_tier":"default"
                }),
            ),
            (
                UpstreamProtocol::Responses,
                response.clone(),
                json!({"type":"response.completed","response":response}),
            ),
        ] {
            let expected_error =
                RelayError::Protocol("OpenAI service tier differs from selected speed");
            let mut decoder =
                Decoder::new(protocol, &Limits::default()).with_expected_speed(Some(Speed::Fast));
            assert_eq!(decoder.json(&json_body).unwrap_err(), expected_error);
            assert!(!decoder.is_done());
            assert_eq!(
                decoder
                    .frame(Frame {
                        event: None,
                        data: stream_event.to_string(),
                    })
                    .unwrap_err(),
                expected_error
            );
            assert!(!decoder.is_done());
        }
    }
}

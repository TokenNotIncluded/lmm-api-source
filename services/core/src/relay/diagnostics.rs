//! Bounded operator diagnostics. Provider text, headers and wire bytes are never stored.
use super::{ClientProtocol, Completion, Phase, RelayError, UpstreamProtocol};
use serde::{Deserialize, Serialize};

/// Only the enum discriminant crosses the converter boundary.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) enum ConverterError {
    InvalidJson,
    DuplicateKey,
    DepthLimitExceeded,
    InputTooLarge,
    WorkspaceTooSmall,
    ScratchTooSmall,
    OutputTooSmall,
    IrTooSmall,
    LimitExceeded,
    InvalidInput,
    Unsupported,
    ContextRequired,
    DirectUnavailable,
    PolicyViolation,
    EventTooLarge,
    TooManyActiveItems,
    TruncatedStream,
    AlreadyFinished,
    Other,
}

impl From<zconv::Error> for ConverterError {
    fn from(error: zconv::Error) -> Self {
        use zconv::Error as E;
        match error {
            E::InvalidJson { .. } => Self::InvalidJson,
            E::DuplicateKey { .. } => Self::DuplicateKey,
            E::DepthLimitExceeded { .. } => Self::DepthLimitExceeded,
            E::InputTooLarge { .. } => Self::InputTooLarge,
            E::WorkspaceTooSmall { .. } => Self::WorkspaceTooSmall,
            E::ScratchTooSmall { .. } => Self::ScratchTooSmall,
            E::OutputTooSmall { .. } => Self::OutputTooSmall,
            E::IrTooSmall { .. } => Self::IrTooSmall,
            E::LimitExceeded { .. } => Self::LimitExceeded,
            E::InvalidInput { .. } => Self::InvalidInput,
            E::Unsupported { .. } => Self::Unsupported,
            E::ContextRequired { .. } => Self::ContextRequired,
            E::DirectUnavailable { .. } => Self::DirectUnavailable,
            E::PolicyViolation { .. } => Self::PolicyViolation,
            E::EventTooLarge { .. } => Self::EventTooLarge,
            E::TooManyActiveItems { .. } => Self::TooManyActiveItems,
            E::TruncatedStream => Self::TruncatedStream,
            E::AlreadyFinished => Self::AlreadyFinished,
            _ => Self::Other,
        }
    }
}

pub(crate) fn safe_event(name: Option<&str>) -> &'static str {
    match name.unwrap_or("message") {
        "message" => "message",
        "error" => "error",
        "ping" => "ping",
        "message_start" => "message_start",
        "message_delta" => "message_delta",
        "message_stop" => "message_stop",
        "content_block_start" => "content_block_start",
        "content_block_delta" => "content_block_delta",
        "content_block_stop" => "content_block_stop",
        "response.created" => "response.created",
        "response.in_progress" => "response.in_progress",
        "response.completed" => "response.completed",
        "response.failed" => "response.failed",
        "response.incomplete" => "response.incomplete",
        "response.output_item.added" => "response.output_item.added",
        "response.output_item.done" => "response.output_item.done",
        "response.content_part.added" => "response.content_part.added",
        "response.content_part.done" => "response.content_part.done",
        "response.output_text.delta" => "response.output_text.delta",
        "response.output_text.done" => "response.output_text.done",
        "response.function_call_arguments.delta" => "response.function_call_arguments.delta",
        "response.function_call_arguments.done" => "response.function_call_arguments.done",
        _ => "other",
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct StreamFrame {
    pub(crate) channel_id: Option<u64>,
    pub(crate) upstream_protocol: UpstreamProtocol,
    pub(crate) ordinal: u64,
    pub(crate) event: String,
    pub(crate) converter: Option<ConverterError>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Observation {
    pub(crate) inbound_protocol: ClientProtocol,
    pub(crate) error_code: Option<String>,
    pub(crate) error_stage: Option<String>,
    pub(crate) sse_frame: Option<StreamFrame>,
}

impl Observation {
    pub(crate) fn completed(client: ClientProtocol, completion: &Completion) -> Self {
        let error = completion.error.as_ref();
        let frame = completion.report.stream_frame.clone();
        let stage = if frame.is_some() {
            Some("sse_decode")
        } else {
            match error {
                Some(RelayError::Timeout(phase)) => Some(match phase {
                    Phase::Route => "route",
                    Phase::Reserve => "reserve",
                    Phase::Connect => "connect",
                    Phase::Headers => "headers",
                    Phase::Read => "read",
                    Phase::Write => "write",
                    Phase::Total => "total",
                    Phase::Settle => "settle",
                }),
                Some(RelayError::UpstreamStatus(_) | RelayError::MappedUpstreamStatus(_)) => {
                    Some("upstream_response")
                }
                Some(RelayError::UpstreamConnection) => Some("upstream_transport"),
                Some(RelayError::SettlementPending) => Some("settle"),
                _ => None,
            }
        };
        Self {
            inbound_protocol: client,
            error_code: error.map(|e| e.code().to_owned()),
            error_stage: stage.map(str::to_owned),
            sse_frame: frame,
        }
    }

    pub(crate) fn valid(&self) -> bool {
        self.error_code.as_deref().is_none_or(|code| {
            matches!(
                code,
                "invalid_request"
                    | "unsupported_feature"
                    | "invalid_upstream_response"
                    | "upstream_truncated"
                    | "relay_limit_exceeded"
                    | "relay_timeout"
                    | "request_cancelled"
                    | "relay_draining"
                    | "relay_busy"
                    | "rate_limit_exceeded"
                    | "model_not_found"
                    | "route_unavailable"
                    | "billing_unavailable"
                    | "unauthorized"
                    | "legacy_key_revoked_recreate_required"
                    | "forbidden"
                    | "insufficient_funds"
                    | "request_conflict"
                    | "upstream_error"
                    | "upstream_connection_error"
                    | "settlement_pending"
            )
        }) && self.error_stage.as_deref().is_none_or(|stage| {
            matches!(
                stage,
                "sse_decode"
                    | "route"
                    | "reserve"
                    | "connect"
                    | "headers"
                    | "read"
                    | "write"
                    | "total"
                    | "settle"
                    | "upstream_response"
                    | "upstream_transport"
            )
        }) && self.sse_frame.as_ref().is_none_or(|frame| {
            frame.ordinal > 0
                && safe_event(Some(&frame.event)) == frame.event
                && self.error_stage.as_deref() == Some("sse_decode")
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn provider_fields_never_enter_the_event_or_converter_labels() {
        assert_eq!(
            safe_event(Some("response.output_text.delta")),
            "response.output_text.delta"
        );
        assert_eq!(safe_event(None), "message");
        assert_eq!(
            safe_event(Some("https://private.invalid/?key=secret")),
            "other"
        );
        let kind = ConverterError::from(zconv::Error::InvalidInput {
            field: "synthetic-secret",
            reason: "synthetic-prompt",
        });
        assert_eq!(serde_json::to_value(kind).unwrap(), "InvalidInput");
    }
}

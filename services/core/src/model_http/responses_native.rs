//! Native compaction and opaque-item replay. Ciphertext is never a token estimate.
use super::input_api::contract::Kind;
use super::*;
use serde::{Deserialize, Serialize};

#[derive(Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub(super) struct Request {
    model: String,
    input: serde_json::Value,
    #[serde(skip_serializing_if = "Option::is_none")]
    instructions: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    max_output_tokens: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    store: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    stream: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    tools: Option<serde_json::Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    tool_choice: Option<serde_json::Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    parallel_tool_calls: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    reasoning: Option<serde_json::Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    text: Option<serde_json::Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    include: Option<Vec<String>>,
    #[serde(skip)]
    compact: bool,
}

pub(super) fn handles(raw: &[u8]) -> bool {
    serde_json::from_slice::<serde_json::Value>(raw)
        .ok()
        .is_some_and(|v| {
            v["input"].as_array().is_some_and(|items| {
                items.iter().any(|item| {
                    item["type"] == "compaction" || item.get("encrypted_content").is_some()
                })
            })
        })
}
impl Request {
    pub(super) fn parse(kind: Kind, raw: &[u8]) -> Result<Self, RelayError> {
        let mut r: Self = serde_json::from_slice(raw)
            .map_err(|_| RelayError::Invalid("invalid native Responses request"))?;
        r.compact = kind == Kind::ResponsesCompact;
        if r.store == Some(true) {
            return Err(RelayError::Unsupported(
                "native compact context requires store:false",
            ));
        }
        if r.compact
            && (r.max_output_tokens.is_some()
                || r.store.is_some()
                || r.stream.is_some()
                || r.tools.is_some()
                || r.tool_choice.is_some()
                || r.parallel_tool_calls.is_some()
                || r.reasoning.is_some()
                || r.text.is_some()
                || r.include.is_some())
        {
            return Err(RelayError::Invalid("unsupported compaction controls"));
        }
        if r.max_output_tokens == Some(0)
            || !(r.input.as_str().is_some_and(|s| !s.is_empty())
                || r.input.as_array().is_some_and(|a| !a.is_empty()))
        {
            return Err(RelayError::Invalid("native Responses input required"));
        }
        if let Some(items) = r.input.as_array() {
            for item in items {
                let kind = item["type"].as_str();
                if matches!(kind, Some("compaction" | "reasoning")) {
                    if !item["encrypted_content"]
                        .as_str()
                        .is_some_and(|s| !s.is_empty())
                    {
                        return Err(RelayError::Invalid(
                            "opaque Responses item requires encrypted_content",
                        ));
                    }
                } else if matches!(kind, Some("function_call" | "function_call_output")) {
                    if !item["call_id"].as_str().is_some_and(|s| !s.is_empty())
                        || (kind == Some("function_call_output") && !item["output"].is_string())
                        || (kind == Some("function_call")
                            && (!item["name"].is_string()
                                || !item["arguments"].as_str().is_some_and(|args| {
                                    serde_json::from_str::<serde_json::Value>(args)
                                        .is_ok_and(|v| v.is_object())
                                })))
                    {
                        return Err(RelayError::Invalid("invalid compact function item"));
                    }
                } else {
                    validate_message(item)?;
                }
            }
        }
        if r.include
            .as_ref()
            .is_some_and(|v| v.iter().any(|s| s != "reasoning.encrypted_content"))
        {
            return Err(RelayError::Unsupported(
                "unsupported native Responses include",
            ));
        }
        // Reuse normal function/schema/reasoning validation without interpreting opaque input as text.
        let mut validation =
            serde_json::to_value(&r).map_err(|_| RelayError::Invalid("native controls"))?;
        validation.as_object_mut().unwrap().remove("include");
        validation["input"] = "validate native controls".into();
        validation["store"] = false.into();
        validation["stream"] = false.into();
        validation["max_output_tokens"] = 1.into();
        relay::Request::parse_deferred_openai(
            relay::ClientProtocol::Responses,
            &serde_json::to_vec(&validation).map_err(|_| RelayError::Invalid("native controls"))?,
            &Limits::default(),
        )?;
        Ok(r)
    }
    pub(super) fn streaming(&self) -> bool {
        self.stream == Some(true)
    }
    pub(super) fn model(&self) -> &str {
        &self.model
    }
    pub(super) fn output_limit(&self, ceiling: u32) -> Result<u32, RelayError> {
        let limit = self.max_output_tokens.unwrap_or(ceiling);
        if limit == 0 || limit > ceiling {
            return Err(RelayError::Limit("native Responses output bound"));
        }
        Ok(limit)
    }
    pub(super) fn wire(&self, model: &str) -> Result<Vec<u8>, RelayError> {
        let mut v = serde_json::to_value(self)
            .map_err(|_| RelayError::Invalid("native Responses encoding"))?;
        v["model"] = model.into();
        if !self.compact {
            v["store"] = false.into();
            v["stream"] = self.streaming().into();
        }
        serde_json::to_vec(&v).map_err(|_| RelayError::Invalid("native Responses encoding"))
    }
    pub(super) fn response(
        &self,
        raw: &[u8],
    ) -> Result<(serde_json::Value, relay::Usage), RelayError> {
        let invalid = || RelayError::Protocol("invalid native Responses terminal receipt");
        let v: serde_json::Value = serde_json::from_slice(raw).map_err(|_| invalid())?;
        if v.get("error").is_some_and(|e| !e.is_null())
            || !v["id"].as_str().is_some_and(|id| !id.is_empty())
            || !v["output"].is_array()
        {
            return Err(invalid());
        }
        if self.compact {
            if v["object"] != "response.compaction"
                || !v["output"].as_array().unwrap().iter().any(|item| {
                    item["type"] == "compaction"
                        && item["encrypted_content"]
                            .as_str()
                            .is_some_and(|s| !s.is_empty())
                })
            {
                return Err(invalid());
            }
        } else if v["object"] != "response"
            || !matches!(v["status"].as_str(), Some("completed" | "incomplete"))
        {
            return Err(invalid());
        }
        for item in v["output"].as_array().unwrap() {
            match item["type"].as_str() {
                Some("compaction" | "reasoning")
                    if item["encrypted_content"]
                        .as_str()
                        .is_some_and(|s| !s.is_empty()) =>
                {
                    ()
                }
                Some("reasoning") if item["summary"].is_array() => (),
                Some("function_call")
                    if item["call_id"].is_string()
                        && item["name"].is_string()
                        && item["arguments"].as_str().is_some_and(|s| {
                            serde_json::from_str::<serde_json::Value>(s)
                                .is_ok_and(|v| v.is_object())
                        }) =>
                {
                    ()
                }
                _ => validate_message(item).map_err(|_| invalid())?,
            }
        }
        let usage =
            relay::native_response_usage(relay::UpstreamProtocol::Responses, &v["usage"], None)?;
        let (Some(input), Some(output)) = (usage.input_tokens, usage.output_tokens) else {
            return Err(invalid());
        };
        if v["usage"]["total_tokens"].as_u64() != input.checked_add(output) {
            return Err(invalid());
        }
        usage.validate()?;
        Ok((v, usage))
    }
}

fn validate_message(item: &serde_json::Value) -> Result<(), RelayError> {
    let invalid =
        || RelayError::Unsupported("compact context supports text, opaque and function items only");
    if item["type"].as_str().is_some_and(|s| s != "message")
        || !matches!(
            item["role"].as_str(),
            Some("user" | "assistant" | "system" | "developer")
        )
    {
        return Err(invalid());
    }
    match &item["content"] {
        serde_json::Value::String(_) => Ok(()),
        serde_json::Value::Array(parts)
            if parts.iter().all(|part| {
                matches!(part["type"].as_str(), Some("input_text" | "output_text"))
                    && part["text"].is_string()
                    && part.as_object().is_some_and(|o| {
                        o.keys()
                            .all(|k| matches!(k.as_str(), "type" | "text" | "annotations"))
                    })
            }) =>
        {
            Ok(())
        }
        _ => Err(invalid()),
    }
}

pub(super) type Sender = tokio::sync::mpsc::Sender<Result<Bytes, RelayError>>;
fn encode_event(event: &serde_json::Value) -> Result<Bytes, RelayError> {
    let name = event["type"]
        .as_str()
        .ok_or(RelayError::Protocol("missing native SSE event type"))?;
    if !name
        .bytes()
        .all(|b| b.is_ascii_alphanumeric() || b"._".contains(&b))
    {
        return Err(RelayError::Protocol("invalid native SSE event type"));
    }
    Ok(Bytes::from(format!("event: {name}\ndata: {event}\n\n")))
}
pub(super) async fn send_packet(
    sender: &Sender,
    packet: Bytes,
    limits: &Limits,
) -> Result<(), RelayError> {
    tokio::time::timeout(limits.write_timeout, sender.send(Ok(packet)))
        .await
        .map_err(|_| RelayError::Cancelled)?
        .map_err(|_| RelayError::Cancelled)
}

pub(super) async fn read_stream(
    gateway: &ModelGateway,
    id: &str,
    connection: &crate::relay::provider_network::ProviderConnection,
    mut response: reqwest::Response,
    sender: &Sender,
    credential: &str,
) -> Result<(Vec<u8>, Bytes, u64), RelayError> {
    if !connection.matches_peer(response.remote_addr())
        || response.headers().contains_key(header::CONTENT_ENCODING)
        || !response
            .headers()
            .get(header::CONTENT_TYPE)
            .and_then(|v| v.to_str().ok())
            .is_some_and(|v| v.split(';').next() == Some("text/event-stream"))
    {
        return Err(RelayError::SettlementPending);
    }
    let mut parser = relay::AssistantSseParser::new(gateway.limits.event_bytes);
    let mut terminal = None;
    let mut response_id: Option<String> = None;
    let mut bytes = 0usize;
    let mut sequence = 1u64;
    while let Some(chunk) = tokio::time::timeout(gateway.limits.read_timeout, response.chunk())
        .await
        .map_err(|_| RelayError::SettlementPending)?
        .map_err(|_| RelayError::SettlementPending)?
    {
        bytes = bytes
            .checked_add(chunk.len())
            .ok_or(RelayError::Limit("native SSE bytes"))?;
        if bytes
            > gateway
                .limits
                .upstream_bytes
                .min(gateway.limits.output_bytes)
            || chunk.len() > gateway.limits.transport_chunk_bytes
        {
            return Err(RelayError::Limit("native SSE bytes"));
        }
        for byte in chunk {
            if terminal.is_some() && !matches!(byte, b'\r' | b'\n') {
                return Err(RelayError::Protocol(
                    "unexpected bytes after native SSE terminal",
                ));
            }
            if let Some(frame) = parser.feed(byte)? {
                if frame.data.contains(credential) {
                    return Err(RelayError::SettlementPending);
                }
                let event: serde_json::Value = serde_json::from_str(&frame.data)
                    .map_err(|_| RelayError::Protocol("invalid native SSE JSON"))?;
                let name = event["type"]
                    .as_str()
                    .ok_or(RelayError::Protocol("missing native SSE type"))?;
                if frame.event.as_ref().is_some_and(|e| e != name) {
                    return Err(RelayError::Protocol("native SSE event mismatch"));
                }
                if let Some(id) = event.get("response").and_then(|v| v["id"].as_str()) {
                    if response_id.as_ref().is_some_and(|old| old != id) {
                        return Err(RelayError::Protocol("native SSE response identity changed"));
                    }
                    response_id = Some(id.to_owned());
                }
                if event
                    .get("response_id")
                    .and_then(|v| v.as_str())
                    .is_some_and(|id| response_id.as_ref().is_some_and(|old| old != id))
                {
                    return Err(RelayError::Protocol(
                        "native SSE response identity mismatch",
                    ));
                }
                if matches!(name, "response.completed" | "response.incomplete") {
                    if event["response"]["status"]
                        != if name == "response.completed" {
                            "completed"
                        } else {
                            "incomplete"
                        }
                    {
                        return Err(RelayError::Protocol("native terminal mismatch"));
                    }
                    let raw = serde_json::to_vec(&event["response"])
                        .map_err(|_| RelayError::SettlementPending)?;
                    terminal = Some((raw, encode_event(&event)?, sequence));
                    continue;
                }
                if !matches!(
                    name,
                    "response.created"
                        | "response.in_progress"
                        | "response.output_item.added"
                        | "response.output_item.done"
                        | "response.content_part.added"
                        | "response.content_part.done"
                        | "response.output_text.delta"
                        | "response.output_text.done"
                        | "response.function_call_arguments.delta"
                        | "response.function_call_arguments.done"
                        | "response.reasoning_summary_part.added"
                        | "response.reasoning_summary_part.done"
                        | "response.reasoning_summary_text.delta"
                        | "response.reasoning_summary_text.done"
                ) {
                    return Err(RelayError::Unsupported("unsupported native SSE output"));
                }
                if let Some(item) = event.get("item") {
                    if !matches!(
                        item["type"].as_str(),
                        Some("message" | "function_call" | "reasoning")
                    ) {
                        return Err(RelayError::Protocol("unpriced native SSE item"));
                    }
                }
                if let Some(part) = event.get("part") {
                    if !matches!(part["type"].as_str(), Some("output_text" | "summary_text")) {
                        return Err(RelayError::Protocol("unpriced native SSE part"));
                    }
                }
                let packet = encode_event(&event)?;
                gateway
                    .bridge
                    .persist_output(
                        &relay::Reservation { id: id.into() },
                        &relay::OutputCheckpoint {
                            sequence,
                            packet_sha256: Sha256::digest(&packet).into(),
                            packet_bytes: packet.len(),
                            usage: relay::Usage::default(),
                            terminal_observed: false,
                        },
                    )
                    .await?;
                sequence = sequence
                    .checked_add(1)
                    .ok_or(RelayError::Limit("native SSE sequence"))?;
                send_packet(sender, packet, &gateway.limits).await?;
            }
        }
    }
    parser.finish()?;
    terminal.ok_or(RelayError::Truncated)
}

#[cfg(test)]
#[path = "responses_native_tests.rs"]
mod tests;

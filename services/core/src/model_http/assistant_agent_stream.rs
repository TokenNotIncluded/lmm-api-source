// Copyright (C) 2026 LIghtJUNction. AGPL-3.0-or-later.
//! One pinned, already-claimed assistant attempt. Observations have no settlement authority.
use super::*;
use crate::relay::{AssistantDecoder, AssistantEncoder, AssistantSseParser, BlockKind, Event};

impl ModelGateway {
    pub(super) async fn fetch_agent_stream(
        &self,
        p: &PreparedAssistant,
        mut publish: impl FnMut(&str) -> Result<(), RelayError> + Send,
    ) -> Result<(Vec<u8>, Value, crate::billing::charging::Usage), RelayError> {
        if !p.route.is_available() {
            return Err(RelayError::RouteUnavailable);
        }
        let limits = Limits {
            output_bytes: self.limits.output_bytes.min(131072),
            event_bytes: self.limits.event_bytes.min(131072),
            tool_argument_bytes: self.limits.tool_argument_bytes.min(32768),
            ..self.limits.clone()
        };
        // Reuse the relay's block assembly, usage merge and completion checks.
        // This non-stream encoder constructs a transcript; it sends no response
        // and performs no billing. The original worker settles its observation.
        let mut payload = p.payload.clone();
        payload["stream"] = json!(false);
        payload
            .as_object_mut()
            .ok_or(RelayError::Invalid("agent payload"))?
            .remove("stream_options");
        let request = relay::Request::parse(
            ClientProtocol::Chat,
            &serde_json::to_vec(&payload).map_err(|_| RelayError::Invalid("agent payload"))?,
            &limits,
        )?;
        let mut output = AgentOutput::new(
            &request,
            &limits,
            p.price.tiered.as_ref().and_then(|t| t.cache_creation_mode),
        );
        let response = p
            .connection
            .client()
            .post(p.route.endpoint.clone())
            .header(header::AUTHORIZATION, p.route.credential.clone())
            .json(&p.payload)
            .send()
            .await
            .map_err(|_| RelayError::SettlementPending)?;
        if !p.connection.matches_peer(response.remote_addr())
            || !response.status().is_success()
            || response
                .headers()
                .get(header::CONTENT_TYPE)
                .and_then(|h| h.to_str().ok())
                .and_then(|s| s.split(';').next())
                .map(str::trim)
                != Some("text/event-stream")
            || response
                .headers()
                .get(header::CONTENT_ENCODING)
                .is_some_and(|h| h.as_bytes() != b"identity")
        {
            return Err(RelayError::SettlementPending);
        }
        let mut parser = AssistantSseParser::new(limits.event_bytes.min(131072));
        let mut bytes = 0usize;
        let mut body = response.bytes_stream();
        'read: while let Some(chunk) = tokio::time::timeout(self.limits.read_timeout, body.next())
            .await
            .map_err(|_| RelayError::SettlementPending)?
        {
            let chunk = chunk.map_err(|_| RelayError::SettlementPending)?;
            bytes = bytes
                .checked_add(chunk.len())
                .ok_or(RelayError::Limit("agent provider output"))?;
            if bytes > self.limits.upstream_bytes {
                return Err(RelayError::Limit("agent provider output"));
            }
            for byte in chunk {
                if let Some(frame) = parser.feed(byte)? {
                    let events = output.decoder.frame(frame)?;
                    output.events(events, &mut publish)?;
                    if output.decoder.is_done() {
                        break 'read;
                    }
                }
            }
        }
        // A terminal frame closes the attempt immediately; never drain/replay it.
        drop(body);
        let events = output.decoder.eof()?;
        output.events(events, &mut publish)?;
        let observed = output.encoder.usage();
        self.bridge
            .provider_policy
            .validate_usage(&p.attempt, &observed, false)?;
        let usage = assistant_usage(&p.price, p.input_ceiling, p.output_ceiling, observed)?;
        let bytes = output
            .encoder
            .terminal()?
            .into_iter()
            .next()
            .ok_or(RelayError::Truncated)?;
        let value = serde_json::from_slice(&bytes).map_err(|_| RelayError::SettlementPending)?;
        Ok((bytes, value, usage))
    }
}

struct AgentOutput {
    decoder: AssistantDecoder,
    encoder: AssistantEncoder,
    prose: BTreeMap<usize, String>,
}
impl AgentOutput {
    fn new(
        request: &relay::Request,
        limits: &Limits,
        mode: Option<crate::routing::CacheCreationMode>,
    ) -> Self {
        Self {
            decoder: AssistantDecoder::new_with_cache_creation_mode(
                relay::UpstreamProtocol::Chat,
                limits,
                mode,
            ),
            encoder: AssistantEncoder::new(request, "assistant", limits),
            prose: BTreeMap::new(),
        }
    }
    fn events(
        &mut self,
        events: Vec<Event>,
        publish: &mut impl FnMut(&str) -> Result<(), RelayError>,
    ) -> Result<(), RelayError> {
        for event in events {
            let changed = match &event {
                Event::Block {
                    index,
                    kind: BlockKind::Text,
                } => {
                    self.prose.insert(*index, String::new());
                    false
                }
                Event::Delta { index, text } if self.prose.contains_key(index) => {
                    self.prose
                        .get_mut(index)
                        .ok_or(RelayError::Protocol("agent text block"))?
                        .push_str(text);
                    true
                }
                _ => false,
            };
            self.encoder.consume(event)?;
            if changed {
                let text: String = self.prose.values().map(String::as_str).collect();
                if text.len() > 32768 {
                    return Err(RelayError::Limit("agent prose"));
                }
                publish(&text)?;
            }
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn decode(wire: &str) -> Result<(Vec<String>, Value, relay::Usage), RelayError> {
        let limits = Limits::default();
        let request = relay::Request::parse(
            ClientProtocol::Chat,
            br#"{"model":"local","messages":[{"role":"user","content":"x"}],"stream":false}"#,
            &limits,
        )?;
        let mut output = AgentOutput::new(&request, &limits, None);
        let mut parser = AssistantSseParser::new(limits.event_bytes);
        let mut prefixes = Vec::new();
        for byte in wire.bytes() {
            if let Some(frame) = parser.feed(byte)? {
                let events = output.decoder.frame(frame)?;
                output.events(events, &mut |text| {
                    prefixes.push(text.to_owned());
                    Ok(())
                })?;
            }
        }
        parser.finish()?;
        let events = output.decoder.eof()?;
        output.events(events, &mut |_| Ok(()))?;
        let usage = output.encoder.usage();
        let encoded = output.encoder.terminal()?;
        Ok((
            prefixes,
            serde_json::from_slice(&encoded[0]).unwrap(),
            usage,
        ))
    }

    #[test]
    fn actual_sse_frames_assemble_prefixes_tool_arguments_and_usage() {
        let wire = [
            json!({"id":"s","choices":[{"index":0,"delta":{"role":"assistant","content":"hello ","tool_calls":[{"index":0,"id":"call1","type":"function","function":{"name":"read","arguments":"{\"x\":"}}]},"finish_reason":null}]}),
            json!({"id":"s","choices":[{"index":0,"delta":{"content":"世界","tool_calls":[{"index":0,"function":{"arguments":"1}"}}]},"finish_reason":null}]}),
            json!({"id":"s","choices":[{"index":0,"delta":{},"finish_reason":"tool_calls"}]}),
            json!({"id":"s","choices":[],"usage":{"prompt_tokens":10,"completion_tokens":20,"prompt_tokens_details":{"cached_tokens":2}}}),
        ].into_iter().map(|v| format!("data: {v}\r\n\r\n")).collect::<String>() + "data: [DONE]\r\n\r\n";
        let (prefixes, value, usage) = decode(&wire).unwrap();
        assert_eq!(prefixes, vec!["hello ", "hello 世界"]);
        let Some((AgentAnswer::Tools(calls), _)) =
            parse_answer(&value, &BTreeSet::from(["read".into()]))
        else {
            panic!("real assembled call")
        };
        assert_eq!(calls[0].id, "call1");
        assert_eq!(calls[0].arguments, b"{\"x\":1}");
        assert_eq!(usage.input_tokens, Some(10));
        assert_eq!(usage.output_tokens, Some(20));
        assert_eq!(usage.cached_input_tokens, Some(2));
    }

    #[test]
    fn partial_provider_stream_never_creates_a_complete_observation() {
        let frame = format!(
            "data: {}\n\n",
            json!({"id":"s","choices":[{"index":0,"delta":{"content":"partial"},"finish_reason":null}]})
        );
        assert!(matches!(decode(&frame), Err(RelayError::Truncated)));
        assert!(decode(&(frame + "data: [DONE]\n\n")).is_err());
    }
}

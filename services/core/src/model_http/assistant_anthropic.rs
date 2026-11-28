// Copyright (C) 2026 LIghtJUNction. AGPL-3.0-or-later.
//! The assistant keeps its Core-owned transcript and uses the relay's existing
//! protocol conversion at the selected supplier boundary. No caller can select
//! an endpoint, credential, tariff or settlement result here.
use super::*;
use crate::relay::{AssistantDecoder, AssistantEncoder, Route, UpstreamProtocol};
#[path = "assistant_anthropic_metadata.rs"]
mod metadata;

pub(super) fn assistant_anthropic_decoder_input(raw: &str) -> Result<String, RelayError> {
    metadata::project(raw)
}

pub(super) fn assistant_wire_payload(
    payload: &Value,
    protocol: UpstreamProtocol,
    model: &str,
    limits: &Limits,
) -> Result<Value, RelayError> {
    match protocol {
        UpstreamProtocol::Chat => Ok(payload.clone()),
        UpstreamProtocol::Anthropic => {
            let bytes = serde_json::to_vec(payload)
                .map_err(|_| RelayError::Invalid("assistant payload"))?;
            let mut wire = relay::Request::parse(ClientProtocol::Chat, &bytes, limits)?
                .upstream_to(protocol, model)?;
            // This fixed assistant tariff only admits the standard service tier.
            // Keep workspace inference geography unchanged and validate its receipt.
            wire["service_tier"] = json!("standard_only");
            Ok(wire)
        }
        _ => Err(RelayError::RouteUnavailable),
    }
}

pub(super) fn assistant_request(
    client: &reqwest::Client,
    route: &Route,
    payload: &Value,
) -> Result<reqwest::RequestBuilder, RelayError> {
    let request = client.post(route.endpoint.clone()).json(payload);
    match route.protocol {
        UpstreamProtocol::Chat => {
            Ok(request.header(header::AUTHORIZATION, route.credential.clone()))
        }
        UpstreamProtocol::Anthropic => route.anthropic_auth(request),
        _ => Err(RelayError::RouteUnavailable),
    }
}

pub(super) fn assistant_response(
    payload: &Value,
    raw_response: &[u8],
    protocol: UpstreamProtocol,
    mode: Option<crate::routing::CacheCreationMode>,
    limits: &Limits,
    stage: &mut &'static str,
) -> Result<Value, RelayError> {
    *stage = "response_decode";
    if protocol == UpstreamProtocol::Chat {
        return serde_json::from_slice(raw_response).map_err(|_| RelayError::SettlementPending);
    }
    if protocol != UpstreamProtocol::Anthropic {
        return Err(RelayError::RouteUnavailable);
    }
    *stage = "anthropic_usage_dimensions";
    let projected = assistant_anthropic_decoder_input(
        std::str::from_utf8(raw_response).map_err(|_| RelayError::SettlementPending)?,
    )?;
    *stage = "response_decode";
    let response: Value =
        serde_json::from_str(&projected).map_err(|_| RelayError::SettlementPending)?;
    let limits = Limits {
        output_bytes: limits.output_bytes.min(131072),
        event_bytes: limits.event_bytes.min(131072),
        tool_argument_bytes: limits.tool_argument_bytes.min(32768),
        ..limits.clone()
    };
    let request = relay::Request::parse(
        ClientProtocol::Chat,
        &serde_json::to_vec(payload).map_err(|_| RelayError::Invalid("assistant payload"))?,
        &limits,
    )?;
    let mut decoder = AssistantDecoder::new_with_cache_creation_mode(protocol, &limits, mode);
    let mut encoder = AssistantEncoder::new(&request, "assistant", &limits);
    for event in decoder.json(&response)? {
        encoder.consume(event)?;
    }
    for event in decoder.eof()? {
        encoder.consume(event)?;
    }
    let packets = encoder.terminal()?;
    if packets.len() != 1 {
        return Err(RelayError::SettlementPending);
    }
    // This projection is only the assistant's transcript. Billing has already
    // read the original Anthropic counters, including cached and created input.
    serde_json::from_slice(&packets[0]).map_err(|_| RelayError::SettlementPending)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn payload() -> Value {
        json!({"model":"internal-alias","messages":[
            {"role":"system","content":"Only answer onboarding questions."},
            {"role":"user","content":"How do I start?"}
        ],"max_tokens":32,"stream":false})
    }

    #[test]
    fn native_messages_uses_bearer_only_on_the_exact_stepfun_api() {
        for (url, bearer) in [
            ("https://api.stepfun.com/v1/messages", true),
            ("https://api.stepfun.com:8443/v1/messages", false),
            ("https://api.stepfun.com/custom/v1/messages", false),
            ("https://api.stepfun.com.example/v1/messages", false),
            (
                "https://api.modelarts-maas.com/anthropic/v1/messages",
                false,
            ),
        ] {
            let route = Route::new(
                "fixture".into(),
                UpstreamProtocol::Anthropic,
                url,
                "published-model".into(),
                "synthetic-only",
                1,
            )
            .unwrap();
            let payload = json!({"model":"published-model","max_tokens":32,"messages":[{"role":"user","content":"fixture"}]});
            let request = assistant_request(&reqwest::Client::new(), &route, &payload)
                .unwrap()
                .build()
                .unwrap();
            assert_eq!(request.url().as_str(), url);
            assert_eq!(request.headers()["anthropic-version"], "2023-06-01");
            assert_eq!(
                request
                    .headers()
                    .get("authorization")
                    .and_then(|h| h.to_str().ok()),
                bearer.then_some("Bearer synthetic-only")
            );
            assert_eq!(
                request
                    .headers()
                    .get("x-api-key")
                    .and_then(|h| h.to_str().ok()),
                (!bearer).then_some("synthetic-only")
            );
            assert_eq!(
                serde_json::from_slice::<Value>(request.body().unwrap().as_bytes().unwrap())
                    .unwrap(),
                payload
            );
        }
    }

    #[test]
    fn anthropic_wire_uses_selected_model_native_tools_and_credentials() {
        let mut original = payload();
        original["tools"] = super::super::super::assistant_gift::tool_schemas(1000);
        assert!(
            assistant_wire_payload(
                &original,
                UpstreamProtocol::Anthropic,
                "claude-selected",
                &Limits::default()
            )
            .is_err(),
            "a general strict-tool request must retain the relay's protocol guard"
        );
        original["tools"] = super::super::super::assistant_gift::anthropic_tool_schemas(1000);
        original["tool_choice"] = json!("auto");
        original["parallel_tool_calls"] = json!(false);
        let wire = assistant_wire_payload(
            &original,
            UpstreamProtocol::Anthropic,
            "claude-selected",
            &Limits::default(),
        )
        .unwrap();
        assert_eq!(wire["model"], "claude-selected");
        assert_eq!(wire["service_tier"], "standard_only");
        assert!(wire.get("inference_geo").is_none());
        assert_eq!(wire["max_tokens"], 32);
        assert!(wire.get("system").is_some());
        assert!(
            wire["messages"]
                .as_array()
                .unwrap()
                .iter()
                .all(|m| m["role"] != "system")
        );
        assert_eq!(wire["tools"][0]["name"], "prepare_new_user_gift");
        assert_eq!(
            wire["tools"][0]["input_schema"]["additionalProperties"],
            false
        );
        assert!(wire["tools"][0].get("function").is_none());
        assert!(wire.get("stream_options").is_none());
        let route = Route::new(
            "assistant-fixture".into(),
            UpstreamProtocol::Anthropic,
            "https://provider.example/prefix/v1/messages",
            "claude-selected".into(),
            "synthetic-assistant-key",
            1,
        )
        .unwrap();
        let request = assistant_request(&reqwest::Client::new(), &route, &wire)
            .unwrap()
            .build()
            .unwrap();
        assert_eq!(request.url().path(), "/prefix/v1/messages");
        assert_eq!(request.headers()["x-api-key"], "synthetic-assistant-key");
        assert!(request.headers()["x-api-key"].is_sensitive());
        assert_eq!(request.headers()["anthropic-version"], "2023-06-01");
        assert!(request.headers().get(header::AUTHORIZATION).is_none());
        assert_eq!(
            assistant_wire_payload(
                &original,
                UpstreamProtocol::Chat,
                "ignored",
                &Limits::default()
            )
            .unwrap(),
            original
        );
    }

    #[test]
    fn complete_anthropic_answer_uses_the_existing_terminal_decoder() {
        let response = json!({"id":"msg_fixture","type":"message","role":"assistant",
            "model":"claude-selected","content":[{"type":"text","text":"Create an API key."}],
            "stop_reason":"end_turn","stop_sequence":null,
            "usage":{"input_tokens":11,"output_tokens":5,"cache_read_input_tokens":7,
                "cache_creation_input_tokens":3,
                "cache_creation":{"ephemeral_5m_input_tokens":3,"ephemeral_1h_input_tokens":0}}});
        let normalized = assistant_response(
            &payload(),
            &serde_json::to_vec(&response).unwrap(),
            UpstreamProtocol::Anthropic,
            None,
            &Limits::default(),
            &mut "",
        )
        .unwrap();
        assert_eq!(
            normalized["choices"][0]["message"]["content"],
            "Create an API key."
        );
        assert_eq!(normalized["choices"][0]["finish_reason"], "stop");
        let usage = relay::provider_usage_with_cache_creation_mode(
            UpstreamProtocol::Anthropic,
            &response["usage"],
            None,
        )
        .unwrap();
        assert_eq!(usage.input_tokens, Some(21));
        assert_eq!(usage.output_tokens, Some(5));
        assert_eq!(usage.cached_input_tokens, Some(7));
        for stop in [Value::Null, json!("unknown_stop")] {
            let mut incomplete = response.clone();
            incomplete["stop_reason"] = stop;
            assert!(
                assistant_response(
                    &payload(),
                    &serde_json::to_vec(&incomplete).unwrap(),
                    UpstreamProtocol::Anthropic,
                    None,
                    &Limits::default(),
                    &mut "",
                )
                .is_err()
            );
        }
    }

    #[test]
    fn standard_metadata_preserves_usage_and_keeps_assistant_geography_scoped() {
        let response = json!({"id":"msg_standard","type":"message","role":"assistant",
            "model":"claude-selected","content":[{"type":"text","text":"Hello"}],
            "stop_reason":"end_turn","stop_sequence":null,
            "usage":{"input_tokens":11,"output_tokens":5,"cache_read_input_tokens":7,
                "cache_creation_input_tokens":0,"service_tier":"standard","inference_geo":"global",
                "server_tool_use":{"web_search_requests":0,"web_fetch_requests":0}}});
        let limits = Limits::default();
        let mut generic = AssistantDecoder::new(UpstreamProtocol::Anthropic, &limits);
        let events = generic.json(&response).unwrap();
        let generic_usage = events
            .iter()
            .find_map(|event| match event {
                relay::Event::Usage(receipt) => Some(receipt),
                _ => None,
            })
            .unwrap();
        let raw = serde_json::to_vec(&response).unwrap();
        let normalized = assistant_response(
            &payload(),
            &raw,
            UpstreamProtocol::Anthropic,
            None,
            &limits,
            &mut "",
        )
        .unwrap();
        assert_eq!(normalized["choices"][0]["message"]["content"], "Hello");
        let usage = relay::provider_usage_with_cache_creation_mode(
            UpstreamProtocol::Anthropic,
            &response["usage"],
            None,
        )
        .unwrap();
        for receipt in [generic_usage, &usage] {
            assert_eq!(
                (
                    receipt.input_tokens,
                    receipt.cached_input_tokens,
                    receipt.output_tokens
                ),
                (Some(18), Some(7), Some(5))
            );
        }
        assert_eq!(serde_json::from_slice::<Value>(&raw).unwrap(), response);

        let mut unavailable_geo = response;
        unavailable_geo["usage"]["inference_geo"] = json!("not_available");
        let mut generic = AssistantDecoder::new(UpstreamProtocol::Anthropic, &limits);
        assert!(generic.json(&unavailable_geo).is_ok());
        assert_eq!(
            assistant_anthropic_decoder_input(&unavailable_geo.to_string()).unwrap_err(),
            RelayError::Unsupported("assistant provider billing dimensions")
        );
    }

    #[test]
    fn standard_metadata_does_not_change_raw_gift_provenance() {
        let raw = br#"{"id":"msg_raw","type":"message","role":"assistant","model":"claude-selected","content":[{"type":"tool_use","id":"toolu_original","name":"prepare_new_user_gift","input":{"amount_credits":0,"reason":"not eligible"}}],"stop_reason":"tool_use","stop_sequence":null,"usage":{"input_tokens":11,"output_tokens":5,"cache_read_input_tokens":0,"cache_creation_input_tokens":0,"service_tier":"standard","inference_geo":"global","server_tool_use":null}}"#;
        let call = crate::model_http::assistant_gift::parse_anthropic(raw)
            .unwrap()
            .unwrap();
        assert_eq!(call.id(), "toolu_original");
        assert_eq!(
            call.response_sha256(),
            <[u8; 32]>::from(Sha256::digest(raw))
        );
        let normalized = assistant_response(
            &payload(),
            raw,
            UpstreamProtocol::Anthropic,
            None,
            &Limits::default(),
            &mut "",
        )
        .unwrap();
        assert_eq!(
            normalized["choices"][0]["message"]["tool_calls"][0]["id"],
            "toolu_original"
        );
        assert!(matches!(
            call.action(),
            crate::model_http::assistant_gift::GiftAction::Prepare(
                crate::model_http::assistant_gift::GiftArguments {
                    amount_credits: 0,
                    ..
                }
            )
        ));
    }

    fn local_gateway(base: &str) -> ModelGateway {
        let mut config: crate::routing::RoutingConfig = serde_json::from_value(json!({
            "version":1,"groups":[{"name":"allowed","enabled":true}],
            "upstreams":[{"id":1,"endpoint":"https://synthetic.invalid/v1","protocol":"anthropic_messages","credential":{"id":7,"revision":1},"enabled":true}],
            "routes":[{"id":1,"model":"internal-alias","group":"allowed","enabled":true,"speeds":["standard"],
                "targets":[{"id":1,"upstream_id":1,"upstream_model":"claude-selected","enabled":true,"priority":0,"weight":1,"speeds":["standard"],"payload":"adapt","timeout_ms":5000}],
                "failover":{"max_attempts":1,"retry_on":[],"allow_replay_after_send":false}}],"aliases":[]
        })).unwrap();
        config.upstreams[0].endpoint =
            crate::routing::Endpoint::local_loopback(base.into()).unwrap();
        let prices = serde_json::from_value(json!({"version":1,"rules":[{
            "id":1,"model":"internal-alias","group":"allowed","speed":"standard",
            "rates":{"input":1000000,"output":1000000,"cache_read":1000000,"cache_write":0,"request":0},
            "group_multiplier_ppm":1000000,"speed_multiplier_ppm":1000000
        }]})).unwrap();
        let router = crate::routing::Router::new(
            config,
            Arc::new(crate::routing::PriceSnapshot::compile(prices).unwrap()),
        )
        .unwrap();
        let pool = sqlx::postgres::PgPoolOptions::new()
            .connect_lazy("postgres://unused:unused@127.0.0.1:1/unused")
            .unwrap();
        ModelGateway::new(
            pool,
            Arc::new(router),
            vec![UpstreamCredential {
                reference: crate::routing::CredentialRef { id: 7, revision: 1 },
                secret: "synthetic-assistant-key".into(),
            }],
            BTreeMap::new(),
            Limits {
                total_timeout: Duration::from_secs(2),
                hook_timeout: Duration::from_millis(100),
                ..Default::default()
            },
            ChargePolicy {
                input_token_ceiling: 16384,
                lease_seconds: 10,
            },
        )
        .unwrap()
    }

    #[tokio::test]
    async fn actual_assistant_http_uses_selected_anthropic_route_and_original_receipt() {
        use axum::{
            Json, Router,
            http::{HeaderMap, StatusCode},
            response::IntoResponse,
            routing::post,
        };
        use std::sync::Mutex;
        let captured = Arc::new(Mutex::new(Vec::new()));
        let requests = captured.clone();
        let app = Router::new().route("/prefix/v1/messages", post(move |headers: HeaderMap, Json(input): Json<Value>| {
            requests.lock().unwrap().push((headers, input.clone()));
            async move {
                let gift = input.get("tools").is_some();
                let mut response = json!({"id":"msg_http","type":"message","role":"assistant","model":"claude-selected",
                    "content":[{"type":"text","text":"Create an API key."}],"stop_reason":"end_turn","stop_sequence":null,
                    "usage":{"input_tokens":11,"output_tokens":5,"cache_read_input_tokens":7,"cache_creation_input_tokens":0,"service_tier":"standard","inference_geo":"global","server_tool_use":{"web_search_requests":0,"web_fetch_requests":0}}});
                if gift {
                    response["content"] = json!([{"type":"tool_use","id":"toolu_http","name":"get_new_user_gift_status","input":{}}]);
                    response["stop_reason"] = json!("tool_use");
                }
                let text = input["messages"][0]["content"].to_string();
                if text.contains("simulate rejection") {
                    return (StatusCode::UNAUTHORIZED, "internal supplier rejection text").into_response();
                }
                if text.contains("missing usage") {
                    response.as_object_mut().unwrap().remove("usage");
                }
                if text.contains("different tier") {
                    response["usage"]["service_tier"] = json!("priority");
                }
                Json(response).into_response()
            }
        }));
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let base = format!("http://{}/prefix/v1", listener.local_addr().unwrap());
        let (stop, stopped) = tokio::sync::oneshot::channel::<()>();
        let server = tokio::spawn(async move {
            axum::serve(listener, app)
                .with_graceful_shutdown(async {
                    let _ = stopped.await;
                })
                .await
                .unwrap();
        });
        let gateway = local_gateway(&base);
        gateway
            .validate_assistant_policy("internal-alias", "allowed", 32)
            .unwrap();
        let quote = gateway
            .assistant_quote("internal-alias", "allowed")
            .unwrap();
        let transport = crate::relay::provider_network::ProviderTransport::new(1);
        for gift_cap in [None, Some(1000)] {
            let prepared = gateway
                .prepare_assistant(
                    "internal-alias",
                    "allowed",
                    32,
                    json!([{"role":"user","content":"How do I start?"}]),
                    &transport,
                    gift_cap,
                    &quote,
                    7,
                )
                .await
                .unwrap();
            assert_eq!(prepared.route.protocol, UpstreamProtocol::Anthropic);
            assert_eq!(prepared.route.endpoint.path(), "/prefix/v1/messages");
            assert_eq!(
                prepared.fingerprint,
                <[u8; 32]>::from(Sha256::digest(
                    serde_json::to_vec(&prepared.wire_payload).unwrap()
                ))
            );
            assert!(prepared.reserve > 0);
            let observation = gateway.execute_assistant(prepared).await.unwrap();
            assert_eq!(observation.usage.input, 18);
            assert_eq!(observation.usage.cached, 7);
            assert_eq!(observation.usage.output, 5);
            match (gift_cap, observation.answer) {
                (None, AssistantAnswer::Text(text)) => assert_eq!(text, "Create an API key."),
                (Some(_), AssistantAnswer::Gift(call)) => {
                    assert_eq!(call.id(), "toolu_http");
                    assert!(matches!(
                        call.action(),
                        super::super::super::assistant_gift::GiftAction::Status
                    ));
                }
                _ => panic!("wrong assistant observation"),
            }
        }
        for (prompt, expected) in [
            (
                "simulate rejection",
                "stage=provider_http protocol=Anthropic code=settlement_pending http_status=401",
            ),
            (
                "missing usage",
                "stage=usage_missing protocol=Anthropic code=settlement_pending http_status=200",
            ),
            (
                "different tier",
                "stage=anthropic_usage_dimensions protocol=Anthropic code=unsupported_feature http_status=200",
            ),
        ] {
            let prepared = gateway
                .prepare_assistant(
                    "internal-alias",
                    "allowed",
                    32,
                    json!([{"role":"user","content":prompt}]),
                    &transport,
                    None,
                    &quote,
                    7,
                )
                .await
                .unwrap();
            let Err(failure) = gateway.execute_assistant(prepared).await else {
                panic!("synthetic failure settled");
            };
            assert_eq!(failure.to_string(), expected);
        }
        {
            let captured = captured.lock().unwrap();
            assert_eq!(captured.len(), 5);
            for (headers, request) in captured.iter() {
                assert_eq!(headers["x-api-key"], "synthetic-assistant-key");
                assert_eq!(headers["anthropic-version"], "2023-06-01");
                assert!(headers.get(header::AUTHORIZATION).is_none());
                assert_eq!(request["model"], "claude-selected");
                assert_eq!(request["max_tokens"], 32);
                assert_eq!(request["service_tier"], "standard_only");
                assert!(request.get("inference_geo").is_none());
            }
        }
        stop.send(()).unwrap();
        tokio::time::timeout(Duration::from_secs(2), server)
            .await
            .unwrap()
            .unwrap();
    }
}

//! Non-streaming OpenAI Chat audio. No fabricated usage or text billing fallback.
use super::super::*;
use super::*;
use crate::{
    billing::charging::{self, Outcome},
    routing::{PayloadPolicy, Protocol, ReplayPermission, RouteRequest},
};
use serde::Deserialize;
use serde_json::{Value, value::RawValue};
use tokio::sync::OwnedSemaphorePermit;

pub(crate) fn validate_rules(rules: &[AudioRule]) -> std::result::Result<(), RelayError> {
    let mut keys = std::collections::BTreeSet::new();
    for rule in rules {
        rule.price
            .validate()
            .map_err(|_| RelayError::Invalid("audio tariff"))?;
        if rule.route_version == 0
            || rule.upstream_id == 0
            || rule.upstream_model.is_empty()
            || rule.price.usage_format != AudioUsageFormat::OpenAiChatV1
            || rule.price.max_input_tokens == 0
            || rule.price.max_input_tokens > u32::MAX as u64 / 2
            || rule.price.max_output_tokens == 0
            || rule.price.max_output_tokens > u32::MAX as u64 / 2
            || rule.price.bounds.text_input > rule.price.max_input_tokens
            || rule.price.bounds.audio_input > rule.price.max_input_tokens
            || rule.price.bounds.text_output > rule.price.max_output_tokens
            || rule.price.bounds.audio_output > rule.price.max_output_tokens
            || rule.price.bounds.text_input + rule.price.bounds.audio_input == 0
            || !keys.insert((
                rule.route_version,
                rule.upstream_id,
                &rule.upstream_model,
                &rule.price.model,
                &rule.price.group,
            ))
        {
            return Err(RelayError::Invalid("audio provider contract"));
        }
        crate::relay::honeypot_network::validate_base_url(&rule.endpoint, true)?;
    }
    Ok(())
}
fn audio_signal(value: &Value) -> bool {
    match value {
        Value::Object(map) => {
            map.contains_key("audio")
                || matches!(
                    map.get("type").and_then(Value::as_str),
                    Some("input_audio" | "audio")
                )
                || map
                    .get("modalities")
                    .and_then(Value::as_array)
                    .is_some_and(|m| m.iter().any(|v| v == "audio"))
                || ["mimeType", "mime_type"].iter().any(|key| {
                    map.get(*key)
                        .and_then(Value::as_str)
                        .is_some_and(|mime| mime.starts_with("audio/"))
                })
                || map
                    .get("responseModalities")
                    .and_then(Value::as_array)
                    .is_some_and(|values| values.iter().any(|v| v == "AUDIO"))
                || map.values().any(audio_signal)
        }
        Value::Array(values) => values.iter().any(audio_signal),
        _ => false,
    }
}
pub(crate) fn handles(gateway: &ModelGateway, raw: &[u8]) -> bool {
    let Ok(value) = serde_json::from_slice::<Value>(raw) else {
        return false;
    };
    if audio_signal(&value) {
        return true;
    }
    let Some(model) = value["model"].as_str() else {
        return false;
    };
    // Also catch aliases of audio-configured models on text-only prompts.
    gateway.audio_rules.iter().any(|r| {
        r.price.model == model
            || gateway
                .bridge
                .routing
                .route(RouteRequest {
                    model,
                    group: &r.price.group,
                    speed: None,
                    protocol: Protocol::OpenAiChat,
                    replay: ReplayPermission::Denied,
                })
                .is_ok_and(|plan| plan.canonical_model() == r.price.model)
    })
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct ChatRequest {
    model: String,
    messages: Vec<Value>,
    max_completion_tokens: u32,
    #[serde(default)]
    modalities: Vec<String>,
    audio: Option<Value>,
    stream: Option<bool>,
    n: Option<u32>,
    temperature: Option<serde_json::Number>,
}
impl ChatRequest {
    fn parse(raw: &[u8]) -> std::result::Result<Self, RelayError> {
        let request: Self = serde_json::from_slice(raw)
            .map_err(|_| RelayError::Invalid("unsupported audio chat request fields"))?;
        if request.model.is_empty()
            || request.messages.is_empty()
            || request.max_completion_tokens == 0
            || request.stream == Some(true)
            || request.n.is_some_and(|n| n != 1)
            || request
                .modalities
                .iter()
                .any(|m| !matches!(m.as_str(), "text" | "audio"))
        {
            return Err(RelayError::Unsupported(
                "audio requires non-streaming single-choice Chat with explicit output limit",
            ));
        }
        if request.modalities.iter().any(|m| m == "audio") {
            let audio = request
                .audio
                .as_ref()
                .and_then(Value::as_object)
                .ok_or(RelayError::Invalid("audio output configuration required"))?;
            if audio.len() != 2
                || audio
                    .get("voice")
                    .and_then(Value::as_str)
                    .is_none_or(str::is_empty)
                || !audio
                    .get("format")
                    .and_then(Value::as_str)
                    .is_some_and(|f| matches!(f, "wav" | "mp3" | "flac" | "opus" | "pcm16"))
            {
                return Err(RelayError::Invalid(
                    "unsupported audio output configuration",
                ));
            }
        } else if request.audio.is_some() {
            return Err(RelayError::Invalid("audio output requires audio modality"));
        }
        for message in &request.messages {
            let object = message
                .as_object()
                .ok_or(RelayError::Invalid("audio message"))?;
            if object.len() != 2
                || !matches!(
                    message["role"].as_str(),
                    Some("user" | "system" | "developer")
                )
            {
                return Err(RelayError::Unsupported(
                    "audio v1 accepts user/system/developer messages without tools or references",
                ));
            }
            match &message["content"] {
                Value::String(_) => (),
                Value::Array(parts) if !parts.is_empty() => {
                    for part in parts {
                        let object = part
                            .as_object()
                            .ok_or(RelayError::Invalid("audio content"))?;
                        match part["type"].as_str() {
                            Some("text") if object.len() == 2 && part["text"].is_string() => (),
                            Some("input_audio") if object.len() == 2 => {
                                let audio = part["input_audio"]
                                    .as_object()
                                    .ok_or(RelayError::Invalid("input audio"))?;
                                if audio.len() != 2
                                    || !matches!(
                                        part["input_audio"]["format"].as_str(),
                                        Some("wav" | "mp3")
                                    )
                                    || part["input_audio"]["data"]
                                        .as_str()
                                        .is_none_or(str::is_empty)
                                {
                                    return Err(RelayError::Invalid(
                                        "input audio requires inline wav/mp3 data",
                                    ));
                                }
                            }
                            _ => {
                                return Err(RelayError::Unsupported(
                                    "unsupported audio message modality",
                                ));
                            }
                        }
                    }
                }
                _ => return Err(RelayError::Invalid("audio message content")),
            }
        }
        Ok(request)
    }
    fn wire(&self, model: &str) -> std::result::Result<Vec<u8>, RelayError> {
        let mut value = json!({"model":model,"messages":self.messages,"max_completion_tokens":self.max_completion_tokens,"stream":false,"n":1});
        if !self.modalities.is_empty() {
            value["modalities"] = json!(self.modalities);
        }
        if let Some(audio) = &self.audio {
            value["audio"] = audio.clone();
        }
        if let Some(temperature) = &self.temperature {
            value["temperature"] = json!(temperature);
        }
        serde_json::to_vec(&value).map_err(|_| RelayError::Invalid("audio wire request"))
    }
}

pub(crate) async fn handle(
    gateway: ModelGateway,
    headers: HeaderMap,
    body: Bytes,
    peer: Option<IpAddr>,
) -> Response {
    let caller = match native_http::authenticate(&gateway, &headers, peer).await {
        Ok(v) => v,
        Err(e) => return public_error(e),
    };
    if !matches!(one_header(&headers, header::CONTENT_TYPE.as_str()), Ok(Some(v)) if v.split(';').next().is_some_and(|s| s.trim().eq_ignore_ascii_case("application/json")))
    {
        return public_error(RelayError::Invalid("audio requires application/json"));
    }
    let request = match ChatRequest::parse(&body) {
        Ok(v) => v,
        Err(e) => return public_error(e),
    };
    let id = match caller
        .principal
        .key_id()
        .ok_or(RelayError::Forbidden)
        .and_then(|key| request_id(key, &headers))
    {
        Ok(v) => v,
        Err(e) => return public_error(e),
    };
    let permit = match gateway.native_http.gate.clone().try_acquire_owned() {
        Ok(v) => v,
        Err(_) => return public_error(RelayError::Busy),
    };
    let worker_id = id.clone();
    let result = tokio::spawn(execute(gateway, caller, request, worker_id, permit)).await;
    let response = match result {
        Ok(Ok(value)) => Json(value).into_response(),
        Ok(Err(error)) => public_error(error),
        Err(_) => public_error(RelayError::SettlementPending),
    };
    let mut response = with_request_id(response, &id);
    response
        .headers_mut()
        .insert(header::CACHE_CONTROL, HeaderValue::from_static("no-store"));
    response
}

async fn execute(
    gateway: ModelGateway,
    caller: native_http::Caller,
    request: ChatRequest,
    id: String,
    _permit: OwnedSemaphorePermit,
) -> std::result::Result<Value, RelayError> {
    if gateway
        .bridge
        .charging
        .resolve(&id)
        .await
        .map_err(billing::error)?
        .is_some()
    {
        return Err(RelayError::Conflict);
    }
    let mut selected = None;
    for group in &caller.groups {
        if let Ok(plan) = gateway.bridge.routing.route(RouteRequest {
            model: &request.model,
            group,
            speed: None,
            protocol: Protocol::OpenAiChat,
            replay: ReplayPermission::Denied,
        }) {
            gateway
                .authorize_request(
                    &caller.principal,
                    &caller.secret,
                    plan.canonical_model(),
                    group,
                    caller.peer,
                )
                .await
                .map_err(identity_error)?;
            selected = Some((group.clone(), plan));
            break;
        }
    }
    let (group, mut plan) = selected.ok_or(RelayError::RouteUnavailable)?;
    let canonical = plan.canonical_model().to_owned();
    let attempt = Arc::new(plan.start().map_err(|_| RelayError::RouteUnavailable)?);
    if attempt.protocol() != Protocol::OpenAiChat
        || attempt.payload_policy() != PayloadPolicy::Adapt
        || !attempt.is_available()
        || u128::from(attempt.timeout_ms()) < gateway.limits.total_timeout.as_millis()
    {
        return Err(RelayError::RouteUnavailable);
    }
    let rule = gateway
        .audio_rules
        .iter()
        .find(|r| {
            r.route_version == attempt.quote().version()
                && r.upstream_id == attempt.upstream_id()
                && r.upstream_model == attempt.upstream_model()
                && r.endpoint == attempt.endpoint()
                && r.price.model == canonical
                && r.price.group == group
        })
        .ok_or(RelayError::Unsupported(
            "reviewed audio tariff and provider bounds are not configured",
        ))?;
    if u64::from(request.max_completion_tokens) > rule.price.max_output_tokens {
        return Err(RelayError::Limit("audio output limit"));
    }
    let mut audio_price = rule.price.clone();
    audio_price.max_output_tokens = u64::from(request.max_completion_tokens);
    audio_price.bounds.text_output = audio_price
        .bounds
        .text_output
        .min(audio_price.max_output_tokens);
    audio_price.bounds.audio_output = audio_price
        .bounds
        .audio_output
        .min(audio_price.max_output_tokens);
    let price = charging::Price {
        audio: Some(audio_price.clone()),
        text_request: None,
        tool: None,
        grounding: None,
        user_trust: None,
        image: None,
        upstream_cost_group_ppm: None,
        tiered: None,
        version: audio_price.price_revision.clone(),
        model: canonical,
        group: group.clone(),
        input_per_million: 0,
        cached_per_million: 0,
        output_per_million: 0,
    };
    let reserve = price
        .reservation_ceiling(charging::ReservationBasis::Audio)
        .map_err(billing::error)?;
    let credential = gateway
        .bridge
        .credentials
        .get(&(
            attempt.credential_ref().id,
            attempt.credential_ref().revision,
        ))
        .ok_or(RelayError::RouteUnavailable)?;
    let mut endpoint = crate::relay::honeypot_network::validate_base_url(attempt.endpoint(), true)?;
    let path = endpoint.path().trim_end_matches('/');
    endpoint.set_path(&format!(
        "{}{}/chat/completions",
        path,
        if path.ends_with("/v1") { "" } else { "/v1" }
    ));
    let connection = tokio::time::timeout(
        gateway.limits.total_timeout,
        gateway
            .native_http
            .transport
            .prepare(&endpoint, &gateway.limits),
    )
    .await
    .map_err(|_| RelayError::RouteUnavailable)??;
    let wire = request.wire(attempt.upstream_model())?;
    if wire.len() > gateway.limits.request_bytes {
        return Err(RelayError::Limit("audio outbound bytes"));
    }
    let upstream_request = connection
        .client()
        .post(endpoint)
        .bearer_auth(&credential)
        .header(header::ACCEPT, "application/json")
        .header(header::CONTENT_TYPE, "application/json")
        .body(wire.clone())
        .build()
        .map_err(|_| RelayError::Invalid("audio upstream request"))?;
    let mut fingerprint = Sha256::new();
    fingerprint.update(b"lmm-audio-chat-v1\0");
    fingerprint.update(group.as_bytes());
    fingerprint.update([0]);
    fingerprint.update(request.wire(&request.model)?);
    let fingerprint: [u8; 32] = fingerprint.finalize().into();
    let mut worker = [0; 32];
    OsRng.fill_bytes(&mut worker);
    let route_id = format!(
        "r{}-t{}-u{}",
        attempt.route_id(),
        attempt.target_id(),
        attempt.upstream_id()
    );
    let wire_sha = charging::contract::digest(&wire);
    let prepared = charging::contract::PreparedReservation::audio_chat(
        charging::contract::hex_digest(&fingerprint),
        charging::contract::ReservationBinding {
            route_id: route_id.clone(),
            target_id: attempt.target_id(),
            upstream_id: attempt.upstream_id(),
            protocol: "audio_chat".into(),
            model_snapshot: attempt.upstream_model().into(),
            generation_url: upstream_request.url().as_str().into(),
            credential_ref: attempt.credential_ref(),
            payload_policy_sha256: charging::contract::typed_digest(&attempt.payload_policy())
                .map_err(billing::error)?,
            provider_contract_revision: audio_price.provider_contract_revision.clone(),
            projection_revision: "audio-chat-raw-v1".into(),
            quote_version: attempt.quote().version(),
        },
        wire_sha.clone(),
        audio_price.bounds,
    )
    .map_err(billing::error)?;
    crate::request_limits::admit(
        &gateway.bridge.pool,
        caller.principal.user_id(),
        &group,
        &id,
    )
    .await
    .map_err(routing::admission_error)?;
    let spec = charging::Request {
        id: id.clone(),
        fingerprint,
        price,
        reserve,
        lease_seconds: gateway.bridge.policy.lease_seconds,
        maximum_seconds: gateway.bridge.policy.lease_seconds,
    };
    let charge = billing::retry(|| {
        gateway.bridge.charging.reserve_prepared_model_with_context(
            &caller.secret,
            &spec,
            &worker,
            caller.peer,
            None,
            charging::ReservationBasis::Audio,
            &prepared,
        )
    })
    .await;
    let charge = match charge {
        Err(charging::Error::CommitUnknown) => {
            gateway
                .bridge
                .charging
                .resolve_prepared_model(
                    &spec,
                    &worker,
                    charging::ReservationBasis::Audio,
                    &prepared,
                )
                .await
        }
        result => result,
    }
    .map_err(billing::error)?;
    if charge.actor_user_id != caller.principal.user_id()
        || charge.key_id != caller.principal.key_id()
    {
        return Err(RelayError::Forbidden);
    }
    // Recheck immediately before start. No release after a durable dispatch claim.
    gateway
        .authorize_request(
            &caller.principal,
            &caller.secret,
            &charge.price.model,
            &group,
            caller.peer,
        )
        .await
        .map_err(identity_error)?;
    if !attempt.is_available() {
        let _ = billing::retry(|| gateway.bridge.charging.release(&id, &worker)).await;
        return Err(RelayError::RouteUnavailable);
    }
    if !billing::retry(|| gateway.bridge.charging.start(&id, &worker))
        .await
        .map_err(billing::error)?
    {
        return Err(RelayError::Conflict);
    }
    let completed = async {
        billing::retry(|| {
            gateway
                .bridge
                .charging
                .record_route_dispatch(&id, &worker, 1, &route_id, &group, &wire_sha)
        })
        .await
        .map_err(billing::error)?;
        let raw = tokio::time::timeout(gateway.limits.total_timeout, async {
            let response = tokio::time::timeout(
                gateway.limits.header_timeout,
                connection.client().execute(upstream_request),
            )
            .await
            .map_err(|_| RelayError::SettlementPending)?
            .map_err(|_| RelayError::SettlementPending)?;
            if !response.status().is_success() {
                return Err(RelayError::SettlementPending);
            }
            native_http::read_response_bytes(&connection, response, &gateway.limits).await
        })
        .await
        .map_err(|_| RelayError::SettlementPending)??;
        if std::str::from_utf8(&raw).is_ok_and(|s| s.contains(credential.as_str())) {
            return Err(RelayError::SettlementPending);
        }
        let (output, usage) = response(&raw, attempt.upstream_model())?;
        let checkpoint = billing::retry(|| {
            gateway.bridge.charging.checkpoint_audio(
                &id,
                &worker,
                usage.get(),
                Sha256::digest(&raw).into(),
            )
        })
        .await
        .map_err(billing::error)?;
        let settled = billing::retry(|| {
            gateway
                .bridge
                .charging
                .settle(&id, &worker, checkpoint.usage, Outcome::Completed)
        })
        .await;
        let settled = match settled {
            Ok(v) => Some(v),
            Err(charging::Error::CommitUnknown) => gateway
                .bridge
                .charging
                .resolve(&id)
                .await
                .map_err(billing::error)?,
            _ => return Err(RelayError::SettlementPending),
        };
        if !settled.as_ref().is_some_and(|s| {
            billing::confirmed_settlement(
                s,
                &id,
                &charge.price,
                checkpoint.usage,
                Outcome::Completed,
            )
        }) {
            return Err(RelayError::SettlementPending);
        }
        Ok(output)
    }
    .await;
    if completed.is_err() {
        let _ = billing::retry(|| gateway.bridge.charging.mark_unknown(&id, &worker)).await;
    }
    completed
}

fn response(
    raw: &[u8],
    expected_model: &str,
) -> std::result::Result<(Value, Box<RawValue>), RelayError> {
    #[derive(Deserialize)]
    struct ResponseUsage {
        usage: Box<RawValue>,
    }
    let receipt: ResponseUsage =
        serde_json::from_slice(raw).map_err(|_| RelayError::SettlementPending)?;
    let output: Value = serde_json::from_slice(raw).map_err(|_| RelayError::SettlementPending)?;
    let choices = output["choices"]
        .as_array()
        .ok_or(RelayError::SettlementPending)?;
    if output["model"].as_str() != Some(expected_model)
        || choices.len() != 1
        || choices[0]["index"] != 0
        || !matches!(
            choices[0]["finish_reason"].as_str(),
            Some("stop" | "length")
        )
        || choices[0]["message"]["role"] != "assistant"
        || !(choices[0]["message"]["content"].is_string()
            || choices[0]["message"]["audio"]["data"]
                .as_str()
                .is_some_and(|data| !data.is_empty()))
    {
        return Err(RelayError::SettlementPending);
    }
    Ok((output, receipt.usage))
}

#[cfg(test)]
#[path = "http_tests.rs"]
mod tests;

//! Native model operations: input APIs, Responses generation and compaction.
//! Shared Core authorization, immutable admission and original ledger settlement.
#[path = "input_contract.rs"]
pub(super) mod contract;
#[cfg(test)]
#[path = "input_api_tests.rs"]
mod tests;

use super::*;
use crate::{
    billing::charging::{self, Outcome},
    routing::{PayloadPolicy, Protocol, ReplayPermission, RouteRequest},
};
use axum::{body::to_bytes, extract::Request};
use contract::{Kind, Request as InputRequest};
use rand::Rng as _;
use tokio::sync::OwnedSemaphorePermit;

type Ready = Arc<Mutex<Option<tokio::sync::oneshot::Sender<Result<(), RelayError>>>>>;
fn notify_ready(ready: &Ready, result: Result<(), RelayError>) -> bool {
    if let Some(sender) = ready.lock().ok().and_then(|mut value| value.take()) {
        let _ = sender.send(result);
        true
    } else {
        false
    }
}

pub(super) fn router(gateway: ModelGateway) -> Router {
    Router::new()
        .route("/v1/embeddings", post(embeddings))
        .route("/v1/rerank", post(rerank))
        .route("/v1/responses/compact", post(compact))
        .layer(axum::middleware::map_response(no_store))
        .with_state(gateway)
}
async fn no_store(mut response: Response) -> Response {
    response
        .headers_mut()
        .insert(header::CACHE_CONTROL, HeaderValue::from_static("no-store"));
    response.headers_mut().insert(
        header::X_CONTENT_TYPE_OPTIONS,
        HeaderValue::from_static("nosniff"),
    );
    response
}
async fn compact(
    State(gateway): State<ModelGateway>,
    peer: Result<ConnectInfo<SocketAddr>, ExtensionRejection>,
    request: Request,
) -> Response {
    handle(
        gateway,
        peer.ok().map(|p| p.0.ip()),
        request,
        Kind::ResponsesCompact,
    )
    .await
}
pub(super) async fn responses_context(
    gateway: ModelGateway,
    headers: HeaderMap,
    body: Bytes,
    peer: Option<IpAddr>,
) -> Response {
    let mut request = Request::new(Body::from(body));
    *request.headers_mut() = headers;
    handle(gateway, peer, request, Kind::ResponsesContext).await
}

async fn embeddings(
    State(gateway): State<ModelGateway>,
    peer: Result<ConnectInfo<SocketAddr>, ExtensionRejection>,
    request: Request,
) -> Response {
    handle(
        gateway,
        peer.ok().map(|p| p.0.ip()),
        request,
        Kind::Embeddings,
    )
    .await
}
async fn rerank(
    State(gateway): State<ModelGateway>,
    peer: Result<ConnectInfo<SocketAddr>, ExtensionRejection>,
    request: Request,
) -> Response {
    handle(gateway, peer.ok().map(|p| p.0.ip()), request, Kind::Rerank).await
}
async fn handle(
    gateway: ModelGateway,
    peer: Option<IpAddr>,
    request: Request,
    kind: Kind,
) -> Response {
    let headers = request.headers();
    if request.uri().query().is_some() {
        return public_error(RelayError::Invalid("unsupported input-only API query"));
    }
    let caller = match native_http::authenticate(&gateway, headers, peer).await {
        Ok(caller) => caller,
        Err(error) => return public_error(error),
    };
    if !matches!(one_header(headers, header::CONTENT_TYPE.as_str()), Ok(Some(value))
        if value.split(';').next().is_some_and(|v| v.trim().eq_ignore_ascii_case("application/json")))
    {
        return public_error(RelayError::Invalid(
            "input-only API requires application/json",
        ));
    }
    let Some(key_id) = caller.principal.key_id() else {
        return public_error(RelayError::Forbidden);
    };
    let id = match request_id(key_id, headers) {
        Ok(id) => id,
        Err(error) => return public_error(error),
    };
    let permit = match gateway.native_http.gate.clone().try_acquire_owned() {
        Ok(permit) => permit,
        Err(_) => return public_error(RelayError::Busy),
    };
    let raw = match tokio::time::timeout(
        gateway.limits.read_timeout,
        to_bytes(request.into_body(), gateway.limits.request_bytes),
    )
    .await
    {
        Ok(Ok(raw)) => raw,
        Ok(Err(_)) => return public_error(RelayError::Limit("input-only request bytes")),
        Err(_) => return public_error(RelayError::Timeout(relay::Phase::Read)),
    };
    let request = match InputRequest::parse(kind, &raw) {
        Ok(request) => request,
        Err(error) => return with_request_id(public_error(error), &id),
    };
    if request.streaming() {
        let (sender, receiver) = tokio::sync::mpsc::channel(8);
        let stream_id = id.clone();
        let timeout = gateway.limits.total_timeout;
        let (start, started) = tokio::sync::oneshot::channel();
        let ready = Arc::new(Mutex::new(Some(start)));
        tokio::spawn(async move {
            if let Err(error) = execute(
                gateway,
                caller,
                request,
                stream_id,
                permit,
                Some(sender.clone()),
                Some(ready.clone()),
            )
            .await
                && !notify_ready(&ready, Err(error.clone()))
            {
                let _ = sender.try_send(Err(error));
            }
        });
        match tokio::time::timeout(timeout, started).await {
            Ok(Ok(Ok(()))) => (),
            Ok(Ok(Err(error))) => return with_request_id(public_error(error), &id),
            _ => return with_request_id(public_error(RelayError::SettlementPending), &id),
        }
        let response = (
            [
                (header::CONTENT_TYPE, "text/event-stream"),
                (header::CACHE_CONTROL, "no-store"),
            ],
            Body::from_stream(tokio_stream::wrappers::ReceiverStream::new(receiver)),
        )
            .into_response();
        return with_request_id(response, &id);
    }
    // The worker owns its admission slot and settles even if the HTTP client disconnects.
    let worker_id = id.clone();
    let result = tokio::spawn(execute(
        gateway, caller, request, worker_id, permit, None, None,
    ))
    .await;
    let response = match result {
        Ok(Ok(packet)) => (
            [(header::CONTENT_TYPE, "application/json")],
            Body::from(packet),
        )
            .into_response(),
        Ok(Err(error)) => public_error(error),
        Err(_) => public_error(RelayError::SettlementPending),
    };
    with_request_id(response, &id)
}

pub(super) fn endpoint(base: &str, protocol: Protocol) -> Result<reqwest::Url, RelayError> {
    let operation = match protocol {
        Protocol::OpenAiEmbeddings => "embeddings",
        Protocol::JinaRerank => "rerank",
        Protocol::OpenAiResponses => "responses",
        _ => return Err(RelayError::RouteUnavailable),
    };
    let mut url = crate::relay::honeypot_network::validate_base_url(base, true)?;
    let versioned = url.path().trim_end_matches('/').ends_with("/v1");
    let mut segments = url
        .path_segments_mut()
        .map_err(|_| RelayError::RouteUnavailable)?;
    segments.pop_if_empty();
    if !versioned {
        segments.push("v1");
    }
    segments.push(operation);
    drop(segments);
    Ok(url)
}

/// Authorized route, exact upstream wire and immutable reservation evidence.
/// This stage performs no reserve or provider dispatch.
struct PreparedRequest {
    parent: Option<super::responses_continuity::Parent>,
    continuity: Option<super::responses_continuity::Policy>,
    client_handle: Option<String>,
    group: String,
    attempt: Arc<crate::routing::Attempt>,
    quote: crate::routing::CreditQuote,
    price: charging::Price,
    input_max: i64,
    output_max: u32,
    credential: String,
    connection: crate::relay::provider_network::ProviderConnection,
    upstream_request: reqwest::Request,
    worker: [u8; 32],
    fingerprint: [u8; 32],
    route_id: String,
    prepared: charging::contract::PreparedReservation,
    reserve: i64,
}
async fn prepare_request(
    gateway: &ModelGateway,
    caller: &native_http::Caller,
    request: &InputRequest,
    id: &str,
    streaming: bool,
) -> Result<PreparedRequest, RelayError> {
    if gateway
        .bridge
        .charging
        .resolve(id)
        .await
        .map_err(billing::error)?
        .is_some()
    {
        return Err(RelayError::Conflict);
    }
    let parent = if let Some(handle) = request.native().and_then(|r| r.previous()) {
        Some(super::responses_continuity::resolve(gateway, caller, handle, request.model()).await?)
    } else {
        None
    };
    let groups = parent
        .as_ref()
        .map_or_else(|| caller.groups.clone(), |p| vec![p.group_name.clone()]);
    let protocol = request.kind().protocol();
    let mut selected = None;
    #[cfg(test)]
    let mut selected_trace = None;
    if parent.is_none() && request.kind().responses() {
        let plans = gateway
            .bridge
            .routing
            .route_adapted_groups(request.model(), &groups, protocol, None)
            .map_err(|_| RelayError::RouteUnavailable)?;
        // Respect the original authorization/cross_group_retry cut before seeking capability.
        let authorized = gateway
            .authorize_model_plans(&caller.principal, &caller.secret, caller.peer, plans)
            .await?;
        let require_continuity = request.kind() != Kind::ResponsesCompact
            && !request
                .native()
                .is_some_and(|r| r.opaque() && r.store() != Some(true));
        for plan in authorized {
            #[cfg(test)]
            let trace = gateway.bridge.observe_plan_before(&plan);
            let eligible = gateway
                .bridge
                .provider_policy
                .responses_capability_targets(
                    &plan,
                    if require_continuity {
                        super::provider_policy::ResponsesCapability::Continuity
                    } else {
                        super::provider_policy::ResponsesCapability::Compaction
                    },
                )?;
            if let Ok(plan) = plan.responses_capability(&eligible) {
                #[cfg(test)]
                {
                    gateway.bridge.observe_plan_after(trace, &plan);
                    selected_trace = trace;
                }
                selected = Some((plan.group().to_owned(), plan));
                break;
            }
        }
        if selected.is_none() {
            return Err(RelayError::Unsupported(
                "required native Responses capability unavailable",
            ));
        }
    } else {
        for group in &groups {
            let routing_request = RouteRequest {
                model: request.model(),
                group,
                speed: None,
                protocol,
                replay: ReplayPermission::Denied,
            };
            let routed = if let Some(parent) = &parent {
                gateway
                    .bridge
                    .routing
                    .route_responses_http_bound(routing_request, parent.binding.target_id)
            } else {
                gateway.bridge.routing.route(routing_request)
            };
            if let Ok(plan) = routed {
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
    }
    let (group, mut plan) = selected.ok_or(if parent.is_some() {
        RelayError::Invalid("continuation binding unavailable")
    } else {
        RelayError::RouteUnavailable
    })?;
    let attempt = Arc::new(plan.start().map_err(|_| {
        if parent.is_some() {
            RelayError::Invalid("continuation binding unavailable")
        } else {
            RelayError::RouteUnavailable
        }
    })?);
    let routing::FrozenTokenPricing { quote, price } = routing::freeze_token_pricing(&attempt)?;
    if !request.kind().responses() && !quote.price().is_input_only_scalar() {
        return Err(RelayError::Unsupported(
            "input-only API requires an input-only token tariff",
        ));
    }
    if request.kind().responses() && attempt.protocol() != Protocol::OpenAiResponses {
        return Err(RelayError::Unsupported(
            "compaction requires a native Responses provider",
        ));
    }
    if attempt.protocol() != protocol
        || attempt.payload_policy() != PayloadPolicy::Adapt
        || !attempt.is_available()
        || u128::from(attempt.timeout_ms()) < gateway.limits.total_timeout.as_millis()
    {
        return Err(RelayError::RouteUnavailable);
    }
    routing::require_cache_tariff(
        request
            .native()
            .is_some_and(|r| r.cache_lifetime_requested()),
        attempt.protocol(),
        &price,
    )?;
    let continuity = if request.kind().responses() {
        gateway
            .bridge
            .provider_policy
            .continuity_contract(&attempt)?
    } else {
        None
    };
    if request
        .native()
        .is_some_and(|r| r.previous().is_some() || r.store() == Some(true))
        && continuity.is_none()
    {
        return Err(RelayError::Unsupported(
            "provider does not support stored Responses continuity",
        ));
    }
    if let Some(parent) = &parent {
        parent.matches(
            &attempt,
            plan.canonical_model(),
            continuity
                .as_ref()
                .ok_or(RelayError::Invalid("continuation unavailable"))?,
        )?;
    }
    let client_handle = continuity
        .as_ref()
        .map(|_| super::responses_continuity::handle(id));
    if request.kind().responses() {
        if request.kind() == Kind::ResponsesCompact
            || continuity.is_none()
            || request.native().is_some_and(|r| r.opaque())
        {
            gateway
                .bridge
                .provider_policy
                .require_compaction(&attempt)?;
        }
        routing::require_native_responses_price(&price)?;
    }
    let output_max = request.output_limit(
        gateway
            .bridge
            .provider_policy
            .output_bound(&attempt, gateway.limits.max_output_tokens)?,
    )?;
    let input_max = gateway.bridge.provider_policy.reservation_input_bound(
        &attempt,
        gateway.bridge.policy.input_token_ceiling,
        output_max,
    )?;
    request.validate_bound(input_max as u64)?;
    let reference = attempt.credential_ref();
    let credential = gateway
        .bridge
        .credentials
        .get(&(reference.id, reference.revision))
        .ok_or(RelayError::RouteUnavailable)?;
    let mut endpoint = endpoint(attempt.endpoint(), protocol)?;
    if request.kind() == Kind::ResponsesCompact {
        endpoint
            .path_segments_mut()
            .map_err(|_| RelayError::RouteUnavailable)?
            .push("compact");
    }
    let connection = tokio::time::timeout(
        gateway.limits.total_timeout,
        gateway
            .native_http
            .transport
            .prepare(&endpoint, &gateway.limits),
    )
    .await
    .map_err(|_| RelayError::RouteUnavailable)??;
    let mut wire = if continuity.is_some() {
        request
            .native()
            .ok_or(RelayError::Invalid("native continuation request"))?
            .continuity_wire(attempt.upstream_model())?
    } else {
        request.wire(attempt.upstream_model())?
    };
    if let Some(parent) = &parent {
        let mut value: serde_json::Value = serde_json::from_slice(&wire)
            .map_err(|_| RelayError::Invalid("native parent encoding"))?;
        value["previous_response_id"] = parent.provider_response_id.clone().into();
        wire = serde_json::to_vec(&value)
            .map_err(|_| RelayError::Invalid("native parent encoding"))?;
    }
    if request.kind() == Kind::ResponsesContext {
        let mut v: serde_json::Value = serde_json::from_slice(&wire)
            .map_err(|_| RelayError::Invalid("native Responses wire"))?;
        v["max_output_tokens"] = output_max.into();
        wire = serde_json::to_vec(&v).map_err(|_| RelayError::Invalid("native Responses wire"))?;
    }
    if wire.len() > gateway.limits.request_bytes {
        return Err(RelayError::Limit("input-only outbound bytes"));
    }
    let upstream_request = connection
        .client()
        .post(endpoint)
        .bearer_auth(&credential)
        .header(
            header::ACCEPT,
            if streaming {
                "text/event-stream"
            } else {
                "application/json"
            },
        )
        .header(header::CONTENT_TYPE, "application/json")
        .body(wire.clone())
        .build()
        .map_err(|_| RelayError::Invalid("input-only provider request"))?;
    let reserve = i64::try_from(
        quote
            .reservation_ceiling(input_max as u64, output_max as u64)
            .map_err(|_| RelayError::BillingUnavailable)?
            .0,
    )
    .map_err(|_| RelayError::BillingUnavailable)?;
    let mut fingerprint = Sha256::new();
    fingerprint.update(b"lmm-input-only-v1\0");
    fingerprint.update(request.kind().name().as_bytes());
    fingerprint.update([0]);
    fingerprint.update(group.as_bytes());
    fingerprint.update([0]);
    fingerprint.update(if continuity.is_some() {
        request
            .native()
            .ok_or(RelayError::Invalid("native continuation request"))?
            .continuity_wire(request.model())?
    } else {
        request.wire(request.model())?
    });
    let fingerprint: [u8; 32] = fingerprint.finalize().into();
    let mut worker = [0; 32];
    rand::rand_core::UnwrapErr(SysRng).fill_bytes(&mut worker);
    let route_id = attempt.route_key();
    let (provider_revision, semantics) = gateway
        .bridge
        .provider_policy
        .reservation_metadata(&attempt)?;
    let binding = charging::contract::ReservationBinding::for_attempt(
        &attempt,
        if request.kind().responses() {
            "responses".into()
        } else {
            request.kind().name().into()
        },
        attempt.upstream_model().into(),
        upstream_request.url().as_str().into(),
        provider_revision,
        if request.kind().responses() {
            "native-json-wire-v1".into()
        } else {
            "input-only-wire-v1".into()
        },
        attempt.quote().version(),
    )
    .map_err(billing::error)?;
    #[cfg(test)]
    gateway
        .bridge
        .observe_prepared_binding(selected_trace, &attempt, &binding);
    let prepared = if request.kind().responses() {
        charging::contract::PreparedReservation::context_max(
            charging::contract::hex_digest(fingerprint),
            binding,
            charging::contract::digest(&wire),
            charging::contract::ReservationBounds {
                model_input_max: input_max as u64,
                input: input_max as u64,
                requested_output: output_max as u64,
                output: output_max as u64,
            },
            semantics,
        )
    } else {
        charging::contract::PreparedReservation::input_only(
            charging::contract::hex_digest(fingerprint),
            binding,
            charging::contract::digest(&wire),
            input_max as u64,
            semantics,
        )
    }
    .map_err(billing::error)?;
    crate::request_limits::admit(&gateway.bridge.pool, caller.principal.user_id(), &group, id)
        .await
        .map_err(routing::admission_error)?;
    Ok(PreparedRequest {
        parent,
        continuity,
        client_handle,
        group,
        attempt,
        quote,
        price,
        input_max,
        output_max,
        credential,
        connection,
        upstream_request,
        worker,
        fingerprint,
        route_id,
        prepared,
        reserve,
    })
}

/// Trusted terminal handling: exact client bytes, durable receipt, original
/// settlement, then activation and publication. UNKNOWN retains its hold.
struct TerminalFinalization<'a> {
    gateway: &'a ModelGateway,
    request: &'a InputRequest,
    id: &'a str,
    worker: &'a [u8; 32],
    attempt: &'a crate::routing::Attempt,
    parent: Option<&'a super::responses_continuity::Parent>,
    continuity: Option<&'a super::responses_continuity::Policy>,
    client_handle: Option<&'a str>,
    credential: &'a str,
    stream: &'a Option<super::responses_native::Sender>,
    terminal_event: Option<Bytes>,
    sequence: u64,
}
impl TerminalFinalization<'_> {
    async fn finish(self, raw: Vec<u8>) -> Result<Bytes, RelayError> {
        let Self {
            gateway,
            request,
            id,
            worker,
            attempt,
            parent,
            continuity,
            client_handle,
            credential,
            stream,
            terminal_event,
            sequence,
        } = self;
        let (mut output, observed) = request.response(&raw)?;
        let candidate = if request.kind() == Kind::ResponsesContext {
            if let Some(policy) = &continuity {
                let stored = request
                    .native()
                    .and_then(|r| r.store())
                    .unwrap_or(policy.default_store);
                if output
                    .get("store")
                    .is_some_and(|v| v.as_bool() != Some(stored))
                {
                    return Err(RelayError::Protocol(
                        "provider storage receipt contradicts contract",
                    ));
                }
                if stored {
                    Some(super::responses_continuity::Candidate {
                        provider_response_id: output["id"]
                            .as_str()
                            .ok_or(RelayError::SettlementPending)?
                            .into(),
                        parent_request_id: parent.as_ref().map(|p| p.request_id.clone()),
                        public_model: request.model().into(),
                        policy_sha256: crate::hex_codec::decode(&policy.sha256)
                            .map_err(|_| RelayError::SettlementPending)?,
                    })
                } else {
                    None
                }
            } else {
                None
            }
        } else {
            None
        };
        if let Some(handle) = &client_handle {
            output["id"] = (*handle).into();
        }
        let normalized = serde_json::to_vec(&output).map_err(|_| RelayError::SettlementPending)?;
        if std::str::from_utf8(&raw).is_ok_and(|value| value.contains(credential))
            || std::str::from_utf8(&normalized).is_ok_and(|value| value.contains(credential))
        {
            return Err(RelayError::SettlementPending);
        }
        gateway
            .bridge
            .provider_policy
            .validate_usage(attempt, &observed, false)?;
        let (frozen, contract) = gateway
            .bridge
            .charging
            .reservation_contract(id)
            .await
            .map_err(billing::error)?;
        let usage = billing::frozen_priced_usage(&contract, &frozen, observed.clone())?;
        // Encode the JSON client body once, then checkpoint and return these same bytes.
        let client_json = Bytes::from(normalized);
        let client_packet = terminal_event.as_ref().unwrap_or(&client_json);
        gateway
            .bridge
            .persist_output_evidence(
                &relay::Reservation { id: id.to_owned() },
                &relay::OutputCheckpoint {
                    sequence,
                    packet_sha256: Sha256::digest(client_packet).into(),
                    packet_bytes: client_packet.len(),
                    usage: observed,
                    terminal_observed: true,
                },
                match candidate.as_ref() {
                    Some(candidate) => {
                        super::checkpoints::TerminalEvidence::Continuity { candidate }
                    }
                    None => super::checkpoints::TerminalEvidence::Ordinary,
                },
            )
            .await?;
        let settled = billing::retry(|| {
            gateway
                .bridge
                .charging
                .settle(id, worker, usage, Outcome::Completed)
        })
        .await;
        let settled = if candidate.is_some()
            && settled.is_ok()
            && super::responses_continuity::lose_ack_once("settlement")
        {
            Err(charging::Error::CommitUnknown)
        } else {
            settled
        };
        let settled = match settled {
            Ok(settled) => Some(settled),
            Err(charging::Error::CommitUnknown) => gateway
                .bridge
                .charging
                .resolve(id)
                .await
                .map_err(billing::error)?,
            _ => return Err(RelayError::SettlementPending),
        };
        if !settled.as_ref().is_some_and(|settled| {
            billing::confirmed_settlement(settled, id, &frozen.price, usage, Outcome::Completed)
        }) {
            return Err(RelayError::SettlementPending);
        }
        if candidate.is_some() {
            super::responses_continuity::crash_at("after_settlement");
            super::responses_continuity::activate(&gateway.bridge, id).await?;
            super::responses_continuity::crash_at("after_activation");
        }
        if let (Some(sender), Some(event)) = (&stream, terminal_event) {
            super::responses_native::send_packet(sender, event, &gateway.limits).await?;
        }
        Ok(client_json)
    }
}

async fn execute(
    gateway: ModelGateway,
    caller: native_http::Caller,
    request: InputRequest,
    id: String,
    _permit: OwnedSemaphorePermit,
    stream: Option<super::responses_native::Sender>,
    ready: Option<Ready>,
) -> Result<Bytes, RelayError> {
    let PreparedRequest {
        parent,
        continuity,
        client_handle,
        group,
        attempt,
        quote,
        price,
        input_max,
        output_max,
        credential,
        connection,
        upstream_request,
        worker,
        fingerprint,
        route_id,
        prepared,
        reserve,
    } = prepare_request(&gateway, &caller, &request, &id, stream.is_some()).await?;
    {
        let mut active = gateway
            .bridge
            .active
            .lock()
            .map_err(|_| RelayError::BillingUnavailable)?;
        if active.contains_key(&id) {
            return Err(RelayError::Conflict);
        }
        active.insert(
            id.clone(),
            Operation {
                route_plan: None,
                secret: caller.secret.clone(),
                group,
                worker,
                fingerprint,
                selected: Some(routing::Selected {
                    attempt: attempt.clone(),
                    price: price.clone(),
                    quote,
                    route_id,
                    input_token_ceiling: input_max,
                }),
                output_token_ceiling: output_max,
                peer: caller.peer,
                oauth_issuer: None,
            },
        );
    }
    let _active = native_http::Active {
        bridge: gateway.bridge.clone(),
        id: id.clone(),
    };
    let spec = charging::Request {
        id: id.clone(),
        fingerprint,
        price,
        reserve,
        lease_seconds: gateway.bridge.policy.lease_seconds,
        maximum_seconds: gateway.bridge.policy.lease_seconds,
    };
    let basis = charging::ReservationBasis::Tokens {
        input: input_max as u64,
        output: u64::from(output_max),
    };
    let charge = billing::retry(|| {
        gateway.bridge.charging.reserve_prepared_model_with_context(
            &caller.secret,
            &spec,
            &worker,
            caller.peer,
            None,
            basis,
            &prepared,
        )
    })
    .await;
    let charge = match charge {
        Err(charging::Error::CommitUnknown) => {
            gateway
                .bridge
                .charging
                .resolve_prepared_model(&spec, &worker, basis, &prepared)
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
    gateway.bridge.freeze_selected_price(&id, &charge)?;
    if !attempt.is_available() {
        let _ = billing::retry(|| gateway.bridge.charging.release(&id, &worker)).await;
        return Err(RelayError::RouteUnavailable);
    }
    let dispatch = billing::retry(|| {
        gateway
            .bridge
            .charging
            .start_model_authorized(&id, &worker, &caller.secret, caller.peer)
    })
    .await;
    let dispatch = match dispatch {
        Err(error @ (charging::Error::Unauthorized | charging::Error::Forbidden)) => {
            // This transaction rejected dispatch, so releasing its original hold is known safe.
            billing::retry(|| gateway.bridge.charging.release(&id, &worker))
                .await
                .map_err(billing::error)?;
            return Err(billing::error(error));
        }
        result => result.map_err(billing::error)?,
    };
    if !dispatch {
        return Err(RelayError::Conflict);
    }
    if let Some(ready) = &ready {
        notify_ready(ready, Ok(()));
    }
    let mut terminal_event = None;
    let mut sequence = 1;
    let completed = async {
        let raw = tokio::time::timeout(gateway.limits.total_timeout, async {
            if !attempt.is_available() {
                return Err(RelayError::RouteUnavailable);
            }
            let response = tokio::time::timeout(
                gateway.limits.header_timeout,
                connection.client().execute(upstream_request),
            )
            .await
            .map_err(|_| RelayError::SettlementPending)?
            .map_err(|_| RelayError::SettlementPending)?;
            // This API has no supplier contract proving free non-successful dispatches.
            if !response.status().is_success() {
                return Err(RelayError::SettlementPending);
            }
            if let Some(sender) = &stream {
                let (response, terminal, next) = super::responses_native::read_stream(
                    &gateway,
                    &id,
                    &connection,
                    response,
                    sender,
                    &credential,
                    client_handle.as_deref(),
                )
                .await?;
                terminal_event = Some(terminal);
                sequence = next;
                Ok(response)
            } else {
                native_http::read_response_bytes(&connection, response, &gateway.limits).await
            }
        })
        .await
        .map_err(|_| RelayError::SettlementPending)??;
        TerminalFinalization {
            gateway: &gateway,
            request: &request,
            id: &id,
            worker: &worker,
            attempt: &attempt,
            parent: parent.as_ref(),
            continuity: continuity.as_ref(),
            client_handle: client_handle.as_deref(),
            credential: &credential,
            stream: &stream,
            terminal_event,
            sequence,
        }
        .finish(raw)
        .await
    }
    .await;
    if completed.is_err() {
        let _ = billing::retry(|| gateway.bridge.charging.mark_unknown(&id, &worker)).await;
    }
    completed
}

//! Core-owned admission of a turn on a Responses WebSocket session.
use super::{Bridge, ModelGateway, Operation, RequestContext, billing, routing, session_affinity};
use crate::relay::{
    Billing, ClientProtocol, Outcome, OutputCheckpoint, Phase, RelayError, Report, Request,
    Reservation, ResponsesSession, Route, RouteProvider, UpstreamProtocol,
    provider_network::ResponsesConnection,
    provider_usage_with_cache_creation_mode,
    responses_ws::{Attribution, Correlation, Create, PreparedCreate, TerminalOutcome, turn_id},
};
use axum::{
    extract::{
        ConnectInfo, State,
        rejection::ExtensionRejection,
        ws::{Message, WebSocket, WebSocketUpgrade},
    },
    http::HeaderMap,
    response::Response,
};
use rand::{Rng, rngs::SysRng};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use std::{
    net::{IpAddr, SocketAddr},
    sync::Arc,
};
use tokio::time::{Instant, timeout, timeout_at};
use tokio_tungstenite::tungstenite::Message as ProviderMessage;

pub(super) struct SessionTurn<'a> {
    pub(super) session: &'a [u8; 32],
    pub(super) sequence: u64,
}

pub(super) struct Turn {
    bridge: Arc<Bridge>,
    context: RequestContext,
    request: Request,
    pub(super) route: Route,
    pub(super) target_id: u64,
    pub(super) frame: PreparedCreate,
}

pub(super) async fn upgrade(
    State(gateway): State<ModelGateway>,
    headers: HeaderMap,
    peer: Result<ConnectInfo<SocketAddr>, ExtensionRejection>,
    socket: WebSocketUpgrade,
) -> Response {
    if let Err(error) = check_origin(&headers, &gateway.responses_websocket_origins) {
        return super::native::error_response(ClientProtocol::Responses, error);
    }
    if let Err(error) = gateway.authenticate(&headers).await {
        return super::native::error_response(ClientProtocol::Responses, error);
    }
    let lease = match gateway.relay.admit_responses_ws() {
        Ok(lease) => lease,
        Err(error) => return super::native::error_response(ClientProtocol::Responses, error),
    };
    let request_bytes = gateway.limits.request_bytes;
    socket
        .protocols(["realtime", "responses"])
        .max_message_size(request_bytes)
        .max_frame_size(request_bytes)
        .on_upgrade(move |client| {
            serve(
                gateway,
                lease,
                headers,
                peer.ok().map(|peer| peer.0.ip()),
                client,
            )
        })
}

async fn serve(
    gateway: ModelGateway,
    lease: ResponsesSession,
    headers: HeaderMap,
    peer: Option<IpAddr>,
    mut client: WebSocket,
) {
    let mut session = [0; 32];
    rand::rand_core::UnwrapErr(SysRng).fill_bytes(&mut session);
    let mut correlation = Correlation::default();
    let mut upstream: Option<ResponsesConnection> = None;
    let mut provider_secret: Option<String> = None;
    let mut provider_closed = false;
    let mut idle_bytes = 0_usize;
    let mut pinned_target = None;
    let mut pinned_model: Option<String> = None;
    let mut previous_responses = std::collections::BTreeSet::<String>::new();
    loop {
        let next = tokio::select! {
            biased;
            () = lease.cancelled() => break,
            message = async {
                match upstream.as_mut() {
                    Some(socket) if !provider_closed => socket.receive().await,
                    _ => std::future::pending().await,
                }
            } => {
                let raw = match message {
                    Ok(Some(ProviderMessage::Text(raw))) => raw,
                    Ok(Some(ProviderMessage::Ping(_) | ProviderMessage::Pong(_))) => continue,
                    _ => {
                        provider_closed = true;
                        continue;
                    },
                };
                idle_bytes = match idle_bytes.checked_add(raw.len()) {
                    Some(bytes) if bytes <= gateway.limits.output_bytes => bytes,
                    _ => break,
                };
                let Ok(mut event) = serde_json::from_str::<Value>(raw.as_str()) else { break; };
                if provider_secret.as_deref().is_some_and(|secret| contains_credential(&event,secret)) { break; }
                match correlation.control_error(&mut event) {
                    Ok(true) => if write(&mut client,&gateway.limits,event).await.is_err() { break; },
                    Ok(false) => (), // Old terminal/foreign output cannot settle a new turn.
                    Err(_) => break,
                }
                continue;
            },
            result = timeout(gateway.limits.total_timeout, client.recv()) => result,
        };
        let raw = match next {
            Ok(Some(Ok(Message::Text(raw)))) => raw,
            Ok(Some(Ok(Message::Ping(_) | Message::Pong(_)))) => continue,
            _ => break,
        };
        let kind = serde_json::from_str::<Value>(raw.as_str())
            .ok()
            .and_then(|event| event["type"].as_str().map(str::to_owned));
        if kind
            .as_deref()
            .is_some_and(|kind| kind != "response.create")
        {
            let result = match upstream.as_mut() {
                Some(socket) => match correlation
                    .authorize_control(raw.as_bytes(), gateway.limits.request_bytes)
                {
                    Ok(()) => socket.send_control(raw.as_str()).await,
                    Err(error) => Err(RelayError::Invalid(error.0)),
                },
                None => Err(RelayError::Invalid(
                    "first responses websocket event must be response.create",
                )),
            };
            if let Err(error) = result {
                let _ = write(
                    &mut client,
                    &gateway.limits,
                    event_error(raw.as_bytes(), &error),
                )
                .await;
                if !matches!(error, RelayError::Invalid(_)) {
                    break;
                }
            }
            continue;
        }
        let mut admitted_sequence = None;
        let mut reserved = false;
        let result = async {
            let create = Create::parse(raw.as_bytes(), gateway.limits.request_bytes)
                .map_err(|error| RelayError::Invalid(error.0))?;
            if let Some(model) = &pinned_model {
                let body: Value = serde_json::from_slice(&create.request_json)
                    .map_err(|_| RelayError::Invalid("invalid create"))?;
                if body["model"].as_str() != Some(model.as_str()) {
                    return Err(RelayError::Invalid(
                        "Responses WebSocket connection is locked to its model",
                    ));
                }
            }
            let sequence = correlation
                .begin(&create)
                .map_err(|error| RelayError::Invalid(error.0))?;
            admitted_sequence = Some(sequence);
            let turn = Turn::prepare(
                &gateway,
                &headers,
                peer,
                SessionTurn {
                    session: &session,
                    sequence,
                },
                &create,
                pinned_target,
                create
                    .previous_response_id
                    .as_deref()
                    .filter(|id| previous_responses.contains(*id)),
            )
            .await?;
            // A closed supplier connection cannot be resumed or redispatched, but
            // each later client turn still passes the live authorization gate.
            if provider_closed {
                return Err(
                    if upstream
                        .as_ref()
                        .is_some_and(|socket| socket.accepts(&turn.route))
                    {
                        RelayError::UpstreamConnection
                    } else {
                        RelayError::RouteUnavailable
                    },
                );
            }
            if upstream.is_none() {
                upstream = Some(lease.upgrade(&turn.route).await?);
                provider_secret = turn
                    .route
                    .credential
                    .to_str()
                    .ok()
                    .and_then(|value| value.strip_prefix("Bearer "))
                    .map(str::to_owned);
                pinned_target = Some(turn.target_id);
                pinned_model = Some(turn.request.model.clone());
            }
            let upstream = upstream.as_mut().ok_or(RelayError::UpstreamConnection)?;
            if !upstream.accepts(&turn.route) {
                return Err(RelayError::RouteUnavailable);
            }
            let reservation = turn.reserve(&create, &gateway.limits).await?;
            reserved = true;
            #[cfg(test)]
            test_gate("reserved").await;
            let mut report = Report {
                request_id: reservation.id.clone(),
                route_id: Some(turn.route.id.clone()),
                price_version: Some(turn.route.price_version),
                outcome: Outcome::Failed,
                usage: Default::default(),
                finish_reason: None,
                provider_complete: false,
                upstream_attempted: false,
                delivery: crate::routing::DeliveryState::NotSent,
                rejection_receipt_sha256: None,
                attempts: 1,
                upstream_status: None,
                upstream_bytes: 0,
                emitted_bytes: 0,
                timings: Default::default(),
            };
            let mut client_open = true;
            let outcome = async {
                turn.record_dispatch().await?;
                let sent = upstream.send_prepared(&turn.route, &turn.frame).await;
                if !matches!(
                    sent,
                    Err(RelayError::RouteUnavailable | RelayError::Invalid(_))
                ) {
                    report.upstream_attempted = true;
                    report.delivery = crate::routing::DeliveryState::PossiblyAccepted;
                }
                sent?;
                #[cfg(test)]
                test_gate("dispatched").await;
                report.upstream_status = Some(101);
                read_turn(
                    &gateway.limits,
                    &lease,
                    &turn,
                    &reservation,
                    TurnStream {
                        correlation: &mut correlation,
                        upstream,
                        client: &mut client,
                        client_open: &mut client_open,
                        report: &mut report,
                    },
                )
                .await
            }
            .await;
            let finalized = turn.finalize(&reservation, &report).await;
            let terminal = outcome?;
            finalized?;
            #[cfg(test)]
            test_gate("settled").await;
            correlation
                .finish(sequence)
                .map_err(|error| RelayError::Protocol(error.0))?;
            if let Some(id) = terminal["response"]["id"].as_str() {
                previous_responses.insert(id.to_owned());
            }
            if client_open {
                write(&mut client, &gateway.limits, terminal).await?;
            }
            Ok::<_, RelayError>(client_open)
        }
        .await;
        match result {
            Ok(true) => (),
            Ok(false) => break,
            Err(error) => {
                let _ = write(
                    &mut client,
                    &gateway.limits,
                    event_error(raw.as_bytes(), &error),
                )
                .await;
                if reserved {
                    break;
                }
                if let Some(sequence) = admitted_sequence {
                    correlation.reject_unreserved(sequence);
                }
            }
        }
    }
}

async fn write(
    client: &mut WebSocket,
    limits: &crate::relay::Limits,
    value: Value,
) -> Result<(), RelayError> {
    timeout(
        limits.write_timeout,
        client.send(Message::Text(value.to_string().into())),
    )
    .await
    .map_err(|_| RelayError::Timeout(Phase::Write))?
    .map_err(|_| RelayError::Cancelled)
}

struct TurnStream<'a> {
    correlation: &'a mut Correlation,
    upstream: &'a mut ResponsesConnection,
    client: &'a mut WebSocket,
    client_open: &'a mut bool,
    report: &'a mut Report,
}
async fn read_turn(
    limits: &crate::relay::Limits,
    lease: &ResponsesSession,
    turn: &Turn,
    reservation: &Reservation,
    stream: TurnStream<'_>,
) -> Result<Value, RelayError> {
    let TurnStream {
        correlation,
        upstream,
        client,
        client_open,
        report,
    } = stream;
    let deadline = Instant::now() + limits.total_timeout;
    let mut sequence = 0_u64;
    let credential = turn
        .route
        .credential
        .to_str()
        .ok()
        .and_then(|value| value.strip_prefix("Bearer "))
        .filter(|value| !value.is_empty())
        .ok_or(RelayError::RouteUnavailable)?;
    loop {
        let message = tokio::select! {
            biased;
            () = lease.cancelled() => return Err(RelayError::Cancelled),
            message = timeout_at(deadline, upstream.receive()) => message.map_err(|_| RelayError::Timeout(Phase::Total))??,
            message = client.recv(), if *client_open => {
                match message {
                    Some(Ok(Message::Text(raw))) => {
                        if serde_json::from_str::<Value>(raw.as_str()).ok()
                            .is_some_and(|event|event["type"]=="response.create") {
                            *client_open = write(client,limits,event_error(raw.as_bytes(),&RelayError::Conflict)).await.is_ok();
                            continue;
                        }
                        match correlation.authorize_control(raw.as_bytes(), limits.request_bytes) {
                            Ok(()) => upstream.send_control(raw.as_str()).await?,
                            Err(error) => {
                                let error = RelayError::Invalid(error.0);
                                *client_open = write(client, limits, event_error(raw.as_bytes(), &error)).await.is_ok();
                            }
                        }
                    }
                    Some(Ok(Message::Ping(_) | Message::Pong(_))) => (),
                    _ => *client_open = false,
                }
                continue;
            }
        };
        let raw = match message {
            Some(ProviderMessage::Text(raw)) => raw,
            Some(ProviderMessage::Ping(_) | ProviderMessage::Pong(_)) => continue,
            _ => return Err(RelayError::UpstreamConnection),
        };
        report.upstream_bytes = report
            .upstream_bytes
            .checked_add(raw.len())
            .filter(|count| *count <= limits.output_bytes)
            .ok_or(RelayError::Limit("WebSocket output"))?;
        let mut event: Value = serde_json::from_str(raw.as_str())
            .map_err(|_| RelayError::Protocol("invalid WebSocket event"))?;
        if contains_credential(&event, credential) {
            return Err(RelayError::Protocol("invalid upstream event"));
        }
        if correlation
            .control_error(&mut event)
            .map_err(|error| RelayError::Protocol(error.0))?
        {
            if *client_open {
                *client_open = write(client, limits, event).await.is_ok();
            }
            continue;
        }
        match correlation
            .attribute(&event)
            .map_err(|error| RelayError::Protocol(error.0))?
        {
            Attribution::Active => (),
            Attribution::Control if *client_open => {
                *client_open = write(client, limits, event).await.is_ok();
                continue;
            }
            _ => continue,
        }
        correlation.decorate_active(&mut event);
        let terminal = crate::relay::responses_ws::terminal(&event)
            .map_err(|error| RelayError::Protocol(error.0))?;
        if let Some(terminal) = &terminal {
            report.usage = provider_usage_with_cache_creation_mode(
                UpstreamProtocol::Responses,
                terminal.usage.unwrap_or(&Value::Null),
                turn.route.cache_creation_mode,
            )?;
            report.provider_complete = true;
            report.outcome = match terminal.outcome {
                TerminalOutcome::Completed | TerminalOutcome::Incomplete => Outcome::Completed,
                TerminalOutcome::Cancelled => Outcome::Cancelled,
                TerminalOutcome::Failed => Outcome::Failed,
            };
        }
        sequence = sequence
            .checked_add(1)
            .filter(|n| *n <= 65_536)
            .ok_or(RelayError::Limit("output checkpoints"))?;
        let encoded = event.to_string();
        let checkpoint = OutputCheckpoint {
            sequence,
            packet_sha256: Sha256::digest(encoded.as_bytes()).into(),
            packet_bytes: encoded.len(),
            usage: report.usage.clone(),
            terminal_observed: terminal.is_some(),
        };
        turn.bridge.checkpoint(reservation, &checkpoint).await?;
        if terminal.is_some() {
            #[cfg(test)]
            test_gate("checkpoint").await;
            return Ok(event);
        }
        if *client_open {
            *client_open = write(client, limits, event).await.is_ok();
        }
    }
}

fn contains_credential(value: &Value, credential: &str) -> bool {
    match value {
        Value::String(value) => value.contains(credential),
        Value::Array(values) => values
            .iter()
            .any(|value| contains_credential(value, credential)),
        Value::Object(fields) => fields
            .iter()
            .any(|(key, value)| key.contains(credential) || contains_credential(value, credential)),
        _ => false,
    }
}

impl Drop for Turn {
    fn drop(&mut self) {
        if let Ok(mut active) = self.bridge.active.lock() {
            active.remove(&self.context.request_id);
        }
    }
}

impl Turn {
    pub(super) async fn prepare(
        gateway: &ModelGateway,
        headers: &HeaderMap,
        peer: Option<IpAddr>,
        session_turn: SessionTurn<'_>,
        create: &Create,
        pinned_target: Option<u64>,
        previous_response: Option<&str>,
    ) -> Result<Self, RelayError> {
        let SessionTurn { session, sequence } = session_turn;
        if let Some(previous) = create.previous_response_id.as_deref()
            && (Some(previous) != previous_response || pinned_target.is_none())
        {
            return Err(RelayError::Forbidden);
        }
        let (principal, secret, groups) = gateway.authenticate(headers).await?;
        let request = Request::parse_responses_ws(
            &create.request_json,
            &gateway.limits,
            create.previous_response_id.is_some(),
            create.generate,
        )?;
        let plans = gateway
            .bridge
            .routing
            .route_responses_ws_groups(
                &request.model,
                &groups,
                session_affinity(headers, &principal)?,
                pinned_target,
            )
            .map_err(|_| RelayError::RouteUnavailable)?;
        if plans.iter().any(|plan| {
            gateway
                .audio_rules
                .iter()
                .any(|rule| rule.price.model == plan.canonical_model())
        }) {
            return Err(RelayError::Unsupported(
                "audio model cannot use text billing",
            ));
        }
        let authorized = gateway
            .authorize_model_plans(&principal, &secret, peer, plans)
            .await?;
        let group = authorized[0].group().to_owned();
        let actor = Sha256::digest(secret.as_bytes()).into();
        let id = crate::billing::charging::contract::hex_digest(turn_id(
            &actor,
            session,
            sequence,
            create.event_id.as_deref(),
        ));
        let context = RequestContext {
            request_id: id.clone(),
            actor_user_id: principal.user_id(),
            key_id: principal.key_id(),
            oauth_source: principal.oauth_source().cloned(),
        };
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
        crate::request_limits::admit(&gateway.bridge.pool, principal.user_id(), &group, &id)
            .await
            .map_err(routing::admission_error)?;
        let mut worker = [0; 32];
        rand::rand_core::UnwrapErr(SysRng).fill_bytes(&mut worker);
        {
            let mut active = gateway
                .bridge
                .active
                .lock()
                .map_err(|_| RelayError::BillingUnavailable)?;
            if active.contains_key(&id) {
                return Err(RelayError::Conflict);
            }
            if active.len() >= gateway.limits.max_in_flight {
                return Err(RelayError::Busy);
            }
            active.insert(
                id.clone(),
                Operation {
                    route_plan: Some(routing::Pending::Planned(authorized)),
                    secret,
                    group,
                    worker,
                    fingerprint: [0; 32],
                    selected: None,
                    output_token_ceiling: request.max_output_tokens,
                    peer,
                    oauth_issuer: principal.oauth_source().and(gateway.oauth2.clone()),
                },
            );
        }
        let selected = (|| {
            let route = gateway.bridge.select_with_fallbacks(&context, &request)?;
            if !matches!(
                route.count_plan,
                crate::relay::input_count::CountPlan::ContextMax
            ) {
                return Err(RelayError::Unsupported(
                    "Responses WebSocket requires context-max reservation",
                ));
            }
            let frame = create.prepare_selected(&request, &route, &gateway.limits)?;
            let mut active = gateway
                .bridge
                .active
                .lock()
                .map_err(|_| RelayError::BillingUnavailable)?;
            let operation = active.get_mut(&id).ok_or(RelayError::Conflict)?;
            operation.fingerprint = frame.frame_commitment;
            let target_id = operation
                .selected
                .as_ref()
                .ok_or(RelayError::RouteUnavailable)?
                .attempt
                .target_id();
            Ok(Self {
                bridge: gateway.bridge.clone(),
                context,
                request,
                route,
                target_id,
                frame,
            })
        })();
        if selected.is_err()
            && let Ok(mut active) = gateway.bridge.active.lock()
        {
            active.remove(&id);
        }
        selected
    }

    pub(super) async fn reserve(
        &self,
        create: &Create,
        limits: &crate::relay::Limits,
    ) -> Result<Reservation, RelayError> {
        self.bridge
            .reserve_responses_ws(
                &self.context,
                &self.route,
                &self.request,
                create,
                &self.frame,
                limits,
            )
            .await
    }

    pub(super) async fn record_dispatch(&self) -> Result<(), RelayError> {
        if self.frame.wire_sha256 != <[u8; 32]>::from(Sha256::digest(&self.frame.wire_bytes)) {
            return Err(RelayError::Conflict);
        }
        self.bridge
            .dispatching(&self.context, &self.route, &self.frame.wire_bytes)
            .await
    }

    pub(super) async fn finalize(
        &self,
        reservation: &Reservation,
        report: &Report,
    ) -> Result<(), RelayError> {
        self.bridge.finalize(reservation, report).await
    }
}

/// Only GET /v1/responses Upgrade consumes browser API-key subprotocols. This
/// outer middleware runs before legacy aliases; POST/other APIs are unchanged.
#[derive(Clone)]
pub(super) struct BrowserProtocols;

pub(super) async fn credentials(
    mut request: axum::extract::Request,
    next: axum::middleware::Next,
) -> Response {
    if request.method() == axum::http::Method::GET
        && request.uri().path() == "/v1/responses"
        && request
            .headers()
            .get(axum::http::header::UPGRADE)
            .and_then(|v| v.to_str().ok())
            .is_some_and(|value| value.eq_ignore_ascii_case("websocket"))
    {
        if let Err(error) = browser_credentials(request.headers_mut()) {
            return super::native::error_response(ClientProtocol::Responses, error);
        }
        request.extensions_mut().insert(BrowserProtocols);
    }
    next.run(request).await
}

fn browser_credentials(headers: &mut HeaderMap) -> Result<(), RelayError> {
    let protocols = super::one_header(headers, "sec-websocket-protocol")?
        .unwrap_or("")
        .to_owned();
    let mut found = false;
    let mut safe = Vec::new();
    for part in protocols.split(',').map(str::trim) {
        if let Some(key) = part.strip_prefix("openai-insecure-api-key.") {
            if !found {
                found = true;
                if !key.is_empty() {
                    let mut value = axum::http::HeaderValue::from_str(&format!("Bearer {key}"))
                        .map_err(|_| RelayError::Unauthorized)?;
                    value.set_sensitive(true);
                    headers.insert(axum::http::header::AUTHORIZATION, value);
                }
            }
        } else if matches!(part, "responses" | "realtime") {
            safe.push(part);
        }
    }
    headers.remove("sec-websocket-protocol");
    if !safe.is_empty() {
        headers.insert(
            "sec-websocket-protocol",
            axum::http::HeaderValue::from_str(&safe.join(", "))
                .map_err(|_| RelayError::Invalid("invalid WebSocket protocols"))?,
        );
    }
    Ok(())
}

pub(super) fn parse_origin(raw: &str) -> Result<reqwest::Url, RelayError> {
    let origin =
        reqwest::Url::parse(raw).map_err(|_| RelayError::Invalid("invalid WebSocket origin"))?;
    if !matches!(origin.scheme(), "http" | "https")
        || origin.host_str().is_none()
        || !origin.username().is_empty()
        || origin.password().is_some()
        || origin.path() != "/"
        || origin.query().is_some()
        || origin.fragment().is_some()
    {
        return Err(RelayError::Invalid("invalid WebSocket origin"));
    }
    Ok(origin)
}
fn loopback(origin: &reqwest::Url) -> bool {
    origin.host_str().is_some_and(|host| {
        host.trim_end_matches('.').eq_ignore_ascii_case("localhost")
            || host
                .trim_matches(['[', ']'])
                .parse::<IpAddr>()
                .is_ok_and(|ip| ip.is_loopback())
    })
}
fn check_origin(headers: &HeaderMap, trusted: &[reqwest::Url]) -> Result<(), RelayError> {
    let Some(raw) = super::one_header(headers, "origin").map_err(|_| RelayError::Forbidden)? else {
        return Ok(());
    };
    let origin = parse_origin(raw).map_err(|_| RelayError::Forbidden)?;
    let host = super::one_header(headers, "host")
        .map_err(|_| RelayError::Forbidden)?
        .ok_or(RelayError::Forbidden)?;
    // Core's existing public listener is plaintext behind the TLS ingress.
    // Preserve the fixed Go forwarded-scheme contract; the ingress must replace
    // this header. Never infer a request's scheme from its claimed Origin.
    let scheme =
        match super::one_header(headers, "x-forwarded-proto").map_err(|_| RelayError::Forbidden)? {
            None => "http",
            Some("http") => "http",
            Some("https") => "https",
            Some(_) => return Err(RelayError::Forbidden),
        };
    let local = parse_origin(&format!("{scheme}://{host}")).map_err(|_| RelayError::Forbidden)?;
    if trusted.iter().any(|url| url.origin() == origin.origin())
        || local.origin() == origin.origin()
        || (loopback(&local) && loopback(&origin))
    {
        Ok(())
    } else {
        Err(RelayError::Forbidden)
    }
}
fn event_error(raw: &[u8], error: &RelayError) -> Value {
    let mut event = json!({"type":"error","status":error.status(),"error":{"type":error.code(),"code":error.code(),"message":error.to_string()}});
    if let Ok(Value::Object(fields)) = serde_json::from_slice::<Value>(raw) {
        for name in ["event_id", "stream_id"] {
            if let Some(Value::String(id)) = fields.get(name) {
                event[name] = json!(id);
            }
        }
    }
    event
}

#[cfg(test)]
async fn test_gate(stage: &str) {
    if std::env::var("LMM_WS_GATE_STAGE").as_deref() != Ok(stage) {
        return;
    }
    let directory = std::path::PathBuf::from(
        std::env::var("LMM_WS_GATE_DIR").expect("owned WS gate directory"),
    );
    std::fs::write(directory.join(format!("{stage}.reached")), b"reached").unwrap();
    while !directory.join(format!("{stage}.release")).exists() {
        tokio::time::sleep(std::time::Duration::from_millis(10)).await;
    }
}

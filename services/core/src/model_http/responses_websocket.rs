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
use rand::{RngCore, rngs::OsRng};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use std::{
    net::{IpAddr, SocketAddr},
    sync::Arc,
};
use tokio::time::{Instant, timeout, timeout_at};
use tokio_tungstenite::tungstenite::Message as ProviderMessage;

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
    if let Err(error) = gateway.authenticate(&headers).await {
        return super::native::error_response(ClientProtocol::Responses, error);
    }
    let lease = match gateway.relay.admit_responses_ws() {
        Ok(lease) => lease,
        Err(error) => return super::native::error_response(ClientProtocol::Responses, error),
    };
    let request_bytes = gateway.limits.request_bytes;
    socket
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
    OsRng.fill_bytes(&mut session);
    let mut correlation = Correlation::default();
    let mut upstream = None;
    let mut pinned_target = None;
    let mut previous_response = None;
    loop {
        let next = tokio::select! {
            biased;
            () = lease.cancelled() => break,
            result = timeout(gateway.limits.total_timeout, client.recv()) => result,
        };
        let raw = match next {
            Ok(Some(Ok(Message::Text(raw)))) => raw,
            Ok(Some(Ok(Message::Ping(_) | Message::Pong(_)))) => continue,
            _ => break,
        };
        let result = async {
            let create = Create::parse(raw.as_bytes(), gateway.limits.request_bytes)
                .map_err(|error| RelayError::Invalid(error.0))?;
            let sequence = correlation
                .begin(&create)
                .map_err(|error| RelayError::Invalid(error.0))?;
            let turn = Turn::prepare(
                &gateway,
                &headers,
                peer,
                &session,
                sequence,
                &create,
                pinned_target,
                previous_response.as_deref(),
            )
            .await?;
            if upstream.is_none() {
                upstream = Some(lease.upgrade(&turn.route).await?);
                pinned_target = Some(turn.target_id);
            }
            let upstream = upstream.as_mut().ok_or(RelayError::UpstreamConnection)?;
            if !upstream.accepts(&turn.route) {
                return Err(RelayError::RouteUnavailable);
            }
            let reservation = turn.reserve(&create, &gateway.limits).await?;
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
                report.upstream_status = Some(101);
                read_turn(
                    &gateway.limits,
                    &lease,
                    &turn,
                    &reservation,
                    &mut correlation,
                    upstream,
                    &mut client,
                    &mut client_open,
                    &mut report,
                )
                .await
            }
            .await;
            let finalized = turn.finalize(&reservation, &report).await;
            let terminal = outcome?;
            finalized?;
            correlation
                .finish(sequence)
                .map_err(|error| RelayError::Protocol(error.0))?;
            previous_response = terminal["response"]["id"].as_str().map(str::to_owned);
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
                    json!({"type":"error","error":{"code":error.code()}}),
                )
                .await;
                break;
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

async fn read_turn(
    limits: &crate::relay::Limits,
    lease: &ResponsesSession,
    turn: &Turn,
    reservation: &Reservation,
    correlation: &mut Correlation,
    upstream: &mut ResponsesConnection,
    client: &mut WebSocket,
    client_open: &mut bool,
    report: &mut Report,
) -> Result<Value, RelayError> {
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
                        match correlation.authorize_cancel(raw.as_bytes(), limits.request_bytes) {
                            Ok(()) => upstream.send_cancel(&turn.route, raw.as_str()).await?,
                            Err(error) => {
                                let code = RelayError::Invalid(error.0).code();
                                *client_open = write(client, limits, json!({"type":"error","error":{"code":code}})).await.is_ok();
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
        let event: Value = serde_json::from_str(raw.as_str())
            .map_err(|_| RelayError::Protocol("invalid WebSocket event"))?;
        if contains_credential(&event, credential) {
            return Err(RelayError::Protocol("invalid upstream event"));
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
        session: &[u8; 32],
        sequence: u64,
        create: &Create,
        pinned_target: Option<u64>,
        previous_response: Option<&str>,
    ) -> Result<Self, RelayError> {
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
        let id = crate::billing::charging::contract::hex_digest(&turn_id(
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
        OsRng.fill_bytes(&mut worker);
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

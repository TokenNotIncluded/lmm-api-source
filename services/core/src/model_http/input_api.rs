//! Input-only APIs retain the same Core authorization, reservation and ledger authority.
#[path = "input_contract.rs"]
mod contract;
#[cfg(test)]
#[path = "input_api_tests.rs"]
mod tests;

use super::*;
use crate::{
    billing::charging::{self, Outcome},
    routing::{CreditRate, PayloadPolicy, Protocol, ReplayPermission, RouteRequest},
};
use axum::{body::to_bytes, extract::Request};
use contract::{Kind, Request as InputRequest};
use tokio::sync::OwnedSemaphorePermit;

pub(super) fn router(gateway: ModelGateway) -> Router {
    Router::new()
        .route("/v1/embeddings", post(embeddings))
        .route("/v1/rerank", post(rerank))
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
    // The worker owns its admission slot and settles even if the HTTP client disconnects.
    let worker_id = id.clone();
    let result = tokio::spawn(execute(gateway, caller, request, worker_id, permit)).await;
    let response = match result {
        Ok(Ok(value)) => Json(value).into_response(),
        Ok(Err(error)) => public_error(error),
        Err(_) => public_error(RelayError::SettlementPending),
    };
    with_request_id(response, &id)
}

pub(super) fn endpoint(base: &str, protocol: Protocol) -> Result<reqwest::Url, RelayError> {
    let operation = match protocol {
        Protocol::OpenAiEmbeddings => "embeddings",
        Protocol::JinaRerank => "rerank",
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

async fn execute(
    gateway: ModelGateway,
    caller: native_http::Caller,
    request: InputRequest,
    id: String,
    _permit: OwnedSemaphorePermit,
) -> Result<serde_json::Value, RelayError> {
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
    let protocol = request.kind().protocol();
    let mut selected = None;
    for group in &caller.groups {
        if let Ok(plan) = gateway.bridge.routing.route(RouteRequest {
            model: request.model(),
            group,
            speed: None,
            protocol,
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
    let quote = plan
        .quote()
        .with_credit_rate(CreditRate::new(1, 500_000).map_err(|_| RelayError::BillingUnavailable)?);
    if !quote.price().is_input_only_scalar() {
        return Err(RelayError::Unsupported(
            "input-only API requires an input-only token tariff",
        ));
    }
    let price = routing::credit_price(quote.price(), plan.canonical_model(), &group)?;
    let attempt = Arc::new(plan.start().map_err(|_| RelayError::RouteUnavailable)?);
    if attempt.protocol() != protocol
        || attempt.payload_policy() != PayloadPolicy::Adapt
        || !attempt.is_available()
        || u128::from(attempt.timeout_ms()) < gateway.limits.total_timeout.as_millis()
    {
        return Err(RelayError::RouteUnavailable);
    }
    let input_max = gateway.bridge.provider_policy.reservation_input_bound(
        &attempt,
        gateway.bridge.policy.input_token_ceiling,
        0,
    )?;
    request.validate_bound(input_max as u64)?;
    let reference = attempt.credential_ref();
    let credential = gateway
        .bridge
        .credentials
        .get(&(reference.id, reference.revision))
        .ok_or(RelayError::RouteUnavailable)?;
    let endpoint = endpoint(attempt.endpoint(), protocol)?;
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
        return Err(RelayError::Limit("input-only outbound bytes"));
    }
    let upstream_request = connection
        .client()
        .post(endpoint)
        .bearer_auth(credential)
        .header(header::ACCEPT, "application/json")
        .header(header::CONTENT_TYPE, "application/json")
        .body(wire.clone())
        .build()
        .map_err(|_| RelayError::Invalid("input-only provider request"))?;
    let reserve = i64::try_from(
        quote
            .reservation_ceiling(input_max as u64, 0)
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
    fingerprint.update(request.wire(request.model())?);
    let fingerprint: [u8; 32] = fingerprint.finalize().into();
    let mut worker = [0; 32];
    OsRng.fill_bytes(&mut worker);
    let route_id = format!(
        "r{}-t{}-u{}",
        attempt.route_id(),
        attempt.target_id(),
        attempt.upstream_id()
    );
    let (provider_revision, semantics) = gateway
        .bridge
        .provider_policy
        .reservation_metadata(&attempt)?;
    let prepared = charging::contract::PreparedReservation::input_only(
        charging::contract::hex_digest(&fingerprint),
        charging::contract::ReservationBinding {
            route_id: route_id.clone(),
            target_id: attempt.target_id(),
            upstream_id: attempt.upstream_id(),
            protocol: request.kind().name().into(),
            model_snapshot: attempt.upstream_model().into(),
            generation_url: upstream_request.url().as_str().into(),
            credential_ref: reference,
            payload_policy_sha256: charging::contract::typed_digest(&attempt.payload_policy())
                .map_err(billing::error)?,
            provider_contract_revision: provider_revision,
            projection_revision: "input-only-wire-v1".into(),
            quote_version: attempt.quote().version(),
        },
        charging::contract::digest(&wire),
        input_max as u64,
        semantics,
    )
    .map_err(billing::error)?;
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
                output_token_ceiling: 0,
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
        output: 0,
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
    if !billing::retry(|| gateway.bridge.charging.start(&id, &worker))
        .await
        .map_err(billing::error)?
    {
        return Err(RelayError::Conflict);
    }
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
            native_http::read_response_bytes(&connection, response, &gateway.limits).await
        })
        .await
        .map_err(|_| RelayError::SettlementPending)??;
        let (output, observed) = request.response(&raw)?;
        let normalized = serde_json::to_vec(&output).map_err(|_| RelayError::SettlementPending)?;
        if std::str::from_utf8(&raw).is_ok_and(|value| value.contains(credential))
            || std::str::from_utf8(&normalized).is_ok_and(|value| value.contains(credential))
        {
            return Err(RelayError::SettlementPending);
        }
        gateway
            .bridge
            .provider_policy
            .validate_usage(&attempt, &observed, false)?;
        let (frozen, contract) = gateway
            .bridge
            .charging
            .reservation_contract(&id)
            .await
            .map_err(billing::error)?;
        let usage = billing::frozen_priced_usage(&contract, &frozen, observed)?;
        gateway
            .bridge
            .persist_output(
                &relay::Reservation { id: id.clone() },
                &relay::OutputCheckpoint {
                    sequence: 1,
                    packet_sha256: Sha256::digest(&raw).into(),
                    packet_bytes: raw.len(),
                    usage: observed,
                    terminal_observed: true,
                },
            )
            .await?;
        let settled = billing::retry(|| {
            gateway
                .bridge
                .charging
                .settle(&id, &worker, usage, Outcome::Completed)
        })
        .await;
        let settled = match settled {
            Ok(settled) => Some(settled),
            Err(charging::Error::CommitUnknown) => gateway
                .bridge
                .charging
                .resolve(&id)
                .await
                .map_err(billing::error)?,
            _ => return Err(RelayError::SettlementPending),
        };
        if !settled.as_ref().is_some_and(|settled| {
            billing::confirmed_settlement(settled, &id, &frozen.price, usage, Outcome::Completed)
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

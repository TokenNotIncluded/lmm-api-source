//! DeepSeek Search/Fetch use the selected provider's successful-request tariff.
#[cfg(test)]
#[path = "tool_api_tests.rs"]
mod pg_tests;

use super::tool_price::{PersistedToolPrice, ToolKind, ToolUsage};
use super::*;
use crate::{
    billing::charging,
    routing::{CreditRate, PayloadPolicy, Protocol, ReplayPermission, RouteRequest},
};
use axum::{body::to_bytes, extract::Request};
use serde_json::Value;

pub(super) fn router(gateway: ModelGateway) -> Router {
    Router::new()
        .route("/v1/tools/search", post(search))
        .route("/v1/tools/fetch", post(fetch))
        .with_state(gateway)
}

async fn search(
    State(gateway): State<ModelGateway>,
    peer: Result<ConnectInfo<SocketAddr>, ExtensionRejection>,
    request: Request,
) -> Response {
    handle(
        gateway,
        peer.ok().map(|p| p.0.ip()),
        request,
        ToolKind::DeepSeekSearch,
    )
    .await
}

async fn fetch(
    State(gateway): State<ModelGateway>,
    peer: Result<ConnectInfo<SocketAddr>, ExtensionRejection>,
    request: Request,
) -> Response {
    handle(
        gateway,
        peer.ok().map(|p| p.0.ip()),
        request,
        ToolKind::DeepSeekFetch,
    )
    .await
}

async fn handle(
    gateway: ModelGateway,
    peer: Option<IpAddr>,
    request: Request,
    kind: ToolKind,
) -> Response {
    let (parts, body) = request.into_parts();
    let mut operation_id = None;
    let result = async {
        let caller = native_http::authenticate(&gateway, &parts.headers, peer).await?;
        if parts.uri.query().is_some() {
            return Err(RelayError::Invalid(
                "unsupported native tool query parameters",
            ));
        }
        let content_type = one_header(&parts.headers, header::CONTENT_TYPE.as_str())?;
        if !content_type.is_some_and(|value| {
            value
                .split(';')
                .next()
                .is_some_and(|v| v.trim().eq_ignore_ascii_case("application/json"))
        }) || parts.headers.contains_key(header::CONTENT_ENCODING)
        {
            return Err(RelayError::Invalid(
                "native tools require uncompressed JSON",
            ));
        }
        let permit = gateway
            .native_http
            .gate
            .clone()
            .try_acquire_owned()
            .map_err(|_| RelayError::Busy)?;
        let bytes = tokio::time::timeout(
            gateway.limits.read_timeout,
            to_bytes(body, gateway.limits.request_bytes.min(128 * 1024)),
        )
        .await
        .map_err(|_| RelayError::Invalid("native tool request timeout"))?
        .map_err(|_| RelayError::Limit("native tool request body"))?;
        let wire = request_body(kind, &bytes)?;
        let id = request_id(
            caller.principal.key_id().ok_or(RelayError::Forbidden)?,
            &parts.headers,
        )?;
        operation_id = Some(id.clone());
        // Disconnecting the caller never cancels a worker after it owns a hold.
        tokio::spawn(async move {
            let _permit = permit;
            execute(gateway, caller, kind, wire, id).await
        })
        .await
        .map_err(|_| RelayError::SettlementPending)?
    }
    .await;
    let mut response = match result {
        Ok((status, bytes, _)) => {
            let content_type = if serde_json::from_slice::<Value>(&bytes).is_ok() {
                "application/json"
            } else {
                "text/plain; charset=utf-8"
            };
            (status, [(header::CONTENT_TYPE, content_type)], bytes).into_response()
        }
        Err(error) => public_error(error),
    };
    if let Some(id) = operation_id {
        response = with_request_id(response, &id);
    }
    response
        .headers_mut()
        .insert(header::CACHE_CONTROL, HeaderValue::from_static("no-store"));
    response.headers_mut().insert(
        header::X_CONTENT_TYPE_OPTIONS,
        HeaderValue::from_static("nosniff"),
    );
    response
}

fn request_body(kind: ToolKind, bytes: &[u8]) -> Result<Value, RelayError> {
    let value: Value = serde_json::from_slice(bytes)
        .map_err(|_| RelayError::Invalid("invalid native tool JSON"))?;
    let source = value
        .as_object()
        .ok_or(RelayError::Invalid("native tool object required"))?;
    let mut body = serde_json::Map::new();
    let text = |name, max| -> Result<&str, RelayError> {
        source
            .get(name)
            .and_then(Value::as_str)
            .filter(|v| !v.trim().is_empty() && v.chars().count() <= max)
            .ok_or(RelayError::Invalid("invalid native tool text"))
    };
    match kind {
        ToolKind::DeepSeekSearch => {
            body.insert("query".into(), text("query", 128)?.into());
            if let Some(count) = source.get("count") {
                if !count.as_u64().is_some_and(|n| (1..=50).contains(&n)) {
                    return Err(RelayError::Invalid("search count must be between 1 and 50"));
                }
                body.insert("count".into(), count.clone());
            }
            if source.get("ori_question").is_some_and(|v| !v.is_null()) {
                body.insert("ori_question".into(), text("ori_question", 128)?.into());
            }
            if let Some(localization) = source.get("localization") {
                let ip = localization
                    .as_str()
                    .ok_or(RelayError::Invalid("invalid localization"))?;
                if !ip.is_empty() && ip.parse::<IpAddr>().is_err() {
                    return Err(RelayError::Invalid("invalid localization"));
                }
                body.insert("localization".into(), localization.clone());
            }
            if let Some(need_content) = source.get("need_content") {
                if !need_content.is_boolean() {
                    return Err(RelayError::Invalid("need_content must be boolean"));
                }
                body.insert("need_content".into(), need_content.clone());
            }
        }
        ToolKind::DeepSeekFetch => {
            let raw = text("url", 8192)?;
            let url =
                reqwest::Url::parse(raw).map_err(|_| RelayError::Invalid("invalid fetch URL"))?;
            if !matches!(url.scheme(), "https" | "http")
                || url.host_str().is_none()
                || !url.username().is_empty()
                || url.password().is_some()
            {
                return Err(RelayError::Invalid("invalid fetch URL"));
            }
            body.insert("url".into(), raw.into());
        }
    }
    Ok(Value::Object(body))
}

pub(super) fn endpoint(base: &str, kind: ToolKind) -> Result<reqwest::Url, RelayError> {
    let mut url = crate::relay::honeypot_network::validate_base_url(base, true)?;
    let path = url.path().trim_end_matches('/');
    let prefix = if path.ends_with("/v1") {
        path.to_owned()
    } else {
        format!("{path}/v1")
    };
    url.set_path(&format!("{prefix}/{}", kind.endpoint()));
    Ok(url)
}

async fn execute(
    gateway: ModelGateway,
    caller: native_http::Caller,
    kind: ToolKind,
    wire: Value,
    id: String,
) -> Result<(StatusCode, Vec<u8>, String), RelayError> {
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
    let protocol = match kind {
        ToolKind::DeepSeekSearch => Protocol::DeepSeekSearch,
        ToolKind::DeepSeekFetch => Protocol::DeepSeekFetch,
    };
    let mut selected = None;
    for group in &caller.groups {
        if let Ok(plan) = gateway.bridge.routing.route(RouteRequest {
            model: kind.model(),
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
    let canonical_model = plan.canonical_model().to_owned();
    let attempt = Arc::new(plan.start().map_err(|_| RelayError::RouteUnavailable)?);
    if attempt.protocol() != protocol
        || attempt.payload_policy() != PayloadPolicy::Adapt
        || !attempt.is_available()
        || u128::from(attempt.timeout_ms()) < gateway.limits.total_timeout.as_millis()
    {
        return Err(RelayError::RouteUnavailable);
    }
    gateway.bridge.provider_policy.validate_binding(
        attempt.upstream_id(),
        attempt.upstream_model(),
        protocol,
        attempt.endpoint(),
        0,
        0,
    )?;
    let (provider_revision, semantics) = gateway
        .bridge
        .provider_policy
        .reservation_metadata(&attempt)?;
    let tariff = PersistedToolPrice::from_quote(kind, quote.price(), provider_revision.clone())
        .ok_or(RelayError::Unsupported(
            "native tools require an explicit successful-request tariff",
        ))?;
    let price = charging::Price {
        text_request: None,
        grounding: None,

        tool: Some(tariff),
        user_trust: None,
        image: None,
        upstream_cost_group_ppm: None,
        tiered: None,
        version: format!(
            "tool-{}-rule-{}",
            quote.price().version(),
            quote.price().rule_id()
        ),
        model: canonical_model,
        group: group.clone(),
        input_per_million: 0,
        cached_per_million: 0,
        output_per_million: 0,
    };
    let basis = charging::ReservationBasis::Tool;
    let reserve = price.reservation_ceiling(basis).map_err(billing::error)?;
    let reference = attempt.credential_ref();
    let credential = gateway
        .bridge
        .credentials
        .get(&(reference.id, reference.revision))
        .ok_or(RelayError::RouteUnavailable)?
        .clone();
    let endpoint = endpoint(attempt.endpoint(), kind)?;
    let connection = tokio::time::timeout(
        gateway.limits.total_timeout,
        gateway
            .native_http
            .transport
            .prepare(&endpoint, &gateway.limits),
    )
    .await
    .map_err(|_| RelayError::RouteUnavailable)??;
    let body = serde_json::to_vec(&wire).map_err(|_| RelayError::Invalid("native tool body"))?;
    let mut digest = Sha256::new();
    digest.update(b"lmm-native-tool-request-v1\0");
    digest.update(kind.protocol_name());
    digest.update([0]);
    digest.update(group.as_bytes());
    digest.update([0]);
    digest.update(&body);
    let fingerprint: [u8; 32] = digest.finalize().into();
    let mut worker = [0; 32];
    OsRng.fill_bytes(&mut worker);
    let route_id = format!(
        "r{}-t{}-u{}",
        attempt.route_id(),
        attempt.target_id(),
        attempt.upstream_id()
    );
    let prepared = charging::contract::PreparedReservation::native_tool(
        charging::contract::hex_digest(&fingerprint),
        charging::contract::ReservationBinding {
            route_id: route_id.clone(),
            target_id: attempt.target_id(),
            upstream_id: attempt.upstream_id(),
            protocol: kind.protocol_name().into(),
            model_snapshot: attempt.upstream_model().into(),
            generation_url: endpoint.as_str().into(),
            credential_ref: reference,
            payload_policy_sha256: charging::contract::typed_digest(&attempt.payload_policy())
                .map_err(billing::error)?,
            provider_contract_revision: provider_revision,
            projection_revision: "successful-request-v1".into(),
            quote_version: quote.price().version(),
        },
        charging::contract::digest(&body),
        semantics,
    )
    .map_err(billing::error)?;
    let upstream = connection
        .client()
        .post(endpoint)
        .bearer_auth(&credential)
        .header(header::CONTENT_TYPE, "application/json")
        .header(header::ACCEPT, "application/json")
        .body(body)
        .build()
        .map_err(|_| RelayError::Invalid("invalid native tool provider request"))?;
    let selected = routing::Selected {
        attempt: attempt.clone(),
        price: price.clone(),
        quote,
        route_id,
        input_token_ceiling: 0,
    };
    crate::request_limits::admit(
        &gateway.bridge.pool,
        caller.principal.user_id(),
        &group,
        &id,
    )
    .await
    .map_err(routing::admission_error)?;
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
                selected: Some(selected),
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
    let completed = tokio::time::timeout(gateway.limits.total_timeout, async {
        if !attempt.is_available() {
            return Err(RelayError::RouteUnavailable);
        }
        let response = tokio::time::timeout(
            gateway.limits.header_timeout,
            connection.client().execute(upstream),
        )
        .await
        .map_err(|_| RelayError::SettlementPending)?
        .map_err(|_| RelayError::SettlementPending)?;
        let status = response.status();
        let bytes = if status.is_client_error() || status.is_server_error() {
            native_http::read_error_response_bytes(&connection, response, &gateway.limits).await?
        } else {
            native_http::read_response_bytes(&connection, response, &gateway.limits).await?
        };
        if !credential.is_empty()
            && bytes
                .windows(credential.len())
                .any(|chunk| chunk == credential.as_bytes())
        {
            return Err(RelayError::SettlementPending);
        }
        let receipt = ToolUsage::from_response(kind, status.as_u16(), &bytes)
            .ok_or(RelayError::SettlementPending)?;
        let outcome = receipt.outcome();
        let observed = relay::Usage {
            tool: Some(receipt),
            input_tokens: Some(0),
            cached_input_tokens: Some(0),
            output_tokens: Some(0),
            ..Default::default()
        };
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
                    packet_sha256: Sha256::digest(&bytes).into(),
                    packet_bytes: bytes.len(),
                    usage: observed,
                    terminal_observed: true,
                },
            )
            .await?;
        let result =
            billing::retry(|| gateway.bridge.charging.settle(&id, &worker, usage, outcome)).await;
        let settled = match result {
            Err(charging::Error::CommitUnknown) => gateway
                .bridge
                .charging
                .resolve(&id)
                .await
                .map_err(billing::error)?
                .ok_or(RelayError::SettlementPending)?,
            result => result.map_err(billing::error)?,
        };
        if !billing::confirmed_settlement(&settled, &id, &frozen.price, usage, outcome) {
            return Err(RelayError::SettlementPending);
        }
        Ok((status, bytes, id.clone()))
    })
    .await
    .unwrap_or(Err(RelayError::SettlementPending));
    if completed.is_err() {
        let _ = billing::retry(|| gateway.bridge.charging.mark_unknown(&id, &worker)).await;
    }
    completed
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tool_request_preserves_search_syntax_and_validates_fetch_url() {
        let body = request_body(
            ToolKind::DeepSeekSearch,
            br#"{"query":"rust site:github.com","count":50,"need_content":true,"price":0}"#,
        )
        .unwrap();
        assert_eq!(body["query"], "rust site:github.com");
        assert!(body.get("price").is_none());
        assert!(request_body(ToolKind::DeepSeekSearch, br#"{"query":"rust","count":51}"#).is_err());
        assert!(
            request_body(
                ToolKind::DeepSeekFetch,
                br#"{"url":"https://example.invalid/a"}"#
            )
            .is_ok()
        );
        assert!(request_body(ToolKind::DeepSeekFetch, br#"{"url":"file:///etc/passwd"}"#).is_err());
        assert!(
            request_body(
                ToolKind::DeepSeekFetch,
                br#"{"url":"https://user:secret@example.invalid"}"#
            )
            .is_err()
        );
        assert_eq!(
            endpoint("https://api.deepseek.com/v1", ToolKind::DeepSeekSearch)
                .unwrap()
                .as_str(),
            "https://api.deepseek.com/v1/tools/search"
        );
    }
}

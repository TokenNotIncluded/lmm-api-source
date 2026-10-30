//! Bounded transport and admission shared by non-streaming native model APIs.
#[cfg(test)]
#[path = "native_api_test_support.rs"]
pub(super) mod test_support;

use super::*;
use crate::relay::provider_network::{ProviderConnection, ProviderTransport};
use tokio::sync::Semaphore;

#[derive(Clone)]
pub(super) struct Host {
    pub(super) gate: Arc<Semaphore>,
    pub(super) transport: ProviderTransport,
    parallelism: usize,
}

impl Host {
    pub(super) fn new(limits: &Limits) -> Self {
        let parallelism = limits.max_in_flight.min(4);
        Self {
            gate: Arc::new(Semaphore::new(parallelism)),
            transport: ProviderTransport::new(parallelism),
            parallelism,
        }
    }

    pub(super) async fn drain(&self, grace: Duration) -> usize {
        self.gate.close();
        let deadline = tokio::time::Instant::now() + grace;
        while self.gate.available_permits() < self.parallelism
            && tokio::time::Instant::now() < deadline
        {
            tokio::time::sleep(Duration::from_millis(5)).await;
        }
        self.parallelism
            .saturating_sub(self.gate.available_permits())
    }
}

pub(super) struct Caller {
    pub(super) principal: ModelIdentity,
    pub(super) secret: String,
    pub(super) groups: Vec<String>,
    pub(super) peer: Option<IpAddr>,
}

pub(super) async fn authenticate(
    gateway: &ModelGateway,
    headers: &HeaderMap,
    peer: Option<IpAddr>,
) -> Result<Caller, RelayError> {
    // Existing OAuth scopes cover the generation APIs, not these new operations.
    if oauth_header(headers) {
        return Err(RelayError::Forbidden);
    }
    if ["x-api-key", "x-goog-api-key", "x-auth-session"]
        .iter()
        .any(|name| headers.contains_key(*name))
    {
        return Err(RelayError::Invalid("ambiguous model credentials"));
    }
    if let Some(honeypot) = &gateway.honeypot
        && honeypot.models(headers, peer).await?.is_some()
    {
        return Err(RelayError::Unsupported(
            "native API moderation policy is not configured",
        ));
    }
    let (principal, secret, groups) = gateway.authenticate(headers).await?;
    if principal.key_id().is_none() {
        return Err(RelayError::Forbidden);
    }
    Ok(Caller {
        principal,
        secret,
        groups,
        peer,
    })
}

/// Keep credentials alive only while the detached financial worker owns this call.
pub(super) struct Active {
    pub(super) bridge: Arc<Bridge>,
    pub(super) id: String,
}
impl Drop for Active {
    fn drop(&mut self) {
        if let Ok(mut active) = self.bridge.active.lock() {
            active.remove(&self.id);
        }
    }
}

/// The caller classifies the HTTP status using its supplier's billing contract.
pub(super) async fn read_response_bytes(
    connection: &ProviderConnection,
    response: reqwest::Response,
    limits: &Limits,
) -> Result<Vec<u8>, RelayError> {
    read_body(connection, response, limits, true).await
}

/// Some native tool providers document a complete HTTP error as zero usage;
/// that evidence need not use their successful JSON response representation.
pub(super) async fn read_error_response_bytes(
    connection: &ProviderConnection,
    response: reqwest::Response,
    limits: &Limits,
) -> Result<Vec<u8>, RelayError> {
    read_body(connection, response, limits, false).await
}

async fn read_body(
    connection: &ProviderConnection,
    mut response: reqwest::Response,
    limits: &Limits,
    require_json: bool,
) -> Result<Vec<u8>, RelayError> {
    if !connection.matches_peer(response.remote_addr())
        || response.headers().contains_key(header::CONTENT_ENCODING)
        || (require_json
            && !response
                .headers()
                .get(header::CONTENT_TYPE)
                .and_then(|v| v.to_str().ok())
                .is_some_and(|v| {
                    v.split(';')
                        .next()
                        .is_some_and(|v| v.trim().eq_ignore_ascii_case("application/json"))
                }))
    {
        return Err(RelayError::SettlementPending);
    }
    let cap = limits.upstream_bytes.min(limits.output_bytes);
    if response.content_length().is_some_and(|n| n > cap as u64) {
        return Err(RelayError::SettlementPending);
    }
    let mut bytes = Vec::new();
    while let Some(chunk) = tokio::time::timeout(limits.read_timeout, response.chunk())
        .await
        .map_err(|_| RelayError::SettlementPending)?
        .map_err(|_| RelayError::SettlementPending)?
    {
        if chunk.len() > limits.transport_chunk_bytes
            || bytes.len().checked_add(chunk.len()).is_none_or(|n| n > cap)
        {
            return Err(RelayError::SettlementPending);
        }
        bytes.extend_from_slice(&chunk);
    }
    Ok(bytes)
}

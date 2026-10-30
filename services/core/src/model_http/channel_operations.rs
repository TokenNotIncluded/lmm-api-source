//! Administrator-requested, non-generating provider catalogue probes.
//! Destinations and credentials come from the published server configuration.
use super::*;
use serde::Serialize;
use serde_json::Value;
use std::collections::BTreeSet;

#[derive(Serialize)]
pub(crate) struct ChannelProbe {
    pub models: Vec<String>,
    pub response_time_ms: u64,
    pub validation_scope: &'static str,
    pub inference_tested: bool,
}

/// A price publication must remain executable by every configured target.
/// The ledger accepts exact integer credits or its existing typed tariffs;
/// the looser routing-document parser alone cannot establish that contract.
pub(crate) fn validate_publication_prices(
    document: &crate::routing::store::ReleaseDocument,
) -> Result<(), RelayError> {
    use crate::routing::{BillingMode, PriceSnapshot, Protocol};
    let prices = PriceSnapshot::compile(document.pricing.clone())
        .map_err(|_| RelayError::Invalid("invalid prices"))?;
    let upstreams: BTreeMap<_, _> = document
        .routing
        .upstreams
        .iter()
        .map(|upstream| (upstream.id, upstream))
        .collect();
    for route in &document.routing.routes {
        for target in &route.targets {
            let upstream = upstreams
                .get(&target.upstream_id)
                .ok_or(RelayError::RouteUnavailable)?;
            for speed in &target.speeds {
                let quote = prices
                    .quote(&route.model, &route.group, *speed)
                    .ok_or(RelayError::Invalid("missing route price"))?;
                if let Some(kind) = tool_price::ToolKind::from_protocol(upstream.protocol) {
                    if route.model != kind.model()
                        || !tool_price::PersistedToolPrice::supports_quote(kind, quote)
                    {
                        return Err(RelayError::Invalid("invalid native tool price"));
                    }
                    continue;
                }
                routing::credit_price(quote, &route.model, &route.group)?;
                if (upstream.protocol.input_only() || upstream.protocol == Protocol::SystemOne)
                    && !quote.is_input_only_scalar()
                {
                    return Err(RelayError::Invalid("invalid input-only price"));
                }
                if quote.tiered().is_some_and(|tariff| tariff.revision() == 4)
                    && !matches!(
                        upstream.protocol,
                        Protocol::OpenAiChat | Protocol::OpenAiResponses
                    )
                {
                    return Err(RelayError::Invalid("price requires an OpenAI protocol"));
                }
                if quote.billing_mode() == BillingMode::UpstreamCost {
                    let endpoint = crate::relay::honeypot_network::validate_base_url(
                        upstream.endpoint.as_str(),
                        true,
                    )?;
                    if !crate::relay::honeypot_network::literal_ip(&endpoint)
                        .is_some_and(|ip| ip.is_loopback())
                        || target.upstream_model != route.model
                    {
                        return Err(RelayError::Invalid("supplier budget contract is missing"));
                    }
                }
            }
        }
    }
    Ok(())
}
fn endpoint(base: &str, protocol: crate::routing::Protocol) -> Result<reqwest::Url, RelayError> {
    if !matches!(
        protocol,
        crate::routing::Protocol::OpenAiChat
            | crate::routing::Protocol::OpenAiResponses
            | crate::routing::Protocol::AnthropicMessages
            | crate::routing::Protocol::Gemini
            | crate::routing::Protocol::OpenAiEmbeddings
    ) {
        return Err(RelayError::Unsupported(
            "This provider protocol has no model-catalogue probe",
        ));
    }
    let mut url = crate::relay::honeypot_network::validate_base_url(base, true)?;
    url.path_segments_mut()
        .map_err(|_| RelayError::RouteUnavailable)?
        .pop_if_empty()
        .push("models");
    Ok(url)
}
fn parse_models(
    bytes: &[u8],
    protocol: crate::routing::Protocol,
    secret: &str,
) -> Result<Vec<String>, RelayError> {
    let value: serde_json::Value =
        serde_json::from_slice(bytes).map_err(|_| RelayError::RouteUnavailable)?;
    let (field, name) = if protocol == crate::routing::Protocol::Gemini {
        ("models", "name")
    } else {
        ("data", "id")
    };
    let items = value
        .get(field)
        .and_then(Value::as_array)
        .filter(|v| v.len() <= 10_000)
        .ok_or(RelayError::RouteUnavailable)?;
    let mut models = BTreeSet::new();
    for item in items {
        let model = item
            .get(name)
            .and_then(Value::as_str)
            .ok_or(RelayError::RouteUnavailable)?;
        if model.is_empty()
            || model.len() > 256
            || model.chars().any(char::is_control)
            || model.contains(secret)
        {
            return Err(RelayError::RouteUnavailable);
        }
        models.insert(
            if protocol == crate::routing::Protocol::Gemini {
                model.strip_prefix("models/").unwrap_or(model)
            } else {
                model
            }
            .to_owned(),
        );
    }
    Ok(models.into_iter().collect())
}
impl ModelGateway {
    /// Enabling an already known destination still requires host-loaded keys
    /// and exact provider contracts. The HTTP body cannot invent either.
    pub(crate) fn validate_channel_activation(
        &self,
        document: &crate::routing::store::ReleaseDocument,
        enabled: &BTreeSet<u64>,
    ) -> Result<(), RelayError> {
        use crate::routing::Protocol;
        let upstreams: BTreeMap<_, _> = document
            .routing
            .upstreams
            .iter()
            .filter(|u| enabled.contains(&u.id))
            .map(|u| (u.id, u))
            .collect();
        for upstream in upstreams.values() {
            let key = self
                .bridge
                .credentials
                .get(&(upstream.credential.id, upstream.credential.revision))
                .ok_or(RelayError::RouteUnavailable)?;
            if key.is_empty() || reqwest::header::HeaderValue::from_str(key.as_str()).is_err() {
                return Err(RelayError::RouteUnavailable);
            }
        }
        for route in &document.routing.routes {
            for target in &route.targets {
                let Some(upstream) = upstreams.get(&target.upstream_id) else {
                    continue;
                };
                let tool = matches!(
                    upstream.protocol,
                    Protocol::DeepSeekSearch | Protocol::DeepSeekFetch
                );
                self.bridge.provider_policy.validate_binding(
                    upstream.id,
                    &target.upstream_model,
                    upstream.protocol,
                    upstream.endpoint.as_str(),
                    if tool {
                        0
                    } else {
                        self.bridge.policy.input_token_ceiling
                    },
                    if tool || upstream.protocol.input_only() {
                        0
                    } else {
                        1
                    },
                )?;
                if u128::from(target.timeout_ms) < self.limits.total_timeout.as_millis() {
                    return Err(RelayError::RouteUnavailable);
                }
            }
        }
        Ok(())
    }

    pub(crate) async fn probe_channel(&self, id: u64) -> Result<ChannelProbe, RelayError> {
        let document = self
            .bridge
            .routing
            .catalog_document()
            .map_err(|_| RelayError::RouteUnavailable)?;
        let upstream = document
            .routing
            .upstreams
            .iter()
            .find(|u| u.id == id)
            .ok_or(RelayError::RouteUnavailable)?;
        let credential = self
            .bridge
            .credentials
            .get(&(upstream.credential.id, upstream.credential.revision))
            .ok_or(RelayError::RouteUnavailable)?;
        let url = endpoint(upstream.endpoint.as_str(), upstream.protocol)?;
        let _permit = self
            .native_http
            .gate
            .clone()
            .try_acquire_owned()
            .map_err(|_| RelayError::RouteUnavailable)?;
        let started = std::time::Instant::now();
        let models = tokio::time::timeout(self.limits.total_timeout, async {
            let connection = self
                .native_http
                .transport
                .prepare(&url, &self.limits)
                .await?;
            let request = connection
                .client()
                .get(url)
                .header(header::ACCEPT, "application/json");
            let request = match upstream.protocol {
                crate::routing::Protocol::AnthropicMessages => request
                    .header("x-api-key", credential)
                    .header("anthropic-version", "2023-06-01"),
                crate::routing::Protocol::Gemini => request.header("x-goog-api-key", credential),
                _ => request.bearer_auth(credential),
            };
            let response = request
                .send()
                .await
                .map_err(|_| RelayError::RouteUnavailable)?;
            if !response.status().is_success() {
                return Err(RelayError::RouteUnavailable);
            }
            let bytes =
                native_http::read_response_bytes(&connection, response, &self.limits).await?;
            parse_models(&bytes, upstream.protocol, credential)
        })
        .await
        .map_err(|_| RelayError::RouteUnavailable)??;
        Ok(ChannelProbe {
            models,
            response_time_ms: started.elapsed().as_millis().try_into().unwrap_or(u64::MAX),
            validation_scope: "model_catalog",
            inference_tested: false,
        })
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    use crate::routing::Protocol;
    #[test]
    fn provider_catalogues_preserve_model_ids_without_exposing_provider_documents() {
        assert_eq!(parse_models(br#"{"data":[{"id":"model-z","owner":"private"},{"id":"model-a"},{"id":"model-a"}]}"#,Protocol::OpenAiChat,"synthetic-secret").unwrap(),vec!["model-a","model-z"]);
        assert_eq!(
            parse_models(
                br#"{"models":[{"name":"models/gemini-test","description":"private"}]}"#,
                Protocol::Gemini,
                "synthetic-secret"
            )
            .unwrap(),
            vec!["gemini-test"]
        );
        for invalid in [
            br#"{"data":[{"id":"synthetic-secret"}]}"#.as_slice(),
            br#"{"data":[{}]}"#,
            br#"{"error":"no access"}"#,
        ] {
            assert!(parse_models(invalid, Protocol::OpenAiChat, "synthetic-secret").is_err());
        }
        assert_eq!(
            endpoint("https://api.example/v1/", Protocol::OpenAiResponses)
                .unwrap()
                .as_str(),
            "https://api.example/v1/models"
        );
        assert!(endpoint("https://127.0.0.2/path?key=secret", Protocol::OpenAiChat).is_err());
        assert!(endpoint("https://api.example/v1", Protocol::DeepSeekFetch).is_err());
    }
}

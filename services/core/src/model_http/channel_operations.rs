//! Administrator-requested, non-generating provider catalogue probes.
//! Destinations and credentials come from the published server configuration.
use super::*;
use crate::billing::tool_price;
use serde::Serialize;
use serde_json::Value;
use std::collections::BTreeSet;

#[cfg(test)]
#[path = "channel_balance_decimal_oracle_tests.rs"]
mod channel_balance_decimal_oracle_tests;

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
                // Publication checks the compiled route template. quote() hides
                // calendar tariffs until an admission timestamp exists; those
                // rules remain publishable and are selected later with quote_at.
                let quote = prices
                    .template(&route.model, &route.group, *speed)
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
    pub(crate) fn channel_contexts(&self) -> Vec<ExternalModelContext> {
        self.bridge.provider_policy.context_document()
    }

    pub(crate) fn channel_credential(
        &self,
        reference: crate::routing::CredentialRef,
    ) -> Option<String> {
        self.bridge
            .credentials
            .get(&(reference.id, reference.revision))
    }

    pub(crate) fn has_channel_credential(&self, reference: crate::routing::CredentialRef) -> bool {
        self.bridge
            .credentials
            .contains_key(&(reference.id, reference.revision))
    }

    pub(crate) fn validate_channel_credentials(
        &self,
        credentials: &[UpstreamCredential],
    ) -> Result<(), RelayError> {
        if credentials.iter().any(|credential| {
            credential.secret.is_empty()
                || credential.secret.len() > 4096
                || reqwest::header::HeaderValue::from_str(&credential.secret).is_err()
                || self
                    .honeypot
                    .as_ref()
                    .is_some_and(|gateway| gateway.contains_secret(&credential.secret))
        }) {
            return Err(RelayError::Invalid(
                "invalid or isolated channel credential",
            ));
        }
        Ok(())
    }

    pub(crate) fn install_channel_bindings(
        &self,
        version: u64,
        credentials: Vec<UpstreamCredential>,
        contexts: Vec<ExternalModelContext>,
    ) -> Result<(), RelayError> {
        self.validate_channel_credentials(&credentials)?;
        self.bridge.credentials.append(credentials)?;
        self.bridge.provider_policy.publish_contexts(
            version,
            contexts,
            self.bridge.policy.input_token_ceiling,
        )
    }

    /// Validate complete executable bindings before a control-plane commit.
    /// The incoming key set contains only already validated vault revisions.
    pub(crate) fn validate_channel_release(
        &self,
        document: &crate::routing::store::ReleaseDocument,
        contexts: Vec<ExternalModelContext>,
        incoming_keys: &BTreeSet<(u64, u64)>,
    ) -> Result<(), RelayError> {
        use crate::routing::Protocol;
        let policy = ProviderPolicy::new(contexts, self.bridge.policy.input_token_ceiling)?;
        let upstreams: BTreeMap<_, _> = document
            .routing
            .upstreams
            .iter()
            .map(|u| (u.id, u))
            .collect();
        for upstream in upstreams.values().filter(|u| u.enabled) {
            let reference = (upstream.credential.id, upstream.credential.revision);
            if !self.bridge.credentials.contains_key(&reference)
                && !incoming_keys.contains(&reference)
            {
                return Err(RelayError::RouteUnavailable);
            }
        }
        for route in &document.routing.routes {
            for target in &route.targets {
                let upstream = upstreams
                    .get(&target.upstream_id)
                    .ok_or(RelayError::RouteUnavailable)?;
                if !upstream.enabled {
                    continue;
                }
                let native_tool = matches!(
                    upstream.protocol,
                    Protocol::DeepSeekSearch | Protocol::DeepSeekFetch
                );
                policy.validate_binding(
                    upstream.id,
                    &target.upstream_model,
                    upstream.protocol,
                    upstream.endpoint.as_str(),
                    if native_tool {
                        0
                    } else {
                        self.bridge.policy.input_token_ceiling
                    },
                    if native_tool || upstream.protocol.input_only() {
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

    pub(crate) async fn probe_channel_snapshot(
        &self,
        upstream: &crate::routing::UpstreamConfig,
    ) -> Result<ChannelProbe, RelayError> {
        let credential = self
            .bridge
            .credentials
            .get(&(upstream.credential.id, upstream.credential.revision))
            .ok_or(RelayError::RouteUnavailable)?;
        let url = endpoint(upstream.endpoint.as_str(), upstream.protocol)?;
        #[cfg(test)]
        let url = if let Some(mapping) = &self.balance_probe_loopback {
            if url.origin().ascii_serialization() != mapping.origin {
                return Err(RelayError::RouteUnavailable);
            }
            let mut endpoint = mapping.endpoint.clone();
            endpoint.set_path(url.path());
            endpoint
        } else {
            url
        };

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
                    .header("x-api-key", &credential)
                    .header("anthropic-version", "2023-06-01"),
                crate::routing::Protocol::Gemini => request.header("x-goog-api-key", &credential),
                _ => request.bearer_auth(&credential),
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
            parse_models(&bytes, upstream.protocol, &credential)
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

    #[test]
    fn balance_requirement_rejects_credentials_hidden_by_json_escaping() {
        for secret in ["quote\"key", "slash\\key", "line\nkey"] {
            let value = Value::String(format!("prefix {secret} suffix"));
            assert!(!value.to_string().contains(secret));
            assert!(balance_value_contains_secret(&value, secret));
        }
        assert!(balance_value_contains_secret(
            &serde_json::json!(12345),
            "234"
        ));
        assert!(!balance_value_contains_secret(
            &serde_json::json!(true),
            "secret"
        ));
    }

    #[test]
    fn cny_conversion_uses_explicit_rate_and_legacy_decimal_rounding() {
        assert_eq!(cny_to_usd(42.0, 7.0), Some(6.0));
        assert_eq!(cny_to_usd(1.0, 3.0), Some(0.3333333333333333));
        assert_eq!(cny_to_usd(-1.0, 3.0), Some(-0.3333333333333333));
        assert_eq!(cny_to_usd(5e-17, 1.0), Some(1e-16));
        assert_eq!(cny_to_usd(-5e-17, 1.0), Some(-1e-16));
        assert_eq!(cny_to_usd(100.0, 7.2), Some(13.88888888888889));
        assert_eq!(
            cny_to_usd(1.2345678901234567, 9.876543210987654),
            Some(0.1249999988609375)
        );
        assert_eq!(cny_to_usd(1.0, 0.0), None);
        assert_eq!(cny_to_usd(1.0, f64::NAN), None);
        assert_eq!(cny_to_usd(f64::MAX, f64::MIN_POSITIVE), None);
    }

    #[test]
    fn deepseek_selects_first_usd_before_cny_without_guessing_rate() {
        let select = |value: Value, rate| deepseek_balance(&value, rate, "secret-value");
        assert_eq!(
            select(
                serde_json::json!({"balance_infos":[{"currency":"CNY","total_balance":"70"},{"currency":" usd ","total_balance":"2.5"}]}),
                None
            ),
            Some(2.5)
        );
        assert_eq!(
            select(
                serde_json::json!({"balance_infos":[null,{"currency":"cny","total_balance":"70"}]}),
                Some(7.0)
            ),
            Some(10.0)
        );
        assert_eq!(
            select(
                serde_json::json!({"balance_infos":[{"currency":"CNY","total_balance":"70"}]}),
                None
            ),
            None
        );
        assert_eq!(
            select(
                serde_json::json!({"balance_infos":[{"currency":"USD","total_balance":"bad"},{"currency":"USD","total_balance":"2"},{"currency":"CNY","total_balance":"70"}]}),
                Some(7.0)
            ),
            None
        );
        for bad in ["-1", "NaN", "Inf", "secret-value"] {
            assert_eq!(
                select(
                    serde_json::json!({"balance_infos":[{"currency":"USD","total_balance":bad}]}),
                    Some(7.0)
                ),
                None
            );
        }
        assert_eq!(
            select(
                serde_json::json!({"balance_infos":[{"currency":"EUR","total_balance":"7"}]}),
                Some(7.0)
            ),
            None
        );
        assert_eq!(
            select(
                serde_json::json!({"balance_infos":[{"currency":"USD","total_balance":7}]}),
                Some(7.0)
            ),
            None
        );
    }
}

/// Synthetic transport mapping compiled only into private unit tests.
#[cfg(test)]
#[derive(Clone)]
pub(super) struct BalanceProbeLoopback {
    origin: String,
    endpoint: reqwest::Url,
}

#[cfg(test)]
impl ModelGateway {
    /// Map one published HTTPS origin to an isolated numeric-loopback HTTP
    /// provider for synthetic tests. Requires the existing host local-mode opt-in.
    /// This mapping applies only to balance GETs, never paid model traffic.
    pub(crate) fn with_balance_probe_loopback(
        mut self,
        published_origin: &str,
        local_endpoint: &str,
    ) -> Result<Self, RelayError> {
        let origin = crate::relay::honeypot_network::validate_origin(published_origin, false)?;
        let endpoint = crate::relay::honeypot_network::validate_base_url(local_endpoint, true)?;
        if !self.local_streaming
            || endpoint.scheme() != "http"
            || !crate::relay::honeypot_network::literal_ip(&endpoint)
                .is_some_and(|ip| ip.is_loopback())
            || endpoint.path() != "/"
        {
            return Err(RelayError::Invalid("invalid synthetic balance transport"));
        }
        self.balance_probe_loopback = Some(BalanceProbeLoopback { origin, endpoint });
        Ok(self)
    }
}

/// A frozen provider GET. The credential never leaves the gateway or enters a URL.
pub(crate) struct BalanceRequest {
    url: reqwest::Url,
    protocol: crate::routing::Protocol,
    credential: String,
    pointer: String,
    subtract_pointer: Option<String>,
    requirements: Vec<(String, Value)>,
    cny_per_usd: Option<f64>,
    deepseek_currency_array: bool,
    openai_subscription: bool,
    api_key_auth: bool,
}

pub(crate) fn valid_balance_path(path: &str) -> bool {
    path.starts_with('/')
        && !path.starts_with("//")
        && !path.contains(['?', '#', '\\'])
        && !path.chars().any(char::is_control)
        && path
            .parse::<axum::http::Uri>()
            .is_ok_and(|uri| uri.scheme().is_none() && uri.authority().is_none())
}

fn balance_value_contains_secret(value: &Value, secret: &str) -> bool {
    match value {
        Value::String(value) => value.contains(secret),
        _ => value.to_string().contains(secret),
    }
}

// Match the legacy decimal library's 16-place division on the shortest
// decimal representations of the already-parsed f64 values. Display only;
// this does not supply an exchange rate or perform ledger conversion.
fn cny_to_usd(amount: f64, rate: f64) -> Option<f64> {
    use num_bigint::BigInt;
    fn parts(value: f64) -> Option<(BigInt, u32)> {
        let text = value.to_string();
        let (mantissa, exponent) = match text.split_once(['e', 'E']) {
            Some((mantissa, exponent)) => (mantissa, exponent.parse::<i32>().ok()?),
            None => (text.as_str(), 0),
        };
        let fractional = mantissa
            .split_once('.')
            .map_or(0, |(_, digits)| digits.len());
        let digits = mantissa.replace('.', "");
        let mut integer = BigInt::parse_bytes(digits.as_bytes(), 10)?;
        let scale = i32::try_from(fractional).ok()?.checked_sub(exponent)?;
        if scale < 0 {
            integer *= BigInt::from(10_u8).pow(scale.unsigned_abs());
            Some((integer, 0))
        } else {
            Some((integer, u32::try_from(scale).ok()?))
        }
    }
    if !amount.is_finite() || !rate.is_finite() || rate <= 0.0 {
        return None;
    }
    let (amount, amount_scale) = parts(amount)?;
    let (rate, rate_scale) = parts(rate)?;
    let numerator = amount * BigInt::from(10_u8).pow(rate_scale.checked_add(16)?);
    let denominator = rate * BigInt::from(10_u8).pow(amount_scale);
    let mut rounded = &numerator / &denominator;
    let remainder = &numerator % &denominator;
    let magnitude = if remainder < BigInt::from(0) {
        -remainder
    } else {
        remainder
    };
    if magnitude * BigInt::from(2) >= denominator {
        rounded += if numerator < BigInt::from(0) { -1 } else { 1 };
    }
    let result = format!("{rounded}e-16").parse::<f64>().ok()?;
    result.is_finite().then_some(result)
}

fn deepseek_balance(value: &Value, rate: Option<f64>, secret: &str) -> Option<f64> {
    #[derive(serde::Deserialize)]
    struct Response {
        #[serde(default, rename = "is_available")]
        _is_available: Option<bool>,
        #[serde(default)]
        balance_infos: Option<Vec<Option<Item>>>,
    }
    #[derive(serde::Deserialize)]
    struct Item {
        #[serde(default)]
        currency: Option<String>,
        #[serde(default)]
        total_balance: Option<String>,
        #[serde(default, rename = "granted_balance")]
        _granted_balance: Option<String>,
        #[serde(default, rename = "topped_up_balance")]
        _topped_up_balance: Option<String>,
    }
    let response: Response = serde_json::from_value(value.clone()).ok()?;
    let items = response.balance_infos?;
    let find = |currency: &str| {
        items.iter().flatten().find(|item| {
            item.currency
                .as_deref()
                .is_some_and(|value| value.trim().to_uppercase() == currency)
        })
    };
    let usd = find("USD");
    let item = usd.or_else(|| find("CNY"))?;
    let text = item.total_balance.as_deref()?;
    if text.contains(secret) {
        return None;
    }
    let amount = text.trim().parse::<f64>().ok()?;
    if !amount.is_finite() || amount < 0.0 {
        return None;
    }
    let result = if usd.is_some() {
        amount
    } else {
        cny_to_usd(amount, rate?)?
    };
    (result.is_finite() && result >= 0.0 && !result.to_string().contains(secret)).then_some(result)
}

fn parse_balance(
    bytes: &[u8],
    pointer: &str,
    subtract_pointer: Option<&str>,
    requirements: &[(String, Value)],
    cny_per_usd: Option<f64>,
    deepseek_currency_array: bool,
    secret: &str,
) -> Result<f64, RelayError> {
    // Do not return even a numeric-looking credential reflected by a provider.
    if !secret.is_empty() && String::from_utf8_lossy(bytes).contains(secret) {
        return Err(RelayError::RouteUnavailable);
    }
    let value: Value = serde_json::from_slice(bytes).map_err(|_| RelayError::RouteUnavailable)?;
    if requirements
        .iter()
        .any(|(pointer, expected)| value.pointer(pointer) != Some(expected))
    {
        return Err(RelayError::RouteUnavailable);
    }
    if deepseek_currency_array {
        return deepseek_balance(&value, cny_per_usd, secret).ok_or(RelayError::RouteUnavailable);
    }
    let number = |pointer: &str| -> Result<f64, RelayError> {
        let selected = value.pointer(pointer).ok_or(RelayError::RouteUnavailable)?;
        let number = match selected {
            Value::Number(number) => number.as_f64(),
            Value::String(number) if !number.contains(secret) => number.parse::<f64>().ok(),
            _ => None,
        };
        number
            .filter(|number| number.is_finite())
            .ok_or(RelayError::RouteUnavailable)
    };
    let mut balance = number(pointer)?;
    if let Some(pointer) = subtract_pointer {
        balance -= number(pointer)?;
    }
    if let Some(rate) = cny_per_usd {
        balance = cny_to_usd(balance, rate).ok_or(RelayError::RouteUnavailable)?;
    }
    if !balance.is_finite() || balance.to_string().contains(secret) {
        return Err(RelayError::RouteUnavailable);
    }
    Ok(balance)
}

pub(crate) struct BalanceProbeInput<'a> {
    pub(crate) path: &'a str,
    pub(crate) pointer: &'a str,
    pub(crate) subtract_pointer: Option<&'a str>,
    pub(crate) requirements: &'a [(&'a str, &'a Value)],
    pub(crate) cny_per_usd: Option<&'a str>,
    pub(crate) deepseek_currency_array: bool,
    pub(crate) openai_subscription: bool,
    pub(crate) unit_label: Option<&'a str>,
    pub(crate) api_key_auth: bool,
}
impl ModelGateway {
    pub(crate) fn prepare_balance_probe(
        &self,
        upstream: &crate::routing::UpstreamConfig,
        input: BalanceProbeInput<'_>,
    ) -> Result<BalanceRequest, RelayError> {
        let BalanceProbeInput {
            path,
            pointer,
            subtract_pointer,
            requirements,
            cny_per_usd,
            deepseek_currency_array,
            openai_subscription,
            unit_label,
            api_key_auth,
        } = input;
        if !valid_balance_path(path) {
            return Err(RelayError::Invalid("invalid balance path"));
        }
        let credential = self
            .channel_credential(upstream.credential)
            .ok_or(RelayError::RouteUnavailable)?;
        if credential.is_empty()
            || [
                Some(path),
                Some(pointer),
                subtract_pointer,
                cny_per_usd,
                unit_label,
            ]
            .into_iter()
            .flatten()
            .any(|value| value.contains(&credential))
            || requirements.iter().any(|(pointer, expected)| {
                pointer.contains(&credential)
                    || balance_value_contains_secret(expected, &credential)
            })
        {
            return Err(RelayError::RouteUnavailable);
        }
        let base =
            crate::relay::honeypot_network::validate_base_url(upstream.endpoint.as_str(), true)?;
        let url = base.join(path).map_err(|_| RelayError::RouteUnavailable)?;
        if url.origin() != base.origin() || url.query().is_some() || url.fragment().is_some() {
            return Err(RelayError::RouteUnavailable);
        }
        Ok(BalanceRequest {
            url,
            protocol: upstream.protocol,
            credential,
            pointer: pointer.to_owned(),
            subtract_pointer: subtract_pointer.map(str::to_owned),
            requirements: requirements
                .iter()
                .map(|(pointer, value)| ((*pointer).to_owned(), (*value).clone()))
                .collect(),
            cny_per_usd: cny_per_usd
                .map(str::parse::<f64>)
                .transpose()
                .map_err(|_| RelayError::RouteUnavailable)?,
            deepseek_currency_array,
            openai_subscription,
            api_key_auth,
        })
    }

    pub(crate) async fn probe_balance(&self, request: BalanceRequest) -> Result<f64, RelayError> {
        #[cfg(test)]
        let request = {
            let mut request = request;
            if let Some(mapping) = &self.balance_probe_loopback {
                if request.url.origin().ascii_serialization() != mapping.origin {
                    return Err(RelayError::RouteUnavailable);
                }
                let mut endpoint = mapping.endpoint.clone();
                endpoint.set_path(request.url.path());
                request.url = endpoint;
            }
            request
        };
        let _permit = self
            .native_http
            .gate
            .clone()
            .try_acquire_owned()
            .map_err(|_| RelayError::RouteUnavailable)?;
        tokio::time::timeout(self.limits.total_timeout, async {
            let get_body = |url: reqwest::Url| async {
                if url.as_str().contains(&request.credential) {
                    return Err(RelayError::RouteUnavailable);
                }
                // ProviderTransport rejects arbitrary queries. Only this private,
                // fixed-path plan constructs the usage query itself; connect and
                // pin the query-free same-origin URL, then send that generated URL.
                let mut connection_url = url.clone();
                if connection_url.query().is_some() {
                    if !request.openai_subscription
                        || connection_url.path() != "/v1/dashboard/billing/usage"
                    {
                        return Err(RelayError::RouteUnavailable);
                    }
                    connection_url.set_query(None);
                }
                let connection = self
                    .native_http
                    .transport
                    .prepare(&connection_url, &self.limits)
                    .await?;
                let get = connection
                    .client()
                    .get(url)
                    .header(header::ACCEPT, "application/json");
                let get = match (
                    request.openai_subscription,
                    request.api_key_auth,
                    request.protocol,
                ) {
                    (true, _, _) => get.bearer_auth(&request.credential),
                    (false, true, _) => get.header("api-key", &request.credential),
                    (false, false, crate::routing::Protocol::AnthropicMessages) => get
                        .header("x-api-key", &request.credential)
                        .header("anthropic-version", "2023-06-01"),
                    (false, false, crate::routing::Protocol::Gemini) => {
                        get.header("x-goog-api-key", &request.credential)
                    }
                    (false, false, _) => get.bearer_auth(&request.credential),
                };
                let response = get.send().await.map_err(|_| RelayError::RouteUnavailable)?;
                if !response.status().is_success() {
                    return Err(RelayError::RouteUnavailable);
                }
                native_http::read_response_bytes(&connection, response, &self.limits).await
            };
            let bytes = get_body(request.url.clone()).await?;
            if request.openai_subscription {
                use chrono::Datelike;
                if String::from_utf8_lossy(&bytes).contains(&request.credential) {
                    return Err(RelayError::RouteUnavailable);
                }
                let subscription: Value =
                    serde_json::from_slice(&bytes).map_err(|_| RelayError::RouteUnavailable)?;
                let paid = subscription
                    .get("has_payment_method")
                    .and_then(Value::as_bool)
                    .ok_or(RelayError::RouteUnavailable)?;
                let limit = subscription
                    .get("hard_limit_usd")
                    .and_then(Value::as_f64)
                    .filter(|n| n.is_finite())
                    .ok_or(RelayError::RouteUnavailable)?;
                let today = chrono::Local::now().date_naive();
                let start = if paid {
                    today.with_day(1)
                } else {
                    today.checked_sub_days(chrono::Days::new(100))
                }
                .ok_or(RelayError::RouteUnavailable)?;
                let mut usage_url = request
                    .url
                    .join("/v1/dashboard/billing/usage")
                    .map_err(|_| RelayError::RouteUnavailable)?;
                if usage_url.origin() != request.url.origin() {
                    return Err(RelayError::RouteUnavailable);
                }
                usage_url
                    .query_pairs_mut()
                    .append_pair("start_date", &start.to_string())
                    .append_pair("end_date", &today.to_string());
                let usage = get_body(usage_url).await?;
                if String::from_utf8_lossy(&usage).contains(&request.credential) {
                    return Err(RelayError::RouteUnavailable);
                }
                let usage: Value =
                    serde_json::from_slice(&usage).map_err(|_| RelayError::RouteUnavailable)?;
                let used = usage
                    .get("total_usage")
                    .and_then(Value::as_f64)
                    .filter(|n| n.is_finite())
                    .ok_or(RelayError::RouteUnavailable)?;
                let balance = limit - used / 100.0;
                if !balance.is_finite() || balance.to_string().contains(&request.credential) {
                    return Err(RelayError::RouteUnavailable);
                }
                return Ok(balance);
            }
            parse_balance(
                &bytes,
                &request.pointer,
                request.subtract_pointer.as_deref(),
                &request.requirements,
                request.cny_per_usd,
                request.deepseek_currency_array,
                &request.credential,
            )
        })
        .await
        .map_err(|_| RelayError::RouteUnavailable)?
    }
}

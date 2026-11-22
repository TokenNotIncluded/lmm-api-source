//! Host-only bounded personal text execution using the real gateway router.
//! There is no deserializable URL, price, payer or upstream credential input.
#[path = "assistant_agent.rs"]
mod agent;
pub(crate) use agent::{AgentAnswer, AgentObservation, PreparedAgent};
#[path = "assistant_anthropic.rs"]
mod anthropic;
use anthropic::{
    assistant_anthropic_decoder_input, assistant_request, assistant_response,
    assistant_wire_payload,
};

use super::*;
use crate::routing::{CreditRate, Protocol, ReplayPermission, RouteRequest};
use serde_json::Value;
use tokio_stream::StreamExt;

pub(crate) struct PreparedAssistant {
    pub price: crate::billing::charging::Price,
    pub reserve: i64,
    pub fingerprint: [u8; 32],
    route: relay::Route,
    payload: Value,
    wire_payload: Value,
    input_ceiling: i64,
    output_ceiling: u32,
    connection: crate::relay::provider_network::ProviderConnection,
    attempt: Arc<crate::routing::Attempt>,
    gift_tools: bool,
}
pub(crate) enum AssistantAnswer {
    Text(String),
    Gift(super::assistant_gift::GiftToolCall),
}
pub(crate) struct AssistantObservation {
    pub answer: AssistantAnswer,
    pub usage: crate::billing::charging::Usage,
}
impl ModelGateway {
    pub(crate) fn assistant_parallelism(&self) -> usize {
        self.limits.max_in_flight.min(16)
    }
    pub(crate) fn assistant_quote(
        &self,
        model: &str,
        group: &str,
    ) -> Result<crate::billing::charging::Price, RelayError> {
        let mut plan = self
            .bridge
            .routing
            .route(RouteRequest {
                model,
                group,
                speed: None,
                protocol: crate::routing::Protocol::OpenAiChat,
                replay: crate::routing::ReplayPermission::Denied,
            })
            .map_err(|_| RelayError::RouteUnavailable)?;
        let mut price = routing::credit_price(plan.quote(), plan.canonical_model(), group)?;
        let attempt = plan.start().map_err(|_| RelayError::RouteUnavailable)?;
        assistant_upstream_protocol(attempt.protocol())?;
        freeze_assistant_tariff(&mut price, &self.bridge.provider_policy, &attempt)?;
        Ok(price)
    }
    pub(crate) fn validate_assistant_policy(
        &self,
        model: &str,
        group: &str,
        max_output: u32,
    ) -> Result<(), RelayError> {
        let mut plan = self
            .bridge
            .routing
            .route(RouteRequest {
                model,
                group,
                speed: None,
                protocol: Protocol::OpenAiChat,
                replay: ReplayPermission::Denied,
            })
            .map_err(|_| RelayError::RouteUnavailable)?;
        let price = routing::credit_price(plan.quote(), plan.canonical_model(), group)?;
        if price.upstream_cost_group_ppm.is_some() {
            return Err(RelayError::Unsupported("assistant requires token quote"));
        }
        let attempt = plan.start().map_err(|_| RelayError::RouteUnavailable)?;
        assistant_upstream_protocol(attempt.protocol())?;
        if attempt.payload_policy() != crate::routing::PayloadPolicy::Adapt
            || u128::from(attempt.timeout_ms()) < self.limits.total_timeout.as_millis()
        {
            return Err(RelayError::RouteUnavailable);
        }
        self.bridge.provider_policy.reservation_input_bound(
            &attempt,
            self.bridge.policy.input_token_ceiling,
            max_output,
        )?;
        let reference = attempt.credential_ref();
        if !self
            .bridge
            .credentials
            .contains_key(&(reference.id, reference.revision))
        {
            return Err(RelayError::RouteUnavailable);
        }
        Ok(())
    }
    // Model, group, quote, transport and actor stay separate call inputs.
    #[allow(clippy::too_many_arguments)]
    pub(crate) async fn prepare_assistant(
        &self,
        model: &str,
        group: &str,
        max_output: u32,
        messages: Value,
        transport: &crate::relay::provider_network::ProviderTransport,
        gift_cap: Option<i64>,
        stored_price: &crate::billing::charging::Price,
        actor_user_id: i64,
    ) -> Result<PreparedAssistant, RelayError> {
        if max_output == 0 || max_output > 4096 {
            return Err(RelayError::Invalid("assistant output limit"));
        }
        let mut plan = self
            .bridge
            .routing
            .route(RouteRequest {
                model,
                group,
                speed: None,
                protocol: Protocol::OpenAiChat,
                replay: ReplayPermission::Denied,
            })
            .map_err(|_| RelayError::RouteUnavailable)?;
        let quote = plan.quote().with_credit_rate(
            CreditRate::new(1, 500_000).map_err(|_| RelayError::BillingUnavailable)?,
        );
        let mut price = routing::credit_price(quote.price(), plan.canonical_model(), group)?;
        if price.upstream_cost_group_ppm.is_some() {
            return Err(RelayError::Unsupported("assistant requires token quote"));
        }
        let attempt = Arc::new(plan.start().map_err(|_| RelayError::RouteUnavailable)?);
        let upstream = assistant_upstream_protocol(attempt.protocol())?;
        if attempt.payload_policy() != crate::routing::PayloadPolicy::Adapt
            || !attempt.is_available()
            || u128::from(attempt.timeout_ms()) < self.limits.total_timeout.as_millis()
        {
            return Err(RelayError::RouteUnavailable);
        }
        freeze_assistant_tariff(&mut price, &self.bridge.provider_policy, &attempt)?;
        price = bind_stored_assistant_price(&price, stored_price, actor_user_id)?;
        let endpoint = routing::operation_endpoint(
            attempt.endpoint(),
            upstream,
            attempt.upstream_model(),
            false,
        )?;
        let local = endpoint
            .host_str()
            .and_then(|h| h.trim_matches(['[', ']']).parse::<IpAddr>().ok())
            .is_some_and(|ip| ip.is_loopback());
        let input_ceiling = self.bridge.provider_policy.reservation_input_bound(
            &attempt,
            self.bridge.policy.input_token_ceiling,
            max_output,
        )?;
        let reference = attempt.credential_ref();
        let secret = self
            .bridge
            .credentials
            .get(&(reference.id, reference.revision))
            .ok_or(RelayError::RouteUnavailable)?;
        let availability = attempt.clone();
        let route = relay::Route::new(
            attempt.route_key(),
            upstream,
            endpoint.as_str(),
            attempt.upstream_model().to_owned(),
            &secret,
            attempt.quote().version(),
        )?
        .with_availability_guard(Arc::new(move || availability.is_available()));
        let mut payload =
            json!({"model":route.model,"messages":messages,"max_tokens":max_output,"stream":false});
        let gift_tools = gift_cap.is_some();
        if let Some(cap) = gift_cap {
            payload["tools"] = match upstream {
                relay::UpstreamProtocol::Anthropic => {
                    super::assistant_gift::anthropic_tool_schemas(cap)
                }
                _ => super::assistant_gift::tool_schemas(cap),
            };
            payload["parallel_tool_calls"] = json!(false);
            payload["messages"].as_array_mut().ok_or(RelayError::Invalid("assistant messages"))?.insert(0,json!({"role":"system","content":"You are the site's onboarding assistant. Only discuss onboarding, account safety and new-user gifts. Use only the fixed Core gift tools. Never claim a gift was credited without a Core receipt. Never request secrets. Zero means no gift; Core enforces eligibility, caps and spending. Ask at least two distinct substantive onboarding questions before proposing a positive gift; never request personal secrets."}));
        }
        let wire_payload =
            assistant_wire_payload(&payload, route.protocol, &route.model, &self.limits)?;
        let bytes = serde_json::to_vec(&wire_payload)
            .map_err(|_| RelayError::Invalid("assistant payload"))?;
        // Synthetic local contract: all serialized UTF8 bytes plus framing are
        // bounded before reserving; never apply this to an external tokenizer.
        if bytes.len() > self.limits.request_bytes
            || (local && bytes.len() as i64 + 1024 > input_ceiling)
        {
            return Err(RelayError::Limit("assistant context budget"));
        }
        let reserve = price
            .reservation_ceiling(crate::billing::charging::ReservationBasis::Tokens {
                input: input_ceiling as u64,
                output: u64::from(max_output),
            })
            .map_err(|_| RelayError::BillingUnavailable)?;
        let connection = tokio::time::timeout(
            self.limits.total_timeout,
            transport.prepare(&route.endpoint, &self.limits),
        )
        .await
        .map_err(|_| RelayError::RouteUnavailable)??;
        Ok(PreparedAssistant {
            price,
            reserve,
            fingerprint: Sha256::digest(bytes).into(),
            route,
            payload,
            wire_payload,
            input_ceiling,
            output_ceiling: max_output,
            connection,
            attempt,
            gift_tools,
        })
    }
    /// Called only after the original Core worker claim has committed. No retry.
    pub(crate) async fn execute_assistant(
        &self,
        p: PreparedAssistant,
    ) -> Result<AssistantObservation, RelayError> {
        tokio::time::timeout(self.limits.total_timeout, self.execute_assistant_inner(p))
            .await
            .map_err(|_| RelayError::SettlementPending)?
    }
    async fn execute_assistant_inner(
        &self,
        p: PreparedAssistant,
    ) -> Result<AssistantObservation, RelayError> {
        let (bytes, value, usage) = self.fetch_assistant_inner(&p).await?;
        let gift = if p.gift_tools {
            match p.route.protocol {
                relay::UpstreamProtocol::Chat => super::assistant_gift::parse(&value, &bytes)?,
                relay::UpstreamProtocol::Anthropic => {
                    super::assistant_gift::parse_anthropic(&bytes)?
                }
                _ => return Err(RelayError::RouteUnavailable),
            }
        } else {
            None
        };
        let value = assistant_response(
            &p.payload,
            &bytes,
            p.route.protocol,
            p.price.tiered.as_ref().and_then(|t| t.cache_creation_mode),
            &self.limits,
        )?;
        let choices = value
            .get("choices")
            .and_then(Value::as_array)
            .ok_or(RelayError::SettlementPending)?;
        let answer = if let Some(call) = gift {
            AssistantAnswer::Gift(call)
        } else {
            if choices.len() != 1
                || !matches!(
                    choices[0].get("finish_reason").and_then(Value::as_str),
                    Some("stop" | "length")
                )
            {
                return Err(RelayError::SettlementPending);
            }
            let tool_calls = choices[0].pointer("/message/tool_calls");
            if tool_calls.is_some_and(|v| !v.is_null() && !v.as_array().is_some_and(Vec::is_empty))
                || choices[0]
                    .pointer("/message/function_call")
                    .is_some_and(|v| !v.is_null())
            {
                return Err(RelayError::SettlementPending);
            }
            let answer = choices[0]
                .pointer("/message/content")
                .and_then(Value::as_str)
                .filter(|s| !s.is_empty() && s.len() <= 32768)
                .ok_or(RelayError::SettlementPending)?
                .to_owned();
            AssistantAnswer::Text(answer)
        };
        Ok(AssistantObservation { answer, usage })
    }
    async fn fetch_assistant_inner(
        &self,
        p: &PreparedAssistant,
    ) -> Result<(Vec<u8>, Value, crate::billing::charging::Usage), RelayError> {
        if !p.route.is_available() {
            return Err(RelayError::RouteUnavailable);
        }
        let response = assistant_request(p.connection.client(), &p.route, &p.wire_payload)?
            .send()
            .await
            .map_err(|_| RelayError::SettlementPending)?;
        if !p.connection.matches_peer(response.remote_addr()) || !response.status().is_success() {
            return Err(RelayError::SettlementPending);
        }
        let mut bytes = Vec::new();
        let mut stream = response.bytes_stream();
        while let Some(chunk) = stream.next().await {
            let chunk = chunk.map_err(|_| RelayError::SettlementPending)?;
            if bytes
                .len()
                .checked_add(chunk.len())
                .is_none_or(|n| n > 131072)
            {
                return Err(RelayError::Limit("assistant provider output"));
            }
            bytes.extend_from_slice(&chunk);
        }
        let value: Value =
            serde_json::from_slice(&bytes).map_err(|_| RelayError::SettlementPending)?;
        // Charge the original supplier counters, before protocol projection.
        let observed = assistant_provider_usage(p.route.protocol, &p.price, &value["usage"])?;
        self.bridge
            .provider_policy
            .validate_usage(&p.attempt, &observed, false)?;
        let usage = assistant_usage(&p.price, p.input_ceiling, p.output_ceiling, observed)?;
        Ok((bytes, value, usage))
    }
}

fn assistant_upstream_protocol(protocol: Protocol) -> Result<relay::UpstreamProtocol, RelayError> {
    match protocol {
        Protocol::OpenAiChat => Ok(relay::UpstreamProtocol::Chat),
        Protocol::AnthropicMessages => Ok(relay::UpstreamProtocol::Anthropic),
        _ => Err(RelayError::RouteUnavailable),
    }
}

fn freeze_assistant_tariff(
    price: &mut crate::billing::charging::Price,
    policy: &ProviderPolicy,
    attempt: &crate::routing::Attempt,
) -> Result<(), RelayError> {
    if let Some(tariff) = &mut price.tiered {
        let compiled = tariff
            .tariff
            .compile()
            .map_err(|_| RelayError::BillingUnavailable)?;
        if tariff.tariff.revision == 4 && attempt.protocol() != Protocol::OpenAiChat {
            return Err(RelayError::Unsupported(
                "twelve-hour cache pricing requires an OpenAI protocol route",
            ));
        }
        // Match the generic gateway: the selected host route, not the logical
        // Chat envelope or a client field, fixes the cache creation semantics.
        tariff.input_semantics = if attempt.protocol() == Protocol::AnthropicMessages {
            crate::routing::InputSemantics::AnthropicText
        } else {
            crate::routing::InputSemantics::Inclusive
        };
        if compiled.requires_cache_creation() {
            tariff.missing_cache_creation_is_zero = policy.missing_cache_creation_is_zero(attempt);
        }
    }
    Ok(())
}

fn assistant_provider_usage(
    protocol: relay::UpstreamProtocol,
    price: &crate::billing::charging::Price,
    value: &Value,
) -> Result<relay::Usage, RelayError> {
    relay::provider_usage_with_cache_creation_mode(
        protocol,
        value,
        price
            .tiered
            .as_ref()
            .and_then(|tariff| tariff.cache_creation_mode),
    )
    .map_err(|_| RelayError::SettlementPending)
}

/// Only the authenticated Core grant/turn row supplies this persisted price.
/// Bind its original evidence unchanged; it cannot mint a new entitlement.
fn bind_stored_assistant_price(
    base: &crate::billing::charging::Price,
    stored: &crate::billing::charging::Price,
    actor_user_id: i64,
) -> Result<crate::billing::charging::Price, RelayError> {
    base.validate()
        .map_err(|_| RelayError::BillingUnavailable)?;
    stored
        .validate()
        .map_err(|_| RelayError::BillingUnavailable)?;
    if actor_user_id <= 0
        || base.user_trust.is_some()
        || stored.base_price() != *base
        || stored
            .user_trust
            .as_ref()
            .is_some_and(|trust| trust.user_id() != actor_user_id)
    {
        return Err(RelayError::Forbidden);
    }
    Ok(stored.clone())
}

fn assistant_usage(
    price: &crate::billing::charging::Price,
    input_ceiling: i64,
    output_ceiling: u32,
    observed: relay::Usage,
) -> Result<crate::billing::charging::Usage, RelayError> {
    if input_ceiling < 0
        || observed
            .input_tokens
            .is_some_and(|input| input > input_ceiling as u64)
        || observed
            .output_tokens
            .is_some_and(|output| output > u64::from(output_ceiling))
    {
        return Err(RelayError::SettlementPending);
    }
    if let Some(tariff) = &price.tiered {
        billing::complete_tiered_usage(observed, tariff)
    } else {
        billing::complete_usage(observed, false)
    }
}

#[cfg(test)]
mod usage_tests {
    use super::*;
    use crate::billing::charging::{CacheCreationUsage, Price};

    fn price(missing_creation_is_zero: bool) -> Price {
        let price: Price = serde_json::from_value(json!({
            "version":"routing-2-rule-1","model":"model","group":"allowed",
            "input_per_million":0,"cached_per_million":0,"output_per_million":0,
            "tiered":{
                "input_semantics":"inclusive","group_multiplier_ppm":1000000,
                "speed_multiplier_ppm":1000000,"credits_per_usd":500000,
                "missing_cache_creation_is_zero":missing_creation_is_zero,
                "tariff":{"revision":2,"source_digest":"34".repeat(32),
                    "uses_cache_read":true,"uses_cache_write":true,
                    "tiers":[{"up_to_input_tokens":null,"rates":{
                        "input":{"numerator":2,"denominator":1},
                        "output":{"numerator":10,"denominator":1},
                        "cache_read":{"numerator":1,"denominator":5},
                        "cache_write":{"numerator":5,"denominator":2}
                    }}]}
            }
        }))
        .unwrap();
        price.validate().unwrap();
        price
    }

    fn observed(value: Value) -> relay::Usage {
        relay::provider_usage(relay::UpstreamProtocol::Chat, &value).unwrap()
    }

    fn generic_price(missing_creation_is_zero: bool) -> Price {
        let mut encoded = serde_json::to_value(price(missing_creation_is_zero)).unwrap();
        encoded["version"] = json!("routing-3-rule-1");
        encoded["tiered"]["cache_creation_mode"] = json!("generic_remainder_v1");
        encoded["tiered"]["tariff"]["revision"] = json!(3);
        encoded["tiered"]["tariff"]["uses_cache_write_1h"] = json!(true);
        encoded["tiered"]["tariff"]["tiers"][0]["rates"]["cache_write_1h"] =
            json!({"numerator":4,"denominator":1});
        let price: Price = serde_json::from_value(encoded).unwrap();
        price.validate().unwrap();
        price
    }

    fn stored_trust_price(base: &Price, user_id: i64) -> Price {
        let mut encoded = serde_json::to_value(base).unwrap();
        encoded["user_trust"] = json!({
            "user_id":user_id,"policy_revision":1,"basis_revision":2,
            "effective_level":1,"level_source":"automatic",
            "discount":{"numerator":9,"denominator":10},
            "evaluated_at":100,"next_decay_at":200,"evidence_digest":vec![1u8;32]
        });
        serde_json::from_value(encoded).unwrap()
    }

    #[test]
    fn stored_logical_quote_reuses_exact_trust_evidence_and_legacy_absence() {
        let base = price(false);
        let stored = stored_trust_price(&base, 7);
        let original = serde_json::to_value(&stored).unwrap();
        for _ in 0..3 {
            let bound = bind_stored_assistant_price(&base, &stored, 7).unwrap();
            assert_eq!(serde_json::to_value(bound).unwrap(), original);
        }
        let legacy = bind_stored_assistant_price(&base, &base, 7).unwrap();
        assert_eq!(legacy, base);
        assert!(legacy.user_trust.is_none());
        assert!(
            serde_json::to_value(legacy)
                .unwrap()
                .get("user_trust")
                .is_none()
        );
    }

    #[test]
    fn stored_quote_rejects_another_actor_or_changed_base_contract() {
        let base = price(false);
        let stored = stored_trust_price(&base, 7);
        assert_eq!(
            bind_stored_assistant_price(&base, &stored, 8),
            Err(RelayError::Forbidden)
        );
        assert_eq!(
            bind_stored_assistant_price(&base, &stored, 0),
            Err(RelayError::Forbidden)
        );
        for change in ["version", "model", "group", "provider_contract"] {
            let mut changed = base.clone();
            match change {
                "version" => changed.version = "routing-3-rule-1".into(),
                "model" => changed.model = "other-model".into(),
                "group" => changed.group = "other-group".into(),
                "provider_contract" => {
                    changed
                        .tiered
                        .as_mut()
                        .unwrap()
                        .missing_cache_creation_is_zero = true
                }
                _ => unreachable!(),
            }
            assert_eq!(
                bind_stored_assistant_price(&changed, &stored, 7),
                Err(RelayError::Forbidden)
            );
        }
        assert_eq!(
            bind_stored_assistant_price(&stored, &stored, 7),
            Err(RelayError::Forbidden)
        );
    }

    fn gateway(missing_creation_is_zero: bool, protocol: Protocol) -> ModelGateway {
        let config = serde_json::from_value(json!({
            "version":1,"groups":[{"name":"allowed","enabled":true}],
            "upstreams":[{"id":1,"endpoint":"https://provider.example/v1","protocol":protocol,"credential":{"id":7,"revision":1},"enabled":true}],
            "routes":[{"id":1,"model":"model","group":"allowed","enabled":true,"speeds":["standard"],
                "targets":[{"id":1,"upstream_id":1,"upstream_model":"bounded-model","enabled":true,"priority":0,"weight":1,"speeds":["standard"],"payload":"adapt","timeout_ms":5000}],
                "failover":{"max_attempts":1,"retry_on":[],"allow_replay_after_send":false}}],"aliases":[]
        })).unwrap();
        let prices = serde_json::from_value(json!({"version":2,"rules":[{
            "id":1,"model":"model","group":"allowed","speed":"standard",
            "billing_mode":"tiered_decimal","tiered":price(false).tiered.unwrap().tariff,
            "rates":{"input":0,"output":0,"cache_read":0,"cache_write":0,"request":0},
            "group_multiplier_ppm":1000000,"speed_multiplier_ppm":1000000
        }]}))
        .unwrap();
        let router = crate::routing::Router::new(
            config,
            Arc::new(crate::routing::PriceSnapshot::compile(prices).unwrap()),
        )
        .unwrap();
        let policy = ProviderPolicy::new(
            vec![ExternalModelContext {
                metadata: None,
                responses_compaction: false,
                responses_continuity: None,
                pre_acceptance_rejection_statuses: Vec::new(),
                grounding: None,

                upstream_id: 1,
                upstream_model: "bounded-model".into(),
                protocol,
                endpoint: "https://provider.example/v1".into(),
                max_input_tokens: 100,
                max_output_tokens: 5,
                contract_revision: "synthetic-assistant-cache-v2".into(),
                missing_cached_tokens_is_zero: false,
                reservation: Default::default(),
                missing_cache_creation_tokens_is_zero: missing_creation_is_zero,
            }],
            100,
        )
        .unwrap();
        // No connection is opened: quote selection and the common freeze helper
        // are in-memory operations, before any transport or charging work.
        let pool = sqlx::postgres::PgPoolOptions::new()
            .connect_lazy("postgres://unused:unused@127.0.0.1:1/unused")
            .unwrap();
        ModelGateway::new_with_provider_policy(
            pool,
            Arc::new(router),
            vec![UpstreamCredential {
                reference: crate::routing::CredentialRef { id: 7, revision: 1 },
                secret: "synthetic-assistant-provider-key".into(),
            }],
            BTreeMap::new(),
            relay::Limits {
                total_timeout: Duration::from_secs(1),
                hook_timeout: Duration::from_millis(100),
                ..Default::default()
            },
            ChargePolicy {
                input_token_ceiling: 100,
                lease_seconds: 10,
            },
            policy,
        )
        .unwrap()
    }

    // Opt-in offline operator check. Normal CI has neither variable and keeps
    // the synthetic matrix below. The typed input has no credential contents.
    fn reviewed_route_quote_fixture() {
        use std::io::{Read, Write};

        let input = std::env::var_os("LMM_ASSISTANT_QUOTE_FIXTURE");
        let output = std::env::var_os("LMM_ASSISTANT_QUOTE_OUTPUT");
        let (input, output) = match (input, output) {
            (None, None) => return,
            (Some(input), Some(output)) => (input, output),
            _ => panic!("both explicit quote fixture paths are required"),
        };
        #[derive(serde::Deserialize)]
        #[serde(deny_unknown_fields)]
        struct Fixture {
            schema_version: u32,
            source_sha256: BTreeMap<String, String>,
            routing: crate::routing::RoutingConfig,
            prices: crate::routing::PriceConfig,
            provider_context: ExternalModelContext,
            model: String,
            group: String,
            installation_input_ceiling: i64,
            max_output_tokens: u32,
            timeout_seconds: u64,
            lease_seconds: i64,
            input_bounds: Vec<u64>,
        }
        let file = std::fs::File::open(input).expect("open quote fixture");
        assert!(file.metadata().unwrap().is_file());
        assert!(file.metadata().unwrap().len() <= 65_536);
        let mut bytes = Vec::new();
        file.take(65_537).read_to_end(&mut bytes).unwrap();
        assert!(!bytes.is_empty() && bytes.len() <= 65_536);
        let fixture: Fixture = serde_json::from_slice(&bytes).expect("strict quote fixture schema");
        assert_eq!(fixture.schema_version, 1);
        assert!(!fixture.source_sha256.is_empty() && fixture.source_sha256.len() <= 8);
        for (name, digest) in &fixture.source_sha256 {
            assert!(!name.is_empty() && name.len() <= 64);
            assert!(digest.len() == 64 && digest.bytes().all(|b| b.is_ascii_hexdigit()));
        }
        assert_eq!(fixture.routing.groups.len(), 1);
        assert_eq!(fixture.routing.upstreams.len(), 1);
        assert_eq!(fixture.routing.routes.len(), 1);
        assert!(fixture.routing.aliases.is_empty());
        assert_eq!(fixture.routing.routes[0].targets.len(), 1);
        assert_eq!(fixture.prices.rules.len(), 1);
        assert_eq!(
            fixture.provider_context.protocol,
            Protocol::AnthropicMessages
        );
        assert!((1..=4096).contains(&fixture.max_output_tokens));
        assert!((1..=300).contains(&fixture.timeout_seconds));
        assert!(!fixture.input_bounds.is_empty() && fixture.input_bounds.len() <= 8);
        assert!(fixture.input_bounds.windows(2).all(|w| w[0] < w[1]));
        let credential = fixture.routing.upstreams[0].credential;
        let router = crate::routing::Router::new(
            fixture.routing,
            Arc::new(crate::routing::PriceSnapshot::compile(fixture.prices).unwrap()),
        )
        .unwrap();
        let provider_policy = ProviderPolicy::new(
            vec![fixture.provider_context],
            fixture.installation_input_ceiling,
        )
        .unwrap();
        // Lazy pool construction and route selection perform no database or
        // network work. No ProviderTransport or execution is constructed here.
        let pool = sqlx::postgres::PgPoolOptions::new()
            .connect_lazy("postgres://unused:unused@127.0.0.1:1/unused")
            .unwrap();
        let gateway = ModelGateway::new_with_provider_policy(
            pool,
            Arc::new(router),
            vec![UpstreamCredential {
                reference: credential,
                secret: "offline-quote-placeholder-not-a-provider-secret".into(),
            }],
            BTreeMap::new(),
            relay::Limits {
                total_timeout: Duration::from_secs(fixture.timeout_seconds),
                ..Default::default()
            },
            ChargePolicy {
                input_token_ceiling: fixture.installation_input_ceiling,
                lease_seconds: fixture.lease_seconds,
            },
            provider_policy,
        )
        .unwrap();
        let quoted = gateway
            .assistant_quote(&fixture.model, &fixture.group)
            .unwrap();
        let mut plan = gateway
            .bridge
            .routing
            .route(RouteRequest {
                model: &fixture.model,
                group: &fixture.group,
                speed: None,
                protocol: Protocol::OpenAiChat,
                replay: ReplayPermission::Denied,
            })
            .unwrap();
        let quote = plan
            .quote()
            .with_credit_rate(CreditRate::new(1, 500_000).unwrap());
        let mut prepared =
            routing::credit_price(quote.price(), plan.canonical_model(), &fixture.group).unwrap();
        let attempt = plan.start().unwrap();
        assert_eq!(attempt.protocol(), Protocol::AnthropicMessages);
        freeze_assistant_tariff(&mut prepared, &gateway.bridge.provider_policy, &attempt).unwrap();
        assert_eq!(quoted, prepared);
        let prepared = bind_stored_assistant_price(&prepared, &quoted, 7).unwrap();
        assert_eq!(
            prepared.tiered.as_ref().unwrap().input_semantics,
            crate::routing::InputSemantics::AnthropicText
        );
        let input_bound = gateway
            .bridge
            .provider_policy
            .reservation_input_bound(
                &attempt,
                fixture.installation_input_ceiling,
                fixture.max_output_tokens,
            )
            .unwrap();
        assert_eq!(
            fixture.input_bounds.last().copied(),
            Some(input_bound as u64)
        );
        let ceilings: Vec<_> = fixture
            .input_bounds
            .iter()
            .map(|&input| {
                assert!(input > 0 && input <= input_bound as u64);
                let credits = prepared
                    .reservation_ceiling(crate::billing::charging::ReservationBasis::Tokens {
                        input,
                        output: u64::from(fixture.max_output_tokens),
                    })
                    .unwrap();
                json!({"input_tokens": input, "reservation_credits": credits})
            })
            .collect();
        let observed = json!({
            "schema_version": 1,
            "status": "PASS_COMPILED_ASSISTANT_ROUTE_QUOTE",
            "fixture_sha256": crate::hex_codec::encode(Sha256::digest(&bytes)),
            "source_sha256": fixture.source_sha256,
            "selected_protocol": attempt.protocol(),
            "quote_equals_preparation": true,
            "frozen_price": prepared,
            "input_bound": input_bound,
            "max_output_tokens": fixture.max_output_tokens,
            "ceilings": ceilings,
            "database_connections": 0,
            "provider_requests": 0,
        });
        let mut options = std::fs::OpenOptions::new();
        options.write(true).create_new(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            options.mode(0o600);
        }
        let mut output = options.open(output).expect("create private quote evidence");
        serde_json::to_writer(&mut output, &observed).unwrap();
        output.write_all(b"\n").unwrap();
    }

    #[tokio::test]
    async fn quote_and_preparation_freeze_the_same_selected_creation_contract() {
        reviewed_route_quote_fixture();
        let mut frozen_quotes = Vec::new();
        for (missing_creation_is_zero, protocol, semantics) in [
            (
                false,
                Protocol::OpenAiChat,
                crate::routing::InputSemantics::Inclusive,
            ),
            (
                true,
                Protocol::OpenAiChat,
                crate::routing::InputSemantics::Inclusive,
            ),
            (
                false,
                Protocol::AnthropicMessages,
                crate::routing::InputSemantics::AnthropicText,
            ),
            (
                true,
                Protocol::AnthropicMessages,
                crate::routing::InputSemantics::AnthropicText,
            ),
        ] {
            let gateway = gateway(missing_creation_is_zero, protocol);
            let quoted = gateway.assistant_quote("model", "allowed").unwrap();
            let mut plan = gateway
                .bridge
                .routing
                .route(RouteRequest {
                    model: "model",
                    group: "allowed",
                    speed: None,
                    protocol: Protocol::OpenAiChat,
                    replay: ReplayPermission::Denied,
                })
                .unwrap();
            let quote = plan
                .quote()
                .with_credit_rate(CreditRate::new(1, 500_000).unwrap());
            let mut prepared_price =
                routing::credit_price(quote.price(), plan.canonical_model(), "allowed").unwrap();
            let attempt = plan.start().unwrap();
            freeze_assistant_tariff(
                &mut prepared_price,
                &gateway.bridge.provider_policy,
                &attempt,
            )
            .unwrap();
            assert_eq!(quoted, prepared_price);
            let tariff = quoted.tiered.as_ref().unwrap();
            assert_eq!(tariff.input_semantics, semantics);
            assert_eq!(
                tariff.missing_cache_creation_is_zero,
                missing_creation_is_zero
            );
            let mut generic = generic_price(false);
            freeze_assistant_tariff(&mut generic, &gateway.bridge.provider_policy, &attempt)
                .unwrap();
            assert_eq!(
                generic
                    .tiered
                    .as_ref()
                    .unwrap()
                    .missing_cache_creation_is_zero,
                missing_creation_is_zero
            );
            assert_eq!(generic.tiered.as_ref().unwrap().input_semantics, semantics);
            let mut wrong_semantics = quoted.clone();
            wrong_semantics.tiered.as_mut().unwrap().input_semantics = match semantics {
                crate::routing::InputSemantics::Inclusive => {
                    crate::routing::InputSemantics::AnthropicText
                }
                crate::routing::InputSemantics::AnthropicText => {
                    crate::routing::InputSemantics::Inclusive
                }
            };
            assert_eq!(
                bind_stored_assistant_price(&prepared_price, &wrong_semantics, 7),
                Err(RelayError::Forbidden)
            );
            let mut invalid = prepared_price;
            invalid.tiered.as_mut().unwrap().tariff.revision = 4;
            assert_eq!(
                freeze_assistant_tariff(&mut invalid, &gateway.bridge.provider_policy, &attempt),
                Err(RelayError::BillingUnavailable)
            );
            frozen_quotes.push(quoted);
        }
        // The financial equality check must distinguish different selected
        // supplier contracts, even when all token rates remain identical.
        assert_ne!(frozen_quotes[0], frozen_quotes[1]);
        assert_ne!(frozen_quotes[2], frozen_quotes[3]);
        assert_ne!(frozen_quotes[0], frozen_quotes[2]);
    }

    #[test]
    fn revision_two_accepts_observed_zero_and_frozen_contract_zero() {
        let explicit = observed(json!({
            "prompt_tokens":100,"completion_tokens":5,"total_tokens":105,
            "prompt_tokens_details":{"cached_tokens":20,"cache_write_tokens":0}
        }));
        let expected = CacheCreationUsage {
            twelve_hour: None,
            cache_creation_mode: None,
            reported_total: None,
            total: 0,
            five_minute: Some(0),
            one_hour: Some(0),
        };
        let usage = assistant_usage(&price(false), 100, 5, explicit).unwrap();
        assert_eq!(usage.cache_creation, Some(expected));
        assert_eq!(price(false).cost(usage).unwrap(), 107);

        let missing = observed(json!({
            "prompt_tokens":100,"completion_tokens":5,
            "prompt_tokens_details":{"cached_tokens":20}
        }));
        assert_eq!(
            assistant_usage(&price(false), 100, 5, missing.clone()),
            Err(RelayError::SettlementPending)
        );
        assert_eq!(
            assistant_usage(&price(true), 100, 5, missing)
                .unwrap()
                .cache_creation,
            Some(expected)
        );
    }

    #[test]
    fn revision_two_keeps_nested_creation_and_ignores_reported_cost() {
        let receipt = observed(json!({
            "prompt_tokens":100,"completion_tokens":5,"total_tokens":105,"cost":999,
            "cache_creation_input_tokens":70,
            "cache_creation_details":{"ephemeral_5m_input_tokens":30,"ephemeral_1h_input_tokens":40},
            "prompt_tokens_details":{"cached_tokens":20,"cached_creation_tokens":70,"cache_write_tokens":70}
        }));
        let price = price(false);
        let usage = assistant_usage(&price, 100, 5, receipt).unwrap();
        assert_eq!(usage.input, 100);
        assert_eq!(usage.cached, 20);
        assert_eq!(usage.output, 5);
        assert_eq!(usage.upstream_cost_atto_usd, None);
        assert_eq!(
            usage.cache_creation,
            Some(CacheCreationUsage {
                twelve_hour: None,
                cache_creation_mode: None,
                reported_total: None,
                total: 70,
                five_minute: Some(30),
                one_hour: Some(40),
            })
        );
        assert_eq!(price.cost(usage).unwrap(), 125);
    }

    #[test]
    fn anthropic_original_counters_use_the_same_frozen_tariff_and_ceilings() {
        let price = price(false);
        let observed = assistant_provider_usage(
            relay::UpstreamProtocol::Anthropic,
            &price,
            &json!({"input_tokens":10,"output_tokens":5,"cache_read_input_tokens":20,
                "cache_creation_input_tokens":70,"cost":999,
                "cache_creation":{"ephemeral_5m_input_tokens":30,"ephemeral_1h_input_tokens":40}}),
        )
        .unwrap();
        assert_eq!(observed.input_tokens, Some(100));
        assert_eq!(observed.cached_input_tokens, Some(20));
        let usage = assistant_usage(&price, 100, 5, observed.clone()).unwrap();
        assert_eq!(usage.input, 100);
        assert_eq!(usage.cached, 20);
        assert_eq!(usage.output, 5);
        assert_eq!(usage.upstream_cost_atto_usd, None);
        assert_eq!(usage.cache_creation.unwrap().total, 70);
        assert_eq!(price.cost(usage).unwrap(), 125);
        assert_eq!(
            assistant_usage(&price, 99, 5, observed.clone()),
            Err(RelayError::SettlementPending)
        );
        assert_eq!(
            assistant_usage(&price, 100, 4, observed),
            Err(RelayError::SettlementPending)
        );
    }

    #[test]
    fn revision_three_keeps_raw_creation_and_unknown_ttl_evidence() {
        let price = generic_price(false);
        for (reported, details, five, hour) in [
            (70, Value::Null, None, None),
            (70, json!({"ephemeral_5m_input_tokens":10}), Some(10), None),
            (
                20,
                json!({"ephemeral_5m_input_tokens":30,"ephemeral_1h_input_tokens":40}),
                Some(30),
                Some(40),
            ),
        ] {
            let receipt = assistant_provider_usage(
                relay::UpstreamProtocol::Chat,
                &price,
                &json!({
                    "prompt_tokens":100,"completion_tokens":5,"total_tokens":105,"cost":999,
                    "prompt_tokens_details":{"cached_tokens":20},
                    "cache_creation_input_tokens":reported,"cache_creation_details":details
                }),
            )
            .unwrap();
            let usage = assistant_usage(&price, 100, 5, receipt).unwrap();
            assert_eq!(usage.upstream_cost_atto_usd, None);
            assert_eq!(
                usage.cache_creation,
                Some(CacheCreationUsage {
                    twelve_hour: None,
                    cache_creation_mode: Some(
                        crate::routing::CacheCreationMode::GenericRemainderV1
                    ),
                    reported_total: Some(reported),
                    total: 70,
                    five_minute: five,
                    one_hour: hour,
                })
            );
            assert_eq!(price.cost(usage).unwrap(), 125);
        }
    }

    #[test]
    fn revision_three_interpretation_comes_only_from_the_frozen_price() {
        let receipt = json!({
            "prompt_tokens":100,"completion_tokens":5,
            "prompt_tokens_details":{"cached_tokens":20},
            "cache_creation_input_tokens":70,
            "cache_creation_details":{"ephemeral_5m_input_tokens":10,"ephemeral_1h_input_tokens":20},
            "cache_creation_mode":"generic_remainder_v1"
        });
        let generic = generic_price(false);
        let observed =
            assistant_provider_usage(relay::UpstreamProtocol::Chat, &generic, &receipt).unwrap();
        let usage = assistant_usage(&generic, 100, 5, observed.clone()).unwrap();
        assert_eq!(usage.cache_creation.unwrap().total, 70);
        assert_eq!(
            assistant_provider_usage(relay::UpstreamProtocol::Chat, &price(false), &receipt),
            Err(RelayError::SettlementPending)
        );
        assert_eq!(
            bind_stored_assistant_price(&price(false), &generic, 7),
            Err(RelayError::Forbidden)
        );
        assert_eq!(
            assistant_usage(&generic, 99, 5, observed.clone()),
            Err(RelayError::SettlementPending)
        );
        assert_eq!(
            assistant_usage(&generic, 100, 4, observed),
            Err(RelayError::SettlementPending)
        );
    }

    #[test]
    fn revision_three_absence_zero_requires_the_frozen_contract_and_no_ttl_evidence() {
        let missing = json!({
            "prompt_tokens":100,"completion_tokens":5,
            "prompt_tokens_details":{"cached_tokens":20}
        });
        let unknown = generic_price(false);
        let observed =
            assistant_provider_usage(relay::UpstreamProtocol::Chat, &unknown, &missing).unwrap();
        assert_eq!(
            assistant_usage(&unknown, 100, 5, observed.clone()),
            Err(RelayError::SettlementPending)
        );
        let contracted = generic_price(true);
        let usage = assistant_usage(&contracted, 100, 5, observed).unwrap();
        assert_eq!(
            usage.cache_creation,
            Some(CacheCreationUsage {
                twelve_hour: None,
                cache_creation_mode: Some(crate::routing::CacheCreationMode::GenericRemainderV1),
                reported_total: None,
                total: 0,
                five_minute: None,
                one_hour: None,
            })
        );
        assert_eq!(contracted.cost(usage).unwrap(), 107);
        assert_eq!(
            serde_json::to_value(usage.cache_creation.unwrap()).unwrap(),
            json!({"total":0,"cache_creation_mode":"generic_remainder_v1"})
        );

        let mut explicit_zero = missing.clone();
        explicit_zero["cache_creation_input_tokens"] = json!(0);
        let observed =
            assistant_provider_usage(relay::UpstreamProtocol::Chat, &unknown, &explicit_zero)
                .unwrap();
        let explicit = assistant_usage(&unknown, 100, 5, observed).unwrap();
        assert_eq!(explicit.cache_creation.unwrap().reported_total, Some(0));
        assert_eq!(explicit.cache_creation.unwrap().five_minute, None);
        assert_eq!(explicit.cache_creation.unwrap().one_hour, None);

        for details in [
            json!({}),
            json!({"ephemeral_5m_input_tokens":0}),
            json!({"ephemeral_1h_input_tokens":0}),
            json!({"ephemeral_5m_input_tokens":0,"ephemeral_1h_input_tokens":0}),
        ] {
            let mut partial = missing.clone();
            partial["cache_creation_details"] = details;
            let observed =
                assistant_provider_usage(relay::UpstreamProtocol::Chat, &contracted, &partial)
                    .unwrap();
            assert_eq!(
                assistant_usage(&contracted, 100, 5, observed),
                Err(RelayError::SettlementPending)
            );
        }
    }

    #[test]
    fn usage_above_original_ceiling_or_missing_totals_remains_pending() {
        let receipt = observed(json!({
            "prompt_tokens":100,"completion_tokens":5,
            "prompt_tokens_details":{"cached_tokens":20,"cache_write_tokens":0}
        }));
        for (input, output) in [(99, 5), (100, 4), (-1, 5)] {
            assert_eq!(
                assistant_usage(&price(true), input, output, receipt.clone()),
                Err(RelayError::SettlementPending)
            );
        }
        for value in [
            json!({}),
            json!({"prompt_tokens":100}),
            json!({"completion_tokens":5}),
        ] {
            assert_eq!(
                assistant_usage(&price(true), 100, 5, observed(value)),
                Err(RelayError::SettlementPending)
            );
        }
    }

    #[test]
    fn real_decoder_rejects_invalid_cache_and_inconsistent_totals() {
        for value in [
            json!({"prompt_tokens":100,"completion_tokens":5,"total_tokens":104}),
            json!({"prompt_tokens":100,"completion_tokens":5,"prompt_tokens_details":{"cached_tokens":-1}}),
            json!({"prompt_tokens":100,"completion_tokens":5,"cache_creation_input_tokens":70,"prompt_tokens_details":{"cache_write_tokens":71}}),
            json!({"prompt_tokens":100,"completion_tokens":5,"cache_creation_input_tokens":70,"cache_creation_details":{"ephemeral_5m_input_tokens":30,"ephemeral_1h_input_tokens":41}}),
        ] {
            assert!(relay::provider_usage(relay::UpstreamProtocol::Chat, &value).is_err());
        }
    }

    #[test]
    fn non_cache_tariff_cannot_settle_reported_cache_creation() {
        let mut price = price(false);
        price.tiered = None;
        price.input_per_million = 1;
        price.validate().unwrap();
        let receipt = observed(json!({
            "prompt_tokens":100,"completion_tokens":5,
            "prompt_tokens_details":{"cache_write_tokens":1}
        }));
        assert_eq!(
            assistant_usage(&price, 100, 5, receipt),
            Err(RelayError::SettlementPending)
        );
    }
}

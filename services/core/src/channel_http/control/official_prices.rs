//! Compile operator-owned model base prices into the immutable price release.
use super::*;
use crate::routing::{
    BillingMode, DecimalRate, MAX_MULTIPLIER, MULTIPLIER_ONE, PriceRule, Protocol, RoutingConfig,
    Speed, TieredRates, TieredTariff, TokenRates, TokenTier, valid_group_name,
};
use std::collections::BTreeMap;

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(in crate::channel_http) struct OfficialPrices {
    pub models: BTreeMap<String, OfficialPrice>,
    #[serde(default)]
    group_multipliers_ppm: BTreeMap<String, u32>,
    speed_multipliers_ppm: SpeedMultipliers,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(in crate::channel_http) struct OfficialPrice {
    rates: TokenRates,
    #[serde(default)]
    billing_mode: BillingMode,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    tiered: Option<TieredTariff>,
    source: String,
    source_unit: String,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct SpeedMultipliers {
    fast: u32,
    ultrafast: u32,
}
impl SpeedMultipliers {
    fn get(&self, s: Speed) -> u32 {
        match s {
            Speed::Standard => MULTIPLIER_ONE,
            Speed::Fast => self.fast,
            Speed::Ultrafast => self.ultrafast,
        }
    }
}
#[derive(Default)]
struct Coverage {
    required: bool,
    input_only: bool,
    openai: bool,
    anthropic: bool,
    gemini: bool,
    legacy_openrouter_chat: bool,
    ordinary_openai: bool,
}
impl Coverage {
    fn add(&mut self, protocol: Protocol, payload: crate::routing::PayloadPolicy) {
        self.required = true;
        match protocol {
            Protocol::SystemOne | Protocol::OpenAiEmbeddings | Protocol::JinaRerank => {
                self.input_only = true
            }
            Protocol::OpenAiChat | Protocol::OpenAiResponses => {
                self.openai = true;
                if protocol == Protocol::OpenAiChat
                    && payload == crate::routing::PayloadPolicy::LegacyOpenRouterChatUsageV1
                {
                    self.legacy_openrouter_chat = true;
                } else {
                    self.ordinary_openai = true;
                }
            }
            Protocol::AnthropicMessages => self.anthropic = true,
            Protocol::Gemini => self.gemini = true,
            Protocol::DeepSeekSearch | Protocol::DeepSeekFetch => {}
        }
    }
}
/// Project a validated official source tariff only where its fee expression is
/// equivalent for the selected providers. Its digest continues to identify the
/// original source, independently of the frozen route-specific representation.
fn project_official_tiered(
    tariff: &mut TieredTariff,
    coverage: &Coverage,
) -> Result<(), &'static str> {
    if coverage.legacy_openrouter_chat && tariff.revision != 3 {
        return Err(
            "The legacy OpenRouter Chat payload needs a revision 3 generic-remainder price",
        );
    }
    match tariff.revision {
        1 => Ok(()),
        3 if coverage.ordinary_openai || coverage.gemini || coverage.input_only => Err(
            "Revision 3 cache creation requires Anthropic or the legacy OpenRouter Chat payload",
        ),
        4 if coverage.anthropic || coverage.gemini || coverage.input_only => Err(
            "OpenAI twelve-hour cache creation prices require only OpenAI Chat or Responses targets",
        ),
        // The OpenAI adapter needs confirmed 30m/12h rates. Only revision 4
        // carries both. A generic tier entry makes no claim about 12h.
        2 if coverage.openai => {
            Err("Confirm the OpenAI twelve-hour cache-write price in every tier using revision 4")
        }
        2 if coverage.input_only => Err("Input-only targets do not report cache writes"),
        // In an Anthropic tariff cc prices 5m writes. An absent 1h coefficient
        // is not a confirmed free rate. Validation checks every tier, including
        // the calendar's alternate tiers, against these flags.
        2 if coverage.anthropic && (!tariff.uses_cache_write || !tariff.uses_cache_write_1h) => {
            Err("Confirm the Anthropic five-minute and one-hour cache-write price in every tier")
        }
        2 if coverage.gemini => {
            // Gemini reports cached reads and has no write meter. Dropping a
            // known zero aggregate changes no observable fee on this protocol.
            if coverage.anthropic
                || tariff.calendar.is_some()
                || !tariff.uses_cache_read
                || !tariff.uses_cache_write
                || tariff.uses_cache_write_1h
                || tariff.uses_cache_write_12h
                || tariff
                    .tiers
                    .iter()
                    .any(|tier| tier.rates.cache_write.numerator != 0)
            {
                return Err(
                    "Gemini tiered pricing needs a known cache-read price and a zero aggregate cache-write price without lifetime or calendar overrides",
                );
            }
            tariff.revision = 1;
            tariff.uses_cache_write = false;
            Ok(())
        }
        _ => Ok(()),
    }
}
impl OfficialPrice {
    fn rule(
        &self,
        id: u64,
        model: &str,
        group: &str,
        speed: Speed,
        group_multiplier_ppm: u32,
        speed_multiplier_ppm: u32,
    ) -> PriceRule {
        PriceRule {
            id,
            model: model.into(),
            group: group.into(),
            speed,
            billing_mode: self.billing_mode,
            rates: self.rates,
            tiered: self.tiered.clone(),
            rounding: None,
            group_multiplier_ppm,
            speed_multiplier_ppm,
        }
    }
    fn effective_rule(
        &self,
        mut rule: PriceRule,
        coverage: &Coverage,
    ) -> Result<PriceRule, ManagementError> {
        let incompatible = |reason| ManagementError::OfficialPriceIncompatible {
            model: rule.model.clone(),
            group: rule.group.clone(),
            speed: rule.speed,
            reason,
        };
        if let Some(tariff) = &mut rule.tiered {
            project_official_tiered(tariff, coverage).map_err(incompatible)?;
            return Ok(rule);
        }
        if coverage.legacy_openrouter_chat {
            return Err(incompatible(
                "The legacy OpenRouter Chat payload needs a revision 3 generic-remainder price",
            ));
        }
        let r = self.rates;
        if self.billing_mode != BillingMode::Tokens || r.request != 0 {
            return Ok(rule);
        }
        let has_write_meter = coverage.openai || coverage.anthropic;
        let text = has_write_meter || coverage.gemini;
        if coverage.input_only && (text || r.output != 0 || r.cache_read != 0 || r.cache_write != 0)
        {
            return Err(incompatible(
                "Input-only targets require a separate price with only an input rate",
            ));
        }
        if coverage.gemini && !has_write_meter && r.cache_write != 0 {
            return Err(incompatible(
                "Gemini does not report cache writes; confirm a zero cache-write price for a Gemini-only route",
            ));
        }
        let mixed_text =
            (coverage.openai && coverage.anthropic) || (coverage.gemini && has_write_meter);
        let writes = has_write_meter || (!text && r.cache_write != 0);
        // Keep every new token tariff exact through both multipliers. Existing
        // decimal contracts round once at settlement and round up reservations.
        let decimal = |amount| DecimalRate {
            numerator: amount,
            denominator: 1_000_000,
        };
        let source =
            serde_json::to_vec(&(&rule.model, self)).map_err(|_| ManagementError::Invalid)?;
        // A mixed route retains the exact uniform scalar until a target is
        // selected. The neutral table is never a settlement tariff.
        let openai_writes = coverage.openai && !mixed_text;
        rule.rates = TokenRates {
            input: 0,
            output: 0,
            cache_read: 0,
            cache_write: 0,
            request: 0,
        };
        rule.billing_mode = if mixed_text {
            BillingMode::OfficialFlat
        } else {
            BillingMode::TieredDecimal
        };
        rule.tiered = Some(TieredTariff {
            revision: if openai_writes {
                4
            } else if writes {
                2
            } else {
                1
            },
            source_digest: crate::hex_codec::encode(Sha256::digest(source)),
            uses_cache_read: !coverage.input_only,
            uses_cache_write: writes,
            uses_cache_write_1h: writes && !openai_writes,
            uses_cache_write_12h: openai_writes,
            calendar: None,
            tiers: vec![TokenTier {
                up_to_input_tokens: None,
                rates: TieredRates {
                    input: decimal(r.input),
                    output: decimal(r.output),
                    cache_read: decimal(r.cache_read),
                    cache_write: decimal(r.cache_write),
                    cache_write_1h: (writes && !openai_writes).then(|| decimal(r.cache_write)),
                    cache_write_12h: openai_writes.then(|| decimal(r.cache_write)),
                },
            }],
        });
        Ok(rule)
    }
}
impl OfficialPrices {
    fn validate(&self) -> Result<(), ManagementError> {
        if self.models.len() > 100_000
            || self.group_multipliers_ppm.len() > 100_000
            || self
                .group_multipliers_ppm
                .iter()
                .any(|(group, ppm)| !valid_group_name(group) || *ppm > MAX_MULTIPLIER)
            || [
                self.speed_multipliers_ppm.fast,
                self.speed_multipliers_ppm.ultrafast,
            ]
            .iter()
            .any(|p| !(1..=MAX_MULTIPLIER).contains(p))
        {
            return Err(ManagementError::Invalid);
        }
        let mut samples = Vec::with_capacity(self.models.len());
        for (model, price) in &self.models {
            // This wire mode is generated from a verified scalar source. It is
            // never an operator-supplied alternative to confirming tier rates.
            if price.billing_mode == BillingMode::OfficialFlat {
                return Err(ManagementError::Invalid);
            }
            let r = price.rates;
            if (price.billing_mode == BillingMode::Tokens
                && r.request != 0
                && [r.input, r.output, r.cache_read, r.cache_write]
                    .iter()
                    .any(|v| *v != 0))
                || price.source.trim().is_empty()
                || price.source.len() > 2048
                || price.source.chars().any(char::is_control)
                || price.source_unit.trim().is_empty()
                || price.source_unit.len() > 128
                || price.source_unit.chars().any(char::is_control)
            {
                return Err(ManagementError::Invalid);
            }
            samples.push(price.rule(
                samples.len() as u64 + 1,
                model,
                "default",
                Speed::Standard,
                MULTIPLIER_ONE,
                MULTIPLIER_ONE,
            ));
        }
        PriceSnapshot::compile(PriceConfig {
            version: 1,
            rules: samples,
        })
        .map(|_| ())
        .map_err(|_| ManagementError::Invalid)
    }
    pub(in crate::channel_http) fn compile(
        &self,
        routing: &RoutingConfig,
        prices: &PriceConfig,
    ) -> Result<PriceConfig, ManagementError> {
        self.validate()?;
        let upstreams: BTreeMap<_, _> = routing
            .upstreams
            .iter()
            .map(|u| (u.id, u.protocol))
            .collect();
        let mut covered = BTreeMap::new();
        for route in &routing.routes {
            if self.models.contains_key(&route.model) {
                for &speed in &route.speeds {
                    let mut coverage = Coverage {
                        required: route.enabled,
                        ..Coverage::default()
                    };
                    for target in route.targets.iter().filter(|t| t.speeds.contains(&speed)) {
                        coverage.add(
                            *upstreams
                                .get(&target.upstream_id)
                                .ok_or(ManagementError::Invalid)?,
                            target.payload,
                        );
                    }
                    covered.insert(
                        (route.model.as_str(), route.group.as_str(), speed),
                        coverage,
                    );
                }
            }
        }
        let effective = |id, model: &str, group: &str, speed, coverage: &Coverage| {
            let group_ppm = self
                .group_multipliers_ppm
                .get(group)
                .copied()
                .unwrap_or(MULTIPLIER_ONE);
            let base = &self.models[model];
            base.effective_rule(
                base.rule(
                    id,
                    model,
                    group,
                    speed,
                    group_ppm,
                    self.speed_multipliers_ppm.get(speed),
                ),
                coverage,
            )
        };
        let mut next = prices.clone();
        let mut max_id = 0;
        for (original, rule) in prices.rules.iter().zip(&mut next.rules) {
            max_id = max_id.max(original.id);
            if let Some(coverage) = covered.remove(&(
                original.model.as_str(),
                original.group.as_str(),
                original.speed,
            )) {
                *rule = effective(
                    original.id,
                    &original.model,
                    &original.group,
                    original.speed,
                    &coverage,
                )?;
            }
        }
        for ((model, group, speed), coverage) in covered {
            if coverage.required {
                max_id = max_id
                    .checked_add(1)
                    .filter(|id| *id <= i64::MAX as u64)
                    .ok_or(ManagementError::Invalid)?;
                next.rules
                    .push(effective(max_id, model, group, speed, &coverage)?);
            }
        }
        PriceSnapshot::compile(next.clone()).map_err(|_| ManagementError::Invalid)?;
        Ok(next)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        billing::charging::{CacheCreationUsage, Price, ReservationBasis, Usage},
        model_http::channel_operations::validate_publication_prices,
        routing::store::ReleaseDocument,
    };
    use serde_json::json;

    fn fixture() -> (OfficialPrices, RoutingConfig, PriceConfig) {
        let book = serde_json::from_value(json!({
            "models": {"alpha": {
                "source": "fixture", "source_unit": "micro-USD per million tokens",
                "rates":{"input":3,"output":5,"cache_read":1,"cache_write":0,"request":0}
            }},
            "group_multipliers_ppm":{"default":1250000,"spare":500000},
            "speed_multipliers_ppm":{"fast":1200000,"ultrafast":3000000}
        }))
        .unwrap();
        let route = |id: u64, group: &str, enabled: bool| {
            json!({
                "id":id,"model":"alpha","group":group,"enabled":enabled,
                "speeds":["standard","fast","ultrafast"],
                "targets":[{"id":id,"upstream_id":1,"upstream_model":"alpha","enabled":false,
                    "priority":0,"weight":1,"speeds":["fast"],"payload":"adapt","timeout_ms":1000}],
                "failover":{"max_attempts":1,"retry_on":[],"allow_replay_after_send":false}
            })
        };
        let routing = serde_json::from_value(json!({
            "version":2,"groups":[{"name":"default","enabled":true},{"name":"spare","enabled":false}],
            "upstreams":[{"id":1,"endpoint":"https://fixture.example/v1/","protocol":"openai_responses",
                "credential":{"id":1,"revision":1},"enabled":true}],
            "routes":[route(1,"default",true),route(2,"spare",false)],"aliases":[]
        })).unwrap();
        let prior = |id: u64, model: &str, group: &str| {
            json!({
                "id":id,"model":model,"group":group,"speed":"standard",
                "rates":{"input":80,"output":160,"cache_read":40,"cache_write":0,"request":0},
                "group_multiplier_ppm":1000000,"speed_multiplier_ppm":1000000,
                "rounding":"legacy_go_ratio_half_away_min1_v1"
            })
        };
        let prices = serde_json::from_value(json!({
            "version":7,"rules":[prior(23,"legacy","default"),prior(7,"alpha","spare")]
        }))
        .unwrap();
        (book, routing, prices)
    }
    fn snapshot(
        book: &OfficialPrices,
        routing: &RoutingConfig,
        prices: &PriceConfig,
    ) -> PriceSnapshot {
        let compiled = book.compile(routing, prices).unwrap();
        validate_publication_prices(&ReleaseDocument {
            contract_version: crate::routing::CONTRACT_VERSION,
            routing: routing.clone(),
            pricing: compiled.clone(),
        })
        .unwrap();
        PriceSnapshot::compile(compiled).unwrap()
    }
    #[test]
    fn official_projection_preserves_policy_legacy_and_ids() {
        let (book, routing, prices) = fixture();
        let compiled = book.compile(&routing, &prices).unwrap();
        assert_eq!(compiled.version, 7);
        assert_eq!(compiled.rules.len(), 6);
        assert_eq!(compiled.rules[0], prices.rules[0]);
        assert_eq!(compiled.rules[1].id, 7);
        assert_eq!(compiled, book.compile(&routing, &compiled).unwrap());
        let live = snapshot(&book, &routing, &prices);
        assert_eq!(
            live.quote("alpha", "default", Speed::Standard)
                .unwrap()
                .multipliers_ppm(),
            (1250000, MULTIPLIER_ONE)
        );
        assert!(live.quote("alpha", "default", Speed::Ultrafast).is_some());
        assert!(live.quote("alpha", "spare", Speed::Fast).is_some());
        assert!(live.quote("alpha", "spare", Speed::Ultrafast).is_none());
    }
    #[test]
    fn official_fractional_price_freezes_hold_and_settlement_across_repricing() {
        let (mut book, routing, mut prices) = fixture();
        let live = snapshot(&book, &routing, &prices);
        let quote = live.quote("alpha", "default", Speed::Fast).unwrap();
        let frozen = quote.freeze_for_price_test("alpha", "default").unwrap();
        let receipt = serde_json::to_vec(&frozen).unwrap();
        let measured_zero = Some(CacheCreationUsage {
            total: 0,
            five_minute: None,
            one_hour: None,
            twelve_hour: Some(0),
            cache_creation_mode: None,
            reported_total: None,
        });
        let usage = Usage {
            cache_creation: measured_zero,
            input: 1000000,
            cached: 200000,
            output: 100000,
            ..Usage::default()
        };
        // 2.25 / .75 / 3.75 credits per million. Ceil(2.625), half-up(2.325).
        assert_eq!(
            frozen
                .reservation_ceiling(ReservationBasis::Tokens {
                    input: 1000000,
                    output: 100000
                })
                .unwrap(),
            3
        );
        assert_eq!(frozen.cost(usage).unwrap(), 2);
        assert_eq!(
            frozen
                .cost(Usage {
                    input: 1,
                    cache_creation: measured_zero,
                    ..Usage::default()
                })
                .unwrap(),
            1
        );
        book.models.get_mut("alpha").unwrap().rates.input = 13;
        prices.version = 8;
        let repriced = snapshot(&book, &routing, &prices)
            .quote("alpha", "default", Speed::Fast)
            .unwrap();
        assert_eq!(repriced.rule_id(), quote.rule_id());
        assert_eq!(
            repriced
                .freeze_for_price_test("alpha", "default")
                .unwrap()
                .cost(usage)
                .unwrap(),
            8
        );
        assert_eq!(
            serde_json::from_slice::<Price>(&receipt)
                .unwrap()
                .cost(usage)
                .unwrap(),
            2
        );
    }
    #[test]
    fn official_cache_price_applies_uniformly_to_both_measured_openai_lifetimes() {
        let (mut book, routing, prices) = fixture();
        book.models.get_mut("alpha").unwrap().rates.cache_write = 7;
        let quote = snapshot(&book, &routing, &prices)
            .quote("alpha", "default", Speed::Fast)
            .unwrap();
        let price = quote.freeze_for_price_test("alpha", "default").unwrap();
        assert_eq!(price.tiered.as_ref().unwrap().tariff.revision, 4);
        for count in [0, 375000, 1000000] {
            let usage = Usage {
                input: 1000000,
                cache_creation: Some(CacheCreationUsage {
                    total: 1000000,
                    five_minute: None,
                    one_hour: None,
                    twelve_hour: Some(count),
                    cache_creation_mode: None,
                    reported_total: None,
                }),
                ..Usage::default()
            };
            assert_eq!(price.cost(usage).unwrap(), 5);
        }
        assert_eq!(
            price
                .reservation_ceiling(ReservationBasis::Tokens {
                    input: 1000000,
                    output: 0
                })
                .unwrap(),
            6
        );
    }
    #[test]
    fn official_measured_free_writes_are_disjoint_from_paid_input() {
        let (book, mut routing, prices) = fixture();
        for (protocol, revision) in [
            (Protocol::OpenAiChat, 4),
            (Protocol::OpenAiResponses, 4),
            (Protocol::AnthropicMessages, 2),
            (Protocol::Gemini, 1),
        ] {
            routing.upstreams[0].protocol = protocol;
            let frozen = snapshot(&book, &routing, &prices)
                .quote("alpha", "default", Speed::Fast)
                .unwrap()
                .freeze_for_price_test("alpha", "default")
                .unwrap();
            let tariff = &frozen.tiered.as_ref().unwrap().tariff;
            assert_eq!(tariff.revision, revision);
            assert_eq!(tariff.uses_cache_write, revision != 1);
            assert_eq!(tariff.tiers[0].rates.cache_write.numerator, 0);
            let measured = match protocol {
                Protocol::OpenAiChat | Protocol::OpenAiResponses => Some(CacheCreationUsage {
                    total: 1_000_000,
                    five_minute: None,
                    one_hour: None,
                    twelve_hour: Some(400_000),
                    cache_creation_mode: None,
                    reported_total: None,
                }),
                Protocol::AnthropicMessages => Some(CacheCreationUsage {
                    total: 1_000_000,
                    five_minute: Some(600_000),
                    one_hour: Some(400_000),
                    twelve_hour: None,
                    cache_creation_mode: None,
                    reported_total: None,
                }),
                _ => None,
            };
            if let Some(creation) = measured {
                let mut frozen = frozen.clone();
                if protocol == Protocol::AnthropicMessages {
                    frozen.tiered.as_mut().unwrap().input_semantics =
                        crate::routing::InputSemantics::AnthropicText;
                }
                let usage = Usage {
                    input: 1_000_000,
                    cache_creation: Some(creation),
                    ..Usage::default()
                };
                assert_eq!(frozen.cost(usage).unwrap(), 0);
                assert_eq!(
                    frozen
                        .cost(Usage {
                            input: 2_000_000,
                            ..usage
                        })
                        .unwrap(),
                    2
                );
                assert!(
                    frozen
                        .cost(Usage {
                            input: 2_000_000,
                            ..Usage::default()
                        })
                        .is_err()
                );
            } else {
                assert_eq!(
                    frozen
                        .cost(Usage {
                            input: 1_000_000,
                            ..Usage::default()
                        })
                        .unwrap(),
                    2
                );
            }
        }
        let mut wire = serde_json::to_value(&book).unwrap();
        wire["models"]["alpha"]["rates"]
            .as_object_mut()
            .unwrap()
            .remove("cache_write");
        assert!(serde_json::from_value::<OfficialPrices>(wire).is_err());
    }
    #[test]
    fn official_explicit_context_tier_and_cache_rates_round_once() {
        let (mut book, routing, prices) = fixture();
        let p = book.models.get_mut("alpha").unwrap();
        let fraction = |n: u64| json!({"numerator":n,"denominator":3});
        let rates = |n: u64| json!({"input":fraction(n),"output":fraction(n),"cache_read":fraction(1),"cache_write":fraction(0)});
        let tariff:TieredTariff=serde_json::from_value(json!({
            "revision":1,"source_digest":"ac".repeat(32),"uses_cache_read":true,"uses_cache_write":false,
            "tiers":[{"up_to_input_tokens":12,"rates":rates(2)},{"up_to_input_tokens":null,"rates":rates(5)}]
        })).unwrap();
        p.rates = TokenRates {
            input: 0,
            output: 0,
            cache_read: 0,
            cache_write: 0,
            request: 0,
        };
        p.billing_mode = BillingMode::TieredDecimal;
        p.tiered = Some(tariff.clone());
        let frozen = snapshot(&book, &routing, &prices)
            .quote("alpha", "default", Speed::Fast)
            .unwrap()
            .freeze_for_price_test("alpha", "default")
            .unwrap();
        assert_eq!(frozen.tiered.as_ref().unwrap().tariff, tariff);
        // At inclusive boundary 12: 5.5; at 13: 14.25.
        assert_eq!(
            frozen
                .cost(Usage {
                    input: 12,
                    cached: 2,
                    ..Usage::default()
                })
                .unwrap(),
            6
        );
        assert_eq!(
            frozen
                .cost(Usage {
                    input: 13,
                    cached: 2,
                    ..Usage::default()
                })
                .unwrap(),
            14
        );
    }
    fn imported_write_tariff(upper_rate: u64) -> TieredTariff {
        let ratio = |numerator| json!({"numerator":numerator,"denominator":3});
        let rates = |write| {
            json!({
                "input":ratio(6),"output":ratio(12),"cache_read":ratio(1),"cache_write":ratio(write)
            })
        };
        serde_json::from_value(json!({
            "revision":2,"source_digest":"bc".repeat(32),"uses_cache_read":true,
            "uses_cache_write":true,"tiers":[
                {"up_to_input_tokens":12,"rates":rates(0)},
                {"up_to_input_tokens":null,"rates":rates(upper_rate)}
            ]
        }))
        .unwrap()
    }

    fn compile_imported_tariff(
        source: &TieredTariff,
        protocol: Protocol,
    ) -> Result<TieredTariff, ManagementError> {
        let (mut book, mut routing, prices) = fixture();
        let base = book.models.get_mut("alpha").unwrap();
        base.billing_mode = BillingMode::TieredDecimal;
        base.rates = TokenRates {
            input: 0,
            output: 0,
            cache_read: 0,
            cache_write: 0,
            request: 0,
        };
        base.tiered = Some(source.clone());
        routing.upstreams[0].protocol = protocol;
        let effective = book.compile(&routing, &prices)?;
        let frozen = PriceSnapshot::compile(effective)
            .unwrap()
            .quote("alpha", "default", Speed::Fast)
            .unwrap()
            .freeze_for_price_test("alpha", "default")
            .unwrap();
        assert_eq!(book.models["alpha"].tiered.as_ref(), Some(source));
        Ok(frozen.tiered.unwrap().tariff)
    }

    #[test]
    fn official_imported_write_preserves_thresholds_source_rates_and_dialect() {
        let source = imported_write_tariff(7);
        for protocol in [
            Protocol::OpenAiChat,
            Protocol::OpenAiResponses,
            Protocol::AnthropicMessages,
        ] {
            // Generic-only input does not document this supplier's longer write lifetime.
            assert!(compile_imported_tariff(&source, protocol).is_err());
            assert!(compile_imported_tariff(&imported_write_tariff(0), protocol).is_err());
        }
        let mut openai = source.clone();
        openai.revision = 4;
        openai.uses_cache_write_12h = true;
        for tier in &mut openai.tiers {
            tier.rates.cache_write_12h = Some(tier.rates.cache_write);
        }
        for protocol in [Protocol::OpenAiChat, Protocol::OpenAiResponses] {
            let projected = compile_imported_tariff(&openai, protocol).unwrap();
            assert_eq!(projected, openai);
            let compiled = projected.compile().unwrap();
            for (input, credits) in [(12, 0), (13, 15)] {
                for twelve in [0, 7, input] {
                    let usage = crate::routing::TieredUsage {
                        input,
                        created: input,
                        created_12h: Some(twelve),
                        ..Default::default()
                    };
                    assert_eq!(
                        compiled
                            .charge(
                                usage,
                                crate::routing::InputSemantics::Inclusive,
                                MULTIPLIER_ONE,
                                MULTIPLIER_ONE,
                                500_000
                            )
                            .unwrap(),
                        credits
                    );
                }
            }
        }
        // Known zero 12h can coexist with a different, paid 30m rate in the same band.
        let mut free_twelve = openai.clone();
        free_twelve.tiers[1].rates.cache_write_12h = Some(DecimalRate {
            numerator: 0,
            denominator: 3,
        });
        let frozen = compile_imported_tariff(&free_twelve, Protocol::OpenAiResponses).unwrap();
        for (twelve, expected) in [(0, 15), (13, 0)] {
            let usage = crate::routing::TieredUsage {
                input: 13,
                created: 13,
                created_12h: Some(twelve),
                ..Default::default()
            };
            assert_eq!(
                frozen
                    .compile()
                    .unwrap()
                    .charge(
                        usage,
                        crate::routing::InputSemantics::Inclusive,
                        MULTIPLIER_ONE,
                        MULTIPLIER_ONE,
                        500_000
                    )
                    .unwrap(),
                expected
            );
        }
        assert!(compile_imported_tariff(&source, Protocol::Gemini).is_err());
        let free = imported_write_tariff(0);
        let mut gemini = free.clone();
        gemini.revision = 1;
        gemini.uses_cache_write = false;
        assert_eq!(
            compile_imported_tariff(&free, Protocol::Gemini).unwrap(),
            gemini
        );
        let mut specific = source;
        specific.uses_cache_write_1h = true;
        for tier in &mut specific.tiers {
            tier.rates.cache_write_1h = Some(DecimalRate {
                numerator: 0,
                denominator: 3,
            });
        }
        specific.tiers[1].rates.cache_write_1h = Some(DecimalRate {
            numerator: 19,
            denominator: 3,
        });
        for protocol in [
            Protocol::OpenAiChat,
            Protocol::OpenAiResponses,
            Protocol::Gemini,
        ] {
            assert!(compile_imported_tariff(&specific, protocol).is_err());
        }
        let frozen = compile_imported_tariff(&specific, Protocol::AnthropicMessages).unwrap();
        assert_eq!(frozen, specific);
        for (input, five, hour, expected) in [(12, 0, 12, 0), (13, 13, 0, 15), (13, 0, 13, 41)] {
            let usage = crate::routing::TieredUsage {
                input,
                created: input,
                created_5m: Some(five),
                created_1h: Some(hour),
                ..Default::default()
            };
            assert_eq!(
                frozen
                    .compile()
                    .unwrap()
                    .charge(
                        usage,
                        crate::routing::InputSemantics::AnthropicText,
                        MULTIPLIER_ONE,
                        MULTIPLIER_ONE,
                        500_000
                    )
                    .unwrap(),
                expected
            );
        }
        assert!(compile_imported_tariff(&openai, Protocol::AnthropicMessages).is_err());
        assert!(compile_imported_tariff(&openai, Protocol::Gemini).is_err());
    }
    #[test]
    fn official_imported_calendar_and_legacy_dialects_stay_explicit() {
        use crate::routing::PayloadPolicy;
        let coverage = |protocol, payload| {
            let mut routes = Coverage::default();
            routes.add(protocol, payload);
            routes
        };
        let openai = coverage(Protocol::OpenAiResponses, PayloadPolicy::Adapt);
        let anthropic = coverage(Protocol::AnthropicMessages, PayloadPolicy::Adapt);
        let gemini = coverage(Protocol::Gemini, PayloadPolicy::Adapt);
        let legacy = coverage(
            Protocol::OpenAiChat,
            PayloadPolicy::LegacyOpenRouterChatUsageV1,
        );

        let mut calendar = imported_write_tariff(0);
        calendar.uses_cache_write_1h = true;
        for tier in &mut calendar.tiers {
            tier.rates.cache_write_1h = Some(tier.rates.cache_write);
        }
        let expression = "official source with dated prices";
        calendar.source_digest = crate::hex_codec::encode(Sha256::digest(expression));
        calendar.calendar = Some(serde_json::from_value(json!({
            "revision":1,"source_expression":expression,"weekday_timezone":"UTC","weekdays":[1],
            "hour_timezone":"Asia/Shanghai","hour_ranges":[{"start":1,"end":2}],
            "excluded_date_timezone":"Asia/Shanghai","excluded_dates":[],"match_tiers":calendar.tiers
        })).unwrap());
        let unchanged = calendar.clone();
        assert!(project_official_tiered(&mut calendar, &openai).is_err());
        assert!(project_official_tiered(&mut calendar, &gemini).is_err());
        project_official_tiered(&mut calendar, &anthropic).unwrap();
        assert_eq!(calendar, unchanged);

        let mut mixed = coverage(Protocol::OpenAiChat, PayloadPolicy::Adapt);
        mixed.add(Protocol::Gemini, PayloadPolicy::Adapt);
        assert!(project_official_tiered(&mut imported_write_tariff(0), &mixed).is_err());
        let mut source = imported_write_tariff(0);
        source.uses_cache_read = false;
        for tier in &mut source.tiers {
            tier.rates.cache_read.numerator = 0;
        }
        assert!(project_official_tiered(&mut source, &openai).is_err());
        assert!(project_official_tiered(&mut source, &gemini).is_err());

        let mut generic_remainder = imported_write_tariff(7);
        generic_remainder.revision = 3;
        generic_remainder.tiers.remove(0);
        generic_remainder.uses_cache_write_1h = true;
        generic_remainder.tiers[0].rates.cache_write_1h = Some(DecimalRate {
            numerator: 19,
            denominator: 3,
        });
        assert!(generic_remainder.compile().is_ok());
        let original = generic_remainder.clone();
        project_official_tiered(&mut generic_remainder, &legacy).unwrap();
        assert_eq!(generic_remainder, original);
        assert!(project_official_tiered(&mut generic_remainder, &openai).is_err());
        assert!(project_official_tiered(&mut imported_write_tariff(0), &legacy).is_err());
        let (book, mut routes, prices) = fixture();
        routes.upstreams[0].protocol = Protocol::OpenAiChat;
        routes.routes[0].targets[0].payload = PayloadPolicy::LegacyOpenRouterChatUsageV1;
        assert!(book.compile(&routes, &prices).is_err());
    }
    #[test]
    fn official_scalar_uses_one_price_rule_for_mixed_text_targets() {
        let (mut book, mut routing, prior) = fixture();
        let base_upstream = routing.upstreams[0].clone();
        let base_target = routing.routes[0].targets[0].clone();
        for (index, protocol) in [
            Protocol::OpenAiChat,
            Protocol::AnthropicMessages,
            Protocol::Gemini,
        ]
        .into_iter()
        .enumerate()
        {
            let id = index as u64 + 100;
            let mut upstream = base_upstream.clone();
            upstream.id = id;
            upstream.protocol = protocol;
            routing.upstreams.push(upstream);
            let mut target = base_target.clone();
            target.id = id;
            target.upstream_id = id;
            routing.routes[0].targets.push(target);
        }
        let release = book.compile(&routing, &prior).unwrap();
        assert_eq!(release, book.compile(&routing, &release).unwrap());
        assert_eq!(
            release
                .rules
                .iter()
                .filter(|r| r.model == "alpha" && r.group == "default" && r.speed == Speed::Fast)
                .count(),
            1
        );
        let live = snapshot(&book, &routing, &prior);
        let template = live.template("alpha", "default", Speed::Fast).unwrap();
        assert_eq!(template.billing_mode(), BillingMode::OfficialFlat);
        let mut receipts = Vec::new();
        for (protocol, revision) in [
            (Protocol::OpenAiChat, 4),
            (Protocol::OpenAiResponses, 4),
            (Protocol::AnthropicMessages, 2),
            (Protocol::Gemini, 1),
        ] {
            let quote = template
                .for_target(protocol, crate::routing::PayloadPolicy::Adapt)
                .unwrap();
            assert_eq!(quote.rule_id(), template.rule_id());
            assert_eq!(quote.version(), template.version());
            assert_eq!(quote.multipliers_ppm(), (1_250_000, 1_200_000));
            let frozen = quote.freeze_for_price_test("alpha", "default").unwrap();
            let price = frozen.tiered.as_ref().unwrap();
            assert_eq!(price.tariff.revision, revision);
            let created = match revision {
                4 => Some(CacheCreationUsage {
                    total: 500_000,
                    twelve_hour: Some(200_000),
                    five_minute: None,
                    one_hour: None,
                    cache_creation_mode: None,
                    reported_total: None,
                }),
                2 => Some(CacheCreationUsage {
                    total: 500_000,
                    twelve_hour: None,
                    five_minute: Some(300_000),
                    one_hour: Some(200_000),
                    cache_creation_mode: None,
                    reported_total: None,
                }),
                _ => None,
            };
            let usage = Usage {
                input: if created.is_some() {
                    1_500_000
                } else {
                    1_000_000
                },
                cached: 200_000,
                output: 100_000,
                cache_creation: created,
                ..Default::default()
            };
            let amount = frozen.cost(usage).unwrap();
            assert_eq!(amount, 2);
            let witness = release
                .rules
                .iter()
                .find(|r| r.model == "alpha" && r.group == "default" && r.speed == Speed::Fast)
                .unwrap()
                .tiered
                .as_ref()
                .unwrap();
            assert_eq!(price.tariff.source_digest, witness.source_digest);
            receipts.push((serde_json::to_vec(&frozen).unwrap(), usage, amount));
        }
        book.models.get_mut("alpha").unwrap().rates.input = 13;
        let changed = snapshot(&book, &routing, &prior)
            .template("alpha", "default", Speed::Fast)
            .unwrap();
        for ((bytes, usage, amount), protocol) in receipts.into_iter().zip([
            Protocol::OpenAiChat,
            Protocol::OpenAiResponses,
            Protocol::AnthropicMessages,
            Protocol::Gemini,
        ]) {
            let old: Price = serde_json::from_slice(&bytes).unwrap();
            assert_eq!(old.cost(usage).unwrap(), amount);
            let repriced = changed
                .for_target(protocol, crate::routing::PayloadPolicy::Adapt)
                .unwrap()
                .freeze_for_price_test("alpha", "default")
                .unwrap();
            assert_eq!(repriced.cost(usage).unwrap(), 8);
        }
        // A real write rate must remain available on the OpenAI and Anthropic targets.
        // Gemini's wire carries no writes; selecting it cannot change the source.
        let scalar = book.models.get_mut("alpha").unwrap();
        scalar.rates.input = 3;
        scalar.rates.cache_write = 7;
        let paid = snapshot(&book, &routing, &prior)
            .template("alpha", "default", Speed::Fast)
            .unwrap();
        for (protocol, rate) in [
            (Protocol::OpenAiChat, 7),
            (Protocol::OpenAiResponses, 7),
            (Protocol::AnthropicMessages, 7),
            (Protocol::Gemini, 0),
        ] {
            let actual = paid
                .for_target(protocol, crate::routing::PayloadPolicy::Adapt)
                .unwrap()
                .tiered()
                .unwrap()
                .tariff();
            assert_eq!(actual.tiers[0].rates.cache_write.numerator, rate);
        }
        assert_eq!(book.models["alpha"].rates.cache_write, 7);
        let mut gemini_only = routing;
        let gemini_id = gemini_only
            .upstreams
            .iter()
            .find(|u| u.protocol == Protocol::Gemini)
            .unwrap()
            .id;
        gemini_only.routes[0]
            .targets
            .retain(|t| t.upstream_id == gemini_id);
        match book.compile(&gemini_only, &prior) {
            Err(ManagementError::OfficialPriceIncompatible {
                model,
                group,
                speed,
                reason,
            }) => {
                assert_eq!(
                    (model.as_str(), group.as_str(), speed),
                    ("alpha", "default", Speed::Fast)
                );
                assert!(reason.contains("Gemini"));
            }
            _ => panic!("a Gemini-only route must explain its unsupported positive write rate"),
        }
        let source = book.models.get_mut("alpha").unwrap();
        source.billing_mode = BillingMode::OfficialFlat;
        assert!(book.validate().is_err());
    }
    #[test]
    fn official_missing_group_defaults_to_one_and_invalid_prices_fail_closed() {
        let (mut book, routing, prices) = fixture();
        book.group_multipliers_ppm.remove("spare");
        let compiled = book.compile(&routing, &prices).unwrap();
        assert!(compiled.rules.iter().any(|rule| {
            rule.model == "alpha"
                && rule.group == "spare"
                && rule.group_multiplier_ppm == MULTIPLIER_ONE
        }));
        book.group_multipliers_ppm.insert("spare".into(), 0);
        book.models.get_mut("alpha").unwrap().rates.request = 1;
        assert!(book.compile(&routing, &prices).is_err());
        book.models.get_mut("alpha").unwrap().rates.request = 0;
        let mut full = prices.clone();
        full.rules[0].id = i64::MAX as u64;
        assert!(book.compile(&routing, &full).is_err());
        book.models.get_mut("alpha").unwrap().source.clear();
        assert!(book.compile(&routing, &prices).is_err());
    }
}

use std::collections::{BTreeMap, HashMap, HashSet};

use serde::{Deserialize, Serialize};

use super::config::{MAX_ROUTES, valid_group, valid_id, valid_model};
use super::{RoutingError, Speed};

pub const MULTIPLIER_ONE: u32 = 1_000_000;
pub const MAX_MULTIPLIER: u32 = 1_000_000_000;
const PER_MILLION: u128 = 1_000_000;
const DENOMINATOR: u128 = PER_MILLION * MULTIPLIER_ONE as u128 * MULTIPLIER_ONE as u128;

#[derive(Clone, Copy, Debug, Default, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum BillingMode {
    #[default]
    Tokens,
    UpstreamCost,
    TieredDecimal,
}
impl BillingMode {
    fn is_tokens(&self) -> bool {
        *self == Self::Tokens
    }
}

/// Explicit owner-selected compatibility for imported ordinary Go ratio prices.
/// Absence keeps the native Core ceiling, including previously frozen receipts.
#[derive(Clone, Copy, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum TokenRounding {
    #[serde(rename = "legacy_go_ratio_half_away_min1_v1")]
    LegacyGoRatioV1,
}

/// All token rates are integer micro-USD per 1,000,000 tokens.
#[derive(Clone, Copy, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct TokenRates {
    pub input: u64,
    pub output: u64,
    pub cache_read: u64,
    pub cache_write: u64,
    /// Integer micro-USD per logical request, not per attempt.
    pub request: u64,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct PriceRule {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub rounding: Option<TokenRounding>,
    #[serde(default, skip_serializing_if = "BillingMode::is_tokens")]
    pub billing_mode: BillingMode,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tiered: Option<super::TieredTariff>,
    pub id: u64,
    pub model: String,
    pub group: String,
    pub speed: Speed,
    pub rates: TokenRates,
    pub group_multiplier_ppm: u32,
    pub speed_multiplier_ppm: u32,
}

/// Independently versioned prices. A route release pins exactly one price version.
#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct PriceConfig {
    pub version: u64,
    pub rules: Vec<PriceRule>,
}

/// Disjoint counters. Input MUST exclude both cache_read and cache_write tokens.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Usage {
    pub input: u64,
    pub output: u64,
    pub cache_read: u64,
    pub cache_write: u64,
}

impl Usage {
    /// Normalize an authoritative total into the disjoint pricing counters.
    /// Callers must resolve missing counters under a versioned provider contract;
    /// an absent total is not zero. Cache reads and writes are disjoint subsets.
    pub fn from_totals(
        input: u64,
        output: u64,
        cache_read: u64,
        cache_write: u64,
    ) -> Result<Self, RoutingError> {
        let input = input
            .checked_sub(cache_read)
            .and_then(|value| value.checked_sub(cache_write))
            .ok_or(RoutingError::InvalidRequest)?;
        Ok(Self {
            input,
            output,
            cache_read,
            cache_write,
        })
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct MicroUsd(pub u64);

/// Integer ledger units. Producing a quote does not reserve or settle funds.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Credits(pub u64);

/// Server-owned conversion, captured with the price before reservation.
/// The revision must change when the conversion changes. This is not HTTP input.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct CreditRate {
    version: u64,
    credits_per_usd: u64,
}

impl CreditRate {
    pub fn new(version: u64, credits_per_usd: u64) -> Result<Self, RoutingError> {
        if !valid_id(version) || !valid_id(credits_per_usd) {
            return Err(RoutingError::InvalidConfig("credit conversion"));
        }
        Ok(Self {
            version,
            credits_per_usd,
        })
    }

    pub fn version(&self) -> u64 {
        self.version
    }

    pub fn credits_per_usd(&self) -> u64 {
        self.credits_per_usd
    }
}

/// One immutable price and conversion for the entire logical request.
/// Persist both with the original operation; never obtain a new quote on retry.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct CreditQuote {
    price: PriceQuote,
    conversion: CreditRate,
    discount: Option<crate::billing::trust_pricing::DiscountRatio>,
}

impl CreditQuote {
    /// Numeric projection of the authority already frozen by Core admission.
    /// This method does not authenticate a user or create spending authority.
    pub(crate) fn with_discount(
        mut self,
        discount: Option<crate::billing::trust_pricing::DiscountRatio>,
    ) -> Self {
        self.discount = discount;
        self
    }
    pub fn price(&self) -> PriceQuote {
        self.price
    }

    pub fn conversion(&self) -> CreditRate {
        self.conversion
    }

    /// Upper bound for every disjoint input/cache split within these ceilings.
    /// Uses this captured quote, including its request fee and conversion. This
    /// calculates an amount only; the charging transaction owns the actual hold.
    pub fn reservation_ceiling(
        &self,
        input_tokens: u64,
        output_tokens: u64,
    ) -> Result<Credits, RoutingError> {
        if let Some(tariff) = self.price.tiered {
            return tariff
                .reservation_ceiling_with_discount(
                    input_tokens,
                    output_tokens,
                    self.price.group_multiplier_ppm,
                    self.price.speed_multiplier_ppm,
                    self.conversion.credits_per_usd,
                    self.discount,
                )
                .map(Credits);
        }
        let mut ceiling = *self;
        // Compatibility affects final settlement only, never the safety hold.
        ceiling.price.rounding = None;
        ceiling.price.rates.input = self
            .price
            .rates
            .input
            .max(self.price.rates.cache_read)
            .max(self.price.rates.cache_write);
        let amount = ceiling.charge(Usage {
            input: input_tokens,
            output: output_tokens,
            ..Usage::default()
        })?;
        // A nonzero legacy ratio can bill min1 for an otherwise free measured
        // dimension (e.g. zero-priced output). Bound that final minimum too.
        Ok(
            if self.price.rounding.is_some()
                && self.price.rates.input > 0
                && self.price.group_multiplier_ppm > 0
                && (input_tokens > 0 || output_tokens > 0)
            {
                Credits(amount.0.max(1))
            } else {
                amount
            },
        )
    }

    /// Sum all dimensions and multipliers, then round up once in ledger units.
    /// Do not round the token rates or the intermediate micro-USD amount.
    pub fn charge(&self, usage: Usage) -> Result<Credits, RoutingError> {
        self.charge_with_semantics(usage, super::InputSemantics::Inclusive)
    }

    pub fn charge_with_semantics(
        &self,
        usage: Usage,
        semantics: super::InputSemantics,
    ) -> Result<Credits, RoutingError> {
        if self.price.rounding.is_some() && usage.cache_write != 0 {
            return Err(RoutingError::InvalidRequest);
        }
        if let Some(tariff) = self.price.tiered {
            return tariff
                .charge_with_discount(
                    super::TieredUsage {
                        input: usage
                            .input
                            .checked_add(usage.cache_read)
                            .and_then(|v| v.checked_add(usage.cache_write))
                            .ok_or(RoutingError::PriceOverflow)?,
                        cached: usage.cache_read,
                        created: usage.cache_write,
                        output: usage.output,
                        ..super::TieredUsage::default()
                    },
                    semantics,
                    self.price.group_multiplier_ppm,
                    self.price.speed_multiplier_ppm,
                    self.conversion.credits_per_usd,
                    self.discount,
                )
                .map(Credits);
        }
        let mut factor = u128::from(self.conversion.credits_per_usd);
        let mut divisor = PER_MILLION;
        // Reduce the conversion before multiplying. For 500000 credits/USD this
        // is exactly 1/2, including fractional per-million token prices.
        let (mut a, mut b) = (factor, divisor);
        while b != 0 {
            (a, b) = (b, a % b);
        }
        factor /= a;
        divisor /= a;
        let numerator = self
            .price
            .numerator(usage)?
            .checked_mul(factor)
            .ok_or(RoutingError::PriceOverflow)?;
        let denominator = DENOMINATOR
            .checked_mul(divisor)
            .ok_or(RoutingError::PriceOverflow)?;
        let amount = match self.price.rounding {
            None => crate::upstream_cost::ceil_with_discount(numerator, denominator, self.discount),
            Some(TokenRounding::LegacyGoRatioV1) => {
                crate::upstream_cost::half_away_with_discount(numerator, denominator, self.discount)
                    .map(|amount| {
                        // Old Go applies min1 even when all measured input is free cache.
                        if self.price.rates.input > 0
                            && self.price.group_multiplier_ppm > 0
                            && (usage.input > 0 || usage.cache_read > 0 || usage.output > 0)
                        {
                            amount.max(1)
                        } else {
                            amount
                        }
                    })
            }
        }
        .filter(|amount| *amount <= i64::MAX as u64)
        .ok_or(RoutingError::PriceOverflow)?;
        Ok(Credits(amount))
    }

    /// Evaluate a revisioned tariff with its authoritative cache TTL evidence.
    /// The scalar token adapter cannot supply a missing TTL measurement.
    pub fn charge_tiered_usage(
        &self,
        usage: super::TieredUsage,
        semantics: super::InputSemantics,
    ) -> Result<Credits, RoutingError> {
        self.price
            .tiered
            .ok_or(RoutingError::InvalidRequest)?
            .charge_with_discount(
                usage,
                semantics,
                self.price.group_multiplier_ppm,
                self.price.speed_multiplier_ppm,
                self.conversion.credits_per_usd,
                self.discount,
            )
            .map(Credits)
    }
}

fn rounded_amount(numerator: u128, denominator: u128) -> Result<u64, RoutingError> {
    let amount = numerator.div_ceil(denominator);
    if amount > i64::MAX as u128 {
        return Err(RoutingError::PriceOverflow);
    }
    Ok(amount as u64)
}

/// Immutable request-local price evidence. No floating point or database lookup.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct PriceQuote {
    rounding: Option<TokenRounding>,
    billing_mode: BillingMode,
    tiered: Option<super::CompiledTariff>,
    version: u64,
    rule_id: u64,
    rates: TokenRates,
    group_multiplier_ppm: u32,
    speed_multiplier_ppm: u32,
}

impl PriceQuote {
    pub(crate) fn rounding(&self) -> Option<TokenRounding> {
        self.rounding
    }
    /// Native input-only protocols cannot infer zero rates from the integer
    /// placeholders carried beside a decimal tariff.
    pub fn is_input_only_scalar(&self) -> bool {
        match (self.billing_mode, self.tiered) {
            (BillingMode::Tokens, None) => {
                self.rates.output == 0
                    && self.rates.cache_read == 0
                    && self.rates.cache_write == 0
                    && self.rates.request == 0
            }
            (BillingMode::TieredDecimal, Some(compiled)) => {
                let tariff = compiled.tariff();
                tariff.revision == 1
                    && !tariff.uses_cache_read
                    && !tariff.uses_cache_write
                    && !tariff.uses_cache_write_1h
                    && !tariff.uses_cache_write_12h
                    && tariff.tiers.len() == 1
                    && tariff.tiers[0].up_to_input_tokens.is_none()
                    && tariff.tiers[0].rates.output.numerator == 0
                    && tariff.tiers[0].rates.cache_read.numerator == 0
                    && tariff.tiers[0].rates.cache_write.numerator == 0
            }
            _ => false,
        }
    }
    pub fn tiered(&self) -> Option<super::CompiledTariff> {
        self.tiered
    }
    pub fn billing_mode(&self) -> BillingMode {
        self.billing_mode
    }
    pub fn version(&self) -> u64 {
        self.version
    }

    pub fn rule_id(&self) -> u64 {
        self.rule_id
    }

    pub fn rates(&self) -> TokenRates {
        self.rates
    }

    pub fn multipliers_ppm(&self) -> (u32, u32) {
        (self.group_multiplier_ppm, self.speed_multiplier_ppm)
    }

    /// Round UP once, after summing all dimensions and both multipliers.
    /// Task 03 must apply this once to a logical request; retries are not new fees.
    pub fn charge(&self, usage: Usage) -> Result<MicroUsd, RoutingError> {
        if let Some(tariff) = self.tiered {
            return tariff
                .charge(
                    super::TieredUsage {
                        input: usage
                            .input
                            .checked_add(usage.cache_read)
                            .and_then(|v| v.checked_add(usage.cache_write))
                            .ok_or(RoutingError::PriceOverflow)?,
                        cached: usage.cache_read,
                        created: usage.cache_write,
                        output: usage.output,
                        ..super::TieredUsage::default()
                    },
                    super::InputSemantics::Inclusive,
                    self.group_multiplier_ppm,
                    self.speed_multiplier_ppm,
                    1_000_000,
                )
                .map(MicroUsd);
        }
        Ok(MicroUsd(rounded_amount(
            self.numerator(usage)?,
            DENOMINATOR,
        )?))
    }

    /// Capture an already validated server conversion without a new price lookup.
    pub fn with_credit_rate(self, conversion: CreditRate) -> CreditQuote {
        CreditQuote {
            price: self,
            conversion,
            discount: None,
        }
    }

    fn numerator(&self, usage: Usage) -> Result<u128, RoutingError> {
        if self.billing_mode == BillingMode::UpstreamCost {
            return Err(RoutingError::InvalidConfig(
                "upstream cost cannot be inferred from token counts",
            ));
        }
        let mut numerator = u128::from(self.rates.request) * PER_MILLION;
        for (count, rate) in [
            (usage.input, self.rates.input),
            (usage.output, self.rates.output),
            (usage.cache_read, self.rates.cache_read),
            (usage.cache_write, self.rates.cache_write),
        ] {
            numerator = numerator
                .checked_add(u128::from(count) * u128::from(rate))
                .ok_or(RoutingError::PriceOverflow)?;
        }
        numerator = numerator
            .checked_mul(u128::from(self.group_multiplier_ppm))
            .and_then(|n| n.checked_mul(u128::from(self.speed_multiplier_ppm)))
            .ok_or(RoutingError::PriceOverflow)?;
        Ok(numerator)
    }
}

type ModelPrices = HashMap<String, BTreeMap<Speed, PriceQuote>>;

#[derive(Clone, Debug)]
pub struct PriceSnapshot {
    config: PriceConfig,
    groups: HashMap<String, ModelPrices>,
    calendars: HashMap<u64, super::calendar::CompiledCalendar>,
}

impl PriceSnapshot {
    pub fn compile(config: PriceConfig) -> Result<Self, RoutingError> {
        if !valid_id(config.version) || config.rules.len() > MAX_ROUTES * 3 {
            return Err(RoutingError::InvalidConfig("price version or size"));
        }
        let mut ids = HashSet::new();
        let mut calendars = HashMap::new();
        let mut groups: HashMap<String, ModelPrices> = HashMap::new();
        for rule in &config.rules {
            if !valid_id(rule.id)
                || !ids.insert(rule.id)
                || !valid_group(&rule.group)
                || !valid_model(&rule.model)
                || rule.group_multiplier_ppm > MAX_MULTIPLIER
                || !(1..=MAX_MULTIPLIER).contains(&rule.speed_multiplier_ppm)
                || [
                    rule.rates.input,
                    rule.rates.output,
                    rule.rates.cache_read,
                    rule.rates.cache_write,
                    rule.rates.request,
                ]
                .iter()
                .any(|&rate| rate > i64::MAX as u64)
            {
                return Err(RoutingError::InvalidConfig("price rule"));
            }
            if rule.rounding.is_some()
                && (rule.billing_mode != BillingMode::Tokens
                    || rule.tiered.is_some()
                    || rule.rates.cache_write != 0
                    || rule.rates.request != 0
                    || (rule.rates.input == 0
                        && (rule.rates.output != 0 || rule.rates.cache_read != 0)))
            {
                return Err(RoutingError::InvalidConfig("legacy ratio rounding"));
            }
            let tiered = match (&rule.tiered, rule.billing_mode) {
                (Some(tariff), BillingMode::TieredDecimal)
                    if [
                        rule.rates.input,
                        rule.rates.output,
                        rule.rates.cache_read,
                        rule.rates.cache_write,
                        rule.rates.request,
                    ]
                    .iter()
                    .all(|v| *v == 0) =>
                {
                    if let Some(calendar) = &tariff.calendar {
                        calendars.insert(rule.id, calendar.compile(tariff)?);
                        let mut base = tariff.clone();
                        base.calendar = None;
                        Some(base.compile()?)
                    } else {
                        Some(tariff.compile()?)
                    }
                }
                (None, BillingMode::Tokens | BillingMode::UpstreamCost) => None,
                _ => return Err(RoutingError::InvalidConfig("tiered token price")),
            };
            if rule.billing_mode == BillingMode::UpstreamCost
                && (!crate::upstream_cost::supported_router(&rule.model)
                    || rule.group_multiplier_ppm == 0
                    || rule.speed != Speed::Standard
                    || rule.speed_multiplier_ppm != MULTIPLIER_ONE
                    || [
                        rule.rates.input,
                        rule.rates.output,
                        rule.rates.cache_read,
                        rule.rates.cache_write,
                        rule.rates.request,
                    ]
                    .iter()
                    .any(|v| *v != 0))
            {
                return Err(RoutingError::InvalidConfig("upstream-cost router price"));
            }
            let quote = PriceQuote {
                rounding: rule.rounding,
                billing_mode: rule.billing_mode,
                tiered,
                version: config.version,
                rule_id: rule.id,
                rates: rule.rates,
                group_multiplier_ppm: rule.group_multiplier_ppm,
                speed_multiplier_ppm: rule.speed_multiplier_ppm,
            };
            if groups
                .entry(rule.group.clone())
                .or_default()
                .entry(rule.model.clone())
                .or_default()
                .insert(rule.speed, quote)
                .is_some()
            {
                return Err(RoutingError::InvalidConfig("duplicate price rule"));
            }
        }
        Ok(Self {
            config,
            groups,
            calendars,
        })
    }

    pub fn version(&self) -> u64 {
        self.config.version
    }

    pub fn quote(&self, model: &str, group: &str, speed: Speed) -> Option<PriceQuote> {
        let quote = self.template(model, group, speed)?;
        // Callers without an admission timestamp cannot choose a calendar price.
        (!self.calendars.contains_key(&quote.rule_id)).then_some(quote)
    }

    pub(super) fn template(&self, model: &str, group: &str, speed: Speed) -> Option<PriceQuote> {
        self.groups.get(group)?.get(model)?.get(&speed).copied()
    }

    pub fn quote_at(
        &self,
        model: &str,
        group: &str,
        speed: Speed,
        timestamp: i64,
    ) -> Result<Option<(PriceQuote, Option<super::FrozenCalendar>)>, RoutingError> {
        let Some(mut quote) = self.template(model, group, speed) else {
            return Ok(None);
        };
        let calendar = if let Some(calendar) = self.calendars.get(&quote.rule_id) {
            let (tariff, frozen) = calendar.select(timestamp)?;
            quote.tiered = Some(tariff);
            Some(frozen)
        } else {
            None
        };
        Ok(Some((quote, calendar)))
    }

    pub fn config(&self) -> &PriceConfig {
        &self.config
    }

    pub(super) fn same_content(&self, other: &Self) -> bool {
        self.groups == other.groups && self.calendars == other.calendars
    }
}

#[cfg(test)]
mod upstream_cost_tests {
    use super::*;
    #[test]
    fn upstream_cost_rules_reject_fixed_rates_unknown_models_and_speed_fees() {
        let mut rule = PriceRule {
            rounding: None,
            billing_mode: BillingMode::UpstreamCost,
            tiered: None,
            id: 1,
            model: "openrouter/fusion".into(),
            group: "default".into(),
            speed: Speed::Standard,
            rates: TokenRates {
                input: 0,
                output: 0,
                cache_read: 0,
                cache_write: 0,
                request: 0,
            },
            group_multiplier_ppm: 1_250_000,
            speed_multiplier_ppm: MULTIPLIER_ONE,
        };
        let compile = |rule: &PriceRule| {
            PriceSnapshot::compile(PriceConfig {
                version: 1,
                rules: vec![rule.clone()],
            })
        };
        let prices = compile(&rule).unwrap();
        let quote = prices.quote(&rule.model, &rule.group, rule.speed).unwrap();
        assert_eq!(quote.billing_mode(), BillingMode::UpstreamCost);
        assert!(quote.charge(Usage::default()).is_err());
        rule.group_multiplier_ppm = 0;
        assert!(compile(&rule).is_err());
        rule.group_multiplier_ppm = 1_250_000;
        rule.rates.request = 1;
        assert!(compile(&rule).is_err());
        rule.rates.request = 0;
        rule.speed = Speed::Fast;
        rule.speed = Speed::Standard;
        rule.model = "arbitrary-model".into();
        assert!(compile(&rule).is_err());
    }
}

#[cfg(test)]
mod explicit_free_group_tests {
    use super::*;

    #[test]
    fn explicitly_free_tokens_keep_zero_quote_but_missing_prices_stay_missing() {
        let prices = PriceSnapshot::compile(PriceConfig {
            version: 1,
            rules: vec![PriceRule {
                rounding: None,
                billing_mode: BillingMode::Tokens,
                tiered: None,
                id: 1,
                model: "configured-model".into(),
                group: "FREE".into(),
                speed: Speed::Standard,
                rates: TokenRates {
                    input: 2_000_000,
                    output: 4_000_000,
                    cache_read: 500_000,
                    cache_write: 0,
                    request: 0,
                },
                group_multiplier_ppm: 0,
                speed_multiplier_ppm: MULTIPLIER_ONE,
            }],
        })
        .unwrap();
        let quote = prices
            .quote("configured-model", "FREE", Speed::Standard)
            .unwrap()
            .with_credit_rate(CreditRate::new(1, 500_000).unwrap());
        assert_eq!(quote.reservation_ceiling(100, 20).unwrap(), Credits(0));
        assert_eq!(
            quote
                .charge(Usage {
                    input: 8,
                    cache_read: 2,
                    output: 20,
                    cache_write: 0
                })
                .unwrap(),
            Credits(0)
        );
        assert!(
            prices
                .quote("unconfigured-model", "FREE", Speed::Standard)
                .is_none()
        );
        assert!(
            prices
                .quote("configured-model", "unconfigured-group", Speed::Standard)
                .is_none()
        );
    }
}

#[cfg(test)]
mod a07_tests {
    use super::*;

    fn quote(rates: TokenRates) -> PriceQuote {
        PriceQuote {
            rounding: None,
            billing_mode: BillingMode::Tokens,
            tiered: None,
            version: 3,
            rule_id: 7,
            rates,
            group_multiplier_ppm: MULTIPLIER_ONE,
            speed_multiplier_ppm: MULTIPLIER_ONE,
        }
    }

    fn rates(input: u64) -> TokenRates {
        TokenRates {
            input,
            output: 0,
            cache_read: 0,
            cache_write: 0,
            request: 0,
        }
    }

    fn credits(price: PriceQuote) -> CreditQuote {
        price.with_credit_rate(CreditRate::new(9, 500_000).unwrap())
    }

    #[test]
    fn fractional_credit_token_rates_are_not_rounded_before_usage() {
        let frozen = credits(quote(rates(1)));
        for (input, expected) in [(0, 0), (1, 1), (1_000_000, 1), (2_000_000, 1)] {
            assert_eq!(
                frozen.charge(Usage {
                    input,
                    ..Usage::default()
                }),
                Ok(Credits(expected))
            );
        }
    }

    #[test]
    fn frozen_user_factor_precedes_quote_and_hold_rounding() {
        let base = credits(quote(rates(20_000)));
        let usage = Usage {
            input: 101,
            ..Usage::default()
        };
        assert_eq!(base.charge(usage), Ok(Credits(2)));
        let frozen = base.with_discount(Some(
            crate::billing::trust_pricing::DiscountRatio::new(9, 10).unwrap(),
        ));
        assert_eq!(frozen.charge(usage), Ok(Credits(1)));
        assert_eq!(frozen.reservation_ceiling(101, 0), Ok(Credits(1)));
        assert_eq!(base.with_discount(None), base);
    }

    #[test]
    fn all_dimensions_and_multipliers_are_summed_before_rounding() {
        let mut price = quote(TokenRates {
            input: 1,
            output: 3,
            cache_read: 2,
            cache_write: 5,
            request: 1,
        });
        price.group_multiplier_ppm = 1_250_000;
        price.speed_multiplier_ppm = 800_000;
        let usage = Usage {
            input: 1_000_000,
            output: 2_000_000,
            cache_read: 3_000_000,
            cache_write: 4_000_000,
        };
        assert_eq!(price.charge(usage), Ok(MicroUsd(34)));
        assert_eq!(credits(price).charge(usage), Ok(Credits(17)));
    }

    #[test]
    fn cache_categories_are_not_double_charged_as_uncached_input() {
        let usage = Usage::from_totals(10, 2, 4, 3).unwrap();
        assert_eq!(
            usage,
            Usage {
                input: 3,
                output: 2,
                cache_read: 4,
                cache_write: 3
            }
        );
        let price = quote(TokenRates {
            input: 2_000_000,
            output: 4_000_000,
            cache_read: 500_000,
            cache_write: 3_000_000,
            request: 0,
        });
        assert_eq!(credits(price).charge(usage), Ok(Credits(13)));
    }

    #[test]
    fn invalid_total_and_cache_subsets_are_rejected() {
        for (input, read, write) in [(1, 2, 0), (1, 0, 2), (3, 2, 2), (u64::MAX, u64::MAX, 1)] {
            assert_eq!(
                Usage::from_totals(input, 0, read, write),
                Err(RoutingError::InvalidRequest)
            );
        }
    }

    #[test]
    fn conversion_is_applied_before_any_currency_rounding() {
        let price = quote(rates(1_100_000));
        let frozen = price.with_credit_rate(CreditRate::new(10, 700_000).unwrap());
        let usage = Usage {
            input: 1,
            ..Usage::default()
        };
        assert_eq!(price.charge(usage), Ok(MicroUsd(2)));
        // 1.1 micro-USD * 0.7 credits/micro-USD = 0.77 credits, rounded to 1.
        // Rounding micro-USD first would incorrectly produce 2 credits.
        assert_eq!(frozen.charge(usage), Ok(Credits(1)));
    }

    #[test]
    fn request_fee_is_once_per_logical_quote_not_per_token_dimension() {
        let price = quote(TokenRates {
            input: 1_000_000,
            output: 1_000_000,
            cache_read: 1_000_000,
            cache_write: 1_000_000,
            request: 1,
        });
        let usage = Usage {
            input: 1,
            output: 1,
            cache_read: 1,
            cache_write: 1,
        };
        assert_eq!(credits(price).charge(usage), Ok(Credits(3)));
        assert_eq!(credits(price).charge(Usage::default()), Ok(Credits(1)));
    }

    #[test]
    fn captured_price_and_conversion_are_immutable_values() {
        let mut original = quote(rates(2_000_000));
        let frozen = credits(original);
        original.version = 4;
        original.rates.input = 4_000_000;
        let later = original.with_credit_rate(CreditRate::new(10, 1_000_000).unwrap());
        let usage = Usage {
            input: 1,
            ..Usage::default()
        };
        assert_eq!(frozen.price().version(), 3);
        assert_eq!(frozen.price().rule_id(), 7);
        assert_eq!(frozen.conversion().version(), 9);
        assert_eq!(frozen.conversion().credits_per_usd(), 500_000);
        assert_eq!(frozen.charge(usage), Ok(Credits(1)));
        assert_eq!(later.charge(usage), Ok(Credits(4)));
    }

    #[test]
    fn zero_price_is_zero_but_conversion_must_be_valid() {
        assert_eq!(
            credits(quote(rates(0))).charge(Usage::default()),
            Ok(Credits(0))
        );
        for (version, rate) in [(0, 500_000), (1, 0), (u64::MAX, 1), (1, u64::MAX)] {
            assert!(CreditRate::new(version, rate).is_err());
        }
    }

    #[test]
    fn signed_ledger_limit_and_intermediate_overflow_fail_closed() {
        let mut request_price = quote(rates(0));
        request_price.rates.request = i64::MAX as u64;
        let equal = request_price.with_credit_rate(CreditRate::new(1, 1_000_000).unwrap());
        assert_eq!(equal.charge(Usage::default()), Ok(Credits(i64::MAX as u64)));
        let doubled = request_price.with_credit_rate(CreditRate::new(2, 2_000_000).unwrap());
        assert_eq!(
            doubled.charge(Usage::default()),
            Err(RoutingError::PriceOverflow)
        );
        let huge = credits(quote(rates(i64::MAX as u64)));
        assert_eq!(
            huge.charge(Usage {
                input: u64::MAX,
                ..Usage::default()
            }),
            Err(RoutingError::PriceOverflow)
        );
    }

    #[test]
    fn reservation_ceiling_covers_all_cache_splits_without_mutating_quote() {
        let frozen = credits(quote(TokenRates {
            input: 1_000_000,
            output: 3_000_000,
            cache_read: 4_000_000,
            cache_write: 7_000_000,
            request: 3,
        }));
        let before = frozen;
        let ceiling = frozen.reservation_ceiling(8, 4).unwrap();
        assert_eq!(ceiling, Credits(36));
        for input in 0..=8 {
            for read in 0..=input {
                for write in 0..=(input - read) {
                    for output in 0..=4 {
                        let usage = Usage::from_totals(input, output, read, write).unwrap();
                        assert!(frozen.charge(usage).unwrap().0 <= ceiling.0);
                    }
                }
            }
        }
        assert_eq!(frozen, before);
    }

    #[test]
    fn reservation_rounds_once_and_preserves_request_fee_for_zero_tokens() {
        let frozen = credits(quote(TokenRates {
            input: 1,
            output: 1,
            cache_read: 1,
            cache_write: 1,
            request: 1,
        }));
        assert_eq!(frozen.reservation_ceiling(0, 0), Ok(Credits(1)));
        assert_eq!(
            frozen.reservation_ceiling(1_000_000, 1_000_000),
            Ok(Credits(2))
        );
        assert_eq!(
            credits(quote(rates(0))).reservation_ceiling(0, 0),
            Ok(Credits(0))
        );
    }
}

#[cfg(test)]
mod legacy_rounding_tests {
    use super::*;

    fn snapshot(group: u32) -> PriceSnapshot {
        PriceSnapshot::compile(PriceConfig {
            version: 1,
            rules: vec![PriceRule {
                rounding: Some(TokenRounding::LegacyGoRatioV1),
                billing_mode: BillingMode::Tokens,
                tiered: None,
                id: 1,
                model: "google/gemini-3.1-flash-image".into(),
                group: "default".into(),
                speed: Speed::Standard,
                rates: TokenRates {
                    input: 500_000,
                    output: 3_000_000,
                    cache_read: 50_000,
                    cache_write: 0,
                    request: 0,
                },
                group_multiplier_ppm: group,
                speed_multiplier_ppm: 1_000_000,
            }],
        })
        .unwrap()
    }

    #[test]
    fn legacy_rounding_fixed_go_oracle_and_reservations() {
        let price = snapshot(1_200_000)
            .quote("google/gemini-3.1-flash-image", "default", Speed::Standard)
            .unwrap();
        let quote = price
            .with_credit_rate(CreditRate::new(1, 500_000).unwrap())
            .with_discount(Some(
                crate::billing::trust_pricing::DiscountRatio::new(9, 10).unwrap(),
            ));
        // cc52aa text_quota.go:353-361, 411-442; same MR=.25, CR=6, cache=.1, group=1.2, trust=.9.
        for (input, cached, output, expected) in [
            (7, 2, 19, 32),
            (7, 2, 1, 3),
            (1, 0, 0, 1),
            (0, 0, 25, 41),
            (0, 0, 0, 0),
        ] {
            assert_eq!(
                quote
                    .charge(Usage::from_totals(input, output, cached, 0).unwrap())
                    .unwrap(),
                Credits(expected)
            );
        }
        assert_eq!(quote.reservation_ceiling(100, 20).unwrap(), Credits(60));
        let mut zero_cache = quote;
        zero_cache.price.rates.cache_read = 0;
        assert_eq!(
            zero_cache
                .charge(Usage::from_totals(1, 0, 1, 0).unwrap())
                .unwrap(),
            Credits(1)
        );
        let mut zero_output = quote;
        zero_output.price.rates.output = 0;
        assert_eq!(
            zero_output
                .charge(Usage::from_totals(0, 1, 0, 0).unwrap())
                .unwrap(),
            Credits(1)
        );
        let free = snapshot(0)
            .quote("google/gemini-3.1-flash-image", "default", Speed::Standard)
            .unwrap()
            .with_credit_rate(CreditRate::new(1, 500_000).unwrap());
        assert_eq!(
            free.charge(Usage::from_totals(7, 19, 2, 0).unwrap())
                .unwrap(),
            Credits(0)
        );
        assert_eq!(free.reservation_ceiling(100, 20).unwrap(), Credits(0));
        let mut zero_ratio = quote;
        zero_ratio.price.rates = TokenRates {
            input: 0,
            output: 0,
            cache_read: 0,
            cache_write: 0,
            request: 0,
        };
        assert_eq!(
            zero_ratio
                .charge(Usage::from_totals(1, 1, 0, 0).unwrap())
                .unwrap(),
            Credits(0)
        );
        assert!(
            quote
                .charge(Usage::from_totals(1, 0, 0, 1).unwrap())
                .is_err()
        );
        assert_eq!(zero_output.reservation_ceiling(0, 1).unwrap(), Credits(1));
        assert_eq!(
            crate::upstream_cost::half_away_with_discount(u128::MAX, u128::MAX, quote.discount),
            Some(1)
        );
        assert_eq!(
            crate::upstream_cost::half_away_with_discount(u128::MAX, 1, None),
            None
        );
        assert_eq!(
            crate::upstream_cost::half_away_with_discount(1, 0, None),
            None
        );
        let mut native = quote;
        native.price.rounding = None;
        assert_eq!(
            native
                .charge(Usage::from_totals(7, 19, 2, 0).unwrap())
                .unwrap(),
            Credits(33)
        );
    }

    #[test]
    fn legacy_rounding_optional_json_and_reject_other_dimensions() {
        let mut config = snapshot(1_200_000).config().clone();
        assert_eq!(
            serde_json::to_value(config.rules[0].rounding).unwrap(),
            "legacy_go_ratio_half_away_min1_v1"
        );
        config.rules[0].rounding = None;
        let old_bytes = serde_json::to_vec(&config).unwrap();
        assert!(!String::from_utf8_lossy(&old_bytes).contains("rounding"));
        assert_eq!(
            serde_json::to_vec(&serde_json::from_slice::<PriceConfig>(&old_bytes).unwrap())
                .unwrap(),
            old_bytes
        );
        assert!(serde_json::from_str::<TokenRounding>("\"legacy_go_ratio_v1\"").is_err());
        config.rules[0].rounding = Some(TokenRounding::LegacyGoRatioV1);
        for dimension in ["request", "cache_write", "upstream_cost"] {
            let mut invalid = config.clone();
            match dimension {
                "request" => invalid.rules[0].rates.request = 1,
                "cache_write" => invalid.rules[0].rates.cache_write = 1,
                _ => invalid.rules[0].billing_mode = BillingMode::UpstreamCost,
            }
            assert!(PriceSnapshot::compile(invalid).is_err());
        }
    }
}

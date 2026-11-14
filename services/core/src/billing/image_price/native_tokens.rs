//! Bounded compilation of the production image expression, not a general evaluator.
//! The five dimensions follow go-v0.2.118 service/tiered_settle.go's classified
//! cache/media union, including cached images exactly once.
use super::{ExactRatio, ImagePriceError};
use crate::billing::{CREDITS_PER_USD, trust_pricing::DiscountRatio};
use num_bigint::BigUint;
use num_rational::Ratio;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use sha2::{Digest, Sha256};

#[derive(Clone, Copy, Debug, Deserialize, Serialize, Eq, PartialEq)]
#[serde(rename_all = "snake_case")]
pub enum ImageTokenCurrencyBasis {
    LegacyPricingUnit,
}
#[derive(Clone, Copy, Debug, Deserialize, Serialize, Eq, PartialEq)]
#[serde(rename_all = "snake_case")]
pub enum ImageOutputTokenSource {
    ReportedDetails,
    /// Images endpoint contract, not a manufactured output_tokens_details object.
    ImagesEndpointTotal,
}

/// Frozen image usage interpretation selected by the installation owner.
#[derive(Clone, Copy, Debug, Default, Deserialize, Serialize, Eq, PartialEq)]
#[serde(rename_all = "snake_case")]
pub enum ImageTokenUsageContract {
    #[default]
    NativeStrict,
    Cc52OpenaiImagesCacheAbsenceV1,
}
impl ImageTokenUsageContract {
    fn is_native_strict(&self) -> bool {
        *self == Self::NativeStrict
    }
}

/// Trusted deployment input; frozen with the original operation's price.
#[derive(Clone, Debug, Deserialize, Serialize, Eq, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct ImageTokenTariff {
    /// Original legacy lookup key; may differ from the routed model alias.
    pub billing_price_key: String,
    pub source_expression: String,
    pub source_sha256: String,
    #[serde(
        default,
        skip_serializing_if = "ImageTokenUsageContract::is_native_strict"
    )]
    pub usage_contract: ImageTokenUsageContract,
    pub currency_basis: ImageTokenCurrencyBasis,
    pub credits_per_usd: u64,
    pub usd_multiplier: ExactRatio,
    /// Hard reservation bounds; not an estimate derived from the prompt/image.
    pub max_input_tokens: u64,
    pub max_output_tokens: u64,
}

/// Only Core's checked native image response parser creates this evidence.
/// Missing cache classification remains missing across restart/reconciliation.
#[derive(Clone, Copy, Debug, Deserialize, Serialize, Eq, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct ImageTokenUsage {
    pub input: u64,
    pub input_text: u64,
    pub input_image: u64,
    pub cached: Option<u64>,
    pub cached_text: Option<u64>,
    pub cached_image: Option<u64>,
    pub output: u64,
    pub output_image: u64,
    pub output_image_source: ImageOutputTokenSource,
}
fn pending() -> ImagePriceError {
    ImagePriceError::SettlementPending
}
fn invalid() -> ImagePriceError {
    ImagePriceError::InvalidConfig
}
fn native_fields(value: &Value, allowed: &[&str]) -> Result<(), ImagePriceError> {
    if value
        .as_object()
        .is_none_or(|object| object.keys().any(|key| !allowed.contains(&key.as_str())))
    {
        return Err(pending());
    }
    Ok(())
}
impl ImageTokenUsage {
    pub(crate) fn validate(&self) -> Result<(), ImagePriceError> {
        if self.input > i64::MAX as u64
            || self.output > i64::MAX as u64
            || self.input_text.checked_add(self.input_image) != Some(self.input)
            || self.output_image > self.output
            || (self.output_image_source == ImageOutputTokenSource::ImagesEndpointTotal
                && self.output_image != self.output)
            || self.cached.is_some_and(|n| n > self.input)
            || self.cached_text.is_some_and(|n| n > self.input_text)
            || self.cached_image.is_some_and(|n| n > self.input_image)
        {
            return Err(pending());
        }
        match (self.cached, self.cached_text, self.cached_image) {
            (Some(total), Some(text), Some(image)) if text.checked_add(image) == Some(total) => {
                Ok(())
            }
            (_, None, None) => Ok(()), // Unknown is valid evidence, not priceable evidence.
            _ => Err(pending()),
        }
    }
    #[cfg(test)]
    pub(crate) fn from_native(value: &Value) -> Result<Self, ImagePriceError> {
        Self::from_native_with_contract(value, ImageTokenUsageContract::NativeStrict)
    }
    pub(crate) fn from_native_with_contract(
        value: &Value,
        contract: ImageTokenUsageContract,
    ) -> Result<Self, ImagePriceError> {
        native_fields(
            value,
            &[
                "input_tokens",
                "output_tokens",
                "total_tokens",
                "input_tokens_details",
                "output_tokens_details",
            ],
        )?;
        let count = |v: &Value, name: &str| v.get(name).and_then(Value::as_u64).ok_or_else(pending);
        let optional = |v: &Value, name: &str| -> Result<Option<u64>, ImagePriceError> {
            v.get(name).map(|_| count(v, name)).transpose()
        };
        let input = count(value, "input_tokens")?;
        let output = count(value, "output_tokens")?;
        if count(value, "total_tokens")? != input.checked_add(output).ok_or_else(pending)? {
            return Err(pending());
        }
        let details = value
            .get("input_tokens_details")
            .filter(|v| v.is_object())
            .ok_or_else(pending)?;
        native_fields(
            details,
            &[
                "text_tokens",
                "image_tokens",
                "cached_tokens",
                "cached_tokens_details",
            ],
        )?;
        // cc52 InputTokenDetails.CachedTokens is a Go int. Only this frozen
        // source contract gives complete field absence its source default zero.
        // Present nulls, malformed values and partial classifications stay strict.
        let source_absence = contract == ImageTokenUsageContract::Cc52OpenaiImagesCacheAbsenceV1
            && details.get("cached_tokens").is_none()
            && details.get("cached_tokens_details").is_none();
        let cached = if source_absence {
            Some(0)
        } else {
            optional(details, "cached_tokens")?
        };
        let (cached_text, cached_image) = match details.get("cached_tokens_details") {
            Some(v) => {
                native_fields(v, &["text_tokens", "image_tokens", "audio_tokens"])?;
                if v.get("audio_tokens").is_some_and(|n| n.as_u64() != Some(0)) {
                    return Err(pending());
                }
                (
                    Some(count(v, "text_tokens")?),
                    Some(count(v, "image_tokens")?),
                )
            }
            None if cached == Some(0) => (Some(0), Some(0)),
            None => (None, None),
        };
        // Images API documents output_tokens as image tokens. Honor an explicit
        // output breakdown instead of charging reported output text as images.
        let (output_image, output_image_source) = match value.get("output_tokens_details") {
            Some(v) => {
                // Some OpenAI-compatible image hosts echo reasoning_tokens=0.
                // Non-zero reasoning is not part of the images settlement contract.
                native_fields(v, &["text_tokens", "image_tokens", "reasoning_tokens"])?;
                if v.get("reasoning_tokens")
                    .is_some_and(|n| n.as_u64() != Some(0))
                {
                    return Err(pending());
                }
                let image = count(v, "image_tokens")?;
                if count(v, "text_tokens")?.checked_add(image) != Some(output) {
                    return Err(pending());
                }
                (image, ImageOutputTokenSource::ReportedDetails)
            }
            None => (output, ImageOutputTokenSource::ImagesEndpointTotal),
        };
        let result = Self {
            input,
            input_text: count(details, "text_tokens")?,
            input_image: count(details, "image_tokens")?,
            cached,
            cached_text,
            cached_image,
            output,
            output_image,
            output_image_source,
        };
        result.validate()?;
        Ok(result)
    }
}

impl ImageTokenTariff {
    /// Accepted grammar is one standard tier with exactly the five production
    /// variables and a unit suffix. Unknown functions/variables never fall back.
    fn rates(&self) -> Result<[Ratio<BigUint>; 5], ImagePriceError> {
        if self.source_expression.len() > 4096
            || self.billing_price_key.is_empty()
            || self.billing_price_key.len() > 512
            || self.billing_price_key.chars().any(char::is_control)
            || self.source_sha256
                != crate::hex_codec::encode(Sha256::digest(self.source_expression.as_bytes()))
            || self.credits_per_usd != CREDITS_PER_USD as u64
            || self.usd_multiplier.denominator == 0
            || self.max_input_tokens == 0
            || self.max_input_tokens > 1_000_000_000
            || self.max_output_tokens == 0
            || self.max_output_tokens > 1_000_000_000
        {
            return Err(invalid());
        }
        let identifier = |c: char| c.is_ascii_alphanumeric() || matches!(c, '_' | '.');
        let mut previous = None;
        let mut whitespace = false;
        let mut quoted = false;
        for c in self.source_expression.chars() {
            if c == '"' {
                quoted = !quoted;
            }
            if c.is_ascii_whitespace() {
                if quoted {
                    return Err(invalid());
                }
                whitespace = true;
            } else {
                if whitespace && previous.is_some_and(identifier) && identifier(c) {
                    return Err(invalid());
                }
                previous = Some(c);
                whitespace = false;
            }
        }
        let expr: String = self
            .source_expression
            .chars()
            .filter(|c| !c.is_ascii_whitespace())
            .collect();
        let body = expr
            .strip_prefix("(tier(\"standard\",")
            .and_then(|v| v.strip_suffix("))*(1)"))
            .ok_or_else(invalid)?;
        let mut rates: [Option<Ratio<BigUint>>; 5] = std::array::from_fn(|_| None);
        for term in body.split('+') {
            let (variable, coefficient) = term.split_once('*').ok_or_else(invalid)?;
            let index = ["p", "cr_text", "img", "cr_img", "img_o"]
                .iter()
                .position(|v| *v == variable)
                .ok_or_else(invalid)?;
            if rates[index].is_some() || coefficient.len() > 20 {
                return Err(invalid());
            }
            let mut parts = coefficient.split('.');
            let whole = parts.next().ok_or_else(invalid)?;
            let fraction = parts.next().unwrap_or("");
            if whole.is_empty()
                || fraction.len() > 9
                || parts.next().is_some()
                || !whole
                    .bytes()
                    .chain(fraction.bytes())
                    .all(|c| c.is_ascii_digit())
            {
                return Err(invalid());
            }
            let numerator = format!("{whole}{fraction}")
                .parse::<u64>()
                .map_err(|_| invalid())?;
            rates[index] = Some(Ratio::new(
                BigUint::from(numerator),
                BigUint::from(10_u64.pow(fraction.len() as u32)),
            ));
        }
        rates
            .map(|r| r.ok_or_else(invalid))
            .into_iter()
            .collect::<Result<Vec<_>, _>>()?
            .try_into()
            .map_err(|_| invalid())
    }
    pub(crate) fn validate(&self) -> Result<(), ImagePriceError> {
        self.rates().map(|_| ())
    }
    pub(crate) fn credits(
        &self,
        usage: Option<ImageTokenUsage>,
        group: ExactRatio,
        discount: Option<DiscountRatio>,
    ) -> Result<i64, ImagePriceError> {
        let rates = self.rates()?;
        let (raw, ceiling) = match usage {
            None => {
                let input_rate = rates[..4].iter().max().ok_or_else(invalid)?.clone();
                (
                    input_rate * BigUint::from(self.max_input_tokens)
                        + rates[4].clone() * BigUint::from(self.max_output_tokens),
                    true,
                )
            }
            Some(u) => {
                u.validate()?;
                if u.input == 0 && u.output == 0 {
                    return Err(pending());
                }
                if u.input > self.max_input_tokens || u.output > self.max_output_tokens {
                    return Err(ImagePriceError::BudgetExceeded);
                }
                let text_cache = u.cached_text.ok_or_else(pending)?;
                let image_cache = u.cached_image.ok_or_else(pending)?;
                // Same union subtraction as the source BuildTieredTokenParams.
                let dims = [
                    u.input_text - text_cache,
                    text_cache,
                    u.input_image - image_cache,
                    image_cache,
                    u.output_image,
                ];
                (
                    rates.into_iter().zip(dims).fold(
                        Ratio::from_integer(BigUint::from(0_u8)),
                        |sum, (rate, count)| sum + rate * BigUint::from(count),
                    ),
                    false,
                )
            }
        };
        if group.denominator == 0 {
            return Err(invalid());
        }
        let mut amount = raw
            * Ratio::new(
                BigUint::from(self.credits_per_usd)
                    * BigUint::from(group.numerator)
                    * BigUint::from(self.usd_multiplier.numerator),
                BigUint::from(1_000_000_u64)
                    * BigUint::from(group.denominator)
                    * BigUint::from(self.usd_multiplier.denominator),
            );
        if let Some(d) = discount {
            amount *= Ratio::new(BigUint::from(d.numerator()), BigUint::from(d.denominator()));
        }
        rounded_credits(amount, ceiling)
    }
}

/// The separately configured old ratio path is not an expression or a per-image
/// price. In particular, retain the source's aggregate-cache/media arithmetic.
#[derive(Clone, Debug, Deserialize, Serialize, Eq, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct LegacyImageRatioTariff {
    pub billing_price_key: String,
    pub model_ratio: ExactRatio,
    pub completion_ratio: ExactRatio,
    pub cache_ratio: ExactRatio,
    pub image_ratio: ExactRatio,
    pub legacy_quota_per_unit: u64,
    pub credits_per_usd: u64,
    pub usd_multiplier: ExactRatio,
    pub max_input_tokens: u64,
    pub max_output_tokens: u64,
}
fn fraction(value: ExactRatio) -> Result<Ratio<BigUint>, ImagePriceError> {
    if value.denominator == 0 {
        return Err(invalid());
    }
    Ok(Ratio::new(
        BigUint::from(value.numerator),
        BigUint::from(value.denominator),
    ))
}
fn rounded_credits(amount: Ratio<BigUint>, ceiling: bool) -> Result<i64, ImagePriceError> {
    let remainder = amount.numer() % amount.denom();
    let round = if ceiling {
        remainder != BigUint::from(0_u8)
    } else {
        (&remainder << 1_usize) >= amount.denom().clone()
    };
    let mut result = amount.to_integer() + BigUint::from(u8::from(round));
    if amount.numer() != &BigUint::from(0_u8) {
        result = result.max(BigUint::from(1_u8));
    }
    i64::try_from(result).map_err(|_| ImagePriceError::Overflow)
}
impl LegacyImageRatioTariff {
    pub(crate) fn validate(&self) -> Result<(), ImagePriceError> {
        if self.billing_price_key.is_empty()
            || self.billing_price_key.len() > 512
            || self.billing_price_key.chars().any(char::is_control)
            || self.legacy_quota_per_unit != 500000
            || self.credits_per_usd != CREDITS_PER_USD as u64
            || !(1..=1_000_000_000).contains(&self.max_input_tokens)
            || !(1..=1_000_000_000).contains(&self.max_output_tokens)
        {
            return Err(invalid());
        }
        for value in [
            self.model_ratio,
            self.completion_ratio,
            self.cache_ratio,
            self.image_ratio,
            self.usd_multiplier,
        ] {
            fraction(value)?;
        }
        Ok(())
    }
    pub(crate) fn credits(
        &self,
        usage: Option<ImageTokenUsage>,
        group: ExactRatio,
        discount: Option<DiscountRatio>,
    ) -> Result<i64, ImagePriceError> {
        self.validate()?;
        let cache = fraction(self.cache_ratio)?;
        let image = fraction(self.image_ratio)?;
        let completion = fraction(self.completion_ratio)?;
        let (raw, ceiling) = match usage {
            None => {
                // Cached and image counters may overlap in the legacy path.
                let maximum = (cache + image).max(Ratio::from_integer(BigUint::from(1_u8)));
                (
                    maximum * BigUint::from(self.max_input_tokens)
                        + completion * BigUint::from(self.max_output_tokens),
                    true,
                )
            }
            Some(u) => {
                u.validate()?;
                if u.input > self.max_input_tokens || u.output > self.max_output_tokens {
                    return Err(ImagePriceError::BudgetExceeded);
                }
                // Do not reproduce the old HTTP fallback's invented one prompt
                // token when the provider supplied no authoritative token usage.
                if u.input == 0 && u.output == 0 {
                    return Err(pending());
                }
                let cached = u.cached.ok_or_else(pending)?;
                let base = u.input.saturating_sub(cached).saturating_sub(u.input_image);
                (
                    Ratio::from_integer(BigUint::from(base))
                        + cache * BigUint::from(cached)
                        + image * BigUint::from(u.input_image)
                        + completion * BigUint::from(u.output),
                    false,
                )
            }
        };
        let mut amount = raw
            * fraction(self.model_ratio)?
            * fraction(group)?
            * fraction(self.usd_multiplier)?
            * Ratio::new(
                BigUint::from(self.credits_per_usd),
                BigUint::from(self.legacy_quota_per_unit),
            );
        if let Some(d) = discount {
            amount *= Ratio::new(BigUint::from(d.numerator()), BigUint::from(d.denominator()));
        }
        rounded_credits(amount, ceiling)
    }
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    pub(crate) const EXPRESSION: &str =
        "(tier(\"standard\", p * 5 + cr_text * 1.25 + img * 8 + cr_img * 2 + img_o * 30)) * (1)";
    pub(crate) fn tariff() -> ImageTokenTariff {
        ImageTokenTariff {
            billing_price_key: "gpt-image-2".into(),
            source_expression: EXPRESSION.into(),
            source_sha256: crate::hex_codec::encode(Sha256::digest(EXPRESSION.as_bytes())),
            usage_contract: ImageTokenUsageContract::NativeStrict,
            currency_basis: ImageTokenCurrencyBasis::LegacyPricingUnit,
            credits_per_usd: 500000,
            usd_multiplier: ExactRatio {
                numerator: 1,
                denominator: 1,
            },
            max_input_tokens: 100,
            max_output_tokens: 100,
        }
    }
    fn group() -> ExactRatio {
        ExactRatio {
            numerator: 1,
            denominator: 1,
        }
    }
    pub(crate) fn measured() -> Value {
        // Fixed legacy go-v0.2.118 image_usage_output_details_test.go fixture.
        // Not a captured provider response. Output fallback follows the Images
        // API contract: output_tokens counts image tokens when no details exist.
        serde_json::json!({"input_tokens":100,"output_tokens":10,"total_tokens":110,
            "input_tokens_details":{"cached_tokens":40,"text_tokens":40,"image_tokens":60,
                "cached_tokens_details":{"text_tokens":10,"image_tokens":30,"audio_tokens":0}}})
    }
    pub(crate) fn ratio_tariff() -> LegacyImageRatioTariff {
        LegacyImageRatioTariff {
            billing_price_key: "google/gemini-3.1-flash-image".into(),
            model_ratio: ExactRatio {
                numerator: 1,
                denominator: 4,
            },
            completion_ratio: ExactRatio {
                numerator: 6,
                denominator: 1,
            },
            cache_ratio: ExactRatio {
                numerator: 1,
                denominator: 10,
            },
            image_ratio: ExactRatio {
                numerator: 1,
                denominator: 1,
            },
            legacy_quota_per_unit: 500000,
            credits_per_usd: 500000,
            usd_multiplier: ExactRatio {
                numerator: 1,
                denominator: 1,
            },
            max_input_tokens: 100,
            max_output_tokens: 100,
        }
    }
    #[test]
    fn native_image_legacy_ratio_keeps_source_path_and_aggregate_cache_semantics() {
        let mut value = measured();
        value["input_tokens_details"]
            .as_object_mut()
            .unwrap()
            .remove("cached_tokens_details");
        let usage = ImageTokenUsage::from_native(&value).unwrap();
        // Source max(100-40-60,0)+40*.1+60*1+10*6, times .25.
        assert_eq!(
            ratio_tariff().credits(Some(usage), group(), None).unwrap(),
            31
        );
        assert_eq!(ratio_tariff().credits(None, group(), None).unwrap(), 178);
        assert_eq!(tariff().credits(Some(usage), group(), None), Err(pending()));
        let mut incompatible = ratio_tariff();
        incompatible.legacy_quota_per_unit = 1000000;
        assert!(incompatible.validate().is_err());
    }
    #[test]
    fn native_image_production_expression_counts_cache_union_once_and_rounds_once() {
        let usage = ImageTokenUsage::from_native(&measured()).unwrap();
        // (30*5 + 10*1.25 + 30*8 + 30*2 + 10*30)/1e6*500000 = 381.25.
        assert_eq!(tariff().credits(Some(usage), group(), None).unwrap(), 381);
        assert_eq!(tariff().credits(None, group(), None).unwrap(), 1900);
        assert_eq!(
            tariff()
                .credits(
                    Some(usage),
                    group(),
                    Some(DiscountRatio::new(1, 2).unwrap())
                )
                .unwrap(),
            191
        );
        let mut value = measured();
        value["output_tokens_details"] = serde_json::json!({"text_tokens":4,"image_tokens":6});
        assert_eq!(
            tariff()
                .credits(
                    Some(ImageTokenUsage::from_native(&value).unwrap()),
                    group(),
                    None
                )
                .unwrap(),
            321
        );
    }
    #[test]
    fn native_image_missing_classification_is_not_zero_or_inferred_from_media() {
        let mut value = measured();
        value["input_tokens_details"]
            .as_object_mut()
            .unwrap()
            .remove("cached_tokens_details");
        let unknown = ImageTokenUsage::from_native(&value).unwrap();
        assert_eq!(unknown.cached, Some(40));
        assert_eq!(unknown.cached_text, None);
        assert_eq!(
            tariff().credits(Some(unknown), group(), None),
            Err(pending())
        );
        value["input_tokens_details"]
            .as_object_mut()
            .unwrap()
            .remove("cached_tokens");
        assert_eq!(
            tariff().credits(
                Some(ImageTokenUsage::from_native(&value).unwrap()),
                group(),
                None
            ),
            Err(pending())
        );
        value["input_tokens_details"]["cached_tokens"] = serde_json::json!(0);
        let zero = ImageTokenUsage::from_native(&value).unwrap();
        assert_eq!((zero.cached_text, zero.cached_image), (Some(0), Some(0)));
        assert_eq!(tariff().credits(Some(zero), group(), None).unwrap(), 490);
    }
    fn absent_cache() -> Value {
        let mut raw = measured();
        let details = raw["input_tokens_details"].as_object_mut().unwrap();
        details.remove("cached_tokens");
        details.remove("cached_tokens_details");
        raw
    }
    #[test]
    fn cc52_cache_absence_is_explicit_and_leaves_raw_usage_unchanged() {
        let raw = absent_cache();
        let original = raw.clone();
        let strict = ImageTokenUsage::from_native(&raw).unwrap();
        assert_eq!(
            (strict.cached, strict.cached_text, strict.cached_image),
            (None, None, None)
        );
        assert_eq!(
            tariff().credits(Some(strict), group(), None),
            Err(pending())
        );
        let source = ImageTokenUsage::from_native_with_contract(
            &raw,
            ImageTokenUsageContract::Cc52OpenaiImagesCacheAbsenceV1,
        )
        .unwrap();
        assert_eq!(
            (source.cached, source.cached_text, source.cached_image),
            (Some(0), Some(0), Some(0))
        );
        // Source dimensions are p=40,img=60,img_o=10, with no cache charge.
        assert_eq!(tariff().credits(Some(source), group(), None), Ok(490));
        assert_eq!(
            tariff().credits(
                Some(source),
                ExactRatio {
                    numerator: 1,
                    denominator: 20
                },
                None
            ),
            Ok(25)
        );
        assert_eq!(raw, original);
        let measured_raw = measured();
        assert_eq!(
            ImageTokenUsage::from_native_with_contract(
                &measured_raw,
                ImageTokenUsageContract::Cc52OpenaiImagesCacheAbsenceV1
            ),
            ImageTokenUsage::from_native(&measured_raw)
        );
    }
    #[test]
    fn cc52_cache_contract_does_not_repair_present_or_partial_measurements() {
        let contract = ImageTokenUsageContract::Cc52OpenaiImagesCacheAbsenceV1;
        for (field, value) in [
            ("cached_tokens", serde_json::json!(null)),
            ("cached_tokens", serde_json::json!("0")),
            ("cached_tokens", serde_json::json!(-1)),
            ("cached_tokens_details", serde_json::json!(null)),
            ("cached_tokens_details", serde_json::json!({})),
            (
                "cached_tokens_details",
                serde_json::json!({"text_tokens":0}),
            ),
            (
                "cached_tokens_details",
                serde_json::json!({"text_tokens":0,"image_tokens":0}),
            ),
        ] {
            let mut raw = absent_cache();
            raw["input_tokens_details"][field] = value;
            assert!(ImageTokenUsage::from_native_with_contract(&raw, contract).is_err());
        }
        let mut positive = measured();
        positive["input_tokens_details"]
            .as_object_mut()
            .unwrap()
            .remove("cached_tokens_details");
        let usage = ImageTokenUsage::from_native_with_contract(&positive, contract).unwrap();
        assert_eq!(
            (usage.cached, usage.cached_text, usage.cached_image),
            (Some(40), None, None)
        );
        assert_eq!(tariff().credits(Some(usage), group(), None), Err(pending()));
        let mut mismatch = measured();
        mismatch["input_tokens_details"]["cached_tokens"] = serde_json::json!(0);
        assert!(ImageTokenUsage::from_native_with_contract(&mismatch, contract).is_err());
    }
    #[test]
    fn native_image_invalid_usage_or_uncompiled_price_cannot_settle() {
        for (field, wrong) in [
            ("input_tokens", serde_json::json!(-1)),
            ("total_tokens", serde_json::json!(111)),
            ("input_tokens", serde_json::json!(100.5)),
        ] {
            let mut value = measured();
            value[field] = wrong;
            assert!(ImageTokenUsage::from_native(&value).is_err());
        }
        let mut value = measured();
        value["input_tokens_details"]["cached_tokens_details"]["image_tokens"] =
            serde_json::json!(61);
        assert!(ImageTokenUsage::from_native(&value).is_err());
        let mut price = tariff();
        price.max_input_tokens = 99;
        assert_eq!(
            price.credits(
                Some(ImageTokenUsage::from_native(&measured()).unwrap()),
                group(),
                None
            ),
            Err(ImagePriceError::BudgetExceeded)
        );
        price = tariff();
        price.source_expression = EXPRESSION.replace("img_o", "c");
        price.source_sha256 =
            crate::hex_codec::encode(Sha256::digest(price.source_expression.as_bytes()));
        assert!(price.validate().is_err());
        price = tariff();
        price.credits_per_usd = 1000000;
        assert!(price.validate().is_err());
        price = tariff();
        price.source_sha256 = "0".repeat(64);
        assert!(price.validate().is_err());
    }
}

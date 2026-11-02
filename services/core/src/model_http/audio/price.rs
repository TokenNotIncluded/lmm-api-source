use crate::billing::{CREDITS_PER_USD, Credits, trust_pricing::DiscountRatio};
use num_bigint::BigUint;
use num_rational::Ratio;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

pub(super) type Exact = Ratio<BigUint>;
pub(super) type Result<T> = std::result::Result<T, AudioError>;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AudioError {
    InvalidQuote,
    UnknownUsage,
    InvalidUsage,
    BindingMismatch,
    BoundsExceeded,
    BudgetExceeded,
    Overflow,
}
impl AudioError {
    pub fn code(self) -> &'static str {
        match self {
            Self::InvalidQuote => "audio_invalid_quote",
            Self::UnknownUsage => "audio_usage_unknown",
            Self::InvalidUsage => "audio_usage_invalid",
            Self::BindingMismatch => "audio_binding_mismatch",
            Self::BoundsExceeded => "audio_bounds_exceeded",
            Self::BudgetExceeded => "audio_budget_exceeded",
            Self::Overflow => "audio_price_overflow",
        }
    }
}

/// Disjoint token counts, not bytes, seconds or text-equivalent tokens.
/// No Default and no serde defaults: absent is not zero.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AudioUsage {
    pub text_input: u64,
    pub text_output: u64,
    pub audio_input: u64,
    pub audio_output: u64,
}
impl AudioUsage {
    pub(super) fn values(self) -> [u64; 4] {
        [
            self.text_input,
            self.text_output,
            self.audio_input,
            self.audio_output,
        ]
    }
    pub(super) fn input(self) -> Result<u64> {
        self.text_input
            .checked_add(self.audio_input)
            .ok_or(AudioError::Overflow)
    }
    pub(super) fn output(self) -> Result<u64> {
        self.text_output
            .checked_add(self.audio_output)
            .ok_or(AudioError::Overflow)
    }
    pub(super) fn within(self, bound: Self) -> bool {
        self.values()
            .into_iter()
            .zip(bound.values())
            .all(|(n, max)| n <= max)
    }
}

/// Nonnegative exact fraction. Units are specified at each use site.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AudioRatio {
    pub numerator: u64,
    pub denominator: u64,
}
impl AudioRatio {
    pub(super) fn exact(self) -> Result<Exact> {
        if self.denominator == 0 || self.denominator > 1_000_000_000_000_000_000 {
            return Err(AudioError::InvalidQuote);
        }
        Ok(Exact::new(self.numerator.into(), self.denominator.into()))
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AudioRounding {
    Ceiling,
    HalfUp,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AudioRates {
    /// Each field is USD per million tokens of that modality.
    pub text_input: AudioRatio,
    pub text_output: AudioRatio,
    pub audio_input: AudioRatio,
    pub audio_output: AudioRatio,
}
impl AudioRates {
    fn values(self) -> [AudioRatio; 4] {
        [
            self.text_input,
            self.text_output,
            self.audio_input,
            self.audio_output,
        ]
    }
    pub(super) fn amount(self, usage: AudioUsage) -> Result<Exact> {
        let mut sum = integer(0);
        for (rate, count) in self.values().into_iter().zip(usage.values()) {
            sum += rate.exact()? * integer(count);
        }
        Ok(sum * integer(CREDITS_PER_USD as u64) / integer(1_000_000))
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AudioTier {
    /// Inclusive text_input + audio_input; final tier must be unbounded.
    /// The selected tier applies to ALL four counts, not progressive blocks.
    pub up_to_input_tokens: Option<u64>,
    pub rates: AudioRates,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum AudioTariff {
    Multipliers {
        /// Credits per text-input token; the other three ratios are dimensionless.
        model: AudioRatio,
        completion: AudioRatio,
        audio: AudioRatio,
        audio_completion: AudioRatio,
    },
    Fixed {
        /// USD per successfully completed logical request, never per retry.
        usd_per_request: AudioRatio,
    },
    /// Bounded compiled tariff, not a legacy expression interpreter.
    Tiered { tiers: Vec<AudioTier> },
}
impl AudioTariff {
    pub(super) fn validate(&self) -> Result<()> {
        match self {
            Self::Multipliers {
                model,
                completion,
                audio,
                audio_completion,
            } => {
                for ratio in [model, completion, audio, audio_completion] {
                    ratio.exact()?;
                }
            }
            Self::Fixed { usd_per_request } => {
                usd_per_request.exact()?;
            }
            Self::Tiered { tiers } => {
                if tiers.is_empty() || tiers.len() > 16 {
                    return Err(AudioError::InvalidQuote);
                }
                let mut previous = None;
                for (index, tier) in tiers.iter().enumerate() {
                    match tier.up_to_input_tokens {
                        Some(limit)
                            if index + 1 < tiers.len() && previous.is_none_or(|p| limit > p) =>
                        {
                            previous = Some(limit)
                        }
                        None if index + 1 == tiers.len() => (),
                        _ => return Err(AudioError::InvalidQuote),
                    }
                    for rate in tier.rates.values() {
                        rate.exact()?;
                    }
                }
            }
        }
        Ok(())
    }
    pub(super) fn amount(&self, usage: AudioUsage) -> Result<Exact> {
        match self {
            Self::Multipliers {
                model,
                completion,
                audio,
                audio_completion,
            } => Ok((integer(usage.text_input)
                + integer(usage.text_output) * completion.exact()?
                + integer(usage.audio_input) * audio.exact()?
                + integer(usage.audio_output) * audio.exact()? * audio_completion.exact()?)
                * model.exact()?),
            Self::Fixed { usd_per_request } => {
                Ok(usd_per_request.exact()? * integer(CREDITS_PER_USD as u64))
            }
            Self::Tiered { tiers } => {
                let input = usage.input()?;
                tiers
                    .iter()
                    .find(|t| t.up_to_input_tokens.is_none_or(|max| input <= max))
                    .ok_or(AudioError::InvalidQuote)?
                    .rates
                    .amount(usage)
            }
        }
    }
}

/// Server-issued attribution. authority_sha256 binds actor/key/payer and the
/// authorization/trust admission snapshot in the existing charging contract.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AudioBinding {
    pub request_id: String,
    pub request_sha256: [u8; 32],
    pub authority_sha256: [u8; 32],
    pub route_id: String,
    pub upstream_id: u64,
    pub model: String,
    pub group: String,
    pub provider_contract_revision: String,
}

/// Installation/admission input only, never a public request body.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AudioQuoteSpec {
    pub version: u32,
    pub price_revision: String,
    pub source_sha256: [u8; 32],
    pub binding: AudioBinding,
    pub tariff: AudioTariff,
    pub group_multiplier: AudioRatio,
    pub discount: Option<DiscountRatio>,
    pub rounding: AudioRounding,
    pub usage_format: super::AudioUsageFormat,
    /// Provider-verified four-dimensional bounds, including message/tool inputs.
    pub bounds: AudioUsage,
    pub max_budget: Credits,
}

/// Private immutable snapshot. Persist spec + digest with the hold atomically.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AudioQuote {
    pub(super) spec: AudioQuoteSpec,
    digest: [u8; 32],
    reserve: Credits,
}
impl AudioQuote {
    pub fn freeze(spec: AudioQuoteSpec) -> Result<Self> {
        let binding = &spec.binding;
        if spec.version != 1
            || binding.upstream_id == 0
            || [
                &spec.price_revision,
                &binding.request_id,
                &binding.route_id,
                &binding.model,
                &binding.group,
                &binding.provider_contract_revision,
            ]
            .iter()
            .any(|s| s.is_empty() || s.len() > 256)
        {
            return Err(AudioError::InvalidQuote);
        }
        spec.tariff.validate()?;
        spec.group_multiplier.exact()?;
        spec.bounds.input()?;
        spec.bounds.output()?;
        let digest =
            Sha256::digest(serde_json::to_vec(&spec).map_err(|_| AudioError::InvalidQuote)?).into();
        let mut quote = Self {
            spec,
            digest,
            reserve: Credits::try_from(0_i64).unwrap(),
        };
        quote.reserve =
            super::AudioPrice::from_spec(&quote.spec).reservation(quote.spec.discount)?;
        Ok(quote)
    }
    pub fn restore(
        spec: AudioQuoteSpec,
        expected_digest: [u8; 32],
        expected_binding: &AudioBinding,
    ) -> Result<Self> {
        let quote = Self::freeze(spec)?;
        if quote.digest != expected_digest || &quote.spec.binding != expected_binding {
            return Err(AudioError::BindingMismatch);
        }
        Ok(quote)
    }
    pub fn persisted(&self) -> &AudioQuoteSpec {
        &self.spec
    }
    pub fn digest(&self) -> [u8; 32] {
        self.digest
    }
    pub fn reservation_credits(&self) -> Credits {
        self.reserve
    }
    pub(super) fn cost(&self, usage: AudioUsage) -> Result<Credits> {
        super::AudioPrice::from_spec(&self.spec).cost(usage, self.spec.discount)
    }
}
pub(super) fn integer(n: u64) -> Exact {
    Exact::from_integer(n.into())
}

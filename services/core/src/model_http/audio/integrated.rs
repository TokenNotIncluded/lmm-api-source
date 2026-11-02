//! Persisted charging variants. Price.user_trust is the sole discount authority.
use super::{
    price::{Exact, Result, integer},
    *,
};
use crate::billing::{Credits, trust_pricing::DiscountRatio};
use num_bigint::BigUint;
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AudioPrice {
    pub version: u32,
    pub price_revision: String,
    pub source_sha256: [u8; 32],
    pub model: String,
    pub group: String,
    pub provider_contract_revision: String,
    pub tariff: AudioTariff,
    pub group_multiplier: AudioRatio,
    pub rounding: AudioRounding,
    pub usage_format: AudioUsageFormat,
    pub bounds: AudioUsage,
    pub max_input_tokens: u64,
    pub max_output_tokens: u64,
    pub max_budget: Credits,
}
impl AudioPrice {
    pub(super) fn from_spec(s: &AudioQuoteSpec) -> Self {
        Self {
            version: s.version,
            price_revision: s.price_revision.clone(),
            source_sha256: s.source_sha256,
            model: s.binding.model.clone(),
            group: s.binding.group.clone(),
            provider_contract_revision: s.binding.provider_contract_revision.clone(),
            tariff: s.tariff.clone(),
            group_multiplier: s.group_multiplier,
            rounding: s.rounding,
            usage_format: s.usage_format,
            bounds: s.bounds,
            max_input_tokens: s.bounds.input().unwrap_or(u64::MAX),
            max_output_tokens: s.bounds.output().unwrap_or(u64::MAX),
            max_budget: s.max_budget,
        }
    }
    pub fn validate(&self) -> Result<()> {
        if self.version != 1
            || [
                &self.price_revision,
                &self.model,
                &self.group,
                &self.provider_contract_revision,
            ]
            .iter()
            .any(|s| s.is_empty() || s.len() > 256)
        {
            return Err(AudioError::InvalidQuote);
        }
        self.tariff.validate()?;
        self.group_multiplier.exact()?;
        self.bounds.input()?;
        self.bounds.output()?;
        Ok(())
    }
    pub fn reservation(&self, discount: Option<DiscountRatio>) -> Result<Credits> {
        self.validate()?;
        // A cheaper high-volume band must not under-reserve a smaller request.
        let maximum = match &self.tariff {
            AudioTariff::Tiered { tiers } => {
                let mut maximum = integer(0);
                let mut lower = 0;
                for tier in tiers {
                    if lower > self.bounds.input()? {
                        break;
                    }
                    maximum = maximum.max(tier.rates.amount(self.bounds)?);
                    match tier.up_to_input_tokens {
                        Some(max) => match max.checked_add(1) {
                            Some(next) => lower = next,
                            None => break,
                        },
                        None => break,
                    }
                }
                maximum
            }
            tariff => tariff.amount(self.bounds)?,
        };
        self.finish(maximum, discount)
    }
    pub fn cost(&self, usage: AudioUsage, discount: Option<DiscountRatio>) -> Result<Credits> {
        self.validate()?;
        if !usage.within(self.bounds)
            || usage.input()? > self.max_input_tokens
            || usage.output()? > self.max_output_tokens
        {
            return Err(AudioError::BoundsExceeded);
        }
        self.finish(self.tariff.amount(usage)?, discount)
    }
    fn finish(&self, amount: Exact, discount: Option<DiscountRatio>) -> Result<Credits> {
        let mut amount = amount * self.group_multiplier.exact()?;
        if let Some(discount) = discount {
            amount *= Exact::new(discount.numerator().into(), discount.denominator().into());
        }
        let n = amount.numer();
        let d = amount.denom();
        let remainder = n % d;
        let increment = match self.rounding {
            AudioRounding::Ceiling => remainder != BigUint::from(0_u8),
            AudioRounding::HalfUp => remainder * BigUint::from(2_u8) >= *d,
        };
        let value = i64::try_from(n / d + BigUint::from(u8::from(increment)))
            .map_err(|_| AudioError::Overflow)?;
        if value > i64::from(self.max_budget) {
            return Err(AudioError::BudgetExceeded);
        }
        Credits::try_from(value).map_err(|_| AudioError::Overflow)
    }
}

/// Exact terminal counters plus raw evidence identifiers; never client supplied.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AudioMeteredUsage {
    pub counts: AudioUsage,
    pub raw_usage_sha256: [u8; 32],
    pub response_sha256: [u8; 32],
}
impl AudioMeteredUsage {
    pub fn valid(self) -> bool {
        self.counts.input().is_ok() && self.counts.output().is_ok()
    }
}

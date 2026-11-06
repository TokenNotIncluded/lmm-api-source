//! Text request-only sales tariffs. Provider tools use a separate contract.
use crate::{billing::trust_pricing::DiscountRatio, routing};
use serde::{Deserialize, Serialize};

/// Core creates this only from a validated terminal receipt or typed delivery
/// evidence. Rejection is not inferred from HTTP status alone.
#[derive(Clone, Copy, Debug, Deserialize, Serialize, Eq, PartialEq, Ord, PartialOrd)]
#[serde(tag = "outcome", rename_all = "snake_case", deny_unknown_fields)]
pub enum TextRequestUsage {
    NotSent,
    Rejected {
        http_status: u16,
        receipt_sha256: [u8; 32],
    },
    Completed,
}
impl TextRequestUsage {
    pub(crate) fn valid(self) -> bool {
        match self {
            Self::NotSent | Self::Completed => true,
            Self::Rejected {
                http_status,
                receipt_sha256,
            } => (400..=599).contains(&http_status) && receipt_sha256 != [0; 32],
        }
    }
    pub(crate) fn completed_requests(self) -> u8 {
        u8::from(self == Self::Completed)
    }
    pub(crate) fn outcome(self) -> crate::billing::charging::Outcome {
        match self {
            Self::NotSent => crate::billing::charging::Outcome::Cancelled,
            Self::Rejected { .. } => crate::billing::charging::Outcome::Failed,
            Self::Completed => crate::billing::charging::Outcome::Completed,
        }
    }
}

#[derive(Clone, Debug, Deserialize, Serialize, Eq, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct TextRequestPrice {
    pub usd_micros_per_request: u64,
    pub group_multiplier_ppm: u32,
    pub speed_multiplier_ppm: u32,
    pub credits_per_usd: u64,
}
impl TextRequestPrice {
    pub(crate) fn from_quote(quote: routing::PriceQuote) -> Option<Self> {
        let rates = quote.rates();
        // An all-zero existing token tariff remains a token tariff. A positive
        // explicit request rate distinguishes this independent sales contract.
        if quote.billing_mode() != routing::BillingMode::Tokens
            || quote.tiered().is_some()
            || rates.request == 0
            || rates.input != 0
            || rates.output != 0
            || rates.cache_read != 0
            || rates.cache_write != 0
        {
            return None;
        }
        let (group_multiplier_ppm, speed_multiplier_ppm) = quote.multipliers_ppm();
        let price = Self {
            usd_micros_per_request: rates.request,
            group_multiplier_ppm,
            speed_multiplier_ppm,
            credits_per_usd: 500_000,
        };
        price.valid().then_some(price)
    }
    pub(crate) fn valid(&self) -> bool {
        self.usd_micros_per_request > 0
            && self.group_multiplier_ppm <= routing::MAX_MULTIPLIER
            && (1..=routing::MAX_MULTIPLIER).contains(&self.speed_multiplier_ppm)
            && self.credits_per_usd == 500_000
            && self.amount(None).is_some()
    }
    pub(crate) fn reservation(&self, discount: Option<DiscountRatio>) -> Option<i64> {
        if !self.valid() {
            return None;
        }
        self.amount(discount)
    }
    pub(crate) fn cost(
        &self,
        usage: TextRequestUsage,
        discount: Option<DiscountRatio>,
    ) -> Option<i64> {
        if !self.valid() || !usage.valid() {
            return None;
        }
        if usage.completed_requests() == 0 {
            Some(0)
        } else {
            self.amount(discount)
        }
    }
    fn amount(&self, discount: Option<DiscountRatio>) -> Option<i64> {
        let numerator = u128::from(self.usd_micros_per_request)
            .checked_mul(u128::from(self.group_multiplier_ppm))?
            .checked_mul(u128::from(self.speed_multiplier_ppm))?
            .checked_mul(u128::from(self.credits_per_usd))?;
        i64::try_from(crate::upstream_cost::ceil_with_discount(
            numerator,
            1_000_000_u128.pow(3),
            discount,
        )?)
        .ok()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::routing::{BillingMode, PriceConfig, PriceRule, PriceSnapshot, Speed, TokenRates};
    #[test]
    fn request_tariff_is_explicit_exact_and_never_mixed_with_token_rates() {
        let mut rule = PriceRule {
            id: 1,
            model: "text".into(),
            group: "default".into(),
            speed: Speed::Standard,
            billing_mode: BillingMode::Tokens,
            tiered: None,
            rates: TokenRates {
                input: 0,
                output: 0,
                cache_read: 0,
                cache_write: 0,
                request: 3,
            },
            group_multiplier_ppm: 500_000,
            speed_multiplier_ppm: 1_000_000,
        };
        let price = |rule: PriceRule| {
            let snapshot = PriceSnapshot::compile(PriceConfig {
                version: 1,
                rules: vec![rule],
            })
            .unwrap();
            TextRequestPrice::from_quote(
                snapshot.quote("text", "default", Speed::Standard).unwrap(),
            )
        };
        let tariff = price(rule.clone()).unwrap();
        assert_eq!(tariff.reservation(None), Some(1));
        assert_eq!(tariff.cost(TextRequestUsage::Completed, None), Some(1));
        assert_eq!(tariff.cost(TextRequestUsage::NotSent, None), Some(0));
        assert_eq!(
            tariff.cost(
                TextRequestUsage::Rejected {
                    http_status: 429,
                    receipt_sha256: [1; 32]
                },
                None
            ),
            Some(0)
        );
        assert_eq!(
            tariff.cost(
                TextRequestUsage::Rejected {
                    http_status: 200,
                    receipt_sha256: [1; 32]
                },
                None
            ),
            None
        );
        rule.rates.input = 1;
        assert!(price(rule.clone()).is_none());
        rule.rates.input = 0;
        rule.rates.request = 0;
        assert!(price(rule).is_none());
    }
}

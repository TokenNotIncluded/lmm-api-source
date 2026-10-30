//! Reviewed Gemini Google Search surcharge, added before the one credit ceiling.
use crate::{billing::trust_pricing::DiscountRatio, routing};
use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, Deserialize, Serialize, Eq, PartialEq)]
#[serde(rename_all = "snake_case")]
pub enum GroundingMode {
    GroundedRequestV1,
}

#[derive(Clone, Copy, Debug, Deserialize, Serialize, Eq, PartialEq)]
#[serde(rename_all = "snake_case")]
pub enum ToolInputSemantics {
    ExcludeSearchRetrievalV1,
}

/// A host-reviewed supplier contract, never selected from the model's name.
/// Per-query suppliers need a separate bounded contract and are not accepted.
#[derive(Clone, Debug, Deserialize, Serialize, Eq, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct GroundingContract {
    pub mode: GroundingMode,
    pub tool_input: ToolInputSemantics,
    pub usd_micros_per_grounded_request: u64,
}

#[derive(Clone, Copy, Debug, Deserialize, Serialize, Eq, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct GroundingUsage {
    pub grounded_requests: u8,
    /// Provider-reported retrieved input, excluded by this frozen contract.
    /// Absence stays unknown even though it does not change the surcharge.
    pub tool_input_tokens: Option<u64>,
}

impl GroundingUsage {
    pub(crate) fn valid(self) -> bool {
        self.grounded_requests <= 1
    }

    pub(crate) fn follows(self, previous: Self) -> bool {
        self.valid()
            && self.grounded_requests >= previous.grounded_requests
            && previous
                .tool_input_tokens
                .is_none_or(|old| self.tool_input_tokens.is_some_and(|value| value >= old))
    }
}

#[derive(Clone, Debug, Deserialize, Serialize, Eq, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct PersistedGroundingPrice {
    pub contract: GroundingContract,
    pub group_multiplier_ppm: u32,
    pub speed_multiplier_ppm: u32,
    pub provider_contract_revision: String,
}

impl PersistedGroundingPrice {
    pub(crate) fn from_quote(
        contract: GroundingContract,
        quote: routing::PriceQuote,
        provider_contract_revision: String,
    ) -> Option<Self> {
        if quote.billing_mode() != routing::BillingMode::Tokens
            || quote.tiered().is_some()
            || quote.rates().request != 0
            || quote.rates().cache_write != 0
        {
            return None;
        }
        let (group_multiplier_ppm, speed_multiplier_ppm) = quote.multipliers_ppm();
        let price = Self {
            contract,
            group_multiplier_ppm,
            speed_multiplier_ppm,
            provider_contract_revision,
        };
        price.valid().then_some(price)
    }

    pub(crate) fn valid(&self) -> bool {
        self.group_multiplier_ppm <= routing::MAX_MULTIPLIER
            && (1..=routing::MAX_MULTIPLIER).contains(&self.speed_multiplier_ppm)
            && !self.provider_contract_revision.is_empty()
            && self.provider_contract_revision.len() <= 512
            && !self
                .provider_contract_revision
                .chars()
                .any(char::is_control)
            && self.combined_cost(0, 1, None).is_some()
    }

    /// `token_millionths` is the exact scalar-token numerator in Core credits.
    /// One micro-USD = half a credit. This common denominator preserves both
    /// group factors and fractional token fees until the single final ceiling.
    pub(crate) fn combined_cost(
        &self,
        token_millionths: u128,
        grounded_requests: u8,
        discount: Option<DiscountRatio>,
    ) -> Option<i64> {
        if grounded_requests > 1 {
            return None;
        }
        let surcharge = u128::from(self.contract.usd_micros_per_grounded_request)
            .checked_mul(u128::from(self.group_multiplier_ppm))?
            .checked_mul(u128::from(self.speed_multiplier_ppm))?
            .checked_mul(u128::from(grounded_requests))?;
        let numerator = token_millionths
            .checked_mul(2_000_000)?
            .checked_add(surcharge)?;
        let credits =
            crate::upstream_cost::ceil_with_discount(numerator, 2_000_000_000_000, discount)?;
        i64::try_from(credits).ok()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn price() -> PersistedGroundingPrice {
        PersistedGroundingPrice {
            contract: GroundingContract {
                mode: GroundingMode::GroundedRequestV1,
                tool_input: ToolInputSemantics::ExcludeSearchRetrievalV1,
                usd_micros_per_grounded_request: 1,
            },
            group_multiplier_ppm: 1_000_000,
            speed_multiplier_ppm: 1_000_000,
            provider_contract_revision: "synthetic-grounding-request-v1".into(),
        }
    }

    #[test]
    fn grounding_and_tokens_share_one_rounding_step() {
        assert!(price().valid());
        // 0.4 credits of tokens + 0.5 credits of grounding rounds once to 1.
        assert_eq!(price().combined_cost(400_000, 1, None), Some(1));
        assert_eq!(price().combined_cost(600_000, 1, None), Some(2));
        let discount = Some(DiscountRatio::new(9, 10).unwrap());
        assert_eq!(price().combined_cost(600_000, 1, discount), Some(1));
        assert_eq!(price().combined_cost(400_000, 0, None), Some(1));
        assert_eq!(price().combined_cost(0, 2, None), None);
        assert!(
            serde_json::from_value::<GroundingContract>(serde_json::json!({
                "mode":"unique_nonempty_queries_v1", "tool_input":"exclude_search_retrieval_v1",
                "usd_micros_per_grounded_request":1
            }))
            .is_err()
        );
    }
}

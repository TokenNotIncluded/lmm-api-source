//! Native provider tools are priced by successful requests, never synthetic tokens.
use crate::{billing::trust_pricing::DiscountRatio, routing};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

#[derive(Clone, Copy, Debug, Deserialize, Serialize, Eq, PartialEq, Ord, PartialOrd)]
#[serde(rename_all = "snake_case")]
pub enum ToolKind {
    DeepSeekSearch,
    DeepSeekFetch,
}

impl ToolKind {
    pub(crate) fn from_protocol(protocol: routing::Protocol) -> Option<Self> {
        match protocol {
            routing::Protocol::DeepSeekSearch => Some(Self::DeepSeekSearch),
            routing::Protocol::DeepSeekFetch => Some(Self::DeepSeekFetch),
            _ => None,
        }
    }

    pub(crate) fn model(self) -> &'static str {
        match self {
            Self::DeepSeekSearch => "deepseek-search",
            Self::DeepSeekFetch => "deepseek-fetch",
        }
    }

    pub(crate) fn protocol_name(self) -> &'static str {
        match self {
            Self::DeepSeekSearch => "deepseek_search",
            Self::DeepSeekFetch => "deepseek_fetch",
        }
    }

    pub(crate) fn endpoint(self) -> &'static str {
        match self {
            Self::DeepSeekSearch => "tools/search",
            Self::DeepSeekFetch => "tools/fetch",
        }
    }
}

/// Evidence extracted by Core from a complete response of the selected provider.
#[derive(Clone, Copy, Debug, Deserialize, Serialize, Eq, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct ToolUsage {
    pub kind: ToolKind,
    pub successful_requests: u8,
    pub http_status: u16,
    pub provider_receipt_sha256: [u8; 32],
}

impl ToolUsage {
    pub(crate) fn valid(self) -> bool {
        self.successful_requests <= 1
            && (self.http_status == 200 || (400..=599).contains(&self.http_status))
            && (self.successful_requests == 0 || self.http_status == 200)
            && self.provider_receipt_sha256 != [0; 32]
    }

    /// Complete validated receipts retain the provider's outcome even when the
    /// request is free. An empty HTTP 200 result still completed successfully.
    pub(crate) fn outcome(self) -> crate::billing::charging::Outcome {
        if self.http_status == 200 {
            crate::billing::charging::Outcome::Completed
        } else {
            crate::billing::charging::Outcome::Failed
        }
    }

    pub(crate) fn from_response(kind: ToolKind, status: u16, body: &[u8]) -> Option<Self> {
        let successful_requests = if status == 200 {
            let response: serde_json::Value = serde_json::from_slice(body).ok()?;
            let billable = match kind {
                ToolKind::DeepSeekSearch => {
                    let results = response.get("results")?.as_array()?;
                    if results.iter().any(|result| !result.is_object()) {
                        return None;
                    }
                    !results.is_empty()
                }
                ToolKind::DeepSeekFetch => !response.get("content")?.as_str()?.trim().is_empty(),
            };
            u8::from(billable)
        } else if (400..=599).contains(&status) {
            0
        } else {
            return None;
        };
        let mut digest = Sha256::new();
        digest.update(b"lmm-native-tool-receipt-v1\0");
        digest.update(kind.protocol_name());
        digest.update(status.to_be_bytes());
        digest.update(body);
        Some(Self {
            kind,
            successful_requests,
            http_status: status,
            provider_receipt_sha256: digest.finalize().into(),
        })
    }
}

/// The selected route's exact tariff is serialized in the existing frozen price.
/// Restart recovery never looks up a current price or infers one from a model name.
#[derive(Clone, Debug, Deserialize, Serialize, Eq, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct PersistedToolPrice {
    pub kind: ToolKind,
    pub usd_micros_per_request: u64,
    pub group_multiplier_ppm: u32,
    pub speed_multiplier_ppm: u32,
    pub credits_per_usd: u64,
    pub provider_contract_revision: String,
}

impl PersistedToolPrice {
    /// Projection only: the request path separately freezes the selected
    /// provider's actual revision before it can reserve money.
    pub(crate) fn supports_quote(kind: ToolKind, quote: routing::PriceQuote) -> bool {
        Self::from_quote(kind, quote, "catalog-projection-only".into()).is_some()
    }

    pub(crate) fn from_quote(
        kind: ToolKind,
        quote: routing::PriceQuote,
        provider_contract_revision: String,
    ) -> Option<Self> {
        let rates = quote.rates();
        if quote.billing_mode() != routing::BillingMode::Tokens
            || quote.tiered().is_some()
            || rates.input != 0
            || rates.output != 0
            || rates.cache_read != 0
            || rates.cache_write != 0
        {
            return None;
        }
        let (group_multiplier_ppm, speed_multiplier_ppm) = quote.multipliers_ppm();
        let price = Self {
            kind,
            usd_micros_per_request: rates.request,
            group_multiplier_ppm,
            speed_multiplier_ppm,
            credits_per_usd: 500_000,
            provider_contract_revision,
        };
        price.valid().then_some(price)
    }

    pub(crate) fn valid(&self) -> bool {
        self.group_multiplier_ppm <= routing::MAX_MULTIPLIER
            && (1..=routing::MAX_MULTIPLIER).contains(&self.speed_multiplier_ppm)
            && self.credits_per_usd == 500_000
            && !self.provider_contract_revision.is_empty()
            && self.provider_contract_revision.len() <= 512
            && !self
                .provider_contract_revision
                .chars()
                .any(char::is_control)
            && self.cost_for_count(1, None).is_some()
    }

    pub(crate) fn cost(&self, usage: ToolUsage, discount: Option<DiscountRatio>) -> Option<i64> {
        if !self.valid() || !usage.valid() || usage.kind != self.kind {
            return None;
        }
        self.cost_for_count(usage.successful_requests, discount)
    }

    pub(crate) fn reservation(&self, discount: Option<DiscountRatio>) -> Option<i64> {
        if !self.valid() {
            return None;
        }
        self.cost_for_count(1, discount)
    }

    fn cost_for_count(&self, count: u8, discount: Option<DiscountRatio>) -> Option<i64> {
        let numerator = u128::from(self.usd_micros_per_request)
            .checked_mul(u128::from(self.group_multiplier_ppm))?
            .checked_mul(u128::from(self.speed_multiplier_ppm))?
            .checked_mul(u128::from(self.credits_per_usd))?
            .checked_mul(u128::from(count))?;
        let amount =
            crate::upstream_cost::ceil_with_discount(numerator, 1_000_000_u128.pow(3), discount)?;
        i64::try_from(amount).ok()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn price() -> PersistedToolPrice {
        PersistedToolPrice {
            kind: ToolKind::DeepSeekSearch,
            usd_micros_per_request: 750,
            group_multiplier_ppm: 1_000_000,
            speed_multiplier_ppm: 1_000_000,
            credits_per_usd: 500_000,
            provider_contract_revision: "synthetic-reviewed-v1".into(),
        }
    }

    #[test]
    fn search_charges_once_regardless_of_result_count_and_empty_costs_zero() {
        for body in [
            br#"{"results":[{"url":"https://example.invalid/a"}]}"#.as_slice(),
            br#"{"results":[{}, {}, {}]}"#.as_slice(),
        ] {
            let usage = ToolUsage::from_response(ToolKind::DeepSeekSearch, 200, body).unwrap();
            assert_eq!(usage.successful_requests, 1);
            assert_eq!(price().cost(usage, None), Some(375));
        }
        let empty =
            ToolUsage::from_response(ToolKind::DeepSeekSearch, 200, br#"{"results":[]}"#).unwrap();
        assert_eq!(price().cost(empty, None), Some(0));
        assert_eq!(price().reservation(None), Some(375));
    }

    #[test]
    fn fetch_requires_content_and_failures_are_not_successful_requests() {
        let kind = ToolKind::DeepSeekFetch;
        assert_eq!(
            ToolUsage::from_response(kind, 200, br#"{"content":"page"}"#)
                .unwrap()
                .successful_requests,
            1
        );
        assert_eq!(
            ToolUsage::from_response(kind, 200, br#"{"content":" "}"#)
                .unwrap()
                .successful_requests,
            0
        );
        for status in [400, 401, 402, 404, 429, 500, 504] {
            assert_eq!(
                ToolUsage::from_response(kind, status, b"failure")
                    .unwrap()
                    .successful_requests,
                0
            );
        }
        for (status, body) in [
            (200, b"{}".as_slice()),
            (200, b"{\"content\":null}"),
            (200, b"{"),
            (302, b"{}"),
        ] {
            assert!(ToolUsage::from_response(kind, status, body).is_none());
        }
    }

    #[test]
    fn frozen_tariff_retains_exact_group_rate_and_response_binding() {
        let mut tariff = price();
        tariff.usd_micros_per_request = 3;
        tariff.group_multiplier_ppm = 500_000;
        let usage = ToolUsage::from_response(ToolKind::DeepSeekSearch, 200, br#"{"results":[{}]}"#)
            .unwrap();
        assert_eq!(tariff.cost(usage, None), Some(1));
        let stored: PersistedToolPrice =
            serde_json::from_slice(&serde_json::to_vec(&tariff).unwrap()).unwrap();
        assert_eq!(stored, tariff);
        assert_eq!(stored.reservation(None), stored.cost(usage, None));
        let fetch =
            ToolUsage::from_response(ToolKind::DeepSeekFetch, 200, br#"{"content":"page"}"#)
                .unwrap();
        assert_eq!(stored.cost(fetch, None), None);
        let corrupt = ToolUsage {
            successful_requests: 2,
            ..usage
        };
        assert_eq!(stored.cost(corrupt, None), None);
    }
}

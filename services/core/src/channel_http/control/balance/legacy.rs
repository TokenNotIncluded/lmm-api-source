//! Explicit legacy type selection, never endpoint or protocol discovery.
//! Mapping evidence: parent 3948a0a1ee1cefdd016a7aa45a11851545bb4b85,
//! apps/api-go/{constant/channel.go,controller/channel-billing.go}.
use super::{BalanceAuth, BalanceProbe, BalanceRequirement, ManagementError};
use crate::routing::Protocol;
use serde::{Deserialize, Serialize};
use serde_json::json;

#[derive(Clone, Debug, PartialEq, Eq, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct LegacyBalance {
    pub channel_type: u32,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cny_per_usd: Option<String>,
}

impl LegacyBalance {
    pub(crate) fn validate(&self) -> Result<(), ManagementError> {
        if let Some(rate) = &self.cny_per_usd {
            if !matches!(self.channel_type, 25 | 43)
                || rate.len() > 32
                || rate.trim() != rate
                || !rate.parse::<f64>().is_ok_and(|n| n.is_finite() && n > 0.0)
            {
                return Err(ManagementError::Invalid);
            }
        }
        Ok(())
    }
}

/// Manual configuration wins unchanged. Unknown/retired types remain unsupported.
/// All paths are relative to the already configured channel origin. No fallback
/// to a hardcoded provider host or historical exchange-rate setting is allowed.
pub(super) fn resolve(
    manual: Option<&BalanceProbe>,
    legacy: Option<&LegacyBalance>,
    protocol: Protocol,
) -> Result<Option<BalanceProbe>, ManagementError> {
    if let Some(manual) = manual {
        return Ok(Some(manual.clone()));
    }
    let Some(legacy) = legacy else {
        return Ok(None);
    };
    legacy.validate()?;
    // The existing transport binds authentication to protocol. These two use
    // different headers; do not accidentally send them for a Bearer adapter.
    if matches!(protocol, Protocol::AnthropicMessages | Protocol::Gemini) {
        return Ok(None);
    }
    let (path, pointer, unit) = match legacy.channel_type {
        1 | 8 => (
            "/v1/dashboard/billing/subscription",
            "/hard_limit_usd",
            Some("USD"),
        ),
        10 => (
            "/api/report/getUserOverview",
            "/data/totalPoints",
            Some("points"),
        ),
        12 => ("/dashboard/billing/credit_grants", "/total_remaining", None),
        13 => ("/dashboard/billing/credit_grants", "/total_available", None),
        20 => ("/api/v1/credits", "/data/total_credits", Some("USD")),
        25 => (
            "/v1/users/me/balance",
            "/data/available_balance",
            Some(if legacy.cny_per_usd.is_some() {
                "USD"
            } else {
                "CNY"
            }),
        ),
        // The frozen adapter does not establish a currency for this scalar.
        40 => ("/v1/user/info", "/data/totalBalance", None),
        43 => ("/user/balance", "/balance_infos", Some("USD")),
        _ => return Ok(None),
    };
    let mut probe = BalanceProbe {
        path: path.into(),
        json_pointer: pointer.into(),
        unit_label: unit.map(str::to_owned),
        subtract_json_pointer: None,
        require: None,
        require_all: None,
        cny_per_usd: legacy.cny_per_usd.clone(),
        deepseek_currency_array: None,
        openai_subscription: None,
        auth: None,
    };
    let requirement = |pointer: &str, equals| BalanceRequirement {
        json_pointer: pointer.into(),
        equals,
    };
    match legacy.channel_type {
        1 | 8 => probe.openai_subscription = Some(true),
        10 => {
            probe.auth = Some(BalanceAuth::ApiKey);
            probe.require = Some(requirement("/success", json!(true)));
        }
        20 => probe.subtract_json_pointer = Some("/data/total_usage".into()),
        25 => {
            probe.require_all = Some(vec![
                requirement("/status", json!(true)),
                requirement("/code", json!(0)),
            ])
        }
        40 => probe.require = Some(requirement("/code", json!(20000))),
        43 => probe.deepseek_currency_array = Some(true),
        _ => {}
    }
    probe.validate()?;
    Ok(Some(probe))
}

#[cfg(test)]
mod tests;

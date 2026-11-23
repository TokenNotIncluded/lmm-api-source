//! Sub2API /v1/usage uses USD for wallet and key quota balances.
//! Contract: Wei-Shaw/sub2api gateway_handler.go (Usage), and
//! QuantumNous/new-api 63cecab824bad1a421d19950004c309bb2054c09.
use serde::Deserialize;
use serde_json::Value;

#[derive(Deserialize)]
struct Usage {
    #[serde(default)]
    remaining: Option<f64>,
    #[serde(default)]
    balance: Option<f64>,
    #[serde(default, rename = "isValid")]
    is_valid: Option<bool>,
    unit: String,
    #[serde(default)]
    mode: Option<String>,
    #[serde(default)]
    quota: Option<Quota>,
    #[serde(default)]
    subscription: Option<Subscription>,
    #[serde(default)]
    error: Option<Value>,
}

#[derive(Deserialize)]
struct Quota {
    unit: String,
}

#[derive(Deserialize)]
struct Subscription {
    #[serde(default)]
    daily_limit_usd: Option<f64>,
    #[serde(default)]
    weekly_limit_usd: Option<f64>,
    #[serde(default)]
    monthly_limit_usd: Option<f64>,
}

// Inspect decoded strings and object keys, including ignored provider metadata.
// A credential escaped as JSON must not be accepted as a successful observation.
fn contains_secret(value: &Value, secret: &str) -> bool {
    match value {
        Value::String(text) => text.contains(secret),
        Value::Array(items) => items.iter().any(|value| contains_secret(value, secret)),
        Value::Object(fields) => fields
            .iter()
            .any(|(key, value)| key.contains(secret) || contains_secret(value, secret)),
        _ => value.to_string().contains(secret),
    }
}

pub(super) fn balance(value: &Value, secret: &str) -> Option<f64> {
    if !value.is_object() || (!secret.is_empty() && contains_secret(value, secret)) {
        return None;
    }
    let usage: Usage = serde_json::from_value(value.clone()).ok()?;
    if usage.is_valid == Some(false)
        || usage.error.is_some()
        || usage.unit != "USD"
        || usage
            .mode
            .as_deref()
            .is_some_and(|mode| !matches!(mode, "unrestricted" | "quota_limited"))
        || usage
            .quota
            .as_ref()
            .is_some_and(|quota| quota.unit != "USD")
        || (usage.mode.as_deref() == Some("quota_limited") && usage.quota.is_none())
    {
        return None;
    }
    // Both official modes publish remaining. Older wallet responses use balance.
    // Missing, malformed or unlimited amounts never become a synthetic zero.
    let amount = if usage.quota.is_some() {
        usage.remaining?
    } else {
        usage.remaining.or(usage.balance)?
    };
    if !amount.is_finite() || (usage.quota.is_some() && amount < 0.0) {
        return None;
    }
    if let Some(subscription) = usage.subscription {
        let finite_limit = [
            subscription.daily_limit_usd,
            subscription.weekly_limit_usd,
            subscription.monthly_limit_usd,
        ]
        .into_iter()
        .flatten()
        .any(|limit| limit.is_finite() && limit > 0.0);
        if !finite_limit || amount < 0.0 {
            return None;
        }
    }
    Some(amount)
}

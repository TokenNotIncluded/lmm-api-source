//! Pancake subscription observations reuse the wallet's original RSA envelope
//! and decimal/payment identity rules. They never request wallet credit.
use super::{Channel, Error, PaymentsStore, Result, legacy_pancake, providers};
use serde::{Deserialize, Serialize};
use serde_json::Value;

mod decimal_id {
    use serde::{Deserialize, Deserializer, Serializer, de::Error};
    pub fn serialize<S: Serializer>(v: &i64, s: S) -> std::result::Result<S::Ok, S::Error> {
        s.serialize_str(&v.to_string())
    }
    pub fn deserialize<'de, D: Deserializer<'de>>(d: D) -> std::result::Result<i64, D::Error> {
        let value = Value::deserialize(d)?;
        let raw = match value {
            Value::String(s) => s,
            Value::Number(n) => n.to_string(),
            _ => return Err(D::Error::custom("decimal identifier required")),
        };
        let parsed: i64 = raw.parse().map_err(D::Error::custom)?;
        if parsed <= 0 || parsed.to_string() != raw {
            return Err(D::Error::custom("canonical positive identifier required"));
        }
        Ok(parsed)
    }
    use serde_json::Value;
}
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct PancakePrice {
    #[serde(with = "decimal_id")]
    pub plan_id: i64,
    #[serde(with = "decimal_id")]
    pub plan_revision: i64,
    #[serde(with = "decimal_id")]
    pub channel_id: i64,
    pub currency: String,
    pub amount_minor: String,
    pub store_id: String,
    pub product_id: String,
    pub product_type: String,
    pub billing_period: String,
    pub provider_plan_reference: String,
    pub return_url: String,
    pub checkout_region: String,
    pub checkout_language: String,
    pub tax_category: String,
}
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct PancakeBinding {
    pub merchant: String,
    pub environment: String,
    pub price: PancakePrice,
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Change {
    Paid,
    Checkout,
    State,
}
#[derive(Clone, Debug)]
pub struct Verified {
    pub event_id: String,
    pub provider_order_id: String,
    pub payment_id: Option<String>,
    pub kind: Change,
    pub state: String,
    pub created: i64,
    pub period_start: i64,
    pub period_end: i64,
}
fn text<'a>(v: &'a Value, k: &str) -> Result<&'a str> {
    v.get(k).and_then(Value::as_str).ok_or(Error::Invalid)
}
fn identifier(v: &Value, k: &str) -> Result<String> {
    let s = text(v, k)?;
    if !super::valid_id(s) {
        return Err(Error::Invalid);
    }
    Ok(s.into())
}
fn date(raw: &str) -> Result<i64> {
    if let Ok(t) = chrono::DateTime::parse_from_rfc3339(raw) {
        return Ok(t.timestamp());
    }
    let day = chrono::NaiveDate::parse_from_str(raw, "%Y-%m-%d").map_err(|_| Error::Invalid)?;
    Ok(day
        .and_hms_opt(0, 0, 0)
        .ok_or(Error::Invalid)?
        .and_utc()
        .timestamp())
}
pub(crate) fn raw_event(raw: &[u8]) -> Result<Value> {
    super::stripe::unique_json(raw)
}
impl PaymentsStore {
    pub(crate) fn pancake_subscription_binding(
        &self,
        price: PancakePrice,
    ) -> Result<PancakeBinding> {
        let c = self
            .channels
            .get(&price.channel_id)
            .ok_or(Error::Unavailable)?;
        if c.provider != "waffo_pancake" || c.currency != price.currency {
            return Err(Error::Conflict);
        }
        let amount = price
            .amount_minor
            .parse::<i64>()
            .map_err(|_| Error::Invalid)?;
        if amount <= 0 || amount > c.max_amount_minor {
            return Err(Error::Invalid);
        }
        if price.store_id != c.verification.store_id || !super::valid_id(&price.product_id) {
            return Err(Error::Invalid);
        }
        Ok(PancakeBinding {
            merchant: c.merchant.clone(),
            environment: c.environment.clone(),
            price,
        })
    }
    pub(crate) fn verify_pancake_subscription(
        &self,
        b: &PancakeBinding,
        reference: &str,
        bound_provider_order: Option<&str>,
        quoted_at: i64,
        raw: &[u8],
        signature: &[u8],
        now: i64,
    ) -> Result<(Channel, Verified)> {
        let original = self
            .channels
            .get(&b.price.channel_id)
            .ok_or(Error::Unavailable)?;
        if original.provider != "waffo_pancake"
            || original.merchant != b.merchant
            || original.environment != b.environment
            || original.currency != b.price.currency
        {
            return Err(Error::Conflict);
        }
        let mut channel = original.clone();
        // Product changes affect new purchases, not an original frozen SKU. Preserve
        // the original store/product while using the configured original key ring.
        channel.verification.store_id = b.price.store_id.clone();
        channel.verification.product_id = b.price.product_id.clone();
        let header = std::str::from_utf8(signature).map_err(|_| Error::Invalid)?;
        let signed_ms = header
            .split(',')
            .find_map(|v| v.trim().strip_prefix("t="))
            .ok_or(Error::Invalid)?
            .parse::<i64>()
            .map_err(|_| Error::Invalid)?;
        let signed = signed_ms.div_euclid(1000);
        if signed < quoted_at.saturating_sub(300) || signed > now.saturating_add(60) {
            return Err(Error::Invalid);
        }
        // An independently authenticated timestamp and frozen original order make a
        // durable candidate recoverable after an outage. The shared verifier still
        // checks the exact original RSA bytes and rejects duplicate header fields.
        let event = providers::pancake_envelope(raw, signature, &channel, signed)?;
        let data = event
            .get("data")
            .filter(|v| v.is_object())
            .ok_or(Error::Invalid)?;
        let mode = if b.environment == "live" {
            "prod"
        } else {
            "test"
        };
        if text(&event, "mode")? != mode
            || text(&event, "storeId")? != b.price.store_id
            || data
                .get("orderMerchantExternalId")
                .and_then(Value::as_str)
                .filter(|v| !v.is_empty())
                .is_some_and(|v| v != reference)
            || (data
                .get("orderMerchantExternalId")
                .and_then(Value::as_str)
                .is_none_or(|v| v.is_empty())
                && bound_provider_order != data.get("orderId").and_then(Value::as_str))
        {
            return Err(Error::Invalid);
        }
        let metadata = data.get("orderMetadata").ok_or(Error::Invalid)?;
        if text(metadata, "lmm_product_id")? != b.price.product_id
            || text(metadata, "lmm_plan_id")? != b.price.provider_plan_reference
        {
            return Err(Error::Invalid);
        }
        let event_id = if event
            .get("id")
            .and_then(Value::as_str)
            .is_some_and(|v| !v.is_empty())
        {
            identifier(&event, "id")?
        } else {
            identifier(&event, "eventId")?
        };
        let kind = text(&event, "eventType")?;
        let created = event
            .get("timestamp")
            .and_then(Value::as_str)
            .map(date)
            .transpose()?
            .unwrap_or(0);
        if created > signed.saturating_add(60) {
            return Err(Error::Invalid);
        }
        let provider_order_id = identifier(data, "orderId")?;
        let mut v = Verified {
            event_id,
            provider_order_id,
            payment_id: None,
            kind: Change::State,
            state: String::new(),
            created,
            period_start: 0,
            period_end: 0,
        };
        match kind {
            "order.completed" | "subscription.payment_succeeded" => {
                let recurring = b.price.product_type == "subscription";
                if kind == "order.completed" && recurring {
                    v.kind = Change::Checkout;
                    return Ok((channel, v));
                }
                if kind == "subscription.payment_succeeded" && !recurring {
                    return Err(Error::Invalid);
                }
                if !text(data, "currency")?.eq_ignore_ascii_case(&b.price.currency) {
                    return Err(Error::Invalid);
                }
                let amount = b
                    .price
                    .amount_minor
                    .parse::<i64>()
                    .map_err(|_| Error::Invalid)?
                    .checked_mul(10000)
                    .ok_or(Error::Invalid)?;
                let charged = match data.get("chargedAmount").filter(|v| !v.is_null()) {
                    Some(v) => v.as_str().ok_or(Error::Invalid)?,
                    None if data.get("listPrice").is_some_and(|v| !v.is_null()) => {
                        return Err(Error::Invalid);
                    }
                    None => text(data, "amount")?,
                };
                if legacy_pancake::decimal_micros(charged)? != amount {
                    return Err(Error::Invalid);
                }
                if data
                    .get("paymentStatus")
                    .and_then(Value::as_str)
                    .is_some_and(|v| !v.is_empty() && v != "succeeded")
                {
                    return Err(Error::Invalid);
                }
                if kind == "order.completed"
                    && data
                        .get("orderStatus")
                        .and_then(Value::as_str)
                        .is_some_and(|v| !v.is_empty() && v != "completed")
                {
                    return Err(Error::Invalid);
                }
                v.payment_id = match data
                    .get("paymentId")
                    .filter(|v| !v.is_null() && v.as_str() != Some(""))
                {
                    Some(_) => Some(identifier(data, "paymentId")?),
                    None => None,
                };
                if recurring {
                    // The signed billing-period pair is retained separately from the
                    // optional payment ID. Never manufacture a provider payment identifier.
                    if data
                        .get("periodNumber")
                        .and_then(Value::as_i64)
                        .is_some_and(|n| n <= 0)
                    {
                        return Err(Error::Invalid);
                    }
                    v.period_start = date(text(data, "currentPeriodStart")?)?;
                    v.period_end = date(text(data, "currentPeriodEnd")?)?;
                    if v.period_start <= 0
                        || v.period_end <= v.period_start
                        || data
                            .get("billingPeriod")
                            .and_then(Value::as_str)
                            .is_some_and(|v| !v.is_empty() && v != b.price.billing_period)
                    {
                        return Err(Error::Invalid);
                    }
                }
                v.kind = Change::Paid;
                v.state = "active".into();
            }
            "subscription.activated"
            | "subscription.renewed"
            | "subscription.recovered"
            | "subscription.uncanceled"
            | "subscription.canceling"
            | "subscription.updated"
            | "subscription.past_due"
            | "subscription.canceled"
            | "subscription.plan_changed"
            | "subscription.plan_change_scheduled"
            | "subscription.plan_change_failed" => {
                if b.price.product_type != "subscription" || created <= 0 {
                    return Err(Error::Invalid);
                }
                v.state = match kind {
                    "subscription.canceling" => "canceling",
                    "subscription.past_due" => "past_due",
                    "subscription.canceled" => "canceled",
                    "subscription.activated"
                    | "subscription.renewed"
                    | "subscription.recovered"
                    | "subscription.uncanceled" => "active",
                    _ => text(data, "orderStatus")?,
                }
                .into();
                if !matches!(
                    v.state.as_str(),
                    "active" | "canceling" | "past_due" | "canceled"
                ) {
                    return Err(Error::Invalid);
                }
            }
            _ => return Err(Error::Unavailable),
        }
        Ok((channel, v))
    }
}

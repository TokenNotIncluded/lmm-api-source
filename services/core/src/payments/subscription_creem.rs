//! Native Creem signed subscription evidence. Only subscription.paid grants;
//! checkout and lifecycle events bind identity without creating wallet money.
use super::{Channel, Error, PaymentsStore, Result, stripe};
use hmac::{Hmac, KeyInit as _, Mac};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use sha2::Sha256;

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct CreemPrice {
    #[serde(with = "decimal_i64")]
    pub plan_id: i64,
    #[serde(with = "decimal_i64")]
    pub plan_revision: i64,
    #[serde(with = "decimal_i64")]
    pub channel_id: i64,
    pub currency: String,
    pub amount_minor: String,
    pub provider_price_id: String,
    pub interval: String,
    #[serde(with = "decimal_i64")]
    pub interval_count: i64,
    pub success_url: String,
    pub cancel_url: String,
}
mod decimal_i64 {
    use serde::{Deserialize, Deserializer, Serializer, de::Error};
    pub fn serialize<S: Serializer>(v: &i64, s: S) -> Result<S::Ok, S::Error> {
        s.serialize_str(&v.to_string())
    }
    pub fn deserialize<'de, D: Deserializer<'de>>(d: D) -> Result<i64, D::Error> {
        #[derive(Deserialize)]
        #[serde(untagged)]
        enum Input {
            Text(String),
            Number(i64),
        }
        match Input::deserialize(d)? {
            Input::Number(n) => Ok(n),
            Input::Text(s) => {
                let n = s.parse::<i64>().map_err(D::Error::custom)?;
                if n.to_string() != s {
                    return Err(D::Error::custom("canonical decimal required"));
                }
                Ok(n)
            }
        }
    }
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Binding {
    pub merchant: String,
    pub environment: String,
    pub price: CreemPrice,
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Change {
    Checkout,
    Paid,
    Failed,
    State,
}
#[derive(Clone, Debug)]
pub struct Verified {
    pub event_id: String,
    pub created: i64,
    pub change: Change,
    pub subscription_id: String,
    pub customer_id: String,
    pub transaction_id: String,
    pub checkout_id: String,
    pub period_start: i64,
    pub period_end: i64,
    pub state: String,
}
fn text<'a>(v: &'a Value, key: &str) -> Result<&'a str> {
    v.get(key).and_then(Value::as_str).ok_or(Error::Invalid)
}
fn reference(v: &Value) -> Option<&str> {
    v.as_str().or_else(|| v.get("id").and_then(Value::as_str))
}
fn id(s: &str, prefix: &str) -> bool {
    s.starts_with(prefix)
        && s.len() > prefix.len()
        && s.len() <= 128
        && s.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'_')
}
fn date(v: &Value, field: &str) -> Result<i64> {
    chrono::DateTime::parse_from_rfc3339(text(v, field)?)
        .map(|d| d.timestamp())
        .map_err(|_| Error::Invalid)
}
pub(crate) fn raw_event(raw: &[u8]) -> Result<Value> {
    stripe::unique_json(raw)
}
pub(crate) fn billing_period(p: &CreemPrice) -> Option<&'static str> {
    match (p.interval.as_str(), p.interval_count) {
        ("month", 1) => Some("every-month"),
        ("month", 3) => Some("every-three-months"),
        ("month", 6) => Some("every-six-months"),
        ("year", 1) => Some("every-year"),
        _ => None,
    }
}
impl PaymentsStore {
    pub(crate) fn creem_subscription_channel(&self, channel: i64) -> Result<&Channel> {
        let c = self.channels.get(&channel).ok_or(Error::Unavailable)?;
        if c.provider != "creem" || c.currency != "USD" {
            return Err(Error::Conflict);
        }
        Ok(c)
    }
    pub(crate) fn creem_subscription_binding(&self, price: CreemPrice) -> Result<Binding> {
        let c = self.creem_subscription_channel(price.channel_id)?;
        if price.currency != c.currency
            || !id(&price.provider_price_id, "prod_")
            || billing_period(&price).is_none()
        {
            return Err(Error::Invalid);
        }
        Ok(Binding {
            merchant: c.merchant.clone(),
            environment: c.environment.clone(),
            price,
        })
    }
    pub(crate) fn verify_creem_subscription(
        &self,
        binding: &Binding,
        order: &str,
        known_subscription: Option<&str>,
        quoted: i64,
        raw: &[u8],
        signature: &[u8],
        now: i64,
    ) -> Result<(Channel, Verified)> {
        if raw.is_empty() || raw.len() > 32768 || signature.len() != 64 {
            return Err(Error::Invalid);
        }
        let c = self.creem_subscription_channel(binding.price.channel_id)?;
        if c.merchant != binding.merchant
            || c.environment != binding.environment
            || c.currency != binding.price.currency
        {
            return Err(Error::Conflict);
        }
        let sig = crate::hex_codec::decode(signature).map_err(|_| Error::Denied)?;
        if !c.webhook_secrets.iter().any(|secret| {
            let Ok(mut mac) = Hmac::<Sha256>::new_from_slice(secret.as_bytes()) else {
                return false;
            };
            mac.update(raw);
            mac.verify_slice(&sig).is_ok()
        }) {
            return Err(Error::Denied);
        }
        let e = raw_event(raw)?;
        let event_id = text(&e, "id")?;
        // Creem created_at is milliseconds; its raw-body HMAC covers the time.
        let created_ms = e
            .get("created_at")
            .and_then(Value::as_i64)
            .ok_or(Error::Invalid)?;
        let created = created_ms / 1000;
        if !id(event_id, "evt_")
            || created_ms <= 0
            || created < quoted.saturating_sub(300)
            || created > now.saturating_add(300)
        {
            return Err(Error::Denied);
        }
        let o = e.get("object").ok_or(Error::Invalid)?;
        let kind = text(&e, "eventType")?;
        let mode = if binding.environment == "live" {
            "prod"
        } else {
            "test"
        };
        if text(o, "mode")? != mode {
            return Err(Error::Denied);
        }
        let mut v = Verified {
            event_id: event_id.into(),
            created: created_ms,
            change: Change::State,
            subscription_id: String::new(),
            customer_id: String::new(),
            transaction_id: String::new(),
            checkout_id: String::new(),
            period_start: 0,
            period_end: 0,
            state: String::new(),
        };
        let meta;
        if kind == "checkout.completed" {
            if text(o, "object")? != "checkout"
                || text(o, "status")? != "completed"
                || text(o, "request_id")? != order
                || !id(text(o, "id")?, "ch_")
            {
                return Err(Error::Denied);
            }
            let sub = &o["subscription"];
            v.subscription_id = reference(sub).ok_or(Error::Invalid)?.into();
            v.customer_id = reference(&o["customer"]).ok_or(Error::Invalid)?.into();
            v.checkout_id = text(o, "id")?.into();
            v.change = Change::Checkout;
            if reference(&o["product"]) != Some(binding.price.provider_price_id.as_str()) {
                return Err(Error::Conflict);
            }
            let pay = &o["order"];
            if text(pay, "type")? != "recurring"
                || text(pay, "status")? != "paid"
                || text(pay, "mode")? != mode
                || pay.get("amount_paid").and_then(Value::as_i64)
                    != binding.price.amount_minor.parse::<i64>().ok()
                || !text(pay, "currency")?.eq_ignore_ascii_case(&binding.price.currency)
            {
                return Err(Error::Conflict);
            }
            meta = &o["metadata"];
        } else {
            if text(o, "object")? != "subscription"
                || !matches!(
                    kind,
                    "subscription.paid"
                        | "subscription.active"
                        | "subscription.update"
                        | "subscription.canceled"
                        | "subscription.expired"
                        | "subscription.paused"
                        | "subscription.trialing"
                )
            {
                return Err(Error::Invalid);
            }
            v.subscription_id = text(o, "id")?.into();
            v.customer_id = reference(&o["customer"]).ok_or(Error::Invalid)?.into();
            if reference(&o["product"]) != Some(binding.price.provider_price_id.as_str()) {
                return Err(Error::Conflict);
            }
            // Reconfiguration must not silently change frozen subscription price.
            let p = &o["product"];
            if p.get("mode").is_some_and(|m| m.as_str() != Some(mode)) {
                return Err(Error::Conflict);
            }
            v.state = match kind {
                "subscription.canceled" => "canceled",
                "subscription.expired" => "expired",
                "subscription.paused" => "paused",
                "subscription.trialing" => "trialing",
                _ => text(o, "status")?,
            }
            .into();
            if !matches!(
                v.state.as_str(),
                "active"
                    | "unpaid"
                    | "paused"
                    | "canceled"
                    | "trialing"
                    | "scheduled_cancel"
                    | "expired"
            ) {
                return Err(Error::Invalid);
            }
            if v.state == "scheduled_cancel" {
                v.state = "canceling".into();
            }
            if kind == "subscription.paid" {
                if text(o, "status")? != "active"
                    || text(o, "collection_method")? != "charge_automatically"
                    || text(p, "billing_type")? != "recurring"
                    || text(p, "billing_period")?
                        != billing_period(&binding.price).ok_or(Error::Invalid)?
                    || !text(p, "currency")?.eq_ignore_ascii_case(&binding.price.currency)
                    || p.get("price").and_then(Value::as_i64)
                        != binding.price.amount_minor.parse::<i64>().ok()
                {
                    return Err(Error::Conflict);
                }
                v.transaction_id = text(o, "last_transaction_id")?.into();
                if !id(&v.transaction_id, "tran_") {
                    return Err(Error::Invalid);
                }
                v.period_start = date(o, "current_period_start_date")?;
                v.period_end = date(o, "current_period_end_date")?;
                if v.period_start <= 0
                    || v.period_end <= v.period_start
                    || v.period_end > 253402300799
                {
                    return Err(Error::Invalid);
                }
                v.change = Change::Paid;
            } else if matches!(v.state.as_str(), "unpaid" | "expired") {
                v.change = Change::Failed;
                v.period_end = o
                    .get("current_period_end_date")
                    .and_then(Value::as_str)
                    .and_then(|s| chrono::DateTime::parse_from_rfc3339(s).ok())
                    .map_or(0, |d| d.timestamp());
            }
            meta = &o["metadata"];
        }
        if !id(&v.subscription_id, "sub_") || !id(&v.customer_id, "cust_") {
            return Err(Error::Invalid);
        }
        if let Some(claim) = meta.get("lmm_subscription_order_id") {
            if claim.as_str() != Some(order)
                || meta.get("lmm_merchant").and_then(Value::as_str)
                    != Some(binding.merchant.as_str())
                || meta.get("lmm_price_id").and_then(Value::as_str)
                    != Some(binding.price.provider_price_id.as_str())
            {
                return Err(Error::Denied);
            }
        } else if known_subscription != Some(v.subscription_id.as_str()) {
            return Err(Error::Denied);
        }
        Ok((c.clone(), v))
    }
}

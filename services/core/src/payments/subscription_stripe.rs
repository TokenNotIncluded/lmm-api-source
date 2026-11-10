//! Stripe subscription evidence shares the wallet HMAC/unique-JSON verifier.
//! Checkout/status never grants. Only an exact original paid invoice can grant.
use super::{Channel, Error, PaymentsStore, Result, stripe};
use serde::{Deserialize, Serialize};
use serde_json::Value;

pub const STRIPE_SUBSCRIPTION_API_VERSION: &str = "2024-10-28.acacia";

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct StripePrice {
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
    pub fn serialize<S: Serializer>(value: &i64, s: S) -> Result<S::Ok, S::Error> {
        s.serialize_str(&value.to_string())
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
                    return Err(D::Error::custom("canonical decimal ID required"));
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
    pub price: StripePrice,
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Change {
    Checkout,
    Expired,
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
    pub invoice_id: String,
    pub payment_id: String,
    pub checkout_id: String,
    pub period_start: i64,
    pub period_end: i64,
    pub state: String,
}
pub fn reference(v: &Value) -> Option<&str> {
    v.as_str().or_else(|| v.get("id").and_then(Value::as_str))
}
fn text<'a>(v: &'a Value, key: &str) -> Result<&'a str> {
    v.get(key).and_then(Value::as_str).ok_or(Error::Invalid)
}
fn number(v: &Value, key: &str) -> Result<i64> {
    v.get(key).and_then(Value::as_i64).ok_or(Error::Invalid)
}
fn id(s: &str, prefix: &str) -> bool {
    s.starts_with(prefix)
        && s.len() > prefix.len()
        && s.len() <= 128
        && s.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'_')
}
fn metadata<'a>(o: &'a Value) -> Option<&'a Value> {
    o.get("subscription_details")
        .and_then(|s| s.get("metadata"))
        .or_else(|| o.pointer("/parent/subscription_details/metadata"))
}
pub(crate) fn raw_event(raw: &[u8]) -> Result<Value> {
    stripe::unique_json(raw)
}
impl PaymentsStore {
    pub(crate) fn stripe_subscription_channel(&self, channel: i64) -> Result<&Channel> {
        let c = self.channels.get(&channel).ok_or(Error::Unavailable)?;
        if c.provider != "stripe" || c.currency != "USD" {
            return Err(Error::Conflict);
        }
        Ok(c)
    }
    pub(crate) fn stripe_subscription_binding(&self, price: StripePrice) -> Result<Binding> {
        let c = self.stripe_subscription_channel(price.channel_id)?;
        if price.currency != c.currency || !id(&price.provider_price_id, "price_") {
            return Err(Error::Invalid);
        }
        Ok(Binding {
            merchant: c.merchant.clone(),
            environment: c.environment.clone(),
            price,
        })
    }
    pub(crate) fn verify_stripe_subscription(
        &self,
        binding: &Binding,
        order: &str,
        quoted: i64,
        raw: &[u8],
        signature: &[u8],
        now: i64,
    ) -> Result<(Channel, Verified)> {
        if raw.is_empty() || raw.len() > 32768 || signature.is_empty() || signature.len() > 2048 {
            return Err(Error::Invalid);
        }
        let c = self.stripe_subscription_channel(binding.price.channel_id)?;
        if c.merchant != binding.merchant
            || c.environment != binding.environment
            || c.currency != binding.price.currency
        {
            return Err(Error::Conflict);
        }
        // Durable original evidence can outlive a delivery's 5-minute timestamp.
        // Verify the SAME HMAC at its authenticated timestamp, then bind it to a
        // confirmed random Core order and immutable event/invoice/period keys.
        // This is intentionally separate from wallet fresh-ingress semantics.
        let header = std::str::from_utf8(signature).map_err(|_| Error::Denied)?;
        let signed_at = header
            .split(',')
            .find_map(|p| p.trim().strip_prefix("t="))
            .ok_or(Error::Denied)?
            .parse::<i64>()
            .map_err(|_| Error::Denied)?;
        if signed_at < quoted.saturating_sub(300) || signed_at > now.saturating_add(300) {
            return Err(Error::Denied);
        }
        stripe::verify_signature(raw, signature, &c.webhook_secrets, signed_at)?;
        let envelope = stripe::unique_json(raw)?;
        // This slice deliberately pins the invoice/PaymentIntent object model.
        // A differently versioned endpoint is configuration to repair, never
        // evidence that a real paid callback was invalid. Keep it recoverable.
        if envelope.get("api_version").and_then(Value::as_str)
            != Some(STRIPE_SUBSCRIPTION_API_VERSION)
        {
            return Err(Error::Unavailable);
        }
        let event_id = text(&envelope, "id")?;
        let created = number(&envelope, "created")?;
        if !id(event_id, "evt_")
            || created <= 0
            || created > signed_at.saturating_add(300)
            || envelope.get("livemode").and_then(Value::as_bool)
                != Some(binding.environment == "live")
            || envelope
                .get("account")
                .and_then(Value::as_str)
                .is_some_and(|a| a != binding.merchant)
        {
            return Err(Error::Denied);
        }
        let o = envelope.pointer("/data/object").ok_or(Error::Invalid)?;
        if o.get("livemode")
            .and_then(Value::as_bool)
            .is_some_and(|v| v != (binding.environment == "live"))
        {
            return Err(Error::Denied);
        }
        let kind = text(&envelope, "type")?;
        let mut v = Verified {
            event_id: event_id.into(),
            created,
            change: Change::State,
            subscription_id: String::new(),
            customer_id: String::new(),
            invoice_id: String::new(),
            payment_id: String::new(),
            checkout_id: String::new(),
            period_start: 0,
            period_end: 0,
            state: String::new(),
        };
        let meta;
        match kind {
            "checkout.session.completed"
            | "checkout.session.async_payment_succeeded"
            | "checkout.session.expired" => {
                if text(o, "object")? != "checkout.session"
                    || text(o, "mode")? != "subscription"
                    || text(o, "client_reference_id")? != order
                    || !id(text(o, "id")?, "cs_")
                {
                    return Err(Error::Denied);
                }
                v.checkout_id = text(o, "id")?.into();
                v.change = if kind == "checkout.session.expired" {
                    Change::Expired
                } else {
                    Change::Checkout
                };
                if v.change == Change::Expired {
                    if text(o, "status")? != "expired" {
                        return Err(Error::Invalid);
                    }
                } else {
                    if text(o, "status")? != "complete" {
                        return Err(Error::Invalid);
                    }
                    v.subscription_id = reference(&o["subscription"]).ok_or(Error::Invalid)?.into();
                    v.customer_id = reference(&o["customer"]).ok_or(Error::Invalid)?.into();
                }
                meta = &o["metadata"];
            }
            "invoice.paid" | "invoice.payment_succeeded" | "invoice.payment_failed" => {
                if text(o, "object")? != "invoice" || !id(text(o, "id")?, "in_") {
                    return Err(Error::Invalid);
                }
                v.invoice_id = text(o, "id")?.into();
                v.subscription_id = reference(&o["subscription"])
                    .or_else(|| reference(&o["parent"]["subscription_details"]["subscription"]))
                    .ok_or(Error::Invalid)?
                    .into();
                v.customer_id = reference(&o["customer"]).ok_or(Error::Invalid)?.into();
                let expected = binding
                    .price
                    .amount_minor
                    .parse::<i64>()
                    .map_err(|_| Error::Invalid)?;
                if !matches!(
                    text(o, "billing_reason")?,
                    "subscription_create" | "subscription_cycle"
                ) {
                    return Err(Error::Denied);
                }
                if !text(o, "currency")?.eq_ignore_ascii_case(&binding.price.currency)
                    || number(o, "total")? != expected
                    || number(o, "amount_due")? != expected
                {
                    return Err(Error::Denied);
                }
                let lines = o
                    .pointer("/lines/data")
                    .and_then(Value::as_array)
                    .ok_or(Error::Invalid)?;
                if o.pointer("/lines/has_more").and_then(Value::as_bool) != Some(false)
                    || lines.len() != 1
                {
                    return Err(Error::Denied);
                }
                let line = &lines[0];
                let price = reference(&line["price"])
                    .or_else(|| reference(&line["pricing"]["price_details"]["price"]));
                if price != Some(binding.price.provider_price_id.as_str())
                    || !text(line, "currency")?.eq_ignore_ascii_case(&binding.price.currency)
                    || number(line, "quantity")? != 1
                    || line.get("proration").and_then(Value::as_bool) == Some(true)
                    || line
                        .pointer("/parent/subscription_item_details/proration")
                        .and_then(Value::as_bool)
                        == Some(true)
                    || number(line, "amount")? != expected
                {
                    return Err(Error::Denied);
                }
                v.period_start = number(&line["period"], "start")?;
                v.period_end = number(&line["period"], "end")?;
                if v.period_start <= 0 || v.period_end <= v.period_start {
                    return Err(Error::Invalid);
                }
                meta = metadata(o).ok_or(Error::Denied)?;
                if kind == "invoice.payment_failed" {
                    if !matches!(text(o, "status")?, "open" | "uncollectible") {
                        return Err(Error::Invalid);
                    }
                    v.change = Change::Failed;
                    v.state = "past_due".into();
                } else {
                    if text(o, "status")? != "paid"
                        || o.get("paid_out_of_band").and_then(Value::as_bool) != Some(false)
                        || number(o, "amount_paid")? != expected
                        || number(o, "amount_remaining")? != 0
                    {
                        return Err(Error::Denied);
                    }
                    // Acacia original PaymentIntent, or a Basil invoice payment
                    // list explicitly expanded on the webhook endpoint.
                    v.payment_id = reference(&o["payment_intent"])
                        .or_else(|| {
                            reference(&o["payments"]["data"][0]["payment"]["payment_intent"])
                        })
                        .ok_or(Error::Invalid)?
                        .into();
                    if !id(&v.payment_id, "pi_") {
                        return Err(Error::Invalid);
                    }
                    v.change = Change::Paid;
                    v.state = "active".into();
                }
            }
            "customer.subscription.created"
            | "customer.subscription.updated"
            | "customer.subscription.deleted" => {
                if text(o, "object")? != "subscription" {
                    return Err(Error::Invalid);
                }
                v.subscription_id = text(o, "id")?.into();
                v.customer_id = reference(&o["customer"]).ok_or(Error::Invalid)?.into();
                v.state = if kind == "customer.subscription.deleted" {
                    "canceled".into()
                } else {
                    text(o, "status")?.into()
                };
                if !matches!(
                    v.state.as_str(),
                    "incomplete"
                        | "incomplete_expired"
                        | "trialing"
                        | "active"
                        | "past_due"
                        | "unpaid"
                        | "paused"
                        | "canceled"
                ) {
                    return Err(Error::Invalid);
                }
                if v.state == "active"
                    && o.get("cancel_at_period_end").and_then(Value::as_bool) == Some(true)
                {
                    v.state = "canceling".into();
                }
                meta = &o["metadata"];
            }
            _ => return Err(Error::Invalid),
        }
        if meta
            .get("lmm_subscription_order_id")
            .and_then(Value::as_str)
            != Some(order)
            || meta.get("lmm_merchant").and_then(Value::as_str) != Some(binding.merchant.as_str())
            || meta.get("lmm_price_id").and_then(Value::as_str)
                != Some(binding.price.provider_price_id.as_str())
        {
            return Err(Error::Denied);
        }
        if v.change != Change::Expired
            && (!id(&v.subscription_id, "sub_") || !id(&v.customer_id, "cus_"))
        {
            return Err(Error::Invalid);
        }
        Ok((c.clone(), v))
    }
}

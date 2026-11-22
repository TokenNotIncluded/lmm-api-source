//! Read-only views of the actual shared payment configuration. No quote, intent,
//! provider request or configuration reload is performed by this projection.
use super::{Channel, CreditQuotePolicy, PaymentsStore, Result};
use serde::Serialize;
use sha2::{Digest, Sha256};

#[derive(Clone)]
pub(crate) struct OfferSource {
    pub(crate) store: PaymentsStore,
    pub(crate) topup_prepare: bool,
    pub(crate) subscription_orders: bool,
}
#[derive(Serialize)]
pub(crate) struct ChannelOffer {
    pub channel_id: i64,
    pub provider: String,
    pub environment: String,
    pub currency: String,
    pub payment_type: String,
    pub binding_sha256: String,
    pub configured: bool,
    pub min_amount_minor: String,
    pub max_amount_minor: String,
    pub credits_per_minor: String,
    pub credit_quote: Option<CreditQuotePolicy>,
}
impl OfferSource {
    pub(crate) fn freeze(&self) -> Result<Self> {
        Ok(Self {
            store: self.store.freeze()?,
            topup_prepare: self.topup_prepare,
            subscription_orders: self.subscription_orders,
        })
    }
    pub(crate) fn channel(&self, id: i64) -> Option<&Channel> {
        self.store.channels.get(&id)
    }
    pub(crate) fn topups(&self) -> Vec<ChannelOffer> {
        self.store
            .channels
            .values()
            .map(|c| ChannelOffer {
                channel_id: c.id,
                provider: c.provider.clone(),
                environment: c.environment.clone(),
                currency: c.currency.clone(),
                payment_type: c.verification.payment_type.clone(),
                binding_sha256: binding(c, false),
                configured: self.topup_prepare
                    && c.enabled()
                    && !(c.provider == "creem" && c.verification.product_id.is_empty()),
                min_amount_minor: c.min_amount_minor().to_string(),
                max_amount_minor: c.max_amount_minor.to_string(),
                credits_per_minor: c.credits_per_minor.to_string(),
                credit_quote: c.credit_quote.clone(),
            })
            .collect()
    }
}
/// Internal Core/Go configuration comparison only. This is not an authorization
/// receipt and is stripped before a tool result is returned to the model.
pub(crate) fn binding(c: &Channel, subscription: bool) -> String {
    let mut hash = Sha256::new();
    hash.update(b"assistant.checkout-binding.v1\0");
    let id = c.id.to_string();
    let purpose = if subscription {
        "subscription"
    } else {
        "topup"
    };
    let payment_type = if c.provider == "epay" {
        c.verification.payment_type.as_str()
    } else {
        ""
    };
    let store = if c.provider == "waffo_pancake" {
        c.verification.store_id.as_str()
    } else {
        ""
    };
    let product = if !subscription && matches!(c.provider.as_str(), "waffo_pancake" | "creem") {
        c.verification.product_id.as_str()
    } else {
        ""
    };
    for field in [
        purpose,
        id.as_str(),
        &c.provider,
        &c.merchant,
        &c.environment,
        &c.currency,
        payment_type,
        store,
        product,
    ] {
        hash.update((field.len() as u32).to_be_bytes());
        hash.update(field.as_bytes());
    }
    crate::hex_codec::encode(hash.finalize())
}

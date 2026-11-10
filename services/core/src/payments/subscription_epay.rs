//! Reuse the Core EPay protocol and private key for frozen subscription orders.
use super::*;
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct Binding {
    pub channel_id: i64,
    pub merchant: String,
    pub environment: String,
    pub currency: String,
    pub payment_type: String,
    pub endpoint: String,
    pub notify_url: String,
    pub return_url: String,
}
impl PaymentsStore {
    pub(crate) fn subscription_channel(&self, id: i64) -> Result<&Channel> {
        let c = self.channels.get(&id).ok_or(Error::Unavailable)?;
        if c.provider != "epay" || !matches!(c.currency.as_str(), "CNY" | "LDC") {
            return Err(Error::Invalid);
        }
        Ok(c)
    }
    pub(crate) fn subscription_binding(&self, id: i64) -> Result<Binding> {
        let c = self.subscription_channel(id)?;
        let mut notify =
            reqwest::Url::parse(&c.verification.notify_url).map_err(|_| Error::Invalid)?;
        let suffix = format!("/callbacks/{id}");
        let prefix = notify
            .path()
            .strip_suffix(&suffix)
            .ok_or(Error::Invalid)?
            .to_owned();
        if notify.query().is_some() || notify.fragment().is_some() {
            return Err(Error::Invalid);
        }
        notify.set_path(&format!("{prefix}/callbacks/subscriptions/{id}"));
        Ok(Binding {
            channel_id: id,
            merchant: c.merchant.clone(),
            environment: c.environment.clone(),
            currency: c.currency.clone(),
            payment_type: c.verification.payment_type.clone(),
            endpoint: c.verification.endpoint.clone(),
            notify_url: notify.into(),
            return_url: c.verification.return_url.clone(),
        })
    }
    pub(crate) fn bound_subscription_channel(&self, binding: &Binding) -> Result<Channel> {
        let mut c = self.subscription_channel(binding.channel_id)?.clone();
        // A channel ID cannot be reassigned to a different financial namespace.
        if c.merchant != binding.merchant
            || c.environment != binding.environment
            || c.currency != binding.currency
        {
            return Err(Error::Conflict);
        }
        c.verification.payment_type = binding.payment_type.clone();
        c.verification.endpoint = binding.endpoint.clone();
        c.verification.notify_url = binding.notify_url.clone();
        c.verification.return_url = binding.return_url.clone();
        Ok(c)
    }
    pub(crate) fn subscription_checkout(
        &self,
        binding: &Binding,
        order: &str,
        amount: i64,
    ) -> Result<pb::EPayCheckout> {
        let mut c = self.bound_subscription_channel(binding)?;
        // Existing obligations retain their frozen amount after admission limits change.
        c.max_amount_minor = c.max_amount_minor.max(amount);
        let checkout_url = providers::epay_checkout(&c, order, amount)?;
        Ok(pb::EPayCheckout {
            intent_id: 0,
            order_id: order.into(),
            checkout_url,
            method: if c.currency == "LDC" { "POST" } else { "GET" }.into(),
        })
    }
    pub(crate) fn subscription_verify(
        &self,
        binding: &Binding,
        order: &str,
        amount: i64,
        raw: &[u8],
        signature: &[u8],
    ) -> Result<(Channel, String, String)> {
        if raw.is_empty() || raw.len() > 32768 || signature.is_empty() || signature.len() > 2048 {
            return Err(Error::Invalid);
        }
        let c = self.bound_subscription_channel(binding)?;
        let verified = providers::verify_epay(raw, signature, &c, order, amount)?;
        Ok((c, verified.event_id, verified.transaction))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[sqlx::test(migrations = false)]
    async fn checkout_freezes_dedicated_callback_and_never_signs_wallet_order(pool: PgPool) {
        let mut c =
            Channel::stripe_test(7, "1001".into(), "synthetic-epay-secret-123".into(), 10000);
        c.provider = "epay".into();
        c.currency = "CNY".into();
        c.verification = ProviderVerification {
            payment_type: "alipay".into(),
            endpoint: "https://epay.example.test".into(),
            notify_url: "https://lmm.example.test/payments/callbacks/7".into(),
            return_url: "https://lmm.example.test/subscriptions".into(),
            ..Default::default()
        };
        let s = PaymentsStore::new(pool, vec![c]).unwrap();
        let binding = s.subscription_binding(7).unwrap();
        assert_eq!(
            binding.notify_url,
            "https://lmm.example.test/payments/callbacks/subscriptions/7"
        );
        let id = format!("sub_{}", "a".repeat(64));
        let checkout = s.subscription_checkout(&binding, &id, 125).unwrap();
        let url = reqwest::Url::parse(&checkout.checkout_url).unwrap();
        let params: BTreeMap<_, _> = url.query_pairs().collect();
        assert_eq!(params.get("out_trade_no").unwrap(), id.as_str());
        assert_eq!(
            params.get("notify_url").unwrap(),
            binding.notify_url.as_str()
        );
        assert_eq!(
            s.subscription_channel(7).unwrap().verification.notify_url,
            "https://lmm.example.test/payments/callbacks/7"
        );
    }
}

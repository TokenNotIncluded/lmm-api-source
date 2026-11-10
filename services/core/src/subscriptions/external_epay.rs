//! External purchases create consumption grants, never wallet credits/charges.
use super::*;
use crate::payments::{
    PaymentsStore,
    identifiers::{self, Subject},
    subscription_epay::Binding,
};
use serde::Deserialize;
use sha2::{Digest, Sha256};

pub const EPAY_SCHEMA: &str = include_str!("../../schema/subscription_epay.sql");
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct ExternalPrice {
    pub plan_id: i64,
    pub plan_revision: i64,
    pub channel_id: i64,
    pub currency: String,
    pub amount_minor: String,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct EPayPurchaseRequest {
    pub operation_key: String,
    pub plan_id: i64,
    pub channel_id: i64,
}
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct EPayPurchase {
    pub order_id: String,
    pub plan: Plan,
    pub account_id: i64,
    pub channel_id: i64,
    pub currency: String,
    pub amount_minor: String,
    pub expires_at: i64,
    pub confirmed_at: Option<i64>,
    pub canceled_at: Option<i64>,
    pub proof_id: Option<String>,
    pub subscription_id: Option<i64>,
    pub granted_at: Option<i64>,
    pub ends_at: Option<i64>,
}
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct EPayProof {
    pub proof_id: String,
    pub order_id: String,
    pub event_id: String,
    pub transaction_id: String,
    pub payload_sha256: Vec<u8>,
    pub accepted_at: i64,
}
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct EPayGrant {
    pub order_id: String,
    pub proof_id: String,
    pub operation_key: String,
    pub subscription_id: i64,
    pub granted_at: i64,
    pub ends_at: i64,
}
fn payment_error(e: crate::payments::Error) -> Error {
    use crate::payments::Error as P;
    match e {
        P::Invalid => Error::Invalid,
        P::Denied => Error::Forbidden,
        P::Conflict => Error::Conflict,
        P::NotFound => Error::NotFound,
        P::Unavailable | P::Storage => Error::Unavailable,
    }
}
fn amount(value: &str) -> Result<i64> {
    let n: i64 = value.parse().map_err(|_| Error::Invalid)?;
    if !(1..=9_000_000_000_000).contains(&n) || n.to_string() != value {
        return Err(Error::Invalid);
    }
    Ok(n)
}
async fn order(c: &mut PgConnection, id: &str) -> Result<EPayPurchase> {
    let r = sqlx::query("SELECT o.*,r.terms,false AS archived,o.plan_revision AS revision,p.id AS proof_id,g.subscription_id,g.granted_at,g.ends_at FROM core_subscriptions.external_orders o JOIN core_subscriptions.plan_revisions r ON r.plan_id=o.plan_id AND r.revision=o.plan_revision LEFT JOIN core_subscriptions.external_proofs p ON p.order_id=o.id LEFT JOIN core_subscriptions.external_grants g ON g.order_id=o.id WHERE o.id=$1")
        .bind(id).fetch_optional(c).await?.ok_or(Error::NotFound)?;
    Ok(EPayPurchase {
        order_id: id.into(),
        plan: plan_row(&r)?,
        account_id: r.try_get("account_id")?,
        channel_id: r.try_get("channel_id")?,
        currency: r.try_get("currency")?,
        amount_minor: r.try_get::<i64, _>("amount_minor")?.to_string(),
        expires_at: r.try_get("expires_at")?,
        confirmed_at: r.try_get("confirmed_at")?,
        canceled_at: r.try_get("canceled_at")?,
        proof_id: r.try_get("proof_id")?,
        subscription_id: r.try_get("subscription_id")?,
        granted_at: r.try_get("granted_at")?,
        ends_at: r.try_get("ends_at")?,
    })
}
async fn binding(c: &mut PgConnection, id: &str) -> Result<Binding> {
    decode(
        sqlx::query_scalar(
            "SELECT channel_binding FROM core_subscriptions.external_orders WHERE id=$1",
        )
        .bind(id)
        .fetch_one(c)
        .await?,
    )
}
async fn owned(c: &mut PgConnection, secret: &str, id: &str) -> Result<Principal> {
    let a = actor(c, secret, false).await?;
    lock(c, a.user_id).await?;
    let owns:bool=sqlx::query_scalar("SELECT actor_user_id=$2 AND account_id=$3 FROM core_subscriptions.external_orders WHERE id=$1 FOR UPDATE")
        .bind(id).bind(a.user_id).bind(a.owner_account_id).fetch_optional(c).await?.ok_or(Error::NotFound)?;
    if !owns {
        return Err(Error::NotFound);
    }
    Ok(a)
}
async fn proof(c: &mut PgConnection, id: &str) -> Result<Option<EPayProof>> {
    let r = sqlx::query("SELECT * FROM core_subscriptions.external_proofs WHERE order_id=$1")
        .bind(id)
        .fetch_optional(c)
        .await?;
    r.map(|r| {
        Ok(EPayProof {
            proof_id: r.try_get("id")?,
            order_id: r.try_get("order_id")?,
            event_id: r.try_get("event_id")?,
            transaction_id: r.try_get("transaction_id")?,
            payload_sha256: r.try_get("payload_sha256")?,
            accepted_at: r.try_get("accepted_at")?,
        })
    })
    .transpose()
}
impl SubscriptionStore {
    /// A price is immutable within a plan revision. Repricing needs a new revision.
    pub async fn save_epay_price(
        &self,
        payments: &PaymentsStore,
        secret: &str,
        key: &str,
        p: &ExternalPrice,
    ) -> Result<ExternalPrice> {
        operation_key(key)?;
        let n = amount(&p.amount_minor)?;
        let mut tx = self.begin().await?;
        let a = actor(&mut tx, secret, true).await?;
        lock(&mut tx, a.user_id).await?;
        let request = json!(["epay_price", p]);
        if let Some(old) = prior(&mut tx, a.user_id, key, &request).await? {
            tx.commit().await?;
            return Ok(old);
        }
        let current = plan(&mut tx, p.plan_id, true).await?;
        let channel = payments
            .subscription_channel(p.channel_id)
            .map_err(payment_error)?;
        if current.revision != p.plan_revision
            || current.archived
            || channel.currency != p.currency
            || n > channel.max_amount_minor
        {
            return Err(Error::Conflict);
        }
        sqlx::query("INSERT INTO core_subscriptions.external_prices(plan_id,plan_revision,channel_id,currency,amount_minor,actor_user_id) VALUES($1,$2,$3,$4,$5,$6)")
            .bind(p.plan_id).bind(p.plan_revision).bind(p.channel_id).bind(&p.currency).bind(n).bind(a.user_id).execute(&mut *tx).await?;
        remember(&mut tx, a.user_id, key, &request, p).await?;
        tx.commit().await?;
        Ok(p.clone())
    }
    pub async fn epay_prices(&self, secret: &str, plan_id: i64) -> Result<Vec<ExternalPrice>> {
        let mut tx = self.begin().await?;
        actor(&mut tx, secret, false).await?;
        let rows=sqlx::query("SELECT e.* FROM core_subscriptions.external_prices e JOIN core_subscriptions.plans p ON p.id=e.plan_id AND p.revision=e.plan_revision JOIN core_subscriptions.plan_revisions r ON r.plan_id=p.id AND r.revision=p.revision WHERE p.id=$1 AND NOT p.archived AND (r.terms->>'enabled')::boolean ORDER BY channel_id")
            .bind(plan_id).fetch_all(&mut *tx).await?;
        let result = rows
            .into_iter()
            .map(|r| {
                Ok(ExternalPrice {
                    plan_id: r.try_get("plan_id")?,
                    plan_revision: r.try_get("plan_revision")?,
                    channel_id: r.try_get("channel_id")?,
                    currency: r.try_get("currency")?,
                    amount_minor: r.try_get::<i64, _>("amount_minor")?.to_string(),
                })
            })
            .collect::<Result<_>>()?;
        tx.commit().await?;
        Ok(result)
    }
    pub async fn prepare_epay_purchase(
        &self,
        payments: &PaymentsStore,
        secret: &str,
        r: &EPayPurchaseRequest,
    ) -> Result<EPayPurchase> {
        operation_key(&r.operation_key)?;
        let mut tx = self.begin().await?;
        let a = actor(&mut tx, secret, false).await?;
        lock(&mut tx, a.user_id).await?;
        let prior:Option<(String,i64,i64,i64)>=sqlx::query_as("SELECT id,plan_id,channel_id,account_id FROM core_subscriptions.external_orders WHERE actor_user_id=$1 AND operation_key=$2")
            .bind(a.user_id).bind(&r.operation_key).fetch_optional(&mut *tx).await?;
        if let Some((id, p, c, account)) = prior {
            if (p, c, account) != (r.plan_id, r.channel_id, a.owner_account_id) {
                return Err(Error::Conflict);
            }
            let result = order(&mut tx, &id).await?;
            tx.commit().await?;
            return Ok(result);
        }
        let p = plan(&mut tx, r.plan_id, true).await?;
        if p.archived || !p.terms.enabled {
            return Err(Error::Conflict);
        }
        let (currency,n):(String,i64)=sqlx::query_as("SELECT currency,amount_minor FROM core_subscriptions.external_prices WHERE plan_id=$1 AND plan_revision=$2 AND channel_id=$3")
            .bind(p.id).bind(p.revision).bind(r.channel_id).fetch_optional(&mut *tx).await?.ok_or(Error::NotFound)?;
        let channel = payments
            .subscription_channel(r.channel_id)
            .map_err(payment_error)?;
        if currency != channel.currency || n > channel.max_amount_minor {
            return Err(Error::Conflict);
        }
        let b = payments
            .subscription_binding(r.channel_id)
            .map_err(payment_error)?;
        let id = format!("sub_{}", random_id());
        let time = now(&mut tx).await?;
        sqlx::query("INSERT INTO core_subscriptions.external_orders(id,actor_user_id,account_id,operation_key,plan_id,plan_revision,channel_id,provider,merchant,environment,currency,amount_minor,channel_binding,quoted_at,expires_at) VALUES($1,$2,$3,$4,$5,$6,$7,'epay',$8,$9,$10,$11,$12,$13,$14)")
            .bind(&id).bind(a.user_id).bind(a.owner_account_id).bind(&r.operation_key).bind(p.id).bind(p.revision).bind(r.channel_id).bind(&b.merchant).bind(&b.environment).bind(currency).bind(n).bind(Json(&b)).bind(time).bind(time+600).execute(&mut *tx).await?;
        let result = order(&mut tx, &id).await?;
        tx.commit().await?;
        Ok(result)
    }
    pub async fn read_epay_purchase(&self, secret: &str, id: &str) -> Result<EPayPurchase> {
        let mut tx = self.begin().await?;
        owned(&mut tx, secret, id).await?;
        let result = order(&mut tx, id).await?;
        tx.commit().await?;
        Ok(result)
    }
    /// No confirmation/identifier mutation. Status remains readable when signing
    /// configuration is unavailable; a pending confirmed order can be recovered
    /// after a lost confirmation response without submitting another purchase.
    pub async fn read_epay_purchase_with_checkout(
        &self,
        payments: &PaymentsStore,
        secret: &str,
        id: &str,
    ) -> Result<(EPayPurchase, Option<crate::internal_rpc::pb::EPayCheckout>)> {
        let mut tx = self.begin().await?;
        owned(&mut tx, secret, id).await?;
        let o = order(&mut tx, id).await?;
        let checkout =
            if o.confirmed_at.is_some() && o.canceled_at.is_none() && o.proof_id.is_none() {
                let b = binding(&mut tx, id).await?;
                // The caller explicitly needs status even if the old channel is
                // temporarily unavailable. Do not convert that into a new order.
                payments
                    .subscription_checkout(&b, id, amount(&o.amount_minor)?)
                    .ok()
            } else {
                None
            };
        tx.commit().await?;
        Ok((o, checkout))
    }
    /// Confirmation reserves a purchase slot before any checkout can leave Core.
    pub async fn confirm_epay_purchase(
        &self,
        payments: &PaymentsStore,
        secret: &str,
        id: &str,
    ) -> Result<crate::internal_rpc::pb::EPayCheckout> {
        let mut tx = self.begin().await?;
        let a = owned(&mut tx, secret, id).await?;
        let o = order(&mut tx, id).await?;
        if o.canceled_at.is_some() {
            return Err(Error::Conflict);
        }
        if o.confirmed_at.is_none() {
            let time = now(&mut tx).await?;
            let current = plan(&mut tx, o.plan.id, true).await?;
            if o.expires_at <= time
                || current.archived
                || !current.terms.enabled
                || current.revision != o.plan.revision
            {
                return Err(Error::Conflict);
            }
            if o.plan.terms.max_purchases_per_user > 0 {
                let count = purchase_count(&mut tx, a.user_id, o.plan.id).await?;
                if count >= o.plan.terms.max_purchases_per_user {
                    return Err(Error::Conflict);
                }
            }
            sqlx::query(
                "UPDATE core_subscriptions.external_orders SET confirmed_at=$2 WHERE id=$1",
            )
            .bind(id)
            .bind(time)
            .execute(&mut *tx)
            .await?;
        }
        if o.proof_id.is_some() {
            return Err(Error::Conflict);
        }
        let b = binding(&mut tx, id).await?;
        let c = payments
            .bound_subscription_channel(&b)
            .map_err(payment_error)?;
        identifiers::claim(&mut tx, &c, Subject::Subscription(id), vec![("order", id)])
            .await
            .map_err(payment_error)?;
        let checkout = payments
            .subscription_checkout(&b, id, amount(&o.amount_minor)?)
            .map_err(payment_error)?;
        tx.commit().await?;
        Ok(checkout)
    }
    /// Cancel stops issuing checkout; an issued order retains its payment obligation.
    pub async fn cancel_epay_purchase(&self, secret: &str, id: &str) -> Result<EPayPurchase> {
        let mut tx = self.begin().await?;
        owned(&mut tx, secret, id).await?;
        sqlx::query("UPDATE core_subscriptions.external_orders SET canceled_at=coalesce(canceled_at,$2) WHERE id=$1").bind(id).bind(now(&mut tx).await?).execute(&mut *tx).await?;
        let result = order(&mut tx, id).await?;
        tx.commit().await?;
        Ok(result)
    }
    /// Service-only ingress. Go's interpretation is never an entitlement authority.
    pub async fn accept_epay_proof(
        &self,
        payments: &PaymentsStore,
        id: &str,
        raw: &[u8],
        signature: &[u8],
    ) -> Result<EPayProof> {
        let hash = Sha256::digest(raw).to_vec();
        let mut tx = self.begin().await?;
        sqlx::query("SELECT id FROM core_subscriptions.external_orders WHERE id=$1 FOR UPDATE")
            .bind(id)
            .fetch_optional(&mut *tx)
            .await?
            .ok_or(Error::NotFound)?;
        if let Some(old) = proof(&mut tx, id).await? {
            let sig: Vec<u8> = sqlx::query_scalar(
                "SELECT signature FROM core_subscriptions.external_proofs WHERE order_id=$1",
            )
            .bind(id)
            .fetch_one(&mut *tx)
            .await?;
            if old.payload_sha256 != hash || sig != signature {
                return Err(Error::Conflict);
            }
            tx.commit().await?;
            return Ok(old);
        }
        let o = order(&mut tx, id).await?;
        if o.confirmed_at.is_none() {
            return Err(Error::Conflict);
        }
        let b = binding(&mut tx, id).await?;
        let (channel, event, transaction) = payments
            .subscription_verify(&b, id, amount(&o.amount_minor)?, raw, signature)
            .map_err(|e| match e {
                // Distinguish invalid original evidence from RPC permission
                // denials, which must stay recoverable after configuration repair.
                crate::payments::Error::Denied => Error::Invalid,
                other => payment_error(other),
            })?;
        identifiers::claim(
            &mut tx,
            &channel,
            Subject::Subscription(id),
            vec![("order", id), ("event", &event), ("payment", &transaction)],
        )
        .await
        .map_err(payment_error)?;
        let p = EPayProof {
            proof_id: format!("sproof_{}", random_id()),
            order_id: id.into(),
            event_id: event,
            transaction_id: transaction,
            payload_sha256: hash,
            accepted_at: now(&mut tx).await?,
        };
        sqlx::query("INSERT INTO core_subscriptions.external_proofs(id,order_id,event_id,transaction_id,payload_sha256,signed_payload,signature,accepted_at) VALUES($1,$2,$3,$4,$5,$6,$7,$8)")
            .bind(&p.proof_id).bind(id).bind(&p.event_id).bind(&p.transaction_id).bind(&p.payload_sha256).bind(raw).bind(signature).bind(p.accepted_at).execute(&mut *tx).await?;
        tx.commit().await?;
        Ok(p)
    }
    pub async fn get_epay_proof(&self, id: &str, event: &str, hash: &[u8]) -> Result<EPayProof> {
        let mut tx = self.begin().await?;
        let p = proof(&mut tx, id).await?.ok_or(Error::NotFound)?;
        if p.event_id != event || p.payload_sha256 != hash {
            return Err(Error::Conflict);
        }
        tx.commit().await?;
        Ok(p)
    }
    pub async fn apply_epay_grant(&self, id: &str, proof_id: &str, key: &str) -> Result<EPayGrant> {
        operation_key(key)?;
        let mut tx = self.begin().await?;
        let value: Value = sqlx::query_scalar(include_str!("external_grant.sql"))
            .bind(id)
            .bind(proof_id)
            .bind(key)
            .fetch_one(&mut *tx)
            .await?;
        let result = decode(value)?;
        tx.commit().await?;
        Ok(result)
    }
}

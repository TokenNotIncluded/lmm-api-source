//! Each original Creem paid transaction creates one frozen-period entitlement grant.
//! No balance mutation, provider-reported API `paid`, or subscription refund.
use super::*;
pub use crate::payments::subscription_creem::CreemPrice;
use crate::payments::{
    PaymentsStore,
    identifiers::{self, Subject},
    subscription_creem::{Binding, Change},
};
use serde::Deserialize;
use sha2::{Digest, Sha256};

pub const CREEM_SCHEMA: &str = include_str!("../../schema/subscription_creem.sql");
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct CreemOrder {
    pub order: EPayPurchase,
    pub binding: Binding,
    pub provider_subscription_id: Option<String>,
    pub provider_customer_id: Option<String>,
    pub state: String,
    pub state_created: i64,
    pub current_period_start: i64,
    pub current_period_end: i64,
    pub latest_grant_id: Option<i64>,
}
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct CreemEventReceipt {
    pub order_id: String,
    pub event_id: String,
    pub payload_sha256: Vec<u8>,
    pub kind: String,
    pub state: String,
    pub transaction_id: String,
    pub subscription_id: Option<i64>,
    pub period_start: i64,
    pub period_end: i64,
}
fn pay_error(e: crate::payments::Error) -> Error {
    use crate::payments::Error as P;
    match e {
        P::Invalid | P::Denied => Error::Invalid,
        P::Conflict => Error::Conflict,
        P::NotFound => Error::NotFound,
        _ => Error::Unavailable,
    }
}
async fn read(c: &mut PgConnection, id: &str) -> Result<CreemOrder> {
    let row=sqlx::query("SELECT o.channel_binding,s.*,(SELECT c.subscription_id FROM core_subscriptions.creem_cycles c WHERE c.order_id=o.id ORDER BY period_end DESC LIMIT 1) AS latest_grant_id FROM core_subscriptions.external_orders o JOIN core_subscriptions.creem_state s ON s.order_id=o.id WHERE o.id=$1 AND o.provider='creem'")
        .bind(id).fetch_optional(&mut *c).await?.ok_or(Error::NotFound)?;
    Ok(CreemOrder {
        order: external_epay::order(c, id).await?,
        binding: decode(row.try_get("channel_binding")?)?,
        provider_subscription_id: row.try_get("provider_subscription_id")?,
        provider_customer_id: row.try_get("provider_customer_id")?,
        state: row.try_get("status")?,
        state_created: row.try_get("event_created")?,
        current_period_start: row.try_get("period_start")?,
        current_period_end: row.try_get("period_end")?,
        latest_grant_id: row.try_get("latest_grant_id")?,
    })
}
async fn price(c: &mut PgConnection, plan_id: i64, channel: i64) -> Result<CreemPrice> {
    let json:Value=sqlx::query_scalar("SELECT to_jsonb(s)||jsonb_build_object('currency',e.currency,'amount_minor',e.amount_minor::text) FROM core_subscriptions.creem_prices s JOIN core_subscriptions.external_prices e USING(plan_id,plan_revision,channel_id) JOIN core_subscriptions.plans p ON p.id=s.plan_id AND p.revision=s.plan_revision WHERE s.plan_id=$1 AND s.channel_id=$2")
        .bind(plan_id).bind(channel).fetch_optional(c).await?.ok_or(Error::NotFound)?;
    decode(json)
}
impl SubscriptionStore {
    pub async fn resolve_creem_subscription_order(
        &self,
        channel: i64,
        subscription: &str,
    ) -> Result<String> {
        if channel <= 0 || subscription.len() > 128 || !subscription.starts_with("sub_") {
            return Err(Error::Invalid);
        }
        let mut tx = self.begin().await?;
        let id=sqlx::query_scalar("SELECT o.id FROM core_subscriptions.external_orders o JOIN core_subscriptions.creem_state s ON s.order_id=o.id WHERE o.channel_id=$1 AND o.provider='creem' AND s.provider_subscription_id=$2")
            .bind(channel).bind(subscription).fetch_optional(&mut *tx).await?.ok_or(Error::NotFound)?;
        tx.commit().await?;
        Ok(id)
    }

    pub async fn save_creem_price(
        &self,
        payments: &PaymentsStore,
        secret: &str,
        key: &str,
        p: &CreemPrice,
    ) -> Result<CreemPrice> {
        operation_key(key)?;
        let amount = p.amount_minor.parse::<i64>().map_err(|_| Error::Invalid)?;
        if !(1..=9_000_000_000_000).contains(&amount)
            || amount.to_string() != p.amount_minor
            || !matches!(p.currency.as_str(), "USD" | "CNY" | "EUR")
            || !matches!(
                (p.interval.as_str(), p.interval_count),
                ("month", 1 | 3 | 6) | ("year", 1)
            )
        {
            return Err(Error::Invalid);
        }
        for raw in [&p.success_url, &p.cancel_url] {
            let u = reqwest::Url::parse(raw).map_err(|_| Error::Invalid)?;
            if u.scheme() != "https"
                || u.host_str().is_none()
                || !u.username().is_empty()
                || u.password().is_some()
                || u.fragment().is_some()
            {
                return Err(Error::Invalid);
            }
        }
        let b = payments
            .creem_subscription_binding(p.clone())
            .map_err(pay_error)?;
        let mut tx = self.begin().await?;
        let a = actor(&mut tx, secret, true).await?;
        lock(&mut tx, a.user_id).await?;
        let request = json!(["creem_price", p]);
        if let Some(old) = prior(&mut tx, a.user_id, key, &request).await? {
            tx.commit().await?;
            return Ok(old);
        }
        let current = plan(&mut tx, p.plan_id, true).await?;
        let duration = match p.interval.as_str() {
            "month" => Some(p.interval_count),
            "year" => p.interval_count.checked_mul(12),
            _ => None,
        };
        let seconds = match p.interval.as_str() {
            "day" => 86400_i64.checked_mul(p.interval_count),
            "week" => 604800_i64.checked_mul(p.interval_count),
            _ => None,
        };
        if current.archived
            || current.revision != p.plan_revision
            || b.price.currency != p.currency
            || duration.is_some_and(|d| d != current.terms.duration_months)
            || seconds.is_some_and(|d| {
                current.terms.duration_months != 0 || d != current.terms.duration_seconds
            })
        {
            return Err(Error::Conflict);
        }
        sqlx::query("INSERT INTO core_subscriptions.external_prices(plan_id,plan_revision,channel_id,currency,amount_minor,actor_user_id) VALUES($1,$2,$3,$4,$5,$6)")
            .bind(p.plan_id).bind(p.plan_revision).bind(p.channel_id).bind(&p.currency).bind(amount).bind(a.user_id).execute(&mut *tx).await?;
        sqlx::query("INSERT INTO core_subscriptions.creem_prices(plan_id,plan_revision,channel_id,provider_price_id,interval,interval_count,success_url,cancel_url) VALUES($1,$2,$3,$4,$5,$6,$7,$8)")
            .bind(p.plan_id).bind(p.plan_revision).bind(p.channel_id).bind(&p.provider_price_id).bind(&p.interval).bind(p.interval_count).bind(&p.success_url).bind(&p.cancel_url).execute(&mut *tx).await?;
        remember(&mut tx, a.user_id, key, &request, p).await?;
        tx.commit().await?;
        Ok(p.clone())
    }
    pub async fn creem_prices(
        &self,
        secret: &str,
        plan_id: i64,
        admin: bool,
    ) -> Result<Vec<CreemPrice>> {
        let mut tx = self.begin().await?;
        actor(&mut tx, secret, admin).await?;
        let p = plan(&mut tx, plan_id, false).await?;
        if !admin && (p.archived || !p.terms.enabled) {
            return Ok(vec![]);
        }
        let channels:Vec<i64>=sqlx::query_scalar("SELECT channel_id FROM core_subscriptions.creem_prices WHERE plan_id=$1 AND plan_revision=$2 ORDER BY channel_id").bind(plan_id).bind(p.revision).fetch_all(&mut *tx).await?;
        let mut out = vec![];
        for channel in channels {
            out.push(price(&mut tx, plan_id, channel).await?);
        }
        tx.commit().await?;
        Ok(out)
    }
    pub async fn prepare_creem_purchase(
        &self,
        payments: &PaymentsStore,
        secret: &str,
        r: &EPayPurchaseRequest,
    ) -> Result<CreemOrder> {
        operation_key(&r.operation_key)?;
        let mut tx = self.begin().await?;
        let a = actor(&mut tx, secret, false).await?;
        lock(&mut tx, a.user_id).await?;
        let request = json!(["creem_purchase", r.plan_id, r.channel_id]);
        if let Some(id) = prior::<String>(&mut tx, a.user_id, &r.operation_key, &request).await? {
            let o = read(&mut tx, &id).await?;
            tx.commit().await?;
            return Ok(o);
        }
        let p = plan(&mut tx, r.plan_id, false).await?;
        if p.archived || !p.terms.enabled {
            return Err(Error::Conflict);
        }
        let binding = payments
            .creem_subscription_binding(price(&mut tx, r.plan_id, r.channel_id).await?)
            .map_err(pay_error)?;
        let at = now(&mut tx).await?;
        let id = format!("sub_{}", random_id());
        sqlx::query("INSERT INTO core_subscriptions.external_orders(id,actor_user_id,account_id,operation_key,plan_id,plan_revision,channel_id,provider,merchant,environment,currency,amount_minor,channel_binding,quoted_at,expires_at) VALUES($1,$2,$3,$4,$5,$6,$7,'creem',$8,$9,$10,$11,$12,$13,$13+1800)")
            .bind(&id).bind(a.user_id).bind(a.owner_account_id).bind(&r.operation_key).bind(p.id).bind(p.revision).bind(r.channel_id).bind(&binding.merchant).bind(&binding.environment).bind(&binding.price.currency).bind(binding.price.amount_minor.parse::<i64>().map_err(|_|Error::Invalid)?).bind(Json(&binding)).bind(at).execute(&mut *tx).await?;
        sqlx::query("INSERT INTO core_subscriptions.creem_state(order_id) VALUES($1)")
            .bind(&id)
            .execute(&mut *tx)
            .await?;
        remember(&mut tx, a.user_id, &r.operation_key, &request, &id).await?;
        let result = read(&mut tx, &id).await?;
        tx.commit().await?;
        Ok(result)
    }
    pub async fn read_creem_purchase(&self, secret: &str, id: &str) -> Result<CreemOrder> {
        let mut tx = self.begin().await?;
        external_epay::owned(&mut tx, secret, id).await?;
        let out = read(&mut tx, id).await?;
        tx.commit().await?;
        Ok(out)
    }
    pub async fn confirm_creem_purchase(&self, secret: &str, id: &str) -> Result<CreemOrder> {
        let mut tx = self.begin().await?;
        let a = external_epay::owned(&mut tx, secret, id).await?;
        let o = read(&mut tx, id).await?;
        if o.order.canceled_at.is_some() {
            return Err(Error::Conflict);
        }
        if o.order.confirmed_at.is_none() {
            let p = plan(&mut tx, o.order.plan.id, false).await?;
            if o.order.canceled_at.is_some()
                || now(&mut tx).await? > o.order.expires_at
                || p.archived
                || !p.terms.enabled
                || p.revision != o.order.plan.revision
            {
                return Err(Error::Conflict);
            }
            if p.terms.max_purchases_per_user > 0
                && purchase_count(&mut tx, a.user_id, p.id).await? >= p.terms.max_purchases_per_user
            {
                return Err(Error::Conflict);
            }
            sqlx::query("UPDATE core_subscriptions.external_orders SET confirmed_at=floor(extract(epoch FROM clock_timestamp()))::bigint WHERE id=$1").bind(id).execute(&mut *tx).await?;
        }
        let result = read(&mut tx, id).await?;
        tx.commit().await?;
        Ok(result)
    }
    pub async fn creem_event(
        &self,
        channel: i64,
        id: &str,
        event: &str,
        hash: &[u8],
    ) -> Result<CreemEventReceipt> {
        let mut tx = self.begin().await?;
        let row=sqlx::query("SELECT payload_sha256,result FROM core_subscriptions.creem_events WHERE order_id=$1 AND event_id=$2 AND EXISTS(SELECT FROM core_subscriptions.external_orders o WHERE o.id=$1 AND o.channel_id=$3 AND o.provider='creem')").bind(id).bind(event).bind(channel).fetch_optional(&mut *tx).await?.ok_or(Error::NotFound)?;
        if row.try_get::<Vec<u8>, _>("payload_sha256")? != hash {
            return Err(Error::Conflict);
        }
        let result = decode(row.try_get("result")?)?;
        tx.commit().await?;
        Ok(result)
    }
    pub async fn accept_creem_event(
        &self,
        payments: &PaymentsStore,
        channel_id: i64,
        id: &str,
        raw: &[u8],
        signature: &[u8],
    ) -> Result<CreemEventReceipt> {
        if raw.is_empty() || raw.len() > 32768 || signature.is_empty() || signature.len() > 2048 {
            return Err(Error::Invalid);
        }
        let hash = Sha256::digest(raw).to_vec();
        let mut tx = self.begin().await?;
        let owner:i64=sqlx::query_scalar("SELECT actor_user_id FROM core_subscriptions.external_orders WHERE id=$1 AND provider='creem'").bind(id).fetch_optional(&mut *tx).await?.ok_or(Error::NotFound)?;
        lock(&mut tx, owner).await?;
        let o = read(&mut tx, id).await?;
        if o.binding.price.channel_id != channel_id {
            return Err(Error::Conflict);
        }
        // Receipt recovery precedes key/config lookup; an already accepted
        // original event is not reinterpreted after rotation or reconfiguration.
        let event: Value =
            crate::payments::subscription_creem::raw_event(raw).map_err(pay_error)?;
        let event_id = event
            .get("id")
            .and_then(Value::as_str)
            .ok_or(Error::Invalid)?;
        if let Some(row)=sqlx::query("SELECT payload_sha256,result FROM core_subscriptions.creem_events WHERE order_id=$1 AND event_id=$2").bind(id).bind(event_id).fetch_optional(&mut *tx).await?{
            if row.try_get::<Vec<u8>,_>("payload_sha256")?!=hash{return Err(Error::Conflict);}
            let result=decode(row.try_get("result")?)?;tx.commit().await?;return Ok(result);
        }
        if o.order.confirmed_at.is_none() {
            return Err(Error::Conflict);
        }
        let quoted: i64 = sqlx::query_scalar(
            "SELECT quoted_at FROM core_subscriptions.external_orders WHERE id=$1",
        )
        .bind(id)
        .fetch_one(&mut *tx)
        .await?;
        let (channel, v) = payments
            .verify_creem_subscription(
                &o.binding,
                id,
                o.provider_subscription_id.as_deref(),
                quoted,
                (raw, signature),
                now(&mut tx).await?,
            )
            .map_err(pay_error)?;
        if o.provider_subscription_id
            .as_ref()
            .is_some_and(|p| !v.subscription_id.is_empty() && p != &v.subscription_id)
            || o.provider_customer_id
                .as_ref()
                .is_some_and(|p| !v.customer_id.is_empty() && p != &v.customer_id)
        {
            return Err(Error::Conflict);
        }
        let mut aliases = vec![("event", v.event_id.as_str())];
        if !v.subscription_id.is_empty() {
            aliases.push(("order", v.subscription_id.as_str()));
        }
        if !v.checkout_id.is_empty() {
            aliases.push(("order", v.checkout_id.as_str()));
        }
        if v.change == Change::Paid {
            aliases.push(("payment", v.transaction_id.as_str()));
        }
        identifiers::claim(&mut tx, &channel, Subject::Subscription(id), aliases)
            .await
            .map_err(pay_error)?;
        let mut grant = None;
        if v.change == Change::Paid {
            let old=sqlx::query("SELECT subscription_id,transaction_id,period_start,period_end FROM core_subscriptions.creem_cycles WHERE order_id=$1 AND transaction_id=$2").bind(id).bind(&v.transaction_id).fetch_optional(&mut *tx).await?;
            if let Some(old) = old {
                if old.try_get::<String, _>("transaction_id")? != v.transaction_id
                    || old.try_get::<i64, _>("period_start")? != v.period_start
                    || old.try_get::<i64, _>("period_end")? != v.period_end
                {
                    return Err(Error::Conflict);
                }
                grant = Some(old.try_get("subscription_id")?);
            } else {
                let terms = &o.order.plan.terms;
                let overlap:bool=sqlx::query_scalar("SELECT EXISTS(SELECT FROM core_subscriptions.creem_cycles WHERE order_id=$1 AND period_start<$3 AND period_end>$2)").bind(id).bind(v.period_start).bind(v.period_end).fetch_one(&mut *tx).await?;
                if overlap {
                    return Err(Error::Conflict);
                }
                let active = true;
                let sub:i64=sqlx::query_scalar("INSERT INTO core_charging.subscriptions(account_id,grant_reference,period,anchor,seconds,ends_at,limit_credits,models,groups,active) VALUES($1,$2,$3,$4,$5,$6,$7,$8,$9,$10) RETURNING id")
                    .bind(o.order.account_id).bind(format!("creem-subscription:{id}:{}",v.transaction_id)).bind(&terms.reset_period).bind(v.period_start).bind(terms.reset_seconds).bind(v.period_end).bind(terms.limit_units.parse::<i64>().map_err(|_|Error::Invalid)?).bind(&terms.models).bind(&terms.groups).bind(active).fetch_one(&mut *tx).await?;
                sqlx::query("INSERT INTO core_subscriptions.creem_cycles(order_id,transaction_id,provider_subscription_id,period_start,period_end,subscription_id,event_id) VALUES($1,$2,$3,$4,$5,$6,$7)")
                    .bind(id).bind(&v.transaction_id).bind(&v.subscription_id).bind(v.period_start).bind(v.period_end).bind(sub).bind(&v.event_id).execute(&mut *tx).await?;
                grant = Some(sub);
            }
        }
        // An older failed transaction cannot downgrade a subsequently paid cycle.
        let latest_paid:i64=sqlx::query_scalar("SELECT coalesce(max(period_end),0) FROM core_subscriptions.creem_cycles WHERE order_id=$1").bind(id).fetch_one(&mut *tx).await?;
        let mut state = o.state.clone();
        let mut state_created = o.state_created;
        let mut state_event: Option<&str> = None;
        if !v.state.is_empty()
            && v.created >= o.state_created
            && !(v.change == Change::Failed && v.period_end <= latest_paid)
        {
            state = v.state.clone();
            state_created = v.created;
            state_event = Some(&v.event_id);
        }
        sqlx::query("UPDATE core_subscriptions.creem_state SET provider_subscription_id=coalesce(provider_subscription_id,nullif($2,'')),provider_customer_id=coalesce(provider_customer_id,nullif($3,'')),status=$4,event_created=greatest(event_created,$5),event_id=coalesce($6,event_id),period_start=CASE WHEN $8>period_end THEN $7 ELSE period_start END,period_end=greatest(period_end,$8) WHERE order_id=$1")
            .bind(id).bind(&v.subscription_id).bind(&v.customer_id).bind(&state).bind(state_created).bind(state_event).bind(if v.change==Change::Paid{v.period_start}else{0}).bind(if v.change==Change::Paid{v.period_end}else{0}).execute(&mut *tx).await?;
        let result = CreemEventReceipt {
            order_id: id.into(),
            event_id: v.event_id.clone(),
            payload_sha256: hash.clone(),
            kind: match v.change {
                Change::Paid => "paid",
                Change::Checkout => "checkout",
                Change::Failed => "failed",
                Change::State => "state",
            }
            .into(),
            state,
            transaction_id: v.transaction_id,
            subscription_id: grant,
            period_start: v.period_start,
            period_end: v.period_end,
        };
        sqlx::query("INSERT INTO core_subscriptions.creem_events(order_id,event_id,kind,event_created,payload_sha256,signed_payload,signature,result) VALUES($1,$2,$3,$4,$5,$6,$7,$8)").bind(id).bind(&result.event_id).bind(&result.kind).bind(v.created).bind(hash).bind(raw).bind(signature).bind(Json(&result)).execute(&mut *tx).await?;
        tx.commit().await?;
        Ok(result)
    }
}

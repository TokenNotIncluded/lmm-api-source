//! Frozen Pancake purchases and original provider events. No wallet credit.
use super::*;
use crate::payments::{
    PaymentsStore,
    identifiers::{self, Subject},
    subscription_pancake::{Change, PancakeBinding, PancakePrice},
};
use serde::Deserialize;
use sha2::{Digest, Sha256};

pub const PANCAKE_SCHEMA: &str = include_str!("../../schema/subscription_pancake.sql");
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct PancakeOrder {
    pub order: EPayPurchase,
    pub binding: PancakeBinding,
    pub provider_reference: String,
    pub provider_order_id: Option<String>,
    pub state: String,
    pub state_created: i64,
    pub paid_period_start: i64,
    pub paid_period_end: i64,
    pub latest_grant_id: Option<i64>,
    pub legacy_source: Option<Value>,
}
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct PancakeEventReceipt {
    pub order_id: String,
    pub event_id: String,
    pub payload_sha256: Vec<u8>,
    pub kind: String,
    pub state: String,
    pub provider_order_id: String,
    pub payment_id: Option<String>,
    pub cycle_key: String,
    pub subscription_id: Option<i64>,
    pub period_start: i64,
    pub period_end: i64,
}
fn pay_error(e: crate::payments::Error) -> Error {
    use crate::payments::Error as P;
    match e {
        P::Invalid => Error::Invalid,
        P::Denied => Error::Forbidden,
        P::Conflict => Error::Conflict,
        P::NotFound => Error::NotFound,
        _ => Error::Unavailable,
    }
}
async fn read(c: &mut PgConnection, id: &str) -> Result<PancakeOrder> {
    let r=sqlx::query("SELECT b.*,coalesce((SELECT subscription_id FROM core_subscriptions.pancake_cycles WHERE order_id=b.order_id ORDER BY period_end DESC LIMIT 1),(SELECT s.id FROM core_charging.subscriptions s JOIN core_subscriptions.external_orders o ON o.id=b.order_id AND o.account_id=s.account_id WHERE s.grant_reference=b.legacy_source->>'opening_grant_reference')) AS latest_grant_id FROM core_subscriptions.pancake_orders b WHERE order_id=$1").bind(id).fetch_optional(&mut *c).await?.ok_or(Error::NotFound)?;
    let mut order = external_epay::order(c, id).await?;
    order.plan = decode(r.try_get("frozen_plan")?)?;
    let latest_grant_id: Option<i64> = r.try_get("latest_grant_id")?;
    if latest_grant_id.is_some() {
        order.subscription_id = latest_grant_id;
        order.granted_at = Some(r.try_get("paid_period_start")?);
        order.ends_at = Some(r.try_get("paid_period_end")?)
    }
    Ok(PancakeOrder {
        order,
        binding: decode(r.try_get("binding")?)?,
        provider_reference: r.try_get("provider_reference")?,
        provider_order_id: r.try_get("provider_order_id")?,
        state: r.try_get("status")?,
        state_created: r.try_get("event_created")?,
        paid_period_start: r.try_get("paid_period_start")?,
        paid_period_end: r.try_get("paid_period_end")?,
        latest_grant_id,
        legacy_source: r.try_get("legacy_source")?,
    })
}
async fn resolve(c: &mut PgConnection, channel: i64, reference: &str) -> Result<String> {
    if channel < 0 || reference.is_empty() || reference.len() > 128 {
        return Err(Error::Invalid);
    }
    let rows:Vec<String>=sqlx::query_scalar("SELECT order_id FROM core_subscriptions.pancake_orders WHERE ($1=0 OR channel_id=$1) AND (provider_reference=$2 OR order_id=$2 OR provider_order_id=$2) LIMIT 2").bind(channel).bind(reference).fetch_all(c).await?;
    match rows.as_slice() {
        [id] => Ok(id.clone()),
        [] => Err(Error::NotFound),
        _ => Err(Error::Conflict),
    }
}
async fn event(
    c: &mut PgConnection,
    id: &str,
    event: &str,
    hash: &[u8],
) -> Result<Option<PancakeEventReceipt>> {
    let row:Option<(Vec<u8>,Value)>=sqlx::query_as("SELECT payload_sha256,result FROM core_subscriptions.pancake_events WHERE order_id=$1 AND event_id=$2").bind(id).bind(event).fetch_optional(c).await?;
    match row {
        None => Ok(None),
        Some((h, v)) if h == hash => Ok(Some(decode(v)?)),
        _ => Err(Error::Conflict),
    }
}
async fn price(c: &mut PgConnection, plan: i64, channel: i64) -> Result<PancakePrice> {
    let v:Value=sqlx::query_scalar("SELECT to_jsonb(p)||jsonb_build_object('currency',e.currency,'amount_minor',e.amount_minor::text) FROM core_subscriptions.pancake_prices p JOIN core_subscriptions.external_prices e USING(plan_id,plan_revision,channel_id) JOIN core_subscriptions.plans r ON r.id=p.plan_id AND r.revision=p.plan_revision WHERE p.plan_id=$1 AND p.channel_id=$2").bind(plan).bind(channel).fetch_optional(c).await?.ok_or(Error::NotFound)?;
    decode(v)
}
#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PublishPancakePrice {
    pub operation_key: String,
    pub expected_plan: Plan,
    pub price: PancakePrice,
}

fn validate_price(payments: &PaymentsStore, p: &PancakePrice) -> Result<i64> {
    let amount = p.amount_minor.parse::<i64>().map_err(|_| Error::Invalid)?;
    if !(1..=9_000_000_000_000).contains(&amount)
        || amount.to_string() != p.amount_minor
        || !matches!(p.currency.as_str(), "USD" | "CNY")
        || !matches!(p.product_type.as_str(), "one_time" | "subscription")
        || !(1..=128).contains(&p.provider_plan_reference.chars().count())
        || p.return_url.chars().count() > 2048
        || !matches!(p.checkout_region.as_str(), "global" | "china")
        || p.checkout_language.chars().count() > 32
        || !matches!(
            p.tax_category.as_str(),
            "digital_goods"
                | "saas"
                | "software"
                | "ebook"
                | "online_course"
                | "consulting"
                | "professional_service"
        )
    {
        return Err(Error::Invalid);
    }
    let url = reqwest::Url::parse(&p.return_url).map_err(|_| Error::Invalid)?;
    if url.scheme() != "https"
        || url.host_str().is_none()
        || !url.username().is_empty()
        || url.password().is_some()
        || url.fragment().is_some()
    {
        return Err(Error::Invalid);
    }
    let _ = payments
        .pancake_subscription_binding(p.clone())
        .map_err(pay_error)?;
    Ok(amount)
}

async fn register_price(
    c: &mut PgConnection,
    actor_id: i64,
    key: &str,
    p: &PancakePrice,
    amount: i64,
    request: &Value,
    expected: Option<&Plan>,
) -> Result<PancakePrice> {
    lock(c, actor_id).await?;
    if let Some(old) = prior(c, actor_id, key, request).await? {
        return Ok(old);
    }
    let current = plan(c, p.plan_id, true).await?;
    if expected.is_some_and(|expected| expected != &current) {
        return Err(Error::Conflict);
    }
    if current.archived || current.revision != p.plan_revision {
        return Err(Error::Conflict);
    }
    let duration = match p.billing_period.as_str() {
        "weekly" => current.terms.duration_months == 0 && current.terms.duration_seconds == 604800,
        "monthly" => current.terms.duration_months == 1,
        "quarterly" => current.terms.duration_months == 3,
        "yearly" => current.terms.duration_months == 12,
        "" => p.product_type == "one_time",
        _ => false,
    };
    if !duration
        || (p.product_type == "subscription"
            && (p.billing_period.is_empty() || p.currency != "USD"))
        || (p.product_type == "one_time" && !p.billing_period.is_empty())
    {
        return Err(Error::Invalid);
    }
    sqlx::query("INSERT INTO core_subscriptions.external_prices(plan_id,plan_revision,channel_id,currency,amount_minor,actor_user_id) VALUES($1,$2,$3,$4,$5,$6)").bind(p.plan_id).bind(p.plan_revision).bind(p.channel_id).bind(&p.currency).bind(amount).bind(actor_id).execute(&mut *c).await?;
    sqlx::query("INSERT INTO core_subscriptions.pancake_prices(plan_id,plan_revision,channel_id,store_id,product_id,product_type,billing_period,provider_plan_reference,return_url,checkout_region,checkout_language,tax_category) VALUES($1,$2,$3,$4,$5,$6,$7,$8,$9,$10,$11,$12)").bind(p.plan_id).bind(p.plan_revision).bind(p.channel_id).bind(&p.store_id).bind(&p.product_id).bind(&p.product_type).bind(&p.billing_period).bind(&p.provider_plan_reference).bind(&p.return_url).bind(&p.checkout_region).bind(&p.checkout_language).bind(&p.tax_category).execute(&mut *c).await?;
    remember(c, actor_id, key, request, p).await?;
    Ok(p.clone())
}

impl SubscriptionStore {
    pub async fn save_pancake_price(
        &self,
        payments: &PaymentsStore,
        secret: &str,
        key: &str,
        p: &PancakePrice,
    ) -> Result<PancakePrice> {
        operation_key(key)?;
        let amount = validate_price(payments, p)?;
        let mut tx = self.begin().await?;
        let a = actor(&mut tx, secret, true).await?;
        let request = json!(["pancake_price", p]);
        let result = register_price(&mut tx, a.user_id, key, p, amount, &request, None).await?;
        tx.commit().await?;
        Ok(result)
    }

    /// Offline publication uses database ownership, without constructing a user credential.
    pub async fn publish_pancake_price_as_owner(
        &self,
        payments: &PaymentsStore,
        input: &PublishPancakePrice,
    ) -> Result<PancakePrice> {
        operation_key(&input.operation_key)?;
        input.expected_plan.terms.validate()?;
        if input.expected_plan.archived
            || !input.expected_plan.terms.enabled
            || input.expected_plan.id != input.price.plan_id
            || input.expected_plan.revision != input.price.plan_revision
        {
            return Err(Error::Invalid);
        }
        let amount = validate_price(payments, &input.price)?;
        let mut tx = self.begin().await?;
        let owner: bool = sqlx::query_scalar(
            "SELECT core_identity.is_identity_relation_owner() AND \
             (current_user=session_user OR EXISTS(SELECT FROM pg_roles r \
              WHERE r.rolname=current_user AND NOT r.rolcanlogin \
              AND pg_has_role(session_user,r.rolname,'MEMBER'))) AND \
             NOT EXISTS(SELECT FROM pg_class WHERE oid IN \
              ('core_subscriptions.external_prices'::regclass, \
               'core_subscriptions.pancake_prices'::regclass, \
               'core_subscriptions.commands'::regclass) \
              AND pg_get_userbyid(relowner)<>current_user)",
        )
        .fetch_one(&mut *tx)
        .await?;
        if !owner {
            return Err(Error::Forbidden);
        }
        // Attribute the publication to the real publisher of this existing revision.
        // The database owner is the authority; this row is not a login or Session.
        let actor_id: i64 = sqlx::query_scalar(
            "SELECT r.actor_user_id FROM core_subscriptions.plan_revisions r \
             JOIN core_identity.users u ON u.id=r.actor_user_id \
             JOIN core_identity.accounts a ON a.id=u.personal_account_id AND a.kind='personal' \
             WHERE r.plan_id=$1 AND r.revision=$2 AND u.platform_role='superadmin' \
             FOR SHARE OF u",
        )
        .bind(input.price.plan_id)
        .bind(input.price.plan_revision)
        .fetch_optional(&mut *tx)
        .await?
        .ok_or(Error::Forbidden)?;
        let request = json!(["owner_pancake_price", input.expected_plan, input.price]);
        let result = register_price(
            &mut tx,
            actor_id,
            &input.operation_key,
            &input.price,
            amount,
            &request,
            Some(&input.expected_plan),
        )
        .await?;
        tx.commit().await?;
        Ok(result)
    }
    pub async fn pancake_prices(
        &self,
        secret: &str,
        id: i64,
        admin: bool,
    ) -> Result<Vec<PancakePrice>> {
        let mut tx = self.begin().await?;
        actor(&mut tx, secret, admin).await?;
        let p = plan(&mut tx, id, true).await?;
        if !admin && (p.archived || !p.terms.enabled) {
            tx.commit().await?;
            return Ok(vec![]);
        }
        let ids:Vec<i64>=sqlx::query_scalar("SELECT channel_id FROM core_subscriptions.pancake_prices WHERE plan_id=$1 AND plan_revision=$2 ORDER BY channel_id").bind(id).bind(p.revision).fetch_all(&mut *tx).await?;
        let mut out = vec![];
        for channel in ids {
            out.push(price(&mut tx, id, channel).await?)
        }
        tx.commit().await?;
        Ok(out)
    }
    pub async fn prepare_pancake_purchase(
        &self,
        payments: &PaymentsStore,
        secret: &str,
        r: &EPayPurchaseRequest,
    ) -> Result<PancakeOrder> {
        operation_key(&r.operation_key)?;
        if r.plan_id <= 0 || r.channel_id <= 0 {
            return Err(Error::Invalid);
        }
        let mut tx = self.begin().await?;
        let a = actor(&mut tx, secret, false).await?;
        lock(&mut tx, a.user_id).await?;
        let request = json!(["pancake_prepare", r]);
        if let Some(id) = prior::<String>(&mut tx, a.user_id, &r.operation_key, &request).await? {
            let out = read(&mut tx, &id).await?;
            tx.commit().await?;
            return Ok(out);
        }
        let plan = plan(&mut tx, r.plan_id, false).await?;
        if plan.archived || !plan.terms.enabled {
            return Err(Error::Conflict);
        }
        let p = price(&mut tx, r.plan_id, r.channel_id).await?;
        let b = payments
            .pancake_subscription_binding(p)
            .map_err(pay_error)?;
        let id = format!("sub_{}", random_id());
        let time = now(&mut tx).await?;
        sqlx::query("INSERT INTO core_subscriptions.external_orders(id,actor_user_id,account_id,operation_key,plan_id,plan_revision,channel_id,provider,merchant,environment,currency,amount_minor,channel_binding,quoted_at,expires_at) VALUES($1,$2,$3,$4,$5,$6,$7,'waffo_pancake',$8,$9,$10,$11,$12,$13,$14)").bind(&id).bind(a.user_id).bind(a.owner_account_id).bind(&r.operation_key).bind(plan.id).bind(plan.revision).bind(r.channel_id).bind(&b.merchant).bind(&b.environment).bind(&b.price.currency).bind(b.price.amount_minor.parse::<i64>().map_err(|_|Error::Invalid)?).bind(serde_json::to_value(&b).map_err(|_|Error::Invalid)?).bind(time).bind(time+1800).execute(&mut *tx).await?;
        sqlx::query("INSERT INTO core_subscriptions.pancake_orders(order_id,channel_id,provider_reference,binding,frozen_plan) VALUES($1,$2,$1,$3,$4)").bind(&id).bind(r.channel_id).bind(serde_json::to_value(&b).map_err(|_|Error::Invalid)?).bind(serde_json::to_value(&plan).map_err(|_|Error::Invalid)?).execute(&mut *tx).await?;
        remember(&mut tx, a.user_id, &r.operation_key, &request, &id).await?;
        let out = read(&mut tx, &id).await?;
        tx.commit().await?;
        Ok(out)
    }
    pub async fn read_pancake_purchase(&self, secret: &str, id: &str) -> Result<PancakeOrder> {
        let mut tx = self.begin().await?;
        let original = resolve(&mut tx, 0, id).await?;
        external_epay::owned(&mut tx, secret, &original).await?;
        let out = read(&mut tx, &original).await?;
        tx.commit().await?;
        Ok(out)
    }
    pub async fn pancake_purchases(&self, secret: &str) -> Result<Vec<PancakeOrder>> {
        let mut tx = self.begin().await?;
        let a = actor(&mut tx, secret, false).await?;
        let ids:Vec<String>=sqlx::query_scalar("SELECT o.id FROM core_subscriptions.external_orders o JOIN core_subscriptions.pancake_orders p ON p.order_id=o.id WHERE o.actor_user_id=$1 AND o.account_id=$2 ORDER BY o.quoted_at DESC,o.id LIMIT 100").bind(a.user_id).bind(a.owner_account_id).fetch_all(&mut *tx).await?;
        let mut out = vec![];
        for id in ids {
            out.push(read(&mut tx, &id).await?)
        }
        tx.commit().await?;
        Ok(out)
    }
    pub async fn confirm_pancake_purchase(&self, secret: &str, id: &str) -> Result<PancakeOrder> {
        let mut tx = self.begin().await?;
        let original = resolve(&mut tx, 0, id).await?;
        let id = original.as_str();
        let a = external_epay::owned(&mut tx, secret, id).await?;
        let out = read(&mut tx, id).await?;
        if out.order.canceled_at.is_some() {
            return Err(Error::Conflict);
        }
        if out.order.confirmed_at.is_none() {
            let current = plan(&mut tx, out.order.plan.id, true).await?;
            if current.archived
                || !current.terms.enabled
                || current.revision != out.order.plan.revision
                || out.order.expires_at <= now(&mut tx).await?
            {
                return Err(Error::Conflict);
            }
            if current.terms.max_purchases_per_user > 0
                && purchase_count(&mut tx, a.user_id, current.id).await?
                    >= current.terms.max_purchases_per_user
            {
                return Err(Error::Conflict);
            }
            sqlx::query(
                "UPDATE core_subscriptions.external_orders SET confirmed_at=$2 WHERE id=$1",
            )
            .bind(id)
            .bind(now(&mut tx).await?)
            .execute(&mut *tx)
            .await?;
        }
        let out = read(&mut tx, id).await?;
        tx.commit().await?;
        Ok(out)
    }
    pub async fn get_pancake_event(
        &self,
        channel: i64,
        reference: &str,
        event_id: &str,
        hash: &[u8],
    ) -> Result<PancakeEventReceipt> {
        if hash.len() != 32 {
            return Err(Error::Invalid);
        }
        let mut tx = self.begin().await?;
        let id = resolve(&mut tx, channel, reference).await?;
        let out = event(&mut tx, &id, event_id, hash)
            .await?
            .ok_or(Error::NotFound)?;
        tx.commit().await?;
        Ok(out)
    }
    pub async fn accept_pancake_event(
        &self,
        payments: &PaymentsStore,
        channel: i64,
        reference: &str,
        raw: &[u8],
        signature: &[u8],
    ) -> Result<PancakeEventReceipt> {
        let envelope = crate::payments::subscription_pancake::raw_event(raw).map_err(pay_error)?;
        let event_id = envelope
            .get("id")
            .and_then(Value::as_str)
            .filter(|s| !s.is_empty())
            .or_else(|| envelope.get("eventId").and_then(Value::as_str))
            .ok_or(Error::Invalid)?;
        let hash = Sha256::digest(raw).to_vec();
        let mut tx = self.begin().await?;
        let id = resolve(&mut tx, channel, reference).await?;
        let user: i64 = sqlx::query_scalar(
            "SELECT actor_user_id FROM core_subscriptions.external_orders WHERE id=$1",
        )
        .bind(&id)
        .fetch_one(&mut *tx)
        .await?;
        lock(&mut tx, user).await?;
        if let Some(old) = event(&mut tx, &id, event_id, &hash).await? {
            tx.commit().await?;
            return Ok(old);
        }
        let o = read(&mut tx, &id).await?;
        if o.order.confirmed_at.is_none() {
            return Err(Error::Conflict);
        }
        let quoted: i64 = sqlx::query_scalar(
            "SELECT quoted_at FROM core_subscriptions.external_orders WHERE id=$1",
        )
        .bind(&id)
        .fetch_one(&mut *tx)
        .await?;
        let time = now(&mut tx).await?;
        let (c, v) = payments
            .verify_pancake_subscription(
                &o.binding,
                &o.provider_reference,
                o.provider_order_id.as_deref(),
                quoted,
                raw,
                signature,
                time,
            )
            .map_err(pay_error)?;
        if o.provider_order_id
            .as_ref()
            .is_some_and(|p| p != &v.provider_order_id)
        {
            return Err(Error::Conflict);
        }
        let mut aliases = vec![
            ("event", v.event_id.as_str()),
            ("order", v.provider_order_id.as_str()),
        ];
        if v.kind == Change::Paid {
            if let Some(payment) = v.payment_id.as_ref() {
                aliases.push(("payment", payment))
            }
        }
        identifiers::claim(&mut tx, &c, Subject::Subscription(&id), aliases)
            .await
            .map_err(pay_error)?;
        let recurring = o.binding.price.product_type == "subscription";
        let mut start = v.period_start;
        let mut end = v.period_end;
        let mut cycle_key = String::new();
        let mut grant = None;
        let mut historical_paid = false;
        if v.kind == Change::Paid {
            cycle_key = if recurring {
                format!("period:{start}:{end}")
            } else {
                "initial".into()
            };
            let prior=sqlx::query("SELECT provider_order_id,payment_id,period_start,period_end,subscription_id FROM core_subscriptions.pancake_cycles WHERE order_id=$1 AND cycle_key=$2").bind(&id).bind(&cycle_key).fetch_optional(&mut *tx).await?;
            if let Some(old) = prior {
                if old.try_get::<String, _>("provider_order_id")? != v.provider_order_id
                    || old
                        .try_get::<Option<String>, _>("payment_id")?
                        .is_some_and(|p| v.payment_id.as_ref() != Some(&p))
                {
                    return Err(Error::Conflict);
                }
                start = old.try_get("period_start")?;
                end = old.try_get("period_end")?;
                grant = Some(old.try_get("subscription_id")?);
            } else {
                // Paid legacy originals may authenticate another delivery without granting
                // their already served entitlement again. A recurring cutoff is immutable
                // reviewed migration evidence, never inferred from today's plan or clock.
                let historical = historical_payment(&o, recurring, start, end)?;
                if !historical {
                    if !recurring {
                        start = time;
                        end = o.order.plan.terms.ends_at(start)?
                    }
                    let overlap:bool=sqlx::query_scalar("SELECT EXISTS(SELECT FROM core_subscriptions.pancake_cycles WHERE order_id=$1 AND ($4 OR (period_start<$3 AND period_end>$2)))").bind(&id).bind(start).bind(end).bind(!recurring).fetch_one(&mut *tx).await?;
                    if overlap {
                        return Err(Error::Conflict);
                    }
                    let t = &o.order.plan.terms;
                    let sub:i64=sqlx::query_scalar("INSERT INTO core_charging.subscriptions(account_id,grant_reference,period,anchor,seconds,ends_at,limit_credits,models,groups,active) VALUES($1,$2,$3,$4,$5,$6,$7,$8,$9,$10) RETURNING id").bind(o.order.account_id).bind(format!("pancake-subscription:{id}:{cycle_key}")).bind(&t.reset_period).bind(start).bind(t.reset_seconds).bind(end).bind(t.limit_units.parse::<i64>().map_err(|_|Error::Invalid)?).bind(&t.models).bind(&t.groups).bind(o.state!="canceled").fetch_one(&mut *tx).await?;
                    sqlx::query("INSERT INTO core_subscriptions.pancake_cycles(order_id,cycle_key,payment_id,provider_order_id,period_start,period_end,subscription_id,event_id) VALUES($1,$2,$3,$4,$5,$6,$7,$8)").bind(&id).bind(&cycle_key).bind(&v.payment_id).bind(&v.provider_order_id).bind(start).bind(end).bind(sub).bind(&v.event_id).execute(&mut *tx).await?;
                    grant = Some(sub);
                } else {
                    historical_paid = true;
                }
            }
        }
        let mut state = o.state.clone();
        let mut event_created = o.state_created;
        let mut state_event = None;
        if !historical_paid
            && !v.state.is_empty()
            && state != "canceled"
            && v.created >= o.state_created
        {
            state = v.state.clone();
            event_created = v.created;
            state_event = Some(v.event_id.as_str());
        }
        if state == "canceled" {
            sqlx::query("UPDATE core_charging.subscriptions SET active=false WHERE id IN(SELECT subscription_id FROM core_subscriptions.pancake_cycles WHERE order_id=$1) OR (account_id=$2 AND grant_reference=$3)").bind(&id).bind(o.order.account_id).bind(o.legacy_source.as_ref().and_then(|s|s.get("opening_grant_reference")).and_then(Value::as_str)).execute(&mut *tx).await?;
        }
        sqlx::query("UPDATE core_subscriptions.pancake_orders SET provider_order_id=coalesce(provider_order_id,$2),status=$3,event_created=$4,event_id=coalesce($5,event_id),paid_period_start=CASE WHEN $7>paid_period_end THEN $6 ELSE paid_period_start END,paid_period_end=greatest(paid_period_end,$7) WHERE order_id=$1").bind(&id).bind(&v.provider_order_id).bind(&state).bind(event_created).bind(state_event).bind(if v.kind==Change::Paid && !historical_paid {start}else{0}).bind(if v.kind==Change::Paid && !historical_paid {end}else{0}).execute(&mut *tx).await?;
        let receipt = PancakeEventReceipt {
            order_id: id.clone(),
            event_id: v.event_id,
            payload_sha256: hash.clone(),
            kind: if historical_paid {
                "historical_paid"
            } else {
                match v.kind {
                    Change::Paid => "paid",
                    Change::Checkout => "checkout",
                    Change::State => "state",
                }
            }
            .into(),
            state,
            provider_order_id: v.provider_order_id,
            payment_id: v.payment_id,
            cycle_key,
            subscription_id: grant,
            period_start: start,
            period_end: end,
        };
        sqlx::query("INSERT INTO core_subscriptions.pancake_events(order_id,event_id,kind,event_created,payload_sha256,signed_payload,signature,result) VALUES($1,$2,$3,$4,$5,$6,$7,$8)").bind(&id).bind(&receipt.event_id).bind(&receipt.kind).bind(v.created).bind(hash).bind(raw).bind(signature).bind(serde_json::to_value(&receipt).map_err(|_|Error::Invalid)?).execute(&mut *tx).await?;
        tx.commit().await?;
        Ok(receipt)
    }
}

// Source success is historical fact, not authority to issue a second grant.
fn historical_payment(o: &PancakeOrder, recurring: bool, start: i64, end: i64) -> Result<bool> {
    let Some(source) = o.legacy_source.as_ref() else {
        return Ok(false);
    };
    if source.get("source_status").and_then(Value::as_str) != Some("success") {
        return Ok(false);
    }
    if !recurring {
        return Ok(true);
    }
    let cutoff = source
        .get("last_paid_period_end")
        .and_then(Value::as_i64)
        .filter(|v| *v > 0)
        .ok_or(Error::Unavailable)?;
    if end <= cutoff {
        return Ok(true);
    }
    if start < cutoff {
        return Err(Error::Conflict);
    }
    Ok(false)
}

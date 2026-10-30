//! Core subscription sales. A wallet charge and its immutable consumption grant
//! commit together; renewals create new grants and never extend old streams.
mod http;
mod types;
pub use http::router;
pub use types::*;

use crate::{
    accounts::AccountKind,
    billing::{
        Credits,
        ledger::{Action, Outcome, Request as LedgerRequest},
    },
    identity::{self, CredentialKind, Principal},
};
use rand::{RngCore, rngs::OsRng};
use serde::{Serialize, de::DeserializeOwned};
use serde_json::{Value, json};
use sqlx::{PgConnection, PgPool, Postgres, Row, Transaction, types::Json};

pub const SCHEMA: &str = include_str!("../../schema/subscriptions.sql");
const SCOPE: &str = "core-subscription-wallet";
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Error {
    Invalid,
    Unauthorized,
    Forbidden,
    NotFound,
    Conflict,
    Used,
    Unavailable,
}
pub type Result<T> = std::result::Result<T, Error>;
impl From<sqlx::Error> for Error {
    fn from(value: sqlx::Error) -> Self {
        match value.as_database_error().and_then(|e| e.code()).as_deref() {
            Some("23505" | "P1001") => Self::Conflict,
            _ => Self::Unavailable,
        }
    }
}
impl From<identity::IdentityError> for Error {
    fn from(value: identity::IdentityError) -> Self {
        match value {
            identity::IdentityError::Unauthorized => Self::Unauthorized,
            identity::IdentityError::Forbidden => Self::Forbidden,
            identity::IdentityError::Invalid => Self::Invalid,
            identity::IdentityError::Conflict => Self::Conflict,
            identity::IdentityError::Storage => Self::Unavailable,
        }
    }
}
#[derive(Clone)]
pub struct SubscriptionStore {
    pool: PgPool,
}
async fn actor(c: &mut PgConnection, secret: &str, admin: bool) -> Result<Principal> {
    let a = identity::authorize_in_transaction(c, secret).await?;
    if a.credential_kind != CredentialKind::Session
        || a.owner.kind != AccountKind::Personal
        || a.owner.id != a.user_id
    {
        return Err(Error::Forbidden);
    }
    if admin {
        // Plan prices and entitlement limits are platform financial policy.
        // Match the existing RootSettings role, with complete imported policy.
        let role: String = sqlx::query_scalar(
            "SELECT platform_role FROM core_identity.users WHERE id=$1 FOR SHARE",
        )
        .bind(a.user_id)
        .fetch_one(&mut *c)
        .await?;
        if role != "superadmin" {
            return Err(Error::Forbidden);
        }
        if identity::permissions::has_privileged_import_facts(c, a.user_id)
            .await
            .map_err(|_| Error::Unavailable)?
        {
            identity::permissions::load_complete_policy(c, a.user_id)
                .await
                .map_err(|_| Error::Forbidden)?;
        }
    }
    Ok(a)
}
async fn lock(c: &mut PgConnection, actor: i64) -> Result<()> {
    sqlx::query(
        "SELECT pg_advisory_xact_lock(hashtextextended('core-subscriptions:'||$1::text,0))",
    )
    .bind(actor)
    .execute(c)
    .await?;
    Ok(())
}
async fn now(c: &mut PgConnection) -> Result<i64> {
    Ok(
        sqlx::query_scalar("SELECT extract(epoch FROM clock_timestamp())::bigint")
            .fetch_one(c)
            .await?,
    )
}
fn random_id() -> String {
    let mut bytes = [0; 32];
    OsRng.fill_bytes(&mut bytes);
    bytes.iter().map(|v| format!("{v:02x}")).collect()
}
fn decode<T: DeserializeOwned>(value: Value) -> Result<T> {
    serde_json::from_value(value).map_err(|_| Error::Unavailable)
}
async fn prior<T: DeserializeOwned>(
    c: &mut PgConnection,
    user: i64,
    key: &str,
    request: &Value,
) -> Result<Option<T>> {
    let row:Option<(Value,Value)>=sqlx::query_as("SELECT request,result FROM core_subscriptions.commands WHERE actor_user_id=$1 AND operation_key=$2")
        .bind(user).bind(key).fetch_optional(c).await?;
    match row {
        Some((old, result)) if old == *request => Ok(Some(decode(result)?)),
        Some(_) => Err(Error::Conflict),
        None => Ok(None),
    }
}
async fn remember<T: Serialize>(
    c: &mut PgConnection,
    user: i64,
    key: &str,
    request: &Value,
    result: &T,
) -> Result<()> {
    sqlx::query("INSERT INTO core_subscriptions.commands(actor_user_id,operation_key,request,result) VALUES($1,$2,$3,$4)")
        .bind(user).bind(key).bind(request).bind(Json(result)).execute(c).await?;
    Ok(())
}
fn plan_row(r: &sqlx::postgres::PgRow) -> Result<Plan> {
    Ok(Plan {
        id: r.try_get("plan_id")?,
        revision: r.try_get("revision")?,
        archived: r.try_get("archived")?,
        terms: decode(r.try_get("terms")?)?,
    })
}
async fn plan(c: &mut PgConnection, id: i64, write: bool) -> Result<Plan> {
    let suffix = if write { " FOR UPDATE OF p" } else { "" };
    let row=sqlx::query(&format!("SELECT p.id AS plan_id,p.revision,p.archived,r.terms FROM core_subscriptions.plans p JOIN core_subscriptions.plan_revisions r ON r.plan_id=p.id AND r.revision=p.revision WHERE p.id=$1{suffix}"))
        .bind(id).fetch_optional(c).await?.ok_or(Error::NotFound)?;
    plan_row(&row)
}
async fn read_quote(c: &mut PgConnection, id: &str, user: i64) -> Result<Quote> {
    let r=sqlx::query("SELECT q.id,q.plan_id,q.plan_revision AS revision,false AS archived,r.terms,q.starts_at,q.ends_at,q.expires_at,q.renew_from FROM core_subscriptions.quotes q JOIN core_subscriptions.plan_revisions r ON r.plan_id=q.plan_id AND r.revision=q.plan_revision WHERE q.id=$1 AND q.actor_user_id=$2")
        .bind(id).bind(user).fetch_optional(c).await?.ok_or(Error::NotFound)?;
    Ok(Quote {
        id: r.try_get("id")?,
        plan: plan_row(&r)?,
        starts_at: r.try_get("starts_at")?,
        ends_at: r.try_get("ends_at")?,
        expires_at: r.try_get("expires_at")?,
        renew_from: r.try_get("renew_from")?,
    })
}
fn purchase_row(r: &sqlx::postgres::PgRow) -> Result<Purchase> {
    Ok(Purchase {
        id: r.try_get("id")?,
        subscription_id: r.try_get("subscription_id")?,
        plan: plan_row(r)?,
        starts_at: r.try_get("starts_at")?,
        ends_at: r.try_get("ends_at")?,
        used_units: r.try_get("used")?,
        reserved_units: r.try_get("reserved")?,
        entitlement_active: r.try_get("entitlement_active")?,
        renewal_canceled: r.try_get::<Option<i64>, _>("canceled_at")?.is_some(),
        canceled_at: r.try_get("canceled_at")?,
        refund_journal_id: r.try_get("refund_journal_id")?,
    })
}
const PURCHASE_SELECT:&str="SELECT p.id,p.subscription_id,p.canceled_at,p.refund_journal_id,q.plan_id,q.plan_revision AS revision,false AS archived,r.terms,q.starts_at,q.ends_at,
    (s.active AND s.anchor<=extract(epoch FROM clock_timestamp())::bigint AND s.ends_at>extract(epoch FROM clock_timestamp())::bigint) AS entitlement_active,
    coalesce((SELECT sum(t.settled-t.refunded)::text FROM core_charging.requests t WHERE t.source_subscription_id=s.id AND t.state='settled'),'0') AS used,
    coalesce((SELECT sum(t.reserved)::text FROM core_charging.requests t WHERE t.source_subscription_id=s.id AND t.state IN ('reserved','streaming','reconcile')),'0') AS reserved
    FROM core_subscriptions.purchases p JOIN core_subscriptions.quotes q ON q.id=p.quote_id
    JOIN core_subscriptions.plan_revisions r ON r.plan_id=q.plan_id AND r.revision=q.plan_revision
    JOIN core_charging.subscriptions s ON s.id=p.subscription_id";
async fn purchase(c: &mut PgConnection, id: i64, user: i64, write: bool) -> Result<Purchase> {
    let suffix = if write { " FOR UPDATE OF p,s" } else { "" };
    let r = sqlx::query(&format!(
        "{PURCHASE_SELECT} WHERE p.id=$1 AND p.actor_user_id=$2{suffix}"
    ))
    .bind(id)
    .bind(user)
    .fetch_optional(c)
    .await?
    .ok_or(Error::NotFound)?;
    purchase_row(&r)
}
async fn unused(c: &mut PgConnection, p: &Purchase) -> Result<()> {
    if p.refund_journal_id.is_some() {
        return Err(Error::Conflict);
    }
    // Lock s before examining all consumption. The charging engine takes SHARE
    // on that same row; SERIALIZABLE prevents any phantom reservation racing us.
    let used:bool=sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM core_charging.requests WHERE source_subscription_id=$1 AND (state IN ('reserved','streaming','reconcile') OR settled>0)) OR EXISTS(SELECT 1 FROM core_charging.subscription_opening_consumption WHERE subscription_id=$1 AND consumed_credits>0)")
        .bind(p.subscription_id).fetch_one(c).await?;
    if used {
        return Err(Error::Used);
    }
    Ok(())
}
async fn post(c: &mut PgConnection, user: i64, key: String, action: Action) -> Result<Outcome> {
    let request = LedgerRequest {
        scope: SCOPE.into(),
        operation_key: key,
        actor_user_id: Some(user),
        reason: "Confirmed wallet subscription transaction".into(),
        action,
    };
    request.validate().map_err(|_| Error::Invalid)?;
    let Json(outcome) = sqlx::query_scalar("SELECT core_billing.post_ledger($1)")
        .bind(Json(request))
        .fetch_one(c)
        .await?;
    Ok(outcome)
}
impl SubscriptionStore {
    pub fn new(pool: PgPool) -> Self {
        Self { pool }
    }
    async fn begin(&self) -> Result<Transaction<'_, Postgres>> {
        let mut tx = self.pool.begin().await?;
        for sql in [
            "SET TRANSACTION ISOLATION LEVEL SERIALIZABLE",
            "SET LOCAL statement_timeout='10s'",
            "SET LOCAL lock_timeout='1s'",
            "SET LOCAL synchronous_commit=on",
        ] {
            sqlx::query(sql).execute(&mut *tx).await?;
        }
        Ok(tx)
    }
    pub async fn plans(&self, secret: &str, admin: bool) -> Result<Vec<Plan>> {
        let mut tx = self.begin().await?;
        actor(&mut tx, secret, admin).await?;
        let rows=sqlx::query("SELECT p.id AS plan_id,p.revision,p.archived,r.terms FROM core_subscriptions.plans p JOIN core_subscriptions.plan_revisions r ON r.plan_id=p.id AND r.revision=p.revision WHERE $1 OR (NOT p.archived AND (r.terms->>'enabled')::boolean) ORDER BY p.id LIMIT 1000")
            .bind(admin).fetch_all(&mut *tx).await?;
        let plans = rows.iter().map(plan_row).collect::<Result<Vec<_>>>()?;
        tx.commit().await?;
        Ok(plans)
    }
    pub async fn save_plan(&self, secret: &str, id: Option<i64>, input: SavePlan) -> Result<Plan> {
        operation_key(&input.operation_key)?;
        input.terms.validate()?;
        if id.is_some_and(|v| v <= 0)
            || id.is_some() != input.expected_revision.is_some()
            || input.expected_revision.is_some_and(|v| v < 1)
        {
            return Err(Error::Invalid);
        }
        let mut tx = self.begin().await?;
        let a = actor(&mut tx, secret, true).await?;
        lock(&mut tx, a.user_id).await?;
        let request = json!(["save_plan", id, input]);
        if let Some(result) = prior(&mut tx, a.user_id, &input.operation_key, &request).await? {
            tx.commit().await?;
            return Ok(result);
        }
        let (id, revision) = if let Some(id) = id {
            let old = plan(&mut tx, id, true).await?;
            if Some(old.revision) != input.expected_revision {
                return Err(Error::Conflict);
            }
            let revision = old.revision.checked_add(1).ok_or(Error::Conflict)?;
            sqlx::query(
                "UPDATE core_subscriptions.plans SET revision=$2,archived=false WHERE id=$1",
            )
            .bind(id)
            .bind(revision)
            .execute(&mut *tx)
            .await?;
            (id, revision)
        } else {
            (
                sqlx::query_scalar(
                    "INSERT INTO core_subscriptions.plans(revision) VALUES(1) RETURNING id",
                )
                .fetch_one(&mut *tx)
                .await?,
                1,
            )
        };
        let time = now(&mut tx).await?;
        sqlx::query("INSERT INTO core_subscriptions.plan_revisions(plan_id,revision,terms,actor_user_id,created_at) VALUES($1,$2,$3,$4,$5)")
            .bind(id).bind(revision).bind(Json(&input.terms)).bind(a.user_id).bind(time).execute(&mut *tx).await?;
        let result = Plan {
            id,
            revision,
            archived: false,
            terms: input.terms,
        };
        remember(&mut tx, a.user_id, &input.operation_key, &request, &result).await?;
        identity::audit(
            &mut tx,
            &a,
            "subscription.plan.save",
            json!({"plan_id":id,"revision":revision}),
        )
        .await?;
        tx.commit().await?;
        Ok(result)
    }
    pub async fn archive_plan(&self, secret: &str, id: i64, input: ArchivePlan) -> Result<Plan> {
        operation_key(&input.operation_key)?;
        if id <= 0 || input.expected_revision <= 0 {
            return Err(Error::Invalid);
        }
        let mut tx = self.begin().await?;
        let a = actor(&mut tx, secret, true).await?;
        lock(&mut tx, a.user_id).await?;
        let request = json!(["archive_plan", id, input]);
        if let Some(result) = prior(&mut tx, a.user_id, &input.operation_key, &request).await? {
            tx.commit().await?;
            return Ok(result);
        }
        let mut result = plan(&mut tx, id, true).await?;
        if result.revision != input.expected_revision {
            return Err(Error::Conflict);
        }
        sqlx::query("UPDATE core_subscriptions.plans SET archived=true WHERE id=$1")
            .bind(id)
            .execute(&mut *tx)
            .await?;
        result.archived = true;
        remember(&mut tx, a.user_id, &input.operation_key, &request, &result).await?;
        identity::audit(
            &mut tx,
            &a,
            "subscription.plan.archive",
            json!({"plan_id":id}),
        )
        .await?;
        tx.commit().await?;
        Ok(result)
    }
    pub async fn quote(&self, secret: &str, input: QuoteRequest) -> Result<Quote> {
        operation_key(&input.operation_key)?;
        if input.plan_id <= 0 || input.renew_from.is_some_and(|v| v <= 0) {
            return Err(Error::Invalid);
        }
        let mut tx = self.begin().await?;
        let a = actor(&mut tx, secret, false).await?;
        lock(&mut tx, a.user_id).await?;
        if let Some((id,plan_id,renew_from))=sqlx::query_as::<_,(String,i64,Option<i64>)>("SELECT id,plan_id,renew_from FROM core_subscriptions.quotes WHERE actor_user_id=$1 AND operation_key=$2")
            .bind(a.user_id).bind(&input.operation_key).fetch_optional(&mut *tx).await? {
            if plan_id!=input.plan_id || renew_from!=input.renew_from {return Err(Error::Conflict);}
            let quote=read_quote(&mut tx,&id,a.user_id).await?;tx.commit().await?;return Ok(quote);
        }
        let plan = plan(&mut tx, input.plan_id, false).await?;
        if plan.archived || !plan.terms.enabled {
            return Err(Error::Conflict);
        }
        let time = now(&mut tx).await?;
        let mut start = time;
        if let Some(parent) = input.renew_from {
            let old = purchase(&mut tx, parent, a.user_id, true).await?;
            if old.plan.id != plan.id || old.renewal_canceled || old.refund_journal_id.is_some() {
                return Err(Error::Conflict);
            }
            let renewed:bool=sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM core_subscriptions.quotes WHERE renew_from=$1 AND state='posted')").bind(parent).fetch_one(&mut *tx).await?;
            if renewed {
                return Err(Error::Conflict);
            }
            start = start.max(old.ends_at);
        }
        let end = start
            .checked_add(plan.terms.duration_seconds)
            .filter(|v| *v <= 253_402_300_799)
            .ok_or(Error::Invalid)?;
        let result = Quote {
            id: random_id(),
            plan,
            starts_at: start,
            ends_at: end,
            expires_at: time + 300,
            renew_from: input.renew_from,
        };
        sqlx::query("INSERT INTO core_subscriptions.quotes(id,actor_user_id,account_id,operation_key,plan_id,plan_revision,renew_from,starts_at,ends_at,expires_at) VALUES($1,$2,$3,$4,$5,$6,$7,$8,$9,$10)")
            .bind(&result.id).bind(a.user_id).bind(a.owner_account_id).bind(&input.operation_key).bind(result.plan.id).bind(result.plan.revision).bind(result.renew_from).bind(start).bind(end).bind(result.expires_at).execute(&mut *tx).await?;
        tx.commit().await?;
        Ok(result)
    }
    pub async fn confirm(&self, secret: &str, id: &str) -> Result<Receipt> {
        token(id)?;
        let mut tx = self.begin().await?;
        let a = actor(&mut tx, secret, false).await?;
        lock(&mut tx, a.user_id).await?;
        let (account,result)=sqlx::query_as::<_,(i64,Option<Value>)>("SELECT account_id,result FROM core_subscriptions.quotes WHERE id=$1 AND actor_user_id=$2 FOR UPDATE")
            .bind(id).bind(a.user_id).fetch_optional(&mut *tx).await?.ok_or(Error::NotFound)?;
        if account != a.owner_account_id {
            return Err(Error::Forbidden);
        }
        if let Some(result) = result {
            let r = decode(result)?;
            tx.commit().await?;
            return Ok(r);
        }
        let q = read_quote(&mut tx, id, a.user_id).await?;
        let time = now(&mut tx).await?;
        let current = plan(&mut tx, q.plan.id, true).await?;
        if q.expires_at <= time
            || q.ends_at <= time
            || current.archived
            || !current.terms.enabled
            || current.revision != q.plan.revision
        {
            return Err(Error::Conflict);
        }
        if let Some(parent) = q.renew_from {
            let old = purchase(&mut tx, parent, a.user_id, true).await?;
            if old.renewal_canceled || old.refund_journal_id.is_some() {
                return Err(Error::Conflict);
            }
            let renewed:bool=sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM core_subscriptions.quotes WHERE renew_from=$1 AND state='posted')").bind(parent).fetch_one(&mut *tx).await?;
            if renewed {
                return Err(Error::Conflict);
            }
        }
        if q.plan.terms.max_purchases_per_user > 0 {
            let n:i64=sqlx::query_scalar("SELECT count(*) FROM core_subscriptions.purchases p JOIN core_subscriptions.quotes q ON q.id=p.quote_id WHERE p.actor_user_id=$1 AND q.plan_id=$2 AND p.refund_journal_id IS NULL")
                .bind(a.user_id).bind(q.plan.id).fetch_one(&mut *tx).await?;
            if n >= q.plan.terms.max_purchases_per_user {
                return Err(Error::Conflict);
            }
        }
        crate::financial_profile::read_wallet(&mut tx, a.user_id)
            .await
            .map_err(|_| Error::Unavailable)?;
        sqlx::query("SELECT core_billing.open_ledger_wallet($1)")
            .bind(account)
            .execute(&mut *tx)
            .await?;
        let amount = units(&q.plan.terms.price_units)?;
        let outcome = post(
            &mut tx,
            a.user_id,
            format!("buy:{id}"),
            Action::Charge {
                account_id: account,
                amount_units: Credits::try_from(amount).map_err(|_| Error::Invalid)?,
            },
        )
        .await?;
        let mut receipt = Receipt {
            quote_id: id.into(),
            status: "rejected".into(),
            purchase_id: None,
            subscription_id: None,
            journal_id: None,
            amount_units: q.plan.terms.price_units.clone(),
            rejection: None,
        };
        match outcome {
            Outcome::Rejected { code } => {
                receipt.rejection = Some(
                    serde_json::to_value(code)
                        .map_err(|_| Error::Unavailable)?
                        .as_str()
                        .ok_or(Error::Unavailable)?
                        .to_owned(),
                )
            }
            Outcome::Posted { journal_id, .. } => {
                let sub:i64=sqlx::query_scalar("INSERT INTO core_charging.subscriptions(account_id,grant_reference,period,anchor,seconds,ends_at,limit_credits,models,groups) VALUES($1,$2,$3,$4,$5,$6,$7,$8,$9) RETURNING id")
                    .bind(account).bind(format!("subscription:{id}")).bind(&q.plan.terms.reset_period).bind(q.starts_at).bind(q.plan.terms.reset_seconds).bind(q.ends_at).bind(units(&q.plan.terms.limit_units)?).bind(&q.plan.terms.models).bind(&q.plan.terms.groups).fetch_one(&mut *tx).await?;
                let purchase:i64=sqlx::query_scalar("INSERT INTO core_subscriptions.purchases(quote_id,actor_user_id,account_id,subscription_id,charge_journal_id) VALUES($1,$2,$3,$4,$5) RETURNING id")
                    .bind(id).bind(a.user_id).bind(account).bind(sub).bind(journal_id).fetch_one(&mut *tx).await?;
                receipt.status = "posted".into();
                receipt.purchase_id = Some(purchase);
                receipt.subscription_id = Some(sub);
                receipt.journal_id = Some(journal_id);
            }
        }
        sqlx::query("UPDATE core_subscriptions.quotes SET state=$2,result=$3 WHERE id=$1")
            .bind(id)
            .bind(&receipt.status)
            .bind(Json(&receipt))
            .execute(&mut *tx)
            .await?;
        identity::audit(&mut tx, &a, "subscription.purchase", json!(&receipt)).await?;
        tx.commit().await?;
        Ok(receipt)
    }
    pub async fn purchases(&self, secret: &str) -> Result<Vec<Purchase>> {
        let mut tx = self.begin().await?;
        let a = actor(&mut tx, secret, false).await?;
        let rows = sqlx::query(&format!(
            "{PURCHASE_SELECT} WHERE p.actor_user_id=$1 ORDER BY p.id DESC LIMIT 1000"
        ))
        .bind(a.user_id)
        .fetch_all(&mut *tx)
        .await?;
        let result = rows.iter().map(purchase_row).collect::<Result<Vec<_>>>()?;
        tx.commit().await?;
        Ok(result)
    }
    /// Cancellation prevents renewals from this record. All already paid
    /// entitlements, including the current period, remain valid until expiry.
    pub async fn cancel(&self, secret: &str, id: i64) -> Result<Purchase> {
        if id <= 0 {
            return Err(Error::Invalid);
        }
        let mut tx = self.begin().await?;
        let a = actor(&mut tx, secret, false).await?;
        lock(&mut tx, a.user_id).await?;
        let p = purchase(&mut tx, id, a.user_id, true).await?;
        if p.canceled_at.is_none() {
            let time = now(&mut tx).await?;
            sqlx::query("UPDATE core_subscriptions.purchases SET canceled_at=$2 WHERE id=$1")
                .bind(id)
                .bind(time)
                .execute(&mut *tx)
                .await?;
            identity::audit(
                &mut tx,
                &a,
                "subscription.cancel_renewal",
                json!({"purchase_id":id}),
            )
            .await?;
        }
        let result = purchase(&mut tx, id, a.user_id, false).await?;
        tx.commit().await?;
        Ok(result)
    }
    pub async fn preview_refund(&self, secret: &str, id: i64) -> Result<RefundQuote> {
        if id <= 0 {
            return Err(Error::Invalid);
        }
        let mut tx = self.begin().await?;
        let a = actor(&mut tx, secret, false).await?;
        lock(&mut tx, a.user_id).await?;
        let p = purchase(&mut tx, id, a.user_id, true).await?;
        unused(&mut tx, &p).await?;
        let time = now(&mut tx).await?;
        if p.ends_at <= time {
            return Err(Error::Conflict);
        }
        let result = RefundQuote {
            id: random_id(),
            purchase_id: id,
            amount_units: p.plan.terms.price_units,
            expires_at: time + 300,
        };
        sqlx::query("INSERT INTO core_subscriptions.refund_quotes(id,purchase_id,actor_user_id,expires_at) VALUES($1,$2,$3,$4)")
            .bind(&result.id).bind(id).bind(a.user_id).bind(result.expires_at).execute(&mut *tx).await?;
        tx.commit().await?;
        Ok(result)
    }
    pub async fn refund(&self, secret: &str, id: &str) -> Result<Receipt> {
        token(id)?;
        let mut tx = self.begin().await?;
        let a = actor(&mut tx, secret, false).await?;
        lock(&mut tx, a.user_id).await?;
        let (purchase_id,expires,result)=sqlx::query_as::<_,(i64,i64,Option<Value>)>("SELECT purchase_id,expires_at,result FROM core_subscriptions.refund_quotes WHERE id=$1 AND actor_user_id=$2 FOR UPDATE")
            .bind(id).bind(a.user_id).fetch_optional(&mut *tx).await?.ok_or(Error::NotFound)?;
        if let Some(result) = result {
            let r = decode(result)?;
            tx.commit().await?;
            return Ok(r);
        }
        let p = purchase(&mut tx, purchase_id, a.user_id, true).await?;
        let time = now(&mut tx).await?;
        if expires <= time || p.ends_at <= time {
            return Err(Error::Conflict);
        }
        unused(&mut tx, &p).await?;
        let (account, original): (i64, i64) = sqlx::query_as(
            "SELECT account_id,charge_journal_id FROM core_subscriptions.purchases WHERE id=$1",
        )
        .bind(purchase_id)
        .fetch_one(&mut *tx)
        .await?;
        if account != a.owner_account_id {
            return Err(Error::Forbidden);
        }
        let outcome = post(
            &mut tx,
            a.user_id,
            format!("refund:{id}"),
            Action::Refund {
                journal_id: original,
                amount_units: Credits::try_from(units(&p.plan.terms.price_units)?)
                    .map_err(|_| Error::Invalid)?,
            },
        )
        .await?;
        let Outcome::Posted { journal_id, .. } = outcome else {
            return Err(Error::Conflict);
        };
        sqlx::query("UPDATE core_charging.subscriptions SET active=false WHERE id=$1")
            .bind(p.subscription_id)
            .execute(&mut *tx)
            .await?;
        sqlx::query("UPDATE core_subscriptions.purchases SET refund_journal_id=$2,canceled_at=coalesce(canceled_at,$3) WHERE id=$1")
            .bind(purchase_id).bind(journal_id).bind(time).execute(&mut *tx).await?;
        let receipt = Receipt {
            quote_id: id.into(),
            status: "refunded".into(),
            purchase_id: Some(purchase_id),
            subscription_id: Some(p.subscription_id),
            journal_id: Some(journal_id),
            amount_units: p.plan.terms.price_units,
            rejection: None,
        };
        sqlx::query("UPDATE core_subscriptions.refund_quotes SET result=$2 WHERE id=$1")
            .bind(id)
            .bind(Json(&receipt))
            .execute(&mut *tx)
            .await?;
        identity::audit(&mut tx, &a, "subscription.refund", json!(&receipt)).await?;
        tx.commit().await?;
        Ok(receipt)
    }
}

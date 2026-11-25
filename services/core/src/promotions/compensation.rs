//! Operator-funded compensation. A Core console owner publishes the private option.
//! A confirmed wallet transfer is the claim; no second balance or mutable claim table.
use super::*;
use crate::billing::ledger::Wallet;
use crate::community_read::personal_spending;
use axum::{
    Json as HttpJson,
    body::to_bytes,
    extract::{Path, Request, State},
    http::HeaderMap,
};
use serde::{Deserialize, Serialize};
use std::collections::{HashMap, HashSet};

const OPTION: &str = "compensation_gifts.v1";
const SCOPE: &str = "core-compensation-gift";
const JS_MAX: i64 = (1_i64 << 53) - 1;

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Policy {
    treasury_account_id: i64,
    campaigns: Vec<CampaignRow>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct CampaignRow {
    id: i64,
    title: String,
    description: String,
    quota: i64,
    budget_quota: i64,
    max_claims: i64,
    start_at: i64,
    end_at: i64,
    min_used_quota: i64,
    min_account_age_days: i64,
    enabled: bool,
    created_at: i64,
}

#[derive(Serialize)]
struct Receipt {
    id: i64,
    gift_id: i64,
    user_id: i64,
    quota: i64,
    created_at: i64,
}

fn key(id: i64, user: i64) -> String {
    format!("{id}/{user}")
}

async fn policy(c: &mut PgConnection) -> Result<Option<Policy>> {
    let row: Option<String> = sqlx::query_scalar(
        "SELECT CASE WHEN octet_length(value)<=131072 THEN value ELSE '' END FROM core_console.options WHERE key=$1",
    ).bind(OPTION).fetch_optional(c).await?;
    let Some(raw) = row else { return Ok(None) };
    let p: Policy = serde_json::from_str(&raw).map_err(|_| Error::Unavailable)?;
    let mut ids = HashSet::new();
    if p.treasury_account_id <= 0
        || p.campaigns.len() > 128
        || p.campaigns.iter().any(|g| {
            !(1..=JS_MAX).contains(&g.id)
                || !ids.insert(g.id)
                || g.title.trim().is_empty()
                || g.title.chars().count() > 64
                || g.description.chars().count() > 255
                || !(1..=JS_MAX).contains(&g.quota)
                || !(g.quota..=JS_MAX).contains(&g.budget_quota)
                || !(1..=1_000_000).contains(&g.max_claims)
                || !(0..=JS_MAX).contains(&g.min_used_quota)
                || !(0..=36500).contains(&g.min_account_age_days)
                || g.created_at <= 0
                || g.start_at < 0
                || g.end_at <= g.start_at
                || g.end_at > 253_402_300_799
        })
    {
        return Err(Error::Unavailable);
    }
    Ok(Some(p))
}

async fn now(c: &mut PgConnection) -> Result<i64> {
    Ok(
        sqlx::query_scalar("SELECT floor(extract(epoch FROM clock_timestamp()))::bigint")
            .fetch_one(c)
            .await?,
    )
}

async fn receipts(
    c: &mut PgConnection,
    a: &Principal,
    ids: &[i64],
) -> Result<HashMap<i64, Receipt>> {
    if ids.is_empty() {
        return Ok(HashMap::new());
    }
    let keys: Vec<_> = ids.iter().map(|id| key(*id, a.user_id)).collect();
    let rows = sqlx::query(
        "SELECT j.id,j.operation_key,j.amount_units,floor(extract(epoch FROM j.created_at))::bigint AS at
         FROM core_billing.ledger_journals j JOIN core_billing.ledger_accounts destination ON destination.id=j.payee_id
         WHERE j.scope=$1 AND j.operation_key=ANY($2) AND j.kind='transfer'
           AND j.actor_user_id=$3 AND destination.owner_account_id=$4 AND destination.bucket='wallet'"
    ).bind(SCOPE).bind(keys).bind(a.user_id).bind(a.owner_account_id).fetch_all(c).await?;
    let mut out = HashMap::new();
    for row in rows {
        let op: String = row.try_get("operation_key")?;
        let gift_id = op
            .split_once('/')
            .and_then(|(id, _)| id.parse::<i64>().ok())
            .ok_or(Error::Unavailable)?;
        let receipt = Receipt {
            id: row.try_get("id")?,
            gift_id,
            user_id: a.user_id,
            quota: row.try_get("amount_units")?,
            created_at: row.try_get("at")?,
        };
        if !(1..=JS_MAX).contains(&receipt.id) {
            return Err(Error::Unavailable);
        }
        out.insert(gift_id, receipt);
    }
    Ok(out)
}

/// Present only awards the configured operator wallet and campaign can fund now.
/// Claim rechecks current authority and capacity under the policy lock.
async fn available_awards(
    c: &mut PgConnection,
    treasury: i64,
    gifts: &[&CampaignRow],
) -> Result<HashSet<i64>> {
    if gifts.is_empty() {
        return Ok(HashSet::new());
    }
    let ids: Vec<_> = gifts.iter().map(|g| g.id).collect();
    let amounts: Vec<_> = gifts.iter().map(|g| g.quota).collect();
    let caps: Vec<_> = gifts.iter().map(|g| g.budget_quota).collect();
    let limits: Vec<_> = gifts.iter().map(|g| g.max_claims).collect();
    let ids:Vec<i64>=sqlx::query_scalar(
        "SELECT g.id FROM unnest($1::bigint[],$2::bigint[],$3::bigint[],$4::bigint[]) g(id,amount,budget,cap)
         JOIN core_identity.users u ON u.personal_account_id=$5 AND u.active AND u.platform_role='superadmin'
         JOIN core_identity.accounts a ON a.id=u.personal_account_id AND a.active AND a.kind='personal'
         JOIN core_billing.ledger_accounts w ON w.owner_account_id=a.id AND w.bucket='wallet'
         JOIN core_billing.balance_state b ON b.ledger_account_id=w.id AND b.balance_units>=g.amount
         CROSS JOIN LATERAL (SELECT count(*) AS claims,coalesce(sum(j.amount_units),0) AS spent
           FROM core_billing.ledger_journals j WHERE j.scope=$6 AND j.kind='transfer'
           AND j.operation_key LIKE g.id::text||'/%') totals
         WHERE totals.claims<g.cap AND totals.spent+g.amount<=g.budget"
    ).bind(ids).bind(amounts).bind(caps).bind(limits).bind(treasury).bind(SCOPE).fetch_all(c).await?;
    Ok(ids.into_iter().collect())
}

struct Eligibility {
    age_days: i64,
    spent: Option<u128>,
}
impl Eligibility {
    async fn read(
        c: &mut PgConnection,
        a: &Principal,
        time: i64,
        gifts: &[&CampaignRow],
    ) -> Result<Self> {
        let age_days = if gifts.iter().any(|g| g.min_account_age_days > 0) {
            let created:i64 = sqlx::query_scalar(
                "SELECT floor(extract(epoch FROM coalesce(o.registered_at,account.created_at)))::bigint
                 FROM core_identity.accounts account JOIN core_identity.users u ON u.personal_account_id=account.id
                 LEFT JOIN core_abuse.registration_observations o ON o.user_id=u.id WHERE u.id=$1 AND account.id=$2",
            ).bind(a.user_id).bind(a.owner_account_id).fetch_one(&mut *c).await?;
            time.saturating_sub(created) / 86400
        } else {
            0
        };
        let spent = if gifts.iter().any(|g| g.min_used_quota > 0) {
            personal_spending(c, a.user_id, a.owner_account_id, time)
                .await
                .map_err(|_| Error::Unavailable)?
        } else {
            Some(0)
        };
        Ok(Self { age_days, spent })
    }
    fn reason(&self, g: &CampaignRow, time: i64) -> Option<&'static str> {
        if time < g.start_at {
            Some("This gift has not started.")
        } else if time >= g.end_at {
            Some("This gift has expired.")
        } else if self.age_days < g.min_account_age_days {
            Some("Account age requirement not met.")
        } else if g.min_used_quota > 0 && self.spent.is_none() {
            Some("Personal spending could not be verified.")
        } else if self
            .spent
            .is_some_and(|spent| spent < g.min_used_quota as u128)
        {
            Some("Usage requirement not met.")
        } else {
            None
        }
    }
}

pub(super) async fn list(
    State(s): State<PromotionStore>,
    headers: HeaderMap,
) -> Result<HttpJson<Value>> {
    let secret = super::http::bearer(&headers)?;
    let mut tx = s.begin().await?;
    let a = actor(&mut tx, secret, false).await?;
    let p = policy(&mut tx).await?;
    let mut items = Vec::new();
    if let Some(p) = p {
        let time = now(&mut tx).await?;
        let mut gifts: Vec<_> = p
            .campaigns
            .iter()
            .filter(|g| g.enabled && g.end_at > time - 7 * 86400)
            .collect();
        gifts.sort_by_key(|g| std::cmp::Reverse(g.id));
        let ids: Vec<_> = gifts.iter().map(|g| g.id).collect();
        let known = receipts(&mut tx, &a, &ids).await?;
        let active: Vec<_> = gifts
            .iter()
            .copied()
            .filter(|g| !known.contains_key(&g.id) && (g.start_at..g.end_at).contains(&time))
            .collect();
        let eligibility = Eligibility::read(&mut tx, &a, time, &active).await?;
        let available = available_awards(&mut tx, p.treasury_account_id, &active).await?;
        for g in gifts {
            let claim = known.get(&g.id);
            let reason = if claim.is_some() {
                None
            } else {
                eligibility.reason(g, time).or_else(|| {
                    (a.owner_account_id == p.treasury_account_id || !available.contains(&g.id))
                        .then_some("This gift is currently unavailable.")
                })
            };
            items.push(json!({"id":g.id,"title":g.title,"description":g.description,"quota":claim.map_or(g.quota,|r|r.quota),
                "start_at":g.start_at,"end_at":g.end_at,"enabled":true,"created_at":g.created_at,
                "min_used_quota":g.min_used_quota,"min_account_age_days":g.min_account_age_days,
                "claimed":claim.is_some(),"claimed_at":claim.map(|r|r.created_at),"eligible":claim.is_none() && reason.is_none(),"reason":reason}));
        }
    }
    tx.commit().await?;
    Ok(HttpJson(json!({"success":true,"data":items})))
}

pub(super) async fn claim(
    State(s): State<PromotionStore>,
    Path(id): Path<i64>,
    request: Request,
) -> Result<HttpJson<Value>> {
    let secret = super::http::bearer(request.headers())?.to_owned();
    if !(1..=JS_MAX).contains(&id)
        || request.uri().query().is_some()
        || to_bytes(request.into_body(), 0).await.is_err()
    {
        return Err(Error::Invalid);
    }
    // Under READ COMMITTED, a claimant waiting behind the policy lock sees the committed
    // winning receipt and budget. This same lock also protects against concurrent option updates.
    let mut tx = s.pool.begin().await?;
    sqlx::query("SET TRANSACTION ISOLATION LEVEL READ COMMITTED")
        .execute(&mut *tx)
        .await?;
    sqlx::query("SET LOCAL statement_timeout='5s'")
        .execute(&mut *tx)
        .await?;
    sqlx::query("SET LOCAL synchronous_commit=on")
        .execute(&mut *tx)
        .await?;
    let a = actor(&mut tx, &secret, false).await?;
    sqlx::query("SELECT core_console.lock_public_options()")
        .execute(&mut *tx)
        .await?;
    if let Some(receipt) = receipts(&mut tx, &a, &[id]).await?.remove(&id) {
        tx.commit().await?;
        return Ok(HttpJson(
            json!({"success":true,"data":{"already_claimed":true,"claim":receipt}}),
        ));
    }
    let p = policy(&mut tx).await?.ok_or(Error::NotFound)?;
    let g = p
        .campaigns
        .iter()
        .find(|g| g.id == id && g.enabled)
        .ok_or(Error::NotFound)?;
    let time = now(&mut tx).await?;
    if !(g.start_at..g.end_at).contains(&time) {
        return Err(Error::Forbidden);
    }
    let eligibility = Eligibility::read(&mut tx, &a, time, &[g]).await?;
    if g.min_used_quota > 0 && eligibility.spent.is_none() {
        return Err(Error::Unavailable);
    }
    if eligibility.reason(g, time).is_some() {
        return Err(Error::Forbidden);
    }
    if a.owner_account_id == p.treasury_account_id {
        return Err(Error::Forbidden);
    }
    let source:Option<i64>=sqlx::query_scalar(
        "SELECT a.id FROM core_identity.users u JOIN core_identity.accounts a ON u.personal_account_id=a.id
         WHERE a.id=$1 AND u.platform_role='superadmin' AND u.active AND a.active AND a.kind='personal' FOR SHARE OF u,a"
    ).bind(p.treasury_account_id).fetch_optional(&mut *tx).await?;
    if source.is_none() {
        return Err(Error::Unavailable);
    }
    if !available_awards(&mut tx, p.treasury_account_id, &[g])
        .await?
        .contains(&id)
    {
        return Err(Error::Exhausted);
    }
    let _: Json<Wallet> = sqlx::query_scalar("SELECT core_billing.open_ledger_wallet($1)")
        .bind(a.owner_account_id)
        .fetch_one(&mut *tx)
        .await?;
    let operation = LedgerRequest {
        scope: SCOPE.into(),
        operation_key: key(id, a.user_id),
        actor_user_id: Some(a.user_id),
        reason: "Operator-funded compensation gift".into(),
        action: Action::Transfer {
            from_account_id: p.treasury_account_id,
            to_account_id: a.owner_account_id,
            amount_units: Credits::try_from(g.quota).map_err(|_| Error::Unavailable)?,
        },
    };
    operation.validate().map_err(|_| Error::Unavailable)?;
    let Json(outcome): Json<Outcome> = sqlx::query_scalar("SELECT core_billing.post_ledger($1)")
        .bind(Json(operation))
        .fetch_one(&mut *tx)
        .await?;
    let Outcome::Posted { journal_id, .. } = outcome else {
        return Err(Error::Exhausted);
    };
    let receipt = receipts(&mut tx, &a, &[id])
        .await?
        .remove(&id)
        .filter(|r| r.id == journal_id)
        .ok_or(Error::Unavailable)?;
    tx.commit().await?;
    Ok(HttpJson(
        json!({"success":true,"data":{"already_claimed":false,"claim":receipt}}),
    ))
}

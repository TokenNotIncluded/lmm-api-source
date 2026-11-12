use super::*;
use crate::{
    accounts::AccountKind,
    identity::{self, CredentialKind, Principal},
};
use rand::{RngCore, rngs::OsRng};
impl CheckinStore {
    pub async fn enabled(&self) -> Result<bool> {
        let value: Option<Json<Policy>> = sqlx::query_scalar("SELECT r.policy FROM core_checkins.active_policy a JOIN core_checkins.policy_revisions r ON r.revision=a.revision WHERE a.singleton")
            .fetch_optional(&self.pool).await?;
        Ok(value.is_some_and(|p| p.0.enabled))
    }

    pub async fn status(&self, secret: &str, month: Option<&str>) -> Result<Status> {
        let mut tx = self.begin().await?;
        let actor = actor(&mut tx, secret).await?;
        let today = calendar_day(now(&mut tx).await?)?;
        let month = month.unwrap_or(&today[..7]).to_owned();
        validate_month(&month)?;
        let policy = policy(&mut tx).await?;
        let range = match &policy {
            Some(p) if p.policy.enabled => {
                Some(reward_range(&mut tx, actor.user_id, &p.policy).await?)
            }
            _ => None,
        };
        let today_record = daily(&mut tx, actor.owner_account_id, &today).await?;
        let records = sqlx::query_scalar::<_, Json<DailyRecord>>("SELECT result FROM core_checkins.days WHERE account_id=$1 AND left(checkin_date::text,7)=$2 ORDER BY checkin_date DESC")
            .bind(actor.owner_account_id).bind(&month).fetch_all(&mut *tx).await?
            .into_iter().map(|r| r.0).collect();
        let totals = sqlx::query("SELECT count(*)::bigint AS count,coalesce(sum(amount_units::numeric),0)::text AS total FROM core_checkins.days WHERE account_id=$1")
            .bind(actor.owner_account_id).fetch_one(&mut *tx).await?;
        let result = Status {
            enabled: policy.as_ref().is_some_and(|p| p.policy.enabled),
            today,
            month,
            calendar_offset_seconds: CALENDAR_OFFSET_SECONDS,
            policy_revision: policy.as_ref().map(|p| p.revision),
            checked_in_today: today_record.is_some(),
            reward_range: range,
            today_record,
            records,
            total_checkins: totals.try_get("count")?,
            total_quota_awarded: totals.try_get("total")?,
        };
        tx.commit().await?;
        Ok(result)
    }

    pub async fn claim(&self, secret: &str, input: ClaimRequest) -> Result<ClaimResult> {
        valid_key(&input.operation_key)?;
        let mut tx = self.begin().await?;
        let actor = actor(&mut tx, secret).await?;
        sqlx::query(
            "SELECT pg_advisory_xact_lock_shared(hashtextextended('core-checkins-owner',0))",
        )
        .execute(&mut *tx)
        .await?;
        sqlx::query("SELECT pg_advisory_xact_lock(hashtextextended('core-checkins:'||$1::text,0))")
            .bind(actor.owner_account_id)
            .execute(&mut *tx)
            .await?;
        // Check the durable operation before the clock, policy or current trust.
        let prior: Option<Json<ClaimResult>> = sqlx::query_scalar(
            "SELECT result FROM core_checkins.claim_commands WHERE user_id=$1 AND operation_key=$2",
        )
        .bind(actor.user_id)
        .bind(&input.operation_key)
        .fetch_optional(&mut *tx)
        .await?;
        if let Some(prior) = prior {
            tx.commit().await?;
            return Ok(prior.0);
        }
        let at = now(&mut tx).await?;
        let today = calendar_day(at)?;
        let result = if let Some(record) = daily(&mut tx, actor.owner_account_id, &today).await? {
            ClaimResult {
                already_claimed: true,
                record,
            }
        } else {
            let published = policy(&mut tx).await?.ok_or(Error::Unavailable)?;
            if !published.policy.enabled {
                return Err(Error::Forbidden);
            }
            if crate::abuse::is_marked_in_transaction(&mut tx, actor.user_id)
                .await
                .map_err(|_| Error::Unavailable)?
            {
                return Err(Error::Forbidden);
            }
            let range = reward_range(&mut tx, actor.user_id, &published.policy).await?;
            let amount = draw(integer(&range.min_quota)?, integer(&range.max_quota)?)?;
            let journal_id = gift(&mut tx, &actor, &today, amount).await?;
            let record = DailyRecord {
                checkin_date: today.clone(),
                quota_awarded: amount.to_string(),
                created_at: at,
                journal_id,
                origin: "native".into(),
                policy_revision: Some(published.revision),
                trust_level: Some(range.trust_level),
            };
            sqlx::query("INSERT INTO core_checkins.days(account_id,user_id,checkin_date,amount_units,origin,policy_revision,journal_id,created_at,result) VALUES($1,$2,$3::text::date,$4,'native',$5,$6,$7,$8)")
                .bind(actor.owner_account_id).bind(actor.user_id).bind(&today).bind(amount)
                .bind(published.revision).bind(journal_id).bind(at).bind(Json(&record)).execute(&mut *tx).await?;
            ClaimResult {
                already_claimed: false,
                record,
            }
        };
        sqlx::query("INSERT INTO core_checkins.claim_commands(user_id,operation_key,result) VALUES($1,$2,$3)")
            .bind(actor.user_id).bind(input.operation_key).bind(Json(&result)).execute(&mut *tx).await?;
        tx.commit().await?;
        Ok(result)
    }
}

async fn actor(c: &mut PgConnection, secret: &str) -> Result<Principal> {
    let actor = identity::authorize_in_transaction(c, secret).await?;
    if actor.credential_kind != CredentialKind::Session
        || actor.owner.kind != AccountKind::Personal
        || actor.owner.id != actor.user_id
    {
        return Err(Error::Forbidden);
    }
    let bound: bool = sqlx::query_scalar("SELECT EXISTS(SELECT FROM core_identity.users u JOIN core_identity.accounts a ON a.id=u.personal_account_id WHERE u.id=$1 AND u.personal_account_id=$2 AND a.kind='personal' AND u.active AND a.active)")
        .bind(actor.user_id).bind(actor.owner_account_id).fetch_one(c).await?;
    if !bound {
        return Err(Error::Forbidden);
    }
    Ok(actor)
}

fn validate_month(month: &str) -> Result<()> {
    if month.len() != 7
        || month.starts_with("0000")
        || NaiveDate::parse_from_str(&format!("{month}-01"), "%Y-%m-%d").is_err()
        || !month.bytes().enumerate().all(|(i, b)| {
            if i == 4 {
                b == b'-'
            } else {
                b.is_ascii_digit()
            }
        })
    {
        return Err(Error::Invalid);
    }
    Ok(())
}
async fn reward_range(c: &mut PgConnection, user: i64, policy: &Policy) -> Result<RewardRange> {
    let authority = crate::billing::trust_pricing::quote_in_transaction(c, user)
        .await
        .map_err(|error| match error {
            crate::billing::trust_pricing::Error::Retry => Error::Retry,
            _ => Error::Unavailable,
        })?;
    policy.reward_range(authority.evidence().level())
}
fn draw(min: i64, max: i64) -> Result<i64> {
    if min < 0 || max < min {
        return Err(Error::Invalid);
    }
    if min == max {
        return Ok(min);
    }
    let width = (max - min) as u64 + 1;
    let threshold = width.wrapping_neg() % width;
    loop {
        let mut bytes = [0; 8];
        OsRng
            .try_fill_bytes(&mut bytes)
            .map_err(|_| Error::Unavailable)?;
        let value = u64::from_le_bytes(bytes);
        if value >= threshold {
            return Ok(min + (value % width) as i64);
        }
    }
}
async fn gift(
    c: &mut PgConnection,
    actor: &Principal,
    day: &str,
    amount: i64,
) -> Result<Option<i64>> {
    if amount == 0 {
        return Ok(None);
    }
    use crate::billing::{
        Credits,
        ledger::{Action, Outcome, Request},
    };
    crate::financial_profile::read_wallet(c, actor.user_id)
        .await
        .map_err(|_| Error::Unavailable)?;
    sqlx::query("SELECT core_billing.open_ledger_wallet($1)")
        .bind(actor.owner_account_id)
        .execute(&mut *c)
        .await?;
    let request = Request {
        scope: "core-checkins".into(),
        operation_key: format!("daily:{}:{day}", actor.owner_account_id),
        actor_user_id: Some(actor.user_id),
        reason: "Daily check-in reward".into(),
        action: Action::Gift {
            account_id: actor.owner_account_id,
            amount_units: Credits::try_from(amount).map_err(|_| Error::Invalid)?,
        },
    };
    request.validate().map_err(|_| Error::Invalid)?;
    let Json(outcome): Json<Outcome> = sqlx::query_scalar("SELECT core_billing.post_ledger($1)")
        .bind(Json(request))
        .fetch_one(c)
        .await?;
    match outcome {
        Outcome::Posted { journal_id, .. } => Ok(Some(journal_id)),
        _ => Err(Error::Conflict),
    }
}

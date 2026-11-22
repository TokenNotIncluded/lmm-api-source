//! One-time reset rights. A reset appends a consumption generation; it neither
//! edits prior requests nor moves money or a subscription's reset calendar.
use super::*;
use serde::Deserialize;
use sha2::{Digest, Sha256};
use std::collections::{BTreeMap, BTreeSet};

pub const RESET_SCHEMA: &str = include_str!("../../schema/subscription_reset.sql");
const MAX_EXPIRY: i64 = 253_402_300_799;
const MAX_ID: i64 = 9_007_199_254_740_991;

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq, PartialOrd, Ord)]
#[serde(deny_unknown_fields)]
pub struct ResetTarget {
    pub user_id: i64,
    pub plan_id: i64,
}
#[derive(Clone, Debug, Default, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct ResetFilter {
    pub query: String,
    pub plan_id: i64,
    pub plan_ids: Vec<i64>,
    pub user_ids: Vec<i64>,
}
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ResetPreviewRequest {
    pub mode: String,
    #[serde(default)]
    pub targets: Vec<ResetTarget>,
    #[serde(default)]
    pub all_matching: bool,
    #[serde(default)]
    pub filter: Option<ResetFilter>,
    pub voucher_expires_at: Option<i64>,
}
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ResetExecuteRequest {
    pub operation_id: String,
    pub preview_token: String,
}
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ResetCodeRequest {
    pub operation_key: String,
    pub code: String,
}
#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
struct FrozenGrant {
    id: i64,
    user_id: i64,
    plan_id: i64,
    window_start: i64,
    window_end: i64,
    limit_credits: i64,
    unlimited: bool,
    used: i64,
    reserved: i64,
    previous_reset_id: Option<i64>,
    reset_basis: String,
    reset_limit: i64,
    reset_unlimited: bool,
    /// Freeze the entire immutable grant policy, including its actual owner,
    /// models, groups, original lifetime and the intentional active flag.
    grant: Value,
}
impl FrozenGrant {
    fn target(&self) -> ResetTarget {
        ResetTarget {
            user_id: self.user_id,
            plan_id: self.plan_id,
        }
    }
    fn after_basis(&self) -> (i64, bool) {
        if self.reset_basis == "explicit" {
            (self.reset_limit, self.reset_unlimited)
        } else {
            (self.limit_credits, self.unlimited)
        }
    }
    fn restored(&self) -> Result<i64> {
        // cc52 resetRestoredQuota: an explicit, possibly refund-rebased reset
        // basis replaces remaining quota. Renewal price/basis is unrelated.
        if self.reset_basis == "explicit" {
            let remaining = (i128::from(self.limit_credits) - i128::from(self.used)).max(0);
            i64::try_from((i128::from(self.reset_limit) - remaining).max(0))
                .map_err(|_| Error::Invalid)
        } else {
            Ok(self.used)
        }
    }
}
fn checked_sum(mut values: impl Iterator<Item = i64>) -> Result<i64> {
    values.try_fold(0_i64, |sum, value| {
        sum.checked_add(value).ok_or(Error::Invalid)
    })
}
fn valid_id(id: i64) -> bool {
    (1..=MAX_ID).contains(&id)
}
fn targets(input: Vec<ResetTarget>) -> Result<Vec<ResetTarget>> {
    if input.len() > 5000
        || input
            .iter()
            .any(|t| !valid_id(t.user_id) || !valid_id(t.plan_id))
    {
        return Err(Error::Invalid);
    }
    Ok(input
        .into_iter()
        .collect::<BTreeSet<_>>()
        .into_iter()
        .collect())
}
fn validate_filter(f: &ResetFilter) -> Result<()> {
    if f.query.chars().count() > 200
        || f.query.chars().any(char::is_control)
        || f.plan_id < 0
        || f.plan_id > MAX_ID
        || (f.plan_id > 0 && !f.plan_ids.is_empty())
        || f.plan_ids.len() > 100
        || f.user_ids.len() > 100
        || f.plan_ids
            .iter()
            .chain(f.user_ids.iter())
            .any(|id| !valid_id(*id))
    {
        return Err(Error::Invalid);
    }
    Ok(())
}
fn preview_digest(mode: &str, expiry: i64, grants: &[FrozenGrant]) -> Result<Vec<u8>> {
    Ok(Sha256::digest(
        serde_json::to_vec(&json!({
            "version":1,"mode":mode,"voucher_expires_at":expiry,"grants":grants
        }))
        .map_err(|_| Error::Unavailable)?,
    )
    .to_vec())
}
pub(crate) fn reset_code_digest(code: &str) -> Result<Vec<u8>> {
    if code.is_empty() || code.len() > 256 || code.chars().any(char::is_control) {
        return Err(Error::Invalid);
    }
    let mut h = Sha256::new();
    h.update(b"core.subscription-reset-code.v1\0");
    h.update(code.as_bytes());
    Ok(h.finalize().to_vec())
}
async fn lock_targets(c: &mut PgConnection, targets: &[ResetTarget]) -> Result<()> {
    // The same user advisory lock is taken by wallet/provider subscription
    // creation. Lock users in canonical order before the charging grant rows.
    let users: BTreeSet<_> = targets.iter().map(|t| t.user_id).collect();
    for user in users {
        lock(c, user).await?;
    }
    Ok(())
}
const GRANTS_SQL: &str = r#"
SELECT s.id,p.user_id,p.plan_id,w.start_at,w.end_at,w.limit_credits,w.unlimited,
       u.used_credits::bigint AS used,u.reserved_credits::bigint AS reserved,u.reset_event_id,
       s.reset_basis,s.reset_limit_credits,s.reset_unlimited,to_jsonb(s) AS grant
FROM core_charging.subscriptions s
CROSS JOIN LATERAL core_charging.subscription_plan(s.id) p
CROSS JOIN LATERAL core_charging.subscription_window(s.id,$3) w
CROSS JOIN LATERAL core_charging.subscription_consumption(s.id,w.start_at,NULL) u
WHERE s.active AND (p.user_id,p.plan_id) IN (SELECT * FROM unnest($1::bigint[],$2::bigint[]))
ORDER BY s.id LIMIT 20001
"#;
async fn grants(
    c: &mut PgConnection,
    targets: &[ResetTarget],
    time: i64,
    write: bool,
) -> Result<Vec<FrozenGrant>> {
    let users: Vec<i64> = targets.iter().map(|t| t.user_id).collect();
    let plans: Vec<i64> = targets.iter().map(|t| t.plan_id).collect();
    let suffix = if write { " FOR UPDATE OF s" } else { "" };
    let rows = sqlx::query(sqlx::AssertSqlSafe(format!("{GRANTS_SQL}{suffix}")))
        .bind(users)
        .bind(plans)
        .bind(time)
        .fetch_all(c)
        .await?;
    if rows.len() > 20000 {
        return Err(Error::Invalid);
    }
    rows.into_iter()
        .map(|r| {
            Ok(FrozenGrant {
                id: r.try_get("id")?,
                user_id: r.try_get("user_id")?,
                plan_id: r.try_get("plan_id")?,
                window_start: r.try_get("start_at")?,
                window_end: r.try_get("end_at")?,
                limit_credits: r.try_get("limit_credits")?,
                unlimited: r.try_get("unlimited")?,
                used: r.try_get("used")?,
                reserved: r.try_get("reserved")?,
                previous_reset_id: r.try_get("reset_event_id")?,
                reset_basis: r.try_get("reset_basis")?,
                reset_limit: r.try_get("reset_limit_credits")?,
                reset_unlimited: r.try_get("reset_unlimited")?,
                grant: r.try_get("grant")?,
            })
        })
        .collect()
}
async fn reject_inflight(c: &mut PgConnection, grants: &[FrozenGrant]) -> Result<()> {
    let ids: Vec<i64> = grants.iter().map(|g| g.id).collect();
    let busy:bool=sqlx::query_scalar("SELECT EXISTS(SELECT FROM core_charging.requests WHERE source_subscription_id=ANY($1) AND state IN ('reserved','streaming','reconcile'))")
        .bind(ids).fetch_one(c).await?;
    if busy { Err(Error::Conflict) } else { Ok(()) }
}
async fn append_reset(
    c: &mut PgConnection,
    a: &Principal,
    g: &FrozenGrant,
    operation: &str,
    voucher: Option<i64>,
    time: i64,
) -> Result<()> {
    let (limit, unlimited) = g.after_basis();
    sqlx::query("INSERT INTO core_subscriptions.reset_events(operation_id,mode,actor_user_id,user_id,plan_id,subscription_id,voucher_id,window_start,window_end,previous_reset_id,before_limit,before_unlimited,before_used,after_limit,after_unlimited,restored_quota,created_at) VALUES($1,$2,$3,$4,$5,$6,$7,$8,$9,$10,$11,$12,$13,$14,$15,$16,$17)")
        .bind(operation).bind(if voucher.is_some(){"voucher_redeem"}else{"hard"})
        .bind(a.user_id).bind(g.user_id).bind(g.plan_id).bind(g.id).bind(voucher)
        .bind(g.window_start).bind(g.window_end).bind(g.previous_reset_id)
        .bind(g.limit_credits).bind(g.unlimited).bind(g.used).bind(limit).bind(unlimited)
        .bind(g.restored()?).bind(time).execute(c).await?;
    Ok(())
}
async fn audit(c: &mut PgConnection, a: &Principal, action: &str, result: &Value) -> Result<()> {
    sqlx::query("INSERT INTO core_identity.audit(actor_user_id,credential_id,action,target) VALUES($1,$2,$3,$4)")
        .bind(a.user_id).bind(a.credential_id).bind(action).bind(result).execute(c).await?;
    Ok(())
}
async fn target_summaries(
    c: &mut PgConnection,
    grants: &[FrozenGrant],
    time: i64,
) -> Result<Vec<Value>> {
    let mut groups: BTreeMap<ResetTarget, Vec<&FrozenGrant>> = BTreeMap::new();
    for g in grants {
        groups.entry(g.target()).or_default().push(g);
    }
    let mut result = Vec::new();
    for (target, items) in groups {
        let r=sqlx::query("SELECT coalesce(u.username,'') AS username,coalesce(u.email,'') AS email,r.terms->>'title' AS title,p.archived,(SELECT count(*) FROM core_subscriptions.reset_vouchers v WHERE v.user_id=$1 AND v.plan_id=$2 AND v.expires_at>$3 AND NOT EXISTS(SELECT FROM core_subscriptions.reset_voucher_redemptions d WHERE d.voucher_id=v.id)) AS vouchers FROM core_subscriptions.plans p JOIN core_subscriptions.plan_revisions r ON r.plan_id=p.id AND r.revision=p.revision LEFT JOIN core_identity.user_profiles u ON u.user_id=$1 WHERE p.id=$2")
            .bind(target.user_id).bind(target.plan_id).bind(time).fetch_one(&mut *c).await?;
        result.push(json!({"user_id":target.user_id,"plan_id":target.plan_id,
            "username":r.try_get::<String,_>("username")?,"email":r.try_get::<String,_>("email")?,
            "plan_title":r.try_get::<String,_>("title")?,"plan_archived_at":if r.try_get::<bool,_>("archived")? {1}else{0},
            "active_subscription_count":items.len(),"amount_total":checked_sum(items.iter().map(|g|g.limit_credits))?,
            "amount_used":checked_sum(items.iter().map(|g|g.used))?,
            "next_reset_time":items.iter().map(|g|g.window_end).min().unwrap_or(0),
            "banked_voucher_count":r.try_get::<i64,_>("vouchers")?}));
    }
    Ok(result)
}
async fn filtered_targets(
    c: &mut PgConnection,
    f: &ResetFilter,
    time: i64,
) -> Result<Vec<ResetTarget>> {
    validate_filter(f)?;
    let rows=sqlx::query(r#"SELECT DISTINCT p.user_id,p.plan_id
        FROM core_charging.subscriptions s CROSS JOIN LATERAL core_charging.subscription_plan(s.id) p
        JOIN core_subscriptions.plans plan ON plan.id=p.plan_id
        JOIN core_subscriptions.plan_revisions r ON r.plan_id=plan.id AND r.revision=plan.revision
        LEFT JOIN core_identity.user_profiles profile ON profile.user_id=p.user_id
        WHERE s.active AND s.anchor<=$1 AND s.ends_at>$1
          AND ($2::bigint=0 OR p.plan_id=$2)
          AND (cardinality($3::bigint[])=0 OR p.plan_id=ANY($3))
          AND (cardinality($4::bigint[])=0 OR p.user_id=ANY($4))
          AND ($5::text='' OR strpos(lower(p.user_id::text||' '||coalesce(profile.username,'')||' '||coalesce(profile.email,'')||' '||coalesce(r.terms->>'title','')),lower($5))>0)
        ORDER BY p.user_id,p.plan_id LIMIT 5001"#)
        .bind(time).bind(f.plan_id).bind(&f.plan_ids).bind(&f.user_ids).bind(f.query.trim()).fetch_all(c).await?;
    if rows.len() > 5000 {
        return Err(Error::Invalid);
    }
    rows.into_iter()
        .map(|r| {
            Ok(ResetTarget {
                user_id: r.try_get("user_id")?,
                plan_id: r.try_get("plan_id")?,
            })
        })
        .collect()
}

impl SubscriptionStore {
    pub async fn reset_vouchers(&self, secret: &str) -> Result<Vec<Value>> {
        let mut tx = self.begin().await?;
        let a = actor(&mut tx, secret, false).await?;
        let time = now(&mut tx).await?;
        let rows:Vec<Value>=sqlx::query_scalar(r#"SELECT jsonb_build_object(
            'id',v.id,'user_id',v.user_id,'plan_id',v.plan_id,'operation_id',v.operation_id,
            'status',CASE WHEN d.voucher_id IS NULL THEN 'available' ELSE 'redeemed' END,
            'expires_at',v.expires_at,'redeemed_at',coalesce(d.created_at,0),'created_by',v.created_by,
            'created_at',v.created_at,'updated_at',coalesce(d.created_at,v.created_at),
            'expired',d.voucher_id IS NULL AND v.expires_at<=$2,'plan_title',r.terms->>'title')
            FROM core_subscriptions.reset_vouchers v
            JOIN core_subscriptions.plans p ON p.id=v.plan_id
            JOIN core_subscriptions.plan_revisions r ON r.plan_id=p.id AND r.revision=p.revision
            LEFT JOIN core_subscriptions.reset_voucher_redemptions d ON d.voucher_id=v.id
            WHERE v.user_id=$1 ORDER BY (d.voucher_id IS NULL AND v.expires_at>$2) DESC,v.id DESC LIMIT 100"#)
            .bind(a.user_id).bind(time).fetch_all(&mut *tx).await?;
        tx.commit().await?;
        Ok(rows)
    }
    pub async fn redeem_reset_voucher(&self, secret: &str, id: i64) -> Result<Value> {
        if !valid_id(id) {
            return Err(Error::Invalid);
        }
        let mut tx = self.begin().await?;
        let a = actor(&mut tx, secret, false).await?;
        lock(&mut tx, a.user_id).await?;
        let v=sqlx::query("SELECT plan_id,expires_at FROM core_subscriptions.reset_vouchers WHERE id=$1 AND user_id=$2")
            .bind(id).bind(a.user_id).fetch_optional(&mut *tx).await?.ok_or(Error::NotFound)?;
        if let Some(result) = sqlx::query_scalar::<_, Value>(
            "SELECT result FROM core_subscriptions.reset_voucher_redemptions WHERE voucher_id=$1",
        )
        .bind(id)
        .fetch_optional(&mut *tx)
        .await?
        {
            tx.commit().await?;
            return Ok(result);
        }
        let plan_id: i64 = v.try_get("plan_id")?;
        let selected = vec![ResetTarget {
            user_id: a.user_id,
            plan_id,
        }];
        let initial = now(&mut tx).await?;
        let current = grants(&mut tx, &selected, initial, true).await?;
        let time = now(&mut tx).await?;
        if v.try_get::<i64, _>("expires_at")? <= time {
            return Err(Error::Conflict);
        }
        if current.is_empty() {
            return Err(Error::Conflict);
        }
        // A lock wait may cross the original cycle or grant expiry.
        if grants(&mut tx, &selected, time, false).await? != current {
            return Err(Error::Conflict);
        }
        reject_inflight(&mut tx, &current).await?;
        let restored = checked_sum(
            current
                .iter()
                .map(FrozenGrant::restored)
                .collect::<Result<Vec<_>>>()?
                .into_iter(),
        )?;
        for g in &current {
            append_reset(&mut tx, &a, g, &format!("voucher:{id}"), Some(id), time).await?;
        }
        let title = plan(&mut tx, plan_id, false).await?.terms.title;
        let result = json!({"plan_id":plan_id,"plan_title":title,"matched_count":current.len(),
            "reset_count":current.len(),"user_count":1,"advance_reset_time":false,
            "restored_quota":restored,"affected_user_ids":[a.user_id]});
        sqlx::query("INSERT INTO core_subscriptions.reset_voucher_redemptions(voucher_id,result,created_at) VALUES($1,$2,$3)")
            .bind(id).bind(&result).bind(time).execute(&mut *tx).await?;
        audit(&mut tx, &a, "subscription.reset.voucher_redeem", &result).await?;
        tx.commit().await?;
        Ok(result)
    }
    /// Return None only when this code is absent from reset authority. A known
    /// expired/claimed/disabled reset code must never fall through to money.
    pub async fn try_redeem_reset_code(
        &self,
        secret: &str,
        input: ResetCodeRequest,
    ) -> Result<Option<Value>> {
        operation_key(&input.operation_key)?;
        let digest = reset_code_digest(&input.code)?;
        let mut tx = self.begin().await?;
        let a = actor(&mut tx, secret, false).await?;
        lock(&mut tx, a.user_id).await?;
        sqlx::query("SELECT pg_advisory_xact_lock(hashtextextended('core-reset-code:'||encode($1,'hex'),0))")
            .bind(&digest).execute(&mut *tx).await?;
        let Some(c)=sqlx::query("SELECT plan_id,issuer_user_id,expires_at,voucher_expires_at,enabled FROM core_subscriptions.reset_codes WHERE digest=$1")
            .bind(&digest).fetch_optional(&mut *tx).await? else {tx.rollback().await?;return Ok(None);};
        let request = json!(["subscription_reset_code", crate::hex_codec::encode(&digest)]);
        if let Some(result) = prior(&mut tx, a.user_id, &input.operation_key, &request).await? {
            tx.commit().await?;
            return Ok(Some(result));
        }
        let claimed: bool = sqlx::query_scalar(
            "SELECT EXISTS(SELECT FROM core_subscriptions.reset_vouchers WHERE code_digest=$1)",
        )
        .bind(&digest)
        .fetch_one(&mut *tx)
        .await?;
        let time = now(&mut tx).await?;
        let expiry: i64 = c.try_get("expires_at")?;
        let voucher_expiry: i64 = c.try_get("voucher_expires_at")?;
        if claimed
            || !c.try_get::<bool, _>("enabled")?
            || (expiry != 0 && expiry < time)
            || voucher_expiry <= time
        {
            return Err(Error::Conflict);
        }
        let plan_id: i64 = c.try_get("plan_id")?;
        let id:i64=sqlx::query_scalar("INSERT INTO core_subscriptions.reset_vouchers(user_id,plan_id,operation_id,code_digest,expires_at,created_by,created_at) VALUES($1,$2,$3,$4,$5,$6,$7) RETURNING id")
            .bind(a.user_id).bind(plan_id).bind(format!("code:{}",crate::hex_codec::encode(&digest)))
            .bind(digest).bind(voucher_expiry).bind(c.try_get::<i64,_>("issuer_user_id")?).bind(time)
            .fetch_one(&mut *tx).await?;
        let result = json!({"reward_type":"reset_voucher","reset_voucher_id":id,
            "reset_plan_id":plan_id,"reset_voucher_expires_at":voucher_expiry});
        remember(&mut tx, a.user_id, &input.operation_key, &request, &result).await?;
        audit(&mut tx, &a, "subscription.reset.code_claim", &result).await?;
        tx.commit().await?;
        Ok(Some(result))
    }
    pub async fn reset_targets(
        &self,
        secret: &str,
        filter: ResetFilter,
        page: u32,
        page_size: u32,
    ) -> Result<Value> {
        if !(1..=1_000_000).contains(&page) || !(1..=100).contains(&page_size) {
            return Err(Error::Invalid);
        }
        let mut tx = self.begin().await?;
        actor(&mut tx, secret, true).await?;
        let time = now(&mut tx).await?;
        let all = filtered_targets(&mut tx, &filter, time).await?;
        let total = all.len();
        let subset = all
            .into_iter()
            .skip((page - 1) as usize * page_size as usize)
            .take(page_size as usize)
            .collect::<Vec<_>>();
        let current = grants(&mut tx, &subset, time, false).await?;
        let items = target_summaries(&mut tx, &current, time).await?;
        tx.commit().await?;
        Ok(json!({"items":items,"total":total,"page":page,"page_size":page_size}))
    }
    pub async fn preview_reset(&self, secret: &str, input: ResetPreviewRequest) -> Result<Value> {
        let mut tx = self.begin().await?;
        let a = actor(&mut tx, secret, true).await?;
        let time = now(&mut tx).await?;
        let mode = input.mode.trim().to_ascii_lowercase();
        let expiry = match mode.as_str() {
            "hard" if input.voucher_expires_at.is_none() => 0,
            "soft" => input
                .voucher_expires_at
                .filter(|t| *t > time && *t <= MAX_EXPIRY)
                .ok_or(Error::Invalid)?,
            _ => return Err(Error::Invalid),
        };
        let selected = if input.all_matching {
            if !input.targets.is_empty() {
                return Err(Error::Invalid);
            }
            filtered_targets(&mut tx, &input.filter.unwrap_or_default(), time).await?
        } else {
            targets(input.targets)?
        };
        let current = grants(&mut tx, &selected, time, false).await?;
        if current.is_empty() {
            return Err(Error::Conflict);
        }
        let targets: BTreeSet<_> = current.iter().map(FrozenGrant::target).collect();
        let users: BTreeSet<_> = targets.iter().map(|t| t.user_id).collect();
        let plans: BTreeSet<_> = targets.iter().map(|t| t.plan_id).collect();
        let restore = checked_sum(
            current
                .iter()
                .map(FrozenGrant::restored)
                .collect::<Result<Vec<_>>>()?
                .into_iter(),
        )?;
        let token = random_id();
        let digest = preview_digest(&mode, expiry, &current)?;
        sqlx::query("INSERT INTO core_subscriptions.reset_previews(token,actor_user_id,mode,targets,payload_sha256,voucher_expires_at,created_at,expires_at) VALUES($1,$2,$3,$4,$5,$6,$7,$8)")
            .bind(&token).bind(a.user_id).bind(&mode).bind(Json(&current)).bind(digest).bind(expiry).bind(time).bind(time+600)
            .execute(&mut *tx).await?;
        let summaries = target_summaries(&mut tx, &current, time).await?;
        let result = json!({"token":token,"mode":mode,"target_count":targets.len(),"user_count":users.len(),
            "plan_count":plans.len(),"active_subscriptions":current.len(),"quota_to_restore":restore,
            "voucher_expires_at":expiry,"expires_at":time+600,"targets":summaries});
        tx.commit().await?;
        Ok(result)
    }
    pub async fn execute_reset(&self, secret: &str, input: ResetExecuteRequest) -> Result<Value> {
        let operation = input.operation_id.trim();
        if operation.is_empty() || operation.len() > 64 || operation.chars().any(char::is_control) {
            return Err(Error::Invalid);
        }
        token(&input.preview_token)?;
        let mut tx = self.begin().await?;
        let a = actor(&mut tx, secret, true).await?;
        sqlx::query(
            "SELECT pg_advisory_xact_lock(hashtextextended('core-reset-operation:'||$1,0))",
        )
        .bind(operation)
        .execute(&mut *tx)
        .await?;
        let existing=sqlx::query("SELECT actor_user_id,preview_token,result FROM core_subscriptions.reset_operations WHERE operation_id=$1")
            .bind(operation).fetch_optional(&mut *tx).await?;
        if let Some(r) = existing {
            if r.try_get::<i64, _>("actor_user_id")? != a.user_id
                || r.try_get::<String, _>("preview_token")? != input.preview_token
            {
                return Err(Error::Conflict);
            }
            let result = r.try_get("result")?;
            tx.commit().await?;
            return Ok(result);
        }
        let p=sqlx::query("SELECT mode,targets,payload_sha256,voucher_expires_at,expires_at FROM core_subscriptions.reset_previews WHERE token=$1 AND actor_user_id=$2")
            .bind(&input.preview_token).bind(a.user_id).fetch_optional(&mut *tx).await?.ok_or(Error::NotFound)?;
        let consumed: bool = sqlx::query_scalar(
            "SELECT EXISTS(SELECT FROM core_subscriptions.reset_operations WHERE preview_token=$1)",
        )
        .bind(&input.preview_token)
        .fetch_one(&mut *tx)
        .await?;
        if consumed {
            return Err(Error::Conflict);
        }
        let mode: String = p.try_get("mode")?;
        let expiry: i64 = p.try_get("voucher_expires_at")?;
        let frozen: Vec<FrozenGrant> = decode(p.try_get("targets")?)?;
        if preview_digest(&mode, expiry, &frozen)? != p.try_get::<Vec<u8>, _>("payload_sha256")? {
            return Err(Error::Conflict);
        }
        let selected = targets(
            frozen
                .iter()
                .map(FrozenGrant::target)
                .collect::<BTreeSet<_>>()
                .into_iter()
                .collect(),
        )?;
        lock_targets(&mut tx, &selected).await?;
        let initial = now(&mut tx).await?;
        let locked = grants(&mut tx, &selected, initial, true).await?;
        let time = now(&mut tx).await?;
        if p.try_get::<i64, _>("expires_at")? <= time
            || (mode == "soft" && expiry <= time)
            || locked != frozen
            || grants(&mut tx, &selected, time, false).await? != frozen
        {
            return Err(Error::Conflict);
        }
        let mut restored = 0_i64;
        if mode == "hard" {
            reject_inflight(&mut tx, &locked).await?;
            for g in &locked {
                append_reset(
                    &mut tx,
                    &a,
                    g,
                    &format!("root:{}:{operation}", a.user_id),
                    None,
                    time,
                )
                .await?;
                restored = restored.checked_add(g.restored()?).ok_or(Error::Invalid)?;
            }
        } else {
            for t in &selected {
                sqlx::query("INSERT INTO core_subscriptions.reset_vouchers(user_id,plan_id,operation_id,expires_at,created_by,created_at) VALUES($1,$2,$3,$4,$5,$6)")
                    .bind(t.user_id).bind(t.plan_id).bind(format!("root:{}:{operation}",a.user_id)).bind(expiry)
                    .bind(a.user_id).bind(time).execute(&mut *tx).await?;
            }
        }
        let result = json!({"operation_id":operation,"mode":mode,"requested_targets":selected.len(),
            "processed_targets":selected.len(),"skipped_targets":0,
            "reset_subscriptions":if mode=="hard" {locked.len()} else {0},
            "restored_quota":restored,"vouchers_issued":if mode=="soft" {selected.len()} else {0},
            "voucher_expires_at":expiry});
        sqlx::query("INSERT INTO core_subscriptions.reset_operations(operation_id,preview_token,actor_user_id,result,created_at) VALUES($1,$2,$3,$4,$5)")
            .bind(operation).bind(&input.preview_token).bind(a.user_id).bind(&result).bind(time).execute(&mut *tx).await?;
        audit(&mut tx, &a, "subscription.reset.batch", &result).await?;
        tx.commit().await?;
        Ok(result)
    }
}

#[cfg(test)]
#[path = "reset_tests.rs"]
mod tests;

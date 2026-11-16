// Copyright (C) 2026 LIghtJUNction. AGPL-3.0-or-later.
//! Single-user profile reads. Site-wide Window validation stays unchanged.
use super::*;

impl UserUsageRequest {
    fn validate(&self) -> Result<()> {
        if self.user_id <= 0 || !(0..=12).contains(&self.model_limit) {
            return Err(Error::Invalid);
        }
        if self.start == 0 && self.bucket_seconds == 0 {
            if !(0..=253_402_300_799).contains(&self.end) {
                return Err(Error::Invalid);
            }
            return Ok(());
        }
        Window {
            start: self.start,
            end: self.end,
            bucket_seconds: self.bucket_seconds,
        }
        .validate()
    }
}

// Unlike the site-wide token projection, a profile's money must preserve NULL
// when the source has no quota or no frozen conversion. Do not price old rows
// with today's directory, ledger unit assumption or display exchange rate.
const USER_HISTORY_LOGS: &str = r#"SELECT l.user_id, l.created_at AS at,
 btrim(coalesce(l.model_name,'')) AS model,
 greatest(coalesce(l.prompt_tokens,0),0)+greatest(coalesce(l.completion_tokens,0),0) AS tokens,
 1::bigint AS requests,
 CASE WHEN l.quota IS NOT NULL AND l.pricing_credits_per_usd>0
 THEN (greatest(l.quota,0)::numeric*500000/l.pricing_credits_per_usd)::bigint
 ELSE NULL END AS credits
 FROM core_history.log_records l JOIN core_history.sources s ON s.source_id=l.source_id
 WHERE s.complete AND s.contract_version='core.history-source.v1' AND l.log_type=2
 AND l.user_id=$3 AND l.created_at BETWEEN $1 AND $2"#;

fn scoped_usage(history_source: &str) -> String {
    // Each UNION branch has its own user predicate. Existing indexes begin with
    // actor_user_id or user_id (history indexes then include owner_account_id).
    // No all-user scan/window is introduced for a lifetime profile request.
    let history = if history_source == "quota_aggregates" {
        format!("{HISTORY_AGGREGATES} AND q.user_id=$3")
    } else {
        USER_HISTORY_LOGS.to_owned()
    };
    format!("({NATIVE} AND r.actor_user_id=$3) UNION ALL ({history})")
}

pub(crate) struct UserUsageTotals {
    pub tokens: u128,
    pub requests: u128,
    pub credits: Option<u128>,
}

pub(crate) struct UserUsageSummary {
    pub totals: UserUsageTotals,
    pub model_count: i64,
}

fn exact_total(value: String) -> Result<u128> {
    value.parse().map_err(|_| Error::Unavailable)
}

fn community_number(value: u128) -> Result<i64> {
    safe(i64::try_from(value).map_err(|_| Error::Unavailable)?)
}

/// Caller owns authorization and a read-only snapshot. This is the same
/// actor-scoped, refund-aware projection used by the community profile. Numeric
/// sums are decoded from decimal text so valid BIGINT rows can total above i64.
/// Callers retain their separate JSON number/string bounds.
pub(crate) async fn read_user_usage_summary(
    connection: &mut sqlx::PgConnection,
    request: &UserUsageRequest,
    history_source: &str,
) -> Result<UserUsageSummary> {
    request.validate()?;
    let scoped = scoped_usage(history_source);
    let row = sqlx::query(sqlx::AssertSqlSafe(format!(
        "SELECT coalesce(sum(tokens),0)::text AS tokens,
         coalesce(sum(requests),0)::text AS requests,
         CASE WHEN coalesce(bool_and(credits IS NOT NULL),true)
          THEN coalesce(sum(credits),0)::text ELSE NULL END AS credits,
         count(DISTINCT coalesce(nullif(model,''),'unknown'))::bigint AS models
         FROM ({scoped}) s"
    )))
    .bind(request.start)
    .bind(request.end)
    .bind(request.user_id)
    .fetch_one(connection)
    .await?;
    Ok(UserUsageSummary {
        totals: UserUsageTotals {
            tokens: exact_total(row.try_get("tokens")?)?,
            requests: exact_total(row.try_get("requests")?)?,
            credits: row
                .try_get::<Option<String>, _>("credits")?
                .map(exact_total)
                .transpose()?,
        },
        model_count: row.try_get("models")?,
    })
}

impl CommunityRead {
    /// Actor-scoped native usage plus the selected complete imported source.
    /// Complete imported hourly aggregates supersede logs, never add to them.
    pub async fn user_usage(&self, r: &UserUsageRequest) -> Result<UserUsage> {
        r.validate()?;
        let mut tx = self.snapshot().await?;
        let user = sqlx::query("SELECT u.active,u.platform_role,p.username,coalesce(nullif(btrim(p.display_name),''),btrim(p.username),'') AS name FROM core_identity.users u JOIN core_identity.user_profiles p ON p.user_id=u.id WHERE u.id=$1")
            .bind(r.user_id)
            .fetch_optional(&mut *tx)
            .await?;
        let (active, name, username, platform_role) = match user {
            Some(row) => (
                row.try_get("active")?,
                row.try_get("name")?,
                row.try_get("username")?,
                row.try_get("platform_role")?,
            ),
            None => (false, String::new(), String::new(), String::new()),
        };
        let scoped = scoped_usage(self.history_source);
        let summary = read_user_usage_summary(&mut tx, r, self.history_source).await?;
        // Preserve the existing numeric community contract's JS-safe bounds.
        // The administrator detail has a separate exact-string contract.
        let totals = UsageTotals {
            tokens: community_number(summary.totals.tokens)?,
            requests: community_number(summary.totals.requests)?,
            credits: summary.totals.credits.map(community_number).transpose()?,
        };
        let pricing_available = totals.credits.is_some();
        let mut buckets = Vec::new();
        if r.bucket_seconds > 0 {
            for row in sqlx::query(sqlx::AssertSqlSafe(format!(
                "SELECT (at/$4)*$4 AS bucket,sum(tokens)::bigint AS tokens,sum(requests)::bigint AS requests
                 FROM ({scoped}) s GROUP BY 1 ORDER BY 1 LIMIT 1000"
            )))
            .bind(r.start)
            .bind(r.end)
            .bind(r.user_id)
            .bind(r.bucket_seconds)
            .fetch_all(&mut *tx)
            .await?
            {
                buckets.push(UsageBucket {
                    bucket: row.try_get("bucket")?,
                    tokens: safe(row.try_get("tokens")?)?,
                    requests: safe(row.try_get("requests")?)?,
                });
            }
        }
        let mut models = Vec::new();
        if r.model_limit > 0 {
            for row in sqlx::query(sqlx::AssertSqlSafe(format!(
                "SELECT * FROM (
                  SELECT coalesce(nullif(model,''),'unknown') AS model,sum(tokens)::bigint AS tokens,
                   sum(requests)::bigint AS requests,
                   CASE WHEN bool_and(credits IS NOT NULL) THEN sum(credits)::bigint ELSE NULL END AS credits
                  FROM ({scoped}) s GROUP BY 1
                 ) m ORDER BY CASE WHEN $5 THEN m.credits END DESC NULLS LAST,
                 tokens DESC,requests DESC,model LIMIT $4"
            )))
            .bind(r.start)
            .bind(r.end)
            .bind(r.user_id)
            .bind(r.model_limit)
            .bind(pricing_available)
            .fetch_all(&mut *tx)
            .await?
            {
                models.push(ModelUsage {
                    model_name: row.try_get("model")?,
                    tokens: safe(row.try_get("tokens")?)?,
                    requests: safe(row.try_get("requests")?)?,
                    credits: row.try_get::<Option<i64>, _>("credits")?.map(safe).transpose()?,
                });
            }
        }
        let rate: Option<String> = sqlx::query_scalar(
            "SELECT value FROM core_console.options WHERE key='USDExchangeRate'",
        )
        .fetch_optional(&mut *tx)
        .await?;
        let cny_per_usd =
            rate.filter(|raw| crate::options_http::display_exchange_rate(raw).is_some());
        tx.commit().await?;
        Ok(UserUsage {
            user_id: r.user_id,
            active,
            name,
            username,
            platform_role,
            totals,
            model_count: summary.model_count,
            buckets,
            models,
            credits_per_usd: crate::billing::CREDITS_PER_USD,
            pricing_available,
            cny_per_usd,
            history_source: self.history_source.into(),
        })
    }
}

#[cfg(test)]
mod tests;

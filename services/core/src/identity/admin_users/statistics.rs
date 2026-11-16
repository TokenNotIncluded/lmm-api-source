//! Detail-only statistics. Authorization and the read-only MVCC transaction
//! belong to AdminUsersStore; no list query or financial write uses this reader.
use super::*;
use crate::community_read::{
    UserUsageRequest, UserUsageSummary, read_user_usage_summary, selected_history_source,
};

#[derive(Serialize)]
pub struct UserStatistics {
    user_id: String,
    personal_account_id: String,
    usage: UsageProjection,
    historical_counters: HistoricalCounters,
    referrals: ReferralStatistics,
}

#[derive(Serialize)]
#[serde(tag = "status", rename_all = "snake_case")]
enum UsageProjection {
    Available {
        scope: &'static str,
        requests: String,
        tokens: String,
        credits: Option<String>,
        model_count: String,
        unit: &'static str,
        credits_per_usd: i64,
        pricing_available: bool,
        history_source: &'static str,
        observed_at_unix_ms: i64,
    },
    Unavailable,
}

#[derive(Serialize)]
#[serde(tag = "status", rename_all = "snake_case")]
enum HistoricalCounters {
    Available {
        unit: &'static str,
        used_quota: String,
        request_count: String,
        affiliate_rewards: String,
        observed_at_unix_ms: i64,
    },
    NotImported,
    Unavailable,
}

#[derive(Serialize)]
struct ReferralStatistics {
    /// Distinct recorded invitees, not an inferred full-history counter.
    invited_users: Option<String>,
    invitation_scope: &'static str,
    /// Signed committed affiliate position. This does not promise claimability.
    affiliate_balance_units: Option<String>,
    unit: &'static str,
    money_basis: &'static str,
    reviewed_rewards: Option<ReviewedReferralRewards>,
}

#[derive(Serialize)]
struct ReviewedReferralRewards {
    scope: &'static str,
    granted_credits: String,
    unit: &'static str,
    credits_per_usd: i64,
}

fn count(value: i64) -> ReadResult<String> {
    if value < 0 {
        Err(AdminUsersError::Unavailable)
    } else {
        Ok(value.to_string())
    }
}

// Resolve exact catalog OIDs before checking privileges. An optional schema
// without USAGE must return unavailable, not abort the caller's transaction.
async fn readable_tables(c: &mut PgConnection, names: &[&str]) -> ReadResult<bool> {
    Ok(sqlx::query_scalar(
        "SELECT coalesce(bool_and(t.oid IS NOT NULL
          AND has_schema_privilege(n.oid,'USAGE')
          AND has_table_privilege(t.oid,'SELECT')),false)
         FROM unnest($1::text[]) wanted(name)
         LEFT JOIN pg_namespace n ON n.nspname=split_part(wanted.name,'.',1)
         LEFT JOIN pg_class t ON t.relnamespace=n.oid
          AND t.relname=split_part(wanted.name,'.',2) AND t.relkind IN ('r','p')",
    )
    .bind(names)
    .fetch_one(c)
    .await?)
}

async fn readable_columns(
    c: &mut PgConnection,
    schema: &str,
    table: &str,
    columns: &[&str],
) -> ReadResult<bool> {
    Ok(sqlx::query_scalar(
        "SELECT coalesce((SELECT has_schema_privilege(n.oid,'USAGE')
          AND (SELECT bool_and(has_column_privilege(t.oid,wanted.column_name,'SELECT'))
               FROM unnest($3::text[]) wanted(column_name))
         FROM pg_namespace n JOIN pg_class t ON t.relnamespace=n.oid
         WHERE n.nspname=$1 AND t.relname=$2 AND t.relkind IN ('r','p')),false)",
    )
    .bind(schema)
    .bind(table)
    .bind(columns)
    .fetch_one(c)
    .await?)
}

async fn usage(c: &mut PgConnection, user_id: i64) -> ReadResult<UsageProjection> {
    if !readable_tables(
        c,
        &[
            "core_charging.requests",
            "core_history.sources",
            "core_history.log_records",
            "core_history.quota_aggregates",
        ],
    )
    .await?
    {
        return Ok(UsageProjection::Unavailable);
    }
    let end: i64 =
        sqlx::query_scalar("SELECT floor(extract(epoch FROM transaction_timestamp()))::bigint")
            .fetch_one(&mut *c)
            .await?;
    let history_source = selected_history_source(c)
        .await
        .map_err(|_| AdminUsersError::Unavailable)?;
    let summary: UserUsageSummary = read_user_usage_summary(
        c,
        &UserUsageRequest {
            user_id,
            start: 0,
            end,
            bucket_seconds: 0,
            model_limit: 0,
        },
        history_source,
    )
    .await
    .map_err(|_| AdminUsersError::Unavailable)?;
    Ok(UsageProjection::Available {
        scope: "actor_settled_and_complete_history",
        requests: summary.totals.requests.to_string(),
        tokens: summary.totals.tokens.to_string(),
        credits: summary.totals.credits.map(|value| value.to_string()),
        model_count: count(summary.model_count)?,
        unit: "credit_500k_usd",
        credits_per_usd: crate::billing::CREDITS_PER_USD,
        pricing_available: summary.totals.credits.is_some(),
        history_source,
        observed_at_unix_ms: safe_time(end.checked_mul(1000).ok_or(AdminUsersError::Unavailable)?)?,
    })
}

async fn historical(
    c: &mut PgConnection,
    user: i64,
    account: i64,
) -> ReadResult<HistoricalCounters> {
    if !readable_tables(
        c,
        &[
            "core_billing.account_opening_counters",
            "core_billing.opening_imports",
            "core_billing.opening_import_runs",
        ],
    )
    .await?
    {
        return Ok(HistoricalCounters::Unavailable);
    }
    // Keep the exact opening/source binding used by financial_profile::read.
    // Raw counters are never combined with the separate normalized usage read.
    let row = sqlx::query(
        "SELECT c.used_quota,c.request_count,c.aff_history,c.observed_at,
          coalesce(i.account_id=c.account_id AND i.source_row_sha256=c.source_row_sha256
           AND r.source_archive_sha256=c.source_archive_sha256
           AND r.mapping_sha256=c.mapping_sha256 AND r.observed_at=c.observed_at,false) AS valid
         FROM core_billing.account_opening_counters c
         LEFT JOIN core_billing.opening_imports i ON i.migration_id=c.migration_id
          AND i.source_entity_key=c.source_entity_key
         LEFT JOIN core_billing.opening_import_runs r ON r.migration_id=c.migration_id
         WHERE c.user_id=$1 AND c.account_id=$2",
    )
    .bind(user)
    .bind(account)
    .fetch_optional(&mut *c)
    .await?;
    let Some(row) = row else {
        return Ok(HistoricalCounters::NotImported);
    };
    if !row.try_get::<bool, _>("valid")? {
        return Err(AdminUsersError::Unavailable);
    }
    let observed: i64 = row.try_get("observed_at")?;
    Ok(HistoricalCounters::Available {
        unit: "legacy_raw_units",
        used_quota: row.try_get::<i64, _>("used_quota")?.to_string(),
        request_count: row.try_get::<i64, _>("request_count")?.to_string(),
        affiliate_rewards: row.try_get::<i64, _>("aff_history")?.to_string(),
        observed_at_unix_ms: safe_time(
            observed
                .checked_mul(1000)
                .ok_or(AdminUsersError::Unavailable)?,
        )?,
    })
}

pub(super) async fn read(
    c: &mut PgConnection,
    user_id: i64,
    account: i64,
    wallet_available: bool,
) -> ReadResult<UserStatistics> {
    let usage = usage(c, user_id).await?;
    let historical_counters = historical(c, user_id, account).await?;
    let invited_users = if readable_columns(
        c,
        "core_identity",
        "affiliate_registrations",
        &["user_id", "inviter_id"],
    )
    .await?
        && readable_columns(c, "core_referrals", "bindings", &["user_id", "inviter_id"]).await?
    {
        let invited: i64 = sqlx::query_scalar(
            "SELECT count(*) FROM (
              SELECT user_id FROM core_identity.affiliate_registrations WHERE inviter_id=$1
              UNION SELECT user_id FROM core_referrals.bindings WHERE inviter_id=$1
             ) invited",
        )
        .bind(user_id)
        .fetch_one(&mut *c)
        .await?;
        Some(count(invited)?)
    } else {
        None
    };
    // Reviewed legacy history already seeds this normalized counter. Settlement
    // increases it, while revocation/restoration does not; never add raw opening
    // counters, balance positions or all positive referral events to it.
    let reviewed_rewards =
        if readable_columns(c, "core_referrals", "history", &["inviter", "granted"]).await? {
            let granted: Option<i64> =
                sqlx::query_scalar("SELECT granted FROM core_referrals.history WHERE inviter=$1")
                    .bind(user_id)
                    .fetch_optional(&mut *c)
                    .await?;
            granted
                .map(|value| -> ReadResult<_> {
                    Ok(ReviewedReferralRewards {
                        scope: "reviewed_history_and_later_settlements",
                        granted_credits: count(value)?,
                        unit: "credit_500k_usd",
                        credits_per_usd: crate::billing::CREDITS_PER_USD,
                    })
                })
                .transpose()?
        } else {
            None
        };
    // The preceding wallet adapter has already established the personal account,
    // all committed buckets, or the audited native zero basis in this snapshot.
    let affiliate_balance_units = if wallet_available {
        let value: Option<i64> = sqlx::query_scalar(
            "SELECT b.balance_units FROM core_billing.ledger_accounts a
             JOIN core_billing.balance_state b ON b.ledger_account_id=a.id
             WHERE a.owner_account_id=$1 AND a.bucket='affiliate'",
        )
        .bind(account)
        .fetch_optional(&mut *c)
        .await?;
        Some(value.unwrap_or(0).to_string())
    } else {
        None
    };
    Ok(UserStatistics {
        user_id: user_id.to_string(),
        personal_account_id: account.to_string(),
        usage,
        historical_counters,
        referrals: ReferralStatistics {
            invited_users,
            invitation_scope: "recorded_native_and_imported_bindings",
            affiliate_balance_units,
            unit: "core_credit",
            money_basis: "unknown",
            reviewed_rewards,
        },
    })
}

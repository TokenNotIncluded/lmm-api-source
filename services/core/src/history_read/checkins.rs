// Copyright (C) 2026 LIghtJUNction. AGPL-3.0-or-later.
//! Exact imported check-in facts. Reading never grants another reward.
use super::{HistoryError, HistoryStore, Result, personal_session};
use serde::Serialize;
use serde_json::Value;
use sqlx::Row;

const SCOPE: &str = "checkin-history.v1";

#[derive(Serialize)]
pub struct CheckinRecord {
    pub checkin_date: String,
    pub raw_quota_awarded: i64,
    pub source_created_at: i64,
}

#[derive(Serialize)]
pub struct CheckinHistory {
    pub contract: &'static str,
    pub month: String,
    pub observed_at: i64,
    pub total_checkins: i64,
    pub total_raw_quota_awarded: String,
    pub records: Vec<CheckinRecord>,
    pub award_unit: &'static str,
    pub native_claim_available: bool,
    pub calendar_timezone: Option<String>,
}

// Both the original user's membership and the row's mapping flag are needed:
// an orphan source ID must not become visible to a newly created target user.
const OWN_ROWS: &str = "WITH own AS (
 SELECT x->'record' AS data FROM core_migration.import_runs i
 CROSS JOIN LATERAL jsonb_array_elements(i.manifest->'rows') x
 WHERE i.migration_id=$1 AND i.manifest->'source_user_ids' @> jsonb_build_array($2::bigint)
 AND x->>'source_user_present'='true' AND (x->'record'->>'user_id')::bigint=$2
) ";

impl HistoryStore {
    pub async fn checkin_history(&self, secret: &str, month: &str) -> Result<CheckinHistory> {
        let mut tx = self.read_snapshot().await?;
        let user = personal_session(&mut tx, secret).await?;
        if month.len() != 7
            || month.as_bytes()[4] != b'-'
            || !month
                .bytes()
                .enumerate()
                .all(|(i, b)| i == 4 || b.is_ascii_digit())
            || chrono::NaiveDate::parse_from_str(&format!("{month}-01"), "%Y-%m-%d").is_err()
            || month.starts_with("0000")
        {
            return Err(HistoryError::Invalid);
        }
        let receipts = sqlx::query("SELECT migration_id,manifest-'rows' AS metadata,jsonb_array_length(manifest->'rows')::bigint AS actual_rows,EXISTS(SELECT FROM core_billing.opening_import_runs o WHERE o.source_archive_sha256=i.manifest->>'source_archive_sha256' AND o.observed_at=(i.manifest->>'observed_at')::bigint) AS source_bound FROM core_migration.import_runs i WHERE manifest->>'scope'=$1 LIMIT 2")
            .bind(SCOPE).fetch_all(&mut *tx).await?;
        let [receipt] = receipts.as_slice() else {
            return Err(HistoryError::Unavailable);
        };
        let metadata: Value = receipt.try_get("metadata")?;
        let rows: i64 = receipt.try_get("actual_rows")?;
        let bound: bool = receipt.try_get("source_bound")?;
        if !bound
            || metadata["history_complete"] != true
            || metadata["source_namespace"] != "lmm_api.lmm_prod_20260802.checkins"
            || metadata["source_row_count"].as_i64() != Some(rows)
            || metadata["balance_effect"] != "none"
            || metadata["award_unit"] != "legacy_raw_no_conversion"
        {
            return Err(HistoryError::Unavailable);
        }
        let migration: String = receipt.try_get("migration_id")?;
        let mapped: bool = sqlx::query_scalar("SELECT manifest->'source_user_ids' @> jsonb_build_array($2::bigint) FROM core_migration.import_runs WHERE migration_id=$1")
            .bind(&migration).bind(user).fetch_one(&mut *tx).await?;
        if !mapped {
            return Err(HistoryError::Unavailable);
        }
        let totals = sqlx::query(sqlx::AssertSqlSafe(OWN_ROWS.to_owned()+"SELECT count(*)::bigint AS count,coalesce(sum((data->>'quota_awarded')::numeric),0)::text AS total FROM own"))
            .bind(&migration).bind(user).fetch_one(&mut *tx).await?;
        let records = sqlx::query(sqlx::AssertSqlSafe(OWN_ROWS.to_owned()+"SELECT data->>'checkin_date' AS day,(data->>'quota_awarded')::bigint AS award,(data->>'created_at')::bigint AS created FROM own WHERE left(data->>'checkin_date',7)=$3 ORDER BY day DESC"))
            .bind(&migration).bind(user).bind(month).fetch_all(&mut *tx).await?
            .into_iter().map(|r| Ok(CheckinRecord {checkin_date:r.try_get("day")?,raw_quota_awarded:r.try_get("award")?,source_created_at:r.try_get("created")?}))
            .collect::<std::result::Result<Vec<_>, sqlx::Error>>()?;
        let result = CheckinHistory {
            contract: "core.checkin-history.v1",
            month: month.to_owned(),
            observed_at: metadata["observed_at"]
                .as_i64()
                .ok_or(HistoryError::Unavailable)?,
            total_checkins: totals.try_get("count")?,
            total_raw_quota_awarded: totals.try_get("total")?,
            records,
            award_unit: "legacy_raw_no_conversion",
            native_claim_available: false,
            calendar_timezone: None,
        };
        tx.commit().await?;
        Ok(result)
    }
}

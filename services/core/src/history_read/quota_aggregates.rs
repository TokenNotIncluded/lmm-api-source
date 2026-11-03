//! Imported legacy hourly usage aggregates (`quota_data`) for dashboard reads.
//! Values are already converted offline to current credits; nothing is priced,
//! settled or written here, and native receipts remain the only live source.
use serde_json::{Value, json};
use sqlx::{PgConnection, Row};

/// Upper bound on returned hourly model rows; denser windows are refused.
pub const MAX_ROWS: usize = 100_000;

pub struct Selection<'a> {
    pub start: i64,
    pub end: i64,
    /// `None` reads every user (administrator projection only).
    pub user: Option<i64>,
    pub owner_account: Option<i64>,
    pub model_name: &'a str,
    pub group: &'a str,
    /// Usage scope label used by the matching native projection.
    pub usage_scope: &'static str,
}

/// Hourly per-user, per-model buckets from complete imported sources only.
/// Returns `None` when the window exceeds [`MAX_ROWS`].
pub async fn buckets(
    c: &mut PgConnection,
    selection: &Selection<'_>,
) -> Result<Option<Vec<Value>>, sqlx::Error> {
    let rows = sqlx::query(
        r#"SELECT q.user_id,max(q.username) AS username,q.model_name,q.created_at,
             sum(q.request_count)::bigint AS count,sum(q.token_used)::bigint AS token_used,
             sum(q.quota_credits)::bigint AS quota
           FROM core_history.quota_aggregates q
           JOIN core_history.sources s ON s.source_id=q.source_id
           WHERE s.complete AND s.contract_version='core.history-source.v1'
             AND q.created_at BETWEEN $1 AND $2
             AND ($3::bigint IS NULL OR q.user_id=$3)
             AND ($4::bigint IS NULL OR q.owner_account_id=$4)
             AND ($5='' OR q.model_name=$5) AND ($6='' OR q.group_name=$6)
           GROUP BY q.user_id,q.model_name,q.created_at
           ORDER BY q.created_at,q.user_id,q.model_name LIMIT $7"#,
    )
    .bind(selection.start)
    .bind(selection.end)
    .bind(selection.user)
    .bind(selection.owner_account)
    .bind(selection.model_name)
    .bind(selection.group)
    .bind(MAX_ROWS as i64 + 1)
    .fetch_all(&mut *c)
    .await?;
    if rows.len() > MAX_ROWS {
        return Ok(None);
    }
    let mut data = Vec::with_capacity(rows.len());
    for row in rows {
        // Each stored counter is safe, but several rows can overflow the JSON
        // integer range when grouped. Refuse a lossy dashboard projection.
        for field in ["count", "token_used", "quota"] {
            if !(0..=9_007_199_254_740_991).contains(&row.try_get::<i64, _>(field)?) {
                return Err(sqlx::Error::Protocol(
                    "unsafe historical usage aggregate".into(),
                ));
            }
        }
        data.push(json!({
            "user_id": row.try_get::<i64, _>("user_id")?,
            "username": row.try_get::<String, _>("username")?,
            "model_name": row.try_get::<String, _>("model_name")?,
            "created_at": row.try_get::<i64, _>("created_at")?,
            "count": row.try_get::<i64, _>("count")?,
            "token_used": row.try_get::<i64, _>("token_used")?,
            "quota": row.try_get::<i64, _>("quota")?,
            "usage_scope": selection.usage_scope,
            "quota_source": "legacy_quota_data",
        }));
    }
    Ok(Some(data))
}

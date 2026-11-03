//! Source-bound first and second cash payment facts for acquisition analytics.
use super::*;

pub(super) const NATIVE_CASH_EVENTS_SQL: &str = r#"SELECT i.created_by_user_id AS user_id,i.id AS order_id,floor(extract(epoch FROM min(r.created_at)))::bigint AS paid_at
 FROM core_identity.users u JOIN core_payments.intents i ON i.created_by_user_id=u.id AND i.account_id=u.personal_account_id AND i.environment='live' AND i.currency<>'LDC' AND i.credit_journal_id IS NOT NULL AND i.provider_transaction IS NOT NULL
 JOIN core_payments.receipts r ON r.intent_id=i.id AND r.method='credit' AND EXISTS(SELECT 1 FROM core_payments.provider_events e JOIN core_payments.accepted_proofs p ON p.channel_id=e.channel_id AND p.event_id=e.event_id AND p.intent_id=e.intent_id JOIN core_billing.ledger_journals j ON j.id=i.credit_journal_id AND j.kind='credit' WHERE e.intent_id=i.id AND e.receipt_id=r.id)
 WHERE u.id=ANY($1) GROUP BY i.created_by_user_id,i.id"#;

pub(super) const HISTORICAL_CASH_CTES: &str = r#"manifests AS MATERIALIZED (
 SELECT i.source_archive_sha256,i.manifest FROM core_migration.import_runs i
 WHERE i.manifest->>'scope'='wallet-topup-history.v1'
 AND i.manifest->>'complete'='true'
 AND i.manifest->>'source_archive_sha256'=i.source_archive_sha256
 AND jsonb_array_length(i.manifest->'rows')=(i.manifest->>'source_row_count')::bigint
), archives AS MATERIALIZED (
 SELECT s.source_key,m.manifest FROM core_history.finance_sources s
 JOIN manifests m ON m.source_archive_sha256=s.source_archive_sha256
 WHERE s.complete
 AND (SELECT count(*) FROM manifests a WHERE a.source_archive_sha256=s.source_archive_sha256)=1
 AND s.user_count=(SELECT count(*) FROM core_history.finance_users a WHERE a.source_key=s.source_key)
 AND s.topup_count=(SELECT count(*) FROM core_history.finance_topups t WHERE t.source_key=s.source_key)
 AND jsonb_array_length(m.manifest->'identity_map')=s.user_count
), raw_orders AS MATERIALIZED (
 SELECT a.source_key,(r->>'target_user_id')::bigint AS user_id,
 (r->>'target_personal_account_id')::bigint AS account_id,
 (r->'record'->>'id')::bigint AS order_id,(r->'record'->>'user_id')::bigint AS record_user_id,
 (r->'record'->>'complete_time')::bigint AS paid_at,r->'record'->>'status' AS status
 FROM archives a CROSS JOIN LATERAL jsonb_array_elements(a.manifest->'rows') r
), verified AS MATERIALIZED (
 SELECT a.* FROM archives a WHERE NOT EXISTS(
  SELECT 1 FROM core_history.finance_topups t WHERE t.source_key=a.source_key
  AND NOT EXISTS(SELECT 1 FROM raw_orders r WHERE r.source_key=a.source_key
    AND r.order_id=t.source_order_id AND r.status='success'))
), owner_maps AS MATERIALIZED (
 SELECT a.source_key,(owner->>'target_user_id')::bigint AS user_id,
 (owner->>'target_personal_account_id')::bigint AS account_id,
 coalesce((a.manifest->'per_source_user_counts'->>(owner->>'source_user_id'))::bigint,-1) AS expected_rows
 FROM verified a CROSS JOIN LATERAL jsonb_array_elements(a.manifest->'identity_map') owner
), mapped AS (
 SELECT u.id AS user_id,u.personal_account_id AS account_id,m.source_key
 FROM core_identity.users u
 JOIN core_history.finance_users f ON f.user_id=u.id AND f.account_id=u.personal_account_id
 JOIN owner_maps m ON m.source_key=f.source_key AND m.user_id=u.id AND m.account_id=u.personal_account_id
 WHERE u.id=ANY($1) AND m.expected_rows=(SELECT count(*) FROM raw_orders r
  WHERE r.source_key=m.source_key AND r.user_id=u.id AND r.account_id=u.personal_account_id)
), orders AS (
 SELECT m.user_id,r.paid_at,
 CASE WHEN r.record_user_id=m.user_id THEN t.classification END AS classification,
 t.currency,t.gross_amount_micros,t.refunded_amount_micros
 FROM mapped m JOIN raw_orders r ON r.source_key=m.source_key AND r.user_id=m.user_id AND r.account_id=m.account_id
 LEFT JOIN core_history.finance_topups t ON t.source_key=m.source_key AND t.source_order_id=r.order_id
 WHERE r.status='success'
)"#;

pub(super) const CASH_HISTORY_COVERED_SQL: &str = r#"(EXISTS(SELECT 1 FROM mapped m WHERE m.user_id=u.id) OR (
  NOT EXISTS(SELECT 1 FROM core_history.finance_users f WHERE f.user_id=u.id)
  AND NOT EXISTS(SELECT 1 FROM core_billing.opening_imports o WHERE o.account_id=u.personal_account_id)
  AND NOT EXISTS(SELECT 1 FROM core_migration.import_runs i WHERE
    coalesce(i.manifest->>'scope','identity-only')='identity-only'
    OR (i.manifest->>'scope'='legacy-cutover-identity.v1' AND i.manifest->'registration_times' ? u.personal_account_id::text))
 ))"#;

#[derive(Debug, Serialize, Deserialize)]
pub struct FirstCashPayment {
    pub user_id: i64,
    pub first_paid_at: Option<i64>,
    pub second_paid_at: Option<i64>,
    pub history_complete: bool,
    pub unclassified_orders: i64,
}
#[derive(Debug, Serialize, Deserialize)]
pub struct FirstCashPayments {
    pub users: Vec<FirstCashPayment>,
}

impl CommunityRead {
    /// Personal-wallet first two cash orders across native and imported history.
    /// Incomplete/unclassified history never supplies purported first/second dates.
    pub async fn first_cash_payments(
        &self,
        request: &UsersInfoRequest,
    ) -> Result<FirstCashPayments> {
        if request.user_ids.is_empty()
            || request.user_ids.len() > 100
            || request.user_ids.iter().any(|id| *id <= 0)
        {
            return Err(Error::Invalid);
        }
        let mut tx = self.snapshot().await?;
        let sql = format!(
            r#"WITH native_events AS MATERIALIZED ({NATIVE_CASH_EVENTS_SQL}), native AS (
 SELECT u.id AS user_id,ARRAY(SELECT e.paid_at FROM native_events e WHERE e.user_id=u.id AND NOT EXISTS(SELECT 1 FROM core_history.finance_sources s WHERE s.complete AND e.order_id<=s.native_intent_floor) ORDER BY e.paid_at,e.order_id LIMIT 2) AS paid_times FROM core_identity.users u WHERE u.id=ANY($1)
), {HISTORICAL_CASH_CTES}, historical AS (
 SELECT user_id,(array_agg(paid_at ORDER BY paid_at) FILTER(WHERE classification='cash' AND paid_at>0))[1:2] AS paid_times,
 count(*) FILTER(WHERE classification IS NULL OR classification IN ('invalid','unconfirmed')
   OR (classification='cash' AND coalesce(paid_at,0)<=0))::bigint AS unclassified
 FROM orders GROUP BY user_id
)
SELECT n.user_id,ARRAY(SELECT at FROM unnest(n.paid_times || coalesce(h.paid_times,'{{}}'::bigint[])) at ORDER BY at LIMIT 2) AS paid_times,
 coalesce(h.unclassified,0) AS unclassified,
 EXISTS(SELECT 1 FROM core_payments.intents i JOIN core_history.finance_sources s ON i.id<=s.native_intent_floor WHERE s.complete AND i.created_by_user_id=n.user_id AND i.account_id=u.personal_account_id AND i.credit_journal_id IS NOT NULL) AS invalid_native_seal,
 {CASH_HISTORY_COVERED_SQL} AS covered
FROM native n JOIN core_identity.users u ON u.id=n.user_id
LEFT JOIN historical h ON h.user_id=n.user_id ORDER BY n.user_id"#
        );
        let rows = sqlx::query(&sql)
            .bind(&request.user_ids)
            .fetch_all(&mut *tx)
            .await?;
        tx.commit().await?;
        let users = rows
            .into_iter()
            .map(|row| {
                let times: Vec<i64> = row.try_get("paid_times")?;
                let unclassified_orders: i64 = row.try_get("unclassified")?;
                let history_complete = row.try_get::<bool, _>("covered")?
                    && unclassified_orders == 0
                    && !row.try_get::<bool, _>("invalid_native_seal")?;
                Ok(FirstCashPayment {
                    user_id: row.try_get("user_id")?,
                    history_complete,
                    unclassified_orders,
                    first_paid_at: if history_complete {
                        times.first().copied()
                    } else {
                        None
                    },
                    second_paid_at: if history_complete {
                        times.get(1).copied()
                    } else {
                        None
                    },
                })
            })
            .collect::<Result<Vec<_>>>()?;
        Ok(FirstCashPayments { users })
    }
}

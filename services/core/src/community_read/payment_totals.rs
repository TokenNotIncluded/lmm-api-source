//! Cash amounts for orders credited in a half-open period. Refunds are the
//! cumulative, applied refunds of those orders as of the same read snapshot.
use super::payments::{CASH_HISTORY_COVERED_SQL, HISTORICAL_CASH_CTES, NATIVE_CASH_EVENTS_SQL};
use super::*;

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CashPeriodRequest {
    pub user_ids: Vec<i64>,
    pub start: i64,
    pub end: i64,
}
#[derive(Debug, Serialize, Deserialize)]
pub struct CashCurrency {
    pub currency: String,
    pub paid_micros: i64,
    pub refund_micros: i64,
    pub net_micros: i64,
    pub orders: i64,
}
#[derive(Debug, Serialize, Deserialize)]
pub struct CashPaymentUser {
    pub user_id: i64,
    pub unclassified_orders: i64,
    pub currencies: Vec<CashCurrency>,
}
#[derive(Debug, Serialize, Deserialize)]
pub struct CashPaymentTotals {
    pub users: Vec<CashPaymentUser>,
}

impl CommunityRead {
    pub async fn cash_payment_totals(&self, r: &CashPeriodRequest) -> Result<CashPaymentTotals> {
        if r.user_ids.is_empty()
            || r.user_ids.len() > 100
            || r.user_ids.iter().any(|id| *id <= 0)
            || r.end <= r.start
        {
            return Err(Error::Invalid);
        }
        Window {
            start: r.start,
            end: r.end,
            bucket_seconds: 0,
        }
        .validate()?;
        let mut tx = self.snapshot().await?;
        // A source overlap or an accepted-but-unapplied provider result is a gap,
        // not a zero refund or a second copy of an imported order.
        let invalid:bool=sqlx::query_scalar(r#"SELECT EXISTS(
 SELECT 1 FROM core_payments.intents i JOIN core_identity.users u ON u.id=i.created_by_user_id AND i.account_id=u.personal_account_id
 WHERE u.id=ANY($1) AND (
  (i.credit_journal_id IS NOT NULL AND EXISTS(SELECT 1 FROM core_history.finance_sources s WHERE s.complete AND i.id<=s.native_intent_floor))
  OR (i.environment='live' AND i.currency<>'LDC' AND (
   EXISTS(SELECT 1 FROM core_payments.accepted_proofs p WHERE p.intent_id=i.id AND (
    (i.credit_journal_id IS NULL AND p.verified_facts->>'kind' IN ('Credit','RefundSucceeded'))
    OR (i.credit_journal_id IS NOT NULL AND p.verified_facts->>'kind'='RefundSucceeded'
     AND NOT EXISTS(SELECT 1 FROM core_payments.provider_refunds f WHERE f.intent_id=i.id AND f.channel_id=p.channel_id AND f.refund_id=p.verified_facts->>'refund_id' AND f.amount_minor::text=p.verified_facts->>'amount_minor'))))
   OR (i.credit_journal_id IS NOT NULL AND i.refunded_minor::numeric<>(SELECT coalesce(sum(f.amount_minor::numeric),0) FROM core_payments.provider_refunds f WHERE f.intent_id=i.id))
  ))))"#).bind(&r.user_ids).fetch_one(&mut *tx).await?;
        if invalid {
            return Err(Error::Unavailable);
        }
        let sql = format!(
            r#"WITH {HISTORICAL_CASH_CTES}, native_events AS ({NATIVE_CASH_EVENTS_SQL}), amounts AS (
 SELECT o.user_id,o.currency,o.gross_amount_micros::numeric AS paid,o.refunded_amount_micros::numeric AS refunded
 FROM orders o WHERE o.classification='cash' AND o.paid_at>=$2 AND o.paid_at<$3
 UNION ALL
 SELECT e.user_id,i.currency,i.amount_minor::numeric*10000,i.refunded_minor::numeric*10000
 FROM native_events e JOIN core_payments.intents i ON i.id=e.order_id WHERE e.paid_at>=$2 AND e.paid_at<$3
), totals AS (
 SELECT user_id,currency,sum(paid)::bigint AS paid_micros,sum(refunded)::bigint AS refund_micros,
 (sum(paid)-sum(refunded))::bigint AS net_micros,count(*)::bigint AS orders
 FROM amounts GROUP BY user_id,currency
)
SELECT u.id AS user_id,{CASH_HISTORY_COVERED_SQL} AS covered,
 (SELECT count(*)::bigint FROM orders o WHERE o.user_id=u.id
 AND (o.classification IS NULL OR o.classification IN ('invalid','unconfirmed') OR (o.classification='cash' AND coalesce(o.paid_at,0)<=0))
 AND (coalesce(o.paid_at,0)<=0 OR (o.paid_at>=$2 AND o.paid_at<$3))) AS unclassified,
 coalesce((SELECT jsonb_agg(to_jsonb(t)-'user_id' ORDER BY currency) FROM totals t WHERE t.user_id=u.id),'[]'::jsonb) AS currencies
FROM core_identity.users u WHERE u.id=ANY($1) ORDER BY u.id"#
        );
        let rows = sqlx::query(&sql)
            .bind(&r.user_ids)
            .bind(r.start)
            .bind(r.end)
            .fetch_all(&mut *tx)
            .await?;
        let mut users = Vec::with_capacity(rows.len());
        for row in rows {
            if !row.try_get::<bool, _>("covered")? {
                return Err(Error::Unavailable);
            }
            let currencies: Vec<CashCurrency> = serde_json::from_value(row.try_get("currencies")?)
                .map_err(|_| Error::Unavailable)?;
            for c in &currencies {
                if [c.paid_micros, c.refund_micros, c.net_micros, c.orders]
                    .iter()
                    .any(|v| *v < 0 || *v > JS_SAFE)
                {
                    return Err(Error::TooDense);
                }
            }
            users.push(CashPaymentUser {
                user_id: row.try_get("user_id")?,
                unclassified_orders: row.try_get("unclassified")?,
                currencies,
            });
        }
        tx.commit().await?;
        Ok(CashPaymentTotals { users })
    }
}

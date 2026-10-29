//! Exact all-site audit projection; never imports, settles or prices money.
use super::{HistoryError, HistoryStore, ReadRoute, authorize};
use serde::Serialize;
use sqlx::Row;

/// Public order counts use the Web contract's exact JSON integer range.
const MAX_JSON_SAFE_COUNT: i64 = 9_007_199_254_740_991;

#[derive(Debug, Serialize)]
pub struct SiteStatistics {
    pub as_of: i64,
    pub credits_per_usd: i64,
    pub total_used_credits: String,
    pub total_balance_credits: String,
    pub recharge: RechargeStatistics,
}

#[derive(Debug, Serialize)]
pub struct RechargeStatistics {
    pub currencies: Vec<RechargeCurrency>,
    pub virtual_units: Vec<RechargeCurrency>,
    pub confirmed_orders: i64,
    pub unconfirmed_orders: i64,
    pub invalid_orders: i64,
}

#[derive(Debug, Serialize)]
pub struct RechargeCurrency {
    pub currency: String,
    pub gross_amount_micros: String,
    pub refunded_amount_micros: String,
    pub net_amount_micros: String,
    pub orders: i64,
}

impl HistoryStore {
    pub async fn site_statistics(&self, secret: &str) -> Result<SiteStatistics, HistoryError> {
        let mut tx = self.read_snapshot().await?;
        authorize(&mut tx, secret, ReadRoute::Administrator).await?;

        // A source is complete only with all its rows and the very same opening
        // lineage. Neither the global flag nor somebody else's history proves a
        // missing user's normalized consumption or original payment amounts.
        let invalid_sources: bool = sqlx::query_scalar(
            r#"SELECT EXISTS (
 SELECT 1 FROM core_history.finance_sources s WHERE NOT s.complete
 OR s.contract<>'core.site-finance-source.v1' OR s.credits_per_usd<>500000
 OR s.user_count<>(SELECT count(*) FROM core_history.finance_users u WHERE u.source_key=s.source_key)
 OR s.topup_count<>(SELECT count(*) FROM core_history.finance_topups t WHERE t.source_key=s.source_key)
) OR EXISTS (
 SELECT 1 FROM core_history.finance_users f
 JOIN core_history.finance_sources s USING(source_key)
 LEFT JOIN core_billing.account_opening_counters c ON c.account_id=f.account_id
 LEFT JOIN core_billing.opening_imports i ON i.migration_id=c.migration_id AND i.source_entity_key=c.source_entity_key
 LEFT JOIN core_billing.opening_import_runs r ON r.migration_id=c.migration_id
 LEFT JOIN core_identity.users u ON u.id=f.user_id
 WHERE c.account_id IS NULL OR i.account_id IS NULL OR r.migration_id IS NULL OR u.id IS NULL
 OR u.personal_account_id<>f.account_id OR c.user_id<>f.user_id
 OR c.used_quota<>f.raw_used_quota OR c.source_row_sha256<>f.source_row_sha256
 OR i.account_id<>f.account_id OR i.source_row_sha256<>c.source_row_sha256
 OR r.source_archive_sha256<>s.source_archive_sha256 OR r.mapping_sha256<>s.mapping_sha256
 OR c.source_archive_sha256<>s.source_archive_sha256 OR c.mapping_sha256<>s.mapping_sha256
 OR r.observed_at<>s.observed_at OR c.observed_at<>s.observed_at
) OR EXISTS (
 SELECT 1 FROM core_history.finance_sources s
 CROSS JOIN LATERAL unnest(s.native_request_ids) sealed(request_id)
 LEFT JOIN core_charging.requests r ON r.id=sealed.request_id
 LEFT JOIN core_history.finance_users f ON f.user_id=r.actor_user_id
 WHERE r.id IS NULL OR (r.state='settled' AND f.user_id IS NOT NULL)
)"#,
        )
        .fetch_one(&mut *tx)
        .await?;
        if invalid_sources {
            return Err(HistoryError::Unavailable);
        }

        let credits = sqlx::query(
            r#"WITH users AS (
 SELECT u.id,u.personal_account_id,c.account_id AS opening_account,
   c.used_quota,f.normalized_used_credits,
   EXISTS(SELECT 1 FROM core_billing.opening_imports i WHERE i.account_id=u.personal_account_id) AS imported,
   EXISTS(SELECT 1 FROM core_identity.audit a WHERE a.actor_user_id=u.id
     AND a.action IN ('user.register','user.bootstrap','installation.first-owner')
     AND a.target->>'user_id'=u.id::text) AS native
 FROM core_identity.users u
 LEFT JOIN core_billing.account_opening_counters c ON c.user_id=u.id AND c.account_id=u.personal_account_id
 LEFT JOIN core_history.finance_users f ON f.user_id=u.id AND f.account_id=u.personal_account_id
), wallets AS (
 SELECT u.id,count(l.id) AS buckets,count(b.ledger_account_id) AS balances,
   count(DISTINCT l.bucket) AS distinct_buckets,
   count(l.id) FILTER(WHERE l.bucket IN ('wallet','reserved','debt','affiliate')) AS known_buckets,
   COALESCE(sum(b.balance_units::numeric) FILTER(WHERE l.bucket IN ('wallet','debt')),0) AS balance
 FROM users u LEFT JOIN core_billing.ledger_accounts l ON l.owner_account_id=u.personal_account_id
 LEFT JOIN core_billing.balance_state b ON b.ledger_account_id=l.id GROUP BY u.id
), native_usage AS (
 SELECT COALESCE(sum(r.settled::numeric-r.refunded::numeric),0) AS used,
   count(*) FILTER(WHERE r.settled<r.refunded) AS invalid
 FROM core_charging.requests r JOIN users u ON u.id=r.actor_user_id WHERE r.state='settled'
)
SELECT floor(extract(epoch FROM statement_timestamp()))::bigint AS as_of,
 (SELECT count(*) FROM users WHERE (opening_account IS NULL AND (imported OR NOT native))
   OR (opening_account IS NOT NULL AND normalized_used_credits IS NULL)) AS missing,
 (SELECT count(*) FROM wallets w JOIN users u USING(id)
   WHERE (w.buckets=0 AND u.opening_account IS NOT NULL)
      OR (w.buckets<>0 AND (w.buckets<>4 OR w.balances<>4 OR w.distinct_buckets<>4 OR w.known_buckets<>4))) AS invalid_wallets,
 (SELECT invalid FROM native_usage) AS invalid_usage,
 (COALESCE((SELECT sum(normalized_used_credits::numeric) FROM users),0)
   +(SELECT used FROM native_usage))::text AS used,
 COALESCE((SELECT sum(balance) FROM wallets),0)::text AS balance"#,
        )
        .fetch_one(&mut *tx)
        .await?;
        for field in ["missing", "invalid_wallets", "invalid_usage"] {
            if credits.try_get::<i64, _>(field)? != 0 {
                return Err(HistoryError::Unavailable);
            }
        }

        // Accepted provider evidence can precede ledger application. Until the
        // exact refund is applied, do not publish an apparently final net cash
        // total. Events of the same refund are deduplicated by provider ID.
        let payments_complete: bool = sqlx::query_scalar(
            "SELECT COALESCE(bool_and(complete),FALSE) FROM core_history.finance_native_payments",
        )
        .fetch_one(&mut *tx)
        .await?;
        if !payments_complete {
            return Err(HistoryError::Unavailable);
        }

        let rows = sqlx::query(
            r#"WITH payments AS (
 SELECT classification,currency,gross_amount_micros::numeric AS gross,
   refunded_amount_micros::numeric AS refunded,1::bigint AS orders
 FROM core_history.finance_topups WHERE classification IN ('cash','virtual')
 UNION ALL
 SELECT 'cash',currency,gross_amount_micros,refunded_amount_micros,orders
 FROM core_history.finance_native_payments WHERE currency IS NOT NULL
)
SELECT classification,currency,sum(gross)::text AS gross,sum(refunded)::text AS refunded,
 (sum(gross)-sum(refunded))::text AS net,sum(orders)::bigint AS orders
FROM payments GROUP BY classification,currency ORDER BY classification,currency COLLATE "C""#,
        )
        .fetch_all(&mut *tx)
        .await?;
        let counts = sqlx::query(
            r#"SELECT count(*) FILTER(WHERE classification='unconfirmed')::bigint AS unconfirmed,
 count(*) FILTER(WHERE classification='invalid')::bigint AS invalid
 FROM core_history.finance_topups"#,
        )
        .fetch_one(&mut *tx)
        .await?;
        let mut recharge = RechargeStatistics {
            currencies: Vec::new(),
            virtual_units: Vec::new(),
            confirmed_orders: 0,
            unconfirmed_orders: counts.try_get("unconfirmed")?,
            invalid_orders: counts.try_get("invalid")?,
        };
        for row in rows {
            let metric = RechargeCurrency {
                currency: row.try_get("currency")?,
                gross_amount_micros: row.try_get("gross")?,
                refunded_amount_micros: row.try_get("refunded")?,
                net_amount_micros: row.try_get("net")?,
                orders: row.try_get("orders")?,
            };
            recharge.confirmed_orders = recharge
                .confirmed_orders
                .checked_add(metric.orders)
                .ok_or(HistoryError::Unavailable)?;
            if row.try_get::<String, _>("classification")? == "virtual" {
                recharge.virtual_units.push(metric);
            } else {
                recharge.currencies.push(metric);
            }
        }
        if [
            recharge.confirmed_orders,
            recharge.unconfirmed_orders,
            recharge.invalid_orders,
        ]
        .into_iter()
        .any(|count| count > MAX_JSON_SAFE_COUNT)
        {
            return Err(HistoryError::Unavailable);
        }
        let result = SiteStatistics {
            as_of: credits.try_get("as_of")?,
            credits_per_usd: 500_000,
            total_used_credits: credits.try_get("used")?,
            total_balance_credits: credits.try_get("balance")?,
            recharge,
        };
        tx.commit().await?;
        Ok(result)
    }
}

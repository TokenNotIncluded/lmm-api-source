// Copyright (C) 2026 LIghtJUNction. AGPL-3.0-or-later.
//! Personal, immutable legacy SMS financial history; never calls a supplier.
use super::{HistoryError, HistoryStore, Result, personal_session};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use sqlx::Row;
use std::collections::BTreeMap;

#[derive(Default, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct SmsHistoryQuery {
    pub p: Option<u32>,
    pub page_size: Option<u32>,
}
#[derive(Serialize)]
pub struct SmsLedgerEntry {
    pub entry_type: String,
    pub raw_amount_units: String,
    pub created_at: i64,
}
#[derive(Serialize)]
pub struct SmsOrder {
    pub id: String,
    pub service: String,
    pub country_id: i64,
    pub operator: String,
    pub status: String,
    /// Original quote, not proof of payment or a conversion of raw quota.
    pub quoted_price_usd: Option<String>,
    pub raw_reserved_units: String,
    pub raw_refunded_units: String,
    pub raw_net_debit_units: String,
    pub created_at: i64,
    pub completed_at: Option<i64>,
    pub ledger: Vec<SmsLedgerEntry>,
}
#[derive(Serialize)]
pub struct SmsHistory {
    pub contract: &'static str,
    pub observed_at: i64,
    pub page: u32,
    pub page_size: u32,
    pub total: i64,
    pub items: Vec<SmsOrder>,
    pub quota_unit: &'static str,
    pub payload_available: bool,
    pub supplier_operations_available: bool,
}
const SCOPE: &str = "hero-sms-history.v1";
const OWN_ORDERS: &str = "WITH own AS (
 SELECT o FROM core_migration.import_runs i
 CROSS JOIN LATERAL jsonb_array_elements(i.manifest->'tables'->'hero_sms_sms_orders') o
 WHERE i.migration_id=$1 AND (o->>'user_id')::bigint=$2
 AND (o->>'history_hidden_at')::bigint=0
) ";

impl HistoryStore {
    pub async fn hero_sms_history(&self, secret: &str, q: &SmsHistoryQuery) -> Result<SmsHistory> {
        let mut tx = self.read_snapshot().await?;
        let user = personal_session(&mut tx, secret).await?;
        let page = q.p.unwrap_or(1);
        let size = q.page_size.unwrap_or(20);
        if page == 0 || page > 100_000 || size == 0 || size > 100 {
            return Err(HistoryError::Invalid);
        }
        let receipts = sqlx::query("SELECT migration_id,manifest-'tables' AS metadata,jsonb_array_length(manifest->'tables'->'hero_sms_sms_orders')::bigint AS orders,jsonb_array_length(manifest->'tables'->'hero_sms_sms_quota_ledgers')::bigint AS ledgers,EXISTS(SELECT FROM core_billing.opening_import_runs o WHERE o.source_archive_sha256=i.manifest->>'source_archive_sha256' AND o.observed_at=(i.manifest->>'observed_at')::bigint) AS bound FROM core_migration.import_runs i WHERE manifest->>'scope'=$1 LIMIT 2")
            .bind(SCOPE).fetch_all(&mut *tx).await?;
        let [receipt] = receipts.as_slice() else {
            return Err(HistoryError::Unavailable);
        };
        let meta: Value = receipt.try_get("metadata")?;
        let bound: bool = receipt.try_get("bound")?;
        if !bound
            || meta["history_complete"] != true
            || meta["source_namespace"] != "lmm_api.lmm_prod_20260802.hero_sms_sms"
            || meta["balance_effect"] != "none"
            || meta["quota_unit"] != "legacy_raw_no_conversion"
            || meta["source_order_count"].as_i64() != Some(receipt.try_get("orders")?)
            || meta["source_ledger_count"].as_i64() != Some(receipt.try_get("ledgers")?)
            || !meta["source_user_ids"]
                .as_array()
                .is_some_and(|ids| ids.contains(&Value::from(user)))
        {
            return Err(HistoryError::Unavailable);
        }
        let migration: String = receipt.try_get("migration_id")?;
        let total: i64 = sqlx::query_scalar(&(OWN_ORDERS.to_owned() + "SELECT count(*) FROM own"))
            .bind(&migration)
            .bind(user)
            .fetch_one(&mut *tx)
            .await?;
        let rows = sqlx::query(&(OWN_ORDERS.to_owned()+"SELECT o->>'id' AS id,o->>'service' AS service,(o->>'country_id')::bigint AS country_id,o->>'operator' AS operator,o->>'status' AS status,o->>'customer_price_usd' AS price,(o->>'reserved_quota') AS reserved,(o->>'refunded_quota') AS refunded,((o->>'reserved_quota')::bigint-(o->>'refunded_quota')::bigint)::text AS net,(o->>'created_at')::bigint AS created,(o->>'completed_at')::bigint AS completed FROM own ORDER BY created DESC,id DESC LIMIT $3 OFFSET $4"))
            .bind(&migration).bind(user).bind(i64::from(size)).bind(i64::from(page-1)*i64::from(size)).fetch_all(&mut *tx).await?;
        let ids: Vec<String> = rows
            .iter()
            .map(|r| r.try_get("id"))
            .collect::<std::result::Result<_, _>>()?;
        let ledger_rows = sqlx::query("SELECT l->>'order_id' AS order_id,l->>'entry_type' AS kind,l->>'amount_quota' AS amount,(l->>'created_at')::bigint AS created FROM core_migration.import_runs i CROSS JOIN LATERAL jsonb_array_elements(i.manifest->'tables'->'hero_sms_sms_quota_ledgers') l WHERE i.migration_id=$1 AND (l->>'user_id')::bigint=$2 AND l->>'order_id'=ANY($3) ORDER BY created,(l->>'id')::bigint")
            .bind(&migration).bind(user).bind(&ids).fetch_all(&mut *tx).await?;
        let mut ledger: BTreeMap<String, Vec<SmsLedgerEntry>> = BTreeMap::new();
        for r in ledger_rows {
            ledger
                .entry(r.try_get("order_id")?)
                .or_default()
                .push(SmsLedgerEntry {
                    entry_type: r.try_get("kind")?,
                    raw_amount_units: r.try_get("amount")?,
                    created_at: r.try_get("created")?,
                });
        }
        let mut items = Vec::with_capacity(rows.len());
        for r in rows {
            let id: String = r.try_get("id")?;
            let entries = ledger.remove(&id).unwrap_or_default();
            items.push(SmsOrder {
                id,
                service: r.try_get("service")?,
                country_id: r.try_get("country_id")?,
                operator: r.try_get("operator")?,
                status: r.try_get("status")?,
                quoted_price_usd: r.try_get("price")?,
                raw_reserved_units: r.try_get("reserved")?,
                raw_refunded_units: r.try_get("refunded")?,
                raw_net_debit_units: r.try_get("net")?,
                created_at: r.try_get("created")?,
                completed_at: r.try_get("completed")?,
                ledger: entries,
            });
        }
        let result = SmsHistory {
            contract: "core.hero-sms-history.v1",
            observed_at: meta["observed_at"]
                .as_i64()
                .ok_or(HistoryError::Unavailable)?,
            page,
            page_size: size,
            total,
            items,
            quota_unit: "legacy_raw_no_conversion",
            payload_available: false,
            supplier_operations_available: false,
        };
        tx.commit().await?;
        Ok(result)
    }
}

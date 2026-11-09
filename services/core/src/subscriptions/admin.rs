//! Administrator reads and revocation of existing subscription grants.
//! These operations neither erase usage nor create, refund or transfer money.
use super::*;
use serde::Deserialize;

const MAX_ID: i64 = 9_007_199_254_740_991;
pub(crate) const RECORDS_SQL: &str = include_str!("admin_records.sql");
pub(crate) const INVALIDATE_SQL: &str = include_str!("admin_invalidate.sql");

#[derive(Clone, Debug, Default, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct AdminGrantRecordsQuery {
    pub page: Option<u32>,
    pub page_size: Option<u32>,
    pub query: Option<String>,
    pub user_id: Option<i64>,
    pub plan_id: Option<i64>,
    pub status: Option<String>,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
pub struct AdminGrantRecord {
    pub id: i64,
    pub user_id: i64,
    pub username: String,
    pub email: String,
    pub grant_reference: String,
    pub source: String,
    /// Only a durable native purchase proves its plan and frozen revision.
    /// Imported grant identities do not contain an authoritative plan mapping.
    pub plan: Option<Plan>,
    pub starts_at: i64,
    pub ends_at: i64,
    pub window_start: i64,
    pub window_end: i64,
    pub limit_units: String,
    pub unlimited: bool,
    pub used_units: String,
    pub reserved_units: String,
    pub active: bool,
    pub status: String,
    pub purchase_id: Option<i64>,
    pub refund_journal_id: Option<i64>,
    pub renewal_canceled: Option<bool>,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
pub struct AdminGrantRecords {
    pub contract: String,
    pub items: Vec<AdminGrantRecord>,
    pub total: i64,
    pub page: u32,
    pub page_size: u32,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct AdminGrantInvalidation {
    pub operation_key: String,
    pub user_id: i64,
    pub reason: String,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
pub struct GrantInvalidationReceipt {
    pub contract: String,
    pub operation_key: String,
    pub subscription_id: i64,
    pub user_id: i64,
    pub invalidated_at: i64,
    pub renewal_stopped: bool,
}

impl SubscriptionStore {
    pub async fn admin_records(
        &self,
        secret: &str,
        input: AdminGrantRecordsQuery,
    ) -> Result<AdminGrantRecords> {
        let mut tx = self.pool.begin().await?;
        for statement in [
            "SET TRANSACTION ISOLATION LEVEL REPEATABLE READ READ ONLY",
            "SET LOCAL statement_timeout='5s'",
            "SET LOCAL lock_timeout='500ms'",
        ] {
            sqlx::query(statement).execute(&mut *tx).await?;
        }
        identity::permissions::authorize_admin_read_snapshot(&mut tx, secret)
            .await
            .map_err(|error| match error {
                identity::admin_users::AdminUsersError::Unauthorized => Error::Unauthorized,
                identity::admin_users::AdminUsersError::Forbidden => Error::Forbidden,
                _ => Error::Unavailable,
            })?;
        let page = input.page.unwrap_or(1);
        let page_size = input.page_size.unwrap_or(20);
        let query = input.query.unwrap_or_default();
        let status = input.status.unwrap_or_else(|| "all".into());
        if !(1..=100_000).contains(&page)
            || !(1..=100).contains(&page_size)
            || query.chars().count() > 200
            || query.chars().any(char::is_control)
            || [input.user_id, input.plan_id]
                .into_iter()
                .flatten()
                .any(|id| !(1..=MAX_ID).contains(&id))
            || !matches!(
                status.as_str(),
                "all" | "active" | "scheduled" | "expired" | "invalidated" | "refunded"
            )
        {
            return Err(Error::Invalid);
        }
        let time = now(&mut tx).await?;
        let Json(value): Json<Value> = sqlx::query_scalar(RECORDS_SQL)
            .bind(time)
            .bind(query.trim())
            .bind(input.user_id)
            .bind(input.plan_id)
            .bind(status)
            .bind(i64::from(page_size))
            .bind(i64::from(page - 1) * i64::from(page_size))
            .bind(i64::from(page))
            .fetch_one(&mut *tx)
            .await?;
        let result = decode(value)?;
        tx.commit().await?;
        Ok(result)
    }

    /// Stop new use and renewal without moving an existing request to another
    /// grant, changing its held amount or inventing a financial refund.
    pub async fn invalidate_grant(
        &self,
        secret: &str,
        id: i64,
        input: AdminGrantInvalidation,
    ) -> Result<GrantInvalidationReceipt> {
        let mut tx = self.begin().await?;
        // Grant revocation changes financial policy; reuse the same root gate
        // as plan prices, including complete imported administrator policy.
        let a = actor(&mut tx, secret, true).await?;
        operation_key(&input.operation_key)?;
        if !(1..=MAX_ID).contains(&id)
            || !(1..=MAX_ID).contains(&input.user_id)
            || input.reason.trim().is_empty()
            || input.reason.len() > 500
            || input.reason.chars().any(char::is_control)
        {
            return Err(Error::Invalid);
        }
        lock(&mut tx, a.user_id).await?;
        let request = json!(["invalidate_grant", id, input]);
        if let Some(result) = prior(&mut tx, a.user_id, &input.operation_key, &request).await? {
            tx.commit().await?;
            return Ok(result);
        }
        // Match the purchase/refund path: purchase row before its charging
        // grant. A concurrent refund keeps its original charge and receipt.
        let _: Option<i64> = sqlx::query_scalar(
            "SELECT id FROM core_subscriptions.purchases WHERE subscription_id=$1 FOR UPDATE",
        )
        .bind(id)
        .fetch_optional(&mut *tx)
        .await?;
        let (user_id, active): (i64, bool) = sqlx::query_as(
            "SELECT u.id,s.active FROM core_charging.subscriptions s
             JOIN core_identity.users u ON u.personal_account_id=s.account_id
             WHERE s.id=$1 FOR UPDATE OF s",
        )
        .bind(id)
        .fetch_optional(&mut *tx)
        .await?
        .ok_or(Error::NotFound)?;
        if user_id != input.user_id || !active {
            return Err(Error::Conflict);
        }
        let time = now(&mut tx).await?;
        let Json(result): Json<Value> = sqlx::query_scalar(INVALIDATE_SQL)
            .bind(id)
            .bind(user_id)
            .bind(a.user_id)
            .bind(a.credential_id)
            .bind(&input.operation_key)
            .bind(Json(request))
            .bind(time)
            .bind(&input.reason)
            .fetch_one(&mut *tx)
            .await?;
        let result = decode(result)?;
        tx.commit().await?;
        Ok(result)
    }
}

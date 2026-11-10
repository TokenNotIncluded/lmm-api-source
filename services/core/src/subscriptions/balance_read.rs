//! Read an original wallet purchase without confirming, spending or granting.
use super::*;
use serde::Deserialize;

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct BalancePurchaseStatus {
    pub quote: Quote,
    pub receipt: Option<Receipt>,
}
impl SubscriptionStore {
    pub async fn read_balance_purchase(
        &self,
        secret: &str,
        id: &str,
    ) -> Result<BalancePurchaseStatus> {
        token(id)?;
        let mut tx = self.begin().await?;
        let a = actor(&mut tx, secret, false).await?;
        let(account,result):(i64,Option<Value>)=sqlx::query_as("SELECT account_id,result FROM core_subscriptions.quotes WHERE id=$1 AND actor_user_id=$2")
   .bind(id).bind(a.user_id).fetch_optional(&mut *tx).await?.ok_or(Error::NotFound)?;
        if account != a.owner_account_id {
            return Err(Error::NotFound);
        }
        let quote = read_quote(&mut tx, id, a.user_id).await?;
        let receipt = result.map(decode).transpose()?;
        tx.commit().await?;
        Ok(BalancePurchaseStatus { quote, receipt })
    }
}

impl SubscriptionStore {
    /// Read the owner-reviewed legacy balance original. Its debit and historical
    /// grant were independently imported; reading never creates either again.
    pub async fn read_legacy_balance_purchase(
        &self,
        secret: &str,
        reference: &str,
    ) -> Result<Value> {
        if reference.is_empty() || reference.len() > 128 {
            return Err(Error::Invalid);
        }
        let mut tx = self.begin().await?;
        let a = actor(&mut tx, secret, false).await?;
        let result:Value=sqlx::query_scalar("SELECT result FROM core_subscriptions.commands WHERE actor_user_id=$1 AND request->>'kind'='legacy_balance_original_v1' AND result->>'reference'=$2 AND (result->>'account_id')::bigint=$3").bind(a.user_id).bind(reference).bind(a.owner_account_id).fetch_optional(&mut *tx).await?.ok_or(Error::NotFound)?;
        tx.commit().await?;
        Ok(result)
    }
}

impl SubscriptionStore {
    /// UNKNOWN preserves source identity, not a fabricated quote or entitlement.
    /// Restore of original materials is an independently reviewed owner action.
    pub async fn legacy_subscription_unknowns(&self, secret: &str) -> Result<Vec<Value>> {
        let mut tx = self.begin().await?;
        let a = actor(&mut tx, secret, false).await?;
        let rows=sqlx::query_scalar("SELECT result FROM core_subscriptions.commands WHERE actor_user_id=$1 AND request->>'kind'='legacy_subscription_unknown_v1' AND (result->>'account_id')::bigint=$2 ORDER BY operation_key LIMIT 100").bind(a.user_id).bind(a.owner_account_id).fetch_all(&mut *tx).await?;
        tx.commit().await?;
        Ok(rows)
    }
    pub async fn read_legacy_subscription_unknown(
        &self,
        secret: &str,
        reference: &str,
    ) -> Result<Value> {
        if reference.is_empty() || reference.len() > 128 {
            return Err(Error::Invalid);
        }
        let mut tx = self.begin().await?;
        let a = actor(&mut tx, secret, false).await?;
        let row=sqlx::query_scalar("SELECT result FROM core_subscriptions.commands WHERE actor_user_id=$1 AND request->>'kind'='legacy_subscription_unknown_v1' AND result->>'reference'=$2 AND (result->>'account_id')::bigint=$3").bind(a.user_id).bind(reference).bind(a.owner_account_id).fetch_optional(&mut *tx).await?.ok_or(Error::NotFound)?;
        tx.commit().await?;
        Ok(row)
    }
}

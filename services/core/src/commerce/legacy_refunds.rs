//! Historical paid balance orders use owner-reviewed source grants, never a
//! fabricated native pay. Only the original seller Session executes; merchant
//! reconciliation reads the exact already persisted operation and cannot pay.
use super::{Account, AccountKind, CommerceStore, Error, Receipt, Result, hash_id, hex, id};
use crate::identity::CredentialKind;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use sqlx::types::Json;

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Command {
    pub key: String,
    pub kind: String,
    pub shop_id: String,
    pub order_id: String,
    pub source_row_sha256: String,
    pub buyer: Account,
    pub seller: Account,
    pub credits: i64,
    pub quantity: i64,
}
impl Command {
    fn parse(bytes: &[u8]) -> Result<(Self, Vec<u8>)> {
        if bytes.len() > 8192 {
            return Err(Error::Invalid);
        }
        let [c]: [Self; 1] = serde_json::from_slice(bytes).map_err(|_| Error::Invalid)?;
        if c.kind != "legacy_refund"
            || !hash_id(&c.shop_id)
            || !hash_id(&c.order_id)
            || !hash_id(&c.source_row_sha256)
            || c.buyer.kind != AccountKind::Personal
            || c.seller.kind != AccountKind::Personal
            || c.buyer.id <= 0
            || c.seller.id <= 0
            || !(1..=9_007_199_254_740_991).contains(&c.credits)
            || c.quantity <= 0
            || c.key
                != id(&serde_json::json!([
                    "store-legacy-refund-v1",
                    c.shop_id,
                    c.order_id
                ]))
        {
            return Err(Error::Invalid);
        }
        Ok((c, Sha256::digest(bytes).to_vec()))
    }
}

/// Used only for transport dispatch. The selected typed decoder still rejects
/// duplicate/unknown fields and enforces the exact command and canonical key.
pub fn is_command(bytes: &[u8]) -> bool {
    serde_json::from_slice::<serde_json::Value>(bytes)
        .ok()
        .is_some_and(|v| {
            v.get(0)
                .and_then(|v| v.get("kind"))
                .and_then(|v| v.as_str())
                == Some("legacy_refund")
        })
}
impl CommerceStore {
    pub async fn execute_legacy_refund(&self, secret: &str, bytes: &[u8]) -> Result<Receipt> {
        let (command, hash) = Command::parse(bytes)?;
        for attempt in 0..4 {
            let result = self.once_legacy_refund(secret, &command, &hash).await;
            match result {
                Err(Error::Retry) if attempt < 3 => tokio::task::yield_now().await,
                other => return other,
            }
        }
        Err(Error::Pending)
    }
    async fn once_legacy_refund(&self, secret: &str, c: &Command, hash: &[u8]) -> Result<Receipt> {
        let mut tx = self.begin().await?;
        let actor = crate::identity::authorize_in_transaction(&mut tx, secret).await?;
        if actor.credential_kind != CredentialKind::Session {
            return Err(Error::Forbidden);
        }
        let result = sqlx::query_scalar::<_, Json<Receipt>>(
            "SELECT core_commerce.finalize_legacy_refund($1,$2,$3,$4)",
        )
        .bind(Json(c))
        .bind(hash)
        .bind(actor.user_id)
        .bind(actor.owner_account_id)
        .fetch_one(&mut *tx)
        .await;
        let Json(receipt) = match result {
            Ok(receipt) => receipt,
            Err(e) => {
                return Err(
                    match e.as_database_error().and_then(|e| e.code()).as_deref() {
                        Some("P1000") => Error::Invalid,
                        Some("P1002") => Error::Missing,
                        Some("P1003") => Error::Pending,
                        Some("42501") => Error::Forbidden,
                        _ => e.into(),
                    },
                );
            }
        };
        // COMMIT transport failure is UNKNOWN; the merchant retains this command.
        tx.commit().await.map_err(|_| Error::Pending)?;
        Ok(receipt)
    }
    pub async fn reconcile_legacy_refund(&self, bytes: &[u8]) -> Result<Option<Receipt>> {
        let (command, hash) = Command::parse(bytes)?;
        let mut tx = self.begin().await?;
        let row: Option<(Json<Command>, Vec<u8>, Json<Receipt>)> = sqlx::query_as(
            "SELECT o.command,o.command_sha256,o.receipt FROM core_commerce.legacy_refund_operations o JOIN core_commerce.legacy_refund_grants g USING(order_id) WHERE o.operation_key=$1 AND g.order_id=$2 AND g.shop_id=$3")
            .bind(&command.key).bind(&command.order_id).bind(&command.shop_id).fetch_optional(&mut *tx).await?;
        let receipt = match row {
            None => None,
            Some((Json(saved), digest, Json(receipt))) => {
                if saved != command || digest != hash || receipt.command_hash != hex(&hash) {
                    return Err(Error::Conflict);
                }
                Some(receipt)
            }
        };
        tx.commit().await?;
        Ok(receipt)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn historical_command_is_full_bound_and_separate_from_native_pay() {
        let shop = "a".repeat(64);
        let order = "b".repeat(64);
        let mut c = serde_json::json!({"key":id(&serde_json::json!(["store-legacy-refund-v1",shop,order])),"kind":"legacy_refund","shop_id":shop,"order_id":order,"source_row_sha256":"c".repeat(64),"buyer":{"kind":"personal","id":2},"seller":{"kind":"personal","id":1},"credits":100,"quantity":1});
        assert!(Command::parse(&serde_json::to_vec(&[&c]).unwrap()).is_ok());
        c["payment_ref"] = "fake-native-pay".into();
        assert!(Command::parse(&serde_json::to_vec(&[&c]).unwrap()).is_err());
        c.as_object_mut().unwrap().remove("payment_ref");
        c["credits"] = 0.into();
        assert!(Command::parse(&serde_json::to_vec(&[&c]).unwrap()).is_err());
    }
}

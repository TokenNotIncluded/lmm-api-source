//! Frozen legacy terms and verified input are separate from financial application.
use serde::{Deserialize, Serialize};
use serde_json::json;
use sha2::{Digest, Sha256};
use sqlx::{PgConnection, types::Json};

use super::{
    Channel, Error, FrozenPancakeQuote, PaymentsStore, Result,
    identifiers::{self, Subject},
    verify_legacy_pancake,
};
use crate::billing::ledger::{Action, Outcome, Request};

#[derive(sqlx::FromRow)]
struct Basis {
    id: String,
    user_id: i64,
    account_id: i64,
    channel_id: i64,
    provider: String,
    merchant: String,
    environment: String,
    currency: String,
    order_reference: String,
    store_id: String,
    source_product_id: Option<String>,
    amount_micros: i64,
    credit_units: i64,
}

/// Durable proof metadata only; signed bytes and financial state stay private.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize, sqlx::FromRow)]
pub struct LegacyProofReceipt {
    pub proof_id: String,
    pub basis_id: String,
    pub channel_id: i64,
    pub order_reference: String,
    pub event_id: String,
    pub order_id: String,
    pub payment_id: Option<String>,
    pub payload_sha256: Vec<u8>,
    pub accepted_unix_seconds: i64,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct LegacyPaymentReceipt {
    pub operation_id: String,
    pub basis_id: String,
    pub proof_id: String,
    pub outcome: Outcome,
}

fn digest_id(prefix: &str, value: serde_json::Value) -> String {
    format!(
        "{prefix}_{}",
        crate::hex_codec::encode(Sha256::digest(value.to_string().as_bytes()))
    )
}

async fn lock_basis(c: &mut PgConnection, id: &str) -> Result<()> {
    sqlx::query("SELECT pg_advisory_xact_lock(hashtextextended($1,38590271))")
        .bind(id)
        .execute(c)
        .await?;
    Ok(())
}

fn channel<'a>(store: &'a PaymentsStore, basis: &Basis) -> Result<&'a Channel> {
    let c = store
        .channels
        .get(&basis.channel_id)
        .ok_or(Error::Unavailable)?;
    if c.provider != basis.provider
        || c.merchant != basis.merchant
        || c.environment != basis.environment
        || c.currency != basis.currency
    {
        return Err(Error::Conflict);
    }
    Ok(c)
}

async fn application(
    c: &mut PgConnection,
    operation: &str,
) -> Result<Option<LegacyPaymentReceipt>> {
    let row: Option<(String, String, Json<Outcome>)> = sqlx::query_as(
        "SELECT basis_id,proof_id,outcome FROM core_payments.legacy_applications WHERE operation_id=$1",
    ).bind(operation).fetch_optional(c).await?;
    Ok(
        row.map(|(basis_id, proof_id, Json(outcome))| LegacyPaymentReceipt {
            operation_id: operation.to_owned(),
            basis_id,
            proof_id,
            outcome,
        }),
    )
}

impl PaymentsStore {
    /// Core ingress only: the owner must already have installed an approved
    /// basis tied to the original opening account. This never posts money.
    pub async fn admit_legacy_pancake(
        &self,
        channel_id: i64,
        order_reference: &str,
        payload: &[u8],
        signature: &[u8],
    ) -> Result<LegacyProofReceipt> {
        let mut tx = self.begin().await?;
        let b: Basis = sqlx::query_as(
            "SELECT * FROM core_payments.legacy_bases WHERE channel_id=$1 AND order_reference=$2",
        )
        .bind(channel_id)
        .bind(order_reference)
        .fetch_optional(&mut *tx)
        .await?
        .ok_or(Error::NotFound)?;
        lock_basis(&mut tx, &b.id).await?;
        let c = channel(self, &b)?;
        let quote = FrozenPancakeQuote {
            order_reference: &b.order_reference,
            currency: &b.currency,
            amount_micros: b.amount_micros,
            credit_units: b.credit_units,
            source_store_id: Some(&b.store_id),
            source_product_id: b.source_product_id.as_deref(),
        };
        let verified = verify_legacy_pancake(payload, signature, c, &quote, super::now()?)?;
        let previous: Vec<(String, Option<String>)> = sqlx::query_as(
            "SELECT order_id,payment_id FROM core_payments.legacy_proofs WHERE basis_id=$1",
        )
        .bind(&b.id)
        .fetch_all(&mut *tx)
        .await?;
        if previous.iter().any(|(order, payment)| {
            order != &verified.order_id
                || matches!((payment, &verified.payment_id), (Some(a),Some(b)) if a!=b)
        }) {
            return Err(Error::Conflict);
        }
        let mut aliases = vec![
            ("event", verified.event_id.as_str()),
            ("order", verified.order_id.as_str()),
        ];
        if let Some(payment) = &verified.payment_id {
            aliases.push(("payment", payment));
        }
        identifiers::claim(&mut tx, c, Subject::Legacy(&b.id), aliases).await?;
        let prior: Option<(String, String, Vec<u8>)> = sqlx::query_as(
            "SELECT id,basis_id,payload_sha256 FROM core_payments.legacy_proofs WHERE provider=$1 AND merchant=$2 AND environment=$3 AND event_id=$4",
        ).bind(&b.provider).bind(&b.merchant).bind(&b.environment).bind(&verified.event_id)
            .fetch_optional(&mut *tx).await?;
        if let Some((_id, basis, hash)) = prior {
            if basis != b.id || hash != verified.payload_sha256 {
                return Err(Error::Conflict);
            }
            tx.commit().await?;
            return self
                .get_legacy_proof(
                    channel_id,
                    order_reference,
                    &verified.event_id,
                    &verified.payload_sha256,
                )
                .await;
        }
        let id = digest_id(
            "lproof",
            json!([b.id, verified.event_id, verified.payload_sha256]),
        );
        sqlx::query("INSERT INTO core_payments.legacy_proofs(id,basis_id,provider,merchant,environment,event_id,order_id,payment_id,payload_sha256,signed_payload,signature) VALUES($1,$2,$3,$4,$5,$6,$7,$8,$9,$10,$11)")
            .bind(&id).bind(&b.id).bind(&b.provider).bind(&b.merchant).bind(&b.environment)
            .bind(&verified.event_id).bind(&verified.order_id).bind(&verified.payment_id)
            .bind(verified.payload_sha256.as_slice()).bind(payload).bind(signature)
            .execute(&mut *tx).await?;
        tx.commit().await?;
        self.get_legacy_proof(
            channel_id,
            order_reference,
            &verified.event_id,
            &verified.payload_sha256,
        )
        .await
    }

    /// Reconcile an ingress commit without re-verifying an expired signature.
    /// The selector binds the original channel namespace, order, event and bytes.
    /// No missing proof is admitted here and this method never posts money.
    pub async fn get_legacy_proof(
        &self,
        channel_id: i64,
        order_reference: &str,
        event_id: &str,
        payload_sha256: &[u8],
    ) -> Result<LegacyProofReceipt> {
        if !super::valid_id(order_reference)
            || !super::valid_id(event_id)
            || payload_sha256.len() != 32
        {
            return Err(Error::Invalid);
        }
        let c = self.channels.get(&channel_id).ok_or(Error::NotFound)?;
        if c.provider != "waffo_pancake" {
            return Err(Error::Invalid);
        }
        let receipt: LegacyProofReceipt = sqlx::query_as(
            "SELECT p.id AS proof_id,p.basis_id,b.channel_id,b.order_reference,p.event_id,p.order_id,p.payment_id,p.payload_sha256,floor(extract(epoch FROM p.accepted_at))::bigint AS accepted_unix_seconds FROM core_payments.legacy_proofs p JOIN core_payments.legacy_bases b ON b.id=p.basis_id WHERE b.channel_id=$1 AND b.order_reference=$2 AND p.event_id=$3 AND b.provider=$4 AND b.merchant=$5 AND b.environment=$6 AND b.currency=$7",
        ).bind(channel_id).bind(order_reference).bind(event_id).bind(&c.provider)
            .bind(&c.merchant).bind(&c.environment).bind(&c.currency)
            .fetch_optional(&self.pool).await?.ok_or(Error::NotFound)?;
        if receipt.payload_sha256 != payload_sha256 {
            return Err(Error::Conflict);
        }
        Ok(receipt)
    }

    /// Apply a durable proof. Retry an uncertain commit using the SAME key.
    /// A new key can retry a permanent rejected receipt after reconciliation;
    /// a posted basis always returns its original immutable receipt.
    pub async fn apply_legacy_payment(
        &self,
        proof_id: &str,
        operation_key: &str,
    ) -> Result<LegacyPaymentReceipt> {
        if !(16..=128).contains(&operation_key.len())
            || !operation_key.bytes().all(|b| b.is_ascii_graphic())
        {
            return Err(Error::Invalid);
        }
        let mut tx = self.begin().await?;
        let (basis_id, order_id, payment_id): (String, String, Option<String>) = sqlx::query_as(
            "SELECT basis_id,order_id,payment_id FROM core_payments.legacy_proofs WHERE id=$1",
        )
        .bind(proof_id)
        .fetch_optional(&mut *tx)
        .await?
        .ok_or(Error::NotFound)?;
        lock_basis(&mut tx, &basis_id).await?;
        let b: Basis = sqlx::query_as("SELECT * FROM core_payments.legacy_bases WHERE id=$1")
            .bind(&basis_id)
            .fetch_one(&mut *tx)
            .await?;
        channel(self, &b)?;
        let operation_id = digest_id("lapply", json!([basis_id, operation_key]));
        if let Some(prior) = application(&mut tx, &operation_id).await? {
            // The financial request is the immutable basis plus operation key.
            // A later verified delivery may enrich ORD/PAY proof metadata;
            // retain the original proof and outcome instead of applying again.
            tx.commit().await?;
            return Ok(prior);
        }
        let posted: Option<String> = sqlx::query_scalar(
            "SELECT operation_id FROM core_payments.legacy_applications WHERE basis_id=$1 AND journal_id IS NOT NULL",
        ).bind(&basis_id).fetch_optional(&mut *tx).await?;
        if let Some(id) = posted {
            let receipt = application(&mut tx, &id).await?.ok_or(Error::Storage)?;
            tx.commit().await?;
            return Ok(receipt);
        }
        sqlx::query("SELECT core_billing.open_ledger_wallet($1)")
            .bind(b.account_id)
            .execute(&mut *tx)
            .await?;
        let transaction = payment_id.as_deref().unwrap_or(&order_id);
        let reference_hash =
            super::hash(json!([b.provider, b.merchant, b.environment, transaction]));
        let payment_reference = format!(
            "{}:{}",
            b.provider,
            crate::hex_codec::encode(reference_hash)
        );
        let request = Request {
            scope: "legacy_payments".into(),
            operation_key: operation_id.clone(),
            actor_user_id: Some(b.user_id),
            reason: "verified frozen legacy top-up".into(),
            action: Action::Credit {
                account_id: b.account_id,
                amount_units: super::units(b.credit_units)?,
                payment_reference,
            },
        };
        request.validate().map_err(|_| Error::Invalid)?;
        let Json(outcome): Json<Outcome> =
            sqlx::query_scalar("SELECT core_billing.post_ledger($1)")
                .bind(Json(request))
                .fetch_one(&mut *tx)
                .await?;
        let journal = match &outcome {
            Outcome::Posted { journal_id, .. } => Some(*journal_id),
            _ => None,
        };
        sqlx::query("INSERT INTO core_payments.legacy_applications(operation_id,basis_id,proof_id,journal_id,outcome) VALUES($1,$2,$3,$4,$5)")
            .bind(&operation_id).bind(&basis_id).bind(proof_id).bind(journal).bind(Json(&outcome))
            .execute(&mut *tx).await?;
        tx.commit().await?;
        Ok(LegacyPaymentReceipt {
            operation_id,
            basis_id,
            proof_id: proof_id.to_owned(),
            outcome,
        })
    }
}

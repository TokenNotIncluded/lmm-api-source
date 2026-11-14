//! Exact 56-component predecessor; no new self-purchase or FX writer objects.
#[path = "navigation_options_predecessor.rs"]
mod frozen46;
use sha2::{Digest, Sha256};

pub async fn install(pool: &sqlx::PgPool) {
    frozen46::install(pool).await;
    let additions = [
        (
            "navigation-options",
            include_str!("../../schema/navigation_options.sql"),
        ),
        ("red-packets", include_str!("../../schema/red_packets.sql")),
        (
            "legacy-history-compat",
            include_str!("../../schema/legacy_history_compat.sql"),
        ),
        (
            "legacy-payments",
            include_str!("../../schema/legacy_payments.sql"),
        ),
        (
            "legacy-payment-opening-identity",
            include_str!("../../schema/legacy_payment_opening_identity.sql"),
        ),
        (
            "affiliate-claims",
            include_str!("../../schema/affiliate_claims.sql"),
        ),
        (
            "referral-ledger-revision",
            include_str!("../../schema/referral_ledger_revision.sql"),
        ),
        (
            "legacy-referrals",
            include_str!("../../schema/legacy_referrals.sql"),
        ),
        (
            "connection-history",
            include_str!("../../schema/connection_history.sql"),
        ),
        (
            "directory-ads",
            include_str!("../../schema/directory_ads.sql"),
        ),
    ];
    let mut tx = pool.begin().await.unwrap();
    for (name, ddl) in additions {
        sqlx::raw_sql(ddl).execute(&mut *tx).await.unwrap();
        sqlx::query("INSERT INTO core_meta.component_contract(name,fingerprint) VALUES($1,$2)")
            .bind(name)
            .bind(Sha256::digest(ddl.as_bytes()).to_vec())
            .execute(&mut *tx)
            .await
            .unwrap();
    }
    let mut manifest = lmm_core::core_schema::schema_manifest();
    manifest.components.truncate(56);
    let mut bytes = serde_json::to_vec(&manifest).unwrap();
    bytes.push(b'\n');
    assert_eq!(
        hex_codec::encode(Sha256::digest(bytes)),
        super::PRE_MANIFEST
    );
    let definitions: Vec<String> =
        sqlx::query_scalar(sqlx::AssertSqlSafe(manifest.catalog.query_utf8))
            .fetch_all(&mut *tx)
            .await
            .unwrap();
    let mut hash = Sha256::new();
    for definition in definitions {
        hash.update((definition.len() as u64).to_be_bytes());
        hash.update(definition.as_bytes());
    }
    let catalog = hash.finalize();
    assert_eq!(hex_codec::encode(catalog), super::PRE_CATALOG);
    sqlx::query("UPDATE core_meta.runtime_contract SET catalog_fingerprint=$1 WHERE singleton")
        .bind(catalog.as_slice())
        .execute(&mut *tx)
        .await
        .unwrap();
    tx.commit().await.unwrap();
}

use crate::hex_codec;

//! Reconstruct the exact Core66 catalog in an isolated empty fixture database.
//! Full historical manifest/catalog pins are checked after removing only Core67.
use sha2::{Digest, Sha256};
pub async fn install(pool: &sqlx::PgPool) {
    lmm_core::core_schema::init(pool).await.unwrap();
    let mut tx = pool.begin().await.unwrap();
    sqlx::raw_sql("DROP TABLE core_meta.pancake_language_upgrades; ALTER TABLE core_subscriptions.pancake_prices DROP CONSTRAINT pancake_prices_checkout_language_check; ALTER TABLE core_subscriptions.pancake_prices ADD CONSTRAINT pancake_prices_checkout_language_check CHECK(length(checkout_language) BETWEEN 1 AND 32); DELETE FROM core_meta.component_contract WHERE name='subscription-pancake-language-optional';")
        .execute(&mut *tx).await.unwrap();
    let mut manifest = lmm_core::core_schema::schema_manifest();
    manifest.components.truncate(66);
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

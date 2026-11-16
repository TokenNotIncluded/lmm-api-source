//! Derive the exact Core66 fixture from the source-bound retained Core67 installer.
//! Both full captured catalogs are checked before/after removing only its suffix.
use sha2::{Digest, Sha256};
pub async fn install(pool: &sqlx::PgPool) {
    use super::harness::preserved_upgrade_executor;
    let mut tx = pool.begin().await.unwrap();
    sqlx::raw_sql("DROP TABLE core_meta.pancake_language_upgrades; ALTER TABLE core_subscriptions.pancake_prices DROP CONSTRAINT pancake_prices_checkout_language_check; ALTER TABLE core_subscriptions.pancake_prices ADD CONSTRAINT pancake_prices_checkout_language_check CHECK(length(checkout_language) BETWEEN 1 AND 32); DELETE FROM core_meta.component_contract WHERE name='subscription-pancake-language-optional';")
        .execute(&mut *tx).await.unwrap();
    assert_eq!(
        preserved_upgrade_executor::manifest_sha256(66),
        super::PRE_MANIFEST
    );
    let definitions: Vec<String> = sqlx::query_scalar(sqlx::AssertSqlSafe(
        preserved_upgrade_executor::catalog_query(),
    ))
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

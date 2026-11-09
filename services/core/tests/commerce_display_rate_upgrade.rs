//! Real, explicit Core56 -> Core58 upgrade and immutable replay receipts.
const ERROR_PREFIX: &str = "COMMERCE_DISPLAY_RATE";
const PROCEDURE: &str = "core-commerce-display-rate-v1";
const PRE_MANIFEST: &str = "f2340fb850e81f168ababe0301d80bd85769b235a8c5b5abb91f0bcd7429624e";
const PRE_CATALOG: &str = "46cbd8e0a9e9bb6c55cb899558b40fa8c8df1affa2e366cbbde733a8527bda43";
const RECEIPT: &str = "core_meta.commerce_display_rate_upgrades";
#[path = "common/suffix_upgrade.rs"]
mod harness;
#[path = "common/commerce_display_rate_predecessor.rs"]
mod predecessor;

async fn seed_history(pool: &sqlx::PgPool) {
    sqlx::query(
        "INSERT INTO core_console.options(key,value,revision) VALUES('USDExchangeRate','7.25',3)",
    )
    .execute(pool)
    .await
    .unwrap();
}

async fn check_history(c: &mut sqlx::PgConnection) {
    let option: (String, i64) = sqlx::query_as(
        "SELECT value,revision FROM core_console.options WHERE key='USDExchangeRate'",
    )
    .fetch_one(&mut *c)
    .await
    .unwrap();
    assert_eq!(option, ("7.25".into(), 3));
    let forbidden: bool = sqlx::query_scalar("SELECT EXISTS(SELECT FROM pg_constraint WHERE conrelid='core_commerce.orders'::regclass AND conname='orders_check')")
        .fetch_one(&mut *c).await.unwrap();
    assert!(!forbidden, "same-wallet order constraint was not upgraded");
}

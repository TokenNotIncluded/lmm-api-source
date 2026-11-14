//! Real Core66 -> Core67 upgrade, exact request replay and financial preservation.
const ERROR_PREFIX: &str = "PANCAKE_LANGUAGE";
const PROCEDURE: &str = "core-subscription-pancake-language-optional-v1";
const PRE_MANIFEST: &str = "424ab67e21d21b7147b01e5f438a1d5938fa16ad9bc464f05d3aa8223e487d9f";
const PRE_CATALOG: &str = "be0b981025782d501487bcc8b9cd9c59bccf9cae2351c77708a1c1ce16b0358e";
const RECEIPT: &str = "core_meta.pancake_language_upgrades";
#[path = "common/suffix_upgrade.rs"]
mod harness;
#[path = "common/pancake_language_predecessor.rs"]
mod predecessor;
async fn seed_history(_: &sqlx::PgPool) {}
async fn check_history(c: &mut sqlx::PgConnection) {
    let definition: String = sqlx::query_scalar("SELECT pg_get_constraintdef(oid) FROM pg_constraint WHERE conrelid='core_subscriptions.pancake_prices'::regclass AND conname='pancake_prices_checkout_language_check'")
        .fetch_one(&mut *c).await.unwrap();
    assert_eq!(definition, "CHECK ((length(checkout_language) <= 32))");
    let not_null: bool = sqlx::query_scalar("SELECT attnotnull FROM pg_attribute WHERE attrelid='core_subscriptions.pancake_prices'::regclass AND attname='checkout_language'")
        .fetch_one(c).await.unwrap();
    assert!(not_null);
}

#[path = "../src/hex_codec.rs"]
mod hex_codec;

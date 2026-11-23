//! Install real dependencies for deliberately partial payment fixtures.
//! Historical component definitions and their guards remain unchanged.
#[allow(dead_code)]
pub async fn install(pool: &sqlx::PgPool) {
    // Every caller already installs payments::SCHEMA, including the immutable
    // legacy payment basis and its opening-identity suffix. Install it once;
    // re-running that DDL fails before any discount behavior is exercised.
    let payment_basis_present: bool = sqlx::query_scalar(
        "SELECT to_regclass('core_payments.legacy_bases') IS NOT NULL \
         AND to_regclass('core_payments.legacy_proofs') IS NOT NULL \
         AND to_regclass('core_payments.legacy_applications') IS NOT NULL",
    )
    .fetch_one(pool)
    .await
    .unwrap();
    assert!(
        payment_basis_present,
        "payments::SCHEMA must precede continuity"
    );
    for ddl in [
        include_str!("../../schema/migration.sql"),
        // Component 81 replaces the history guard; its schema is a real prerequisite.
        include_str!("../../schema/history.sql"),
        lmm_core::promotions::SCHEMA,
        lmm_core::promotions::red_packets::SCHEMA,
        lmm_core::payments::discounts::CONTINUITY_SCHEMA,
    ] {
        sqlx::raw_sql(ddl).execute(pool).await.unwrap();
    }
}

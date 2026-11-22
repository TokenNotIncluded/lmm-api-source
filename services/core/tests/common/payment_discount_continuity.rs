//! Install real dependencies for deliberately partial payment fixtures.
//! Historical component definitions and their guards remain unchanged.
#[allow(dead_code)]
pub async fn install(pool: &sqlx::PgPool) {
    for ddl in [
        include_str!("../../schema/migration.sql"),
        lmm_core::promotions::SCHEMA,
        lmm_core::promotions::red_packets::SCHEMA,
        include_str!("../../schema/legacy_payments.sql"),
        lmm_core::payments::discounts::CONTINUITY_SCHEMA,
    ] {
        sqlx::raw_sql(ddl).execute(pool).await.unwrap();
    }
}

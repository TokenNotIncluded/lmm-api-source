//! Frozen 42-component SQLx fixture; no use of the latest initializer.
// This frozen module also exposes the 41-only entry point used by its own suite.
#[allow(dead_code)]
#[path = "appeals_predecessor.rs"]
mod frozen41;
pub async fn install(pool: &sqlx::PgPool) {
    frozen41::install_suffix(pool, true).await;
}

//! Separate 45 -> 46 upgrade; old policy, money and projection work stay unchanged.
const ERROR_PREFIX: &str = "CHANNEL_OBSERVATIONS";
const PROCEDURE: &str = "core-channel-observations-v1";
const PRE_MANIFEST: &str = "567d84fa06adf6916a62b6a5de0ae4234da1c0876972254f8393f0a55cd8d9c8";
const PRE_CATALOG: &str = "b3e3db14753bcb575eba90541b09cf9fac2955a22e513741a91b1f11019dbe6f";
const RECEIPT: &str = "core_meta.channel_observations_upgrades";
#[path = "common/canonical_native_history.rs"]
mod canonical;
#[path = "common/suffix_upgrade.rs"]
mod harness;
#[path = "common/channel_accounting_history.rs"]
#[allow(dead_code)]
mod history;
#[path = "common/channel_observations_predecessor.rs"]
mod predecessor;
async fn seed_history(pool: &sqlx::PgPool) {
    canonical::seed_history(pool).await;
}
async fn check_history(c: &mut sqlx::PgConnection) {
    let state:(bool,i64,i64)=sqlx::query_as("SELECT ready,(SELECT count(*) FROM core_channels.projection_dirty),(SELECT count(*) FROM core_channels.usage_contributions) FROM core_channels.projection_state WHERE singleton")
        .fetch_one(&mut *c).await.unwrap();
    assert_eq!(
        state,
        (true, 2, 0),
        "unrelated upgrade must preserve pending projection work"
    );
    assert_eq!(
        sqlx::query_scalar::<_, i64>("SELECT count(*) FROM core_channels.catalog_observations")
            .fetch_one(&mut *c)
            .await
            .unwrap(),
        0,
        "upgrade cannot relabel legacy probes or fabricate freshness"
    );
    let authority:(i64,i64,i64)=sqlx::query_as("SELECT (SELECT count(*) FROM core_trust_pricing.policy_edit_signing_key),(SELECT count(*) FROM core_trust_pricing.policy_editors),(SELECT count(*) FROM core_trust_pricing.policy_edit_audit)")
        .fetch_one(c).await.unwrap();
    assert_eq!(
        authority,
        (0, 0, 0),
        "upgrade cannot invent editor keys or approval"
    );
}

#[sqlx::test(migrations = false)]
async fn fresh_current_is_bound_to_captured_target_and_seeds_no_authority(pool: sqlx::PgPool) {
    lmm_core::core_schema::init(&pool).await.unwrap();
    assert_eq!(lmm_core::core_schema::component_count(), 47);
    // Deliberately fails until central captures and approves target46. Never skip.
    lmm_core::core_schema::check(&pool)
        .await
        .expect("capture and review current target catalog first");
    let empty:bool=sqlx::query_scalar("SELECT NOT EXISTS(SELECT FROM core_channels.catalog_observations) AND NOT EXISTS(SELECT FROM core_meta.channel_observations_upgrades) AND NOT EXISTS(SELECT FROM core_trust_pricing.policy_edit_signing_key) AND NOT EXISTS(SELECT FROM core_trust_pricing.policy_editors) AND NOT EXISTS(SELECT FROM core_identity.users) AND NOT EXISTS(SELECT FROM core_billing.ledger_journals) AND NOT EXISTS(SELECT FROM core_billing.ledger_entries)")
        .fetch_one(&pool).await.unwrap();
    assert!(empty);
}

#[sqlx::test(migrations = false)]
async fn changing_catalog_and_saved_receipt_cannot_redefine_current_target(pool: sqlx::PgPool) {
    use sha2::{Digest, Sha256};
    lmm_core::core_schema::init(&pool).await.unwrap();
    lmm_core::core_schema::check(&pool)
        .await
        .expect("capture current target first");
    sqlx::raw_sql("CREATE TABLE core_channels.unreviewed_display_column(id bigint)")
        .execute(&pool)
        .await
        .unwrap();
    let manifest = lmm_core::core_schema::schema_manifest();
    let definitions: Vec<String> = sqlx::query_scalar(manifest.catalog.query_utf8)
        .fetch_all(&pool)
        .await
        .unwrap();
    let mut hash = Sha256::new();
    for definition in definitions {
        hash.update((definition.len() as u64).to_be_bytes());
        hash.update(definition.as_bytes());
    }
    sqlx::query("UPDATE core_meta.runtime_contract SET catalog_fingerprint=$1 WHERE singleton")
        .bind(hash.finalize().to_vec())
        .execute(&pool)
        .await
        .unwrap();
    assert!(lmm_core::core_schema::check(&pool).await.is_err());
}

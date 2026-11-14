//! Separate 44 -> 45 upgrade; old policy, money and projection work stay unchanged.
const ERROR_PREFIX: &str = "TRUST_POLICY_EDIT";
const PROCEDURE: &str = "core-trust-policy-edit-v1";
const PRE_MANIFEST: &str = "5f6b75aaae3110c460c58270aac77028d8d4c32360481a52831872515131261d";
const PRE_CATALOG: &str = "d4cb2dbfd24de133c92dbd8128ea691d50dc93f72c4753ec995135d081b8f584";
const RECEIPT: &str = "core_meta.trust_policy_upgrades";
#[path = "common/canonical_native_history.rs"]
mod canonical;
#[path = "common/suffix_upgrade.rs"]
mod harness;
#[path = "common/channel_accounting_history.rs"]
#[allow(dead_code)]
mod history;
#[path = "common/trust_policy_predecessor.rs"]
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
    let authority:(i64,i64,i64)=sqlx::query_as("SELECT (SELECT count(*) FROM core_trust_pricing.policy_edit_signing_key),(SELECT count(*) FROM core_trust_pricing.policy_editors),(SELECT count(*) FROM core_trust_pricing.policy_edit_audit)")
        .fetch_one(c).await.unwrap();
    assert_eq!(
        authority,
        (0, 0, 0),
        "upgrade cannot invent editor keys or approval"
    );
}

#[path = "../src/hex_codec.rs"]
mod hex_codec;

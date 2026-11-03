//! Separate 46 -> 47 upgrade; old policy, money and projection work stay unchanged.
const ERROR_PREFIX: &str = "NAVIGATION_OPTIONS";
const PROCEDURE: &str = "core-navigation-options-v1";
const PRE_MANIFEST: &str = "7e69eafaa516a67dfc1b719ae2f5a950a90596f912f47d187422c470f9c170ee";
const PRE_CATALOG: &str = "c1b305360f39946be511368c8aba21635158dc767ca65eaab380784673a5d9a6";
const RECEIPT: &str = "core_meta.navigation_options_upgrades";
#[path = "common/canonical_native_history.rs"]
mod canonical;
#[path = "common/suffix_upgrade.rs"]
mod harness;
#[path = "common/channel_accounting_history.rs"]
#[allow(dead_code)]
mod history;
#[path = "common/navigation_options_predecessor.rs"]
mod predecessor;
async fn seed_history(pool: &sqlx::PgPool) {
    canonical::seed_history(pool).await;
    sqlx::query("INSERT INTO core_console.options(key,value,revision) VALUES ('HeaderNavModules','{\"home\":false}',7)")
        .execute(pool).await.unwrap();
}
async fn check_history(c: &mut sqlx::PgConnection) {
    let option: (String, i64) = sqlx::query_as(
        "SELECT value,revision FROM core_console.options WHERE key='HeaderNavModules'",
    )
    .fetch_one(&mut *c)
    .await
    .unwrap();
    assert_eq!(option, ("{\"home\":false}".into(), 7));
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

//! Exact 42 -> 43 with real synthetic deleted and reused channel history.
const ERROR_PREFIX: &str = "CHANNEL_ACCOUNTING";
const PROCEDURE: &str = "core-channel-accounting-v1";
const PRE_MANIFEST: &str = "cbc4b9567d9754250afb1646324e61fc13c9368d69e72c5506fa692ad169cd8f";
const PRE_CATALOG: &str = "e1c5e00088abac46cdec398830bd275945f9e269d2aef33bbe6fd1982c227076";
const RECEIPT: &str = "core_meta.channel_accounting_upgrades";
#[path = "common/channel_accounting_history.rs"]
mod history;
#[path = "common/accounting_predecessor.rs"]
mod predecessor;
use history::{check_history, seed_history};
#[path = "common/suffix_upgrade.rs"]
mod harness;

#[sqlx::test(migrations = false)]
async fn unpublished_bootstrap_receipts_reserve_ids_without_inventing_retirement(
    pool: sqlx::PgPool,
) {
    predecessor::install(&pool).await;
    lmm_core::identity::IdentityStore::from_pool(pool.clone())
        .bootstrap_user(7001, 6)
        .await
        .unwrap();
    history::seed_native_history(&pool).await;
    sqlx::query("INSERT INTO core_identity.audit(actor_user_id,action,target) VALUES(7001,'channel.delete','{\"change\":{\"id\":202}}')")
        .execute(&pool).await.unwrap();
    sqlx::raw_sql(include_str!("../schema/channel_accounting.sql"))
        .execute(&pool)
        .await
        .unwrap();
    let ids: Vec<(i64,String,bool,bool)> = sqlx::query_as("SELECT id,origin,history_ambiguous,retired_at IS NOT NULL FROM core_channels.identities ORDER BY id")
        .fetch_all(&pool).await.unwrap();
    assert_eq!(
        ids,
        vec![
            (201, "migrated".into(), false, false),
            (202, "migrated".into(), false, true),
            (203, "migrated".into(), false, false)
        ]
    );
    let counts:(i64,i64)=sqlx::query_as("SELECT (SELECT count(*) FROM core_channels.counter_baselines),(SELECT count(*) FROM core_channels.balance_observations)")
        .fetch_one(&pool).await.unwrap();
    assert_eq!(
        counts,
        (0, 0),
        "reserved IDs are not proof of current active configuration or complete lineage"
    );
}

//! Exact frozen 43 -> 44; original facts/ACLs/sequences survive canonical backfill.
const ERROR_PREFIX: &str = "CHANNEL_PROJECTION";
const PROCEDURE: &str = "core-channel-projection-v1";
const PRE_MANIFEST: &str = "f12d5e81ca219acdc59ac07be931d52e46f0b64974f1e2b3154ad102b5cc0c1c";
const PRE_CATALOG: &str = "1f9ef4951b101320e4016f2e8347985c7a321b4c3405fe756864abba1bf7c680";
const RECEIPT: &str = "core_meta.channel_projection_upgrades";
#[path = "common/canonical_native_history.rs"]
mod canonical;
#[path = "common/suffix_upgrade.rs"]
mod harness;
#[path = "common/channel_accounting_history.rs"]
#[allow(dead_code)]
mod history;
#[path = "common/projection_predecessor.rs"]
mod predecessor;
async fn seed_history(pool: &sqlx::PgPool) {
    canonical::seed_history(pool).await;
}
async fn check_history(c: &mut sqlx::PgConnection) {
    let state:(bool,i64,i64)=sqlx::query_as("SELECT ready,(SELECT count(*) FROM core_channels.projection_dirty),(SELECT count(*) FROM core_channels.usage_contributions) FROM core_channels.projection_state WHERE singleton")
        .fetch_one(&mut *c).await.unwrap();
    assert_eq!(state, (true, 0, 2));
    let contributions:Vec<(i64,i64,i64)>=sqlx::query_as("SELECT channel_id,pending_requests,unknown_requests FROM core_channels.usage_contributions ORDER BY channel_id")
        .fetch_all(&mut *c).await.unwrap();
    assert_eq!(contributions, vec![(0, 0, 1), (203, 1, 0)]);
    // Direct historical facts predate registry admission; projection must reserve
    // neither a false current identity nor fabricated lineage to display them.
    let ids: i64 = sqlx::query_scalar("SELECT count(*) FROM core_channels.identities")
        .fetch_one(&mut *c)
        .await
        .unwrap();
    assert_eq!(ids, 0);
}

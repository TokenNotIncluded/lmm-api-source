//! Explicit owner rebuild; no startup, runtime HTTP, or read-time repair path.
use super::*;
use sqlx::Connection;

pub(super) const SOURCE_LOCK_ORDER: &[&str] = &[
    "core_charging.requests",
    "core_charging.usage_events",
    "core_charging.ledger_commands",
    "core_charging.refunds",
];
const REBUILD_LOCKS: &str = "LOCK TABLE core_charging.requests,core_charging.usage_events,core_charging.ledger_commands,core_charging.refunds,core_channels.usage_contributions,core_channels.usage_rollups,core_channels.projection_dirty,core_channels.projection_state IN SHARE ROW EXCLUSIVE MODE";
async fn owner(c: &mut PgConnection) -> Result<(), IdentityError> {
    let allowed: bool = sqlx::query_scalar(
        r#"
SELECT current_user=session_user
 AND (SELECT pg_catalog.pg_get_userbyid(datdba)=current_user FROM pg_catalog.pg_database WHERE datname=pg_catalog.current_database())
 AND pg_catalog.count(*)=12 AND pg_catalog.bool_and(pg_catalog.pg_get_userbyid(c.relowner)=current_user)
FROM pg_catalog.pg_class c WHERE c.oid IN (
 pg_catalog.to_regclass('core_charging.requests'),pg_catalog.to_regclass('core_charging.usage_events'),
 pg_catalog.to_regclass('core_charging.ledger_commands'),pg_catalog.to_regclass('core_charging.refunds'),
 pg_catalog.to_regclass('core_channels.usage_contributions'),pg_catalog.to_regclass('core_channels.usage_rollups'),
 pg_catalog.to_regclass('core_channels.projection_dirty'),pg_catalog.to_regclass('core_channels.projection_state'),
 pg_catalog.to_regclass('core_meta.component_contract'),pg_catalog.to_regclass('core_meta.runtime_contract'),
 pg_catalog.to_regclass('core_channels.opening_sources'),pg_catalog.to_regclass('core_channels.opening_baselines'))
"#,
    )
    .fetch_one(c)
    .await?;
    if !allowed {
        return Err(IdentityError::Forbidden);
    }
    Ok(())
}

/// Rebuild the derived channel usage projection with current-schema owner
/// authority. Every original source row and ACL remains outside the write set.
/// A detached connection ensures cancellation releases the session fence.
pub async fn rebuild_channel_usage(pool: &PgPool) -> Result<u64, IdentityError> {
    let mut c = pool.acquire().await?.detach();
    let result = rebuild_connected(&mut c).await;
    let _ = c.close().await;
    result
}
async fn rebuild_connected(c: &mut PgConnection) -> Result<u64, IdentityError> {
    owner(c).await?;
    sqlx::raw_sql("SET lock_timeout='3s'; SET statement_timeout='15s'; SET idle_in_transaction_session_timeout='20s'; SET search_path=pg_catalog")
        .execute(&mut *c).await?;
    // Acquire outside BEGIN: waiting must not fix a stale transaction snapshot.
    sqlx::query("SELECT pg_catalog.pg_advisory_lock(741803120)")
        .execute(&mut *c)
        .await?;
    let mut tx = c.begin().await?;
    sqlx::raw_sql("SET TRANSACTION ISOLATION LEVEL SERIALIZABLE; SET LOCAL synchronous_commit=on")
        .execute(&mut *tx)
        .await?;
    // First statement that accesses relations: source order precedes projection.
    sqlx::raw_sql(REBUILD_LOCKS).execute(&mut *tx).await?;
    owner(&mut tx).await?;
    check_connection(&mut tx).await?;
    let count = crate::usage::channel_projection::backfill_channel_usage(&mut tx)
        .await
        .map_err(|_| IdentityError::Storage)?;
    let ready:bool=sqlx::query_scalar("SELECT ready AND NOT EXISTS(SELECT FROM core_channels.projection_dirty) FROM core_channels.projection_state WHERE singleton")
        .fetch_one(&mut *tx).await?;
    if !ready {
        return Err(IdentityError::Storage);
    }
    tx.commit().await?;
    Ok(count)
}

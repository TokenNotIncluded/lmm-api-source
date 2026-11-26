//! A PostgreSQL clock and transaction lock bound capacity across serving processes.
use super::{Binding, Config};
use serde::Serialize;
use sqlx::{PgConnection, PgPool};
use std::collections::BTreeMap;

pub(crate) struct Mapping {
    pub(crate) channel: u64,
    pub(crate) revision: Vec<u8>,
}

pub(crate) async fn lookup(pool: &PgPool, key: &[u8; 32]) -> sqlx::Result<Option<Mapping>> {
    let row: Option<(i64, Vec<u8>)> = sqlx::query_as(
        "SELECT channel_id,binding_sha256 FROM core_channels.affinity_cache
         WHERE key_sha256=$1 AND expires_at>clock_timestamp()",
    )
    .bind(key.as_slice())
    .fetch_optional(pool)
    .await?;
    Ok(row.map(|(channel, revision)| Mapping {
        channel: channel as u64,
        revision,
    }))
}
pub(crate) async fn invalidate(
    pool: &PgPool,
    key: &[u8; 32],
    mapping: &Mapping,
) -> sqlx::Result<()> {
    sqlx::query("DELETE FROM core_channels.affinity_cache WHERE key_sha256=$1 AND channel_id=$2 AND binding_sha256=$3")
        .bind(key.as_slice()).bind(mapping.channel as i64).bind(&mapping.revision).execute(pool).await?;
    Ok(())
}
async fn lock(c: &mut PgConnection) -> sqlx::Result<()> {
    sqlx::query("SELECT pg_advisory_xact_lock(730814197,193617101)")
        .execute(c)
        .await?;
    Ok(())
}
pub(crate) async fn success(
    pool: &PgPool,
    binding: &Binding,
    channel: u64,
    revision: &[u8; 32],
) -> sqlx::Result<()> {
    if binding.preserve {
        return Ok(());
    }
    let mut tx = pool.begin().await?;
    lock(&mut tx).await?;
    sqlx::query("DELETE FROM core_channels.affinity_cache WHERE expires_at<=clock_timestamp()")
        .execute(&mut *tx)
        .await?;
    let updated = sqlx::query(
        "UPDATE core_channels.affinity_cache SET expires_at=clock_timestamp()+make_interval(secs=>$4),touched_at=clock_timestamp()
         WHERE key_sha256=$1 AND channel_id=$2 AND binding_sha256=$3",
    ).bind(binding.key.as_slice()).bind(channel as i64).bind(revision.as_slice()).bind(f64::from(binding.ttl)).execute(&mut *tx).await?;
    if updated.rows_affected() == 0 {
        sqlx::query(
            "INSERT INTO core_channels.affinity_cache AS a(key_sha256,rule_name,channel_id,binding_sha256,expires_at,touched_at)
             VALUES($1,$2,$3,$4,clock_timestamp()+make_interval(secs=>$5),clock_timestamp())
             ON CONFLICT(key_sha256) DO UPDATE SET rule_name=excluded.rule_name,
                channel_id=excluded.channel_id,binding_sha256=excluded.binding_sha256,
                expires_at=excluded.expires_at,touched_at=excluded.touched_at
             WHERE $6"
        ).bind(binding.key.as_slice()).bind(&binding.rule).bind(channel as i64).bind(revision.as_slice())
            .bind(f64::from(binding.ttl)).bind(binding.switch_on_success).execute(&mut *tx).await?;
        sqlx::query("DELETE FROM core_channels.affinity_cache WHERE key_sha256 IN
            (SELECT key_sha256 FROM core_channels.affinity_cache ORDER BY touched_at DESC,key_sha256 OFFSET $1)")
            .bind(binding.max_entries as i64).execute(&mut *tx).await?;
    }
    tx.commit().await
}

#[derive(Serialize)]
pub(crate) struct Stats {
    enabled: bool,
    total: i64,
    unknown: usize,
    by_rule_name: BTreeMap<String, i64>,
    cache_capacity: usize,
    cache_algo: &'static str,
}
pub(crate) async fn stats(pool: &PgPool, config: Option<&Config>) -> sqlx::Result<Stats> {
    let counts: Vec<(String,i64)> = sqlx::query_as("SELECT rule_name,count(*) FROM core_channels.affinity_cache WHERE expires_at>clock_timestamp() GROUP BY rule_name")
        .fetch_all(pool).await?;
    Ok(Stats {
        enabled: config.is_some_and(|c| c.enabled),
        total: counts.iter().map(|(_, n)| n).sum(),
        unknown: 0,
        by_rule_name: counts.into_iter().collect(),
        cache_capacity: config.map_or(0, |c| c.max_entries),
        cache_algo: "last_success",
    })
}
pub(crate) async fn clear(c: &mut PgConnection, rule: Option<&str>) -> sqlx::Result<u64> {
    lock(c).await?;
    Ok(sqlx::query(
        "DELETE FROM core_channels.affinity_cache WHERE $1::text IS NULL OR rule_name=$1",
    )
    .bind(rule)
    .execute(c)
    .await?
    .rows_affected())
}

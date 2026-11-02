//! Exact legacy display offsets; these never authorize or change funds.
use super::{Failure, ManagementError};
use crate::history_read::projection::MoneyProjection;
use serde::Serialize;
use sqlx::{PgConnection, Row};

#[derive(Serialize)]
pub(super) struct LegacyOpening {
    kind: String,
    source_channel_id: String,
    raw_used_quota: String,
    credits_per_usd: String,
    recorded_usd: String,
    observed_at: Option<i64>,
    archive_sha256: String,
}

#[derive(Serialize)]
pub(super) struct Lifetime {
    basis: &'static str,
    total_usd: Option<String>,
    exact_usd_numerator: Option<String>,
    exact_usd_denominator: Option<String>,
    pub(super) complete: bool,
    legacy_opening: Option<LegacyOpening>,
}

struct ExactMoney {
    numerator: String,
    denominator: String,
    usd: String,
}

async fn money(
    c: &mut PgConnection,
    native: &str,
    offset_numerator: &str,
    offset_denominator: &str,
) -> Result<ExactMoney, Failure> {
    // Round once, after adding exact fractions. Integer div/mod avoids the
    // precision chosen by PostgreSQL's ordinary numeric division.
    let row = sqlx::query(
        "WITH amount AS (SELECT $1::text::numeric * $3::text::numeric + $2::text::numeric * 500000 AS n, $3::text::numeric * 500000 AS d), scaled AS (SELECT n,d,div(abs(n)*1000000000000000000,d) + CASE WHEN mod(abs(n)*1000000000000000000,d)*2>=d THEN 1 ELSE 0 END AS s FROM amount) SELECT n::text,d::text,s::text,n<0 AS negative FROM scaled",
    )
    .bind(native)
    .bind(offset_numerator)
    .bind(offset_denominator)
    .fetch_one(c)
    .await
    .map_err(ManagementError::from)?;
    let scaled: String = row.try_get("s").map_err(ManagementError::from)?;
    let mut usd = MoneyProjection::scaled(&scaled, "recorded_channel_lifetime")
        .and_then(|money| money.quota_usd)
        .ok_or(Failure::AccountingUnavailable)?;
    if row
        .try_get::<bool, _>("negative")
        .map_err(ManagementError::from)?
        && usd != "0"
    {
        usd.insert(0, '-');
    }
    Ok(ExactMoney {
        numerator: row.try_get("n").map_err(ManagementError::from)?,
        denominator: row.try_get("d").map_err(ManagementError::from)?,
        usd,
    })
}

pub(super) async fn read(
    c: &mut PgConnection,
    id: u64,
    native_net: &str,
    native_complete: bool,
    native_unknown: bool,
    legacy_gap_only: bool,
) -> Result<Lifetime, Failure> {
    let row = sqlx::query("SELECT b.kind,b.source_channel_id::text,b.raw_used_quota::text,b.credits_per_usd::text,b.offset_numerator::text,b.offset_denominator::text,b.historical_coverage,s.observed_at,s.archive_sha256 FROM core_channels.opening_baselines b JOIN core_channels.opening_sources s USING(source_key) WHERE b.channel_id=$1")
        .bind(id as i64).fetch_optional(&mut *c).await.map_err(ManagementError::from)?;
    let (legacy_opening, numerator, denominator, complete) = if let Some(row) = row {
        let kind: String = row.try_get("kind").map_err(ManagementError::from)?;
        let covered: bool = row
            .try_get("historical_coverage")
            .map_err(ManagementError::from)?;
        let raw: String = row
            .try_get("raw_used_quota")
            .map_err(ManagementError::from)?;
        let basis: String = row
            .try_get("credits_per_usd")
            .map_err(ManagementError::from)?;
        let recorded = money(c, "0", &raw, &basis).await?;
        let complete =
            covered && !native_unknown && (native_complete || legacy_gap_only || kind == "copied");
        (
            Some(LegacyOpening {
                kind,
                source_channel_id: row
                    .try_get("source_channel_id")
                    .map_err(ManagementError::from)?,
                raw_used_quota: raw,
                credits_per_usd: basis,
                recorded_usd: recorded.usd,
                observed_at: row.try_get("observed_at").map_err(ManagementError::from)?,
                archive_sha256: row
                    .try_get("archive_sha256")
                    .map_err(ManagementError::from)?,
            }),
            row.try_get::<String, _>("offset_numerator")
                .map_err(ManagementError::from)?,
            row.try_get::<String, _>("offset_denominator")
                .map_err(ManagementError::from)?,
            complete,
        )
    } else {
        (None, "0".into(), "1".into(), native_complete)
    };
    let total = if complete {
        Some(money(c, native_net, &numerator, &denominator).await?)
    } else {
        None
    };
    Ok(Lifetime {
        basis: if legacy_opening.is_some() {
            "legacy_recorded_plus_native"
        } else {
            "native"
        },
        total_usd: total.as_ref().map(|total| total.usd.clone()),
        exact_usd_numerator: total.as_ref().map(|total| total.numerator.clone()),
        exact_usd_denominator: total.map(|total| total.denominator),
        complete,
        legacy_opening,
    })
}

pub(super) async fn copy(
    c: &mut PgConnection,
    source: u64,
    target: u64,
    complete: bool,
) -> Result<(), Failure> {
    sqlx::query("INSERT INTO core_channels.opening_baselines(channel_id,source_key,source_channel_id,raw_used_quota,credits_per_usd,offset_numerator,offset_denominator,historical_coverage,kind,parent_channel_id) SELECT $1,source_key,source_channel_id,raw_used_quota,credits_per_usd,offset_numerator,offset_denominator,historical_coverage AND $3,'copied',$2 FROM core_channels.opening_baselines WHERE channel_id=$2")
        .bind(target as i64).bind(source as i64).bind(complete).execute(c).await.map_err(ManagementError::from)?;
    Ok(())
}

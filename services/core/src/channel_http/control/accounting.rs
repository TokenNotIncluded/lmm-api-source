//! Permanent channel identities and display-only copies. No ledger command is emitted.
use super::*;
use axum::extract::Query;
use serde::{Deserialize, Serialize};
use sqlx::{PgConnection, Row};
use std::time::{SystemTime, UNIX_EPOCH};

pub(super) async fn ensure_identities(
    management: &ManagedChannels,
    c: &mut PgConnection,
) -> Result<(), Failure> {
    let current = management
        .routing
        .catalog_document()
        .map_err(|_| ManagementError::Storage)?;
    let ids: Vec<i64> = management
        .bootstrap
        .routing
        .upstreams
        .iter()
        .chain(current.routing.upstreams.iter())
        .map(|u| u.id as i64)
        .collect::<BTreeSet<_>>()
        .into_iter()
        .collect();
    sqlx::query("INSERT INTO core_channels.identities(id,origin) SELECT id,'bootstrap' FROM unnest($1::bigint[]) id ON CONFLICT(id) DO NOTHING")
        .bind(&ids).execute(&mut *c).await.map_err(ManagementError::from)?;
    sqlx::query("INSERT INTO core_channels.counter_baselines(channel_id,historical_coverage) SELECT id,FALSE FROM unnest($1::bigint[]) id ON CONFLICT(channel_id) DO NOTHING")
        .bind(&ids).execute(&mut *c).await.map_err(ManagementError::from)?;
    sqlx::query("INSERT INTO core_channels.balance_observations(channel_id,kind) SELECT id,'unknown' FROM unnest($1::bigint[]) id ON CONFLICT(channel_id) DO NOTHING")
        .bind(&ids).execute(&mut *c).await.map_err(ManagementError::from)?;
    Ok(())
}
pub(super) async fn maximum_id(c: &mut PgConnection) -> Result<u64, Failure> {
    let id: i64 = sqlx::query_scalar("SELECT COALESCE(max(id),0) FROM core_channels.identities")
        .fetch_one(c)
        .await
        .map_err(ManagementError::from)?;
    Ok(id as u64)
}
async fn unambiguous(c: &mut PgConnection, id: u64) -> Result<(), Failure> {
    usable(c, id).await?;
    let ambiguous: Option<bool> =
        sqlx::query_scalar("SELECT history_ambiguous FROM core_channels.identities WHERE id=$1")
            .bind(id as i64)
            .fetch_optional(c)
            .await
            .map_err(ManagementError::from)?;
    if ambiguous == Some(true) {
        return Err(Failure::AccountingIdentityAmbiguous);
    }
    Ok(())
}
pub(super) async fn usable(c: &mut PgConnection, id: u64) -> Result<(), Failure> {
    let retired: Option<bool> = sqlx::query_scalar(
        "SELECT retired_at IS NOT NULL FROM core_channels.identities WHERE id=$1",
    )
    .bind(id as i64)
    .fetch_optional(c)
    .await
    .map_err(ManagementError::from)?;
    if retired == Some(true) {
        return Err(Failure::RetiredIdentity);
    }
    Ok(())
}

#[derive(Serialize)]
struct Amounts {
    gross_settled_credits: String,
    refunded_credits: String,
    net_used_credits: String,
}
impl Amounts {
    fn new(gross: u128, refunded: u128) -> Result<Self, Failure> {
        let net = gross
            .checked_sub(refunded)
            .ok_or(Failure::AccountingUnavailable)?;
        Ok(Self {
            gross_settled_credits: gross.to_string(),
            refunded_credits: refunded.to_string(),
            net_used_credits: net.to_string(),
        })
    }
}
#[derive(Serialize)]
struct CounterView {
    balance: super::balance::Observation,
    native: crate::usage::ChannelUsage,
    inherited: Option<Amounts>,
    combined: Amounts,
    historical_coverage: bool,
    complete: bool,
    used_quota: Option<String>,
    credits_per_usd: &'static str,
}
fn integer(value: &str) -> Result<u128, Failure> {
    value.parse().map_err(|_| Failure::AccountingUnavailable)
}
async fn view(c: &mut PgConnection, id: u64) -> Result<CounterView, Failure> {
    unambiguous(c, id).await?;
    let native = crate::usage::channel_totals(c, id)
        .await
        .map_err(usage_error)?;
    let baseline=sqlx::query("SELECT gross_credits::text,refunded_credits::text,historical_coverage FROM core_channels.counter_baselines WHERE channel_id=$1")
        .bind(id as i64).fetch_optional(&mut *c).await.map_err(ManagementError::from)?;
    let (inherited, historical_coverage) = if let Some(row) = baseline {
        (
            Some(Amounts::new(
                integer(
                    &row.try_get::<String, _>("gross_credits")
                        .map_err(ManagementError::from)?,
                )?,
                integer(
                    &row.try_get::<String, _>("refunded_credits")
                        .map_err(ManagementError::from)?,
                )?,
            )?),
            row.try_get::<bool, _>("historical_coverage")
                .map_err(ManagementError::from)?,
        )
    } else {
        (None, false)
    };
    let inherited_gross = inherited
        .as_ref()
        .map_or(Ok(0), |v| integer(&v.gross_settled_credits))?;
    let inherited_refunded = inherited
        .as_ref()
        .map_or(Ok(0), |v| integer(&v.refunded_credits))?;
    let gross = integer(&native.gross_settled_credits)?
        .checked_add(inherited_gross)
        .ok_or(Failure::AccountingUnavailable)?;
    let refunded = integer(&native.refunded_credits)?
        .checked_add(inherited_refunded)
        .ok_or(Failure::AccountingUnavailable)?;
    let combined = Amounts::new(gross, refunded)?;
    let complete = historical_coverage && native.unknown_provenance_requests == "0";
    let used_quota = complete.then(|| combined.net_used_credits.clone());
    let balance = super::balance::read_observation(c, id).await?;
    Ok(CounterView {
        balance,
        native,
        inherited,
        combined,
        historical_coverage,
        complete,
        used_quota,
        credits_per_usd: "500000",
    })
}

pub(super) async fn publish(
    c: &mut PgConnection,
    management: &ManagedChannels,
    current: &ReleaseDocument,
    next: &ReleaseDocument,
    change: &PublicationChange,
) -> Result<(), Failure> {
    ensure_identities(management, c).await?;
    let current_ids: BTreeSet<_> = current.routing.upstreams.iter().map(|u| u.id).collect();
    let next_ids: BTreeSet<_> = next.routing.upstreams.iter().map(|u| u.id).collect();
    for id in current_ids.intersection(&next_ids) {
        usable(c, *id).await?;
    }
    for channel_id in next_ids.difference(&current_ids).copied() {
        let copied = matches!(change,PublicationChange::Copy{id,..} if *id==channel_id);
        let inserted:Option<i64>=sqlx::query_scalar("INSERT INTO core_channels.identities(id,origin) VALUES($1,$2) ON CONFLICT(id) DO NOTHING RETURNING id")
            .bind(channel_id as i64).bind(if copied {"copy"} else {"native"}).fetch_optional(&mut *c).await.map_err(ManagementError::from)?;
        if inserted.is_none() {
            return Err(Failure::RetiredIdentity);
        }
        match change {
            PublicationChange::Copy {
                source_id,
                id,
                reset_balance,
            } if *id == channel_id => {
                let (gross, refunded, coverage) = if *reset_balance {
                    ("0".into(), "0".into(), true)
                } else {
                    let source = view(c, *source_id).await?;
                    (
                        source.combined.gross_settled_credits,
                        source.combined.refunded_credits,
                        source.complete,
                    )
                };
                sqlx::query("INSERT INTO core_channels.counter_baselines(channel_id,gross_credits,refunded_credits,historical_coverage,source_channel_id,source_routing_version) VALUES($1,$2::text::numeric,$3::text::numeric,$4,$5,$6)")
                    .bind(*id as i64).bind(gross).bind(refunded).bind(coverage).bind(*source_id as i64).bind(current.routing.version as i64).execute(&mut *c).await.map_err(ManagementError::from)?;
                // Preserve the original observation time even for a reset, as
                // in the legacy contract; the kind prevents a fresh-probe claim.
                sqlx::query("INSERT INTO core_channels.balance_observations(channel_id,balance,unit_label,observed_at,kind,credential_id,credential_revision,source_channel_id) SELECT $1,CASE WHEN $2 THEN 0::double precision ELSE balance END,unit_label,observed_at,CASE WHEN $2 THEN 'reset' ELSE 'copied' END,credential_id,credential_revision,$3 FROM core_channels.balance_observations WHERE channel_id=$3")
                    .bind(*id as i64).bind(*reset_balance).bind(*source_id as i64).execute(&mut *c).await.map_err(ManagementError::from)?;
            }
            _ => {
                sqlx::query("INSERT INTO core_channels.counter_baselines(channel_id,historical_coverage) VALUES($1,TRUE)")
                    .bind(channel_id as i64).execute(&mut *c).await.map_err(ManagementError::from)?;
                sqlx::query("INSERT INTO core_channels.balance_observations(channel_id,kind) VALUES($1,'unknown')")
                    .bind(channel_id as i64).execute(&mut *c).await.map_err(ManagementError::from)?;
            }
        }
    }
    for id in current_ids.difference(&next_ids) {
        sqlx::query("UPDATE core_channels.identities SET retired_at=COALESCE(retired_at,clock_timestamp()) WHERE id=$1")
                .bind(*id as i64).execute(&mut *c).await.map_err(ManagementError::from)?;
    }
    Ok(())
}

#[derive(Default, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub(in crate::channel_http) struct Window {
    start_timestamp: Option<i64>,
    end_timestamp: Option<i64>,
}
pub(in crate::channel_http) async fn usage(
    State(state): State<ChannelState>,
    headers: HeaderMap,
    Path(id): Path<u64>,
    Query(input): Query<Window>,
) -> Result<Json<Value>, Failure> {
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_err(|_| ManagementError::Storage)?
        .as_secs() as i64;
    let end = input.end_timestamp.unwrap_or(now);
    let start = input
        .start_timestamp
        .unwrap_or(end.saturating_sub(31 * 86400).max(0));
    if start < 0 || end < start || end > 253402300799 || end - start > 31 * 86400 {
        return Err(Failure::InvalidUsageWindow);
    }
    super::super::admit(&state, &headers).await?;
    let management = state.management.as_ref().ok_or(Failure::Disabled)?;
    management.synchronize().await?;
    let (mut tx, _) = management
        .begin(
            bearer(&headers)?,
            Action::ChannelRead,
            super::ReadView::Snapshot,
        )
        .await?;
    let document = management
        .routing
        .catalog_document()
        .map_err(|_| ManagementError::Storage)?;
    if !document.routing.upstreams.iter().any(|u| u.id == id) {
        return Err(Failure::NotFound);
    }
    unambiguous(&mut tx, id).await?;
    let result = crate::usage::channel_usage(&mut tx, id, start, end)
        .await
        .map_err(usage_error)?;
    tx.commit().await.map_err(ManagementError::from)?;
    Ok(Json(json!({"success":true,"data":result})))
}
fn usage_error(error: crate::usage::ChannelUsageError) -> Failure {
    match error {
        crate::usage::ChannelUsageError::HistoryAmbiguous => Failure::AccountingIdentityAmbiguous,
        crate::usage::ChannelUsageError::TooManyRequests
        | crate::usage::ChannelUsageError::TooManyReceipts => Failure::AccountingLimit,
        _ => Failure::AccountingUnavailable,
    }
}
pub(in crate::channel_http) async fn counters(
    State(state): State<ChannelState>,
    headers: HeaderMap,
    Path(id): Path<u64>,
) -> Result<Json<Value>, Failure> {
    super::super::admit(&state, &headers).await?;
    let management = state.management.as_ref().ok_or(Failure::Disabled)?;
    management.synchronize().await?;
    let (mut tx, _) = management
        .begin(
            bearer(&headers)?,
            Action::ChannelRead,
            super::ReadView::Snapshot,
        )
        .await?;
    if !management
        .routing
        .catalog_document()
        .map_err(|_| ManagementError::Storage)?
        .routing
        .upstreams
        .iter()
        .any(|u| u.id == id)
    {
        return Err(Failure::NotFound);
    }
    let result = view(&mut tx, id).await?;
    tx.commit().await.map_err(ManagementError::from)?;
    Ok(Json(json!({"success":true,"data":result})))
}

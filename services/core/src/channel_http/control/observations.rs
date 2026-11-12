//! Batched display reads and fenced catalogue observations; no ledger writes.
use super::*;
use num_bigint::BigInt;
use num_rational::BigRational;
use sqlx::{PgConnection, Row};
use std::collections::BTreeMap;

#[cfg(test)]
const SCHEMA: &str = include_str!("../../../schema/channel_observations.sql");

#[derive(Default, Serialize)]
pub(in crate::channel_http) struct Statistics {
    pub lifetime: Lifetime,
    pub observation: Option<Balance>,
    pub catalog_observation: Option<CatalogObservation>,
    pub response_time: Option<i64>,
    pub test_time: Option<i64>,
    #[serde(skip)]
    pub amount: Option<BigRational>,
    #[serde(skip)]
    pub generation: Option<String>,
}
#[derive(Default, Serialize)]
pub(in crate::channel_http) struct Lifetime {
    pub complete: bool,
    pub historical_observed_at: Option<i64>,
    pub total_usd: Option<String>,
    pub exact_usd_numerator: Option<String>,
    pub exact_usd_denominator: Option<String>,
}
#[derive(Serialize)]
pub(in crate::channel_http) struct Balance {
    pub balance: Option<String>,
    pub unit_label: Option<String>,
    pub balance_updated_time: Option<i64>,
    pub recorded_at: Option<i64>,
    pub source_channel_id: Option<i64>,
    pub kind: String,
    #[serde(skip)]
    pub number: Option<f64>,
}
#[derive(Serialize)]
pub(in crate::channel_http) struct CatalogObservation {
    pub scope: &'static str,
    pub inference_tested: bool,
    pub result: String,
    pub observed_at: i64,
    pub duration_ms: i64,
}

fn binding(upstream: &crate::routing::UpstreamConfig) -> Result<Vec<u8>, Failure> {
    Ok(
        Sha256::digest(serde_json::to_vec(upstream).map_err(|_| ManagementError::Invalid)?)
            .to_vec(),
    )
}

fn exact(n: &str, d: &str) -> Result<BigRational, Failure> {
    let n = n
        .parse::<BigInt>()
        .map_err(|_| Failure::AccountingUnavailable)?;
    let d = d
        .parse::<BigInt>()
        .map_err(|_| Failure::AccountingUnavailable)?;
    if d <= BigInt::from(0) {
        return Err(Failure::AccountingUnavailable);
    }
    Ok(BigRational::new(n, d))
}
fn display(amount: &BigRational) -> String {
    let zero = BigInt::from(0);
    let negative = amount.numer() < &zero;
    let n = if negative {
        -amount.numer()
    } else {
        amount.numer().clone()
    };
    let scale = BigInt::from(10_u64).pow(18);
    let scaled = n * &scale;
    let rounded = &scaled / amount.denom()
        + if (&scaled % amount.denom()) * 2 >= *amount.denom() {
            BigInt::from(1)
        } else {
            zero
        };
    let integer = &rounded / &scale;
    let fraction = format!("{:0>18}", (&rounded % &scale).to_string());
    let fraction = fraction.trim_end_matches('0');
    let sign = if negative && rounded != BigInt::from(0) {
        "-"
    } else {
        ""
    };
    if fraction.is_empty() {
        format!("{sign}{integer}")
    } else {
        format!("{sign}{integer}.{fraction}")
    }
}

pub(in crate::channel_http) async fn batch(
    c: &mut PgConnection,
    routing: &crate::routing::RoutingConfig,
) -> Result<BTreeMap<u64, Statistics>, Failure> {
    // Same guards as channel_totals. A dirty/unready projection is not zero.
    let ready: Option<bool> =
        sqlx::query_scalar("SELECT ready FROM core_channels.projection_state WHERE singleton")
            .fetch_optional(&mut *c)
            .await
            .map_err(ManagementError::from)?;
    let dirty: bool =
        sqlx::query_scalar("SELECT EXISTS(SELECT FROM core_channels.projection_dirty)")
            .fetch_one(&mut *c)
            .await
            .map_err(ManagementError::from)?;
    if ready != Some(true) || dirty {
        return Err(Failure::AccountingUnavailable);
    }
    let ids: Vec<i64> = routing.upstreams.iter().map(|u| u.id as i64).collect();
    let keys: Vec<i64> = routing
        .upstreams
        .iter()
        .map(|u| u.credential.id as i64)
        .collect();
    let revisions: Vec<i64> = routing
        .upstreams
        .iter()
        .map(|u| u.credential.revision as i64)
        .collect();
    let bindings: Vec<Vec<u8>> = routing
        .upstreams
        .iter()
        .map(binding)
        .collect::<Result<_, _>>()?;
    let rows = sqlx::query(include_str!("observations/read.sql"))
        .bind(ids)
        .bind(keys)
        .bind(revisions)
        .bind(bindings)
        .fetch_all(c)
        .await
        .map_err(ManagementError::from)?;
    let mut result = BTreeMap::new();
    for row in rows {
        let mut stats = Statistics::default();
        stats.generation = row.try_get("generation").map_err(ManagementError::from)?;
        stats.lifetime.historical_observed_at = row
            .try_get("historical_observed_at")
            .map_err(ManagementError::from)?;
        stats.lifetime.complete = row.try_get("complete").map_err(ManagementError::from)?;
        if stats.lifetime.complete {
            let n: String = row.try_get("numerator").map_err(ManagementError::from)?;
            let d: String = row.try_get("denominator").map_err(ManagementError::from)?;
            let amount = exact(&n, &d)?;
            stats.lifetime.total_usd = Some(display(&amount));
            stats.lifetime.exact_usd_numerator = Some(n);
            stats.lifetime.exact_usd_denominator = Some(d);
            stats.amount = Some(amount);
        }
        if let Some(kind) = row
            .try_get::<Option<String>, _>("balance_kind")
            .map_err(ManagementError::from)?
        {
            let number: Option<f64> = row.try_get("balance").map_err(ManagementError::from)?;
            if number.is_some_and(|n| !n.is_finite()) {
                return Err(Failure::AccountingUnavailable);
            }
            stats.observation = Some(Balance {
                balance: number.map(|n| n.to_string()),
                number,
                kind,
                unit_label: row
                    .try_get::<Option<String>, _>("unit_label")
                    .map_err(ManagementError::from)?
                    .filter(|unit| {
                        !unit.is_empty()
                            && unit.trim() == unit
                            && unit.chars().count() <= 32
                            && !unit.chars().any(char::is_control)
                    }),
                balance_updated_time: row
                    .try_get("balance_updated_time")
                    .map_err(ManagementError::from)?,
                recorded_at: row
                    .try_get("balance_recorded_at")
                    .map_err(ManagementError::from)?,
                source_channel_id: row
                    .try_get("balance_source_channel_id")
                    .map_err(ManagementError::from)?,
            });
        }
        if let Some(result) = row
            .try_get::<Option<String>, _>("probe_result")
            .map_err(ManagementError::from)?
        {
            let duration_ms = row.try_get("duration_ms").map_err(ManagementError::from)?;
            let observed_at = row.try_get("test_time").map_err(ManagementError::from)?;
            stats.response_time = (result == "success").then_some(duration_ms);
            stats.test_time = Some(observed_at);
            stats.catalog_observation = Some(CatalogObservation {
                scope: "model_catalog",
                inference_tested: false,
                result,
                duration_ms,
                observed_at,
            });
        }
        let id: i64 = row.try_get("id").map_err(ManagementError::from)?;
        result.insert(id as u64, stats);
    }
    Ok(result)
}

async fn snapshot(
    c: &mut PgConnection,
    management: &ManagedChannels,
) -> Result<(ReleaseDocument, Bindings), Failure> {
    let value: Option<Value> = sqlx::query_scalar("SELECT public.routing_control_read()")
        .fetch_one(&mut *c)
        .await
        .map_err(ManagementError::from)?;
    let Some(value) = value else {
        return Ok((management.bootstrap.clone(), Bindings::default()));
    };
    let document: ReleaseDocument =
        serde_json::from_value(value).map_err(|_| ManagementError::Storage)?;
    let value: Option<Value> = sqlx::query_scalar("SELECT public.routing_control_bindings($1)")
        .bind(document.routing.version as i64)
        .fetch_one(c)
        .await
        .map_err(ManagementError::from)?;
    let bindings = value
        .map(serde_json::from_value)
        .transpose()
        .map_err(|_| ManagementError::Storage)?
        .unwrap_or_default();
    Ok((document, bindings))
}

fn digest(
    document: &ReleaseDocument,
    bindings: &Bindings,
    stats: &BTreeMap<u64, Statistics>,
    params: &crate::channel_http::Params,
) -> Result<String, Failure> {
    let selection = json!({
        "page_size":params.page_size.unwrap_or(10), "keyword":params.keyword, "group":params.group,
        "model":params.model, "status":params.status, "type":params.channel_type,
        "sort_by":params.sort_by.as_deref().unwrap_or("id"), "sort_order":params.sort_order,
        "id_sort":params.id_sort, "tag_mode":params.tag_mode, "sort_unit":params.sort_unit,
    });
    let generations: Vec<_> = stats
        .iter()
        .map(|(id, value)| (id, &value.generation))
        .collect();
    let bytes = serde_json::to_vec(&(
        document,
        &bindings.names,
        &bindings.tags,
        stats,
        generations,
        selection,
    ))
    .map_err(|_| ManagementError::Storage)?;
    Ok(format!("{:x}", Sha256::digest(bytes)))
}

pub(in crate::channel_http) async fn list(
    state: &ChannelState,
    headers: &HeaderMap,
    params: crate::channel_http::Params,
) -> Result<Value, Failure> {
    let management = state.management.as_ref().ok_or(Failure::Disabled)?;
    let (mut tx, _) = management
        .begin(bearer(headers)?, Action::ChannelRead, ReadView::Snapshot)
        .await?;
    let (document, bindings) = snapshot(&mut tx, management).await?;
    let stats = batch(&mut tx, &document.routing).await?;
    let units: Vec<String> = stats
        .values()
        .filter_map(|s| s.observation.as_ref())
        .filter(|b| b.number.is_some() && b.kind != "unknown")
        .filter_map(|b| b.unit_label.clone())
        .collect::<BTreeSet<_>>()
        .into_iter()
        .collect();
    if params.sort_by.as_deref() == Some("balance")
        && params
            .sort_unit
            .as_deref()
            .is_none_or(|u| u.trim().is_empty())
    {
        return Err(Failure::BalanceUnitRequired(units));
    }
    let snapshot_digest = digest(&document, &bindings, &stats, &params)?;
    if params
        .snapshot_digest
        .as_ref()
        .is_some_and(|d| d != &snapshot_digest)
    {
        return Err(Failure::SnapshotChanged);
    }
    let checked = params.snapshot_digest.is_some();
    let snapshot_at: i64 =
        sqlx::query_scalar("SELECT floor(extract(epoch FROM transaction_timestamp()))::bigint")
            .fetch_one(&mut *tx)
            .await
            .map_err(ManagementError::from)?;
    let mut value = crate::channel_http::page_with_statistics(
        &document.routing,
        params,
        &bindings.names,
        &bindings.tags,
        stats,
    )
    .map_err(|_| ManagementError::Invalid)?;
    value["data"]["snapshot_digest"] = json!(snapshot_digest);
    value["data"]["pagination_consistency"] = json!(if checked {
        "digest_checked"
    } else {
        "single_request"
    });
    value["data"]["statistics_snapshot_at"] = json!(snapshot_at);
    value["data"]["balance_units"] = json!(units);
    value["data"]["pricing_version"] = json!(document.pricing.version);
    tx.commit().await.map_err(ManagementError::from)?;
    Ok(value)
}

pub(super) async fn probe(
    state: &ChannelState,
    headers: &HeaderMap,
    id: u64,
) -> Result<crate::model_http::channel_operations::ChannelProbe, Failure> {
    let management = state.management.as_ref().ok_or(Failure::Disabled)?;
    super::super::admit(state, headers).await?;
    let gateway = management.gateway.as_ref().ok_or(Failure::Disabled)?;
    let (mut tx, _) = management
        .begin(bearer(headers)?, Action::ChannelOperate, ReadView::Snapshot)
        .await?;
    let (before, _) = snapshot(&mut tx, management).await?;
    let upstream = before
        .routing
        .upstreams
        .iter()
        .find(|u| u.id == id)
        .ok_or(Failure::NotFound)?
        .clone();
    let started_at: chrono::DateTime<chrono::Utc> = sqlx::query_scalar("SELECT clock_timestamp()")
        .fetch_one(&mut *tx)
        .await
        .map_err(ManagementError::from)?;
    tx.commit().await.map_err(ManagementError::from)?;
    let timer = std::time::Instant::now();
    let outcome = gateway.probe_channel_snapshot(&upstream).await;
    let duration_ms = outcome.as_ref().map_or_else(
        |_| timer.elapsed().as_millis().min(9_007_199_254_740_991) as i64,
        |probe| probe.response_time_ms.min(9_007_199_254_740_991) as i64,
    );
    // No network I/O while holding publication/identity locks.
    let _publication = management.publication.lock().await;
    let (mut tx, principal) = management
        .begin(bearer(headers)?, Action::ChannelOperate, ReadView::Current)
        .await?;
    sqlx::query("SELECT public.routing_control_balance_lock()")
        .execute(&mut *tx)
        .await
        .map_err(ManagementError::from)?;
    let (after, _) = snapshot(&mut tx, management).await?;
    if Version::of(&before) != Version::of(&after)
        || after.routing.upstreams.iter().find(|u| u.id == id) != Some(&upstream)
    {
        return Err(ManagementError::Conflict.into());
    }
    sqlx::query("INSERT INTO core_channels.identities(id,origin) VALUES($1,'bootstrap') ON CONFLICT(id) DO NOTHING")
        .bind(id as i64).execute(&mut *tx).await.map_err(ManagementError::from)?;
    super::accounting::usable(&mut tx, id).await?;
    record(&mut tx, &upstream, started_at, outcome.is_ok(), duration_ms).await?;
    sqlx::query("INSERT INTO core_identity.audit(actor_user_id,credential_id,action,target) VALUES($1,$2,'channel.catalog.observe',$3)")
        .bind(principal.user_id).bind(principal.credential_id)
        .bind(json!({"id":id,"scope":"model_catalog","success":outcome.is_ok()}))
        .execute(&mut *tx).await.map_err(ManagementError::from)?;
    tx.commit()
        .await
        .map_err(|_| ManagementError::CommitUnknown)?;
    outcome.map_err(|_| ManagementError::Storage.into())
}

async fn record(
    c: &mut PgConnection,
    upstream: &crate::routing::UpstreamConfig,
    started_at: chrono::DateTime<chrono::Utc>,
    success: bool,
    duration_ms: i64,
) -> Result<(), Failure> {
    sqlx::query("INSERT INTO core_channels.catalog_observations(channel_id,credential_id,credential_revision,binding_sha256,scope,result,started_at,observed_at,duration_ms) VALUES($1,$2,$3,$7,'model_catalog',$4,$5,clock_timestamp(),$6) ON CONFLICT(channel_id) DO UPDATE SET credential_id=EXCLUDED.credential_id,credential_revision=EXCLUDED.credential_revision,binding_sha256=EXCLUDED.binding_sha256,scope=EXCLUDED.scope,result=EXCLUDED.result,started_at=EXCLUDED.started_at,observed_at=EXCLUDED.observed_at,duration_ms=EXCLUDED.duration_ms WHERE core_channels.catalog_observations.started_at<=EXCLUDED.started_at")
        .bind(upstream.id as i64).bind(upstream.credential.id as i64).bind(upstream.credential.revision as i64)
        .bind(if success { "success" } else { "failed" }).bind(started_at).bind(duration_ms).bind(binding(upstream)?)
        .execute(c).await.map_err(ManagementError::from)?;
    Ok(())
}

#[cfg(test)]
mod tests;

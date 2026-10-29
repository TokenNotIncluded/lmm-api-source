//! Measured relay observations, separate from financial and recovery authority.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use sqlx::{PgPool, Row};
use tokio_stream::StreamExt;

use super::{HistoryError, HistoryStore, Result, personal_session};
use crate::relay::{Completion, Outcome, Settlement};

const EVENT: &str = "performance-v1";

#[derive(Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct Sample {
    schema: String,
    finished_at: u64,
    elapsed_us: u64,
    first_output_us: Option<u64>,
    output_tokens: Option<u64>,
    success: bool,
}

/// Best-effort telemetry never retries dispatch, changes money or delays the
/// existing operation cleanup. Missing observations remain visible on reads.
pub(crate) async fn record(pool: &PgPool, completion: &Completion) {
    let report = &completion.report;
    let (Some(finished_at), Some(elapsed_us)) =
        (report.timings.finished_at, report.timings.elapsed_us)
    else {
        return;
    };
    let sample = Sample {
        schema: "core.performance.v1".into(),
        finished_at,
        elapsed_us,
        first_output_us: report.timings.first_output_us,
        output_tokens: report.usage.output_tokens,
        success: report.outcome == Outcome::Completed
            && report.provider_complete
            && completion.settlement == Settlement::Confirmed
            && completion.error.is_none(),
    };
    let write = async {
        let mut tx = pool.begin().await?;
        sqlx::query("SET TRANSACTION ISOLATION LEVEL SERIALIZABLE")
            .execute(&mut *tx)
            .await?;
        sqlx::query(
            "INSERT INTO core_charging.usage_events(request_id,event_id,detail)
             SELECT $1,$2,$3 WHERE EXISTS (SELECT FROM core_charging.requests WHERE id=$1)
             ON CONFLICT (request_id,event_id) DO NOTHING",
        )
        .bind(&report.request_id)
        .bind(EVENT)
        .bind(json!(sample))
        .execute(&mut *tx)
        .await?;
        tx.commit().await?;
        Ok::<(), sqlx::Error>(())
    };
    if !matches!(
        tokio::time::timeout(std::time::Duration::from_secs(2), write).await,
        Ok(Ok(_))
    ) {
        eprintln!("performance observation was not confirmed; no dispatch or settlement replay");
    }
}

#[derive(Debug, Default, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Query {
    pub hours: Option<u32>,
    pub bucket_seconds: Option<u32>,
    pub model: Option<String>,
}

impl Query {
    fn validate(&self, detail: bool) -> Result<(i64, u64)> {
        let hours = self.hours.unwrap_or(24);
        let bucket = self.bucket_seconds.unwrap_or(3600);
        if !(1..=720).contains(&hours)
            || !matches!(bucket, 60 | 300 | 3600)
            || (detail && self.model.is_none())
            || self
                .model
                .as_ref()
                .is_some_and(|v| v.is_empty() || v.len() > 256 || v.chars().any(char::is_control))
        {
            return Err(HistoryError::Invalid);
        }
        Ok((i64::from(hours) * 3600, u64::from(bucket)))
    }
}

#[derive(Default)]
struct Aggregate {
    requests: u64,
    successes: u64,
    elapsed_us: u128,
    first_output_us: u128,
    first_outputs: u64,
    output_tokens: u128,
    throughput_us: u128,
    throughput_samples: u64,
}

impl Aggregate {
    fn add(&mut self, sample: &Sample) {
        self.requests += 1;
        self.successes += u64::from(sample.success);
        self.elapsed_us += u128::from(sample.elapsed_us);
        if let Some(first) = sample.first_output_us {
            self.first_output_us += u128::from(first);
            self.first_outputs += 1;
        }
        if sample.success
            && sample.elapsed_us > 0
            && let Some(tokens) = sample.output_tokens
        {
            self.output_tokens += u128::from(tokens);
            self.throughput_us += u128::from(sample.elapsed_us);
            self.throughput_samples += 1;
        }
    }

    fn json(&self) -> Value {
        json!({
            "request_count": self.requests,
            "success_count": self.successes,
            "avg_latency_ms": (self.requests > 0).then(|| self.elapsed_us as f64 / self.requests as f64 / 1000.0),
            "avg_ttft_ms": (self.first_outputs > 0).then(|| self.first_output_us as f64 / self.first_outputs as f64 / 1000.0),
            "success_rate": (self.requests > 0).then(|| self.successes as f64 / self.requests as f64 * 100.0),
            "avg_tps": (self.throughput_us > 0).then(|| self.output_tokens as f64 / self.throughput_us as f64 * 1_000_000.0),
            "latency_sum_us": self.elapsed_us.to_string(),
            "ttft_sum_us": self.first_output_us.to_string(),
            "ttft_count": self.first_outputs,
            "output_tokens": self.output_tokens.to_string(),
            "throughput_time_us": self.throughput_us.to_string(),
            "throughput_count": self.throughput_samples,
        })
    }
}

impl HistoryStore {
    /// Authorization, current group/model visibility and samples share one
    /// read-only repeatable-read snapshot. Original frozen prices supply labels.
    pub(crate) async fn performance(
        &self,
        secret: &str,
        query: &Query,
        detail: bool,
    ) -> Result<Value> {
        let (window, bucket) = query.validate(detail)?;
        let mut tx = self.read_snapshot().await?;
        let user = personal_session(&mut tx, secret).await?;
        let visible = crate::abuse::public_models_in_transaction(&mut tx, user)
            .await
            .map_err(|_| HistoryError::Unavailable)?;
        // End is the next UTC second; the half-open window includes all samples
        // already committed in the snapshot, including the current second.
        let end: i64 =
            sqlx::query_scalar("SELECT floor(extract(epoch FROM clock_timestamp()))::bigint + 1")
                .fetch_one(&mut *tx)
                .await?;
        let start = end - window;
        let mut rows = sqlx::query(
            "SELECT r.price->>'model' AS model,r.price->>'group' AS group_name,e.detail
             FROM core_charging.requests r
             LEFT JOIN core_charging.usage_events e
               ON e.request_id=r.id AND e.event_id=$1
             WHERE EXISTS (SELECT FROM core_identity.user_routing_groups g
               WHERE g.user_id=$2 AND g.enabled AND g.group_name=r.price->>'group')
             AND ($3='' OR r.price->>'model'=$3)
             AND (e.created_at>=to_timestamp($4) OR (e.request_id IS NULL AND r.authorized_at >= $4))
             ORDER BY r.id",
        ).bind(EVENT).bind(user).bind(query.model.as_deref().unwrap_or(""))
            .bind(start as f64).fetch(&mut *tx);
        let mut models: BTreeMap<String, Aggregate> = BTreeMap::new();
        let mut groups: BTreeMap<String, Aggregate> = BTreeMap::new();
        let mut model_buckets: BTreeMap<(String, u64), Aggregate> = BTreeMap::new();
        let mut group_buckets: BTreeMap<(String, u64), Aggregate> = BTreeMap::new();
        let mut total = Aggregate::default();
        let mut missing = 0u64;
        while let Some(row) = rows.next().await {
            let row = row?;
            let model: String = row.try_get("model")?;
            if visible.as_ref().is_some_and(|v| !v.contains(&model)) {
                continue;
            }
            let group: String = row.try_get("group_name")?;
            let value: Option<Value> = row.try_get("detail")?;
            let Some(value) = value else {
                missing += 1;
                continue;
            };
            let sample: Sample =
                serde_json::from_value(value).map_err(|_| HistoryError::Unavailable)?;
            if sample.schema != "core.performance.v1"
                || sample.finished_at > 253_402_300_799
                || sample.elapsed_us > 86_400_000_000
                || sample
                    .first_output_us
                    .is_some_and(|t| t > sample.elapsed_us)
            {
                return Err(HistoryError::Unavailable);
            }
            if sample.finished_at < start as u64 || sample.finished_at >= end as u64 {
                continue;
            }
            let at = sample.finished_at / bucket * bucket;
            total.add(&sample);
            models.entry(model.clone()).or_default().add(&sample);
            groups.entry(group.clone()).or_default().add(&sample);
            model_buckets.entry((model, at)).or_default().add(&sample);
            group_buckets.entry((group, at)).or_default().add(&sample);
        }
        drop(rows);
        tx.commit().await?;
        let data = if detail {
            let groups: Vec<Value> = groups
                .into_iter()
                .map(|(name, stats)| {
                    let mut value = stats.json();
                    let series: Vec<Value> = group_buckets
                        .iter()
                        .filter(|((g, _), _)| g == &name)
                        .map(|((_, ts), stats)| {
                            let mut v = stats.json();
                            v["ts"] = json!(ts);
                            v
                        })
                        .collect();
                    value["group"] = json!(name);
                    value["series"] = json!(series);
                    value
                })
                .collect();
            json!({"model_name":query.model,"series_schema":"core.performance.v1","groups":groups,"aggregate":total.json()})
        } else {
            let models: Vec<Value> = models
                .into_iter()
                .map(|(name, stats)| {
                    let mut value = stats.json();
                    let mut recent: Vec<Value> = model_buckets
                        .iter()
                        .rev()
                        .filter(|((m, _), _)| m == &name)
                        .take(3)
                        .map(|(_, s)| s.json()["success_rate"].clone())
                        .collect();
                    recent.reverse();
                    value["model_name"] = json!(name);
                    value["recent_success_rates"] = json!(recent);
                    value
                })
                .collect();
            json!({"models":models,"aggregate":total.json()})
        };
        Ok(
            json!({"success":true,"data":data,"availability":"partial","coverage":{
                "native_shared_relay":true,"historical_imports":false,
                "missing_samples":missing,"sampled_requests":total.requests,
                "window_start":start,"window_end_exclusive":end,"bucket_seconds":bucket,
                "latency":"relay_elapsed_before_finalization","ttft":"first_decoded_stream_content",
                "tps":"successful_output_tokens_per_relay_second","success":"provider_complete_and_confirmed_completion",
                "counts":"accepted_logical_requests_with_samples","unreserved_http_requests":false
            }}),
        )
    }
}

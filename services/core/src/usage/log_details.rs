//! Read-only diagnostic facts. These never change the validated money receipt.
use super::{Error, ValidatedRequest, add, channel_projection};
use crate::{billing::charging::contract, history_read::performance::Sample};
use serde_json::{Value, json};
use sqlx::{PgConnection, Row};
use std::collections::{BTreeMap, HashMap};

#[derive(Default)]
pub(super) struct LogDetails {
    pub(super) sample: Option<Sample>,
    pub(super) username: Option<String>,
    pub(super) channel: Option<i64>,
    pub(super) attempts: Vec<Value>,
}

impl LogDetails {
    pub(super) fn duration_ms(&self) -> Option<f64> {
        self.sample
            .as_ref()
            .map(|sample| sample.elapsed_us as f64 / 1000.0)
    }

    pub(super) fn first_token_ms(&self) -> Option<f64> {
        self.sample
            .as_ref()
            .and_then(|sample| sample.first_output_us)
            .map(|us| us as f64 / 1000.0)
    }

    pub(super) fn upstream_request_id(&self) -> Option<&str> {
        self.sample
            .as_ref()
            .and_then(|sample| sample.upstream_request_id.as_deref())
    }

    pub(super) fn project(&self, request: &ValidatedRequest, administrator: bool) -> Value {
        let mut item = request.log();
        let sample = self.sample.as_ref();
        item["diagnostics_available"] = json!(sample.is_some());
        item["duration_ms"] = json!(self.duration_ms());
        item["first_token_ms"] = json!(self.first_token_ms());
        item["use_time"] = json!(self.duration_ms().map(|ms| ms / 1000.0));
        item["is_stream"] = json!(sample.and_then(|sample| sample.is_stream));
        item["upstream_request_id"] = json!(self.upstream_request_id());
        item["status_code"] = json!(
            sample
                .and_then(|sample| sample.status_code)
                .or_else(|| request.provider_status())
        );
        item["price_version"] = json!(request.price.version);
        let mut other: Value = serde_json::from_str(item["other"].as_str().unwrap()).unwrap();
        other["frt"] = json!(self.first_token_ms());
        other["duration_ms"] = json!(self.duration_ms());
        other["status_code"] = item["status_code"].clone();
        other["price_version"] = item["price_version"].clone();
        // A route's URL, credential reference, wire digest and raw provider
        // response never enter either DTO. Ordinary users receive no route IDs.
        if administrator {
            item["username"] = json!(self.username);
            item["channel"] = json!(self.channel);
            item["channel_name"] = Value::Null;
            item["route_attempts"] = json!(self.attempts);
            other["admin_info"] = json!({"route_attempts":self.attempts});
        }
        item["other"] = json!(other.to_string());
        item
    }
}

pub(super) async fn load(
    connection: &mut PgConnection,
    requests: &[ValidatedRequest],
    administrator: bool,
) -> Result<HashMap<String, LogDetails>, Error> {
    if requests.is_empty() {
        return Ok(HashMap::new());
    }
    let ids: Vec<_> = requests.iter().map(|request| request.id.as_str()).collect();
    let by_id: HashMap<_, _> = requests
        .iter()
        .map(|request| (request.id.as_str(), request))
        .collect();
    let rows = sqlx::query(
        "SELECT r.id,r.fingerprint,r.spec_digest,p.username,performance.detail AS performance,
         contract.detail AS contract,admission.detail AS admission
         FROM core_charging.requests r
         LEFT JOIN core_identity.user_profiles p ON p.user_id=r.actor_user_id
         LEFT JOIN core_charging.usage_events performance ON performance.request_id=r.id AND performance.event_id='performance-v1'
         LEFT JOIN core_charging.usage_events contract ON contract.request_id=r.id AND contract.event_id='reservation-contract:v1'
         LEFT JOIN core_charging.usage_events admission ON admission.request_id=r.id AND admission.event_id='reservation-admission:v1'
         WHERE r.id=ANY($1)",
    ).bind(&ids).fetch_all(&mut *connection).await?;
    let mut attempts: HashMap<String, Vec<channel_projection::AttemptEvent>> = HashMap::new();
    if administrator {
        for row in sqlx::query("SELECT request_id,event_id,detail FROM core_charging.usage_events WHERE request_id=ANY($1) AND (event_id LIKE 'route-dispatch:%' OR event_id LIKE 'route-failure:%') ORDER BY request_id,event_id")
            .bind(&ids).fetch_all(&mut *connection).await?
        {
            attempts.entry(row.try_get("request_id")?).or_default().push(channel_projection::AttemptEvent {
                event_id: row.try_get("event_id")?,
                detail: row.try_get("detail")?,
            });
        }
    }
    let mut output = HashMap::new();
    for row in rows {
        let id: String = row.try_get("id")?;
        let request = by_id.get(id.as_str()).ok_or(Error::Storage)?;
        // Optional telemetry has no financial authority. A malformed or absent
        // observation stays unknown without hiding a valid settlement receipt.
        let sample = row
            .try_get::<Option<Value>, _>("performance")?
            .and_then(|value| serde_json::from_value::<Sample>(value).ok())
            .filter(Sample::valid);
        let mut details = LogDetails {
            sample,
            username: administrator
                .then(|| row.try_get("username"))
                .transpose()?
                .flatten(),
            ..Default::default()
        };
        if administrator {
            let proof = contract::read_attribution(
                row.try_get("contract")?,
                row.try_get("admission")?,
                &request.price,
                &row.try_get::<Vec<u8>, _>("fingerprint")?,
                &row.try_get::<Vec<u8>, _>("spec_digest")?,
            )
            .ok()
            .flatten();
            let events = attempts.remove(&id).unwrap_or_default();
            if let Some(image) = &request.price.image {
                details.channel = Some(
                    image
                        .clone()
                        .restore()
                        .map_err(|_| Error::Storage)?
                        .binding()
                        .upstream_id as i64,
                );
            } else if let Some(proof) = &proof {
                details.channel =
                    channel_projection::final_channel(proof, &events).map(|id| id as i64);
                let failures: BTreeMap<_, _> = events
                    .iter()
                    .filter(|event| event.event_id.starts_with("route-failure:"))
                    .filter_map(|event| {
                        let number = event.detail["number"].as_u64()?;
                        let failure = serde_json::from_value::<crate::relay::AttemptFailure>(
                            event.detail["failure"].clone(),
                        )
                        .ok()?;
                        Some((number, failure))
                    })
                    .collect();
                for event in events
                    .iter()
                    .filter(|event| event.event_id.starts_with("route-dispatch:"))
                {
                    let Some(number) = event.detail["number"]
                        .as_u64()
                        .filter(|number| (1..=8).contains(number))
                    else {
                        continue;
                    };
                    let (Some(route), Some(wire)) = (
                        event.detail["route_id"].as_str(),
                        event.detail["wire_sha256"].as_str(),
                    ) else {
                        continue;
                    };
                    let Some(channel) = proof.dispatched_channel(route, wire) else {
                        continue;
                    };
                    let failure = failures.get(&number);
                    details.attempts.push(json!({
                        "attempt":number,"channel_id":channel,"route_id":route,
                        "status_code":failure.and_then(|failure| failure.http_status),
                        "error_kind":failure.map(|failure| failure.failure.kind),
                        "delivery":failure.map(|failure| failure.failure.delivery),
                    }));
                }
                details
                    .attempts
                    .sort_by_key(|attempt| attempt["attempt"].as_u64().unwrap());
            }
        }
        if output.insert(id, details).is_some() {
            return Err(Error::Storage);
        }
    }
    Ok(output)
}

// Exact nearest-rank P95 for bounded windows. Beyond the limit it is explicitly
// unavailable; counts and the mean still cover every matching observation.
const MAX_PERCENTILE_SAMPLES: usize = 100_000;

#[derive(Default)]
pub(super) struct PerformanceSummary {
    count: i64,
    total_us: u128,
    samples: Vec<u64>,
    percentile_unavailable: bool,
}

impl PerformanceSummary {
    pub(super) fn include(&mut self, details: &LogDetails) -> Result<(), Error> {
        let Some(sample) = &details.sample else {
            return Ok(());
        };
        add(&mut self.count, 1)?;
        self.total_us += u128::from(sample.elapsed_us);
        if !self.percentile_unavailable {
            if self.samples.len() < MAX_PERCENTILE_SAMPLES {
                self.samples.push(sample.elapsed_us);
            } else {
                self.percentile_unavailable = true;
                self.samples.clear();
                self.samples.shrink_to_fit();
            }
        }
        Ok(())
    }

    pub(super) fn project(mut self, result: &mut Value) {
        let p95 = if self.samples.is_empty() || self.percentile_unavailable {
            None
        } else {
            let index = (self.samples.len() * 95).div_ceil(100) - 1;
            let (_, value, _) = self.samples.select_nth_unstable(index);
            Some(*value as f64 / 1000.0)
        };
        result["duration_sample_count"] = json!(self.count);
        result["average_duration_ms"] =
            json!((self.count > 0).then(|| self.total_us as f64 / self.count as f64 / 1000.0));
        result["p95_duration_ms"] = json!(p95);
        result["duration_percentile_scope"] = json!(if self.percentile_unavailable {
            "unavailable"
        } else {
            "all_samples"
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn percentile_limit_never_turns_a_prefix_into_a_full_window_statistic() {
        let details = LogDetails {
            sample: Some(
                serde_json::from_value(json!({
                    "schema":"core.performance.v1","finished_at":1700000000,
                    "elapsed_us":1234567,"first_output_us":null,"output_tokens":20,"success":true
                }))
                .unwrap(),
            ),
            ..Default::default()
        };
        let mut summary = PerformanceSummary::default();
        for _ in 0..=MAX_PERCENTILE_SAMPLES {
            summary.include(&details).unwrap();
        }
        let mut value = json!({});
        summary.project(&mut value);
        assert_eq!(value["duration_sample_count"], MAX_PERCENTILE_SAMPLES + 1);
        assert_eq!(value["average_duration_ms"], 1234.567);
        assert!(value["p95_duration_ms"].is_null());
        assert_eq!(value["duration_percentile_scope"], "unavailable");
    }
}

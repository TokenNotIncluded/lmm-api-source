//! Offline correction for completed inclusive text requests whose recorded
//! output exceeded their original allowance or whose write counter was absent.
use super::*;
use serde::Deserialize;

pub(crate) const EVENT: &str = "operator-model-reconcile:v1";
const OBSERVATION: &str = "provider-observation:terminal";
const MAX_BATCH: usize = 500;

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct OperatorReconcileRequest {
    pub request_ids: Vec<String>,
    pub route_id: String,
    pub reason: String,
    /// Omit for preview. To apply, echo the exact preview digest.
    pub confirmation_digest: Option<String>,
}

#[derive(Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct Entry {
    request_id: String,
    revision: i64,
    reserved: i64,
    amount: i64,
    supplement: i64,
    release: i64,
    observed_output: u64,
    billable: Usage,
    observation_sha256: String,
    reservation_sha256: String,
}

impl Charging {
    pub async fn operator_reconcile(&self, mut request: OperatorReconcileRequest) -> Result<Value> {
        if !(1..=MAX_BATCH).contains(&request.request_ids.len())
            || request.request_ids.iter().any(|id| {
                id.len() != 64
                    || !id
                        .bytes()
                        .all(|b| b.is_ascii_hexdigit() && !b.is_ascii_uppercase())
            })
            || request.route_id.is_empty()
            || request.route_id.len() > 128
            || request.route_id.chars().any(char::is_control)
            || request.reason.trim().is_empty()
            || request.reason.len() > 512
            || request.reason.chars().any(char::is_control)
        {
            return Err(Error::Invalid);
        }
        request.request_ids.sort_unstable();
        if request
            .request_ids
            .windows(2)
            .any(|pair| pair[0] == pair[1])
        {
            return Err(Error::Invalid);
        }

        let mut tx = self.begin().await?;
        if request.confirmation_digest.is_none() {
            sqlx::query("SET TRANSACTION READ ONLY")
                .execute(&mut *tx)
                .await?;
        }
        let mut entries = Vec::with_capacity(request.request_ids.len());
        let mut charges = Vec::with_capacity(request.request_ids.len());
        let mut replayed = 0;
        for id in &request.request_ids {
            if request.confirmation_digest.is_some() {
                lock(&mut tx, id).await?;
            }
            let charge = select_stored(&mut tx, id, request.confirmation_digest.is_some())
                .await?
                .ok_or(Error::NotFound)?
                .charge;
            if let Some(digest) = &request.confirmation_digest
                && charge.state == "settled"
            {
                let audit: Value = sqlx::query_scalar(
                    "SELECT detail FROM core_charging.usage_events WHERE request_id=$1 AND event_id=$2",
                )
                .bind(id)
                .bind(EVENT)
                .fetch_optional(&mut *tx)
                .await?
                .ok_or(Error::Conflict)?;
                if audit["confirmation_digest"].as_str() != Some(digest.as_str())
                    || audit["route_id"] != request.route_id
                    || audit["reason"] != request.reason
                {
                    return Err(Error::Conflict);
                }
                let entry: Entry =
                    serde_json::from_value(audit["entry"].clone()).map_err(|_| Error::Conflict)?;
                if entry.request_id != *id
                    || charge.settled != entry.amount
                    || charge.usage != entry.billable
                    || charge.outcome.as_deref() != Some("completed")
                {
                    return Err(Error::Conflict);
                }
                entries.push(entry);
                replayed += 1;
                continue;
            }
            entries.push(prepare(&mut tx, &charge, &request.route_id).await?);
            charges.push(charge);
        }

        let digest = crate::hex_codec::encode(hash(&(
            EVENT,
            &request.route_id,
            &request.reason,
            &entries,
        ))?);
        let total = |value: fn(&Entry) -> i64| -> Result<i64> {
            entries.iter().try_fold(0_i64, |sum, entry| {
                sum.checked_add(value(entry)).ok_or(Error::Invalid)
            })
        };
        let receipt = json!({
            "confirmation_digest": digest,
            "mode": if request.confirmation_digest.is_none() { "preview" } else { "applied" },
            "route_id": request.route_id,
            "reason": request.reason,
            "settle_credits": total(|e| e.amount)?,
            "supplement_credits": total(|e| e.supplement)?,
            "release_credits": total(|e| e.release)?,
            "entries": entries,
        });
        let Some(confirmed) = &request.confirmation_digest else {
            tx.rollback().await?;
            return Ok(receipt);
        };
        if confirmed != &digest || (replayed > 0 && replayed != entries.len()) {
            return Err(Error::Conflict);
        }
        if replayed == 0 {
            for (entry, mut charge) in entries.iter().zip(charges) {
                event(
                    &mut tx,
                    &charge.id,
                    EVENT,
                    json!({
                        "kind": EVENT,
                        "route_id": request.route_id,
                        "reason": request.reason,
                        "confirmation_digest": digest,
                        "entry": entry,
                    }),
                )
                .await?;
                if entry.supplement > 0 {
                    sqlx::query(
                        "UPDATE core_charging.requests SET reserved=$2, revision=revision+1 WHERE id=$1",
                    )
                    .bind(&charge.id)
                    .bind(entry.amount)
                    .execute(&mut *tx)
                    .await?;
                    charge = load(&mut tx, &charge.id).await?.charge;
                    limits::budgets(&mut tx, &charge, entry.amount).await?;
                    self.ledger
                        .apply(
                            &mut tx,
                            command(
                                &charge,
                                &format!("reserve:{}", entry.amount),
                                Change::Reserve {
                                    total: entry.amount,
                                },
                            )?,
                        )
                        .await?;
                }
                self.post_settlement(
                    &mut tx,
                    charge,
                    entry.billable,
                    Outcome::Completed,
                    Some(EVENT),
                    entry.amount,
                )
                .await?;
            }
        }
        commit(tx).await?;
        Ok(receipt)
    }
}

async fn prepare(c: &mut PgConnection, charge: &Charge, route_id: &str) -> Result<Entry> {
    if charge.state != "reconcile"
        || charge.source != Source::Wallet
        || charge.key_id.is_none()
        || charge.oauth_source.is_some()
        || charge.usage != Usage::default()
        || charge.price.tiered.as_ref().is_none_or(|price| {
            price.tariff.revision != 2
                || !price.tariff.uses_cache_write
                || price.tariff.uses_cache_write_1h
                || price.input_semantics != crate::routing::InputSemantics::Inclusive
        })
    {
        return Err(Error::Invalid);
    }
    let proof = contract::load(c, charge).await?.ok_or(Error::Invalid)?;
    if !matches!(
        proof.legacy_precharge(),
        Some(contract::LegacyPrechargeAuthorization::Cc52TieredWalletV1 { .. })
    ) {
        return Err(Error::Invalid);
    }
    let (dispatch_id, dispatch): (String, Value) = sqlx::query_as(
        "SELECT event_id,detail FROM core_charging.usage_events WHERE request_id=$1 \
         AND event_id BETWEEN 'route-dispatch:1' AND 'route-dispatch:8' ORDER BY event_id DESC LIMIT 1",
    )
    .bind(&charge.id)
    .fetch_optional(&mut *c)
    .await?
    .ok_or(Error::Conflict)?;
    let wire = dispatch["wire_sha256"].as_str().ok_or(Error::Invalid)?;
    if dispatch["kind"] != "model_route_dispatch"
        || dispatch["route_id"].as_str() != Some(route_id)
        || !proof.permits_dispatch(route_id, wire)
    {
        return Err(Error::Conflict);
    }
    let (observation, failed): (Value, bool) = sqlx::query_as(
        "SELECT detail, EXISTS(SELECT 1 FROM core_charging.usage_events \
         WHERE request_id=$1 AND event_id=$3) FROM core_charging.usage_events \
         WHERE request_id=$1 AND event_id=$2",
    )
    .bind(&charge.id)
    .bind(OBSERVATION)
    .bind(dispatch_id.replace("route-dispatch:", "route-failure:"))
    .fetch_optional(&mut *c)
    .await?
    .ok_or(Error::Conflict)?;
    let (input_cap, output_cap) = proof.bounds();
    if failed
        || observation["kind"] != "provider_observation"
        || observation["provider_complete"] != true
        || observation["settlement_eligible"] != false
        || observation["route_id"].as_str() != Some(route_id)
        || observation["input_token_ceiling"].as_u64() != Some(input_cap)
        || observation["output_token_ceiling"].as_u64() != Some(output_cap)
    {
        return Err(Error::Conflict);
    }
    let observed: crate::relay::Usage =
        serde_json::from_value(observation["raw_usage"].clone()).map_err(|_| Error::Invalid)?;
    let (billable, pricing_observation) = corrected_observation(&observed, output_cap)?;
    proof.check_usage(charge, billable)?;
    let amount = contract::terminal_cost(
        &charge.price,
        charge.source,
        Some(&proof),
        billable,
        Some(&pricing_observation),
    )?;
    let observed_output = observed.output_tokens.ok_or(Error::Invalid)?;
    Ok(Entry {
        request_id: charge.id.clone(),
        revision: charge.revision,
        reserved: charge.reserved,
        amount,
        supplement: (amount - charge.reserved).max(0),
        release: (charge.reserved - amount).max(0),
        observed_output,
        billable,
        observation_sha256: crate::hex_codec::encode(hash(&observation)?),
        reservation_sha256: proof.prepared_digest().to_owned(),
    })
}

pub(crate) fn corrected_observation(
    observed: &crate::relay::Usage,
    output_cap: u64,
) -> Result<(Usage, crate::relay::Usage)> {
    observed.validate().map_err(|_| Error::Invalid)?;
    let input = observed.input_tokens.ok_or(Error::Invalid)?;
    let cached = observed.cached_input_tokens.ok_or(Error::Invalid)?;
    let output = observed.output_tokens.ok_or(Error::Invalid)?;
    if observed.uncached_input_tokens.is_some()
        || observed.cache_creation_input_tokens.is_some()
        || observed.cache_creation_details.is_some()
        || observed.openai_cache_write_details.is_some()
        || observed.cache_creation_mode.is_some()
        || observed.upstream_cost_atto_usd.is_some()
        || observed.legacy_openrouter_raw.is_some()
        || observed.tool.is_some()
        || observed.google_search_grounded_requests.is_some()
        || observed.tool_use_prompt_tokens.is_some()
    {
        return Err(Error::Invalid);
    }
    let to_i64 = |n| i64::try_from(n).map_err(|_| Error::Invalid);
    let mut pricing_observation = observed.clone();
    let capped = output.min(output_cap);
    pricing_observation.output_tokens = Some(capped);
    pricing_observation.reasoning_output_tokens = observed
        .reasoning_output_tokens
        .map(|tokens| tokens.min(capped));
    pricing_observation.cache_creation_input_tokens = Some(0);
    Ok((
        Usage {
            input: to_i64(input)?,
            cached: to_i64(cached)?,
            output: to_i64(capped)?,
            cache_creation: Some(CacheCreationUsage {
                total: 0,
                five_minute: Some(0),
                one_hour: Some(0),
                twelve_hour: None,
                cache_creation_mode: None,
                reported_total: None,
            }),
            ..Usage::default()
        },
        pricing_observation,
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn caps_only_billable_output_and_requires_measured_cache_reads() {
        let raw = crate::relay::Usage {
            input_tokens: Some(100),
            cached_input_tokens: Some(20),
            output_tokens: Some(12),
            ..Default::default()
        };
        let (usage, projected) = corrected_observation(&raw, 1).unwrap();
        assert_eq!((usage.input, usage.cached, usage.output), (100, 20, 1));
        assert_eq!(raw.output_tokens, Some(12));
        assert_eq!(projected.output_tokens, Some(1));
        assert_eq!(projected.cache_creation_input_tokens, Some(0));
        assert_eq!(usage.cache_creation.unwrap().total, 0);
        assert!(
            corrected_observation(
                &crate::relay::Usage {
                    cached_input_tokens: None,
                    ..raw.clone()
                },
                1
            )
            .is_err()
        );
        assert!(
            corrected_observation(
                &crate::relay::Usage {
                    cache_creation_input_tokens: Some(5),
                    ..raw
                },
                1
            )
            .is_err()
        );
    }
}

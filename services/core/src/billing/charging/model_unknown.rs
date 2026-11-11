//! An attempted supplier request with no authoritative final receipt retains
//! its entire hold. Only its original worker can persist this UNKNOWN marker.
use super::*;
impl Charging {
    #[allow(clippy::too_many_arguments)]
    pub(crate) async fn settle_measured_cost_with_supplement(
        &self,
        id: &str,
        worker: &[u8; 32],
        secret: &str,
        peer: Option<IpAddr>,
        issuer: Option<&crate::identity::grants::LmmIssuer>,
        usage: Usage,
        outcome: Outcome,
    ) -> Result<Charge> {
        let mut tx = self.begin().await?;
        lock(&mut tx, id).await?;
        let stored = load(&mut tx, id).await?;
        worker_matches(&stored, worker)?;
        let charge = stored.charge;
        if charge.price.upstream_cost_group_ppm.is_none() || usage.upstream_cost_atto_usd.is_none()
        {
            return Err(Error::Invalid);
        }
        let evidence: Value = sqlx::query_scalar(
            "SELECT detail FROM core_charging.usage_events WHERE request_id=$1 AND event_id='model-provider-terminal:v1'",
        )
        .bind(id)
        .fetch_optional(&mut *tx)
        .await?
        .ok_or(Error::Conflict)?;
        if evidence["kind"] != "model_provider_terminal"
            || evidence["normalized_usage"] != json(&usage)?
            || evidence["price_sha256"].as_str()
                != Some(contract::typed_digest(&charge.price)?.as_str())
        {
            return Err(Error::Conflict);
        }
        if charge.state == "settled" {
            if charge.usage != usage || charge.outcome.as_deref() != Some(outcome.text()) {
                return Err(Error::Conflict);
            }
            commit(tx).await?;
            return Ok(charge);
        }
        live(&charge, now(&mut tx).await?)?;
        if charge.state != "streaming" {
            return Err(Error::WrongState);
        }
        let amount = charge.price.cost(usage)?;
        let charge = if amount > charge.reserved {
            self.increase_in_transaction(&mut tx, charge, secret, amount, peer, issuer)
                .await?
        } else {
            charge
        };
        let charge = self
            .finish(
                &mut tx,
                charge,
                usage,
                outcome,
                Some("model-provider-terminal:v1"),
            )
            .await?;
        commit(tx).await?;
        Ok(charge)
    }

    /// Record normalized supplier usage before a funding attempt. This receipt
    /// carries no permission to increase a hold or debit a wallet.
    pub(crate) async fn record_model_terminal(
        &self,
        id: &str,
        worker: &[u8; 32],
        route_id: &str,
        usage: Usage,
    ) -> Result<()> {
        let mut tx = self.begin().await?;
        lock(&mut tx, id).await?;
        let stored = load(&mut tx, id).await?;
        worker_matches(&stored, worker)?;
        let charge = &stored.charge;
        let proof = contract::load(&mut tx, charge)
            .await?
            .ok_or(Error::Invalid)?;
        proof.check_usage(charge, usage)?;
        if !usage.follows(charge.usage) || charge.price.audio.is_some() {
            return Err(Error::Invalid);
        }
        let (dispatch_id, dispatch): (String, Value) = sqlx::query_as(
            "SELECT event_id,detail FROM core_charging.usage_events \
             WHERE request_id=$1 AND event_id BETWEEN 'route-dispatch:1' AND 'route-dispatch:8' \
             ORDER BY event_id DESC LIMIT 1",
        )
        .bind(id)
        .fetch_optional(&mut *tx)
        .await?
        .ok_or(Error::Conflict)?;
        let wire = dispatch["wire_sha256"].as_str().ok_or(Error::Invalid)?;
        if dispatch["kind"] != "model_route_dispatch"
            || dispatch["route_id"].as_str() != Some(route_id)
            || !proof.permits_dispatch(route_id, wire)
        {
            return Err(Error::Conflict);
        }
        let (observation, observed_at, failed): (Value, i64, bool) = sqlx::query_as(
            "SELECT detail,floor(extract(epoch FROM created_at))::bigint, \
             EXISTS(SELECT 1 FROM core_charging.usage_events WHERE request_id=$1 AND event_id=$2) \
             FROM core_charging.usage_events \
             WHERE request_id=$1 AND event_id='provider-observation:terminal'",
        )
        .bind(id)
        .bind(dispatch_id.replace("route-dispatch:", "route-failure:"))
        .fetch_optional(&mut *tx)
        .await?
        .ok_or(Error::Conflict)?;
        if failed
            || observation["kind"] != "provider_observation"
            || observation["provider_complete"] != true
            || observation["route_id"].as_str() != Some(route_id)
        {
            return Err(Error::Conflict);
        }
        let event_id = "model-provider-terminal:v1";
        let detail = json!({
            "kind": "model_provider_terminal",
            "normalized_usage": usage,
            "route_id": route_id,
            "dispatch_event": dispatch_id,
            "wire_sha256": wire,
            "price_sha256": contract::typed_digest(&charge.price)?,
            "observation_sha256": contract::typed_digest(&observation)?,
            "observed_at": observed_at,
        });
        let previous: Option<Value> = sqlx::query_scalar(
            "SELECT detail FROM core_charging.usage_events WHERE request_id=$1 AND event_id=$2",
        )
        .bind(id)
        .bind(event_id)
        .fetch_optional(&mut *tx)
        .await?;
        if let Some(previous) = previous {
            if previous != detail {
                return Err(Error::Conflict);
            }
            return commit(tx).await;
        }
        if !matches!(charge.state.as_str(), "streaming" | "reconcile") {
            return Err(Error::WrongState);
        }
        event(&mut tx, id, event_id, detail).await?;
        commit(tx).await
    }

    /// Audit only: never feeds accepted usage, provider cost or recovery.
    pub(crate) async fn record_provider_observation(
        &self,
        id: &str,
        worker: &[u8; 32],
        route_id: &str,
        raw_usage: &Value,
        input_token_ceiling: i64,
        output_token_ceiling: u32,
    ) -> Result<()> {
        // @context-fold begin charging-provider-observation
        if !raw_usage.is_object() {
            return Err(Error::Invalid);
        }
        let detail = json!({
            "kind": "provider_observation",
            "provider_complete": true,
            "settlement_eligible": false,
            "route_id": route_id,
            "raw_usage": raw_usage,
            "input_token_ceiling": input_token_ceiling,
            "output_token_ceiling": output_token_ceiling,
        });
        let mut tx = self.begin().await?;
        lock(&mut tx, id).await?;
        let stored = load(&mut tx, id).await?;
        worker_matches(&stored, worker)?;
        let event_id = "provider-observation:terminal";
        let previous: Option<Value> = sqlx::query_scalar(
            "SELECT detail FROM core_charging.usage_events WHERE request_id=$1 AND event_id=$2",
        )
        .bind(id)
        .bind(event_id)
        .fetch_optional(&mut *tx)
        .await?;
        if let Some(previous) = previous {
            if previous != detail {
                return Err(Error::Conflict);
            }
            return commit(tx).await;
        }
        if !matches!(stored.charge.state.as_str(), "streaming" | "reconcile") {
            return Err(Error::WrongState);
        }
        event(&mut tx, id, event_id, detail).await?;
        commit(tx).await
        // @context-fold end charging-provider-observation
    }

    pub(crate) async fn mark_unknown(&self, id: &str, worker: &[u8; 32]) -> Result<()> {
        let mut tx = self.begin().await?;
        lock(&mut tx, id).await?;
        let stored = load(&mut tx, id).await?;
        worker_matches(&stored, worker)?;
        match stored.charge.state.as_str() {
            "streaming" => {
                sqlx::query("UPDATE core_charging.requests SET state='reconcile',revision=revision+1 WHERE id=$1 AND state='streaming'").bind(id).execute(&mut *tx).await?;
                event(
                    &mut tx,
                    id,
                    "unknown",
                    json!({"kind":"unknown_provider_usage","hold_retained":true}),
                )
                .await?;
            }
            "reconcile" | "settled" => (),
            _ => return Err(Error::WrongState),
        }
        commit(tx).await
    }
}

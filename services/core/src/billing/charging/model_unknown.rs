//! An attempted supplier request with no authoritative final receipt retains
//! its entire hold. Only its original worker can persist this UNKNOWN marker.
use super::*;
impl Charging {
    /// Read the same durable terminal price used by settlement. This read does
    /// not dispatch, reserve, settle or mutate a request.
    pub(crate) async fn terminal_amount(&self, id: &str, usage: Usage) -> Result<i64> {
        let mut tx = self.begin().await?;
        lock(&mut tx, id).await?;
        let charge = load(&mut tx, id).await?.charge;
        let amount = Self::terminal_amount_in_transaction(&mut tx, &charge, usage).await?;
        commit(tx).await?;
        Ok(amount)
    }

    pub(super) async fn terminal_amount_in_transaction(
        c: &mut PgConnection,
        charge: &Charge,
        usage: Usage,
    ) -> Result<i64> {
        let proof = contract::load(c, charge).await?;
        if !proof.as_ref().is_some_and(|proof| {
            charge.source == Source::Wallet
                && matches!(
                    proof.legacy_precharge(),
                    Some(contract::LegacyPrechargeAuthorization::Cc52TieredWalletV1 { .. })
                )
        }) {
            return charge.price.cost(usage);
        }
        proof
            .as_ref()
            .ok_or(Error::Invalid)?
            .check_usage(charge, usage)?;
        let observation: Option<Value> = sqlx::query_scalar(
            "SELECT detail FROM core_charging.usage_events WHERE request_id=$1 AND event_id='provider-observation:terminal'"
        ).bind(&charge.id).fetch_optional(&mut *c).await?;
        let observed = if let Some(observation) = observation {
            let proof = proof.as_ref().ok_or(Error::Invalid)?;
            if observation["kind"] != "provider_observation"
                || observation["provider_complete"] != true
                || !observation["route_id"]
                    .as_str()
                    .is_some_and(|route| proof.contains_route(route))
            {
                return Err(Error::Conflict);
            }
            Some(
                serde_json::from_value::<crate::relay::Usage>(observation["raw_usage"].clone())
                    .map_err(|_| Error::Invalid)?,
            )
        } else if usage == Usage::default() {
            legacy_terminal(c, charge, usage).await?;
            None
        } else {
            return Err(Error::Conflict);
        };
        contract::terminal_cost(
            &charge.price,
            charge.source,
            proof.as_ref(),
            usage,
            observed.as_ref(),
        )
    }

    /// The original frozen wallet authorization survives key revocation and
    /// membership changes. Only supplier terminal evidence may consume it.
    pub(crate) async fn settle_legacy_wallet_terminal(
        &self,
        id: &str,
        worker: &[u8; 32],
        usage: Usage,
        outcome: Outcome,
    ) -> Result<Charge> {
        let mut tx = self.begin().await?;
        lock(&mut tx, id).await?;
        let stored = load(&mut tx, id).await?;
        worker_matches(&stored, worker)?;
        let charge = self
            .finish_legacy_wallet(
                &mut tx,
                stored.charge,
                usage,
                outcome,
                Some("model-provider-terminal:v1"),
            )
            .await?;
        commit(tx).await?;
        Ok(charge)
    }

    pub(super) async fn finish_legacy_wallet(
        &self,
        c: &mut PgConnection,
        mut charge: Charge,
        usage: Usage,
        outcome: Outcome,
        evidence: Option<&str>,
    ) -> Result<Charge> {
        let proof = contract::load(c, &charge).await?.ok_or(Error::Invalid)?;
        let legacy = proof.legacy_precharge().ok_or(Error::Invalid)?;
        if charge.source != Source::Wallet {
            return Err(Error::Invalid);
        }
        proof.check_usage(&charge, usage)?;
        let terminal = legacy_terminal(c, &charge, usage).await?;
        let amount = Self::terminal_amount_in_transaction(c, &charge, usage).await?;
        if amount > legacy.max_billable() || !usage.follows(charge.usage) {
            return Err(Error::Invalid);
        }
        if charge.state == "settled" {
            if charge.usage != usage || charge.outcome.as_deref() != Some(outcome.text()) {
                return Err(Error::Conflict);
            }
            return Ok(charge);
        }
        if !matches!(charge.state.as_str(), "streaming" | "reconcile") {
            return Err(Error::WrongState);
        }
        if amount > charge.reserved {
            let previous_reserved = charge.reserved;
            sqlx::query(
                "UPDATE core_charging.requests SET reserved=$2,revision=revision+1 WHERE id=$1",
            )
            .bind(&charge.id)
            .bind(amount)
            .execute(&mut *c)
            .await?;
            charge = load(c, &charge.id).await?.charge;
            // These checks keep the original actor/payer and authorized_at,
            // including OAuth family, every issuer and the original period.
            limits::budgets(c, &charge, amount).await?;
            event(
                c,
                &charge.id,
                "legacy-wallet-supplement:v1",
                json!({
                    "kind": "legacy_wallet_supplement",
                    "prepared_sha256": proof.prepared_digest(),
                    "terminal": terminal,
                    "usage": usage,
                    "price": charge.price,
                    "previous_reserved": previous_reserved,
                    "amount": amount,
                    "operator_max_billable": legacy.max_billable(),
                    "request_revision": charge.revision,
                }),
            )
            .await?;
            self.ledger
                .apply(
                    c,
                    command(
                        &charge,
                        &format!("reserve:{}", amount),
                        Change::Reserve { total: amount },
                    )?,
                )
                .await?;
        }
        let charge = self.finish(c, charge, usage, outcome, evidence).await?;
        Ok(charge)
    }

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
        // The immutable terminal receipt proves the original dispatch completed.
        // Recovery retains its original worker and reauthorizes any extra hold.
        // Its dispatch lease may have expired while the payer adds funds.
        if !matches!(charge.state.as_str(), "streaming" | "reconcile") {
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

    /// Repair the crash window after durable complete supplier audit and the
    /// gateway terminal checkpoint, before its normalized charging receipt.
    /// The trusted recovery caller has reprojected the full immutable raw audit
    /// with the frozen price and checked it against the terminal checkpoint.
    #[allow(clippy::too_many_arguments)]
    pub(crate) async fn reconcile_legacy_wallet_observation(
        &self,
        id: &str,
        expected_revision: i64,
        route_id: &str,
        usage: Usage,
        outcome: Outcome,
        evidence: &str,
    ) -> Result<Charge> {
        Self::evidence(evidence)?;
        let mut tx = self.begin().await?;
        lock(&mut tx, id).await?;
        let stored = load(&mut tx, id).await?;
        let proof = contract::load(&mut tx, &stored.charge)
            .await?
            .ok_or(Error::Invalid)?;
        if stored.charge.source != Source::Wallet || proof.legacy_precharge().is_none() {
            return Err(Error::Invalid);
        }
        let raw: Value = sqlx::query_scalar(
            "SELECT detail->'raw_usage' FROM core_charging.usage_events WHERE request_id=$1 AND event_id='provider-observation:terminal'"
        ).bind(id).fetch_optional(&mut *tx).await?.ok_or(Error::Conflict)?;
        let observed: crate::relay::Usage =
            serde_json::from_value(raw).map_err(|_| Error::Conflict)?;
        if stored.charge.state == "settled" {
            Self::record_model_terminal_in_transaction(&mut tx, &stored.charge, route_id, usage)
                .await?;
            let previous: Value = sqlx::query_scalar(
                "SELECT detail FROM core_charging.usage_events WHERE request_id=$1 AND event_id='final'"
            ).bind(id).fetch_one(&mut *tx).await?;
            if stored.charge.usage != usage
                || stored.charge.outcome.as_deref() != Some(outcome.text())
                || previous["evidence"].as_str() != Some(evidence)
            {
                return Err(Error::Conflict);
            }
            commit(tx).await?;
            return Ok(stored.charge);
        }
        if stored.charge.state != "reconcile" || stored.charge.revision != expected_revision {
            return Err(Error::WrongState);
        }
        let checkpoint = legacy_terminal_checkpoint(&mut tx, id, expected_revision).await?;
        if checkpoint != json(&observed.checkpoint_usage())? {
            return Err(Error::Conflict);
        }
        Self::record_model_terminal_in_transaction(&mut tx, &stored.charge, route_id, usage)
            .await?;
        let charge = self
            .finish_legacy_wallet(&mut tx, stored.charge, usage, outcome, Some(evidence))
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
        Self::record_model_terminal_in_transaction(&mut tx, &stored.charge, route_id, usage)
            .await?;
        commit(tx).await
    }

    async fn record_model_terminal_in_transaction(
        c: &mut PgConnection,
        charge: &Charge,
        route_id: &str,
        usage: Usage,
    ) -> Result<()> {
        let id = &charge.id;
        let proof = contract::load(c, charge).await?.ok_or(Error::Invalid)?;
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
        let (observation, observed_at, failed): (Value, i64, bool) = sqlx::query_as(
            "SELECT detail,floor(extract(epoch FROM created_at))::bigint, \
             EXISTS(SELECT 1 FROM core_charging.usage_events WHERE request_id=$1 AND event_id=$2) \
             FROM core_charging.usage_events \
             WHERE request_id=$1 AND event_id='provider-observation:terminal'",
        )
        .bind(id)
        .bind(dispatch_id.replace("route-dispatch:", "route-failure:"))
        .fetch_optional(&mut *c)
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
        .fetch_optional(&mut *c)
        .await?;
        if let Some(previous) = previous {
            if previous != detail {
                return Err(Error::Conflict);
            }
            return Ok(());
        }
        if !matches!(charge.state.as_str(), "streaming" | "reconcile") {
            return Err(Error::WrongState);
        }
        event(c, id, event_id, detail).await?;
        Ok(())
    }

    /// Complete raw supplier evidence. It acquires charging authority only
    /// after frozen-contract projection and the checked terminal receipt path.
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

/// Shared with finish so generic settlement/recovery cannot bypass the legacy
/// terminal requirement, even when the measured amount fits the initial hold.
pub(super) async fn legacy_terminal(
    c: &mut PgConnection,
    charge: &Charge,
    usage: Usage,
) -> Result<Value> {
    if usage == Usage::default() {
        let rejected: Option<Value> = sqlx::query_scalar(
            "SELECT detail FROM core_charging.usage_events WHERE request_id=$1 AND event_id='model-no-acceptance:v1'"
        ).bind(&charge.id).fetch_optional(&mut *c).await?;
        if let Some(rejected) = rejected {
            let receipt: crate::billing::text_request_price::TextRequestUsage =
                serde_json::from_value(rejected["receipt"].clone()).map_err(|_| Error::Invalid)?;
            let proof = contract::load(c, charge).await?.ok_or(Error::Invalid)?;
            if rejected["kind"] != "model_no_acceptance"
                || rejected["price_sha256"].as_str()
                    != Some(contract::typed_digest(&charge.price)?.as_str())
                || !receipt.valid()
                || receipt.completed_requests() != 0
                || !rejected["route_id"]
                    .as_str()
                    .is_some_and(|route| proof.contains_route(route))
            {
                return Err(Error::Conflict);
            }
            return Ok(rejected);
        }
    }
    let evidence: Value = sqlx::query_scalar(
        "SELECT detail FROM core_charging.usage_events WHERE request_id=$1 AND event_id='model-provider-terminal:v1'"
    ).bind(&charge.id).fetch_optional(&mut *c).await?.ok_or(Error::Conflict)?;
    if evidence["kind"] != "model_provider_terminal"
        || evidence["normalized_usage"] != json(&usage)?
        || evidence["price_sha256"].as_str()
            != Some(contract::typed_digest(&charge.price)?.as_str())
    {
        return Err(Error::Conflict);
    }
    Ok(evidence)
}

/// Read through the canonical SECURITY DEFINER boundary in the original funds
/// transaction. The function intentionally exposes only reconciliation rows.
async fn legacy_terminal_checkpoint(
    c: &mut PgConnection,
    id: &str,
    revision: i64,
) -> Result<Value> {
    // IDs are validated ASCII. A shorter prefix sorts before the exact ID and
    // avoids scanning unrelated earlier requests. Other matching prefixes can
    // fill a page, so continue until the exact ID or the end of its range.
    let mut after = id
        .get(..id.len().saturating_sub(1))
        .filter(|prefix| !prefix.is_empty())
        .map(str::to_owned);
    loop {
        let rows: Vec<(String, i64, Value)> = sqlx::query_as(
            "SELECT request_id,revision,provider_usage FROM core_model.completed_streams_after(1000,$1) ORDER BY request_id"
        ).bind(after.as_deref()).fetch_all(&mut *c).await?;
        for (request_id, observed_revision, checkpoint) in &rows {
            if request_id == id {
                if *observed_revision != revision {
                    return Err(Error::WrongState);
                }
                return Ok(checkpoint.clone());
            }
        }
        let Some((last, _, _)) = rows.last() else {
            return Err(Error::Conflict);
        };
        if rows.len() < 1000 || last.as_str() >= id {
            return Err(Error::Conflict);
        }
        after = Some(last.clone());
    }
}

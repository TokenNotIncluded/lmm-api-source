//! Append-only transport evidence for one reservation and one frozen sale.
use super::*;

impl Charging {
    /// The final worker classifies delivery before recording zero consumption.
    /// A zero token count or bare HTTP status can never create this evidence.
    pub(crate) async fn record_no_acceptance(
        &self,
        id: &str,
        worker: &[u8; 32],
        route_id: &str,
        receipt: crate::model_http::text_request_price::TextRequestUsage,
    ) -> Result<()> {
        if !receipt.valid() || receipt.completed_requests() != 0 {
            return Err(Error::Invalid);
        }
        let mut tx = self.begin().await?;
        lock(&mut tx, id).await?;
        let stored = load(&mut tx, id).await?;
        worker_matches(&stored, worker)?;
        let proof = contract::load(&mut tx, &stored.charge)
            .await?
            .ok_or(Error::Invalid)?;
        if !proof.contains_route(route_id) {
            return Err(Error::Conflict);
        }
        let detail = json!({"kind":"model_no_acceptance","receipt":receipt,"route_id":route_id,
            "price_sha256":contract::typed_digest(&stored.charge.price)?});
        let event_id = "model-no-acceptance:v1";
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
    }
    pub(crate) async fn record_route_dispatch(
        &self,
        id: &str,
        worker: &[u8; 32],
        number: u8,
        route_id: &str,
        actual_group: &str,
        wire_sha256: &str,
    ) -> Result<()> {
        if !(1..=8).contains(&number) {
            return Err(Error::Invalid);
        }
        let detail = json!({"kind":"model_route_dispatch","number":number,
            "route_id":route_id,"actual_group":actual_group,"wire_sha256":wire_sha256});
        let mut tx = self.begin().await?;
        lock(&mut tx, id).await?;
        let stored = load(&mut tx, id).await?;
        worker_matches(&stored, worker)?;
        let proof = contract::load(&mut tx, &stored.charge)
            .await?
            .ok_or(Error::Invalid)?;
        if !proof.permits_dispatch(route_id, wire_sha256) {
            return Err(Error::Conflict);
        }
        let event_id = format!("route-dispatch:{number}");
        let previous: Option<Value> = sqlx::query_scalar(
            "SELECT detail FROM core_charging.usage_events WHERE request_id=$1 AND event_id=$2",
        )
        .bind(id)
        .bind(&event_id)
        .fetch_optional(&mut *tx)
        .await?;
        if let Some(previous) = previous {
            if previous != detail {
                return Err(Error::Conflict);
            }
            return commit(tx).await;
        }
        if stored.charge.state != "streaming" {
            return Err(Error::WrongState);
        }
        if number > 1 {
            let previous: Value = sqlx::query_scalar(
                "SELECT detail FROM core_charging.usage_events WHERE request_id=$1 AND event_id=$2",
            )
            .bind(id)
            .bind(format!("route-failure:{}", number - 1))
            .fetch_optional(&mut *tx)
            .await?
            .ok_or(Error::Conflict)?;
            let failure: crate::relay::AttemptFailure =
                serde_json::from_value(previous["failure"].clone()).map_err(|_| Error::Invalid)?;
            if !matches!(
                failure.failure.delivery,
                crate::routing::DeliveryState::NotSent | crate::routing::DeliveryState::Rejected
            ) {
                return Err(Error::Invalid);
            }
        }
        event(&mut tx, id, &event_id, detail).await?;
        commit(tx).await
    }

    pub(crate) async fn record_route_failure(
        &self,
        id: &str,
        worker: &[u8; 32],
        number: u8,
        route_id: &str,
        failure: &crate::relay::AttemptFailure,
    ) -> Result<()> {
        if !(1..=8).contains(&number)
            || (failure.failure.delivery == crate::routing::DeliveryState::Rejected
                && (failure.http_status.is_none() || failure.response_sha256.is_none()))
        {
            return Err(Error::Invalid);
        }
        let detail = json!({"kind":"model_route_failure","number":number,"route_id":route_id,"failure":failure});
        let mut tx = self.begin().await?;
        lock(&mut tx, id).await?;
        let stored = load(&mut tx, id).await?;
        worker_matches(&stored, worker)?;
        let dispatch: Value = sqlx::query_scalar(
            "SELECT detail FROM core_charging.usage_events WHERE request_id=$1 AND event_id=$2",
        )
        .bind(id)
        .bind(format!("route-dispatch:{number}"))
        .fetch_optional(&mut *tx)
        .await?
        .ok_or(Error::Conflict)?;
        if dispatch["route_id"].as_str() != Some(route_id) {
            return Err(Error::Conflict);
        }
        let event_id = format!("route-failure:{number}");
        let previous: Option<Value> = sqlx::query_scalar(
            "SELECT detail FROM core_charging.usage_events WHERE request_id=$1 AND event_id=$2",
        )
        .bind(id)
        .bind(&event_id)
        .fetch_optional(&mut *tx)
        .await?;
        if let Some(previous) = previous {
            if previous != detail {
                return Err(Error::Conflict);
            }
            return commit(tx).await;
        }
        if stored.charge.state != "streaming" {
            return Err(Error::WrongState);
        }
        event(&mut tx, id, &event_id, detail).await?;
        commit(tx).await
    }
}

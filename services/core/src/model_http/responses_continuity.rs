//! Ownership and immutable provider-context references, never response bodies.
use super::*;
use crate::{
    billing::charging::{self, contract::ReservationBinding},
    routing::Protocol,
};
use serde::Deserialize;

pub(super) async fn uses_native(gateway: &ModelGateway, headers: &HeaderMap, raw: &[u8]) -> bool {
    // OAuth keeps its original scope checks and OAuth request-ID namespace.
    // The new persisted continuation contract currently admits API keys only.
    if oauth_header(headers) {
        return false;
    }
    let Ok(value) = serde_json::from_slice::<serde_json::Value>(raw) else {
        return false;
    };
    if value.get("previous_response_id").is_some()
        || value["store"] == true
        || super::responses_native::handles(raw)
    {
        return true;
    }
    let Some(model) = value["model"].as_str() else {
        return false;
    };
    let Ok((_, _, groups)) = gateway.authenticate(headers).await else {
        return false;
    };
    gateway
        .bridge
        .routing
        .route_adapted_groups(model, &groups, Protocol::OpenAiResponses, None)
        .ok()
        .and_then(|plans| plans.into_iter().next())
        .is_some_and(|plan| {
            plan.planned_attempts()
                .first()
                .is_some_and(|attempt| gateway.bridge.provider_policy.has_continuity(attempt))
        })
}

pub(super) struct Policy {
    pub default_store: bool,
    pub sha256: String,
}

#[derive(Deserialize)]
pub(super) struct Parent {
    pub request_id: String,
    pub provider_response_id: String,
    pub public_model: String,
    pub canonical_model: String,
    pub group_name: String,
    pub binding: ReservationBinding,
    pub policy_sha256: String,
}

pub(super) struct Candidate {
    pub provider_response_id: String,
    pub parent_request_id: Option<String>,
    pub public_model: String,
    pub policy_sha256: Vec<u8>,
}

pub(super) fn handle(id: &str) -> String {
    format!(
        "resp_lmm_{}",
        crate::hex_codec::encode(Sha256::digest(id.as_bytes()))
    )
}

pub(super) fn rewrite_event(event: &mut serde_json::Value, handle: &str) {
    if let Some(response) = event.get_mut("response")
        && response.get("id").is_some()
    {
        response["id"] = handle.into();
    }
    if event.get("response_id").is_some() {
        event["response_id"] = handle.into();
    }
}

#[cfg(test)]
pub(super) use super::responses_fault_support::{crash_at, lose_ack_once};
#[cfg(not(test))]
pub(super) fn crash_at(_stage: &str) {}
#[cfg(not(test))]
pub(super) fn lose_ack_once(_stage: &str) -> bool {
    false
}

pub(super) async fn resolve(
    gateway: &ModelGateway,
    caller: &native_http::Caller,
    handle: &str,
    model: &str,
) -> Result<Parent, RelayError> {
    if handle.len() != 73
        || !handle.starts_with("resp_lmm_")
        || !handle[9..]
            .bytes()
            .all(|b| b.is_ascii_hexdigit() && !b.is_ascii_uppercase())
    {
        return Err(RelayError::Invalid("continuation unavailable"));
    }
    let mut tx = gateway
        .bridge
        .pool
        .begin()
        .await
        .map_err(|_| RelayError::BillingUnavailable)?;
    sqlx::query("SET TRANSACTION ISOLATION LEVEL SERIALIZABLE")
        .execute(&mut *tx)
        .await
        .map_err(|_| RelayError::BillingUnavailable)?;
    let value: Option<serde_json::Value> =
        sqlx::query_scalar("SELECT core_model.resolve_continuation($1,$2)")
            .bind(handle)
            .bind(&caller.secret)
            .fetch_one(&mut *tx)
            .await
            .map_err(|_| RelayError::BillingUnavailable)?;
    let parent: Parent =
        serde_json::from_value(value.ok_or(RelayError::Invalid("continuation unavailable"))?)
            .map_err(|_| RelayError::SettlementPending)?;
    if parent.public_model != model || !caller.groups.contains(&parent.group_name) {
        return Err(RelayError::Invalid("continuation unavailable"));
    }
    crate::identity::authorize_model_request_in_transaction(
        &mut tx,
        &caller.secret,
        &parent.canonical_model,
        caller.peer,
        &parent.group_name,
    )
    .await
    .map_err(identity_error)?;
    tx.commit()
        .await
        .map_err(|_| RelayError::SettlementPending)?;
    Ok(parent)
}

impl Parent {
    pub(super) fn matches(
        &self,
        attempt: &crate::routing::Attempt,
        canonical: &str,
        policy: &Policy,
    ) -> Result<(), RelayError> {
        let binding = &self.binding;
        let endpoint = super::input_api::endpoint(attempt.endpoint(), Protocol::OpenAiResponses)?;
        let route = attempt.route_key();
        if self.canonical_model != canonical
            || binding.route_id != route
            || binding.target_id != attempt.target_id()
            || binding.upstream_id != attempt.upstream_id()
            || binding.credential_ref != attempt.credential_ref()
            || binding.model_snapshot != attempt.upstream_model()
            || binding.protocol != "responses"
            || binding.generation_url != endpoint.as_str()
            || binding.payload_policy_sha256
                != charging::contract::typed_digest(&attempt.payload_policy())
                    .map_err(billing::error)?
            || self.policy_sha256 != policy.sha256
        {
            return Err(RelayError::Invalid("continuation binding unavailable"));
        }
        Ok(())
    }
}

pub(super) async fn activate(bridge: &Bridge, id: &str) -> Result<(), RelayError> {
    for _ in 0..3 {
        let mut tx = match bridge.pool.begin().await {
            Ok(tx) => tx,
            Err(_) => continue,
        };
        if sqlx::query("SET LOCAL synchronous_commit=on")
            .execute(&mut *tx)
            .await
            .is_err()
        {
            continue;
        }
        let active = sqlx::query_scalar::<_, bool>("SELECT core_model.activate_continuation($1)")
            .bind(id)
            .fetch_one(&mut *tx)
            .await;
        match active {
            Ok(true) => {
                if tx.commit().await.is_ok() && !lose_ack_once("activation") {
                    return Ok(());
                }
            }
            Ok(false) => return Err(RelayError::SettlementPending),
            Err(_) => (),
        }
    }
    Err(RelayError::SettlementPending)
}

/// Financial settlements and activation transitions have separate meanings.
#[derive(Clone, Debug, PartialEq, Eq, serde::Serialize)]
pub struct RecoveryPage<C> {
    pub settled: usize,
    pub activated: usize,
    pub next: Option<C>,
}
/// Independent keyset scans: None starts a sweep, not a shared request ID.
#[derive(Clone, Default, Debug, PartialEq, Eq, serde::Serialize)]
pub struct ContinuityRecoveryCursor {
    completed_after: Option<String>,
    activation_after: Option<String>,
    completed_done: bool,
    activation_done: bool,
}
impl Bridge {
    pub(super) async fn activate_continuations_page(
        &self,
        limit: i32,
        after: Option<&str>,
    ) -> Result<RecoveryPage<String>, RelayError> {
        let rows: Vec<(String, bool)> = sqlx::query_as(
            "SELECT request_id,activated FROM core_model.activate_continuations_after($1,$2)",
        )
        .bind(limit)
        .bind(after)
        .fetch_all(&self.pool)
        .await
        .map_err(|_| RelayError::BillingUnavailable)?;
        let next = rows
            .last()
            .filter(|_| rows.len() == limit as usize)
            .map(|r| r.0.clone());
        Ok(RecoveryPage {
            settled: 0,
            activated: rows.iter().filter(|r| r.1).count(),
            next,
        })
    }
}
impl ModelGateway {
    /// Both scans retain their own progress, including pages with zero activations.
    pub async fn reconcile_responses_continuity_page(
        &self,
        limit: i32,
        cursor: Option<&ContinuityRecoveryCursor>,
    ) -> Result<RecoveryPage<ContinuityRecoveryCursor>, RelayError> {
        if !(1..=1000).contains(&limit) {
            return Err(RelayError::Invalid("continuation recovery page"));
        }
        let mut cursor = cursor.cloned().unwrap_or_default();
        let mut settled = 0;
        let mut activated = 0;
        if !cursor.completed_done {
            let page = self
                .reconcile_completed_streams_page(limit, cursor.completed_after.as_deref())
                .await?;
            settled += page.settled;
            cursor.completed_done = page.next.is_none();
            cursor.completed_after = page.next;
        }
        if !cursor.activation_done {
            let page = self
                .bridge
                .activate_continuations_page(limit, cursor.activation_after.as_deref())
                .await?;
            activated = page.activated;
            cursor.activation_done = page.next.is_none();
            cursor.activation_after = page.next;
        }
        Ok(RecoveryPage {
            settled,
            activated,
            next: (!(cursor.completed_done && cursor.activation_done)).then_some(cursor),
        })
    }
}

#[cfg(test)]
#[path = "responses_continuity_tests.rs"]
pub(super) mod tests;

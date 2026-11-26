//! Apply a hint only inside the already-authorized frozen route plan.
use super::{Bridge, ModelIdentity};
use crate::{
    relay::{RelayError, Request},
    routing::{
        Attempt, RoutePlan,
        affinity::{Binding, Input, store},
    },
};
use axum::http::HeaderMap;
use sqlx::PgPool;
use std::sync::Arc;

pub(super) async fn prepare(
    bridge: &Bridge,
    plans: &mut [RoutePlan],
    principal: &ModelIdentity,
    headers: &HeaderMap,
    path: &str,
    raw: &[u8],
    request: Option<&Request>,
) -> Result<(), RelayError> {
    let Some(first) = plans.first().filter(|p| p.affinity_policy().enabled()) else {
        return Ok(());
    };
    let value =
        serde_json::from_slice(raw).map_err(|_| RelayError::Invalid("invalid request JSON"))?;
    let model = first.requested_model().to_owned();
    let tenant_id = match principal {
        ModelIdentity::Native(p) => p.owner_account_id,
        ModelIdentity::OAuth(p) => p.personal_account_id(),
    };
    let input = Input {
        tenant_id,
        user_id: principal.user_id(),
        key_id: principal.key_id(),
        headers,
        model: &model,
        path,
        body: &value,
    };
    for plan in plans {
        let Some(mut binding) = plan.affinity_policy().bind(&input, plan.group()) else {
            continue;
        };
        let mapped = store::lookup(&bridge.pool, &binding.key)
            .await
            .map_err(|_| RelayError::RouteUnavailable)?;
        if let Some(mapped) = mapped {
            let admissible = plan
                .candidate_attempts()
                .into_iter()
                .find(|a| a.upstream_id() == mapped.channel)
                .is_some_and(|attempt| {
                    let reference = attempt.credential_ref();
                    if !attempt.is_available()
                        || !bridge
                            .credentials
                            .contains_key(&(reference.id, reference.revision))
                        || mapped.revision.as_slice() != attempt.affinity_revision()
                    {
                        return false;
                    }
                    if let Some(request) = request {
                        bridge
                            .route_for_attempt(Arc::new(attempt), request)
                            .and_then(|(_, route)| request.upstream_body(&route))
                            .is_ok()
                    } else {
                        true
                    }
                });
            if admissible {
                plan.prefer_channel(mapped.channel);
            } else if binding.keep_invalid {
                binding.preserve = true;
            } else {
                store::invalidate(&bridge.pool, &binding.key, &mapped)
                    .await
                    .map_err(|_| RelayError::RouteUnavailable)?;
            }
        }
        plan.affinity_binding = Some(binding);
    }
    Ok(())
}

pub(super) async fn completed(pool: &PgPool, binding: &Binding, attempt: &Attempt) {
    if store::success(
        pool,
        binding,
        attempt.upstream_id(),
        &attempt.affinity_revision(),
    )
    .await
    .is_err()
    {
        // The supplier and ledger already completed. A cache write cannot change their outcome.
        eprintln!("core_channel_affinity_success_write_failed");
    }
}

//! One pinned candidate set, one sales quote, one charging request.
use super::*;
use crate::billing::charging::contract::{
    self, PreparedReservation, ReservationBinding, ReservationBounds, ReservationFallback,
};
use crate::routing::{DeliveryState, RoutePlan, RoutingError};
use std::collections::{HashSet, VecDeque};

pub(crate) enum Pending {
    Planned(Vec<RoutePlan>),
    Active(Session),
}

struct Candidate {
    selected: Selected,
    route: Route,
    binding: ReservationFallback,
}

pub(crate) struct Session {
    plans: VecDeque<RoutePlan>,
    candidates: BTreeMap<String, Candidate>,
    primary: String,
    number: u8,
    attempt_limit: u8,
    inbound_protocol: ClientProtocol,
}

fn binding(
    bridge: &Bridge,
    selected: &Selected,
    route: &Route,
    request: &Request,
) -> Result<ReservationFallback, RelayError> {
    let (revision, semantics) = bridge
        .provider_policy
        .reservation_metadata(&selected.attempt)?;
    Ok(ReservationFallback {
        binding: ReservationBinding::for_attempt(
            &selected.attempt,
            match route.protocol {
                UpstreamProtocol::Chat => "chat",
                UpstreamProtocol::Responses => "responses",
                UpstreamProtocol::Anthropic => "anthropic",
                UpstreamProtocol::Gemini => "gemini",
            }
            .into(),
            route.model.clone(),
            route.endpoint.as_str().into(),
            revision,
            "context-max-v1".into(),
            route.price_version,
        )
        .map_err(super::super::billing::error)?,
        wire_sha256: contract::digest(&request.upstream_body(route)?),
        bounds: ReservationBounds {
            model_input_max: selected.input_token_ceiling as u64,
            input: selected.input_token_ceiling as u64,
            requested_output: u64::from(request.max_output_tokens),
            output: u64::from(request.max_output_tokens),
        },
        semantics,
    })
}

impl Pending {
    pub(in crate::model_http) fn affinity_binding(
        &self,
    ) -> Option<&crate::routing::affinity::Binding> {
        match self {
            Self::Active(session) => session.plans.front()?.affinity_binding.as_ref(),
            Self::Planned(_) => None,
        }
    }
}

impl Session {
    fn next(
        &mut self,
        failure: crate::routing::Failure,
        status: Option<u16>,
    ) -> Result<Option<String>, RelayError> {
        if self.number >= self.attempt_limit {
            return Ok(None);
        }
        let Some(current) = self.plans.front_mut() else {
            return Ok(None);
        };
        if !current.allows_failure(failure, status) {
            return Ok(None);
        }
        let attempt = match current.retry_http(failure, status) {
            Ok(attempt) => Some(attempt),
            Err(RoutingError::AttemptsExhausted) => {
                self.plans.pop_front();
                let mut next = None;
                while let Some(plan) = self.plans.front_mut() {
                    match plan.start() {
                        Ok(attempt) => {
                            next = Some(attempt);
                            break;
                        }
                        Err(RoutingError::AttemptsExhausted) => {
                            self.plans.pop_front();
                        }
                        Err(_) => return Err(RelayError::RouteUnavailable),
                    }
                }
                next
            }
            Err(_) => None,
        };
        if let Some(attempt) = attempt {
            self.number += 1;
            let id = attempt.route_key();
            if !self.candidates.contains_key(&id) {
                return Err(RelayError::Conflict);
            }
            return Ok(Some(id));
        }
        Ok(None)
    }
}

// A retry uses the existing reservation. Its next supplier must interpret
// usage under the same frozen cache contract, even on the same wire protocol.
fn compatible_official_meter(
    primary: &crate::billing::charging::Price,
    next: &crate::billing::charging::Price,
) -> bool {
    let signature = |price: &crate::billing::charging::Price| {
        price.tiered.as_ref().map(|stored| {
            let tariff = &stored.tariff;
            (
                stored.input_semantics,
                stored.missing_cache_creation_is_zero,
                stored.cache_creation_mode,
                tariff.revision,
                tariff.uses_cache_read,
                tariff.uses_cache_write,
                tariff.uses_cache_write_1h,
                tariff.uses_cache_write_12h,
            )
        })
    };
    signature(primary) == signature(next)
}

impl Bridge {
    pub(in crate::model_http) fn select_with_fallbacks(
        &self,
        context: &RequestContext,
        request: &Request,
    ) -> Result<Route, RelayError> {
        let (group, pending) = {
            let mut active = self
                .active
                .lock()
                .map_err(|_| RelayError::BillingUnavailable)?;
            let op = active
                .get_mut(&context.request_id)
                .ok_or(RelayError::Forbidden)?;
            if op.selected.is_some() {
                return Err(RelayError::Conflict);
            }
            (op.group.clone(), op.route_plan.take())
        };
        let plans = match pending {
            Some(Pending::Planned(plans)) => plans,
            Some(Pending::Active(_)) => return Err(RelayError::Conflict),
            None => vec![
                self.routing
                    .route_adapted(RouteRequest {
                        model: &request.model,
                        group: &group,
                        speed: None,
                        protocol: client_protocol(request.client),
                        replay: ReplayPermission::Denied,
                    })
                    .map_err(|_| RelayError::RouteUnavailable)?,
            ],
        };
        let mut plans: VecDeque<_> = plans.into();
        let attempt_limit = plans
            .front()
            .and_then(RoutePlan::global_attempt_limit)
            .unwrap_or(8);
        // Request-specific bounds are not part of the routing snapshot. Reject
        // incompatible candidates before consuming any actual dispatch attempts.
        let mut selected_route = None;
        'select: for plan in &plans {
            for attempt in plan
                .candidate_attempts()
                .into_iter()
                .filter(Attempt::is_available)
            {
                match self.route_for_attempt(Arc::new(attempt), request) {
                    Ok(candidate) => {
                        selected_route = Some(candidate);
                        break 'select;
                    }
                    Err(RelayError::RouteUnavailable) => continue,
                    Err(error) => return Err(error),
                }
            }
        }
        let (selected, route) = selected_route.ok_or(RelayError::RouteUnavailable)?;
        let primary = binding(self, &selected, &route, request)?;
        let mut candidates = BTreeMap::new();
        candidates.insert(
            route.id.clone(),
            Candidate {
                selected: selected.clone(),
                route: route.clone(),
                binding: primary.clone(),
            },
        );
        // Exact-count evidence is tied to one generator. A different tokenizer
        // or normalization contract cannot inherit that paid reservation.
        if matches!(
            route.count_plan,
            crate::relay::input_count::CountPlan::ContextMax
        ) && selected.price.upstream_cost_group_ppm.is_none()
        {
            for plan in &plans {
                let mut remaining = plan.planned_attempts().len();
                for attempt in plan.candidate_attempts() {
                    if candidates.len() >= usize::from(attempt_limit) || remaining == 0 {
                        break;
                    }
                    let id = attempt.route_key();
                    if candidates.contains_key(&id) {
                        remaining -= 1;
                        continue;
                    }
                    let Ok((candidate, next)) = self.route_for_attempt(Arc::new(attempt), request)
                    else {
                        continue;
                    };
                    let official = selected.attempt.quote().is_bound_official_flat()
                        || candidate.attempt.quote().is_bound_official_flat();
                    if next.protocol != route.protocol
                        || (official
                            && !compatible_official_meter(&selected.price, &candidate.price))
                        || !matches!(
                            next.count_plan,
                            crate::relay::input_count::CountPlan::ContextMax
                        )
                        || candidate.input_token_ceiling > selected.input_token_ceiling
                        || candidate.price.grounding.as_ref().map(|p| &p.contract)
                            != selected.price.grounding.as_ref().map(|p| &p.contract)
                    {
                        continue;
                    }
                    let Ok(binding) = binding(self, &candidate, &next, request) else {
                        continue;
                    };
                    if binding.semantics != primary.semantics {
                        continue;
                    }
                    candidates.insert(
                        id,
                        Candidate {
                            selected: candidate,
                            route: next,
                            binding,
                        },
                    );
                    remaining -= 1;
                }
            }
        }
        let admitted: HashSet<_> = candidates.keys().cloned().collect();
        for plan in &mut plans {
            plan.retain_dispatches(&admitted)
                .map_err(|_| RelayError::Conflict)?;
        }
        plans.retain(|plan| !plan.planned_attempts().is_empty());
        let first = plans
            .front_mut()
            .ok_or(RelayError::RouteUnavailable)?
            .start()
            .map_err(|_| RelayError::RouteUnavailable)?;
        if first.route_key() != selected.route_id {
            return Err(RelayError::RouteUnavailable);
        }
        let mut active = self
            .active
            .lock()
            .map_err(|_| RelayError::BillingUnavailable)?;
        let op = active
            .get_mut(&context.request_id)
            .ok_or(RelayError::Forbidden)?;
        op.selected = Some(selected);
        op.route_plan = Some(Pending::Active(Session {
            plans,
            candidates,
            primary: route.id.clone(),
            number: 1,
            attempt_limit,
            inbound_protocol: request.client,
        }));
        Ok(route)
    }

    pub(crate) fn prepared_with_route_fallbacks(
        &self,
        id: &str,
        prepared: PreparedReservation,
    ) -> Result<PreparedReservation, RelayError> {
        let active = self
            .active
            .lock()
            .map_err(|_| RelayError::BillingUnavailable)?;
        let op = active.get(id).ok_or(RelayError::Forbidden)?;
        let fallbacks = match &op.route_plan {
            Some(Pending::Active(session)) => session
                .candidates
                .iter()
                .filter(|(id, _)| **id != session.primary)
                .map(|(_, candidate)| candidate.binding.clone())
                .collect(),
            _ => Vec::new(),
        };
        prepared
            .with_fallbacks(fallbacks)
            .map_err(super::super::billing::error)
    }

    pub(super) async fn record_selected_dispatch(
        &self,
        context: &RequestContext,
        route: &Route,
        body: &[u8],
    ) -> Result<(), RelayError> {
        let (worker, number, inbound, selected, secret, peer, issuer) = {
            let active = self
                .active
                .lock()
                .map_err(|_| RelayError::BillingUnavailable)?;
            let op = active
                .get(&context.request_id)
                .ok_or(RelayError::Forbidden)?;
            let (number, inbound) = match &op.route_plan {
                Some(Pending::Active(session)) => (session.number, Some(session.inbound_protocol)),
                _ => (1, None),
            };
            (
                op.worker,
                number,
                inbound,
                op.selected.clone().ok_or(RelayError::RouteUnavailable)?,
                op.secret.clone(),
                op.peer,
                op.oauth_issuer.clone(),
            )
        };
        if route.id != selected.route_id || !selected.attempt.is_available() {
            return Err(RelayError::RouteUnavailable);
        }
        if let Some(source) = &context.oauth_source {
            let issuer = issuer.ok_or(RelayError::Forbidden)?;
            let mut tx = self
                .pool
                .begin()
                .await
                .map_err(|_| RelayError::BillingUnavailable)?;
            let current = issuer
                .authorize_model_in_transaction(
                    &mut tx,
                    &secret,
                    Some(&selected.price.model),
                    Some(selected.attempt.group()),
                )
                .await
                .map_err(super::super::identity_error)?;
            if &current != source {
                return Err(RelayError::Forbidden);
            }
            tx.commit()
                .await
                .map_err(|_| RelayError::BillingUnavailable)?;
        } else {
            let identity = crate::identity::IdentityStore::from_pool(self.pool.clone());
            let current = identity
                .authorize_model_request(
                    &secret,
                    &selected.price.model,
                    peer,
                    selected.attempt.group(),
                )
                .await
                .map_err(super::super::identity_error)?;
            if current.user_id != context.actor_user_id
                || Some(current.credential_id) != context.key_id
            {
                return Err(RelayError::Forbidden);
            }
            if selected.attempt.group() != selected.price.group
                && !identity
                    .api_key_cross_group_retry(&secret)
                    .await
                    .map_err(super::super::identity_error)?
            {
                return Err(RelayError::Forbidden);
            }
        }
        crate::request_limits::admit(
            &self.pool,
            context.actor_user_id,
            selected.attempt.group(),
            &context.request_id,
        )
        .await
        .map_err(super::admission_error)?;
        let wire_sha256 = contract::digest(body);
        super::super::billing::retry(|| {
            self.charging.record_observed_dispatch(
                &context.request_id,
                &worker,
                number,
                &route.id,
                selected.attempt.group(),
                (&wire_sha256, inbound),
            )
        })
        .await
        .map_err(super::super::billing::error)
    }

    pub(super) async fn retry_selected_route(
        &self,
        context: &RequestContext,
        _request: &Request,
        route: &Route,
        failure: crate::relay::AttemptFailure,
    ) -> Result<Option<Route>, RelayError> {
        let (worker, number) = {
            let active = self
                .active
                .lock()
                .map_err(|_| RelayError::BillingUnavailable)?;
            let op = active
                .get(&context.request_id)
                .ok_or(RelayError::Forbidden)?;
            (
                op.worker,
                match &op.route_plan {
                    Some(Pending::Active(session)) => session.number,
                    _ => 1,
                },
            )
        };
        self.charging
            .record_route_failure(&context.request_id, &worker, number, &route.id, &failure)
            .await
            .map_err(super::super::billing::error)?;
        if !matches!(
            failure.failure.delivery,
            DeliveryState::NotSent | DeliveryState::Rejected
        ) {
            return Ok(None);
        }
        let mut active = self
            .active
            .lock()
            .map_err(|_| RelayError::BillingUnavailable)?;
        let op = active
            .get_mut(&context.request_id)
            .ok_or(RelayError::Forbidden)?;
        if op
            .route_plan
            .as_ref()
            .and_then(Pending::affinity_binding)
            .is_some_and(|binding| binding.skip_retry)
        {
            return Ok(None);
        }
        let Some(Pending::Active(session)) = &mut op.route_plan else {
            return Ok(None);
        };
        let Some(next) = session.next(failure.failure, failure.http_status)? else {
            return Ok(None);
        };
        let candidate = session.candidates.get(&next).ok_or(RelayError::Conflict)?;
        let previous = op.selected.as_ref().ok_or(RelayError::RouteUnavailable)?;
        let mut selected = candidate.selected.clone();
        selected.price = previous.price.clone();
        selected.quote = previous.quote;
        op.selected = Some(selected);
        Ok(Some(candidate.route.clone()))
    }
}

#[cfg(test)]
mod mixed_official_meter_tests {
    use super::compatible_official_meter;
    use crate::{billing::charging::Price, routing::InputSemantics};
    use serde_json::json;

    #[test]
    fn retry_compares_frozen_evidence_contracts_even_when_prices_are_free() {
        let zero = json!({"numerator":0,"denominator":1});
        let input = json!({"numerator":2,"denominator":1});
        let primary: Price = serde_json::from_value(json!({
            "model":"metered","group":"default","version":"routing-1-rule-1",
            "input_per_million":0,"cached_per_million":0,"output_per_million":0,
            "tiered":{"input_semantics":"inclusive","group_multiplier_ppm":1_000_000,
            "speed_multiplier_ppm":1_000_000,"credits_per_usd":500_000,"tariff":{
              "revision":2,"source_digest":"12".repeat(32),"uses_cache_read":true,
              "uses_cache_write":true,"uses_cache_write_1h":true,"tiers":[{
                "up_to_input_tokens":null,"rates":{"input":input,"output":zero,"cache_read":zero,
                "cache_write":zero,"cache_write_1h":zero}
              }]
            }}
        }))
        .unwrap();
        primary.validate().unwrap();
        let restored: Price =
            serde_json::from_slice(&serde_json::to_vec(&primary).unwrap()).unwrap();
        assert!(compatible_official_meter(&primary, &restored));

        // Each alternative is a valid tariff; none can inherit the primary's
        // interpretation of missing counters or its measurement categories.
        for difference in 0..5 {
            let mut candidate = primary.clone();
            let stored = candidate.tiered.as_mut().unwrap();
            match difference {
                0 => stored.missing_cache_creation_is_zero = true,
                1 => stored.input_semantics = InputSemantics::AnthropicText,
                2 => stored.tariff.uses_cache_read = false,
                3 => {
                    stored.tariff.uses_cache_write_1h = false;
                    stored.tariff.tiers[0].rates.cache_write_1h = None;
                }
                _ => {
                    stored.tariff.revision = 4;
                    stored.tariff.uses_cache_write_1h = false;
                    stored.tariff.uses_cache_write_12h = true;
                    let rates = &mut stored.tariff.tiers[0].rates;
                    rates.cache_write_12h = rates.cache_write_1h.take();
                }
            }
            candidate.validate().unwrap();
            assert!(!compatible_official_meter(&primary, &candidate));
            assert!(!compatible_official_meter(&candidate, &primary));
        }
    }
}

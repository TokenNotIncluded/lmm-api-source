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
}

fn route_id(attempt: &Attempt) -> String {
    format!(
        "r{}-t{}-u{}",
        attempt.route_id(),
        attempt.target_id(),
        attempt.upstream_id()
    )
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
        binding: ReservationBinding {
            route_id: route.id.clone(),
            target_id: selected.attempt.target_id(),
            upstream_id: selected.attempt.upstream_id(),
            protocol: match route.protocol {
                UpstreamProtocol::Chat => "chat",
                UpstreamProtocol::Responses => "responses",
                UpstreamProtocol::Anthropic => "anthropic",
                UpstreamProtocol::Gemini => "gemini",
            }
            .into(),
            model_snapshot: route.model.clone(),
            generation_url: route.endpoint.as_str().into(),
            credential_ref: selected.attempt.credential_ref(),
            payload_policy_sha256: contract::typed_digest(&selected.attempt.payload_policy())
                .map_err(super::super::billing::error)?,
            provider_contract_revision: revision,
            projection_revision: "context-max-v1".into(),
            quote_version: route.price_version,
        },
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

impl Session {
    fn next(&mut self, failure: crate::routing::Failure) -> Result<Option<String>, RelayError> {
        if self.number >= 8 {
            return Ok(None);
        }
        let Some(current) = self.plans.front_mut() else {
            return Ok(None);
        };
        if !current.allows_failure(failure) {
            return Ok(None);
        }
        let attempt = match current.retry(failure) {
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
            let id = route_id(&attempt);
            if !self.candidates.contains_key(&id) {
                return Err(RelayError::Conflict);
            }
            return Ok(Some(id));
        }
        Ok(None)
    }
}

impl Bridge {
    pub(super) fn select_with_fallbacks(
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
        let first = plans
            .front()
            .ok_or(RelayError::RouteUnavailable)?
            .planned_attempts()
            .into_iter()
            .find(Attempt::is_available)
            .ok_or(RelayError::RouteUnavailable)?;
        let (selected, route) = self.route_for_attempt(Arc::new(first), request)?;
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
                for attempt in plan.planned_attempts() {
                    if candidates.len() >= 8 {
                        break;
                    }
                    let id = route_id(&attempt);
                    if candidates.contains_key(&id) {
                        continue;
                    }
                    let Ok((candidate, next)) = self.route_for_attempt(Arc::new(attempt), request)
                    else {
                        continue;
                    };
                    if next.protocol != route.protocol
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
                }
            }
        }
        let targets: HashSet<_> = candidates
            .values()
            .map(|candidate| candidate.selected.attempt.target_id())
            .collect();
        for plan in &mut plans {
            plan.retain_targets(&targets)
                .map_err(|_| RelayError::Conflict)?;
        }
        plans.retain(|plan| !plan.planned_attempts().is_empty());
        let first = plans
            .front_mut()
            .ok_or(RelayError::RouteUnavailable)?
            .start()
            .map_err(|_| RelayError::RouteUnavailable)?;
        if route_id(&first) != selected.route_id {
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
        let (worker, number, selected, secret, peer, issuer) = {
            let active = self
                .active
                .lock()
                .map_err(|_| RelayError::BillingUnavailable)?;
            let op = active
                .get(&context.request_id)
                .ok_or(RelayError::Forbidden)?;
            let number = match &op.route_plan {
                Some(Pending::Active(session)) => session.number,
                _ => 1,
            };
            (
                op.worker,
                number,
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
        self.charging
            .record_route_dispatch(
                &context.request_id,
                &worker,
                number,
                &route.id,
                selected.attempt.group(),
                &contract::digest(body),
            )
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
        let Some(Pending::Active(session)) = &mut op.route_plan else {
            return Ok(None);
        };
        let Some(next) = session.next(failure.failure)? else {
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

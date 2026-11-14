//! Test-only observations of the executing immutable plan. No routing or RNG.
use super::*;
use crate::billing::charging;
use serde::Serialize;
use serde_json::Value;
#[derive(Clone, Debug, PartialEq, Serialize)]
pub(super) struct AttemptTrace {
    pub(super) route_id: u64,
    pub(super) target_id: u64,
    pub(super) upstream_id: u64,
    pub(super) route_key: String,
    pub(super) binding: Value,
}
#[derive(Clone, Debug, Serialize)]
pub(super) struct PlanTrace {
    pub(super) group: String,
    pub(super) before: Vec<AttemptTrace>,
    pub(super) after: Vec<AttemptTrace>,
    pub(super) selected: Option<AttemptTrace>,
}

type Traces = Arc<Mutex<Vec<PlanTrace>>>;
fn identity(attempt: &crate::routing::Attempt) -> AttemptTrace {
    AttemptTrace {
        route_id: attempt.route_id(),
        target_id: attempt.target_id(),
        upstream_id: attempt.upstream_id(),
        route_key: attempt.route_key(),
        binding: serde_json::json!({"route_id":attempt.route_key(),"target_id":attempt.target_id(),"upstream_id":attempt.upstream_id()}),
    }
}
impl ModelGateway {
    pub(super) fn with_continuity_plan_observer(self, traces: Traces) -> Self {
        *self.bridge.review_plan_traces.lock().unwrap() = Some(traces);
        self
    }
}
impl Bridge {
    fn review_traces(&self) -> Option<Traces> {
        self.review_plan_traces.lock().unwrap().clone()
    }
    pub(super) fn observe_plan_before(&self, plan: &crate::routing::RoutePlan) -> Option<usize> {
        let traces = self.review_traces()?;
        let mut traces = traces.lock().unwrap();
        let index = traces.len();
        traces.push(PlanTrace {
            group: plan.group().into(),
            before: plan.candidate_attempts().iter().map(identity).collect(),
            after: Vec::new(),
            selected: None,
        });
        Some(index)
    }
    pub(super) fn observe_plan_after(
        &self,
        index: Option<usize>,
        plan: &crate::routing::RoutePlan,
    ) {
        if let (Some(index), Some(traces)) = (index, self.review_traces()) {
            traces.lock().unwrap()[index].after =
                plan.candidate_attempts().iter().map(identity).collect();
        }
    }
    pub(super) fn observe_prepared_binding(
        &self,
        index: Option<usize>,
        attempt: &crate::routing::Attempt,
        binding: &charging::contract::ReservationBinding,
    ) {
        if let (Some(index), Some(traces)) = (index, self.review_traces()) {
            let mut selected = identity(attempt);
            selected.binding = serde_json::to_value(binding).unwrap();
            let mut traces = traces.lock().unwrap();
            let trace = &mut traces[index];
            // The selected canonical binding is supplied by the actual preparer
            // before persistence; it is never reconstructed from database rows.
            for item in trace.before.iter_mut().chain(trace.after.iter_mut()) {
                if item.route_key == selected.route_key {
                    *item = selected.clone();
                }
            }
            trace.selected = Some(selected);
        }
    }
}

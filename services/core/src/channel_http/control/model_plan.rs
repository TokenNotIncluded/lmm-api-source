//! Read-only channel model simulation shared by the effective-state Web view
//! and the operator's Git proposal CLI. No database, vault or provider writes.
use super::*;
use crate::routing::{
    FailoverPolicy, MAX_ATTEMPTS, MAX_TARGETS_PER_ROUTE, ModelRoute, PayloadPolicy, RetryCause,
    RouteTarget, RoutingConfig, Speed, valid_model,
};
use std::collections::BTreeMap;

#[derive(Clone, Copy, Default, Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
enum Placement {
    #[default]
    Backup,
    Primary,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct NewRoute {
    id: u64,
    model: String,
    group: String,
    speeds: Vec<Speed>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Attachment {
    route_id: u64,
    upstream_model: String,
    #[serde(default)]
    placement: Option<Placement>,
    priority: u16,
    weight: u32,
    speeds: Vec<Speed>,
    timeout_ms: u32,
    payload: PayloadPolicy,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(in crate::channel_http) struct Input {
    expected_version: Version,
    channel_id: u64,
    #[serde(default)]
    new_routes: Vec<NewRoute>,
    attachments: Vec<Attachment>,
    #[serde(default)]
    remove_route_ids: Vec<u64>,
}

#[derive(Serialize)]
struct Candidate {
    route_id: u64,
    model: String,
    group: String,
    changed: bool,
    target: RouteTarget,
    failover: FailoverPolicy,
    previous_failover: Option<FailoverPolicy>,
}

#[derive(Serialize)]
struct Preview {
    version: Version,
    channel_id: u64,
    ready: bool,
    saved: bool,
    scope: &'static str,
    missing_prices: Vec<Value>,
    candidates: Vec<Candidate>,
    draft_bindings: Vec<Value>,
    candidate_version: Option<Version>,
    candidate_routing: Option<RoutingConfig>,
    remove_route_ids: Vec<u64>,
}

/// Stable, sanitized offline and HTTP error. Input text and credentials are
/// never copied from parser diagnostics.
#[derive(Debug, Serialize)]
pub struct ChannelModelPlanError {
    pub code: &'static str,
    pub message: &'static str,
    pub data: Value,
}

impl std::fmt::Display for ChannelModelPlanError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}: {}", self.code, self.message)
    }
}

impl std::error::Error for ChannelModelPlanError {}

fn invalid() -> ChannelModelPlanError {
    ChannelModelPlanError {
        code: "CHANNEL_CONTROL_INVALID",
        message: "Invalid model routing proposal",
        data: Value::Null,
    }
}

fn placement_error(route_id: u64, reason: &'static str) -> ChannelModelPlanError {
    ChannelModelPlanError {
        code: "CHANNEL_PLACEMENT_UNAVAILABLE",
        message: "The selected placement cannot preserve the existing routing order",
        data: json!({"route_id":route_id, "reason":reason}),
    }
}

/// Produce a configuration candidate from credential references already present
/// in this source release. The CLI receives the full candidate in memory. The
/// HTTP projection omits the full routing configuration.
pub fn preview_channel_model_plan(
    document: &ReleaseDocument,
    selection: Value,
) -> Result<Value, ChannelModelPlanError> {
    let input: Input = serde_json::from_value(selection).map_err(|_| invalid())?;
    serde_json::to_value(simulate(document, input)?).map_err(|_| invalid())
}

pub(in crate::channel_http) async fn preview(
    State(state): State<ChannelState>,
    headers: HeaderMap,
    Json(input): Json<Input>,
) -> Result<Json<Value>, Failure> {
    super::super::admit(&state, &headers).await?;
    let document = state.routing.catalog_document().map_err(|_| ManagementError::Storage)?;
    let mut result = simulate(&document, input).map_err(Failure::ModelPlan)?;
    result.candidate_routing = None;
    Ok(Json(json!({"success":true, "data":result})))
}

fn valid_id(id: u64) -> bool {
    id > 0 && id <= i64::MAX as u64
}

fn valid_speeds(speeds: &[Speed]) -> bool {
    !speeds.is_empty() && speeds.iter().copied().collect::<BTreeSet<_>>().len() == speeds.len()
}

fn place(
    route: &mut ModelRoute,
    attachment: &Attachment,
    enabled_upstreams: &BTreeSet<u64>,
) -> Result<u16, ChannelModelPlanError> {
    if route.targets.len() >= MAX_TARGETS_PER_ROUTE {
        return Err(placement_error(route.id, "target_limit"));
    }
    if route.targets.is_empty() {
        return Ok(0);
    }

    let placement = attachment.placement.unwrap_or_default();
    let priority = match placement {
        Placement::Backup => route.targets.iter().map(|t| t.priority).max().and_then(|p| p.checked_add(1)),
        Placement::Primary => route.targets.iter().map(|t| t.priority).min().and_then(|p| p.checked_sub(1)),
    }.ok_or_else(|| placement_error(route.id, "priority_exhausted"))?;
    // Count each speed independently. Health and request-specific protocol
    // filters may remove earlier candidates, never add an earlier candidate.
    let eligible = attachment.speeds.iter().map(|speed| {
        route.targets.iter().filter(|t| {
            t.enabled && t.weight > 0 && enabled_upstreams.contains(&t.upstream_id)
                && t.speeds.contains(speed)
        }).count()
    }).max().unwrap_or(0);
    let attempts = match placement {
        Placement::Backup => 1 + eligible,
        Placement::Primary => 1 + usize::from(eligible > 0),
    };
    if attempts > usize::from(MAX_ATTEMPTS) {
        return Err(placement_error(route.id, "attempt_limit"));
    }
    if attempts > 1 {
        route.failover.max_attempts = route.failover.max_attempts.max(attempts as u8);
        if route.failover.retry_on.is_empty() {
            route.failover.retry_on.push(RetryCause::Connect);
        }
    }
    Ok(priority)
}

fn simulate(source: &ReleaseDocument, input: Input) -> Result<Preview, ChannelModelPlanError> {
    let version = Version::of(source);
    if version != input.expected_version {
        return Err(ChannelModelPlanError {
            code:"CHANNEL_VERSION_CONFLICT",
            message:"The effective routing or pricing version changed; recompute the proposal",
            data:json!({"expected":input.expected_version,"effective":version}),
        });
    }
    if !valid_id(input.channel_id) || input.new_routes.len() > 1000
        || input.attachments.len() > 1000 || input.remove_route_ids.len() > 1000 {
        return Err(invalid());
    }
    let selected: BTreeSet<_> = input.attachments.iter().map(|a| a.route_id).collect();
    let confirmed: BTreeSet<_> = input.remove_route_ids.iter().copied().collect();
    if selected.len() != input.attachments.len() || confirmed.len() != input.remove_route_ids.len() {
        return Err(invalid());
    }

    let mut document = source.clone();
    let routing = &mut document.routing;
    let removed: BTreeSet<_> = routing.routes.iter()
        .filter(|r| !selected.contains(&r.id) && r.targets.iter().any(|t| t.upstream_id == input.channel_id))
        .map(|r| r.id).collect();
    if removed != confirmed {
        return Err(ChannelModelPlanError {
            code: "CHANNEL_MODEL_REMOVAL_CONFIRMATION_REQUIRED",
            message: "Confirm every removed channel binding by route ID",
            data: json!({"route_ids":removed}),
        });
    }

    let groups: BTreeSet<_> = routing.groups.iter().map(|g| g.name.as_str()).collect();
    let mut route_ids: BTreeSet<_> = routing.routes.iter().map(|r| r.id).collect();
    let mut names: BTreeSet<_> = routing.routes.iter().map(|r| (r.group.clone(), r.model.clone()))
        .chain(routing.aliases.iter().map(|a| (a.group.clone(), a.alias.clone()))).collect();
    let mut created = BTreeSet::new();
    for new in input.new_routes {
        if !valid_id(new.id) || !valid_model(&new.model) || !groups.contains(new.group.as_str())
            || !valid_speeds(&new.speeds) || !selected.contains(&new.id)
            || !route_ids.insert(new.id) || !names.insert((new.group.clone(), new.model.clone())) {
            return Err(invalid());
        }
        created.insert(new.id);
        routing.routes.push(ModelRoute {
            id: new.id, group: new.group, model: new.model, speeds: new.speeds,
            enabled: true, targets: Vec::new(), failover: FailoverPolicy::default(),
        });
    }

    let enabled_upstreams = routing.upstreams.iter().filter(|u| u.enabled).map(|u| u.id).collect();
    let mut next = routing.routes.iter().flat_map(|r| &r.targets).map(|t| t.id).max().unwrap_or(0);
    let prices = PriceSnapshot::compile(document.pricing.clone()).map_err(|_| invalid())?;
    let mut routes: BTreeMap<_, _> = routing.routes.iter_mut().map(|r| (r.id, r)).collect();
    let mut missing_prices = Vec::new();
    let mut candidates = Vec::new();
    let mut draft_bindings = Vec::new();
    let mut excluded = BTreeSet::new();
    for attachment in input.attachments {
        let route = routes.get_mut(&attachment.route_id).ok_or_else(invalid)?;
        if !valid_model(&attachment.upstream_model) || !valid_speeds(&attachment.speeds)
            || !attachment.speeds.iter().all(|s| route.speeds.contains(s))
            || !(1..=3_600_000).contains(&attachment.timeout_ms) {
            return Err(invalid());
        }

        // Protect all policy, pin identities and source-model mappings of a
        // retained binding. The model-specific operation can only add a speed.
        let old = route.targets.iter().find(|t| t.upstream_id == input.channel_id).cloned();
        if let Some(t) = &old {
            if attachment.placement.is_some() || t.priority != attachment.priority
                || t.weight != attachment.weight || t.timeout_ms != attachment.timeout_ms
                || t.payload != attachment.payload || t.upstream_model != attachment.upstream_model
                || !t.speeds.iter().all(|s| attachment.speeds.contains(s)) {
                return Err(placement_error(route.id, "existing_policy"));
            }
        } else if attachment.weight == 0 {
            return Err(placement_error(route.id, "zero_weight"));
        }
        let check = if route.enabled { &route.speeds } else { &attachment.speeds };
        let gaps: Vec<_> = check.iter()
            .filter(|&&speed| prices.template(&route.model, &route.group, speed).is_none())
            .map(|speed| json!({"model":route.model,"group":route.group,"speed":speed})).collect();
        if !gaps.is_empty() {
            draft_bindings.push(json!({
                "route_id":route.id, "model":route.model, "group":route.group,
                "upstream_model":attachment.upstream_model, "speeds":attachment.speeds,
                "placement":attachment.placement.unwrap_or_default(), "missing_prices":gaps
            }));
            missing_prices.extend(gaps);
            if created.contains(&route.id) {
                excluded.insert(route.id);
            }
            continue;
        }

        let previous_failover = (!created.contains(&route.id)).then(|| route.failover.clone());
        let (id, enabled, priority, client_protocols) = if let Some(t) = &old {
            (t.id, t.enabled, t.priority, t.client_protocols.clone())
        } else {
            let priority = place(route, &attachment, &enabled_upstreams)?;
            next = next.checked_add(1).filter(|id| valid_id(*id)).ok_or_else(invalid)?;
            (next, true, priority, None)
        };
        let target = RouteTarget {
            id, enabled, upstream_id:input.channel_id, upstream_model:attachment.upstream_model,
            priority, weight:attachment.weight, speeds:attachment.speeds, client_protocols,
            payload:attachment.payload, timeout_ms:attachment.timeout_ms,
        };
        let changed = old.as_ref() != Some(&target);
        if let Some(t) = route.targets.iter_mut().find(|t| t.upstream_id == input.channel_id) {
            *t = target.clone();
        } else {
            route.targets.push(target.clone());
        }
        candidates.push(Candidate {
            route_id:route.id, model:route.model.clone(), group:route.group.clone(),
            changed, target, failover:route.failover.clone(), previous_failover,
        });
    }
    for route_id in &confirmed {
        if let Some(route) = routes.get_mut(route_id) {
            route.targets.retain(|t| t.upstream_id != input.channel_id);
        }
    }
    routing.routes.retain(|r| !excluded.contains(&r.id));
    let existing_channel = routing.upstreams.iter().any(|u| u.id == input.channel_id);
    let candidate_version = if existing_channel {
        if *routing != source.routing {
            routing.version = routing.version.checked_add(1).filter(|id| valid_id(*id)).ok_or_else(invalid)?;
        }
        crate::model_http::channel_operations::validate_publication_prices(&document).map_err(|_| invalid())?;
        document.clone().into_router().map_err(|_| invalid())?;
        Some(Version::of(&document))
    } else {
        None
    };
    Ok(Preview {
        version, channel_id:input.channel_id, ready:missing_prices.is_empty(), saved:false, scope:"routing_prices",
        missing_prices, candidates, draft_bindings, candidate_version,
        candidate_routing:existing_channel.then_some(document.routing), remove_route_ids:input.remove_route_ids,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::routing::{self, DeliveryState, FailureKind, Protocol, ReplayPermission, RouteRequest};

    fn fixture() -> ReleaseDocument {
        let (routing, mut pricing) = crate::channel_http::tests::fixture();
        for rule in &mut pricing.rules {
            rule.rates.input = 2_000_000;
            rule.rates.output = 4_000_000;
        }
        ReleaseDocument { contract_version: crate::routing::CONTRACT_VERSION, routing, pricing }
    }

    fn binding(id: u64, model: &str) -> Value {
        json!({"route_id":id, "upstream_model":model, "priority":0, "weight":1,
            "speeds":["standard"], "timeout_ms":30000, "payload":"adapt"})
    }

    fn selection(id: u64) -> Value {
        json!({"expected_version":{"routing":1,"pricing":1},"channel_id":id,
            "new_routes":[], "attachments":[binding(1, "same-source")]})
    }

    fn proposed(doc: &ReleaseDocument, result: &Value) -> ReleaseDocument {
        let mut candidate = doc.clone();
        candidate.routing = serde_json::from_value(result["candidate_routing"].clone()).unwrap();
        candidate
    }

    #[test]
    fn priced_backup_preserves_primary_and_actual_retry_order() {
        let doc = fixture();
        let result = preview_channel_model_plan(&doc, selection(2)).unwrap();
        assert_eq!(result["ready"], true);
        assert_eq!(result["saved"], false);
        assert_eq!(result["candidate_version"], json!({"routing":2,"pricing":1}));
        let candidate = proposed(&doc, &result);
        assert_eq!(candidate.pricing, doc.pricing);
        assert_eq!(candidate.routing.routes[0].targets[0], doc.routing.routes[0].targets[0]);
        let router = candidate.into_router().unwrap();
        let mut plan = router.route(RouteRequest {
            model:"model-a", group:"default", speed:None, protocol:Protocol::OpenAiChat,
            replay:ReplayPermission::Denied,
        }).unwrap();
        assert_eq!(plan.start().unwrap().upstream_id(), 1);
        assert_eq!(plan.retry(routing::Failure { kind:FailureKind::Connect, delivery:DeliveryState::NotSent }).unwrap().upstream_id(),2);
        let mut unsafe_plan = router.route(RouteRequest {
            model:"model-a", group:"default", speed:None, protocol:Protocol::OpenAiChat,
            replay:ReplayPermission::Denied,
        }).unwrap();
        unsafe_plan.start().unwrap();
        assert!(unsafe_plan.retry(routing::Failure { kind:FailureKind::Connect, delivery:DeliveryState::PossiblyAccepted }).is_err());
        assert_eq!(doc.routing.routes[0].targets.len(), 1);
    }

    #[test]
    fn nine_models_in_two_groups_compile_only_seven_priced_models() {
        let mut doc = fixture();
        let initial_price = doc.pricing.rules[0].clone();
        let mut intent = selection(2);
        let mut new_routes = Vec::new();
        let mut attachments = Vec::new();
        for (n, group) in ["Enterprise","国产"].into_iter().enumerate() {
            doc.routing.groups.push(routing::ModelGroup {name:group.into(),enabled:true});
            for model_no in 1..=9 {
                let id = 100 + 10 * n as u64 + model_no;
                let model = format!("synthetic-{model_no}");
                new_routes.push(json!({"id":id,"model":model,"group":group,"speeds":["standard"]}));
                attachments.push(binding(id, &model));
                if model_no <= 7 {
                    let mut rule = initial_price.clone();
                    rule.id = id;
                    rule.model = model;
                    rule.group = group.into();
                    doc.pricing.rules.push(rule);
                }
            }
        }
        intent["new_routes"] = json!(new_routes);
        intent["attachments"] = json!(attachments);
        let result = preview_channel_model_plan(&doc, intent).unwrap();
        assert_eq!(result["candidates"].as_array().unwrap().len(), 14);
        assert_eq!(result["draft_bindings"].as_array().unwrap().len(), 4);
        assert_eq!(result["missing_prices"].as_array().unwrap().len(), 4);
        assert_eq!(result["ready"], false);
        let candidate = proposed(&doc, &result);
        assert_eq!(candidate.routing.routes.len(), 15);
        assert_eq!(candidate.routing.upstreams, doc.routing.upstreams);
        assert_eq!(candidate.pricing, doc.pricing);
        let router = candidate.into_router().unwrap();
        for group in ["Enterprise", "国产"] {
            let visible = router.catalog(group).unwrap();
            assert_eq!(visible.len(), 7);
            assert!(visible.iter().all(|model| model.name != "synthetic-8" && model.name != "synthetic-9"));
        }
    }

    #[test]
    fn attempts_priority_capacity_and_stale_versions_do_not_change_source() {
        let mut doc = fixture();
        let mut stale = selection(2);
        stale["expected_version"]["pricing"] = json!(2);
        assert_eq!(preview_channel_model_plan(&doc, stale).unwrap_err().code, "CHANNEL_VERSION_CONFLICT");
        let mut stale = selection(2);
        stale["expected_version"]["routing"] = json!(2);
        assert_eq!(preview_channel_model_plan(&doc, stale).unwrap_err().code, "CHANNEL_VERSION_CONFLICT");
        doc.routing.routes[0].targets[0].priority = u16::MAX;
        assert_eq!(preview_channel_model_plan(&doc, selection(2)).unwrap_err().data["reason"], "priority_exhausted");
        doc.routing.routes[0].targets[0].priority = 0;
        let mut primary = selection(2);
        primary["attachments"][0]["placement"] = json!("primary");
        assert_eq!(preview_channel_model_plan(&doc, primary).unwrap_err().data["reason"], "priority_exhausted");
        let old = doc.routing.routes[0].targets[0].clone();
        for id in 3..=9 {
            let mut target = old.clone();
            target.id = id;
            target.upstream_id = id;
            doc.routing.routes[0].targets.push(target);
        }
        assert_eq!(preview_channel_model_plan(&doc, selection(2)).unwrap_err().data["reason"], "attempt_limit");
    }

    #[test]
    fn disappearances_require_explicit_exact_confirmation() {
        let doc = fixture();
        let mut empty = selection(1);
        empty["attachments"] = json!([]);
        let err = preview_channel_model_plan(&doc, empty.clone()).unwrap_err();
        assert_eq!(err.code, "CHANNEL_MODEL_REMOVAL_CONFIRMATION_REQUIRED");
        assert_eq!(err.data["route_ids"],json!([1]));
        empty["remove_route_ids"] = json!([1]);
        let result = preview_channel_model_plan(&doc, empty).unwrap();
        assert_eq!(result["remove_route_ids"],json!([1]));
        assert!(proposed(&doc,&result).routing.routes[0].targets.is_empty());
        assert_eq!(doc.routing.routes[0].targets.len(), 1);
    }
}

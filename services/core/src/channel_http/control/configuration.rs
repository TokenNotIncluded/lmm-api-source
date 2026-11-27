//! Native control-plane documents and secret lifecycle. All mutations retain
//! the current Session/policy locks, version CAS and audit through one commit.
use super::*;
use crate::{
    model_http::ExternalModelContext,
    routing::{
        CredentialRef, Endpoint, PayloadPolicy, Protocol, RouteTarget, RoutingConfig, Speed,
    },
};
use rand::TryRng as _;
use rand::rngs::SysRng;
use std::collections::BTreeMap;

#[derive(Clone, Default, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub(in crate::channel_http) struct KeyPool {
    #[serde(default)]
    pub polling: bool,
    #[serde(default)]
    pub disabled: Vec<CredentialRef>,
}

#[derive(Clone, Default, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub(in crate::channel_http) struct Bindings {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub assistant_model: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub assistant_group: Option<String>,
    #[serde(default)]
    pub names: BTreeMap<u64, String>,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub key_pools: BTreeMap<u64, KeyPool>,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub tags: BTreeMap<u64, String>,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub channel_groups: BTreeMap<u64, BTreeSet<String>>,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub channel_providers: BTreeMap<u64, String>,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub channel_notes: BTreeMap<u64, String>,
    #[serde(default)]
    pub contexts: Vec<ExternalModelContext>,
    #[serde(default)]
    pub vendors: BTreeMap<u64, super::catalog::Vendor>,
    #[serde(default)]
    pub models: BTreeMap<u64, super::catalog::ModelMetadata>,
    #[serde(default)]
    pub discoveries: BTreeMap<u64, super::catalog::Discovery>,
    #[serde(default)]
    pub price_drafts: BTreeMap<u64, super::price_drafts::PriceDraft>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub official_prices: Option<super::official_prices::OfficialPrices>,
    #[serde(default)]
    pub balance_probes: BTreeMap<u64, super::balance::BalanceProbe>,
    /// Explicit imported/operator-supplied old type; never inferred from a URL.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub legacy_balance: BTreeMap<u64, super::balance::legacy::LegacyBalance>,
}
impl Bindings {
    pub(in crate::channel_http) fn validate(
        &self,
        _document: &ReleaseDocument,
    ) -> Result<(), ManagementError> {
        if self.names.len() > 100_000
            || self.tags.len() > 100_000
            || self.contexts.len() > 100_000
            || self
                .names
                .iter()
                .any(|(id, name)| *id == 0 || *id > i64::MAX as u64 || !valid_label(name))
        {
            return Err(ManagementError::Invalid);
        }
        let upstream_ids = _document
            .routing
            .upstreams
            .iter()
            .map(|upstream| upstream.id)
            .collect::<BTreeSet<_>>();
        let groups: BTreeSet<_> = _document
            .routing
            .groups
            .iter()
            .map(|g| g.name.as_str())
            .collect();
        for (id, assigned) in &self.channel_groups {
            if !upstream_ids.contains(id)
                || assigned.is_empty()
                || assigned
                    .iter()
                    .any(|group| !groups.contains(group.as_str()))
            {
                return Err(ManagementError::Invalid);
            }
        }
        for (id, tag) in &self.tags {
            if !valid_label(tag) || !upstream_ids.contains(id) {
                return Err(ManagementError::Invalid);
            }
        }
        for (id, note) in &self.channel_notes {
            if !upstream_ids.contains(id) || !valid_note(note) {
                return Err(ManagementError::Invalid);
            }
        }
        for (id, provider) in &self.channel_providers {
            if !upstream_ids.contains(id)
                || !super::provider_presets::PROVIDERS
                    .iter()
                    .any(|preset| preset.id == provider)
            {
                return Err(ManagementError::Invalid);
            }
        }
        for (id, pool) in &self.key_pools {
            let upstream = _document
                .routing
                .upstreams
                .iter()
                .find(|upstream| upstream.id == *id)
                .ok_or(ManagementError::Invalid)?;
            let unique = pool
                .disabled
                .iter()
                .map(|reference| (reference.id, reference.revision))
                .collect::<BTreeSet<_>>();
            if pool.disabled.len() > 1000
                || unique.len() != pool.disabled.len()
                || pool
                    .disabled
                    .iter()
                    .any(|reference| reference.id == 0 || reference.revision == 0)
                || (!pool.polling && !pool.disabled.is_empty())
                || pool.disabled.contains(&upstream.credential)
            {
                return Err(ManagementError::Invalid);
            }
        }
        if self.price_drafts.len() > 16
            || serde_json::to_vec(self)
                .map_err(|_| ManagementError::Invalid)?
                .len()
                > 16_000_000
        {
            return Err(ManagementError::Invalid);
        }
        for (id, draft) in &self.price_drafts {
            if *id != draft.id {
                return Err(ManagementError::Invalid);
            }
            draft.validate()?;
        }
        for (id, probe) in &self.balance_probes {
            if !_document
                .routing
                .upstreams
                .iter()
                .any(|upstream| upstream.id == *id)
            {
                return Err(ManagementError::Invalid);
            }
            probe.validate()?;
        }
        for (id, legacy) in &self.legacy_balance {
            if !_document
                .routing
                .upstreams
                .iter()
                .any(|upstream| upstream.id == *id)
            {
                return Err(ManagementError::Invalid);
            }
            legacy.validate()?;
        }
        super::catalog::validate_metadata(self, _document)?;
        Ok(())
    }
}
pub(super) fn valid_label(name: &str) -> bool {
    !name.trim().is_empty() && name.len() <= 128 && !name.chars().any(char::is_control)
}

fn valid_note(note: &str) -> bool {
    note.len() <= 4096
        && !note
            .chars()
            .any(|c| c.is_control() && c != '\n' && c != '\t')
}

#[derive(Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub(in crate::channel_http) struct ConfigurationInput {
    expected_version: Version,
    routing: RoutingConfig,
    prices: PriceConfig,
    bindings: Bindings,
    #[serde(default)]
    confirmation_digest: Option<String>,
}
impl ConfigurationInput {
    fn prepare(
        &mut self,
        management: &ManagedChannels,
    ) -> Result<ReleaseDocument, ManagementError> {
        let current = next_document(management, self.expected_version)?;
        if self.routing.version != current.routing.version
            || self.prices.version < current.pricing.version
        {
            return Err(ManagementError::Invalid);
        }
        let previous = management.current_bindings()?;
        // Only the versioned channel update edits labels; older full-document
        // clients must preserve labels for every channel that survives the edit.
        let upstream_ids: BTreeSet<_> = self
            .routing
            .upstreams
            .iter()
            .map(|upstream| upstream.id)
            .collect();
        self.bindings
            .channel_groups
            .clone_from(&previous.channel_groups);
        self.bindings
            .channel_groups
            .retain(|id, _| upstream_ids.contains(id));
        if (self.bindings.assistant_group.is_some()
            && self.bindings.assistant_group != previous.assistant_group)
            || (self.bindings.assistant_model.is_some()
                && self.bindings.assistant_model != previous.assistant_model)
        {
            return Err(ManagementError::Conflict);
        }
        self.bindings.assistant_group = previous.assistant_group;
        self.bindings.assistant_model = previous.assistant_model;

        // Only the channel-specific endpoint edits pending selections. Carry
        // them through full-document saves from clients predating the field.
        for discovery in self.bindings.discoveries.values_mut() {
            discovery.pending_models.clear();
        }
        let previous_upstreams: BTreeMap<_, _> = current
            .routing
            .upstreams
            .iter()
            .map(|upstream| (upstream.id, upstream))
            .collect();
        for upstream in &self.routing.upstreams {
            if previous_upstreams
                .get(&upstream.id)
                .is_some_and(|old| old.same_destination(upstream))
                && let Some(discovery) = previous.discoveries.get(&upstream.id)
                && !discovery.pending_models.is_empty()
            {
                self.bindings
                    .discoveries
                    .entry(upstream.id)
                    .or_insert_with(|| discovery.clone())
                    .pending_models
                    .clone_from(&discovery.pending_models);
            }
        }

        if self.bindings.official_prices.is_none() {
            self.bindings
                .official_prices
                .clone_from(&previous.official_prices);
        }
        if let Some(official) = &self.bindings.official_prices {
            if previous.official_prices.as_ref() != Some(official)
                && current.pricing.version.checked_add(1) != Some(self.prices.version)
            {
                return Err(ManagementError::Invalid);
            }
            if let Some(old) = &previous.official_prices {
                self.prices.rules.retain(|rule| {
                    !old.models.contains_key(&rule.model)
                        || official.models.contains_key(&rule.model)
                });
            }
            self.prices = official.compile(&self.routing, &self.prices)?;
        }
        if self.prices.version == current.pricing.version && self.prices != current.pricing {
            return Err(ManagementError::Invalid);
        }
        let document = ReleaseDocument {
            contract_version: crate::routing::CONTRACT_VERSION,
            routing: self.routing.clone(),
            pricing: self.prices.clone(),
        };
        super::catalog::promote_pending(&document, &mut self.bindings)?;
        Ok(document)
    }
    fn digest(&self) -> Result<String, ManagementError> {
        let value = serde_json::to_vec(&(
            "lmm-routing-publication-v1",
            self.expected_version,
            &self.routing,
            &self.prices,
            &self.bindings,
        ))
        .map_err(|_| ManagementError::Invalid)?;
        Ok(crate::hex_codec::encode(Sha256::digest(value)))
    }
}
pub(in crate::channel_http) async fn read(
    State(state): State<ChannelState>,
    headers: HeaderMap,
) -> Result<Json<Value>, Failure> {
    super::super::admit(&state, &headers).await?;
    let management = state.management.as_ref().ok_or(Failure::Disabled)?;
    management.synchronize().await?;
    let _guard = management.publication.lock().await;
    let (mut tx, _) = management
        .begin(
            bearer(&headers)?,
            Action::ChannelSensitiveWrite,
            super::ReadView::Current,
        )
        .await?;
    let document = management
        .routing
        .catalog_document()
        .map_err(|_| ManagementError::Storage)?;
    let bindings = management.current_bindings()?;
    let channel_id_high_water = accounting::maximum_id(&mut tx).await?.max(
        management
            .bootstrap
            .routing
            .upstreams
            .iter()
            .chain(document.routing.upstreams.iter())
            .map(|u| u.id)
            .max()
            .unwrap_or(0),
    );
    tx.commit().await.map_err(ManagementError::from)?;
    let model_limits = management
        .gateway
        .as_ref()
        .map(|gateway| json!({ "total_timeout_ms": gateway.channel_total_timeout_ms() }));
    Ok(Json(
        json!({"success":true,"data":{"version":Version::of(&document),"routing":document.routing,"prices":document.pricing,"bindings":bindings,"provider_presets":super::provider_presets::PROVIDERS,"policy_sections":["routing_reliability","channel_affinity","global_model_policy","channel_overrides"],"channel_id_high_water":channel_id_high_water,"model_limits":model_limits,"secret_storage_enabled":management.gateway.as_ref().and_then(|g|g.channel_secret_key()).is_some()}}),
    ))
}
pub(in crate::channel_http) async fn preview(
    State(state): State<ChannelState>,
    headers: HeaderMap,
    Json(input): Json<ConfigurationInput>,
) -> Result<Json<Value>, Failure> {
    super::super::admit(&state, &headers).await?;
    let management = state.management.as_ref().ok_or(Failure::Disabled)?;
    let _guard = management.publication.lock().await;
    let (tx, _) = management
        .begin(
            bearer(&headers)?,
            Action::ChannelSensitiveWrite,
            super::ReadView::Current,
        )
        .await?;
    let mut input = input;
    let document = input.prepare(management)?;
    management.validate_bindings(&document, &input.bindings, &BTreeSet::new())?;
    let digest = input.digest()?;
    tx.rollback().await.map_err(ManagementError::from)?;
    Ok(Json(
        json!({"success":true,"data":{"saved":false,"expected_version":input.expected_version,"routing":document.routing,"prices":document.pricing,"bindings":input.bindings,"confirmation_digest":digest}}),
    ))
}
pub(in crate::channel_http) async fn publish(
    State(state): State<ChannelState>,
    headers: HeaderMap,
    Json(input): Json<ConfigurationInput>,
) -> Result<Json<Value>, Failure> {
    super::super::admit(&state, &headers).await?;
    let management = state.management.as_ref().ok_or(Failure::Disabled)?;
    let _guard = management.publication.lock().await;
    let (tx, principal) = management
        .begin(
            bearer(&headers)?,
            Action::ChannelSensitiveWrite,
            super::ReadView::Current,
        )
        .await?;
    let mut input = input;
    let document = input.prepare(management)?;
    let digest = input.digest()?;
    if input.confirmation_digest.as_deref() != Some(digest.as_str()) {
        return Err(ManagementError::Invalid.into());
    }
    let version = management
        .publish_bindings(
            tx,
            &principal,
            input.expected_version,
            document,
            input.bindings,
            Vec::new(),
            PublicationChange::Normal {
                action: "channel.configuration.publish",
                target: json!({"confirmation_digest":digest}),
            },
        )
        .await?;
    Ok(Json(json!({"success":true,"data":{"version":version}})))
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(in crate::channel_http) struct Attachment {
    route_id: u64,
    upstream_model: String,
    priority: u16,
    weight: u32,
    speeds: Vec<Speed>,
    timeout_ms: u32,
    payload: PayloadPolicy,
}
fn attach(
    document: &mut ReleaseDocument,
    id: u64,
    attachments: Vec<Attachment>,
) -> Result<(), ManagementError> {
    if attachments.len() > 1000 {
        return Err(ManagementError::Invalid);
    }
    let mut next = document
        .routing
        .routes
        .iter()
        .flat_map(|r| &r.targets)
        .map(|t| t.id)
        .max()
        .unwrap_or(0);
    let mut by_route = BTreeMap::new();
    for attachment in attachments {
        if by_route.insert(attachment.route_id, attachment).is_some() {
            return Err(ManagementError::Invalid);
        }
    }
    for route in &mut document.routing.routes {
        let Some(attachment) = by_route.remove(&route.id) else {
            route.targets.retain(|target| target.upstream_id != id);
            continue;
        };
        if let Some(target) = route
            .targets
            .iter_mut()
            .find(|target| target.upstream_id == id)
        {
            target.upstream_model = attachment.upstream_model;
            target.priority = attachment.priority;
            target.weight = attachment.weight;
            target.speeds = attachment.speeds;
            target.payload = attachment.payload;
            target.timeout_ms = attachment.timeout_ms;
            continue;
        }
        next = next
            .checked_add(1)
            .filter(|v| *v <= i64::MAX as u64)
            .ok_or(ManagementError::Invalid)?;
        route.targets.push(RouteTarget {
            id: next,
            upstream_id: id,
            upstream_model: attachment.upstream_model,
            enabled: true,
            priority: attachment.priority,
            weight: attachment.weight,
            speeds: attachment.speeds,
            client_protocols: None,
            payload: attachment.payload,
            timeout_ms: attachment.timeout_ms,
        });
    }
    if !by_route.is_empty() {
        return Err(ManagementError::Invalid);
    }
    Ok(())
}

#[cfg(test)]
#[test]
fn channel_attachment_edit_preserves_target_policy() {
    for enabled in [false, true] {
        let mut document: ReleaseDocument = serde_json::from_value(json!({
            "contract_version": 1,
            "routing": {
                "version": 1, "groups": [], "upstreams": [], "aliases": [],
                "routes": [{
                    "id": 1, "model": "example", "group": "default", "enabled": true,
                    "speeds": ["standard"],
                    "failover": {"max_attempts": 1, "retry_on": [], "allow_replay_after_send": false},
                    "targets": [{
                        "id": 5, "upstream_id": 2, "upstream_model": "before",
                        "enabled": enabled, "priority": 0, "weight": 1,
                        "speeds": ["standard"], "client_protocols": ["anthropic_messages"],
                        "payload": "adapt", "timeout_ms": 5000
                    }]
                }]
            },
            "pricing": {"version": 1, "rules": []}
        }))
        .unwrap();
        let attachments = vec![Attachment {
            route_id: 1,
            upstream_model: "after".into(),
            priority: 1,
            weight: 1,
            speeds: vec![Speed::Standard],
            timeout_ms: 5000,
            payload: PayloadPolicy::Adapt,
        }];
        attach(&mut document, 2, attachments).unwrap();

        let [target] = document.routing.routes[0].targets.as_slice() else {
            panic!("expected one replacement target");
        };
        assert_eq!(target.id, 5);
        assert_eq!(target.enabled, enabled);
        assert_eq!(target.upstream_model, "after");
        assert!(target.allows_client(Protocol::AnthropicMessages));
        assert!(!target.allows_client(Protocol::OpenAiChat));
        attach(
            &mut document,
            3,
            vec![Attachment {
                route_id: 1,
                upstream_model: "new".into(),
                priority: 1,
                weight: 1,
                speeds: vec![Speed::Standard],
                timeout_ms: 5000,
                payload: PayloadPolicy::Adapt,
            }],
        )
        .unwrap();
        assert_eq!(document.routing.routes[0].targets[0].enabled, enabled);
        assert!(document.routing.routes[0].targets[1].enabled);
        let pricing = serde_json::to_value(&document.pricing).unwrap();
        let mut channel: ChannelInput = serde_json::from_value(json!({
            "expected_version":{"routing":1,"pricing":1}, "id":52,
            "name":"channel 52", "endpoint":"https://supplier.example", "protocol":"openai_chat",
            "enabled":false, "contexts":[{
                "upstream_id":52,"upstream_model":"model","protocol":"openai_chat",
                "endpoint":"https://supplier.example/","max_input_tokens":2048,
                "max_output_tokens":256,"contract_revision":"fixture-v1"
            }], "attachments":[]
        }))
        .unwrap();
        edit_document(&mut document, &mut Bindings::default(), &mut channel).unwrap();
        assert_eq!(channel.endpoint.as_str(), "https://supplier.example/");
        assert_eq!(channel.contexts[0].endpoint, channel.endpoint.as_str());
        attach(
            &mut document,
            2,
            vec![Attachment {
                route_id: 1,
                upstream_model: "edited again".into(),
                priority: 2,
                weight: 4,
                speeds: vec![Speed::Standard],
                timeout_ms: 6000,
                payload: PayloadPolicy::Adapt,
            }],
        )
        .unwrap();
        let ids: Vec<_> = document.routing.routes[0]
            .targets
            .iter()
            .map(|t| t.id)
            .collect();
        assert_eq!(ids, [5, 6]);
        assert_eq!(serde_json::to_value(&document.pricing).unwrap(), pricing);
    }
}

#[cfg(test)]
#[test]
fn channel_52_ordinary_save_keeps_binding_target_order_and_price() {
    let target = |id: u64, channel: u64, enabled: bool| {
        json!({
            "id":id,"upstream_id":channel,"upstream_model":"before","enabled":enabled,
            "priority":1,"weight":10,"speeds":["standard"],"client_protocols":["openai_chat"],
            "payload":"adapt","timeout_ms":5000
        })
    };
    let upstream = |id: u64, endpoint: &str| {
        json!({
            "id":id,"endpoint":endpoint,"protocol":"openai_chat",
            "enabled":true,"credential":{"id":id,"revision":2}
        })
    };
    let mut document: ReleaseDocument = serde_json::from_value(json!({
        "contract_version":1, "routing":{
            "version":23,"groups":[{"name":"default","enabled":true}],"aliases":[],
            "upstreams":[upstream(10,"https://first.example/"),upstream(52,"https://supplier.example"),
                         upstream(14,"https://last.example/")],
            "routes":[{"id":18,"model":"example","group":"default","enabled":true,"speeds":["standard"],
                "failover":{"max_attempts":2,"retry_on":["connect"],"allow_replay_after_send":false},
                "targets":[target(102,10,true),target(510,52,false),target(43,14,true)]}]
        }, "pricing":{"version":7,"rules":[{
            "id":29,"model":"example","group":"default","speed":"standard",
            "rates":{"input":3000000,"output":12000000,"cache_read":300000,"cache_write":3750000,"request":10},
            "group_multiplier_ppm":1000000,"speed_multiplier_ppm":1000000
        }]}
    })).unwrap();
    let mut input: ChannelInput = serde_json::from_value(json!({
        "expected_version":{"routing":23,"pricing":7},"id":52,"name":"channel 52 renamed",
        "endpoint":"https://supplier.example","protocol":"openai_chat","enabled":true,
        "contexts":[{"upstream_id":52,"upstream_model":"after","protocol":"openai_chat",
            "endpoint":"https://supplier.example/","max_input_tokens":2048,"max_output_tokens":256,
            "contract_revision":"fixture-v1"}],
        "attachments":[{"route_id":18,"upstream_model":"after","priority":0,"weight":25,
            "speeds":["standard"],"timeout_ms":7500,"payload":"adapt"}]
    }))
    .unwrap();
    let prices = serde_json::to_vec(&document.pricing).unwrap();
    let neighbors = document.routing.routes[0].targets.clone();
    let mut bindings = Bindings::default();
    edit_document(&mut document, &mut bindings, &mut input).unwrap();
    attach(&mut document, input.id, input.attachments).unwrap();

    assert_eq!(bindings.names[&52], "channel 52 renamed");
    let saved = &document.routing.upstreams[1];
    assert_eq!(saved.id, 52);
    assert_eq!(
        saved.credential,
        CredentialRef {
            id: 52,
            revision: 2
        }
    );
    assert!(saved.enabled);
    assert_eq!(saved.endpoint.as_str(), "https://supplier.example/");
    assert_eq!(bindings.contexts[0].endpoint, saved.endpoint.as_str());
    let [first, edited, last] = document.routing.routes[0].targets.as_slice() else {
        panic!("expected all three saved targets");
    };
    assert_eq!([first.id, edited.id, last.id], [102, 510, 43]);
    assert_eq!(first, &neighbors[0]);
    assert_eq!(last, &neighbors[2]);
    assert_eq!((edited.upstream_id, edited.enabled), (52, false));
    assert_eq!(edited.client_protocols, neighbors[1].client_protocols);
    assert!(edited.allows_client(Protocol::OpenAiChat));
    assert!(!edited.allows_client(Protocol::AnthropicMessages));
    assert_eq!(edited.upstream_model, "after");
    assert_eq!(
        (edited.priority, edited.weight, edited.timeout_ms),
        (0, 25, 7500)
    );
    assert_eq!(serde_json::to_vec(&document.pricing).unwrap(), prices);
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(in crate::channel_http) struct ChannelInput {
    expected_version: Version,
    id: u64,
    name: String,
    #[serde(default)]
    tag: Option<String>,
    #[serde(default)]
    provider: Option<String>,
    #[serde(default)]
    note: Option<String>,
    endpoint: Endpoint,
    protocol: Protocol,
    enabled: bool,
    contexts: Vec<ExternalModelContext>,
    attachments: Vec<Attachment>,
    #[serde(default)]
    secret: Option<String>,
}
fn edit_document(
    document: &mut ReleaseDocument,
    bindings: &mut Bindings,
    input: &mut ChannelInput,
) -> Result<(), ManagementError> {
    if !valid_label(&input.name)
        || input
            .tag
            .as_deref()
            .is_some_and(|tag| !tag.is_empty() && !valid_label(tag))
        || input.id == 0
        || input.id > i64::MAX as u64
        || input.contexts.len() > 1000
        || (input.enabled && input.attachments.is_empty())
    {
        return Err(ManagementError::Invalid);
    }
    // The provider binding and upstream must identify the same parsed base URL.
    // In particular URL serializes an empty root path as "/".
    let normalized =
        crate::relay::honeypot_network::validate_base_url(input.endpoint.as_str(), false)
            .map_err(|_| ManagementError::Invalid)?;
    for context in &mut input.contexts {
        if context.upstream_id != input.id
            || context.protocol != input.protocol
            || crate::relay::honeypot_network::validate_base_url(&context.endpoint, false)
                .map_err(|_| ManagementError::Invalid)?
                != normalized
        {
            return Err(ManagementError::Invalid);
        }
        context.endpoint = normalized.to_string();
    }
    input.endpoint = normalized
        .to_string()
        .try_into()
        .map_err(|_| ManagementError::Invalid)?;
    bindings.names.insert(input.id, input.name.clone());
    if let Some(tag) = &input.tag {
        if tag.is_empty() {
            bindings.tags.remove(&input.id);
        } else {
            bindings.tags.insert(input.id, tag.clone());
        }
    }
    if let Some(note) = &input.note {
        if !valid_note(note) {
            return Err(ManagementError::Invalid);
        }
        if note.trim().is_empty() {
            bindings.channel_notes.remove(&input.id);
        } else {
            bindings.channel_notes.insert(input.id, note.clone());
        }
    }
    if let Some(provider) = &input.provider {
        if provider.is_empty() {
            bindings.channel_providers.remove(&input.id);
        } else {
            bindings
                .channel_providers
                .insert(input.id, provider.clone());
        }
    }
    bindings
        .contexts
        .retain(|context| context.upstream_id != input.id);
    bindings.contexts.extend(input.contexts.clone());
    if let Some(upstream) = document
        .routing
        .upstreams
        .iter_mut()
        .find(|u| u.id == input.id)
    {
        if upstream.endpoint != input.endpoint || upstream.protocol != input.protocol {
            bindings.discoveries.remove(&input.id);
        }
        upstream.endpoint = input.endpoint.clone();
        upstream.protocol = input.protocol;
        upstream.enabled = input.enabled;
    }
    Ok(())
}
pub(super) async fn store_secret(
    management: &ManagedChannels,
    tx: &mut Transaction<'_, Postgres>,
    channel: u64,
    label: &str,
    secret: String,
) -> Result<crate::model_http::UpstreamCredential, Failure> {
    if !valid_label(label) {
        return Err(ManagementError::Invalid.into());
    }
    let gateway = management.gateway.as_ref().ok_or(Failure::Disabled)?;
    let key = gateway.channel_secret_key().ok_or(Failure::Disabled)?;
    let mut random = [0_u8; 8];
    SysRng
        .try_fill_bytes(&mut random)
        .map_err(|_| ManagementError::Storage)?;
    let id = (u64::from_be_bytes(random) & ((1_u64 << 53) - 1)).max(1);
    let reference = CredentialRef { id, revision: 1 };
    let ciphertext = vault::encrypt(key, reference, &secret)?;
    sqlx::query("SELECT public.routing_control_secret_create($1,$2,$3,$4,$5)")
        .bind(channel as i64)
        .bind(id as i64)
        .bind(1_i64)
        .bind(label)
        .bind(ciphertext)
        .execute(&mut **tx)
        .await
        .map_err(ManagementError::from)?;
    Ok(crate::model_http::UpstreamCredential { reference, secret })
}
pub(in crate::channel_http) async fn create(
    State(state): State<ChannelState>,
    headers: HeaderMap,
    Json(mut input): Json<ChannelInput>,
) -> Result<Json<Value>, Failure> {
    super::super::admit(&state, &headers).await?;
    let management = state.management.as_ref().ok_or(Failure::Disabled)?;
    let _guard = management.publication.lock().await;
    let (mut tx, principal) = management
        .begin(
            bearer(&headers)?,
            Action::ChannelSensitiveWrite,
            super::ReadView::Current,
        )
        .await?;
    let mut document = next_document(management, input.expected_version)?;
    if document.routing.upstreams.iter().any(|u| u.id == input.id) {
        return Err(ManagementError::Conflict.into());
    }
    let mut bindings = management.current_bindings()?;
    edit_document(&mut document, &mut bindings, &mut input)?;
    let key = store_secret(
        management,
        &mut tx,
        input.id,
        "Primary",
        input.secret.take().ok_or(ManagementError::Invalid)?,
    )
    .await?;
    document
        .routing
        .upstreams
        .push(crate::routing::UpstreamConfig {
            id: input.id,
            endpoint: input.endpoint,
            protocol: input.protocol,
            credential: key.reference,
            enabled: input.enabled,
        });
    attach(&mut document, input.id, input.attachments)?;
    let version = management
        .publish_bindings(
            tx,
            &principal,
            input.expected_version,
            document,
            bindings,
            vec![key],
            PublicationChange::Normal {
                action: "channel.create",
                target: json!({"id":input.id}),
            },
        )
        .await?;
    Ok(Json(
        json!({"success":true,"data":{"id":input.id,"version":version}}),
    ))
}
pub(in crate::channel_http) async fn update(
    State(state): State<ChannelState>,
    headers: HeaderMap,
    Path(id): Path<u64>,
    Json(mut input): Json<ChannelInput>,
) -> Result<Json<Value>, Failure> {
    super::super::admit(&state, &headers).await?;
    let management = state.management.as_ref().ok_or(Failure::Disabled)?;
    let _guard = management.publication.lock().await;
    let (tx, principal) = management
        .begin(
            bearer(&headers)?,
            Action::ChannelSensitiveWrite,
            super::ReadView::Current,
        )
        .await?;
    let mut document = next_document(management, input.expected_version)?;
    if id != input.id || input.secret.is_some() {
        return Err(ManagementError::Invalid.into());
    }
    if !document.routing.upstreams.iter().any(|u| u.id == id) {
        return Err(Failure::NotFound);
    }
    let mut bindings = management.current_bindings()?;
    edit_document(&mut document, &mut bindings, &mut input)?;
    attach(&mut document, id, input.attachments)?;
    let version = management
        .publish_bindings(
            tx,
            &principal,
            input.expected_version,
            document,
            bindings,
            Vec::new(),
            PublicationChange::Normal {
                action: "channel.update",
                target: json!({"id":id}),
            },
        )
        .await?;
    Ok(Json(json!({"success":true,"data":{"version":version}})))
}

pub(in crate::channel_http) async fn keys(
    State(state): State<ChannelState>,
    headers: HeaderMap,
    Path(id): Path<u64>,
) -> Result<Json<Value>, Failure> {
    super::super::admit(&state, &headers).await?;
    let management = state.management.as_ref().ok_or(Failure::Disabled)?;
    management.synchronize().await?;
    let _guard = management.publication.lock().await;
    let (mut tx, _) = management
        .begin(
            bearer(&headers)?,
            Action::ChannelSensitiveWrite,
            super::ReadView::Current,
        )
        .await?;
    let document = management
        .routing
        .catalog_document()
        .map_err(|_| ManagementError::Storage)?;
    let upstream = document
        .routing
        .upstreams
        .iter()
        .find(|u| u.id == id)
        .ok_or(Failure::NotFound)?;
    let bindings = management.current_bindings()?;
    let pool = bindings.key_pools.get(&id);
    let polling = pool.is_some_and(|pool| pool.polling);
    let mut list: Value = sqlx::query_scalar("SELECT public.routing_control_secret_list($1)")
        .bind(id as i64)
        .fetch_one(&mut *tx)
        .await
        .map_err(ManagementError::from)?;
    for key in list.as_array_mut().into_iter().flatten() {
        let reference = serde_json::from_value::<CredentialRef>(key["reference"].clone())
            .map_err(|_| ManagementError::Invalid)?;
        key["enabled"] = json!(if polling {
            !pool.is_some_and(|pool| pool.disabled.contains(&reference))
        } else {
            reference == upstream.credential
        });
    }
    tx.commit().await.map_err(ManagementError::from)?;
    Ok(Json(
        json!({"success":true,"data":{"version":Version::of(&document),"active_reference":upstream.credential,"polling":polling,"keys":list,"plaintext_available":false}}),
    ))
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(in crate::channel_http) struct KeyImport {
    label: String,
    secret: String,
}

#[derive(Deserialize)]
#[serde(tag = "operation", rename_all = "snake_case", deny_unknown_fields)]
pub(in crate::channel_http) enum KeyOperation {
    Add {
        expected_version: Version,
        label: String,
        secret: String,
        activate: bool,
    },
    AddMany {
        expected_version: Version,
        keys: Vec<KeyImport>,
        polling: bool,
    },
    Activate {
        expected_version: Version,
        reference: CredentialRef,
    },
    SetPolling {
        expected_version: Version,
        enabled: bool,
    },
    SetEnabled {
        expected_version: Version,
        reference: CredentialRef,
        enabled: bool,
    },
    Remove {
        expected_version: Version,
        reference: CredentialRef,
    },
}
pub(in crate::channel_http) async fn change_key(
    State(state): State<ChannelState>,
    headers: HeaderMap,
    Path(id): Path<u64>,
    Json(input): Json<KeyOperation>,
) -> Result<Json<Value>, Failure> {
    super::super::admit(&state, &headers).await?;
    let management = state.management.as_ref().ok_or(Failure::Disabled)?;
    let _guard = management.publication.lock().await;
    let (mut tx, principal) = management
        .begin(
            bearer(&headers)?,
            Action::ChannelSensitiveWrite,
            super::ReadView::Current,
        )
        .await?;
    let expected = match &input {
        KeyOperation::Add {
            expected_version, ..
        }
        | KeyOperation::AddMany {
            expected_version, ..
        }
        | KeyOperation::Activate {
            expected_version, ..
        }
        | KeyOperation::SetPolling {
            expected_version, ..
        }
        | KeyOperation::SetEnabled {
            expected_version, ..
        }
        | KeyOperation::Remove {
            expected_version, ..
        } => *expected_version,
    };
    let mut document = next_document(management, expected)?;
    let upstream = document
        .routing
        .upstreams
        .iter_mut()
        .find(|u| u.id == id)
        .ok_or(Failure::NotFound)?;
    let previous_credential = upstream.credential;
    let mut bindings = management.current_bindings()?;
    let mut credentials = Vec::new();
    let mut imported_count = None;
    let (reference, action) = match input {
        KeyOperation::Add {
            label,
            secret,
            activate,
            ..
        } => {
            let key = store_secret(management, &mut tx, id, &label, secret).await?;
            let reference = key.reference;
            if activate {
                upstream.credential = reference;
            }
            credentials.push(key);
            (reference, "channel.key.add")
        }
        KeyOperation::AddMany { keys, polling, .. } => {
            let mut secrets = BTreeSet::new();
            if !(1..=100).contains(&keys.len()) {
                return Err(ManagementError::Invalid.into());
            }
            for key in &keys {
                if !valid_label(&key.label) || !secrets.insert(key.secret.as_str()) {
                    return Err(ManagementError::Invalid.into());
                }
                vault::validate_secret(&key.secret)?;
            }
            imported_count = Some(keys.len());
            for key in keys {
                credentials
                    .push(store_secret(management, &mut tx, id, &key.label, key.secret).await?);
            }
            if polling {
                bindings.key_pools.entry(id).or_default().polling = true;
            } else {
                bindings.key_pools.remove(&id);
            }
            (upstream.credential, "channel.key.add_many")
        }
        KeyOperation::Activate { reference, .. } => {
            let keys: Value = sqlx::query_scalar("SELECT public.routing_control_secret_list($1)")
                .bind(id as i64)
                .fetch_one(&mut *tx)
                .await
                .map_err(ManagementError::from)?;
            let reference_json =
                serde_json::to_value(reference).map_err(|_| ManagementError::Invalid)?;
            if !keys
                .as_array()
                .is_some_and(|values| values.iter().any(|v| v["reference"] == reference_json))
            {
                return Err(Failure::NotFound);
            }
            let ciphertext: Option<Vec<u8>> =
                sqlx::query_scalar("SELECT public.routing_control_secret_read($1,$2)")
                    .bind(reference.id as i64)
                    .bind(reference.revision as i64)
                    .fetch_one(&mut *tx)
                    .await
                    .map_err(ManagementError::from)?;
            let key = management
                .gateway
                .as_ref()
                .and_then(|g| g.channel_secret_key())
                .ok_or(Failure::Disabled)?;
            let secret = vault::decrypt(key, reference, &ciphertext.ok_or(Failure::NotFound)?)?;
            credentials.push(crate::model_http::UpstreamCredential { reference, secret });
            upstream.credential = reference;
            if let Some(pool) = bindings.key_pools.get_mut(&id) {
                pool.disabled.retain(|item| *item != reference);
            }
            (reference, "channel.key.activate")
        }
        KeyOperation::SetPolling { enabled, .. } => {
            let reference = upstream.credential;
            if enabled {
                bindings.key_pools.entry(id).or_default().polling = true;
            } else {
                bindings.key_pools.remove(&id);
            }
            (reference, "channel.key.polling")
        }
        KeyOperation::SetEnabled {
            reference, enabled, ..
        } => {
            if upstream.credential == reference && !enabled {
                return Err(ManagementError::Conflict.into());
            }
            let pool = bindings.key_pools.get(&id).filter(|pool| pool.polling);
            if pool.is_none() {
                return Err(ManagementError::Invalid.into());
            }
            let keys: Value = sqlx::query_scalar("SELECT public.routing_control_secret_list($1)")
                .bind(id as i64)
                .fetch_one(&mut *tx)
                .await
                .map_err(ManagementError::from)?;
            let reference_json =
                serde_json::to_value(reference).map_err(|_| ManagementError::Invalid)?;
            if !keys
                .as_array()
                .is_some_and(|values| values.iter().any(|v| v["reference"] == reference_json))
            {
                return Err(Failure::NotFound);
            }
            let pool = bindings.key_pools.get_mut(&id).unwrap();
            if enabled {
                pool.disabled.retain(|item| *item != reference);
            } else if !pool.disabled.contains(&reference) {
                pool.disabled.push(reference);
                pool.disabled.sort_by_key(|item| (item.id, item.revision));
            }
            (
                reference,
                if enabled {
                    "channel.key.enable"
                } else {
                    "channel.key.disable"
                },
            )
        }
        KeyOperation::Remove { reference, .. } => {
            if upstream.credential == reference {
                return Err(ManagementError::Conflict.into());
            }
            let removed: bool =
                sqlx::query_scalar("SELECT public.routing_control_secret_remove($1,$2,$3)")
                    .bind(id as i64)
                    .bind(reference.id as i64)
                    .bind(reference.revision as i64)
                    .fetch_one(&mut *tx)
                    .await
                    .map_err(ManagementError::from)?;
            if !removed {
                return Err(Failure::NotFound);
            }
            if let Some(pool) = bindings.key_pools.get_mut(&id) {
                pool.disabled.retain(|item| *item != reference);
            }
            (reference, "channel.key.remove")
        }
    };
    if previous_credential != upstream.credential {
        bindings.discoveries.remove(&id);
    }
    let mut target = json!({"id":id,"reference":reference});
    if let Some(count) = imported_count {
        target["imported_count"] = json!(count);
    }
    let version = management
        .publish_bindings(
            tx,
            &principal,
            expected,
            document,
            bindings,
            credentials,
            PublicationChange::Normal { action, target },
        )
        .await?;
    let mut data = json!({"version":version,"reference":reference});
    if let Some(count) = imported_count {
        data["imported_count"] = json!(count);
    }
    Ok(Json(json!({"success":true,"data":data})))
}

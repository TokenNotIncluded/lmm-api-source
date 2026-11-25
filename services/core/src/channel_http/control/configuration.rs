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
    #[serde(default)]
    pub names: BTreeMap<u64, String>,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub key_pools: BTreeMap<u64, KeyPool>,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub tags: BTreeMap<u64, String>,
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
        for (id, tag) in &self.tags {
            if !valid_label(tag) || !upstream_ids.contains(id) {
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
    fn document(&self) -> Result<ReleaseDocument, ManagementError> {
        if self.routing.version
            != self
                .expected_version
                .routing
                .checked_add(1)
                .ok_or(ManagementError::Invalid)?
            || self.prices.version < self.expected_version.pricing
        {
            return Err(ManagementError::Invalid);
        }
        Ok(ReleaseDocument {
            contract_version: crate::routing::CONTRACT_VERSION,
            routing: self.routing.clone(),
            pricing: self.prices.clone(),
        })
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
    let routing = serde_json::to_value(&document.routing).map_err(|_| ManagementError::Invalid)?;
    let prices = serde_json::to_value(&document.pricing).map_err(|_| ManagementError::Invalid)?;

    let authority = json!({
        "mode": management.authority,
        "source": if management.api_managed() { "api_managed_runtime" } else { "model.config_file" },
        "version": Version::of(&document),
        "routing_sha256": document_digest(&routing)?,
        "pricing_sha256": document_digest(&prices)?,
        "git_revision": null,
        "observed_at_unix": chrono::Utc::now().timestamp(),
    });
    Ok(Json(
        json!({"success":true,"data":{"version":Version::of(&document),"authority":authority,"routing":routing,"prices":prices,"bindings":bindings,"channel_id_high_water":channel_id_high_water,"model_limits":model_limits,"secret_storage_enabled":management.api_managed() && management.secret_storage_enabled()}}),
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
    let current = next_document(management, input.expected_version)?;
    let document = input.document()?;
    if document.pricing.version == input.expected_version.pricing
        && document.pricing != current.pricing
    {
        return Err(ManagementError::Invalid.into());
    }
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
    let document = input.document()?;
    let digest = input.digest()?;
    if input.confirmation_digest.as_deref() != Some(digest.as_str()) {
        return Err(ManagementError::Invalid.into());
    }
    let current = next_document(management, input.expected_version)?;
    if document.pricing.version == input.expected_version.pricing
        && document.pricing != current.pricing
    {
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
    // Omitted on older channel edits. An explicit null clears the previous filter.
    #[serde(default, deserialize_with = "attachment_protocols")]
    client_protocols: Option<Option<Vec<Protocol>>>,
}

fn attachment_protocols<'de, D: serde::Deserializer<'de>>(
    deserializer: D,
) -> Result<Option<Option<Vec<Protocol>>>, D::Error> {
    Option::<Vec<Protocol>>::deserialize(deserializer).map(Some)
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
    let mut seen = BTreeSet::new();
    // Preserve routing policy when a channel edit replaces its targets.
    let mut previous_target_policy = BTreeMap::new();
    for route in &mut document.routing.routes {
        route.targets.retain(|target| {
            if target.upstream_id != id {
                return true;
            }
            previous_target_policy
                .insert(route.id, (target.enabled, target.client_protocols.clone()));
            false
        });
    }
    for attachment in attachments {
        if !seen.insert(attachment.route_id) {
            return Err(ManagementError::Invalid);
        }
        next = next
            .checked_add(1)
            .filter(|v| *v <= i64::MAX as u64)
            .ok_or(ManagementError::Invalid)?;
        let route = document
            .routing
            .routes
            .iter_mut()
            .find(|r| r.id == attachment.route_id)
            .ok_or(ManagementError::Invalid)?;
        let (enabled, client_protocols) = previous_target_policy
            .remove(&route.id)
            .unwrap_or((true, None));
        route.targets.push(RouteTarget {
            id: next,
            upstream_id: id,
            upstream_model: attachment.upstream_model,
            enabled,
            priority: attachment.priority,
            weight: attachment.weight,
            speeds: attachment.speeds,
            client_protocols: attachment.client_protocols.unwrap_or(client_protocols),
            payload: attachment.payload,
            timeout_ms: attachment.timeout_ms,
        });
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
            client_protocols: None,
        }];
        attach(&mut document, 2, attachments).unwrap();

        let [target] = document.routing.routes[0].targets.as_slice() else {
            panic!("expected one replacement target");
        };
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
                client_protocols: None,
            }],
        )
        .unwrap();
        assert_eq!(document.routing.routes[0].targets[0].enabled, enabled);
        assert!(document.routing.routes[0].targets[1].enabled);

        // Exercise the same JSON attachment shape accepted by channel create and update.
        let mut edit = json!({
            "route_id": 1, "upstream_model": "gemini-test", "priority": 1,
            "weight": 1, "speeds": ["standard"], "payload":"adapt", "timeout_ms": 5000
        });
        for (protocols, allow_chat) in [(json!(["gemini"]), false), (Value::Null, true)] {
            edit["client_protocols"] = protocols;
            let input: Attachment = serde_json::from_value(edit.clone()).unwrap();
            attach(&mut document, 2, vec![input]).unwrap();
            let target = document.routing.routes[0]
                .targets
                .iter()
                .find(|t| t.upstream_id == 2)
                .unwrap();
            assert_eq!(target.enabled, enabled);
            assert!(target.allows_client(Protocol::Gemini));
            assert_eq!(target.allows_client(Protocol::OpenAiChat), allow_chat);
        }
    }
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(in crate::channel_http) struct ChannelInput {
    expected_version: Version,
    id: u64,
    name: String,
    #[serde(default)]
    tag: Option<String>,
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
    input: &ChannelInput,
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
    crate::relay::honeypot_network::validate_base_url(input.endpoint.as_str(), false)
        .map_err(|_| ManagementError::Invalid)?;
    if input.contexts.iter().any(|context| {
        context.upstream_id != input.id
            || context.endpoint != input.endpoint.as_str()
            || context.protocol != input.protocol
    }) {
        return Err(ManagementError::Invalid);
    }
    bindings.names.insert(input.id, input.name.clone());
    if let Some(tag) = &input.tag {
        if tag.is_empty() {
            bindings.tags.remove(&input.id);
        } else {
            bindings.tags.insert(input.id, tag.clone());
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
    if !management.api_managed() {
        return Err(Failure::FileManaged);
    }
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
    edit_document(&mut document, &mut bindings, &input)?;
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
    Json(input): Json<ChannelInput>,
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
    edit_document(&mut document, &mut bindings, &input)?;
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
    let mut list: Value = if management.api_managed() {
        sqlx::query_scalar("SELECT public.routing_control_secret_list($1)")
            .bind(id as i64)
            .fetch_one(&mut *tx)
            .await
            .map_err(ManagementError::from)?
    } else {
        // The file reference is reported as active_reference below, outside the API vault.
        json!([])
    };
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
    if !management.api_managed() {
        return Err(Failure::FileManaged);
    }
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

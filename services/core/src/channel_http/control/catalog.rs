//! Vendors, executable model routes and non-published provider discoveries.
use super::*;
use crate::{
    model_http::ExternalModelContext,
    routing::{ModelRoute, PriceRule},
};
use serde::{Deserialize, Serialize};

#[derive(Clone, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub(in crate::channel_http) struct Vendor {
    pub id: u64,
    pub name: String,
    pub description: String,
}
#[derive(Clone, Default, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub(in crate::channel_http) struct ModelMetadata {
    pub display_name: String,
    pub description: String,
    pub vendor_id: Option<u64>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub tags: Vec<String>,
}
#[derive(Clone, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub(in crate::channel_http) struct Discovery {
    pub models: Vec<String>,
    pub observed_at: i64,
    #[serde(default, skip_serializing_if = "BTreeSet::is_empty")]
    pub pending_models: BTreeSet<String>,
}

fn enabled_targets(document: &ReleaseDocument) -> impl Iterator<Item = (u64, &str, &str)> {
    let groups: BTreeSet<_> = document
        .routing
        .groups
        .iter()
        .filter(|group| group.enabled)
        .map(|group| group.name.as_str())
        .collect();
    let upstreams: BTreeSet<_> = document
        .routing
        .upstreams
        .iter()
        .filter(|upstream| upstream.enabled)
        .map(|upstream| upstream.id)
        .collect();
    document
        .routing
        .routes
        .iter()
        .filter(move |route| route.enabled && groups.contains(route.group.as_str()))
        .flat_map(|route| {
            route
                .targets
                .iter()
                .filter(move |target| {
                    target.enabled
                        && target.weight > 0
                        && target
                            .speeds
                            .iter()
                            .any(|speed| route.speeds.contains(speed))
                })
                .map(move |target| {
                    (
                        target.upstream_id,
                        target.upstream_model.as_str(),
                        route.model.as_str(),
                    )
                })
        })
        .filter(move |(id, _, _)| upstreams.contains(id))
}

// A provider model can back multiple public routes. Check every public base
// rate before removing the shared pending selection from its channel.
pub(super) fn promote_pending(
    document: &ReleaseDocument,
    bindings: &mut Bindings,
) -> Result<(), ManagementError> {
    if !bindings
        .discoveries
        .values()
        .any(|row| !row.pending_models.is_empty())
    {
        return Ok(());
    }
    let mut promoted = Vec::new();
    for (id, raw_model, public_model) in enabled_targets(document) {
        if bindings
            .discoveries
            .get(&id)
            .is_some_and(|row| row.pending_models.contains(raw_model))
        {
            if !bindings
                .official_prices
                .as_ref()
                .is_some_and(|official| official.models.contains_key(public_model))
            {
                return Err(ManagementError::Invalid);
            }
            promoted.push((id, raw_model));
        }
    }
    for (id, model) in promoted {
        if let Some(row) = bindings.discoveries.get_mut(&id) {
            row.pending_models.remove(model);
        }
    }
    Ok(())
}
fn label(value: &str) -> bool {
    !value.trim().is_empty() && value.len() <= 128 && !value.chars().any(char::is_control)
}
fn description(value: &str) -> bool {
    value.len() <= 4096
        && !value
            .chars()
            .any(|c| c.is_control() && c != '\n' && c != '\t')
}
pub(in crate::channel_http) fn validate_metadata(
    bindings: &Bindings,
    document: &ReleaseDocument,
) -> Result<(), ManagementError> {
    if bindings.vendors.len() > 1000
        || bindings.models.len() > 100_000
        || bindings.discoveries.len() > 100_000
    {
        return Err(ManagementError::Invalid);
    }
    let mut names = BTreeSet::new();
    for (id, vendor) in &bindings.vendors {
        if *id == 0
            || *id > i64::MAX as u64
            || vendor.id != *id
            || !label(&vendor.name)
            || !description(&vendor.description)
            || !names.insert(vendor.name.trim().to_lowercase())
        {
            return Err(ManagementError::Invalid);
        }
    }
    for (id, model) in &bindings.models {
        if !document.routing.routes.iter().any(|r| r.id == *id)
            || !label(&model.display_name)
            || !description(&model.description)
            || model.tags.len() > 20
            || model.tags.iter().any(|tag| !label(tag))
            || model.tags.iter().collect::<BTreeSet<_>>().len() != model.tags.len()
            || model
                .vendor_id
                .is_some_and(|id| !bindings.vendors.contains_key(&id))
        {
            return Err(ManagementError::Invalid);
        }
    }
    for (id, discovery) in &bindings.discoveries {
        if !document
            .routing
            .upstreams
            .iter()
            .any(|upstream| upstream.id == *id)
            || !(1..=253_402_300_799).contains(&discovery.observed_at)
            || discovery.models.len() > 10_000
            || discovery.pending_models.len() > 10_000
            || discovery
                .models
                .iter()
                .chain(&discovery.pending_models)
                .any(|model| {
                    model.is_empty() || model.len() > 256 || model.chars().any(char::is_control)
                })
        {
            return Err(ManagementError::Invalid);
        }
    }
    if bindings
        .discoveries
        .values()
        .any(|row| !row.pending_models.is_empty())
        && enabled_targets(document).any(|(id, model, _)| {
            bindings
                .discoveries
                .get(&id)
                .is_some_and(|row| row.pending_models.contains(model))
        })
    {
        return Err(ManagementError::Invalid);
    }
    Ok(())
}
pub(in crate::channel_http) async fn vendors(
    State(state): State<ChannelState>,
    headers: HeaderMap,
) -> Result<Json<Value>, Failure> {
    super::super::admit(&state, &headers).await?;
    let management = state.management.as_ref().ok_or(Failure::Disabled)?;
    management.synchronize().await?;
    let _guard = management.publication.lock().await;
    let (tx, _) = management
        .begin(
            bearer(&headers)?,
            Action::ChannelSensitiveWrite,
            super::ReadView::Current,
        )
        .await?;
    let bindings = management.current_bindings()?;
    let version = management
        .routing
        .version()
        .map_err(|_| ManagementError::Storage)?;
    tx.commit().await.map_err(ManagementError::from)?;
    Ok(Json(
        json!({"success":true,"data":{"version":Version::from(version),"items":bindings.vendors.into_values().collect::<Vec<_>>()}}),
    ))
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(in crate::channel_http) struct VendorInput {
    expected_version: Version,
    vendor: Vendor,
}
pub(in crate::channel_http) async fn create_vendor(
    State(state): State<ChannelState>,
    headers: HeaderMap,
    Json(input): Json<VendorInput>,
) -> Result<Json<Value>, Failure> {
    save_vendor(&state, &headers, input, false).await
}
pub(in crate::channel_http) async fn update_vendor(
    State(state): State<ChannelState>,
    headers: HeaderMap,
    Path(id): Path<u64>,
    Json(input): Json<VendorInput>,
) -> Result<Json<Value>, Failure> {
    if input.vendor.id != id {
        return Err(ManagementError::Invalid.into());
    }
    save_vendor(&state, &headers, input, true).await
}
async fn save_vendor(
    state: &ChannelState,
    headers: &HeaderMap,
    input: VendorInput,
    existing: bool,
) -> Result<Json<Value>, Failure> {
    super::super::admit(state, headers).await?;
    let management = state.management.as_ref().ok_or(Failure::Disabled)?;
    let _guard = management.publication.lock().await;
    let (tx, principal) = management
        .begin(
            bearer(headers)?,
            Action::ChannelSensitiveWrite,
            super::ReadView::Current,
        )
        .await?;
    let document = next_document(management, input.expected_version)?;
    let mut bindings = management.current_bindings()?;
    let id = input.vendor.id;
    if bindings.vendors.contains_key(&id) != existing {
        return Err(ManagementError::Conflict.into());
    }
    bindings.vendors.insert(id, input.vendor);
    let version = management
        .publish_bindings(
            tx,
            &principal,
            input.expected_version,
            document,
            bindings,
            Vec::new(),
            super::PublicationChange::Normal {
                action: if existing {
                    "vendor.update"
                } else {
                    "vendor.create"
                },
                target: json!({"id":id}),
            },
        )
        .await?;
    Ok(Json(
        json!({"success":true,"data":{"version":version,"id":id}}),
    ))
}
pub(in crate::channel_http) async fn remove_vendor(
    State(state): State<ChannelState>,
    headers: HeaderMap,
    Path(id): Path<u64>,
    Json(input): Json<DeleteInput>,
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
    let document = next_document(management, input.expected_version)?;
    let mut bindings = management.current_bindings()?;
    if bindings.models.values().any(|m| m.vendor_id == Some(id)) {
        return Err(ManagementError::Conflict.into());
    }
    if bindings.vendors.remove(&id).is_none() {
        return Err(Failure::NotFound);
    }
    let version = management
        .publish_bindings(
            tx,
            &principal,
            input.expected_version,
            document,
            bindings,
            Vec::new(),
            super::PublicationChange::Normal {
                action: "vendor.delete",
                target: json!({"id":id}),
            },
        )
        .await?;
    Ok(Json(json!({"success":true,"data":{"version":version}})))
}
pub(in crate::channel_http) async fn models(
    State(state): State<ChannelState>,
    headers: HeaderMap,
) -> Result<Json<Value>, Failure> {
    super::super::admit(&state, &headers).await?;
    let management = state.management.as_ref().ok_or(Failure::Disabled)?;
    management.synchronize().await?;
    let _guard = management.publication.lock().await;
    let (tx, _) = management
        .begin(
            bearer(&headers)?,
            Action::ChannelSensitiveWrite,
            super::ReadView::Current,
        )
        .await?;
    let bindings = management.current_bindings()?;
    let document = management
        .routing
        .catalog_document()
        .map_err(|_| ManagementError::Storage)?;
    let items=document.routing.routes.iter().map(|route|json!({"route":route,"metadata":bindings.models.get(&route.id),"prices":document.pricing.rules.iter().filter(|p|p.model==route.model && p.group==route.group).collect::<Vec<_>>()})).collect::<Vec<_>>();
    tx.commit().await.map_err(ManagementError::from)?;
    Ok(Json(
        json!({"success":true,"data":{"version":Version::of(&document),"items":items,"discoveries":bindings.discoveries}}),
    ))
}
#[derive(Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub(in crate::channel_http) struct ModelInput {
    expected_version: Version,
    route: ModelRoute,
    metadata: ModelMetadata,
    prices: Vec<PriceRule>,
    contexts: Vec<ExternalModelContext>,
    #[serde(default)]
    confirmation_digest: Option<String>,
}
impl ModelInput {
    fn digest(&self, editing: bool) -> Result<String, ManagementError> {
        let bytes = serde_json::to_vec(&(
            "lmm-model-publication-v1",
            editing,
            self.expected_version,
            &self.route,
            &self.metadata,
            &self.prices,
            &self.contexts,
        ))
        .map_err(|_| ManagementError::Invalid)?;
        Ok(crate::hex_codec::encode(Sha256::digest(bytes)))
    }
    fn compile(
        &self,
        management: &ManagedChannels,
        editing: bool,
    ) -> Result<(ReleaseDocument, Bindings), ManagementError> {
        let mut document = next_document(management, self.expected_version)?;
        let previous = document
            .routing
            .routes
            .iter()
            .find(|r| r.id == self.route.id)
            .cloned();
        let mut bindings = management.current_bindings()?;
        let official = bindings
            .official_prices
            .as_ref()
            .filter(|prices| prices.models.contains_key(&self.route.model));
        if previous.is_some() != editing
            || (official.is_none() && self.prices.is_empty())
            || self
                .prices
                .iter()
                .any(|p| p.model != self.route.model || p.group != self.route.group)
            || self.contexts.len() > 1000
        {
            return Err(ManagementError::Invalid);
        }
        if let Some(old) = previous {
            document.routing.routes.retain(|r| r.id != old.id);
            if old.model != self.route.model || old.group != self.route.group {
                document
                    .pricing
                    .rules
                    .retain(|p| p.model != old.model || p.group != old.group);
            }
            // Preserve aliases while changing the canonical model identifier.
            for alias in &mut document.routing.aliases {
                if alias.model == old.model && alias.group == old.group {
                    alias.model = self.route.model.clone();
                    alias.group = self.route.group.clone();
                }
            }
        }
        document.routing.routes.push(self.route.clone());
        bindings.apply_channel_target_defaults(&mut document.routing);
        document.pricing.version = self
            .expected_version
            .pricing
            .checked_add(1)
            .filter(|v| *v <= i64::MAX as u64)
            .ok_or(ManagementError::Invalid)?;
        if let Some(official) = official {
            document.pricing = official.compile(&document.routing, &document.pricing)?;
        } else if !self.prices.is_empty() {
            document
                .pricing
                .rules
                .retain(|p| p.model != self.route.model || p.group != self.route.group);
            document.pricing.rules.extend(self.prices.clone());
        }
        bindings.models.insert(self.route.id, self.metadata.clone());
        for context in &self.contexts {
            if !self.route.targets.iter().any(|t| {
                t.upstream_id == context.upstream_id && t.upstream_model == context.upstream_model
            }) {
                return Err(ManagementError::Invalid);
            }
            bindings.contexts.retain(|old| {
                old.upstream_id != context.upstream_id
                    || old.upstream_model != context.upstream_model
            });
            bindings.contexts.push(context.clone());
        }
        promote_pending(&document, &mut bindings)?;
        management.validate_bindings(&document, &bindings, &BTreeSet::new())?;
        Ok((document, bindings))
    }
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(in crate::channel_http) struct ModelPreviewInput {
    editing: bool,
    model: ModelInput,
}
pub(in crate::channel_http) async fn preview_model(
    State(state): State<ChannelState>,
    headers: HeaderMap,
    Json(input): Json<ModelPreviewInput>,
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
    let (document, bindings) = input.model.compile(management, input.editing)?;
    let digest = input.model.digest(input.editing)?;
    tx.rollback().await.map_err(ManagementError::from)?;
    Ok(Json(
        json!({"success":true,"data":{"saved":false,"version":Version::of(&document),"routing":document.routing,"prices":document.pricing,"bindings":bindings,"confirmation_digest":digest}}),
    ))
}
pub(in crate::channel_http) async fn create_model(
    State(state): State<ChannelState>,
    headers: HeaderMap,
    Json(input): Json<ModelInput>,
) -> Result<Json<Value>, Failure> {
    save_model(&state, &headers, input, false).await
}
pub(in crate::channel_http) async fn update_model(
    State(state): State<ChannelState>,
    headers: HeaderMap,
    Path(id): Path<u64>,
    Json(input): Json<ModelInput>,
) -> Result<Json<Value>, Failure> {
    if input.route.id != id {
        return Err(ManagementError::Invalid.into());
    }
    save_model(&state, &headers, input, true).await
}
async fn save_model(
    state: &ChannelState,
    headers: &HeaderMap,
    input: ModelInput,
    editing: bool,
) -> Result<Json<Value>, Failure> {
    super::super::admit(state, headers).await?;
    let management = state.management.as_ref().ok_or(Failure::Disabled)?;
    let _guard = management.publication.lock().await;
    let (tx, principal) = management
        .begin(
            bearer(headers)?,
            Action::ChannelSensitiveWrite,
            super::ReadView::Current,
        )
        .await?;
    let digest = input.digest(editing)?;
    if input.confirmation_digest.as_deref() != Some(digest.as_str()) {
        return Err(ManagementError::Invalid.into());
    }
    let (document, bindings) = input.compile(management, editing)?;
    let version = management
        .publish_bindings(
            tx,
            &principal,
            input.expected_version,
            document,
            bindings,
            Vec::new(),
            super::PublicationChange::Normal {
                action: if editing {
                    "model.update"
                } else {
                    "model.create"
                },
                target: json!({"id":input.route.id,"confirmation_digest":digest}),
            },
        )
        .await?;
    Ok(Json(
        json!({"success":true,"data":{"version":version,"id":input.route.id}}),
    ))
}
pub(in crate::channel_http) async fn remove_model(
    State(state): State<ChannelState>,
    headers: HeaderMap,
    Path(id): Path<u64>,
    Json(input): Json<DeleteInput>,
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
    let old = document
        .routing
        .routes
        .iter()
        .find(|r| r.id == id)
        .cloned()
        .ok_or(Failure::NotFound)?;
    document.routing.routes.retain(|r| r.id != id);
    document
        .routing
        .aliases
        .retain(|a| a.model != old.model || a.group != old.group);
    document
        .pricing
        .rules
        .retain(|p| p.model != old.model || p.group != old.group);
    document.pricing.version = input
        .expected_version
        .pricing
        .checked_add(1)
        .filter(|v| *v <= i64::MAX as u64)
        .ok_or(ManagementError::Invalid)?;
    let mut bindings = management.current_bindings()?;
    bindings.models.remove(&id);
    let version = management
        .publish_bindings(
            tx,
            &principal,
            input.expected_version,
            document,
            bindings,
            Vec::new(),
            super::PublicationChange::Normal {
                action: "model.delete",
                target: json!({"id":id}),
            },
        )
        .await?;
    Ok(Json(json!({"success":true,"data":{"version":version}})))
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(in crate::channel_http) struct PendingInput {
    expected_version: Version,
    models: Vec<String>,
}

pub(in crate::channel_http) async fn save_pending(
    State(state): State<ChannelState>,
    headers: HeaderMap,
    Path(id): Path<u64>,
    Json(input): Json<PendingInput>,
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
    let document = next_document(management, input.expected_version)?;
    let mut bindings = management.current_bindings()?;
    let discovery = bindings.discoveries.get_mut(&id).ok_or(Failure::NotFound)?;
    if input.models.len() > 10_000 {
        return Err(ManagementError::Invalid.into());
    }
    let observed: BTreeSet<_> = discovery.models.iter().map(String::as_str).collect();
    let count = input.models.len();
    let selected: BTreeSet<_> = input.models.into_iter().collect();
    if selected.len() != count
        || selected.iter().any(|model| {
            !(observed.contains(model.as_str()) || discovery.pending_models.contains(model))
        })
        || enabled_targets(&document)
            .any(|(channel, model, _)| channel == id && selected.contains(model))
    {
        return Err(ManagementError::Invalid.into());
    }
    discovery.pending_models = selected.clone();
    let version = management
        .publish_bindings(
            tx,
            &principal,
            input.expected_version,
            document,
            bindings,
            Vec::new(),
            super::PublicationChange::Normal {
                action: "channel.models.pending",
                target: json!({"id":id, "model_count":count}),
            },
        )
        .await?;
    Ok(Json(
        json!({"success":true,"data":{"version":version,"models":selected,"routes_published":false}}),
    ))
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(in crate::channel_http) struct SynchronizeInput {
    expected_version: Version,
    expected_models: Option<BTreeSet<String>>,
    reason: Option<super::configuration::PublicationReason>,
}

pub(in crate::channel_http) async fn synchronize_models(
    State(state): State<ChannelState>,
    headers: HeaderMap,
    Path(id): Path<u64>,
    Json(input): Json<SynchronizeInput>,
) -> Result<Json<Value>, Failure> {
    // No lock/transaction survives the remote request. Reauthenticate and CAS
    // afterward; a changed endpoint cannot publish an old provider's discovery.
    let discovery = super::channel_probe(&state, &headers, id).await?;
    let observed_models: BTreeSet<_> = discovery.models.iter().cloned().collect();
    if input.reason.is_some() != input.expected_models.is_some() {
        return Err(ManagementError::Invalid.into());
    }
    if input
        .expected_models
        .as_ref()
        .is_some_and(|expected| expected != &observed_models)
    {
        return Err(ManagementError::Conflict.into());
    }
    let management = state.management.as_ref().ok_or(Failure::Disabled)?;
    let _guard = management.publication.lock().await;
    let (mut tx, principal) = management
        .begin(
            bearer(&headers)?,
            Action::ChannelOperate,
            super::ReadView::Current,
        )
        .await?;
    let document = next_document(management, input.expected_version)?;
    let observed: i64 =
        sqlx::query_scalar("SELECT floor(extract(epoch FROM clock_timestamp()))::bigint")
            .fetch_one(&mut *tx)
            .await
            .map_err(ManagementError::from)?;
    let mut bindings = management.current_bindings()?;
    let previous_models: BTreeSet<_> = bindings
        .discoveries
        .get(&id)
        .into_iter()
        .flat_map(|row| row.models.iter().cloned())
        .collect();
    let pending_models = bindings
        .discoveries
        .remove(&id)
        .map(|d| d.pending_models)
        .unwrap_or_default();
    let count = discovery.models.len();
    bindings.discoveries.insert(
        id,
        Discovery {
            models: discovery.models.clone(),
            observed_at: observed,
            pending_models,
        },
    );
    let version = management
        .publish_bindings(
            tx,
            &principal,
            input.expected_version,
            document,
            bindings,
            Vec::new(),
            super::PublicationChange::Normal {
                action: "channel.models.synchronize",
                target: json!({"id":id,"model_count":count,"reason":input.reason,"added":observed_models.difference(&previous_models).collect::<Vec<_>>(),"removed":previous_models.difference(&observed_models).collect::<Vec<_>>()}),
            },
        )
        .await?;
    Ok(Json(
        json!({"success":true,"data":{"version":version,"models":discovery.models,"observed_at":observed,"routes_published":false,"prices_inferred":false}}),
    ))
}

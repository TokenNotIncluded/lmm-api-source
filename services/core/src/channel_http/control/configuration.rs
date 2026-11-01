//! Native control-plane documents and secret lifecycle. All mutations retain
//! the current Session/policy locks, version CAS and audit through one commit.
use super::*;
use crate::{
    model_http::ExternalModelContext,
    routing::{
        CredentialRef, Endpoint, PayloadPolicy, Protocol, RouteTarget, RoutingConfig, Speed,
    },
};
use rand::{RngCore, rngs::OsRng};
use std::collections::BTreeMap;

#[derive(Clone, Default, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub(in crate::channel_http) struct Bindings {
    #[serde(default)]
    pub names: BTreeMap<u64, String>,
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
}
impl Bindings {
    pub(in crate::channel_http) fn validate(
        &self,
        _document: &ReleaseDocument,
    ) -> Result<(), ManagementError> {
        if self.names.len() > 100_000
            || self.contexts.len() > 100_000
            || self
                .names
                .iter()
                .any(|(id, name)| *id == 0 || *id > i64::MAX as u64 || !valid_label(name))
        {
            return Err(ManagementError::Invalid);
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
        super::catalog::validate_metadata(self, _document)?;
        Ok(())
    }
}
fn valid_label(name: &str) -> bool {
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
        Ok(format!("{:x}", Sha256::digest(value)))
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
    let (tx, _) = management
        .begin(bearer(&headers)?, Action::ChannelSensitiveWrite)
        .await?;
    let document = management
        .routing
        .catalog_document()
        .map_err(|_| ManagementError::Storage)?;
    let bindings = management.current_bindings()?;
    tx.commit().await.map_err(ManagementError::from)?;
    Ok(Json(
        json!({"success":true,"data":{"version":Version::of(&document),"routing":document.routing,"prices":document.pricing,"bindings":bindings,"secret_storage_enabled":management.gateway.as_ref().and_then(|g|g.channel_secret_key()).is_some()}}),
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
        .begin(bearer(&headers)?, Action::ChannelSensitiveWrite)
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
        .begin(bearer(&headers)?, Action::ChannelSensitiveWrite)
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
            "channel.configuration.publish",
            json!({"confirmation_digest":digest}),
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
    let mut seen = BTreeSet::new();
    for route in &mut document.routing.routes {
        route.targets.retain(|t| t.upstream_id != id);
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
        route.targets.push(RouteTarget {
            id: next,
            upstream_id: id,
            upstream_model: attachment.upstream_model,
            enabled: true,
            priority: attachment.priority,
            weight: attachment.weight,
            speeds: attachment.speeds,
            payload: attachment.payload,
            timeout_ms: attachment.timeout_ms,
        });
    }
    Ok(())
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(in crate::channel_http) struct ChannelInput {
    expected_version: Version,
    id: u64,
    name: String,
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
    if !valid_label(label) {
        return Err(ManagementError::Invalid.into());
    }
    let gateway = management.gateway.as_ref().ok_or(Failure::Disabled)?;
    let key = gateway.channel_secret_key().ok_or(Failure::Disabled)?;
    let mut random = [0_u8; 8];
    OsRng
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
        .begin(bearer(&headers)?, Action::ChannelSensitiveWrite)
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
            "channel.create",
            json!({"id":input.id}),
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
        .begin(bearer(&headers)?, Action::ChannelSensitiveWrite)
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
            "channel.update",
            json!({"id":id}),
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
        .begin(bearer(&headers)?, Action::ChannelSensitiveWrite)
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
    let list: Value = sqlx::query_scalar("SELECT public.routing_control_secret_list($1)")
        .bind(id as i64)
        .fetch_one(&mut *tx)
        .await
        .map_err(ManagementError::from)?;
    tx.commit().await.map_err(ManagementError::from)?;
    Ok(Json(
        json!({"success":true,"data":{"version":Version::of(&document),"active_reference":upstream.credential,"keys":list,"plaintext_available":false}}),
    ))
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
    Activate {
        expected_version: Version,
        reference: CredentialRef,
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
        .begin(bearer(&headers)?, Action::ChannelSensitiveWrite)
        .await?;
    let expected = match &input {
        KeyOperation::Add {
            expected_version, ..
        }
        | KeyOperation::Activate {
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
    let mut credentials = Vec::new();
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
            (reference, "channel.key.activate")
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
            (reference, "channel.key.remove")
        }
    };
    let version = management
        .publish_bindings(
            tx,
            &principal,
            expected,
            document,
            management.current_bindings()?,
            credentials,
            action,
            json!({"id":id,"reference":reference}),
        )
        .await?;
    Ok(Json(
        json!({"success":true,"data":{"version":version,"reference":reference}}),
    ))
}

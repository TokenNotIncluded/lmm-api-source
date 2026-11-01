//! Transactional channel controls. PostgreSQL is the publication authority;
//! the serving snapshot changes only after its audit and release commit.
pub(super) mod accounting;
pub(super) mod balance;
pub(super) mod catalog;
pub(super) mod configuration;
pub(super) mod copy;
pub(super) mod price_drafts;
mod vault;
use super::{ChannelState, auth_error, authorize_action, error_response};
use crate::{
    history_read::HistoryError,
    identity::{self, permissions::Action},
    model_http::ModelGateway,
    routing::{
        PriceConfig, PriceSnapshot, ReleaseVersion, Router as ServingRouter, store::ReleaseDocument,
    },
};
use axum::{
    Json,
    extract::{Path, State},
    http::{HeaderMap, StatusCode, header},
    response::{IntoResponse, Response},
};
use configuration::Bindings;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use sqlx::{PgPool, Postgres, Transaction};
use std::{collections::BTreeSet, sync::Arc};

#[derive(Debug)]
pub enum ManagementError {
    Storage,
    Invalid,
    Conflict,
    BindingChanged,
    CommitUnknown,
    ApplyPending,
}
impl std::fmt::Display for ManagementError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "channel management: {self:?}")
    }
}
impl std::error::Error for ManagementError {}
impl From<sqlx::Error> for ManagementError {
    fn from(error: sqlx::Error) -> Self {
        match error.as_database_error().and_then(|e| e.code()).as_deref() {
            Some("40001" | "23505") => Self::Conflict,
            _ => Self::Storage,
        }
    }
}

#[derive(Clone, Copy, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub(super) struct Version {
    routing: u64,
    pricing: u64,
}
impl From<ReleaseVersion> for Version {
    fn from(v: ReleaseVersion) -> Self {
        Self {
            routing: v.routing,
            pricing: v.pricing,
        }
    }
}
impl Version {
    fn of(document: &ReleaseDocument) -> Self {
        Self {
            routing: document.routing.version,
            pricing: document.pricing.version,
        }
    }
}

pub struct ManagedChannels {
    pool: PgPool,
    routing: Arc<ServingRouter>,
    bootstrap: ReleaseDocument,
    gateway: Option<ModelGateway>,
    publication: tokio::sync::Mutex<()>,
    bindings: std::sync::RwLock<(u64, Bindings)>,
}

pub(super) enum PublicationChange {
    Normal {
        action: &'static str,
        target: Value,
    },
    Copy {
        source_id: u64,
        id: u64,
        reset_balance: bool,
    },
}
pub(super) enum ReadView {
    Current,
    Snapshot,
}
impl PublicationChange {
    fn audit(&self) -> (&'static str, Value) {
        match self {
            Self::Normal { action, target } => (action, target.clone()),
            Self::Copy {
                source_id,
                id,
                reset_balance,
            } => (
                "channel.copy",
                json!({"source_id":source_id,"id":id,"reset_balance":reset_balance}),
            ),
        }
    }
}
impl ManagedChannels {
    /// Called by the host before it exposes routes. This reads existing state;
    /// it never initializes a database or creates a publication at startup.
    pub async fn recover(pool: PgPool, gateway: ModelGateway) -> Result<Self, ManagementError> {
        let mut state = Self::from_router(pool, gateway.dashboard_routing())?;
        state.bindings = std::sync::RwLock::new((
            state.bootstrap.routing.version,
            Bindings {
                contexts: gateway.channel_contexts(),
                ..Bindings::default()
            },
        ));
        state.gateway = Some(gateway);
        if let Some(document) = state.load_active().await? {
            state.install(document).await?;
        }
        Ok(state)
    }
    fn from_router(pool: PgPool, routing: Arc<ServingRouter>) -> Result<Self, ManagementError> {
        let bootstrap = routing
            .catalog_document()
            .map_err(|_| ManagementError::Invalid)?;
        let version = bootstrap.routing.version;
        Ok(Self {
            pool,
            routing,
            bootstrap,
            gateway: None,
            publication: tokio::sync::Mutex::new(()),
            bindings: std::sync::RwLock::new((version, Bindings::default())),
        })
    }
    pub(super) fn routing(&self) -> Arc<ServingRouter> {
        self.routing.clone()
    }

    async fn load_active(&self) -> Result<Option<ReleaseDocument>, ManagementError> {
        let value: Option<Value> = sqlx::query_scalar("SELECT public.routing_control_read()")
            .fetch_one(&self.pool)
            .await?;
        value
            .map(|v| serde_json::from_value(v).map_err(|_| ManagementError::Invalid))
            .transpose()
    }
    pub async fn synchronize(&self) -> Result<(), ManagementError> {
        let _guard = self.publication.lock().await;
        if let Some(document) = self.load_active().await? {
            self.install(document).await?;
        }
        Ok(())
    }

    fn current_bindings(&self) -> Result<Bindings, ManagementError> {
        self.bindings
            .read()
            .map(|guard| guard.1.clone())
            .map_err(|_| ManagementError::Storage)
    }

    pub(super) fn display_names(
        &self,
        version: u64,
    ) -> Result<std::collections::BTreeMap<u64, String>, ManagementError> {
        let (current, bindings) = &*self.bindings.read().map_err(|_| ManagementError::Storage)?;
        if *current != version {
            return Err(ManagementError::ApplyPending);
        }
        Ok(bindings.names.clone())
    }

    pub(super) fn secret_storage_enabled(&self) -> bool {
        self.gateway
            .as_ref()
            .and_then(|gateway| gateway.channel_secret_key())
            .is_some()
    }

    fn validate(&self, document: &ReleaseDocument) -> Result<(), ManagementError> {
        self.validate_bindings(document, &self.current_bindings()?, &BTreeSet::new())
    }

    fn validate_bindings(
        &self,
        document: &ReleaseDocument,
        bindings: &Bindings,
        new_keys: &BTreeSet<(u64, u64)>,
    ) -> Result<(), ManagementError> {
        bindings.validate(document)?;
        document
            .clone()
            .into_router()
            .map_err(|_| ManagementError::Invalid)?;
        crate::model_http::channel_operations::validate_publication_prices(document)
            .map_err(|_| ManagementError::Invalid)?;
        if let Some(gateway) = &self.gateway {
            return gateway
                .validate_channel_release(document, bindings.contexts.clone(), new_keys)
                .map_err(|_| ManagementError::BindingChanged);
        }
        // Runtime edits cannot create an unconfigured credential or provider
        // context. Existing keys/endpoints remain bound to the same upstream ID.
        for upstream in &document.routing.upstreams {
            let original = self
                .bootstrap
                .routing
                .upstreams
                .iter()
                .find(|u| u.id == upstream.id)
                .ok_or(ManagementError::BindingChanged)?;
            if upstream.endpoint != original.endpoint
                || upstream.protocol != original.protocol
                || upstream.credential != original.credential
            {
                return Err(ManagementError::BindingChanged);
            }
        }
        for route in &document.routing.routes {
            for target in &route.targets {
                if !self
                    .bootstrap
                    .routing
                    .routes
                    .iter()
                    .flat_map(|r| &r.targets)
                    .any(|original| {
                        original.upstream_id == target.upstream_id
                            && original.upstream_model == target.upstream_model
                    })
                {
                    return Err(ManagementError::BindingChanged);
                }
            }
        }
        document
            .clone()
            .into_router()
            .map_err(|_| ManagementError::Invalid)?;
        Ok(())
    }

    async fn install(&self, document: ReleaseDocument) -> Result<(), ManagementError> {
        let raw: Option<Value> = sqlx::query_scalar("SELECT public.routing_control_bindings($1)")
            .bind(document.routing.version as i64)
            .fetch_one(&self.pool)
            .await?;
        let bindings = match raw {
            Some(value) => serde_json::from_value(value).map_err(|_| ManagementError::Invalid)?,
            None => Bindings {
                contexts: self
                    .gateway
                    .as_ref()
                    .map(|g| g.channel_contexts())
                    .unwrap_or_default(),
                ..Bindings::default()
            },
        };
        let mut credentials = Vec::new();
        if let Some(gateway) = &self.gateway {
            for upstream in &document.routing.upstreams {
                let reference = upstream.credential;
                if gateway.has_channel_credential(reference) {
                    continue;
                }
                let encrypted: Option<Vec<u8>> =
                    sqlx::query_scalar("SELECT public.routing_control_secret_read($1,$2)")
                        .bind(reference.id as i64)
                        .bind(reference.revision as i64)
                        .fetch_one(&self.pool)
                        .await?;
                if encrypted.is_none() && !upstream.enabled {
                    continue;
                }
                let key = gateway
                    .channel_secret_key()
                    .ok_or(ManagementError::BindingChanged)?;
                let secret = vault::decrypt(
                    key,
                    reference,
                    &encrypted.ok_or(ManagementError::BindingChanged)?,
                )?;
                credentials.push(crate::model_http::UpstreamCredential { reference, secret });
            }
        }
        let references = credentials
            .iter()
            .map(|c| (c.reference.id, c.reference.revision))
            .collect();
        self.validate_bindings(&document, &bindings, &references)?;
        if let Some(gateway) = &self.gateway {
            gateway
                .install_channel_bindings(
                    document.routing.version,
                    credentials,
                    bindings.contexts.clone(),
                )
                .map_err(|_| ManagementError::BindingChanged)?;
        }
        let current = self
            .routing
            .catalog_document()
            .map_err(|_| ManagementError::Storage)?;
        let version = document.routing.version;
        if current != document {
            document
                .apply(&self.routing)
                .map_err(|_| ManagementError::ApplyPending)?;
        }
        *self
            .bindings
            .write()
            .map_err(|_| ManagementError::Storage)? = (version, bindings);
        if let Some(gateway) = &self.gateway {
            gateway
                .refresh_catalog()
                .map_err(|_| ManagementError::ApplyPending)?;
        }
        Ok(())
    }
    async fn begin<'a>(
        &'a self,
        secret: &str,
        action: Action,
        view: ReadView,
    ) -> Result<(Transaction<'a, Postgres>, identity::Principal), Failure> {
        let mut tx = self.pool.begin().await.map_err(ManagementError::from)?;
        if matches!(view, ReadView::Snapshot) {
            sqlx::query("SET TRANSACTION ISOLATION LEVEL REPEATABLE READ")
                .execute(&mut *tx)
                .await
                .map_err(ManagementError::from)?;
        }
        sqlx::query("SET LOCAL statement_timeout='5s'")
            .execute(&mut *tx)
            .await
            .map_err(ManagementError::from)?;
        sqlx::query("SET LOCAL lock_timeout='500ms'")
            .execute(&mut *tx)
            .await
            .map_err(ManagementError::from)?;
        let principal = identity::authorize_in_transaction(&mut tx, secret)
            .await
            .map_err(HistoryError::from)?;
        authorize_action(&mut tx, &principal, action).await?;
        Ok((tx, principal))
    }
    async fn publish(
        &self,
        tx: Transaction<'_, Postgres>,
        principal: &identity::Principal,
        expected: Version,
        document: ReleaseDocument,
        action: &'static str,
        target: Value,
    ) -> Result<Version, Failure> {
        let bindings = self.current_bindings()?;
        self.publish_bindings(
            tx,
            principal,
            expected,
            document,
            bindings,
            Vec::new(),
            PublicationChange::Normal { action, target },
        )
        .await
    }

    #[allow(clippy::too_many_arguments)]
    async fn publish_bindings(
        &self,
        mut tx: Transaction<'_, Postgres>,
        principal: &identity::Principal,
        expected: Version,
        document: ReleaseDocument,
        bindings: Bindings,
        incoming_keys: Vec<crate::model_http::UpstreamCredential>,
        change: PublicationChange,
    ) -> Result<Version, Failure> {
        if let Some(gateway) = &self.gateway {
            gateway
                .validate_channel_credentials(&incoming_keys)
                .map_err(|_| ManagementError::Invalid)?;
        }
        let refs = incoming_keys
            .iter()
            .map(|c| (c.reference.id, c.reference.revision))
            .collect();
        self.validate_bindings(&document, &bindings, &refs)?;
        let active: Option<Value> = sqlx::query_scalar("SELECT public.routing_control_read()")
            .fetch_one(&mut *tx)
            .await
            .map_err(ManagementError::from)?;
        let durable: Option<ReleaseDocument> = active
            .map(serde_json::from_value)
            .transpose()
            .map_err(|_| ManagementError::Invalid)?;
        let current = durable.as_ref().unwrap_or(&self.bootstrap);
        if Version::of(current) != expected {
            return Err(ManagementError::Conflict.into());
        }
        if document.routing.version <= expected.routing
            || document.pricing.version < expected.pricing
        {
            return Err(ManagementError::Conflict.into());
        }
        accounting::publish(&mut tx, self, current, &document, &change).await?;
        let routing =
            serde_json::to_value(&document.routing).map_err(|_| ManagementError::Invalid)?;
        let prices =
            serde_json::to_value(&document.pricing).map_err(|_| ManagementError::Invalid)?;
        sqlx::query("SELECT public.routing_control_publish($1,$2,$3,$4,$5)")
            .bind(routing)
            .bind(prices)
            .bind(serde_json::to_value(&bindings).map_err(|_| ManagementError::Invalid)?)
            .bind(durable.as_ref().map(|d| d.routing.version as i64))
            .bind(durable.as_ref().map(|d| d.pricing.version as i64))
            .execute(&mut *tx)
            .await
            .map_err(ManagementError::from)?;
        let (action, target) = change.audit();
        sqlx::query("INSERT INTO core_identity.audit(actor_user_id,credential_id,action,target) VALUES ($1,$2,$3,$4)")
            .bind(principal.user_id).bind(principal.credential_id).bind(action)
            .bind(json!({"before":expected,"after":Version::of(&document),"change":target}))
            .execute(&mut *tx).await.map_err(ManagementError::from)?;
        tx.commit()
            .await
            .map_err(|_| ManagementError::CommitUnknown)?;
        let version = Version::of(&document);
        self.install(document).await?;
        Ok(version)
    }
}

pub(super) enum Failure {
    Auth(HistoryError),
    Management(ManagementError),
    NotFound,
    Disabled,
    AccountingUnavailable,
    RetiredIdentity,
    InvalidUsageWindow,
    AccountingIdentityAmbiguous,
    AccountingLimit,
}
impl From<HistoryError> for Failure {
    fn from(v: HistoryError) -> Self {
        Self::Auth(v)
    }
}
impl From<ManagementError> for Failure {
    fn from(v: ManagementError) -> Self {
        Self::Management(v)
    }
}
impl IntoResponse for Failure {
    fn into_response(self) -> Response {
        let (status, code, message) = match self {
            Self::Auth(error) => return auth_error(error),
            Self::AccountingUnavailable => (
                StatusCode::SERVICE_UNAVAILABLE,
                "CHANNEL_ACCOUNTING_UNAVAILABLE",
                "Channel accounting cannot be verified",
            ),
            Self::RetiredIdentity => (
                StatusCode::CONFLICT,
                "CHANNEL_IDENTITY_RETIRED",
                "This channel ID has already been used; select a new ID",
            ),
            Self::InvalidUsageWindow => (
                StatusCode::BAD_REQUEST,
                "CHANNEL_USAGE_WINDOW_INVALID",
                "Select a valid time window of at most 31 days",
            ),
            Self::AccountingIdentityAmbiguous => (
                StatusCode::CONFLICT,
                "CHANNEL_USAGE_IDENTITY_AMBIGUOUS",
                "This channel ID was reused; its historical accounting requires review",
            ),
            Self::AccountingLimit => (
                StatusCode::UNPROCESSABLE_ENTITY,
                "CHANNEL_ACCOUNTING_LIMIT",
                "This accounting snapshot exceeds the supported request or receipt limit",
            ),
            Self::Disabled => (
                StatusCode::NOT_IMPLEMENTED,
                "CHANNEL_CONTROL_DISABLED",
                "Channel control is not configured",
            ),
            Self::NotFound => (
                StatusCode::NOT_FOUND,
                "CHANNEL_NOT_FOUND",
                "Channel does not exist",
            ),
            Self::Management(ManagementError::Invalid) => (
                StatusCode::BAD_REQUEST,
                "CHANNEL_CONTROL_INVALID",
                "Invalid routing or price configuration",
            ),
            Self::Management(ManagementError::BindingChanged) => (
                StatusCode::CONFLICT,
                "CHANNEL_BINDING_NOT_CONFIGURED",
                "Configure the provider context and secret before publishing this binding",
            ),
            Self::Management(ManagementError::Conflict) => (
                StatusCode::CONFLICT,
                "CHANNEL_VERSION_CONFLICT",
                "Configuration changed; reload before saving",
            ),
            Self::Management(ManagementError::CommitUnknown) => (
                StatusCode::SERVICE_UNAVAILABLE,
                "CHANNEL_COMMIT_UNKNOWN",
                "Save result is unknown; reload before retrying",
            ),
            Self::Management(ManagementError::ApplyPending) => (
                StatusCode::SERVICE_UNAVAILABLE,
                "CHANNEL_APPLY_PENDING",
                "The release is saved; reload the Core configuration before accepting traffic",
            ),
            Self::Management(ManagementError::Storage) => (
                StatusCode::SERVICE_UNAVAILABLE,
                "CHANNEL_CONTROL_UNAVAILABLE",
                "Channel configuration storage is unavailable",
            ),
        };
        error_response(status, code, message)
    }
}
fn bearer(headers: &HeaderMap) -> Result<&str, Failure> {
    if headers.get_all(header::AUTHORIZATION).iter().count() != 1 {
        return Err(HistoryError::Unauthorized.into());
    }
    let raw = headers
        .get(header::AUTHORIZATION)
        .and_then(|v| v.to_str().ok())
        .ok_or(HistoryError::Unauthorized)?;
    let (scheme, secret) = raw.split_once(' ').ok_or(HistoryError::Unauthorized)?;
    if !scheme.eq_ignore_ascii_case("Bearer") || secret.is_empty() {
        return Err(HistoryError::Unauthorized.into());
    }
    Ok(secret)
}
fn next_document(
    state: &ManagedChannels,
    expected: Version,
) -> Result<ReleaseDocument, ManagementError> {
    let mut document = state
        .routing
        .catalog_document()
        .map_err(|_| ManagementError::Storage)?;
    if Version::of(&document) != expected {
        return Err(ManagementError::Conflict);
    }
    document.routing.version = expected
        .routing
        .checked_add(1)
        .filter(|v| *v <= i64::MAX as u64)
        .ok_or(ManagementError::Invalid)?;
    Ok(document)
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct StatusInput {
    expected_version: Version,
    enabled: bool,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct BatchInput {
    expected_version: Version,
    ids: Vec<u64>,
    enabled: bool,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct DeleteInput {
    expected_version: Version,
}

pub(super) async fn status(
    State(state): State<ChannelState>,
    headers: HeaderMap,
    Path(id): Path<u64>,
    Json(input): Json<StatusInput>,
) -> Result<Json<Value>, Failure> {
    change_status(
        &state,
        &headers,
        BatchInput {
            expected_version: input.expected_version,
            ids: vec![id],
            enabled: input.enabled,
        },
    )
    .await
}
pub(super) async fn batch_status(
    State(state): State<ChannelState>,
    headers: HeaderMap,
    Json(input): Json<BatchInput>,
) -> Result<Json<Value>, Failure> {
    change_status(&state, &headers, input).await
}
async fn change_status(
    state: &ChannelState,
    headers: &HeaderMap,
    input: BatchInput,
) -> Result<Json<Value>, Failure> {
    super::admit(state, headers).await?;
    let management = state.management.as_ref().ok_or(Failure::Disabled)?;
    if input.ids.is_empty()
        || input.ids.len() > 1000
        || input.ids.iter().copied().collect::<BTreeSet<_>>().len() != input.ids.len()
    {
        return Err(ManagementError::Invalid.into());
    }
    let _guard = management.publication.lock().await;
    let (tx, principal) = management
        .begin(bearer(headers)?, Action::ChannelOperate, ReadView::Current)
        .await?;
    let mut document = next_document(management, input.expected_version)?;
    for id in &input.ids {
        let channel = document
            .routing
            .upstreams
            .iter_mut()
            .find(|u| u.id == *id)
            .ok_or(Failure::NotFound)?;
        channel.enabled = input.enabled;
    }
    let version = management
        .publish(
            tx,
            &principal,
            input.expected_version,
            document,
            "channel.status",
            json!({"ids":input.ids,"enabled":input.enabled}),
        )
        .await?;
    Ok(Json(json!({"success":true,"data":{"version":version}})))
}
pub(super) async fn remove(
    State(state): State<ChannelState>,
    headers: HeaderMap,
    Path(id): Path<u64>,
    body: axum::body::Bytes,
) -> Result<Json<Value>, Failure> {
    super::admit(&state, &headers).await?;
    let management = state.management.as_ref().ok_or(Failure::Disabled)?;
    let input: DeleteInput = serde_json::from_slice(&body).map_err(|_| ManagementError::Invalid)?;
    let _guard = management.publication.lock().await;
    let (tx, principal) = management
        .begin(bearer(&headers)?, Action::ChannelWrite, ReadView::Current)
        .await?;
    let mut document = next_document(management, input.expected_version)?;
    if !document.routing.upstreams.iter().any(|u| u.id == id) {
        return Err(Failure::NotFound);
    }
    document.routing.upstreams.retain(|u| u.id != id);
    for route in &mut document.routing.routes {
        route.targets.retain(|t| t.upstream_id != id);
    }
    let mut bindings = management.current_bindings()?;
    bindings.names.remove(&id);
    bindings
        .contexts
        .retain(|context| context.upstream_id != id);
    bindings.discoveries.remove(&id);
    bindings.balance_probes.remove(&id);
    let version = management
        .publish_bindings(
            tx,
            &principal,
            input.expected_version,
            document,
            bindings,
            Vec::new(),
            PublicationChange::Normal {
                action: "channel.delete",
                target: json!({"id":id}),
            },
        )
        .await?;
    Ok(Json(json!({"success":true,"data":{"version":version}})))
}
pub(super) async fn prices(
    State(state): State<ChannelState>,
    headers: HeaderMap,
) -> Result<Json<Value>, Failure> {
    super::admit(&state, &headers).await?;
    let management = state.management.as_ref().ok_or(Failure::Disabled)?;
    management.synchronize().await?;
    let _guard = management.publication.lock().await;
    let (tx, _) = management
        .begin(
            bearer(&headers)?,
            Action::ChannelSensitiveWrite,
            ReadView::Current,
        )
        .await?;
    let document = management
        .routing
        .catalog_document()
        .map_err(|_| ManagementError::Storage)?;
    tx.commit().await.map_err(ManagementError::from)?;
    Ok(Json(
        json!({"success":true,"data":{"version":Version::of(&document),"prices":document.pricing}}),
    ))
}
#[derive(Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub(super) struct PriceInput {
    expected_version: Version,
    prices: PriceConfig,
    confirmation_digest: String,
}
fn price_confirmation(expected: Version, prices: &PriceConfig) -> Result<String, ManagementError> {
    let bytes = serde_json::to_vec(&("lmm-price-publication-v1", expected, prices))
        .map_err(|_| ManagementError::Invalid)?;
    Ok(format!("{:x}", Sha256::digest(bytes)))
}
pub(super) async fn publish_prices(
    State(state): State<ChannelState>,
    headers: HeaderMap,
    Json(input): Json<PriceInput>,
) -> Result<Json<Value>, Failure> {
    super::admit(&state, &headers).await?;
    let management = state.management.as_ref().ok_or(Failure::Disabled)?;
    let _guard = management.publication.lock().await;
    let (tx, principal) = management
        .begin(
            bearer(&headers)?,
            Action::ChannelSensitiveWrite,
            ReadView::Current,
        )
        .await?;
    let mut document = next_document(management, input.expected_version)?;
    if input.prices.version <= input.expected_version.pricing
        || input.confirmation_digest != price_confirmation(input.expected_version, &input.prices)?
    {
        return Err(ManagementError::Invalid.into());
    }
    PriceSnapshot::compile(input.prices.clone()).map_err(|_| ManagementError::Invalid)?;
    document.pricing = input.prices;
    crate::model_http::channel_operations::validate_publication_prices(&document)
        .map_err(|_| ManagementError::Invalid)?;
    let version = management
        .publish(
            tx,
            &principal,
            input.expected_version,
            document,
            "channel.prices.publish",
            json!({"confirmation_digest":input.confirmation_digest}),
        )
        .await?;
    Ok(Json(json!({"success":true,"data":{"version":version}})))
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct PricePreviewInput {
    expected_version: Version,
    prices: PriceConfig,
}
pub(super) async fn preview_prices(
    State(state): State<ChannelState>,
    headers: HeaderMap,
    Json(input): Json<PricePreviewInput>,
) -> Result<Json<Value>, Failure> {
    super::admit(&state, &headers).await?;
    let management = state.management.as_ref().ok_or(Failure::Disabled)?;
    management.synchronize().await?;
    let _guard = management.publication.lock().await;
    let (tx, _) = management
        .begin(
            bearer(&headers)?,
            Action::ChannelSensitiveWrite,
            ReadView::Current,
        )
        .await?;
    let mut document = next_document(management, input.expected_version)?;
    if input.prices.version <= input.expected_version.pricing {
        return Err(ManagementError::Invalid.into());
    }
    let before = document.pricing.clone();
    document.pricing = input.prices;
    management.validate(&document)?;
    crate::model_http::channel_operations::validate_publication_prices(&document)
        .map_err(|_| ManagementError::Invalid)?;
    let confirmation = price_confirmation(input.expected_version, &document.pricing)?;
    tx.rollback().await.map_err(ManagementError::from)?;
    Ok(Json(
        json!({"success":true,"data":{"saved":false,"expected_version":input.expected_version,
        "before":before,"after":document.pricing,"confirmation_digest":confirmation}}),
    ))
}

async fn channel_probe(
    state: &ChannelState,
    headers: &HeaderMap,
    id: u64,
) -> Result<crate::model_http::channel_operations::ChannelProbe, Failure> {
    super::admit(state, headers).await?;
    let management = state.management.as_ref().ok_or(Failure::Disabled)?;
    let (tx, _) = management
        .begin(bearer(headers)?, Action::ChannelOperate, ReadView::Current)
        .await?;
    let gateway = management.gateway.as_ref().ok_or(Failure::Disabled)?;
    tx.commit().await.map_err(ManagementError::from)?;
    gateway
        .probe_channel(id)
        .await
        .map_err(|_| ManagementError::Storage.into())
}
pub(super) async fn probe(
    State(state): State<ChannelState>,
    headers: HeaderMap,
    Path(id): Path<u64>,
) -> Result<Json<Value>, Failure> {
    let result = channel_probe(&state, &headers, id).await?;
    Ok(Json(
        json!({"success":true,"time":result.response_time_ms as f64/1000.0,"data":result}),
    ))
}
pub(super) async fn fetch_models(
    State(state): State<ChannelState>,
    headers: HeaderMap,
    Path(id): Path<u64>,
) -> Result<Json<Value>, Failure> {
    let result = channel_probe(&state, &headers, id).await?;
    Ok(Json(
        json!({"success":true,"data":result.models,"validation_scope":result.validation_scope,"inference_tested":false}),
    ))
}

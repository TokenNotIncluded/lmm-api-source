//! Transactional channel controls. PostgreSQL is the publication authority;
//! the serving snapshot changes only after its audit and release commit.
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
struct Version {
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
}
impl ManagedChannels {
    /// Called by the host before it exposes routes. This reads existing state;
    /// it never initializes a database or creates a publication at startup.
    pub async fn recover(pool: PgPool, gateway: ModelGateway) -> Result<Self, ManagementError> {
        let mut state = Self::from_router(pool, gateway.dashboard_routing())?;
        state.gateway = Some(gateway);
        if let Some(document) = state.load_active().await? {
            state.install(document)?;
        }
        Ok(state)
    }
    fn from_router(pool: PgPool, routing: Arc<ServingRouter>) -> Result<Self, ManagementError> {
        let bootstrap = routing
            .catalog_document()
            .map_err(|_| ManagementError::Invalid)?;
        Ok(Self {
            pool,
            routing,
            bootstrap,
            gateway: None,
            publication: tokio::sync::Mutex::new(()),
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
            self.install(document)?;
        }
        Ok(())
    }

    fn validate(&self, document: &ReleaseDocument) -> Result<(), ManagementError> {
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
    fn install(&self, document: ReleaseDocument) -> Result<(), ManagementError> {
        self.validate(&document)?;
        let current = self
            .routing
            .catalog_document()
            .map_err(|_| ManagementError::Storage)?;
        if current != document {
            document
                .apply(&self.routing)
                .map_err(|_| ManagementError::ApplyPending)?;
        }
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
    ) -> Result<(Transaction<'a, Postgres>, identity::Principal), Failure> {
        let mut tx = self.pool.begin().await.map_err(ManagementError::from)?;
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
        mut tx: Transaction<'_, Postgres>,
        principal: &identity::Principal,
        expected: Version,
        document: ReleaseDocument,
        action: &'static str,
        target: Value,
    ) -> Result<Version, Failure> {
        self.validate(&document)?;
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
        let routing =
            serde_json::to_value(&document.routing).map_err(|_| ManagementError::Invalid)?;
        let prices =
            serde_json::to_value(&document.pricing).map_err(|_| ManagementError::Invalid)?;
        sqlx::query("SELECT public.routing_control_publish($1,$2,$3,$4)")
            .bind(routing)
            .bind(prices)
            .bind(durable.as_ref().map(|d| d.routing.version as i64))
            .bind(durable.as_ref().map(|d| d.pricing.version as i64))
            .execute(&mut *tx)
            .await
            .map_err(ManagementError::from)?;
        sqlx::query("INSERT INTO core_identity.audit(actor_user_id,credential_id,action,target) VALUES ($1,$2,$3,$4)")
            .bind(principal.user_id).bind(principal.credential_id).bind(action)
            .bind(json!({"before":expected,"after":Version::of(&document),"change":target}))
            .execute(&mut *tx).await.map_err(ManagementError::from)?;
        tx.commit()
            .await
            .map_err(|_| ManagementError::CommitUnknown)?;
        let version = Version::of(&document);
        self.install(document)?;
        Ok(version)
    }
}

pub(super) enum Failure {
    Auth(HistoryError),
    Management(ManagementError),
    NotFound,
    Disabled,
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
        .begin(bearer(headers)?, Action::ChannelOperate)
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
    if input.enabled {
        management
            .gateway
            .as_ref()
            .ok_or(ManagementError::BindingChanged)?
            .validate_channel_activation(&document, &input.ids.iter().copied().collect())
            .map_err(|_| ManagementError::BindingChanged)?;
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
        .begin(bearer(&headers)?, Action::ChannelWrite)
        .await?;
    let mut document = next_document(management, input.expected_version)?;
    if !document.routing.upstreams.iter().any(|u| u.id == id) {
        return Err(Failure::NotFound);
    }
    document.routing.upstreams.retain(|u| u.id != id);
    for route in &mut document.routing.routes {
        route.targets.retain(|t| t.upstream_id != id);
    }
    let version = management
        .publish(
            tx,
            &principal,
            input.expected_version,
            document,
            "channel.delete",
            json!({"id":id}),
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
    let (tx, _) = management
        .begin(bearer(&headers)?, Action::ChannelSensitiveWrite)
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
        .begin(bearer(&headers)?, Action::ChannelSensitiveWrite)
        .await?;
    let mut document = next_document(management, input.expected_version)?;
    if input.prices.version <= input.expected_version.pricing
        || input.confirmation_digest != price_confirmation(input.expected_version, &input.prices)?
    {
        return Err(ManagementError::Invalid.into());
    }
    PriceSnapshot::compile(input.prices.clone()).map_err(|_| ManagementError::Invalid)?;
    document.pricing = input.prices;
    // The installed routing SQL still requires a positive group multiplier.
    // Reject before confirmation/publication until its explicit schema upgrade.
    if document
        .pricing
        .rules
        .iter()
        .any(|rule| rule.group_multiplier_ppm == 0)
    {
        return Err(ManagementError::Invalid.into());
    }
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
    let (tx, _) = management
        .begin(bearer(&headers)?, Action::ChannelSensitiveWrite)
        .await?;
    let mut document = next_document(management, input.expected_version)?;
    if input.prices.version <= input.expected_version.pricing {
        return Err(ManagementError::Invalid.into());
    }
    let before = document.pricing.clone();
    document.pricing = input.prices;
    management.validate(&document)?;
    if document
        .pricing
        .rules
        .iter()
        .any(|rule| rule.group_multiplier_ppm == 0)
    {
        return Err(ManagementError::Invalid.into());
    }
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
        .begin(bearer(headers)?, Action::ChannelOperate)
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

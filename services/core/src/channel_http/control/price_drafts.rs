//! Durable price drafts share the immutable release metadata. Saving a draft
//! leaves the active price version unchanged; only a confirmed publication
//! replaces the executable tariff.
use super::*;

#[derive(Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(in crate::channel_http) struct PriceDraft {
    pub id: u64,
    pub name: String,
    pub base_pricing_version: u64,
    pub prices: PriceConfig,
    pub updated_by: i64,
    pub updated_at: i64,
}
impl PriceDraft {
    pub(super) fn validate(&self) -> Result<(), ManagementError> {
        if self.id == 0
            || self.id > i64::MAX as u64
            || self.name.trim().is_empty()
            || self.name.len() > 128
            || self.name.chars().any(char::is_control)
            || self.updated_by <= 0
            || !(1..=253_402_300_799).contains(&self.updated_at)
            || self.base_pricing_version.checked_add(1) != Some(self.prices.version)
        {
            return Err(ManagementError::Invalid);
        }
        PriceSnapshot::compile(self.prices.clone()).map_err(|_| ManagementError::Invalid)?;
        Ok(())
    }
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(in crate::channel_http) struct SaveDraftInput {
    expected_version: Version,
    name: String,
    prices: PriceConfig,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(in crate::channel_http) struct PublishDraftInput {
    expected_version: Version,
    confirmation_digest: String,
}
pub(in crate::channel_http) async fn list(
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
    let document = management
        .routing
        .catalog_document()
        .map_err(|_| ManagementError::Storage)?;
    let bindings = management.current_bindings()?;
    let items: Vec<_> = bindings.price_drafts.into_values().map(|draft| {
        json!({"stale":draft.base_pricing_version != document.pricing.version,"draft":draft})
    }).collect();
    tx.commit().await.map_err(ManagementError::from)?;
    Ok(Json(
        json!({"success":true,"data":{"version":Version::of(&document),"items":items}}),
    ))
}
pub(in crate::channel_http) async fn save(
    State(state): State<ChannelState>,
    headers: HeaderMap,
    Path(id): Path<u64>,
    Json(input): Json<SaveDraftInput>,
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
    let document = next_document(management, input.expected_version)?;
    let now: i64 =
        sqlx::query_scalar("SELECT floor(extract(epoch FROM clock_timestamp()))::bigint")
            .fetch_one(&mut *tx)
            .await
            .map_err(ManagementError::from)?;
    let draft = PriceDraft {
        id,
        name: input.name,
        base_pricing_version: input.expected_version.pricing,
        prices: input.prices,
        updated_by: principal.user_id,
        updated_at: now,
    };
    draft.validate()?;
    let mut proposed = document.clone();
    proposed.pricing = draft.prices.clone();
    management.validate(&proposed)?;
    let mut bindings = management.current_bindings()?;
    bindings.price_drafts.insert(id, draft);
    let version = management
        .publish_bindings(
            tx,
            &principal,
            input.expected_version,
            document,
            bindings,
            Vec::new(),
            super::PublicationChange::Normal {
                action: "channel.prices.draft.save",
                target: json!({"id":id}),
            },
        )
        .await?;
    Ok(Json(
        json!({"success":true,"data":{"id":id,"version":version,"published":false}}),
    ))
}
fn prepare(
    management: &ManagedChannels,
    expected: Version,
    id: u64,
) -> Result<(ReleaseDocument, Bindings, String), Failure> {
    let mut document = next_document(management, expected)?;
    let mut bindings = management.current_bindings()?;
    let draft = bindings.price_drafts.remove(&id).ok_or(Failure::NotFound)?;
    if draft.base_pricing_version != expected.pricing {
        return Err(ManagementError::Conflict.into());
    }
    document.pricing = draft.prices;
    management.validate_bindings(&document, &bindings, &BTreeSet::new())?;
    let digest = price_confirmation(expected, &document.pricing)?;
    Ok((document, bindings, digest))
}
pub(in crate::channel_http) async fn preview(
    State(state): State<ChannelState>,
    headers: HeaderMap,
    Path(id): Path<u64>,
    Json(input): Json<DeleteInput>,
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
    let (document, _, digest) = prepare(management, input.expected_version, id)?;
    tx.rollback().await.map_err(ManagementError::from)?;
    Ok(Json(
        json!({"success":true,"data":{"saved":false,"expected_version":input.expected_version,
        "prices":document.pricing,"confirmation_digest":digest}}),
    ))
}
pub(in crate::channel_http) async fn publish(
    State(state): State<ChannelState>,
    headers: HeaderMap,
    Path(id): Path<u64>,
    Json(input): Json<PublishDraftInput>,
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
    let (document, bindings, digest) = prepare(management, input.expected_version, id)?;
    if input.confirmation_digest != digest {
        return Err(ManagementError::Invalid.into());
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
                action: "channel.prices.draft.publish",
                target: json!({"id":id,"confirmation_digest":digest}),
            },
        )
        .await?;
    Ok(Json(
        json!({"success":true,"data":{"version":version,"published":true}}),
    ))
}
pub(in crate::channel_http) async fn remove(
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
    if bindings.price_drafts.remove(&id).is_none() {
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
                action: "channel.prices.draft.delete",
                target: json!({"id":id}),
            },
        )
        .await?;
    Ok(Json(json!({"success":true,"data":{"version":version}})))
}

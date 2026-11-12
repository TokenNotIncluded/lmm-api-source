//! Channel labels share the durable routing publication, permission locks and audit.
use super::*;

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(in crate::channel_http) struct BatchTagInput {
    ids: Vec<u64>,
    tag: Option<String>,
    #[serde(default)]
    expected_version: Option<Version>,
}

pub(in crate::channel_http) async fn batch(
    State(state): State<ChannelState>,
    headers: HeaderMap,
    Json(input): Json<BatchTagInput>,
) -> Result<Json<Value>, Failure> {
    super::super::admit(&state, &headers).await?;
    let management = state.management.as_ref().ok_or(Failure::Disabled)?;
    if input.ids.is_empty()
        || input.ids.len() > 1000
        || input.ids.iter().copied().collect::<BTreeSet<_>>().len() != input.ids.len()
    {
        return Err(ManagementError::Invalid.into());
    }
    let tag = input.tag.filter(|value| !value.is_empty());
    if tag
        .as_deref()
        .is_some_and(|value| !configuration::valid_label(value))
    {
        return Err(ManagementError::Invalid.into());
    }
    let _guard = management.publication.lock().await;
    let (tx, principal) = management
        .begin(bearer(&headers)?, Action::ChannelWrite, ReadView::Current)
        .await?;
    let expected = match input.expected_version {
        Some(version) => version,
        None => Version::from(
            management
                .routing
                .version()
                .map_err(|_| ManagementError::Storage)?,
        ),
    };
    let document = next_document(management, expected)?;
    let mut bindings = management.current_bindings()?;
    for id in &input.ids {
        if !document
            .routing
            .upstreams
            .iter()
            .any(|upstream| upstream.id == *id)
        {
            return Err(Failure::NotFound);
        }
        match &tag {
            Some(tag) => {
                bindings.tags.insert(*id, tag.clone());
            }
            None => {
                bindings.tags.remove(id);
            }
        }
    }
    let version = management
        .publish_bindings(
            tx,
            &principal,
            expected,
            document,
            bindings,
            Vec::new(),
            PublicationChange::Normal {
                action: "channel.tag_batch_set",
                target: json!({"ids": input.ids, "tag": tag}),
            },
        )
        .await?;
    Ok(Json(
        json!({"success":true,"data":input.ids.len(),"version":version}),
    ))
}

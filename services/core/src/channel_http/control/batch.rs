//! Atomic updates for the fields shared by channel cards and batch editing.
use super::*;

#[derive(Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub(in crate::channel_http) struct Patch {
    enabled: Option<bool>,
    tag: Option<String>,
    groups: Option<Vec<String>>,
    priority: Option<u16>,
    weight: Option<u32>,
    timeout_ms: Option<u32>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(in crate::channel_http) struct Input {
    expected_version: Version,
    ids: Vec<u64>,
    patch: Patch,
}

pub(in crate::channel_http) async fn configure(
    State(state): State<ChannelState>,
    headers: HeaderMap,
    Json(input): Json<Input>,
) -> Result<Json<Value>, Failure> {
    super::super::admit(&state, &headers).await?;
    let management = state.management.as_ref().ok_or(Failure::Disabled)?;
    let ids = input.ids.iter().copied().collect::<BTreeSet<_>>();
    let patch = &input.patch;
    let changes_targets =
        patch.priority.is_some() || patch.weight.is_some() || patch.timeout_ms.is_some();
    if input.ids.is_empty()
        || input.ids.len() > 1000
        || ids.len() != input.ids.len()
        || ids.iter().any(|id| *id == 0 || *id > i64::MAX as u64)
        || (!changes_targets
            && patch.enabled.is_none()
            && patch.tag.is_none()
            && patch.groups.is_none())
        || patch
            .tag
            .as_deref()
            .is_some_and(|tag| !tag.is_empty() && !configuration::valid_label(tag))
        || patch
            .timeout_ms
            .is_some_and(|timeout| !(1..=3_600_000).contains(&timeout))
    {
        return Err(ManagementError::Invalid.into());
    }
    let mut actions = [
        (patch.enabled.is_some(), Action::ChannelOperate),
        (
            patch.tag.is_some() || patch.groups.is_some(),
            Action::ChannelWrite,
        ),
        (changes_targets, Action::ChannelSensitiveWrite),
    ]
    .into_iter()
    .filter_map(|(required, action)| required.then_some(action));
    let first = actions.next().ok_or(ManagementError::Invalid)?;
    let _guard = management.publication.lock().await;
    let (mut tx, principal) = management
        .begin(bearer(&headers)?, first, ReadView::Current)
        .await?;
    for action in actions {
        authorize_action(&mut tx, &principal, action).await?;
    }
    let mut document = next_document(management, input.expected_version)?;
    let mut bindings = management.current_bindings()?;
    let assigned_groups = patch
        .groups
        .as_ref()
        .map(|names| {
            let groups = names.iter().cloned().collect::<BTreeSet<_>>();
            let available = document
                .routing
                .groups
                .iter()
                .map(|group| group.name.as_str())
                .collect::<BTreeSet<_>>();
            if groups.is_empty()
                || groups.len() != names.len()
                || groups.iter().any(|name| !available.contains(name.as_str()))
            {
                return Err(ManagementError::Invalid);
            }
            Ok(groups)
        })
        .transpose()?;
    for id in &ids {
        let upstream = document
            .routing
            .upstreams
            .iter_mut()
            .find(|upstream| upstream.id == *id)
            .ok_or(Failure::NotFound)?;
        if let Some(enabled) = patch.enabled {
            upstream.enabled = enabled;
        }
        if let Some(groups) = &assigned_groups {
            bindings.channel_groups.insert(*id, groups.clone());
        }
        if let Some(tag) = &patch.tag {
            if tag.is_empty() {
                bindings.tags.remove(id);
            } else {
                bindings.tags.insert(*id, tag.clone());
            }
        }
    }
    let mut target_count = 0;
    let mut configured = BTreeSet::new();
    for target in document
        .routing
        .routes
        .iter_mut()
        .flat_map(|route| &mut route.targets)
        .filter(|target| ids.contains(&target.upstream_id))
    {
        configured.insert(target.upstream_id);
        target_count += 1;
        if let Some(priority) = patch.priority {
            target.priority = priority;
        }
        if let Some(weight) = patch.weight {
            target.weight = weight;
        }
        if let Some(timeout_ms) = patch.timeout_ms {
            target.timeout_ms = timeout_ms;
        }
    }
    if changes_targets && configured != ids {
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
            PublicationChange::Normal {
                action: "channel.configuration_batch_set",
                target: json!({"ids":input.ids,"patch":input.patch}),
            },
        )
        .await?;
    Ok(Json(json!({
        "success":true,
        "data":{"version":version,"updated":ids.len(),"target_count":target_count}
    })))
}

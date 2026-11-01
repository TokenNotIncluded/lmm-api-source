//! Channel copies publish fresh identities, independent keys and matching route bindings.
use super::{Failure, ManagementError, Version, accounting, bearer, configuration, next_document};
use crate::{channel_http::ChannelState, identity::permissions::Action};
use axum::{
    Json,
    extract::{Path, Query, State},
    http::HeaderMap,
};
use serde::Deserialize;
use serde_json::{Value, json};

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(in crate::channel_http) struct CopyInput {
    expected_version: Version,
    suffix: String,
    #[serde(default = "reset_counters")]
    reset_balance: bool,
}
fn reset_counters() -> bool {
    true
}

#[derive(Default, Deserialize)]
#[serde(deny_unknown_fields)]
pub(in crate::channel_http) struct LegacyInput {
    suffix: Option<String>,
    reset_balance: Option<String>,
}

pub(in crate::channel_http) async fn native(
    State(state): State<ChannelState>,
    headers: HeaderMap,
    Path(id): Path<u64>,
    Json(input): Json<CopyInput>,
) -> Result<Json<Value>, Failure> {
    duplicate(
        &state,
        &headers,
        id,
        Some(input.expected_version),
        &input.suffix,
        input.reset_balance,
    )
    .await
}

pub(in crate::channel_http) async fn legacy(
    State(state): State<ChannelState>,
    headers: HeaderMap,
    Path(id): Path<u64>,
    Query(input): Query<LegacyInput>,
) -> Result<Json<Value>, Failure> {
    duplicate(
        &state,
        &headers,
        id,
        None,
        input.suffix.as_deref().unwrap_or("_复制"),
        !matches!(
            input.reset_balance.as_deref(),
            Some("0" | "f" | "F" | "false" | "False" | "FALSE")
        ),
    )
    .await
}

async fn duplicate(
    state: &ChannelState,
    headers: &HeaderMap,
    id: u64,
    expected: Option<Version>,
    suffix: &str,
    reset_balance: bool,
) -> Result<Json<Value>, Failure> {
    super::super::admit(state, headers).await?;
    let management = state.management.as_ref().ok_or(Failure::Disabled)?;
    let _guard = management.publication.lock().await;
    let (mut tx, principal) = management
        .begin(
            bearer(headers)?,
            Action::ChannelSensitiveWrite,
            super::ReadView::Snapshot,
        )
        .await?;
    let expected = match expected {
        Some(expected) => expected,
        None => Version::from(
            management
                .routing
                .version()
                .map_err(|_| ManagementError::Storage)?,
        ),
    };
    let mut document = next_document(management, expected)?;
    let mut channel = document
        .routing
        .upstreams
        .iter()
        .find(|u| u.id == id)
        .cloned()
        .ok_or(Failure::NotFound)?;
    accounting::ensure_identities(management, &mut tx).await?;
    accounting::usable(&mut tx, id).await?;
    let copied_id = accounting::maximum_id(&mut tx)
        .await?
        .checked_add(1)
        .filter(|id| *id < (1_u64 << 53))
        .ok_or(ManagementError::Invalid)?;
    let mut bindings = management.current_bindings()?;
    let name = bindings
        .names
        .get(&id)
        .cloned()
        .unwrap_or_else(|| format!("Channel {id}"))
        + suffix;
    if name.is_empty() || name.len() > 128 || name.chars().any(char::is_control) {
        return Err(ManagementError::Invalid.into());
    }
    let secret = management
        .gateway
        .as_ref()
        .ok_or(Failure::Disabled)?
        .channel_credential(channel.credential)
        .ok_or(ManagementError::BindingChanged)?;
    let key =
        configuration::store_secret(management, &mut tx, copied_id, "Primary", secret).await?;
    channel.id = copied_id;
    channel.credential = key.reference;
    document.routing.upstreams.push(channel);
    bindings.names.insert(copied_id, name);
    if let Some(probe) = bindings.balance_probes.get(&id).cloned() {
        bindings.balance_probes.insert(copied_id, probe);
    }
    let contexts: Vec<_> = bindings
        .contexts
        .iter()
        .filter(|c| c.upstream_id == id)
        .cloned()
        .map(|mut context| {
            context.upstream_id = copied_id;
            context
        })
        .collect();
    bindings.contexts.extend(contexts);
    let mut next_target = document
        .routing
        .routes
        .iter()
        .flat_map(|r| &r.targets)
        .map(|t| t.id)
        .max()
        .unwrap_or(0);
    for route in &mut document.routing.routes {
        let mut targets = Vec::new();
        for target in route.targets.iter().filter(|t| t.upstream_id == id) {
            next_target = next_target
                .checked_add(1)
                .filter(|id| *id <= i64::MAX as u64)
                .ok_or(ManagementError::Invalid)?;
            let mut copied = target.clone();
            copied.id = next_target;
            copied.upstream_id = copied_id;
            targets.push(copied);
        }
        route.targets.extend(targets);
    }
    let version = management
        .publish_bindings(
            tx,
            &principal,
            expected,
            document,
            bindings,
            vec![key],
            super::PublicationChange::Copy {
                source_id: id,
                id: copied_id,
                reset_balance,
            },
        )
        .await?;
    Ok(Json(
        json!({"success":true,"data":{"id":copied_id,"version":version}}),
    ))
}

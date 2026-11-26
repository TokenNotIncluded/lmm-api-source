//! Administrative aggregate read and audited cache eviction.
use super::*;
use crate::routing::affinity::store;
use axum::extract::Query;

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(in crate::channel_http) struct Eviction {
    all: Option<bool>,
    rule_name: Option<String>,
}
pub(in crate::channel_http) async fn stats(
    State(state): State<ChannelState>,
    headers: HeaderMap,
) -> Result<Json<Value>, Failure> {
    super::super::admit(&state, &headers).await?;
    let management = state.management.as_ref().ok_or(Failure::Disabled)?;
    let config = state
        .routing
        .affinity_config()
        .map_err(|_| ManagementError::Storage)?;
    let stats = store::stats(&management.pool, config.as_ref())
        .await
        .map_err(ManagementError::from)?;
    Ok(Json(json!({"success":true,"data":stats})))
}
pub(in crate::channel_http) async fn clear(
    State(state): State<ChannelState>,
    headers: HeaderMap,
    Query(query): Query<Eviction>,
) -> Result<Json<Value>, Failure> {
    let management = state.management.as_ref().ok_or(Failure::Disabled)?;
    let (mut tx, actor) = management
        .begin(
            bearer(&headers)?,
            Action::ChannelSensitiveWrite,
            ReadView::Current,
        )
        .await?;
    let rule = match (query.all, query.rule_name.as_deref()) {
        (Some(true), None) => None,
        (None, Some(name))
            if !name.trim().is_empty()
                && name.len() <= 128
                && !name.chars().any(char::is_control) =>
        {
            Some(name)
        }
        _ => return Err(ManagementError::Invalid.into()),
    };
    let cleared = store::clear(&mut tx, rule)
        .await
        .map_err(ManagementError::from)?;
    sqlx::query("INSERT INTO core_identity.audit(actor_user_id,credential_id,action,target) VALUES($1,$2,'option.clear_affinity_cache',$3)")
        .bind(actor.user_id).bind(actor.credential_id).bind(json!({"rule_name":rule,"cleared":cleared}))
        .execute(&mut *tx).await.map_err(ManagementError::from)?;
    tx.commit()
        .await
        .map_err(|_| ManagementError::CommitUnknown)?;
    Ok(Json(json!({"success":true,"data":{"cleared":cleared}})))
}

//! Assistant model selection is a versioned binding on the existing routing release.
use super::*;
use crate::assistant_model::PersonalTextPolicy;
use axum::{Router, routing::get};

#[derive(Clone)]
struct Settings {
    gateway: ModelGateway,
    default_model: String,
    group: String,
    max_output_tokens: u32,
}

pub fn router(gateway: ModelGateway, policy: &PersonalTextPolicy) -> Router {
    Router::new()
        .route("/api/option/assistant-model", get(read).put(save))
        .layer(axum::extract::DefaultBodyLimit::max(2048))
        .layer(axum::middleware::from_fn(super::super::no_store))
        .with_state(Settings {
            gateway,
            default_model: policy.model.clone(),
            group: policy.group.clone(),
            max_output_tokens: policy.max_output_tokens,
        })
}

impl Settings {
    async fn manager(&self, token: &str) -> Result<Arc<ManagedChannels>, Failure> {
        let manager = self.gateway.managed_channels().ok_or(Failure::Disabled)?;
        let (check, _) = manager
            .begin(token, Action::ChannelSensitiveWrite, ReadView::Snapshot)
            .await?;
        check.commit().await.map_err(ManagementError::from)?;
        manager.synchronize_if_changed().await?;
        Ok(manager)
    }

    fn available(&self, group: &str, model: &str) -> bool {
        self.gateway
            .validate_assistant_policy(model, group, self.max_output_tokens)
            .is_ok()
            && self.gateway.assistant_quote(model, group).is_ok()
    }

    fn project(&self, manager: &ManagedChannels, version: Version) -> Result<Json<Value>, Failure> {
        let (group, model) = manager.assistant_route()?;
        let group = group.as_deref().unwrap_or(&self.group);
        let model = model.as_deref().unwrap_or(&self.default_model);
        let document = manager
            .routing
            .catalog_document()
            .map_err(|_| ManagementError::Storage)?;
        let mut routes = std::collections::BTreeMap::new();
        for name in document
            .routing
            .groups
            .iter()
            .filter(|g| g.enabled)
            .map(|g| g.name.as_str())
        {
            let models = manager
                .routing
                .catalog(name)
                .map_err(|_| ManagementError::Storage)?
                .into_iter()
                .map(|item| item.name)
                .filter(|model| self.available(name, model))
                .collect::<BTreeSet<_>>();
            if !models.is_empty() {
                routes.insert(name.to_owned(), models);
            }
        }
        let models = routes.get(group).cloned().unwrap_or_default();
        Ok(Json(
            json!({"success":true, "data":{"model":model, "default_model":self.default_model,
            "group":group, "models":models, "routes":routes, "version":version}}),
        ))
    }
}

async fn read(
    State(settings): State<Settings>,
    headers: HeaderMap,
) -> Result<Json<Value>, Failure> {
    let token = bearer(&headers)?;
    let manager = settings.manager(token).await?;
    let _guard = manager.publication.lock().await;
    let version = manager
        .routing
        .version()
        .map_err(|_| ManagementError::Storage)?
        .into();
    settings.project(&manager, version)
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Selection {
    expected_version: Version,
    model: String,
    #[serde(default)]
    group: Option<String>,
}

async fn save(
    State(settings): State<Settings>,
    headers: HeaderMap,
    Json(input): Json<Selection>,
) -> Result<Json<Value>, Failure> {
    let token = bearer(&headers)?;
    let manager = settings.manager(token).await?;
    let _guard = manager.publication.lock().await;
    let (tx, actor) = manager
        .begin(token, Action::ChannelSensitiveWrite, ReadView::Current)
        .await?;
    let document = next_document(&manager, input.expected_version)?;
    let group = input.group.as_deref().unwrap_or(&settings.group);
    if !settings.available(group, &input.model)
        || !manager
            .routing
            .catalog(group)
            .map_err(|_| ManagementError::Storage)?
            .iter()
            .any(|m| m.name == input.model)
    {
        return Err(ManagementError::Invalid.into());
    }
    let mut bindings = manager.current_bindings()?;
    let selection = (input.group.is_some() || input.model != settings.default_model)
        .then(|| input.model.clone());
    if input.group.is_none() && bindings.assistant_group.is_some() {
        return Err(ManagementError::Conflict.into());
    }
    if bindings.assistant_model == selection && bindings.assistant_group == input.group {
        tx.commit().await.map_err(ManagementError::from)?;
        return settings.project(&manager, input.expected_version);
    }
    bindings.assistant_model = selection;
    bindings.assistant_group = input.group.clone();
    let version = manager
        .publish_bindings(
            tx,
            &actor,
            input.expected_version,
            document,
            bindings,
            Vec::new(),
            PublicationChange::Normal {
                action: "assistant.model.select",
                target: json!({"model":input.model, "group":group}),
            },
        )
        .await?;
    settings.project(&manager, version)
}

//! Reuse the legacy custom-provider family; native builtin settings share the
//! same checked manager without ever writing legacy stored options.
use super::*;
use crate::identity::federated::ProviderWrite;
use axum::extract::{OriginalUri, Path};

pub(super) fn routes() -> Router<AuthState> {
    Router::new()
        .route("/api/custom-oauth-provider", get(list).post(create))
        .route("/api/custom-oauth-provider/", get(list).post(create))
        .route("/api/custom-oauth-provider/discovery", post(discover))
        .route(
            "/api/custom-oauth-provider/{id}",
            get(read).put(update).delete(delete),
        )
        .route("/api/user/federated-providers", get(list).post(create))
        .route("/api/user/federated-providers/discovery", post(discover))
        .route(
            "/api/user/federated-providers/{id}",
            get(read).put(update).delete(delete),
        )
        .layer(DefaultBodyLimit::max(2 * 1024 * 1024))
}
fn custom(uri: &axum::http::Uri) -> bool {
    uri.path().starts_with("/api/custom-oauth-provider")
}
fn identifier(value: &str) -> Result<i64, IdentityError> {
    let id = value.parse::<i64>().map_err(|_| IdentityError::Invalid)?;
    if id <= 0 || id.to_string() != value {
        return Err(IdentityError::Invalid);
    }
    Ok(id)
}
fn response(result: Result<Value, BrowserAuthError>) -> Response {
    match result {
        Ok(data) => success(Some(data)),
        Err(error) => auth_failure(error),
    }
}
async fn list(
    State(state): State<AuthState>,
    headers: HeaderMap,
    OriginalUri(uri): OriginalUri,
) -> Response {
    let result = async {
        state
            .browser
            .federated_providers_admin(
                bearer(&headers)?.ok_or(IdentityError::Unauthorized)?,
                custom(&uri),
                None,
            )
            .await
    }
    .await;
    response(result)
}
async fn read(
    State(state): State<AuthState>,
    headers: HeaderMap,
    OriginalUri(uri): OriginalUri,
    Path(id): Path<String>,
) -> Response {
    let result = async {
        state
            .browser
            .federated_providers_admin(
                bearer(&headers)?.ok_or(IdentityError::Unauthorized)?,
                custom(&uri),
                Some(identifier(&id)?),
            )
            .await
    }
    .await;
    response(result)
}
async fn create(
    State(state): State<AuthState>,
    headers: HeaderMap,
    OriginalUri(uri): OriginalUri,
    input: Result<Json<ProviderWrite>, JsonRejection>,
) -> Response {
    mutate(state, headers, uri, "create", None, input).await
}
async fn update(
    State(state): State<AuthState>,
    headers: HeaderMap,
    OriginalUri(uri): OriginalUri,
    Path(id): Path<String>,
    input: Result<Json<ProviderWrite>, JsonRejection>,
) -> Response {
    let id = match identifier(&id) {
        Ok(id) => id,
        Err(error) => return auth_failure(error.into()),
    };
    mutate(state, headers, uri, "update", Some(id), input).await
}
async fn delete(
    State(state): State<AuthState>,
    headers: HeaderMap,
    OriginalUri(uri): OriginalUri,
    Path(id): Path<String>,
    input: Result<Json<ProviderWrite>, JsonRejection>,
) -> Response {
    let id = match identifier(&id) {
        Ok(id) => id,
        Err(error) => return auth_failure(error.into()),
    };
    mutate(state, headers, uri, "delete", Some(id), input).await
}
async fn mutate(
    state: AuthState,
    headers: HeaderMap,
    uri: axum::http::Uri,
    action: &str,
    target: Option<i64>,
    input: Result<Json<ProviderWrite>, JsonRejection>,
) -> Response {
    let result = async {
        let Json(input) = input.map_err(|_| IdentityError::Invalid)?;
        state
            .browser
            .mutate_federated_provider(
                bearer(&headers)?.ok_or(IdentityError::Unauthorized)?,
                action,
                target,
                custom(&uri),
                &input,
            )
            .await
    }
    .await;
    response(result)
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Discovery {
    well_known_url: Option<String>,
    issuer_url: Option<String>,
}
async fn discover(
    State(state): State<AuthState>,
    headers: HeaderMap,
    input: Result<Json<Discovery>, JsonRejection>,
) -> Response {
    let result = async {
        let Json(input) = input.map_err(|_| IdentityError::Invalid)?;
        let url = match (&input.well_known_url, &input.issuer_url) {
            (Some(url), _) if !url.is_empty() => url.clone(),
            (_, Some(issuer)) if !issuer.is_empty() => format!(
                "{}/.well-known/openid-configuration",
                issuer.trim_end_matches('/')
            ),
            _ => return Err(IdentityError::Invalid.into()),
        };
        let discovery = state
            .browser
            .federated_discovery_admin(
                bearer(&headers)?.ok_or(IdentityError::Unauthorized)?,
                &url,
                input.issuer_url.as_deref(),
            )
            .await?;
        Ok(json!({"well_known_url":url,"discovery":discovery}))
    }
    .await;
    response(result)
}

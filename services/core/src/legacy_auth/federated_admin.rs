//! Reuse the legacy custom-provider family; native builtin settings share the
//! same checked manager without ever writing legacy stored options.
use super::*;
use axum::extract::{OriginalUri, Path};

pub(super) fn routes() -> Router<AuthState> {
    Router::new()
        .route("/api/custom-oauth-provider", get(list))
        .route("/api/custom-oauth-provider/", get(list))
        .route("/api/custom-oauth-provider/discovery", post(discover))
        .route("/api/custom-oauth-provider/{id}", get(read))
        .route("/api/user/federated-providers", get(list))
        .route("/api/user/federated-providers/discovery", post(discover))
        .route("/api/user/federated-providers/{id}", get(read))
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

#[cfg(test)]
mod tests {
    use super::*;
    use tower::ServiceExt;

    #[tokio::test]
    async fn provider_configuration_is_read_only_over_http() {
        let pool = sqlx::postgres::PgPoolOptions::new()
            .connect_lazy("postgres://test@localhost/test")
            .unwrap();
        let store = IdentityStore::from_pool(pool);
        let config = LegacyAuthConfig::new("https://app.example.test", &[42; 32]).unwrap();
        let app = routes().with_state(AuthState {
            browser: BrowserAuth::new(store.clone(), &config.refresh_key).unwrap(),
            store,
            config,
            email: None,
        });
        for collection in [
            "/api/custom-oauth-provider",
            "/api/custom-oauth-provider/",
            "/api/user/federated-providers",
        ] {
            for (method, path) in [
                ("POST", collection.to_owned()),
                ("PUT", format!("{}/1", collection.trim_end_matches('/'))),
                ("DELETE", format!("{}/1", collection.trim_end_matches('/'))),
            ] {
                let req = axum::http::Request::builder()
                    .method(method)
                    .uri(path.as_str())
                    .body(axum::body::Body::empty())
                    .unwrap();
                assert_eq!(
                    app.clone().oneshot(req).await.unwrap().status(),
                    StatusCode::METHOD_NOT_ALLOWED,
                    "{method} {path}"
                );
            }
        }
    }
}

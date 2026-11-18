use super::*;
use axum::{
    Json, Router,
    extract::{DefaultBodyLimit, Path, State, rejection::JsonRejection},
    http::{HeaderMap, StatusCode, header},
    response::{IntoResponse, Response},
    routing::{get, put},
};
struct Failure(ConfigurationError);
impl From<ConfigurationError> for Failure {
    fn from(error: ConfigurationError) -> Self {
        Self(error)
    }
}
impl IntoResponse for Failure {
    fn into_response(self) -> Response {
        let (status, code) = match self.0 {
            ConfigurationError::Invalid => (
                StatusCode::UNPROCESSABLE_ENTITY,
                "PAYMENT_CONFIGURATION_INVALID",
            ),
            ConfigurationError::Denied => (StatusCode::FORBIDDEN, "PAYMENT_CONFIGURATION_DENIED"),
            ConfigurationError::NotFound => {
                (StatusCode::NOT_FOUND, "PAYMENT_CONFIGURATION_NOT_FOUND")
            }
            ConfigurationError::Conflict => {
                (StatusCode::CONFLICT, "PAYMENT_CONFIGURATION_CONFLICT")
            }
            ConfigurationError::Unavailable => (
                StatusCode::SERVICE_UNAVAILABLE,
                "PAYMENT_CONFIGURATION_UNAVAILABLE",
            ),
            ConfigurationError::DurabilityUnknown => (
                StatusCode::SERVICE_UNAVAILABLE,
                "PAYMENT_CONFIGURATION_DURABILITY_UNKNOWN",
            ),
        };
        (status, Json(json!({"success":false,"error":{"code":code}}))).into_response()
    }
}
fn bearer(headers: &HeaderMap) -> std::result::Result<&str, Failure> {
    if headers.get_all(header::AUTHORIZATION).iter().count() != 1 {
        return Err(Failure(ConfigurationError::Denied));
    }
    headers
        .get(header::AUTHORIZATION)
        .and_then(|value| value.to_str().ok())
        .and_then(|value| value.strip_prefix("Bearer "))
        .filter(|value| !value.is_empty())
        .ok_or(Failure(ConfigurationError::Denied))
}
pub fn router(store: PaymentsStore) -> Router {
    Router::new()
        .route("/core/v1/payment-gateways/epay", get(read))
        .route("/core/v1/payment-gateways/epay/{id}", put(update))
        .layer(DefaultBodyLimit::max(8192))
        .layer(axum::middleware::from_fn(
            |request: axum::extract::Request, next: axum::middleware::Next| async move {
                let mut response = next.run(request).await;
                response
                    .headers_mut()
                    .insert(header::CACHE_CONTROL, "private, no-store".parse().unwrap());
                response
                    .headers_mut()
                    .insert(header::VARY, "Authorization".parse().unwrap());
                response
            },
        ))
        .with_state(store)
}
async fn read(
    State(store): State<PaymentsStore>,
    headers: HeaderMap,
) -> std::result::Result<Json<serde_json::Value>, Failure> {
    let result = store.owner_epay_configuration(bearer(&headers)?).await?;
    Ok(Json(json!({"success":true,"data":result})))
}
async fn update(
    State(store): State<PaymentsStore>,
    Path(id): Path<i64>,
    headers: HeaderMap,
    body: std::result::Result<Json<EPayUpdate>, JsonRejection>,
) -> std::result::Result<Json<serde_json::Value>, Failure> {
    let credential = bearer(&headers)?;
    let Json(input) = body.map_err(|_| Failure(ConfigurationError::Invalid))?;
    let result = store
        .update_epay_configuration(credential, id, input)
        .await?;
    Ok(Json(json!({"success":true,"data":result})))
}

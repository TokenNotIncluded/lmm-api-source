use super::*;
use axum::{
    Json, Router,
    extract::{DefaultBodyLimit, Path, State, rejection::JsonRejection},
    http::{HeaderMap, StatusCode, header},
    response::{IntoResponse, Response},
    routing::{get, post},
};
fn bearer(headers: &HeaderMap) -> Result<&str> {
    if headers.get_all(header::AUTHORIZATION).iter().count() != 1 {
        return Err(Error::Unauthorized);
    }
    headers
        .get(header::AUTHORIZATION)
        .and_then(|h| h.to_str().ok())
        .and_then(|v| v.split_once(' '))
        .filter(|(scheme, value)| {
            scheme.eq_ignore_ascii_case("Bearer")
                && !value.is_empty()
                && !value.bytes().any(|b| b.is_ascii_whitespace())
        })
        .map(|(_, value)| value)
        .ok_or(Error::Unauthorized)
}
impl IntoResponse for Error {
    fn into_response(self) -> Response {
        let (status, code) = match self {
            Self::Invalid => (StatusCode::BAD_REQUEST, "SUBSCRIPTION_INVALID"),
            Self::Unauthorized => (StatusCode::UNAUTHORIZED, "SUBSCRIPTION_UNAUTHORIZED"),
            Self::Forbidden => (StatusCode::FORBIDDEN, "SUBSCRIPTION_FORBIDDEN"),
            Self::NotFound => (StatusCode::NOT_FOUND, "SUBSCRIPTION_NOT_FOUND"),
            Self::Conflict => (StatusCode::CONFLICT, "SUBSCRIPTION_CONFLICT"),
            Self::Used => (StatusCode::CONFLICT, "SUBSCRIPTION_ALREADY_USED"),
            Self::Unavailable => (StatusCode::SERVICE_UNAVAILABLE, "SUBSCRIPTION_UNAVAILABLE"),
        };
        (status, Json(json!({"error":{"code":code}}))).into_response()
    }
}
pub fn router(pool: PgPool) -> Router {
    Router::new()
        .route("/core/v1/subscriptions/plans", get(plans))
        .route(
            "/core/v1/subscriptions/admin/plans",
            get(admin_plans).post(create),
        )
        .route(
            "/core/v1/subscriptions/admin/plans/{id}",
            axum::routing::put(update),
        )
        .route(
            "/core/v1/subscriptions/admin/plans/{id}/archive",
            post(archive),
        )
        .route("/core/v1/subscriptions/quotes", post(quote))
        .route("/core/v1/subscriptions/quotes/{id}/confirm", post(confirm))
        .route("/core/v1/subscriptions/purchases", get(purchases))
        .route(
            "/core/v1/subscriptions/purchases/{id}/cancel-renewal",
            post(cancel),
        )
        .route(
            "/core/v1/subscriptions/purchases/{id}/refund-preview",
            post(preview_refund),
        )
        .route(
            "/core/v1/subscriptions/refund-quotes/{id}/confirm",
            post(refund),
        )
        .layer(DefaultBodyLimit::max(64 * 1024))
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
        .with_state(SubscriptionStore::new(pool))
}
async fn plans(State(s): State<SubscriptionStore>, h: HeaderMap) -> Result<Json<Value>> {
    Ok(Json(
        json!({"contract":CONTRACT,"plans":s.plans(bearer(&h)?,false).await?,"payment_methods":["wallet"],"automatic_renewal":false}),
    ))
}
async fn admin_plans(State(s): State<SubscriptionStore>, h: HeaderMap) -> Result<Json<Value>> {
    Ok(Json(
        json!({"contract":CONTRACT,"plans":s.plans(bearer(&h)?,true).await?}),
    ))
}
async fn create(
    State(s): State<SubscriptionStore>,
    h: HeaderMap,
    j: std::result::Result<Json<SavePlan>, JsonRejection>,
) -> Result<Json<Plan>> {
    let secret = bearer(&h)?;
    let Json(input) = j.map_err(|_| Error::Invalid)?;
    Ok(Json(s.save_plan(secret, None, input).await?))
}
async fn update(
    State(s): State<SubscriptionStore>,
    Path(id): Path<i64>,
    h: HeaderMap,
    j: std::result::Result<Json<SavePlan>, JsonRejection>,
) -> Result<Json<Plan>> {
    let secret = bearer(&h)?;
    let Json(input) = j.map_err(|_| Error::Invalid)?;
    Ok(Json(s.save_plan(secret, Some(id), input).await?))
}
async fn archive(
    State(s): State<SubscriptionStore>,
    Path(id): Path<i64>,
    h: HeaderMap,
    j: std::result::Result<Json<ArchivePlan>, JsonRejection>,
) -> Result<Json<Plan>> {
    let secret = bearer(&h)?;
    let Json(input) = j.map_err(|_| Error::Invalid)?;
    Ok(Json(s.archive_plan(secret, id, input).await?))
}
async fn quote(
    State(s): State<SubscriptionStore>,
    h: HeaderMap,
    j: std::result::Result<Json<QuoteRequest>, JsonRejection>,
) -> Result<Json<Quote>> {
    let secret = bearer(&h)?;
    let Json(input) = j.map_err(|_| Error::Invalid)?;
    Ok(Json(s.quote(secret, input).await?))
}
async fn confirm(
    State(s): State<SubscriptionStore>,
    Path(id): Path<String>,
    h: HeaderMap,
) -> Result<Json<Receipt>> {
    Ok(Json(s.confirm(bearer(&h)?, &id).await?))
}
async fn purchases(State(s): State<SubscriptionStore>, h: HeaderMap) -> Result<Json<Value>> {
    Ok(Json(
        json!({"contract":CONTRACT,"purchases":s.purchases(bearer(&h)?).await?}),
    ))
}
async fn cancel(
    State(s): State<SubscriptionStore>,
    Path(id): Path<i64>,
    h: HeaderMap,
) -> Result<Json<Purchase>> {
    Ok(Json(s.cancel(bearer(&h)?, id).await?))
}
async fn preview_refund(
    State(s): State<SubscriptionStore>,
    Path(id): Path<i64>,
    h: HeaderMap,
) -> Result<Json<RefundQuote>> {
    Ok(Json(s.preview_refund(bearer(&h)?, id).await?))
}
async fn refund(
    State(s): State<SubscriptionStore>,
    Path(id): Path<String>,
    h: HeaderMap,
) -> Result<Json<Receipt>> {
    Ok(Json(s.refund(bearer(&h)?, &id).await?))
}

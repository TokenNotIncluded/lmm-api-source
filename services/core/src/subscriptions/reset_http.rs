//! Retained reset-voucher and root-preview entry points used by the existing Web.
use super::{reset::*, *};
use axum::{
    Json, Router,
    extract::{
        DefaultBodyLimit, Path, Query, State,
        rejection::{JsonRejection, QueryRejection},
    },
    http::{HeaderMap, header},
    response::{IntoResponse, Response},
    routing::{get, post},
};
use serde::Deserialize;

struct LegacyError(Error);
impl From<Error> for LegacyError {
    fn from(value: Error) -> Self {
        Self(value)
    }
}
impl IntoResponse for LegacyError {
    fn into_response(self) -> Response {
        let message = match self.0 {
            Error::Invalid => "SUBSCRIPTION_RESET_INVALID",
            Error::Unauthorized => "SUBSCRIPTION_UNAUTHORIZED",
            Error::Forbidden => "SUBSCRIPTION_FORBIDDEN",
            Error::NotFound => "SUBSCRIPTION_RESET_NOT_FOUND",
            Error::Conflict | Error::Used => "SUBSCRIPTION_RESET_CONFLICT",
            Error::Unavailable => "SUBSCRIPTION_UNAVAILABLE",
        };
        let status = self.0.into_response().status();
        (status, Json(json!({"success":false,"message":message}))).into_response()
    }
}
type LegacyResult = std::result::Result<Json<Value>, LegacyError>;
fn success(data: impl Serialize) -> Json<Value> {
    Json(json!({"success":true,"data":data}))
}
pub(super) fn router(pool: PgPool) -> Router {
    Router::new()
        .route("/api/subscription/self/reset-vouchers", get(list))
        .route(
            "/api/subscription/self/reset-vouchers/{id}/redeem",
            post(redeem),
        )
        .route("/api/subscription/root/reset-targets", get(eligible))
        .route("/api/subscription/root/reset/preview", post(preview))
        .route("/api/subscription/root/reset", post(execute))
        .route("/core/v1/subscriptions/reset-codes/redeem", post(code))
        .layer(DefaultBodyLimit::max(512 * 1024))
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
async fn list(State(s): State<SubscriptionStore>, h: HeaderMap) -> LegacyResult {
    Ok(success(s.reset_vouchers(super::http::bearer(&h)?).await?))
}
async fn redeem(
    State(s): State<SubscriptionStore>,
    Path(id): Path<i64>,
    h: HeaderMap,
) -> LegacyResult {
    Ok(success(
        s.redeem_reset_voucher(super::http::bearer(&h)?, id).await?,
    ))
}
#[derive(Default, Deserialize)]
#[serde(default, deny_unknown_fields)]
struct QueryFilter {
    page: Option<u32>,
    page_size: Option<u32>,
    query: String,
    plan_id: i64,
    plan_ids: String,
    user_ids: String,
}
fn ids(value: &str) -> Result<Vec<i64>> {
    if value.trim().is_empty() {
        return Ok(vec![]);
    }
    let ids = value
        .split(',')
        .map(|s| s.trim().parse::<i64>().map_err(|_| Error::Invalid))
        .collect::<Result<Vec<_>>>()?;
    if ids.len() > 100 {
        return Err(Error::Invalid);
    }
    Ok(ids)
}
async fn eligible(
    State(s): State<SubscriptionStore>,
    h: HeaderMap,
    q: std::result::Result<Query<QueryFilter>, QueryRejection>,
) -> LegacyResult {
    let secret = super::http::bearer(&h)?;
    let Query(q) = q.map_err(|_| Error::Invalid)?;
    let filter = ResetFilter {
        query: q.query,
        plan_id: q.plan_id,
        plan_ids: ids(&q.plan_ids)?,
        user_ids: ids(&q.user_ids)?,
    };
    Ok(success(
        s.reset_targets(
            secret,
            filter,
            q.page.unwrap_or(1),
            q.page_size.unwrap_or(20),
        )
        .await?,
    ))
}
async fn preview(
    State(s): State<SubscriptionStore>,
    h: HeaderMap,
    j: std::result::Result<Json<ResetPreviewRequest>, JsonRejection>,
) -> LegacyResult {
    let secret = super::http::bearer(&h)?;
    let Json(input) = j.map_err(|_| Error::Invalid)?;
    Ok(success(s.preview_reset(secret, input).await?))
}
async fn execute(
    State(s): State<SubscriptionStore>,
    h: HeaderMap,
    j: std::result::Result<Json<ResetExecuteRequest>, JsonRejection>,
) -> LegacyResult {
    let secret = super::http::bearer(&h)?;
    let Json(input) = j.map_err(|_| Error::Invalid)?;
    Ok(success(s.execute_reset(secret, input).await?))
}
async fn code(
    State(s): State<SubscriptionStore>,
    h: HeaderMap,
    j: std::result::Result<Json<ResetCodeRequest>, JsonRejection>,
) -> Result<Json<Value>> {
    let secret = super::http::bearer(&h)?;
    let Json(input) = j.map_err(|_| Error::Invalid)?;
    Ok(Json(
        s.try_redeem_reset_code(secret, input)
            .await?
            .ok_or(Error::NotFound)?,
    ))
}

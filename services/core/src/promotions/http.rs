use super::*;
use axum::{
    Json, Router,
    extract::{DefaultBodyLimit, Path, State, rejection::JsonRejection},
    http::{HeaderMap, StatusCode, header},
    response::{IntoResponse, Response},
    routing::{get, post, put},
};
fn bearer(h: &HeaderMap) -> Result<&str> {
    if h.get_all(header::AUTHORIZATION).iter().count() != 1 {
        return Err(Error::Unauthorized);
    }
    h.get(header::AUTHORIZATION)
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.split_once(' '))
        .filter(|(scheme, secret)| {
            scheme.eq_ignore_ascii_case("Bearer")
                && !secret.is_empty()
                && !secret.bytes().any(|b| b.is_ascii_whitespace())
        })
        .map(|(_, secret)| secret)
        .ok_or(Error::Unauthorized)
}
impl IntoResponse for Error {
    fn into_response(self) -> Response {
        let (status, code) = match self {
            Self::Invalid => (StatusCode::BAD_REQUEST, "PROMOTION_INVALID"),
            Self::Unauthorized => (StatusCode::UNAUTHORIZED, "PROMOTION_UNAUTHORIZED"),
            Self::Forbidden => (StatusCode::FORBIDDEN, "PROMOTION_FORBIDDEN"),
            Self::NotFound => (StatusCode::NOT_FOUND, "PROMOTION_NOT_FOUND"),
            Self::Conflict => (StatusCode::CONFLICT, "PROMOTION_CONFLICT"),
            Self::Exhausted => (StatusCode::CONFLICT, "PROMOTION_BUDGET_EXHAUSTED"),
            Self::Unavailable => (StatusCode::SERVICE_UNAVAILABLE, "PROMOTION_UNAVAILABLE"),
        };
        (status, Json(json!({"error":{"code":code}}))).into_response()
    }
}
pub fn router(pool: PgPool) -> Router {
    Router::new()
        .route("/core/v1/promotions/campaigns", get(campaigns))
        .route(
            "/core/v1/promotions/admin/campaigns",
            get(admin_campaigns).post(create),
        )
        .route("/core/v1/promotions/admin/campaigns/{id}", put(update))
        .route(
            "/core/v1/promotions/admin/campaigns/{id}/codes",
            get(codes).post(issue),
        )
        .route("/core/v1/promotions/admin/codes/{id}/revoke", post(revoke))
        .route("/core/v1/promotions/redemptions", post(redeem))
        .route("/core/v1/promotions/campaigns/{id}/checkin", post(checkin))
        .route(
            "/core/v1/promotions/campaigns/{id}/envelope",
            post(envelope),
        )
        .route("/core/v1/promotions/history", get(history))
        .layer(DefaultBodyLimit::max(16384))
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
        .with_state(PromotionStore::new(pool))
}
async fn campaigns(State(s): State<PromotionStore>, h: HeaderMap) -> Result<Json<Value>> {
    Ok(Json(
        json!({"contract":CONTRACT,"campaigns":s.campaigns(bearer(&h)?,false).await?,"timezone":"UTC","budget_basis":"authorized_promotional_credit","referral_commissions":false}),
    ))
}
async fn admin_campaigns(State(s): State<PromotionStore>, h: HeaderMap) -> Result<Json<Value>> {
    Ok(Json(
        json!({"contract":CONTRACT,"campaigns":s.campaigns(bearer(&h)?,true).await?}),
    ))
}
async fn create(
    State(s): State<PromotionStore>,
    h: HeaderMap,
    j: std::result::Result<Json<SaveCampaign>, JsonRejection>,
) -> Result<Json<Campaign>> {
    let secret = bearer(&h)?;
    let Json(input) = j.map_err(|_| Error::Invalid)?;
    Ok(Json(s.save_campaign(secret, None, input).await?))
}
async fn update(
    State(s): State<PromotionStore>,
    Path(id): Path<String>,
    h: HeaderMap,
    j: std::result::Result<Json<SaveCampaign>, JsonRejection>,
) -> Result<Json<Campaign>> {
    let secret = bearer(&h)?;
    let Json(input) = j.map_err(|_| Error::Invalid)?;
    Ok(Json(s.save_campaign(secret, Some(&id), input).await?))
}
async fn codes(
    State(s): State<PromotionStore>,
    Path(id): Path<String>,
    h: HeaderMap,
) -> Result<Json<Vec<CodeStatus>>> {
    Ok(Json(s.codes(bearer(&h)?, &id).await?))
}
async fn issue(
    State(s): State<PromotionStore>,
    Path(id): Path<String>,
    h: HeaderMap,
    j: std::result::Result<Json<IssueRequest>, JsonRejection>,
) -> Result<Json<Issue>> {
    let secret = bearer(&h)?;
    let Json(input) = j.map_err(|_| Error::Invalid)?;
    Ok(Json(s.issue_codes(secret, &id, input).await?))
}
async fn revoke(
    State(s): State<PromotionStore>,
    Path(id): Path<String>,
    h: HeaderMap,
) -> Result<Json<CodeStatus>> {
    Ok(Json(s.revoke_code(bearer(&h)?, &id).await?))
}
async fn redeem(
    State(s): State<PromotionStore>,
    h: HeaderMap,
    j: std::result::Result<Json<RedeemRequest>, JsonRejection>,
) -> Result<Json<Receipt>> {
    let secret = bearer(&h)?;
    let Json(input) = j.map_err(|_| Error::Invalid)?;
    Ok(Json(s.redeem(secret, input).await?))
}
async fn checkin(
    State(s): State<PromotionStore>,
    Path(id): Path<String>,
    h: HeaderMap,
    j: std::result::Result<Json<ClaimRequest>, JsonRejection>,
) -> Result<Json<Receipt>> {
    let secret = bearer(&h)?;
    let Json(input) = j.map_err(|_| Error::Invalid)?;
    Ok(Json(s.checkin(secret, &id, input).await?))
}
async fn envelope(
    State(s): State<PromotionStore>,
    Path(id): Path<String>,
    h: HeaderMap,
    j: std::result::Result<Json<ClaimRequest>, JsonRejection>,
) -> Result<Json<Receipt>> {
    let secret = bearer(&h)?;
    let Json(input) = j.map_err(|_| Error::Invalid)?;
    Ok(Json(s.envelope(secret, &id, input).await?))
}
async fn history(State(s): State<PromotionStore>, h: HeaderMap) -> Result<Json<Value>> {
    Ok(Json(
        json!({"contract":CONTRACT,"claims":s.history(bearer(&h)?).await?}),
    ))
}

//! Native Web API for raw Core-credit quotes and frozen intents. Checkout
//! dispatch remains a separate provider integration; no URL is invented here.
use super::*;
use axum::{
    Json, Router,
    extract::{DefaultBodyLimit, Path, Query, State},
    http::{HeaderMap, StatusCode, header},
    response::{IntoResponse, Response},
    routing::{delete, get, post, put},
};

struct Failure(Error);
impl From<Error> for Failure {
    fn from(e: Error) -> Self {
        Self(e)
    }
}
impl IntoResponse for Failure {
    fn into_response(self) -> Response {
        let (status, code) = match self.0 {
            Error::Invalid => (StatusCode::BAD_REQUEST, "PAYMENT_DISCOUNT_INVALID"),
            Error::Denied => (StatusCode::FORBIDDEN, "PAYMENT_DISCOUNT_DENIED"),
            Error::NotFound => (StatusCode::NOT_FOUND, "PAYMENT_DISCOUNT_NOT_FOUND"),
            Error::Conflict => (StatusCode::CONFLICT, "PAYMENT_DISCOUNT_CONFLICT"),
            Error::Storage | Error::Unavailable => (
                StatusCode::SERVICE_UNAVAILABLE,
                "PAYMENT_DISCOUNT_UNAVAILABLE",
            ),
        };
        (status, Json(json!({"success":false,"error":{"code":code}}))).into_response()
    }
}
type HttpResult = std::result::Result<Json<serde_json::Value>, Failure>;
fn bearer(h: &HeaderMap) -> std::result::Result<&str, Failure> {
    if h.get_all(header::AUTHORIZATION).iter().count() != 1 {
        return Err(Failure(Error::Denied));
    }
    h.get(header::AUTHORIZATION)
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.strip_prefix("Bearer "))
        .filter(|v| !v.is_empty())
        .ok_or(Failure(Error::Denied))
}
pub fn router(store: PaymentsStore) -> Router {
    Router::new()
        .route(
            "/core/v1/payment-discounts/admin/codes",
            get(list).post(create),
        )
        .route(
            "/core/v1/payment-discounts/admin/codes/{id}",
            put(update).delete(remove),
        )
        .route(
            "/core/v1/payment-discounts/admin/codes/{id}/status",
            put(status),
        )
        .route("/core/v1/payment-discounts/admin/codes/batch", post(batch))
        .route(
            "/core/v1/payment-discounts/admin/codes/exhausted",
            delete(exhausted),
        )
        .route("/core/v1/payment-discounts/quote", post(quote))
        .route("/core/v1/payment-discounts/prepare", post(prepare))
        .layer(DefaultBodyLimit::max(4096))
        .layer(axum::middleware::from_fn(
            |r: axum::extract::Request, next: axum::middleware::Next| async move {
                let mut response = next.run(r).await;
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
async fn list(
    State(s): State<PaymentsStore>,
    h: HeaderMap,
    Query(p): Query<CodesQuery>,
) -> HttpResult {
    Ok(Json(
        json!({"success":true,"data":s.list_discount_codes(bearer(&h)?, &p).await?}),
    ))
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct StatusInput {
    enabled: bool,
}
async fn status(
    State(s): State<PaymentsStore>,
    Path(id): Path<i64>,
    h: HeaderMap,
    Json(input): Json<StatusInput>,
) -> HttpResult {
    Ok(Json(
        json!({"success":true,"data":s.set_discount_status(bearer(&h)?,id,input.enabled).await?}),
    ))
}
async fn batch(
    State(s): State<PaymentsStore>,
    h: HeaderMap,
    Json(input): Json<Batch>,
) -> HttpResult {
    Ok(Json(
        json!({"success":true,"data":{"items":s.generate_discount_codes(bearer(&h)?, input).await?}}),
    ))
}
async fn remove(State(s): State<PaymentsStore>, Path(id): Path<i64>, h: HeaderMap) -> HttpResult {
    s.delete_discount_code(bearer(&h)?, id).await?;
    Ok(Json(json!({"success":true,"data":null})))
}
async fn exhausted(State(s): State<PaymentsStore>, h: HeaderMap) -> HttpResult {
    Ok(Json(
        json!({"success":true,"data":{"count":s.delete_exhausted_discount_codes(bearer(&h)?).await?.to_string()}}),
    ))
}
async fn create(
    State(s): State<PaymentsStore>,
    h: HeaderMap,
    Json(input): Json<Definition>,
) -> HttpResult {
    let credential = bearer(&h)?;
    let code = s.save_discount(credential, None, input).await?;
    Ok(Json(
        json!({"success":true,"data":s.discount_code_view(credential,code.id).await?}),
    ))
}
async fn update(
    State(s): State<PaymentsStore>,
    Path(id): Path<i64>,
    h: HeaderMap,
    Json(input): Json<Definition>,
) -> HttpResult {
    let credential = bearer(&h)?;
    let code = s.save_discount(credential, Some(id), input).await?;
    Ok(Json(
        json!({"success":true,"data":s.discount_code_view(credential,code.id).await?}),
    ))
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct QuoteInput {
    payment_channel_id: i64,
    credit_units: i64,
    discount_code: String,
}
async fn quote(
    State(s): State<PaymentsStore>,
    h: HeaderMap,
    Json(input): Json<QuoteInput>,
) -> HttpResult {
    Ok(Json(
        json!({"success":true,"data":s.quote_discount(bearer(&h)?, input.payment_channel_id, input.credit_units, &input.discount_code).await?}),
    ))
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct PrepareInput {
    idempotency_key: String,
    payment_channel_id: i64,
    credit_units: i64,
    currency: String,
    expected_amount_minor: i64,
    discount_code: String,
}
async fn prepare(
    State(s): State<PaymentsStore>,
    h: HeaderMap,
    Json(input): Json<PrepareInput>,
) -> HttpResult {
    if input.expected_amount_minor <= 0 {
        return Err(Failure(Error::Invalid));
    }
    let intent = s
        .prepare_personal_discount(
            bearer(&h)?,
            pb::PreparePersonalTopupRequest {
                discount_code: String::new(),
                mutation: Some(pb::MutationContext {
                    idempotency_key: input.idempotency_key,
                    expected_version: None,
                }),
                payment_channel_id: input.payment_channel_id,
                currency: input.currency,
                amount_minor: input.expected_amount_minor,
                requested_credit_units: input.credit_units,
            },
            &input.discount_code,
        )
        .await?;
    Ok(Json(json!({"success":true,"data":{
        "intent_id":intent.intent_id,"account_id":intent.account_id,"currency":intent.currency,
        "amount_minor":intent.amount_minor,"credit_units":intent.credit_units,"credit_unit":intent.credit_unit,
        "receipt_id":intent.receipt_id,"provider":intent.provider,
        "checkout_available":false
    }})))
}

//! Canonical routes mounted only by the explicit candidate constructor.
use super::*;
use crate::email_http::{self as email, EmailHttp};
use axum::{Extension, extract::rejection::JsonRejection};

pub(super) fn router(store: IdentityStore, email: EmailHttp) -> Router {
    Router::new()
        .route(
            "/core/v1/identity/email/verification/request",
            post(verification),
        )
        .route(
            "/core/v1/identity/email/password-reset/request",
            post(reset_request),
        )
        .route("/core/v1/identity/email/register", post(register))
        .route("/core/v1/identity/email/bind", post(bind))
        .route(
            "/core/v1/identity/email/password-reset/consume",
            post(reset),
        )
        .layer(Extension(email))
        .route_layer(middleware::from_fn_with_state(
            store.clone(),
            auth::rate_limit,
        ))
        .layer(DefaultBodyLimit::max(16 * 1024))
        .layer(middleware::from_fn(no_store))
        .with_state(store)
}

async fn verification(
    Extension(mail): Extension<EmailHttp>,
    headers: HeaderMap,
    input: std::result::Result<Json<email::VerificationRequest>, JsonRejection>,
) -> Response {
    let Ok(Json(body)) = input else {
        return email::failure(email::Failure::Invalid);
    };
    email::accepted(mail.request_verification(&headers, body).await)
}

async fn reset_request(
    Extension(mail): Extension<EmailHttp>,
    input: std::result::Result<Json<email::ResetRequest>, JsonRejection>,
) -> Response {
    let Ok(Json(body)) = input else {
        return email::failure(email::Failure::Invalid);
    };
    email::accepted(mail.request_reset(body).await)
}

async fn register(
    Extension(mail): Extension<EmailHttp>,
    input: std::result::Result<Json<email::Registration>, JsonRejection>,
) -> Response {
    let Ok(Json(body)) = input else {
        return email::failure(email::Failure::Invalid);
    };
    email::response(
        mail.register(&body, true).await,
        StatusCode::CREATED,
        json!({"registered":true,"login_required":true}),
    )
}

async fn bind(
    Extension(mail): Extension<EmailHttp>,
    headers: HeaderMap,
    input: std::result::Result<Json<email::Binding>, JsonRejection>,
) -> Response {
    let Ok(Json(body)) = input else {
        return email::failure(email::Failure::Invalid);
    };
    email::response(
        mail.bind(&headers, body).await,
        StatusCode::OK,
        json!({"email_bound":true}),
    )
}

async fn reset(
    Extension(mail): Extension<EmailHttp>,
    input: std::result::Result<Json<email::Reset>, JsonRejection>,
) -> Response {
    let Ok(Json(body)) = input else {
        return email::failure(email::Failure::Invalid);
    };
    email::response(
        mail.reset(body).await,
        StatusCode::OK,
        json!({"password_changed":true,"login_required":true}),
    )
}

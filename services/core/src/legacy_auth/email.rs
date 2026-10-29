//! Existing Web paths retain the legacy origin and source-rate middleware.
use super::*;
use crate::email_http as email;

pub(super) fn routes() -> Router<AuthState> {
    Router::new()
        .route(
            "/api/identity/email/verification/request",
            post(verification),
        )
        .route(
            "/api/identity/email/password-reset/request",
            post(reset_request),
        )
        .route("/api/oauth/email/bind", post(bind))
        .route("/api/user/reset", post(reset))
}

pub(super) async fn register(state: &AuthState, body: Register) -> Response {
    let Some(mail) = state.email.as_ref() else {
        return email::failure(email::Failure::Unavailable);
    };
    email::response(
        mail.register(
            &email::Registration {
                username: body.username,
                password: body.password,
                email: body.email,
                verification_code: body.verification_code,
                aff_code: body.aff_code,
                accepted_legal: body.accepted_legal,
                turnstile: body.turnstile,
            },
            state.config.require_legal_acceptance,
        )
        .await,
        StatusCode::OK,
        json!({"registered":true,"login_required":true}),
    )
}

async fn verification(
    State(state): State<AuthState>,
    headers: HeaderMap,
    input: Result<Json<email::VerificationRequest>, JsonRejection>,
) -> Response {
    let (Some(mail), Ok(Json(body))) = (state.email, input) else {
        return email::failure(email::Failure::Invalid);
    };
    email::accepted(mail.request_verification(&headers, body).await)
}

async fn reset_request(
    State(state): State<AuthState>,
    input: Result<Json<email::ResetRequest>, JsonRejection>,
) -> Response {
    let (Some(mail), Ok(Json(body))) = (state.email, input) else {
        return email::failure(email::Failure::Invalid);
    };
    email::accepted(mail.request_reset(body).await)
}

async fn bind(
    State(state): State<AuthState>,
    headers: HeaderMap,
    input: Result<Json<email::Binding>, JsonRejection>,
) -> Response {
    let (Some(mail), Ok(Json(body))) = (state.email, input) else {
        return email::failure(email::Failure::Invalid);
    };
    email::response(
        mail.bind(&headers, body).await,
        StatusCode::OK,
        json!({"email_bound":true}),
    )
}

async fn reset(
    State(state): State<AuthState>,
    input: Result<Json<email::Reset>, JsonRejection>,
) -> Response {
    let (Some(mail), Ok(Json(body))) = (state.email, input) else {
        return email::failure(email::Failure::Invalid);
    };
    email::response(
        mail.reset(body).await,
        StatusCode::OK,
        json!({"password_changed":true,"login_required":true}),
    )
}

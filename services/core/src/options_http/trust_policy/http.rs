//! Dedicated financial preview and explicit passkey-confirmation routes.
use super::{Confirmation, Service, store};
use crate::options_http::{auth_error, bearer, response};
use axum::{
    Json, Router,
    extract::{DefaultBodyLimit, State},
    http::{HeaderMap, StatusCode},
    response::Response,
    routing::{get, post},
};
use serde::Deserialize;
use serde_json::json;

pub fn router(state: Service) -> Router {
    Router::new()
        .route("/core/v1/admin/trust-policy", get(read))
        .route("/api/option/trust-policy/preview", post(preview))
        .route("/api/option/trust-policy/challenge", post(challenge))
        .route("/api/option/trust-policy/confirm", post(confirm))
        .route("/api/option/trust-policy/resolve", post(resolve))
        .layer(DefaultBodyLimit::max(100_000))
        .with_state(state)
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct PreviewInput {
    value: String,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct ChallengeInput {
    token: String,
    digest: String,
    editor: String,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct ResolveInput {
    token: String,
}

fn error(err: store::Error) -> Response {
    let (status, code) = match err {
        store::Error::Auth(err) => return auth_error(err),
        store::Error::Invalid => (StatusCode::BAD_REQUEST, "TRUST_POLICY_INVALID"),
        store::Error::Conflict => (StatusCode::CONFLICT, "TRUST_POLICY_CONFLICT"),
        store::Error::Limited => (StatusCode::TOO_MANY_REQUESTS, "TRUST_POLICY_PREVIEW_LIMIT"),
        store::Error::Storage => (StatusCode::SERVICE_UNAVAILABLE, "TRUST_POLICY_UNAVAILABLE"),
        store::Error::CommitUnknown => (
            StatusCode::SERVICE_UNAVAILABLE,
            "TRUST_POLICY_COMMIT_UNKNOWN",
        ),
    };
    response(status, json!({"success":false,"code":code}))
}

async fn preview(
    State(state): State<Service>,
    headers: HeaderMap,
    Json(input): Json<PreviewInput>,
) -> Response {
    let secret = match bearer(&headers) {
        Ok(value) => value,
        Err(err) => return auth_error(err),
    };
    match state.preview(secret, &input.value).await {
        Ok(created) => response(StatusCode::OK, json!({"success":true,"data":created})),
        Err(err) => error(err),
    }
}
async fn confirm(
    State(state): State<Service>,
    headers: HeaderMap,
    Json(input): Json<Confirmation>,
) -> Response {
    let secret = match bearer(&headers) {
        Ok(value) => value,
        Err(err) => return auth_error(err),
    };
    match state.confirm(secret, &input).await {
        Ok(revision) => response(
            StatusCode::OK,
            json!({"success":true,"data":{"revision":revision}}),
        ),
        Err(err) => error(err),
    }
}
async fn resolve(
    State(state): State<Service>,
    headers: HeaderMap,
    Json(input): Json<ResolveInput>,
) -> Response {
    let secret = match bearer(&headers) {
        Ok(value) => value,
        Err(err) => return auth_error(err),
    };
    match state.resolve(secret, &input.token).await {
        Ok(state) => response(StatusCode::OK, json!({"success":true,"data":state})),
        Err(err) => error(err),
    }
}

async fn challenge(
    State(state): State<Service>,
    headers: HeaderMap,
    Json(input): Json<ChallengeInput>,
) -> Response {
    let secret = match bearer(&headers) {
        Ok(v) => v,
        Err(e) => return auth_error(e),
    };
    match state
        .challenge(secret, &input.token, &input.digest, &input.editor)
        .await
    {
        Ok(value) => response(StatusCode::OK, json!({"success":true,"data":value})),
        Err(err) => error(err),
    }
}

async fn read(State(state): State<Service>, headers: HeaderMap) -> Response {
    let session = match bearer(&headers) {
        Ok(v) => v,
        Err(e) => return auth_error(e),
    };
    match state.read(session).await {
        Ok(value) => response(StatusCode::OK, json!({"success":true,"data":value})),
        Err(err) => error(err),
    }
}

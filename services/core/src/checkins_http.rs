//! Browser check-in transport. Identity, day selection and money stay in Core.
use crate::checkins::{CheckinStore, ClaimRequest, Error};
use axum::{
    Json, Router,
    extract::{
        DefaultBodyLimit, Query, State,
        rejection::{JsonRejection, QueryRejection},
    },
    http::{HeaderMap, StatusCode, header},
    response::{IntoResponse, Response},
    routing::get,
};
use serde::Deserialize;
use serde_json::json;
use sqlx::PgPool;

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct StatusQuery {
    month: Option<String>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct ClaimQuery {
    turnstile: Option<String>,
}

fn bearer(headers: &HeaderMap) -> Result<&str, Error> {
    if headers.get_all(header::AUTHORIZATION).iter().count() != 1 {
        return Err(Error::Unauthorized);
    }
    headers
        .get(header::AUTHORIZATION)
        .and_then(|value| value.to_str().ok())
        .and_then(|value| value.split_once(' '))
        .filter(|(scheme, secret)| {
            scheme.eq_ignore_ascii_case("Bearer")
                && !secret.is_empty()
                && !secret.bytes().any(|byte| byte.is_ascii_whitespace())
        })
        .map(|(_, secret)| secret)
        .ok_or(Error::Unauthorized)
}

impl IntoResponse for Error {
    fn into_response(self) -> Response {
        let (status, code, message) = match self {
            Self::Invalid => (
                StatusCode::BAD_REQUEST,
                "CHECKIN_INVALID",
                "Invalid check-in request",
            ),
            Self::Unauthorized => (
                StatusCode::UNAUTHORIZED,
                "CHECKIN_UNAUTHORIZED",
                "Login required",
            ),
            Self::Forbidden => (
                StatusCode::FORBIDDEN,
                "CHECKIN_FORBIDDEN",
                "Check-in is unavailable for this account",
            ),
            Self::NotFound => (
                StatusCode::NOT_FOUND,
                "CHECKIN_NOT_FOUND",
                "Check-in record not found",
            ),
            Self::Conflict => (
                StatusCode::CONFLICT,
                "CHECKIN_CONFLICT",
                "Check-in request conflicts with its original operation",
            ),
            Self::Retry => (
                StatusCode::CONFLICT,
                "CHECKIN_RETRY",
                "Read check-in status before retrying the same operation",
            ),
            Self::Unavailable => (
                StatusCode::SERVICE_UNAVAILABLE,
                "CHECKIN_UNAVAILABLE",
                "Check-in is temporarily unavailable",
            ),
        };
        (
            status,
            Json(json!({"success":false,"message":message,"error":{"code":code}})),
        )
            .into_response()
    }
}

pub fn router(pool: PgPool) -> Router {
    Router::new()
        .route("/api/user/checkin", get(status).post(claim))
        .layer(DefaultBodyLimit::max(16_384))
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
        .with_state(CheckinStore::new(pool))
}

async fn status(
    State(store): State<CheckinStore>,
    headers: HeaderMap,
    query: Result<Query<StatusQuery>, QueryRejection>,
) -> Result<Json<serde_json::Value>, Error> {
    let secret = bearer(&headers)?;
    let Query(query) = query.map_err(|_| Error::Invalid)?;
    let status = store.status(secret, query.month.as_deref()).await?;
    Ok(Json(json!({"success":true,"data":status})))
}

async fn claim(
    State(store): State<CheckinStore>,
    headers: HeaderMap,
    query: Result<Query<ClaimQuery>, QueryRejection>,
    body: Result<Json<ClaimRequest>, JsonRejection>,
) -> Result<Json<serde_json::Value>, Error> {
    let secret = bearer(&headers)?;
    let Query(query) = query.map_err(|_| Error::Invalid)?;
    // Match the existing Core email challenge boundary until a configured
    // verifier can authenticate this proof. A supplied token is never ignored.
    if query.turnstile.is_some_and(|proof| !proof.is_empty()) {
        return Err(Error::Unavailable);
    }
    let Json(request) = body.map_err(|_| Error::Invalid)?;
    let result = store.claim(secret, request).await?;
    Ok(Json(json!({"success":true,"data":result})))
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::{body::Body, http::Request};
    use tower::ServiceExt;

    #[test]
    fn credentials_require_exactly_one_bearer_value() {
        let mut headers = HeaderMap::new();
        assert_eq!(bearer(&headers), Err(Error::Unauthorized));
        for value in [
            "Basic secret",
            "Bearer ",
            "Bearer with space",
            "Bearer\tsecret",
        ] {
            headers.insert(header::AUTHORIZATION, value.parse().unwrap());
            assert_eq!(bearer(&headers), Err(Error::Unauthorized));
        }
        headers.insert(
            header::AUTHORIZATION,
            "bearer session-secret".parse().unwrap(),
        );
        assert_eq!(bearer(&headers), Ok("session-secret"));
        headers.append(header::AUTHORIZATION, "Bearer second".parse().unwrap());
        assert_eq!(bearer(&headers), Err(Error::Unauthorized));
    }

    #[tokio::test]
    async fn malformed_requests_and_unverified_proofs_stop_before_storage() {
        let pool = sqlx::postgres::PgPoolOptions::new()
            .acquire_timeout(std::time::Duration::from_millis(10))
            .connect_lazy("postgresql://127.0.0.1:1/checkin_transport_test")
            .unwrap();
        let app = router(pool);
        let cases = [
            (
                "GET",
                "/api/user/checkin",
                None,
                "",
                StatusCode::UNAUTHORIZED,
                "CHECKIN_UNAUTHORIZED",
            ),
            (
                "GET",
                "/api/user/checkin?account_id=99",
                Some("Bearer session"),
                "",
                StatusCode::BAD_REQUEST,
                "CHECKIN_INVALID",
            ),
            (
                "POST",
                "/api/user/checkin",
                Some("Bearer session"),
                "{}",
                StatusCode::BAD_REQUEST,
                "CHECKIN_INVALID",
            ),
            (
                "POST",
                "/api/user/checkin",
                Some("Bearer session"),
                r#"{"operation_key":"same-operation","amount":500000}"#,
                StatusCode::BAD_REQUEST,
                "CHECKIN_INVALID",
            ),
            (
                "POST",
                "/api/user/checkin",
                Some("Bearer session"),
                r#"{"operation_key":""}"#,
                StatusCode::BAD_REQUEST,
                "CHECKIN_INVALID",
            ),
            (
                "POST",
                "/api/user/checkin?turnstile=unverified",
                Some("Bearer session"),
                r#"{"operation_key":"same-operation"}"#,
                StatusCode::SERVICE_UNAVAILABLE,
                "CHECKIN_UNAVAILABLE",
            ),
        ];
        for (method, uri, authorization, body, expected, code) in cases {
            let mut request = Request::builder()
                .method(method)
                .uri(uri)
                .header(header::CONTENT_TYPE, "application/json");
            if let Some(authorization) = authorization {
                request = request.header(header::AUTHORIZATION, authorization);
            }
            let response = app
                .clone()
                .oneshot(request.body(Body::from(body)).unwrap())
                .await
                .unwrap();
            assert_eq!(response.status(), expected, "{method} {uri}");
            assert_eq!(
                response.headers()[header::CACHE_CONTROL],
                "private, no-store"
            );
            assert_eq!(response.headers()[header::VARY], "Authorization");
            let bytes = axum::body::to_bytes(response.into_body(), 4096)
                .await
                .unwrap();
            let body: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
            assert_eq!(body["success"], false);
            assert_eq!(body["error"]["code"], code);
        }
    }
}

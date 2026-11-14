//! Dedicated read-only HTTP path: neither success nor error sets any cookie.
use super::*;

pub(super) const PATH: &str = "/api/user/auth/reconcile";
static READERS: tokio::sync::Semaphore = tokio::sync::Semaphore::const_new(16);

pub(super) async fn reconcile(
    State(state): State<AuthState>,
    headers: HeaderMap,
    body: Bytes,
) -> Response {
    if !body.is_empty() {
        return failure(
            StatusCode::BAD_REQUEST,
            "AUTH_RECONCILE_INVALID",
            "Session reconciliation requires an empty body",
        );
    }
    let Ok(_permit) = READERS.try_acquire() else {
        return failure(
            StatusCode::TOO_MANY_REQUESTS,
            "AUTH_RECONCILE_BUSY",
            "Please retry later",
        );
    };
    let input =
        refresh_cookie(&headers).and_then(|cookie| expected_sid(&headers).map(|sid| (cookie, sid)));
    let (cookie, expected) = match input {
        Ok(input) => input,
        Err(_) => {
            return failure(
                StatusCode::BAD_REQUEST,
                "AUTH_RECONCILE_INVALID",
                "Invalid session reconciliation request",
            );
        }
    };
    match state.browser.reconcile_session(cookie, expected).await {
        Ok(value) => success(Some(value)),
        Err(BrowserAuthError::Identity(IdentityError::Invalid)) => failure(
            StatusCode::BAD_REQUEST,
            "AUTH_RECONCILE_INVALID",
            "Invalid session reconciliation request",
        ),
        Err(_) => failure(
            StatusCode::SERVICE_UNAVAILABLE,
            "AUTH_RECONCILE_UNAVAILABLE",
            "Session reconciliation is temporarily unavailable",
        ),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::body::{Body, to_bytes};
    use tower::ServiceExt;

    fn app() -> Router {
        let pool = sqlx::postgres::PgPoolOptions::new()
            .acquire_timeout(Duration::from_millis(20))
            .connect_lazy("postgresql://unused@127.0.0.1:1/unreachable")
            .unwrap();
        router(
            IdentityStore::from_pool(pool),
            LegacyAuthConfig::new("https://app.example", &[42; 32]).unwrap(),
        )
    }

    #[tokio::test]
    async fn no_cookie_is_readonly_and_every_response_keeps_cookie_jar() {
        let response = app()
            .oneshot(
                axum::http::Request::post(PATH)
                    .header(header::ORIGIN, "https://app.example")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::OK);
        assert!(!response.headers().contains_key(header::SET_COOKIE));
        let value: Value =
            serde_json::from_slice(&to_bytes(response.into_body(), 4096).await.unwrap()).unwrap();
        assert_eq!(value["data"]["status"], "no_cookie");
        assert_eq!(
            value["data"]["protocol"],
            "core.browser-session-reconciliation.v1"
        );
        for (origin, cookie, expected_status) in [
            ("https://wrong.example", None, StatusCode::FORBIDDEN),
            (
                "https://app.example",
                Some("new_api_refresh=one; new_api_refresh=two"),
                StatusCode::BAD_REQUEST,
            ),
        ] {
            let mut request = axum::http::Request::post(PATH).header(header::ORIGIN, origin);
            if let Some(cookie) = cookie {
                request = request.header(header::COOKIE, cookie);
            }
            let response = app()
                .oneshot(request.body(Body::empty()).unwrap())
                .await
                .unwrap();
            assert_eq!(response.status(), expected_status);
            assert!(!response.headers().contains_key(header::SET_COOKIE));
        }
    }
}

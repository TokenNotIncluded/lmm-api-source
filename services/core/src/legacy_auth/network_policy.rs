//! Browser-owned network settings use the existing session and Origin checks.
use super::*;
use crate::identity::network_policy::{AllowlistError, AllowlistUpdate, AllowlistView};
use std::net::IpAddr;

pub(super) fn routes() -> Router<AuthState> {
    Router::new().route("/api/user/self/ip-allowlist", get(read).put(update))
}
fn peer_ip(
    peer: Result<ConnectInfo<SocketAddr>, axum::extract::rejection::ExtensionRejection>,
) -> Option<IpAddr> {
    peer.ok().map(|peer| peer.0.ip())
}
fn result(value: Result<AllowlistView, AllowlistError>) -> Response {
    match value {
        Ok(data) => (StatusCode::OK, Json(json!({"success":true,"data":data}))).into_response(),
        Err(AllowlistError::Invalid) => failure(
            StatusCode::BAD_REQUEST,
            "USER_IP_ALLOWLIST_INVALID",
            "Enter up to 128 valid IPv4 or IPv6 addresses or CIDR networks.",
        ),
        Err(AllowlistError::Lockout) => failure(
            StatusCode::BAD_REQUEST,
            "USER_IP_ALLOWLIST_LOCKOUT",
            "Include your current IP address before enabling the allowlist.",
        ),
        Err(AllowlistError::Identity(IdentityError::Forbidden)) => failure(
            StatusCode::FORBIDDEN,
            "USER_IP_REJECTED",
            "Your IP address is not allowed for this account.",
        ),
        Err(AllowlistError::Identity(error)) => auth_failure(error.into()),
    }
}
async fn read(
    State(state): State<AuthState>,
    headers: HeaderMap,
    peer: Result<ConnectInfo<SocketAddr>, axum::extract::rejection::ExtensionRejection>,
) -> Response {
    let value = async {
        let secret = bearer(&headers)?.ok_or(IdentityError::Unauthorized)?;
        state.store.user_ip_allowlist(secret, peer_ip(peer)).await
    }
    .await;
    result(value)
}
async fn update(
    State(state): State<AuthState>,
    headers: HeaderMap,
    peer: Result<ConnectInfo<SocketAddr>, axum::extract::rejection::ExtensionRejection>,
    input: Result<Json<AllowlistUpdate>, JsonRejection>,
) -> Response {
    let value = async {
        let secret = bearer(&headers)?.ok_or(IdentityError::Unauthorized)?;
        let Json(input) = input.map_err(|_| AllowlistError::Invalid)?;
        state
            .store
            .update_user_ip_allowlist(secret, &input, peer_ip(peer))
            .await
    }
    .await;
    result(value)
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::body::{Body, to_bytes};
    use axum::http::Request;
    use sqlx::PgPool;
    use tower::ServiceExt;

    fn request(secret: &str, method: &str, ip: &str, body: Value) -> Request<Body> {
        let mut request = Request::builder()
            .uri("/api/user/self/ip-allowlist")
            .method(method)
            .header(header::ORIGIN, "https://lmm.test")
            .header(header::AUTHORIZATION, format!("Bearer {secret}"))
            .header(header::CONTENT_TYPE, "application/json")
            .body(if method == "GET" {
                Body::empty()
            } else {
                Body::from(body.to_string())
            })
            .unwrap();
        request
            .extensions_mut()
            .insert(ConnectInfo(SocketAddr::new(ip.parse().unwrap(), 12345)));
        request
    }
    async fn body(response: Response) -> Value {
        serde_json::from_slice(&to_bytes(response.into_body(), 16384).await.unwrap()).unwrap()
    }
    #[sqlx::test(migrations = false)]
    async fn user_ip_allowlist_http_saves_reports_ip_and_rejects_lockout(pool: PgPool) {
        let store = IdentityStore::from_pool(pool.clone()).with_registration_enabled(true);
        store.init_database().await.unwrap();
        let session = store
            .register(&crate::identity::LoginRequest {
                login_name: "network-http-user".into(),
                password: "test password 123".into(),
            })
            .await
            .unwrap();
        let app = crate::legacy_auth::router(
            store,
            LegacyAuthConfig::new("https://lmm.test", &[41; 32]).unwrap(),
        );
        let response = app
            .clone()
            .oneshot(request(&session.secret, "GET", "192.0.2.7", Value::Null))
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::OK);
        let data = body(response).await;
        assert_eq!(data["data"]["client_ip"], "192.0.2.7");
        assert_eq!(data["data"]["allow_ips"], json!([]));
        let response = app
            .clone()
            .oneshot(request(
                &session.secret,
                "PUT",
                "192.0.2.7",
                json!({"allow_ips":["192.0.2.0/24"]}),
            ))
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::OK);
        assert_eq!(
            body(response).await["data"]["allow_ips"],
            json!(["192.0.2.0/24"])
        );
        let response = app
            .clone()
            .oneshot(request(
                &session.secret,
                "PUT",
                "192.0.2.7",
                json!({"allow_ips":["198.51.100.0/24"]}),
            ))
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::BAD_REQUEST);
        assert_eq!(body(response).await["code"], "USER_IP_ALLOWLIST_LOCKOUT");
        let response = app
            .clone()
            .oneshot(request(
                &session.secret,
                "PUT",
                "192.0.2.7",
                json!({"allow_ips":["invalid"]}),
            ))
            .await
            .unwrap();
        assert_eq!(body(response).await["code"], "USER_IP_ALLOWLIST_INVALID");
        let response = app
            .clone()
            .oneshot(request(&session.secret, "GET", "198.51.100.8", Value::Null))
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::FORBIDDEN);
        assert_eq!(body(response).await["code"], "USER_IP_REJECTED");
        let mut invalid_origin =
            request(&session.secret, "PUT", "192.0.2.7", json!({"allow_ips":[]}));
        invalid_origin.headers_mut().insert(
            header::ORIGIN,
            HeaderValue::from_static("https://other.test"),
        );
        assert_eq!(
            app.oneshot(invalid_origin).await.unwrap().status(),
            StatusCode::FORBIDDEN
        );
    }
}

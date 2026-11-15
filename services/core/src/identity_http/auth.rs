use super::*;
use crate::identity::{LoginOutcome, LoginRequest, MfaCompletion, MfaError};
use axum::{
    extract::{
        ConnectInfo,
        rejection::{JsonRejection, QueryRejection},
    },
    response::Redirect,
};
use rand::TryRng as _;
use std::net::SocketAddr;

const FLOW_COOKIE: &str = "__Host-lmm_google_flow";

pub(super) fn routes(store: IdentityStore) -> Router<IdentityStore> {
    Router::new()
        .route("/core/v1/auth/register", post(register))
        .route("/core/v1/auth/login", post(login))
        .route("/core/v1/auth/mfa/complete", post(complete_mfa_login))
        .route("/core/v1/auth/password", post(change_password))
        .route("/core/v1/auth/session/refresh", post(refresh))
        .route("/core/v1/auth/logout", post(logout))
        .route("/core/v1/auth/logout-all", post(logout_all))
        .route("/core/v1/auth/oauth/google", get(google_start))
        .route("/core/v1/auth/oauth/google/callback", get(google_callback))
        .route_layer(middleware::from_fn_with_state(store, rate_limit))
}
pub(super) async fn rate_limit(
    State(store): State<IdentityStore>,
    request: Request,
    next: Next,
) -> Response {
    let peer = request
        .extensions()
        .get::<ConnectInfo<SocketAddr>>()
        .map(|p| p.0.ip().to_string())
        .unwrap_or_else(|| "unknown-peer".to_owned());
    match store.auth_rate_limit("peer", &peer, 60).await {
        Ok(true) => next.run(request).await,
        Ok(false) => (
            StatusCode::TOO_MANY_REQUESTS,
            [(header::RETRY_AFTER, "60")],
            Json(json!({"error":{"code":"identity_rate_limited"}})),
        )
            .into_response(),
        Err(e) => e.into_response(),
    }
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct PasswordInput {
    login_name: String,
    password: String,
    #[serde(default)]
    turnstile: String,
}
#[derive(Default, Deserialize)]
#[serde(deny_unknown_fields)]
struct ChallengeQuery {
    #[serde(default)]
    turnstile: String,
}
async fn password_input(
    store: &IdentityStore,
    query: std::result::Result<Query<ChallengeQuery>, QueryRejection>,
    input: std::result::Result<Json<PasswordInput>, JsonRejection>,
) -> std::result::Result<LoginRequest, Box<Response>> {
    let (Ok(Query(query)), Ok(Json(body))) = (query, input) else {
        return Err(Box::new(IdentityError::Invalid.into_response()));
    };
    if !query.turnstile.is_empty() && !body.turnstile.is_empty() {
        return Err(Box::new(IdentityError::Invalid.into_response()));
    }
    let token = if query.turnstile.is_empty() {
        &body.turnstile
    } else {
        &query.turnstile
    };
    store
        .verify_turnstile(token)
        .await
        .map_err(|error| Box::new(error.response()))?;
    Ok(LoginRequest {
        login_name: body.login_name,
        password: body.password,
    })
}
async fn register(
    State(store): State<IdentityStore>,
    query: std::result::Result<Query<ChallengeQuery>, QueryRejection>,
    input: std::result::Result<Json<PasswordInput>, JsonRejection>,
) -> Response {
    let body = match password_input(&store, query, input).await {
        Ok(body) => body,
        Err(response) => return *response,
    };
    match store.register(&body).await {
        Ok(issued) => (StatusCode::CREATED, Json(issued)).into_response(),
        Err(error) => error.into_response(),
    }
}
async fn login(
    State(store): State<IdentityStore>,
    query: std::result::Result<Query<ChallengeQuery>, QueryRejection>,
    input: std::result::Result<Json<PasswordInput>, JsonRejection>,
) -> Response {
    let body = match password_input(&store, query, input).await {
        Ok(body) => body,
        Err(response) => return *response,
    };
    match store.login_with_mfa(&body).await {
        Ok(LoginOutcome::Session(issued)) => Json(issued).into_response(),
        Ok(LoginOutcome::Challenge(challenge)) => Json(challenge).into_response(),
        Err(error) => mfa_failure(error),
    }
}
async fn complete_mfa_login(
    State(store): State<IdentityStore>,
    input: std::result::Result<Json<MfaCompletion>, JsonRejection>,
) -> Response {
    let Ok(Json(body)) = input else {
        return IdentityError::Invalid.into_response();
    };
    match store.complete_mfa_login(&body).await {
        Ok(issued) => Json(issued).into_response(),
        Err(error) => mfa_failure(error),
    }
}
fn mfa_failure(error: MfaError) -> Response {
    let (status, code, retry_after) = match error {
        MfaError::Identity(error) => return error.into_response(),
        MfaError::CodeInvalid => (StatusCode::BAD_REQUEST, "MFA_CODE_INVALID", None),
        MfaError::FlowExpired => (StatusCode::BAD_REQUEST, "MFA_FLOW_EXPIRED", None),
        MfaError::Locked(seconds) => (StatusCode::TOO_MANY_REQUESTS, "MFA_LOCKED", Some(seconds)),
        MfaError::Unavailable => (StatusCode::SERVICE_UNAVAILABLE, "MFA_UNAVAILABLE", None),
    };
    let mut response = (status, Json(json!({"error":{"code":code}}))).into_response();
    if let Some(seconds) = retry_after
        && let Ok(value) = HeaderValue::from_str(&seconds.max(1).to_string())
    {
        response.headers_mut().insert(header::RETRY_AFTER, value);
    }
    response
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct PasswordChange {
    original_password: String,
    password: String,
}
async fn change_password(
    State(store): State<IdentityStore>,
    headers: HeaderMap,
    Json(body): Json<PasswordChange>,
) -> Result<impl IntoResponse> {
    Ok(Json(
        store
            .change_password(bearer(&headers)?, &body.original_password, &body.password)
            .await?,
    ))
}
async fn refresh(
    State(store): State<IdentityStore>,
    headers: HeaderMap,
) -> Result<impl IntoResponse> {
    Ok(Json(store.refresh_session(bearer(&headers)?).await?))
}
async fn logout(State(store): State<IdentityStore>, headers: HeaderMap) -> Result<StatusCode> {
    store.logout(bearer(&headers)?).await?;
    Ok(StatusCode::NO_CONTENT)
}
async fn logout_all(State(store): State<IdentityStore>, headers: HeaderMap) -> Result<StatusCode> {
    store.logout_all(bearer(&headers)?).await?;
    Ok(StatusCode::NO_CONTENT)
}
pub(super) async fn list_credentials(
    State(store): State<IdentityStore>,
    headers: HeaderMap,
    Query(query): Query<TeamQuery>,
) -> Result<impl IntoResponse> {
    Ok(Json(
        store
            .list_credentials(bearer(&headers)?, query.after, None)
            .await?,
    ))
}
async fn google_start(State(store): State<IdentityStore>) -> Result<Response> {
    if !store.google_native_enabled() {
        return Err(IdentityError::Forbidden);
    }
    let mut random = [0u8; 32];
    rand::rngs::SysRng
        .try_fill_bytes(&mut random)
        .map_err(|_| IdentityError::Storage)?;
    let binding: String = crate::hex_codec::encode(random);
    let url = store.begin_google_login(&binding).await?;
    let mut response = Redirect::to(&url).into_response();
    let cookie =
        format!("{FLOW_COOKIE}={binding}; Path=/; Secure; HttpOnly; SameSite=Lax; Max-Age=600");
    response.headers_mut().insert(
        header::SET_COOKIE,
        HeaderValue::from_str(&cookie).map_err(|_| IdentityError::Storage)?,
    );
    Ok(response)
}
fn flow_cookie(headers: &HeaderMap) -> Result<&str> {
    let mut found = None;
    for header in headers.get_all(header::COOKIE) {
        let header = header.to_str().map_err(|_| IdentityError::Unauthorized)?;
        for part in header.split(';') {
            if let Some((name, value)) = part.trim().split_once('=')
                && name == FLOW_COOKIE
            {
                if found.is_some()
                    || value.len() != 64
                    || !value.bytes().all(|b| b.is_ascii_hexdigit())
                {
                    return Err(IdentityError::Unauthorized);
                }
                found = Some(value);
            }
        }
    }
    found.ok_or(IdentityError::Unauthorized)
}
#[derive(Deserialize)]
struct Callback {
    state: Option<String>,
    code: Option<String>,
    error: Option<String>,
}
async fn google_callback(
    State(store): State<IdentityStore>,
    headers: HeaderMap,
    Query(query): Query<Callback>,
) -> Response {
    let result = async {
        if !store.google_native_enabled() {
            return Err(IdentityError::Forbidden);
        }
        if query.error.is_some() {
            return Err(IdentityError::Unauthorized);
        }
        let binding = flow_cookie(&headers)?;
        let state = query.state.as_deref().ok_or(IdentityError::Unauthorized)?;
        let code = query.code.as_deref().ok_or(IdentityError::Unauthorized)?;
        store.finish_google_login(state, binding, code).await
    }
    .await;
    let mut response = match result {
        Ok(issued) => Json(issued).into_response(),
        Err(error) => error.into_response(),
    };
    response.headers_mut().insert(
        header::SET_COOKIE,
        HeaderValue::from_static(
            "__Host-lmm_google_flow=; Path=/; Secure; HttpOnly; SameSite=Lax; Max-Age=0",
        ),
    );
    response
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn binding_cookie_rejects_missing_duplicates_and_unbounded_values() {
        let mut headers = HeaderMap::new();
        assert!(flow_cookie(&headers).is_err());
        headers.insert(
            header::COOKIE,
            HeaderValue::from_str(&format!("{FLOW_COOKIE}={}", "a".repeat(64))).unwrap(),
        );
        assert_eq!(flow_cookie(&headers).unwrap(), "a".repeat(64));
        headers.append(
            header::COOKIE,
            HeaderValue::from_str(&format!("{FLOW_COOKIE}={}", "b".repeat(64))).unwrap(),
        );
        assert!(flow_cookie(&headers).is_err());
    }
}

#[cfg(test)]
mod turnstile_tests {
    use super::*;

    #[tokio::test]
    async fn native_auth_rejects_missing_challenge_before_identity_work() {
        let pool = sqlx::postgres::PgPoolOptions::new()
            .connect_lazy("postgres://unused:unused@127.0.0.1:1/unused")
            .unwrap();
        let store = IdentityStore::from_pool(pool)
            .with_registration_enabled(true)
            .with_turnstile(Some(crate::config::turnstile_config::tests::verifier()));
        let input = || {
            Ok(Json(PasswordInput {
                login_name: "test-user".into(),
                password: "12345678".into(),
                turnstile: String::new(),
            }))
        };
        assert_eq!(
            register(
                State(store.clone()),
                Ok(Query(ChallengeQuery::default())),
                input()
            )
            .await
            .status(),
            StatusCode::FORBIDDEN
        );
        assert_eq!(
            login(State(store), Ok(Query(ChallengeQuery::default())), input())
                .await
                .status(),
            StatusCode::FORBIDDEN
        );
    }
}

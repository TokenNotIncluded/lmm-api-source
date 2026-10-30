//! Same-origin browser Passkey endpoints backed by Core's one-use ceremonies.
use super::*;
use axum::extract::Path;

pub(super) fn routes() -> Router<AuthState> {
    Router::new()
        .route("/api/user/passkey", get(status))
        .route("/api/user/passkey/register/begin", post(register_begin))
        .route("/api/user/passkey/register/finish", post(register_finish))
        .route("/api/user/passkey/{id}", axum::routing::delete(remove))
        .route("/api/user/passkey/login/begin", post(login_begin))
        .route("/api/user/passkey/login/finish", post(login_finish))
        .layer(DefaultBodyLimit::max(65536))
}
fn cookie_name(config: &LegacyAuthConfig, purpose: &str) -> String {
    format!(
        "{}lmm_passkey_{purpose}",
        if config.secure { "__Host-" } else { "" }
    )
}
fn cookie(
    response: Response,
    config: &LegacyAuthConfig,
    purpose: &str,
    value: Option<&str>,
) -> Response {
    external_login::set_flow_cookie(
        response,
        &cookie_name(config, purpose),
        value,
        config,
        300,
        "Strict",
    )
}
fn binding<'a>(
    headers: &'a HeaderMap,
    config: &LegacyAuthConfig,
    purpose: &str,
) -> Result<&'a str, IdentityError> {
    external_login::cookie_value(headers, &cookie_name(config, purpose), "", 64)
}
fn required_bearer(headers: &HeaderMap) -> Result<&str, IdentityError> {
    bearer(headers)?.ok_or(IdentityError::Unauthorized)
}
async fn status(State(state): State<AuthState>, headers: HeaderMap) -> Response {
    if state.config.passkeys.is_none() {
        return auth_failure(IdentityError::Forbidden.into());
    }
    let result = async {
        state
            .browser
            .passkey_status(required_bearer(&headers)?)
            .await
    }
    .await;
    match result {
        Ok(status) => success(Some(status)),
        Err(error) => auth_failure(error),
    }
}
async fn register_begin(State(state): State<AuthState>, headers: HeaderMap) -> Response {
    let result = async {
        let config = state
            .config
            .passkeys
            .as_ref()
            .ok_or(IdentityError::Forbidden)?;
        let binding = external_login::random_binding()?;
        let challenge = state
            .browser
            .begin_passkey_registration(config, required_bearer(&headers)?, &binding)
            .await?;
        Ok::<_, BrowserAuthError>((binding, challenge))
    }
    .await;
    match result {
        Ok((binding, challenge)) => cookie(
            success(Some(json!(challenge))),
            &state.config,
            "register",
            Some(&binding),
        ),
        Err(error) => auth_failure(error),
    }
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Registration {
    flow_token: String,
    credential: Value,
    #[serde(default)]
    name: String,
}
async fn register_finish(
    State(state): State<AuthState>,
    headers: HeaderMap,
    input: Result<Json<Registration>, JsonRejection>,
) -> Response {
    let result = async {
        let config = state
            .config
            .passkeys
            .as_ref()
            .ok_or(IdentityError::Forbidden)?;
        let Json(input) = input.map_err(|_| IdentityError::Invalid)?;
        state
            .browser
            .finish_passkey_registration(
                config,
                required_bearer(&headers)?,
                binding(&headers, &state.config, "register")?,
                &input.flow_token,
                input.credential,
                &input.name,
            )
            .await
    }
    .await;
    let response = match result {
        Ok(bundle) => bundle_response(bundle, &state.config),
        Err(error) => auth_failure(error),
    };
    cookie(response, &state.config, "register", None)
}
async fn remove(
    State(state): State<AuthState>,
    headers: HeaderMap,
    Path(id): Path<i64>,
) -> Response {
    let result = async {
        state
            .config
            .passkeys
            .as_ref()
            .ok_or(IdentityError::Forbidden)?;
        state
            .browser
            .remove_passkey(required_bearer(&headers)?, id)
            .await
    }
    .await;
    match result {
        Ok(bundle) => bundle_response(bundle, &state.config),
        Err(error) => auth_failure(error),
    }
}
#[derive(Default, Deserialize)]
#[serde(deny_unknown_fields)]
struct LoginBegin {
    username: Option<String>,
}
async fn login_begin(State(state): State<AuthState>, body: Bytes) -> Response {
    let result = async {
        let config = state
            .config
            .passkeys
            .as_ref()
            .ok_or(IdentityError::Forbidden)?;
        let input: LoginBegin = if body.is_empty() {
            LoginBegin::default()
        } else {
            serde_json::from_slice(&body).map_err(|_| IdentityError::Invalid)?
        };
        let binding = external_login::random_binding()?;
        let challenge = state
            .browser
            .begin_passkey_login(config, &binding, input.username.as_deref())
            .await?;
        Ok::<_, BrowserAuthError>((binding, challenge))
    }
    .await;
    match result {
        Ok((binding, challenge)) => cookie(
            success(Some(json!(challenge))),
            &state.config,
            "login",
            Some(&binding),
        ),
        Err(error) => auth_failure(error),
    }
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct LoginFinish {
    flow_token: String,
    credential: Value,
}
async fn login_finish(
    State(state): State<AuthState>,
    headers: HeaderMap,
    peer: Result<ConnectInfo<SocketAddr>, axum::extract::rejection::ExtensionRejection>,
    input: Result<Json<LoginFinish>, JsonRejection>,
) -> Response {
    let result = async {
        let config = state
            .config
            .passkeys
            .as_ref()
            .ok_or(IdentityError::Forbidden)?;
        let Json(input) = input.map_err(|_| IdentityError::Invalid)?;
        let mfa_binding = external_login::random_binding()?;
        let metadata = BrowserSessionMetadata {
            ip: peer.map(|peer| peer.0.ip().to_string()).unwrap_or_default(),
            user_agent: headers
                .get(header::USER_AGENT)
                .and_then(|h| h.to_str().ok())
                .unwrap_or_default()
                .chars()
                .take(512)
                .collect(),
        };
        let outcome = state
            .browser
            .finish_passkey_login(
                config,
                binding(&headers, &state.config, "login")?,
                &input.flow_token,
                input.credential,
                &metadata,
                &mfa_binding,
            )
            .await?;
        Ok::<_, MfaError>((outcome, mfa_binding))
    }
    .await;
    let response = match result {
        Ok((BrowserLoginOutcome::Session(bundle), _)) => bundle_response(bundle, &state.config),
        Ok((BrowserLoginOutcome::Challenge(challenge), binding)) => mfa_flow_cookie(
            success(Some(json!(challenge))),
            Some(&binding),
            &state.config,
        ),
        Err(error) => mfa_failure(error),
    };
    cookie(response, &state.config, "login", None)
}

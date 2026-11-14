//! Native OIDC, WeChat code bridge and legacy custom OAuth entry points.
use super::*;
use crate::identity::{AccountBindFinish, FederatedBindOutcome, FederatedLoginOutcome};
use axum::extract::Path;

pub(super) fn routes() -> Router<AuthState> {
    Router::new()
        .route("/api/oauth/federated/providers", get(catalog))
        .route("/api/oauth/federated/{provider}/begin", post(begin_login))
        .route("/api/oauth/federated/{provider}/finish", post(finish_login))
}

fn reference(value: &str) -> Result<&str, IdentityError> {
    let value = value.strip_prefix("federated:").unwrap_or(value);
    if matches!(value, "oidc" | "wechat") {
        return Ok(value);
    }
    let id = value.parse::<i64>().map_err(|_| IdentityError::Invalid)?;
    if id <= 0 || id.to_string() != value {
        return Err(IdentityError::Invalid);
    }
    Ok(value)
}
fn cookie_name(
    config: &LegacyAuthConfig,
    provider: &str,
    bind: bool,
) -> Result<String, IdentityError> {
    Ok(format!(
        "{}lmm_federated_{}_{}",
        if config.secure { "__Host-" } else { "" },
        reference(provider)?,
        if bind { "bind" } else { "login" }
    ))
}
fn cookie(
    response: Response,
    config: &LegacyAuthConfig,
    name: &str,
    binding: Option<&str>,
) -> Response {
    external_login::set_flow_cookie(response, name, binding, config, 600, "Lax")
}
async fn catalog(State(state): State<AuthState>) -> Response {
    if !state.store.federated_auth_enabled() {
        return auth_failure(IdentityError::Forbidden.into());
    }
    match state.store.federated_catalog().await {
        Ok(providers) => success(Some(providers)),
        Err(error) => auth_failure(error.into()),
    }
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Start {
    accepted_legal: bool,
    #[serde(default)]
    affiliate: String,
}

async fn begin_login(
    State(state): State<AuthState>,
    Path(provider): Path<String>,
    input: Result<Json<Start>, JsonRejection>,
) -> Response {
    let result = async {
        let Json(input) = input.map_err(|_| IdentityError::Invalid)?;
        if state.config.require_legal_acceptance && !input.accepted_legal {
            return Err(IdentityError::Invalid.into());
        }
        let name = cookie_name(&state.config, &provider, false)?;
        let binding = external_login::random_binding()?;
        let start = state
            .browser
            .begin_federated_login(
                reference(&provider)?,
                &binding,
                &state.config.origin,
                input.accepted_legal || !state.config.require_legal_acceptance,
                &input.affiliate,
            )
            .await?;
        Ok::<_, BrowserAuthError>((name, binding, start))
    }
    .await;
    match result {
        Ok((name, binding, start)) => cookie(
            success(Some(json!(start))),
            &state.config,
            &name,
            Some(&binding),
        ),
        Err(error) => auth_failure(error),
    }
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Finish {
    state: String,
    code: String,
    iss: Option<String>,
}

async fn finish_login(
    State(state): State<AuthState>,
    Path(provider): Path<String>,
    headers: HeaderMap,
    peer: Result<ConnectInfo<SocketAddr>, axum::extract::rejection::ExtensionRejection>,
    input: Result<Json<Finish>, JsonRejection>,
) -> Response {
    let name = match cookie_name(&state.config, &provider, false) {
        Ok(name) => name,
        Err(error) => return auth_failure(error.into()),
    };
    let result = async {
        let Json(input) = input.map_err(|_| IdentityError::Invalid)?;
        let binding = external_login::cookie_value(&headers, &name, "", 64)?;
        let mfa_binding = external_login::random_binding()?;
        let metadata = BrowserSessionMetadata {
            ip: peer.map(|peer| peer.0.ip().to_string()).unwrap_or_default(),
            user_agent: headers
                .get(header::USER_AGENT)
                .and_then(|value| value.to_str().ok())
                .unwrap_or_default()
                .chars()
                .take(512)
                .collect(),
        };
        let outcome = state
            .browser
            .finish_federated_login(
                crate::identity::FederatedCallbackInput {
                    reference: reference(&provider)?,
                    state: &input.state,
                    binding,
                    origin: &state.config.origin,
                    code: &input.code,
                    issuer: input.iss.as_deref(),
                },
                &metadata,
                &mfa_binding,
            )
            .await?;
        Ok::<_, MfaError>((outcome, mfa_binding))
    }
    .await;
    let response = match result {
        Ok((FederatedLoginOutcome::Denied(message), _)) => {
            failure(StatusCode::FORBIDDEN, "OAUTH_ACCESS_DENIED", &message)
        }
        Ok((FederatedLoginOutcome::Login(BrowserLoginOutcome::Disabled(grant)), _)) => {
            account_appeals::disabled_response(grant, &state.config, false)
        }
        Ok((FederatedLoginOutcome::Login(BrowserLoginOutcome::Session(bundle)), _)) => {
            bundle_response(bundle, &state.config)
        }
        Ok((FederatedLoginOutcome::Login(BrowserLoginOutcome::Challenge(challenge)), binding)) => {
            mfa_flow_cookie(
                success(Some(json!(challenge))),
                Some(&binding),
                &state.config,
            )
        }
        Err(error) => return mfa_failure(error),
    };
    cookie(response, &state.config, &name, None)
}

pub(super) async fn begin_bind(
    state: AuthState,
    headers: HeaderMap,
    provider: &str,
    input: Result<Json<account_links::Start>, JsonRejection>,
) -> Response {
    let result = async {
        let Json(input) = input.map_err(|_| IdentityError::Invalid)?;
        let name = cookie_name(&state.config, provider, true)?;
        let binding = external_login::random_binding()?;
        let start = state
            .browser
            .begin_federated_bind(
                bearer(&headers)?.ok_or(IdentityError::Unauthorized)?,
                reference(provider)?,
                &binding,
                &state.config.origin,
                &input.expected_auth_version,
            )
            .await?;
        Ok::<_, BrowserAuthError>((name, binding, start))
    }
    .await;
    match result {
        Ok((name, binding, start)) => cookie(
            success(Some(json!(start))),
            &state.config,
            &name,
            Some(&binding),
        ),
        Err(error) => auth_failure(error),
    }
}

pub(super) async fn finish_bind(
    state: AuthState,
    headers: HeaderMap,
    provider: &str,
    input: Result<Json<AccountBindFinish>, JsonRejection>,
) -> Response {
    let name = match cookie_name(&state.config, provider, true) {
        Ok(name) => name,
        Err(error) => return auth_failure(error.into()),
    };
    let result = async {
        let Json(input) = input.map_err(|_| IdentityError::Invalid)?;
        let binding = external_login::cookie_value(&headers, &name, "", 64)?;
        state
            .browser
            .finish_federated_bind(
                bearer(&headers)?.ok_or(IdentityError::Unauthorized)?,
                crate::identity::FederatedCallbackInput {
                    reference: reference(provider)?,
                    state: &input.state,
                    binding,
                    origin: &state.config.origin,
                    code: &input.code,
                    issuer: input.iss.as_deref(),
                },
            )
            .await
    }
    .await;
    let response = match result {
        Ok(FederatedBindOutcome::Session(bundle)) => bundle_response(bundle, &state.config),
        Ok(FederatedBindOutcome::Denied(message)) => {
            failure(StatusCode::FORBIDDEN, "OAUTH_ACCESS_DENIED", &message)
        }
        Err(error) => return auth_failure(error),
    };
    cookie(response, &state.config, &name, None)
}

//! Same-origin native binding writes run in the original authenticated window.
//! Provider callbacks only relay fixed-route code/state to that popup's opener.
use super::*;
use crate::identity::{AccountBindFinish, AccountLinkProvider, AccountUnlinkRequest};
use axum::extract::Path;

pub(super) fn routes() -> Router<AuthState> {
    Router::new()
        .route("/api/user/account-links", get(list))
        .route("/api/user/account-links/unlink", post(unlink))
        .route("/api/user/account-links/{provider}/begin", post(begin))
        .route("/api/user/account-links/{provider}/finish", post(finish))
}
fn available(state: &AuthState, provider: AccountLinkProvider) -> bool {
    state.store.account_links_enabled()
        && match provider {
            AccountLinkProvider::Google => state.config.google_client_id(&state.store).is_some(),
            AccountLinkProvider::External(provider) => state
                .config
                .external_client_id(&state.store, provider)
                .is_some(),
        }
}
fn cookie_name(config: &LegacyAuthConfig, provider: AccountLinkProvider) -> String {
    format!(
        "{}lmm_account_bind_{}",
        if config.secure { "__Host-" } else { "" },
        provider.slug()
    )
}
fn flow_cookie(
    response: Response,
    config: &LegacyAuthConfig,
    provider: AccountLinkProvider,
    binding: Option<&str>,
) -> Response {
    external_login::set_flow_cookie(
        response,
        &cookie_name(config, provider),
        binding,
        config,
        600,
        "Lax",
    )
}
async fn list(State(state): State<AuthState>, headers: HeaderMap) -> Response {
    let result=async {
        let mut result=state.browser.account_links(bearer(&headers)?.ok_or(IdentityError::Unauthorized)?).await?;
        result["providers"]=json!(["google","github","linuxdo","discord"].into_iter().map(|slug| {
            let provider=AccountLinkProvider::parse(slug).expect("fixed provider");
            json!({"provider":slug,"issuer":provider.issuer(),"can_bind":available(&state,provider)})
        }).collect::<Vec<_>>());
        Ok::<_,BrowserAuthError>(result)
    }.await;
    match result {
        Ok(value) => success(Some(value)),
        Err(error) => auth_failure(error),
    }
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Start {
    expected_auth_version: String,
}
async fn begin(
    State(state): State<AuthState>,
    headers: HeaderMap,
    Path(provider): Path<String>,
    input: Result<Json<Start>, JsonRejection>,
) -> Response {
    let result = async {
        let provider = AccountLinkProvider::parse(&provider)?;
        if !available(&state, provider) {
            return Err(IdentityError::Forbidden.into());
        }
        let Json(input) = input.map_err(|_| IdentityError::Invalid)?;
        let binding = external_login::random_binding()?;
        let result = state
            .browser
            .begin_account_bind(
                bearer(&headers)?.ok_or(IdentityError::Unauthorized)?,
                provider,
                &binding,
                &input.expected_auth_version,
            )
            .await?;
        Ok::<_, BrowserAuthError>((provider, binding, result))
    }
    .await;
    match result {
        Ok((provider, binding, result)) => flow_cookie(
            success(Some(json!(result))),
            &state.config,
            provider,
            Some(&binding),
        ),
        Err(error) => auth_failure(error),
    }
}
async fn finish(
    State(state): State<AuthState>,
    headers: HeaderMap,
    Path(slug): Path<String>,
    input: Result<Json<AccountBindFinish>, JsonRejection>,
) -> Response {
    let provider = match AccountLinkProvider::parse(&slug) {
        Ok(provider) => provider,
        Err(error) => return auth_failure(error.into()),
    };
    let result = async {
        if !available(&state, provider) {
            return Err(IdentityError::Forbidden.into());
        }
        let Json(input) = input.map_err(|_| IdentityError::Invalid)?;
        let binding =
            external_login::cookie_value(&headers, &cookie_name(&state.config, provider), "", 64)?;
        state
            .browser
            .finish_account_bind(
                bearer(&headers)?.ok_or(IdentityError::Unauthorized)?,
                provider,
                binding,
                &input,
            )
            .await
    }
    .await;
    let response = match result {
        Ok(bundle) => bundle_response(bundle, &state.config),
        Err(error) => auth_failure(error),
    };
    flow_cookie(response, &state.config, provider, None)
}
async fn unlink(
    State(state): State<AuthState>,
    headers: HeaderMap,
    input: Result<Json<AccountUnlinkRequest>, JsonRejection>,
) -> Response {
    let result = async {
        let Json(input) = input.map_err(|_| IdentityError::Invalid)?;
        state
            .browser
            .unlink_account(
                bearer(&headers)?.ok_or(IdentityError::Unauthorized)?,
                &input,
                state.config.passkeys.as_deref(),
            )
            .await
    }
    .await;
    match result {
        Ok(bundle) => bundle_response(bundle, &state.config),
        Err(error) => auth_failure(error),
    }
}
pub(super) async fn callback_redirect(
    state: &AuthState,
    provider: AccountLinkProvider,
    token: Option<&str>,
    code: Option<&str>,
    error: Option<&str>,
    issuer: Option<&str>,
) -> Result<Option<Response>, BrowserAuthError> {
    let Some(token) = token else { return Ok(None) };
    if !state.browser.is_account_bind_state(provider, token).await? {
        return Ok(None);
    }
    let mut destination =
        reqwest::Url::parse(&state.config.origin).map_err(|_| IdentityError::Invalid)?;
    destination.set_path(&format!("/oauth/{}", provider.slug()));
    destination
        .query_pairs_mut()
        .append_pair("state", token)
        .append_pair("account_bind", "1");
    for (key, value) in [("code", code), ("error", error), ("iss", issuer)] {
        if let Some(value) = value {
            destination.query_pairs_mut().append_pair(key, value);
        }
    }
    Ok(Some(Redirect::to(destination.as_str()).into_response()))
}

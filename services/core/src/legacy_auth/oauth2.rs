//! Explicit native-client browser ceremony inside the existing refresh path.
use super::*;
use crate::html::escape;
use crate::identity::grants::{
    AuthorizationRequest, ConsentForm, ConsentPage, ContinueForm, ManagedAuthorizationRequest,
};

#[derive(serde::Deserialize)]
#[serde(untagged)]
enum AuthorizationInput {
    Oidc(crate::identity::grants::OidcAuthorizationRequest),
    Legacy(AuthorizationRequest),
    Managed(ManagedAuthorizationRequest),
}
use axum::extract::Path;
use axum::extract::{Form, rejection::FormRejection};
use axum::routing::delete;
use sha2::{Digest, Sha256};
pub(super) fn routes() -> Router<AuthState> {
    Router::new()
        .route("/api/oauth2/authorize", get(authorize))
        .route(
            "/api/user/auth/oauth2/continue",
            post(continue_authorization),
        )
        .route("/api/user/auth/oauth2/consent", post(consent))
        .route("/api/user/auth/oauth2/grants", get(list_grants))
        .route(
            "/api/user/auth/oauth2/grants/revoke-client",
            post(revoke_client_grants),
        )
        .route("/api/user/auth/oauth2/grants/{id}", delete(revoke_grant))
        .route(
            "/api/user/auth/oauth2/grants/revoke-all",
            post(revoke_all_grants),
        )
}
// Every immutable browser stage has a different CSRF-selected cookie name.
// Delayed responses may clear only their own stage, never another tab's flow.
// This selector grants no authority: consume still binds both independent
// random cookie and one-use CSRF to the persisted request and native Session.
fn cookie_name(config: &LegacyAuthConfig, csrf: &str) -> String {
    let prefix = if config.secure { "__Secure-" } else { "" };
    let mut hash = Sha256::new();
    hash.update(b"lmm-oauth2-browser-stage-v1\0");
    hash.update(csrf.as_bytes());
    format!(
        "{prefix}lmm_oauth2_{}",
        crate::hex_codec::encode(hash.finalize())
    )
}
fn cookie<'a>(
    headers: &'a HeaderMap,
    config: &LegacyAuthConfig,
    csrf: &str,
) -> Result<&'a str, IdentityError> {
    let expected = cookie_name(config, csrf);
    let mut found = None;
    for value in headers.get_all(header::COOKIE) {
        for part in value
            .to_str()
            .map_err(|_| IdentityError::Unauthorized)?
            .split(';')
        {
            if let Some((name, value)) = part.trim().split_once('=')
                && name == expected
            {
                if found.is_some()
                    || value.len() != 43
                    || !value
                        .bytes()
                        .all(|v| v.is_ascii_alphanumeric() || b"-_".contains(&v))
                {
                    return Err(IdentityError::Unauthorized);
                }
                found = Some(value);
            }
        }
    }
    found.ok_or(IdentityError::Unauthorized)
}
fn flow_cookie(
    mut response: Response,
    binding: Option<&str>,
    csrf: &str,
    config: &LegacyAuthConfig,
) -> Response {
    let secure = if config.secure { "Secure; " } else { "" };
    let value = format!(
        "{}={}; Path=/api/user/auth/oauth2; {secure}HttpOnly; SameSite=Strict; Max-Age={}",
        cookie_name(config, csrf),
        binding.unwrap_or_default(),
        if binding.is_some() { 300 } else { 0 }
    );
    if let Ok(value) = HeaderValue::from_str(&value) {
        response.headers_mut().append(header::SET_COOKIE, value);
    }
    response
}
const CONSENT_STYLE: &str = "*{box-sizing:border-box}html{color-scheme:light dark}body{margin:0;background:#f4f5f7;color:#20242c;font:16px/1.6 system-ui,sans-serif}main{max-width:620px;margin:7vh auto;padding:32px;background:#fff;border:1px solid #dde1e7;border-radius:20px;box-shadow:0 16px 48px #1520350a}header{border-bottom:1px solid #e3e7ed;padding-bottom:20px;margin-bottom:24px}header p{font-size:12px;font-weight:700;letter-spacing:.14em;color:#546277}h1{margin:4px 0;font-size:30px;line-height:1.2}p,li{overflow-wrap:anywhere}ul{display:flex;flex-wrap:wrap;gap:8px;padding:0;list-style:none}li{padding:4px 12px;border-radius:20px;background:#edf1f7;font-size:14px}label{display:block;margin:24px 0;padding:20px;border:1px solid #d9e0ec;border-radius:12px;font-weight:600}input[type=number]{display:block;width:100%;padding:12px;margin-top:12px;border:1px solid #9ba7ba;border-radius:8px;font:inherit;background:transparent;color:inherit}button{padding:12px 18px;margin:8px 8px 0 0;border:1px solid #aab5c5;border-radius:8px;font:600 15px system-ui;cursor:pointer}button[value=allow],button:not([value]){background:#244b8f;color:#fff;border-color:#244b8f}button[value=deny]{background:transparent;color:inherit}a{color:#244b8f}a:focus-visible,button:focus-visible,input:focus-visible{outline:3px solid #6289cc;outline-offset:3px}footer{margin-top:24px;color:#647087;font-size:13px}@media(max-width:640px){main{margin:16px;padding:24px}button{width:100%}}@media(prefers-color-scheme:dark){body{background:#14171c;color:#e5e9f1}main{background:#1c2028;border-color:#343d4c}header{border-color:#343d4c}header p,footer{color:#a8b4c8}li{background:#2c3443}label{border-color:#45516a}a{color:#a9c6ff}}";

fn page(page: ConsentPage, config: &LegacyAuthConfig) -> Response {
    let pi = page.client_id == "lmm-pi";
    let managed = matches!(page.client_id.as_str(), "lmm-cc-switch" | "lmm-magpie");
    let model = pi || managed;
    let client_name = if pi {
        "LMM Pi"
    } else if page.client_id == "lmm-magpie" {
        "LMM Magpie"
    } else if managed {
        "LMM CC Switch"
    } else {
        "LMM CLI"
    };
    let mut body = format!(
        "<!doctype html><html lang=en><meta charset=utf-8><meta name=viewport content=\"width=device-width,initial-scale=1\"><title>LMM authorization</title><style>{CONSENT_STYLE}</style><main><header><p>LMM · CLIENT AUTHORIZATION</p><h1>{client_name}</h1></header>"
    );
    if managed && let (Some(id), Some(label)) = (&page.installation_id, &page.display_label) {
        body.push_str(&format!(
            "<p>Application: {}</p><p>Installation: {}</p><p>Device: {}</p>",
            if page.client_id == "lmm-magpie" {
                "Magpie"
            } else {
                "CC Switch"
            },
            escape(id),
            escape(label)
        ));
    }
    if page.openid {
        body.push_str(
            "<p>Share your stable account identifier with this client (OpenID Connect).</p>",
        );
    }
    let action = if let Some(account) = &page.account {
        body.push_str(&format!("<p>Account: {}</p><p>Read the model catalog and your personal wallet balance.</p><p>Approved groups:</p><ul>",escape(account)));
        for group in &page.groups {
            body.push_str(&format!("<li>{}</li>", escape(group)));
        }
        body.push_str("</ul>");
        if model {
            body.push_str("<p>Allow model calls and usage for this personal account. Optional MCP/remote scopes are unavailable and will not be granted.</p>");
        }
        "/api/user/auth/oauth2/consent"
    } else {
        body.push_str(&format!(
            "<p>Sign in, then explicitly continue here. This does not authorize {client_name}.</p><p><a href=\"/sign-in?reauth=1\" target=\"_blank\" rel=\"noopener noreferrer\">Open sign in</a></p>"
        ));
        "/api/user/auth/oauth2/continue"
    };
    body.push_str(&format!(
        "<form method=post action=\"{action}\"><input type=hidden name=csrf value=\"{}\">",
        escape(&page.csrf)
    ));
    if page.account.is_some() {
        if let Some(maximum) = page.maximum_budget_credits {
            body.push_str(&format!("<label>Lifetime budget in credits (500000 credits per USD, including pending holds)<input type=number name=budget_credits min=1 max={maximum} step=1 required></label><button name=decision value=allow>Allow scoped model access</button> <button name=decision value=deny formnovalidate>Deny</button>"));
        } else {
            body.push_str("<button name=decision value=allow>Allow read-only access</button> <button name=decision value=deny>Deny</button>");
        }
    } else {
        body.push_str("<button>Continue after sign in</button>");
    }
    body.push_str("</form><footer>You can revoke client access from your account at any time.</footer></main></html>");
    let mut response = axum::response::Html(body).into_response();
    use base64::Engine as _;
    let style_hash =
        base64::engine::general_purpose::STANDARD.encode(Sha256::digest(CONSENT_STYLE.as_bytes()));
    let mut policy = format!(
        "default-src 'none'; frame-ancestors 'none'; base-uri 'none'; style-src 'sha256-{style_hash}'; form-action 'self'"
    );
    if page.account.is_some() {
        // Browsers also enforce form-action on the consent POST's redirect.
        // pending() supplies only the canonical callback validated by the issuer;
        // allow this request's exact endpoint, never arbitrary loopback ports.
        policy.push(' ');
        policy.push_str(&page.redirect_uri);
    }
    response.headers_mut().insert(
        header::CONTENT_SECURITY_POLICY,
        HeaderValue::from_str(&policy).expect("validated OAuth callback is a CSP source"),
    );
    response.headers_mut().insert(
        header::REFERRER_POLICY,
        HeaderValue::from_static("strict-origin"),
    );
    flow_cookie(response, Some(&page.binding), &page.csrf, config)
}
async fn authorize(
    State(state): State<AuthState>,
    input: Result<Query<AuthorizationInput>, QueryRejection>,
) -> Response {
    let Some(issuer) = &state.config.oauth2 else {
        return StatusCode::NOT_FOUND.into_response();
    };
    let Ok(Query(input)) = input else {
        return auth_failure(IdentityError::Invalid.into());
    };
    let result = match input {
        AuthorizationInput::Legacy(input) => issuer.begin(input).await,
        AuthorizationInput::Oidc(input) => issuer.begin_oidc(input).await,
        AuthorizationInput::Managed(input) => issuer.begin_managed(input).await,
    };
    match result {
        Ok(v) => page(v, &state.config),
        Err(e) => auth_failure(e.into()),
    }
}
async fn continue_authorization(
    State(state): State<AuthState>,
    headers: HeaderMap,
    input: Result<Form<ContinueForm>, FormRejection>,
) -> Response {
    let Some(issuer) = &state.config.oauth2 else {
        return StatusCode::NOT_FOUND.into_response();
    };
    let Ok(Form(form)) = input else {
        return auth_failure(IdentityError::Invalid.into());
    };
    let result = async {
        let binding = cookie(&headers, &state.config, &form.csrf)?;
        issuer
            .continue_authorization(binding, &form.csrf, refresh_cookie(&headers).ok().flatten())
            .await
    }
    .await;
    let response = match result {
        Ok(v) => page(v, &state.config),
        Err(e) => auth_failure(e.into()),
    };
    flow_cookie(response, None, &form.csrf, &state.config)
}
async fn consent(
    State(state): State<AuthState>,
    headers: HeaderMap,
    input: Result<Form<ConsentForm>, FormRejection>,
) -> Response {
    let Some(issuer) = &state.config.oauth2 else {
        return StatusCode::NOT_FOUND.into_response();
    };
    let Ok(Form(form)) = input else {
        return auth_failure(IdentityError::Invalid.into());
    };
    let result = async {
        let allow = match form.decision.as_str() {
            "allow" => true,
            "deny" => false,
            _ => return Err(IdentityError::Invalid),
        };
        let binding = cookie(&headers, &state.config, &form.csrf)?;
        let refresh = refresh_cookie(&headers).ok().flatten();
        issuer
            .consent_with_budget(binding, &form.csrf, refresh, allow, form.budget_credits)
            .await
    }
    .await;
    let response = match result {
        Ok(v) => Redirect::to(&v.uri).into_response(),
        Err(e) => auth_failure(e.into()),
    };
    flow_cookie(response, None, &form.csrf, &state.config)
}

async fn list_grants(State(state): State<AuthState>, headers: HeaderMap) -> Response {
    let Some(issuer) = &state.config.oauth2 else {
        return StatusCode::NOT_FOUND.into_response();
    };
    let result = async {
        let cookie = refresh_cookie(&headers)?.ok_or(IdentityError::Unauthorized)?;
        issuer.browser_grants(cookie).await
    }
    .await;
    match result {
        Ok(grants) => success(Some(serde_json::json!(grants))),
        Err(error) => auth_failure(error.into()),
    }
}

async fn revoke_grant(
    State(state): State<AuthState>,
    headers: HeaderMap,
    Path(id): Path<String>,
) -> Response {
    let Some(issuer) = &state.config.oauth2 else {
        return StatusCode::NOT_FOUND.into_response();
    };
    let result = async {
        let cookie = refresh_cookie(&headers)?.ok_or(IdentityError::Unauthorized)?;
        issuer.revoke_browser_grant(cookie, &id).await
    }
    .await;
    match result {
        Ok(()) => success(None),
        Err(error) => auth_failure(error.into()),
    }
}

async fn revoke_all_grants(State(state): State<AuthState>, headers: HeaderMap) -> Response {
    let Some(issuer) = &state.config.oauth2 else {
        return StatusCode::NOT_FOUND.into_response();
    };
    let result = async {
        let cookie = refresh_cookie(&headers)?.ok_or(IdentityError::Unauthorized)?;
        issuer.revoke_all_browser_grants(cookie).await
    }
    .await;
    match result {
        Ok(count) => success(Some(serde_json::json!({"revoked": count}))),
        Err(error) => auth_failure(error.into()),
    }
}

#[derive(serde::Deserialize)]
#[serde(deny_unknown_fields)]
struct RevokeClient {
    client_id: String,
}

async fn revoke_client_grants(
    State(state): State<AuthState>,
    headers: HeaderMap,
    input: Result<Json<RevokeClient>, JsonRejection>,
) -> Response {
    let Some(issuer) = &state.config.oauth2 else {
        return StatusCode::NOT_FOUND.into_response();
    };
    let result = async {
        let Json(form) = input.map_err(|_| IdentityError::Invalid)?;
        let cookie = refresh_cookie(&headers)?.ok_or(IdentityError::Unauthorized)?;
        issuer
            .revoke_browser_client_grants(cookie, &form.client_id)
            .await
    }
    .await;
    match result {
        Ok(count) => success(Some(serde_json::json!({"revoked": count}))),
        Err(error) => auth_failure(error.into()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[tokio::test]
    async fn consent_csp_allows_only_the_current_validated_callback() {
        use crate::identity::grants::LmmProfile;

        let config = LegacyAuthConfig::new("https://issuer.example.test", &[3; 32]).unwrap();
        let profile = LmmProfile::new("https://issuer.example.test", vec!["default".into()])
            .unwrap()
            .with_pi_budget_limit(100)
            .unwrap();
        for client in ["lmm", "lmm-pi"] {
            for port in [38423, 52917] {
                let request = profile
                    .validate_authorization(AuthorizationRequest {
                        response_type: "code".into(),
                        client_id: client.into(),
                        redirect_uri: format!("http://127.0.0.1:{port}/oauth/lmm/callback"),
                        scope: if client == "lmm-pi" {
                            "catalog:read balance:read usage:read models:invoke"
                        } else {
                            "catalog:read balance:read"
                        }
                        .into(),
                        resource: profile.resource().into(),
                        code_challenge: "A".repeat(43),
                        code_challenge_method: "S256".into(),
                        state: "test-state".into(),
                    })
                    .unwrap();
                for signed_in in [false, true] {
                    let response = page(
                        ConsentPage {
                            openid: false,
                            binding: "B".repeat(43),
                            csrf: "C".repeat(43),
                            expires_at: 300,
                            account: signed_in.then(|| "synthetic-user".into()),
                            client_id: request.client_id.clone(),
                            redirect_uri: request.redirect_uri.clone(),
                            maximum_budget_credits: (client == "lmm-pi").then_some(100),
                            installation_id: None,
                            display_label: None,
                            groups: vec!["default".into()],
                        },
                        &config,
                    );
                    let policy = response.headers()[header::CONTENT_SECURITY_POLICY]
                        .to_str()
                        .unwrap();
                    use base64::Engine as _;
                    let hash = base64::engine::general_purpose::STANDARD
                        .encode(Sha256::digest(CONSENT_STYLE.as_bytes()));
                    let mut expected = format!(
                        "default-src 'none'; frame-ancestors 'none'; base-uri 'none'; style-src 'sha256-{hash}'; form-action 'self'"
                    );
                    if signed_in {
                        expected.push(' ');
                        expected.push_str(&request.redirect_uri);
                    }
                    assert_eq!(policy, expected);
                    let body = axum::body::to_bytes(response.into_body(), 8192)
                        .await
                        .unwrap();
                    let html = std::str::from_utf8(&body).unwrap();
                    if signed_in
                        && client == "lmm-pi"
                        && port == 38423
                        && let Ok(path) = std::env::var("LMM_CONSENT_PREVIEW_FILE")
                    {
                        std::fs::write(path, html).unwrap();
                    }
                    let action = if signed_in { "consent" } else { "continue" };
                    assert!(html.contains(&format!(
                        "<form method=post action=\"/api/user/auth/oauth2/{action}\">"
                    )));
                    assert!(html.contains("type=hidden name=csrf"));
                    assert!(!html.contains(&request.redirect_uri));
                    if signed_in {
                        assert!(html.contains("name=decision value=allow"));
                        assert!(html.contains("name=decision value=deny"));
                    }
                }
            }
        }
    }

    #[test]
    fn oidc_query_discriminates_openid_without_requiring_nonce() {
        for scope in [
            "catalog%3Aread+balance%3Aread",
            "openid+catalog%3Aread+balance%3Aread",
        ] {
            let uri:axum::http::Uri=format!("/api/oauth2/authorize?response_type=code&client_id=lmm&redirect_uri=http%3A%2F%2F127.0.0.1%3A38423%2Foauth%2Flmm%2Fcallback&scope={scope}&resource=https%3A%2F%2Fissuer.example.test%2Fapi%2Foauth2&code_challenge={}&code_challenge_method=S256&state=test", "A".repeat(43)).parse().unwrap();
            let Query(parsed) = Query::<AuthorizationInput>::try_from_uri(&uri).unwrap();
            assert_eq!(
                matches!(parsed, AuthorizationInput::Oidc(_)),
                scope.starts_with("openid")
            );
        }
    }
    #[test]
    fn stage_cookie_selection_requires_its_own_random_binding() {
        let config = LegacyAuthConfig::new("https://issuer.example.test", &[3; 32]).unwrap();
        let a = "A".repeat(43);
        let b = "B".repeat(43);
        let a_name = cookie_name(&config, &a);
        let b_name = cookie_name(&config, &b);
        assert_ne!(a_name, b_name);
        let mut headers = HeaderMap::new();
        headers.insert(
            header::COOKIE,
            HeaderValue::from_str(&format!("{a_name}={a}; {b_name}={b}")).unwrap(),
        );
        assert_eq!(cookie(&headers, &config, &a).unwrap(), a);
        assert_eq!(cookie(&headers, &config, &b).unwrap(), b);
        assert!(cookie(&headers, &config, &"C".repeat(43)).is_err());
        headers.append(
            header::COOKIE,
            HeaderValue::from_str(&format!("{a_name}={a}")).unwrap(),
        );
        assert!(cookie(&headers, &config, &a).is_err());
        assert_eq!(cookie(&headers, &config, &b).unwrap(), b);
    }
    #[test]
    fn delayed_clear_is_scoped_to_the_original_stage_cookie() {
        let config = LegacyAuthConfig::new("https://issuer.example.test", &[3; 32]).unwrap();
        let a = "A".repeat(43);
        let b = "B".repeat(43);
        let response = flow_cookie(StatusCode::OK.into_response(), None, &a, &config);
        let cleared = response.headers()[header::SET_COOKIE].to_str().unwrap();
        assert!(cleared.starts_with(&format!("{}=;", cookie_name(&config, &a))));
        assert!(!cleared.contains(&cookie_name(&config, &b)));
        assert!(cleared.contains("HttpOnly; SameSite=Strict; Max-Age=0"));
        assert!(cleared.contains("Path=/api/user/auth/oauth2; Secure;"));
    }
}

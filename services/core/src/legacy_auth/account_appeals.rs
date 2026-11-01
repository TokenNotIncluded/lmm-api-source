//! Disabled ownership proofs are private cookies, never ordinary credentials.
use super::*;
use crate::identity::AccountAppealGrant;

const APPEAL_COOKIE: &str = "lmm_account_appeal";
const APPEAL_PATH: &str = "/api/user/account-action-requests";

pub(super) fn routes() -> Router<AuthState> {
    Router::new().route(
        "/api/user/account-action-requests/appeal",
        get(context).post(submit),
    )
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Submission {
    username: Option<String>,
    password: Option<String>,
    reason: String,
}

fn proof_cookie(headers: &HeaderMap) -> Result<&str, IdentityError> {
    let mut found = None;
    for raw in headers.get_all(header::COOKIE) {
        for part in raw
            .to_str()
            .map_err(|_| IdentityError::Unauthorized)?
            .split(';')
        {
            let part = part.trim();
            if part == APPEAL_COOKIE {
                return Err(IdentityError::Unauthorized);
            }
            if let Some((name, value)) = part.split_once('=')
                && name == APPEAL_COOKIE
            {
                if found.is_some()
                    || !(32..=256).contains(&value.len())
                    || !value
                        .bytes()
                        .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'_' | b'-'))
                {
                    return Err(IdentityError::Unauthorized);
                }
                found = Some(value);
            }
        }
    }
    found.ok_or(IdentityError::Unauthorized)
}

fn set_proof_cookie(mut response: Response, token: &str, config: &LegacyAuthConfig) -> Response {
    let secure = if config.secure { "Secure; " } else { "" };
    let value = format!(
        "{APPEAL_COOKIE}={token}; Path={APPEAL_PATH}; {secure}HttpOnly; SameSite=Lax; Max-Age=600"
    );
    if let Ok(value) = HeaderValue::from_str(&value) {
        response.headers_mut().append(header::SET_COOKIE, value);
    }
    response
}

pub(super) fn clear_proof_cookie(mut response: Response, config: &LegacyAuthConfig) -> Response {
    let secure = if config.secure { "Secure; " } else { "" };
    let value =
        format!("{APPEAL_COOKIE}=; Path={APPEAL_PATH}; {secure}HttpOnly; SameSite=Lax; Max-Age=0");
    if let Ok(value) = HeaderValue::from_str(&value) {
        response.headers_mut().append(header::SET_COOKIE, value);
    }
    response
}

fn clear_login_cookies(response: Response, config: &LegacyAuthConfig) -> Response {
    let response = set_cookie(response, None, config);
    let response = mfa_flow_cookie(response, None, config);
    let mut response = google_flow_cookie(response, None, config);
    let prefix = if config.secure { "__Host-" } else { "" };
    for name in [
        "lmm_external_github_flow",
        "lmm_external_linuxdo_flow",
        "lmm_external_discord_flow",
        "lmm_external_mfa_pending",
        "lmm_telegram_login_flow",
        "lmm_passkey_login",
    ] {
        response = external_login::set_flow_cookie(
            response,
            &format!("{prefix}{name}"),
            None,
            config,
            600,
            "Lax",
        );
    }
    response
}

pub(super) fn disabled_response(
    grant: AccountAppealGrant,
    config: &LegacyAuthConfig,
    direct_browser: bool,
) -> Response {
    let response = if direct_browser {
        Redirect::to("/sign-in").into_response()
    } else {
        let mut context = json!(grant.context);
        context["appeal_available"] = json!(true);
        (
            StatusCode::FORBIDDEN,
            Json(json!({
                "success":false,
                "code":"AUTH_USER_DISABLED",
                "message":"This account is disabled",
                "data":context,
            })),
        )
            .into_response()
    };
    set_proof_cookie(clear_login_cookies(response, config), &grant.token, config)
}

fn proof_failure(error: IdentityError) -> Response {
    if error == IdentityError::Unauthorized {
        failure(
            StatusCode::UNAUTHORIZED,
            "AUTH_ACCOUNT_APPEAL_PROOF_INVALID",
            "Account appeal proof is invalid or expired",
        )
    } else {
        auth_failure(error.into())
    }
}

async fn context(State(state): State<AuthState>, headers: HeaderMap) -> Response {
    let token = match proof_cookie(&headers) {
        Ok(token) => token,
        Err(error) => return proof_failure(error),
    };
    match state.browser.account_appeal_context(token).await {
        Ok(context) => success(Some(json!(context))),
        Err(error) => proof_failure(error),
    }
}

fn valid_write_origin(headers: &HeaderMap, config: &LegacyAuthConfig) -> bool {
    matches!(one_header(headers, header::ORIGIN), Ok(Some(origin)) if origin == config.origin)
}

async fn submit(
    State(state): State<AuthState>,
    headers: HeaderMap,
    input: Result<Json<Submission>, JsonRejection>,
) -> Response {
    // Unlike general legacy routes, appeal writes require Origin itself; a
    // Referer fallback cannot authorize an ownership-proof Cookie write.
    if !valid_write_origin(&headers, &state.config) {
        return failure(
            StatusCode::FORBIDDEN,
            "AUTH_ORIGIN_FORBIDDEN",
            "Request origin is not allowed",
        );
    }
    let Ok(Json(input)) = input else {
        return auth_failure(IdentityError::Invalid.into());
    };
    let grant = match (&input.username, &input.password) {
        (Some(username), Some(password)) => match state
            .browser
            .password_account_appeal(username, password)
            .await
        {
            Ok(grant) => Some(grant),
            Err(error) => return proof_failure(error),
        },
        (None, None) => None,
        _ => return auth_failure(IdentityError::Invalid.into()),
    };
    let token = match &grant {
        Some(grant) => grant.token.as_str(),
        None => match proof_cookie(&headers) {
            Ok(token) => token,
            Err(error) => return proof_failure(error),
        },
    };
    match state
        .browser
        .submit_account_appeal(token, &input.reason)
        .await
    {
        Ok(receipt) => {
            let response = success(Some(json!(receipt)));
            match grant {
                Some(grant) => set_proof_cookie(
                    clear_login_cookies(response, &state.config),
                    &grant.token,
                    &state.config,
                ),
                None => response,
            }
        }
        Err(error) => proof_failure(error),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn config() -> LegacyAuthConfig {
        LegacyAuthConfig::new("https://app.example", &[42; 32]).unwrap()
    }

    fn grant() -> AccountAppealGrant {
        AccountAppealGrant {
            token: format!("lmma_{}", "a".repeat(64)),
            context: crate::identity::AccountAppealContext {
                username: "disabled-owner".into(),
                public_reason: None,
                public_reason_available: false,
                expires_at: 1_800_000_000,
                request: None,
            },
        }
    }

    #[tokio::test]
    async fn disabled_spa_response_exposes_context_but_never_the_proof() {
        let response = disabled_response(grant(), &config(), false);
        assert_eq!(response.status(), StatusCode::FORBIDDEN);
        let proofs: Vec<_> = response
            .headers()
            .get_all(header::SET_COOKIE)
            .iter()
            .map(|v| v.to_str().unwrap())
            .filter(|v| v.starts_with("lmm_account_appeal="))
            .collect();
        assert_eq!(proofs.len(), 1);
        assert!(proofs[0].starts_with("lmm_account_appeal=lmma_"));

        assert!(!response.headers().contains_key(header::LOCATION));
        let bytes = axum::body::to_bytes(response.into_body(), 65536)
            .await
            .unwrap();
        let body: Value = serde_json::from_slice(&bytes).unwrap();
        assert_eq!(body["code"], "AUTH_USER_DISABLED");
        assert_eq!(body["data"]["appeal_available"], true);
        assert_eq!(body["data"]["username"], "disabled-owner");
        assert_eq!(body["data"]["public_reason_available"], false);
        assert!(!body.to_string().contains("lmma_"));
        assert!(body["data"].get("access_token").is_none());
        assert!(body["data"].get("flow_token").is_none());
    }

    #[tokio::test]
    async fn direct_callback_redirect_has_only_cookie_proof() {
        let response = disabled_response(grant(), &config(), true);
        assert_eq!(response.status(), StatusCode::SEE_OTHER);
        assert_eq!(response.headers()[header::LOCATION], "/sign-in");
        assert!(
            response
                .headers()
                .get_all(header::SET_COOKIE)
                .iter()
                .any(|v| v.to_str().unwrap().starts_with("lmm_account_appeal=lmma_"))
        );
        let bytes = axum::body::to_bytes(response.into_body(), 65536)
            .await
            .unwrap();
        assert!(bytes.is_empty());
    }

    #[test]
    fn ordinary_session_and_direct_callback_clear_old_proof_after_refresh_cookie() {
        for response in [success(None), Redirect::to("/console").into_response()] {
            let response = set_cookie(response, Some(("normal-refresh", 1_900_000_000)), &config());
            let cookies: Vec<_> = response
                .headers()
                .get_all(header::SET_COOKIE)
                .iter()
                .map(|v| v.to_str().unwrap())
                .collect();
            assert!(cookies[0].starts_with("new_api_refresh=normal-refresh;"));
            assert_eq!(
                cookies
                    .iter()
                    .filter(|v| v.starts_with("lmm_account_appeal="))
                    .count(),
                1
            );
            assert!(
                cookies[1]
                    .starts_with("lmm_account_appeal=; Path=/api/user/account-action-requests;")
            );
            assert!(cookies[1].contains("Secure; HttpOnly; SameSite=Lax; Max-Age=0"));
        }
    }

    #[test]
    fn proof_requires_one_private_cookie_and_never_a_bearer() {
        let mut headers = HeaderMap::new();
        headers.insert(
            header::AUTHORIZATION,
            HeaderValue::from_static("Bearer ordinary-session"),
        );
        assert_eq!(proof_cookie(&headers), Err(IdentityError::Unauthorized));
        let token = "a".repeat(64);
        headers.insert(
            header::COOKIE,
            HeaderValue::from_str(&format!("{APPEAL_COOKIE}={token}")).unwrap(),
        );
        assert_eq!(proof_cookie(&headers).unwrap(), token);
        for invalid in [
            format!("{APPEAL_COOKIE}={token}; {APPEAL_COOKIE}={token}"),
            format!("{APPEAL_COOKIE}={token}; {APPEAL_COOKIE}"),
            format!("{APPEAL_COOKIE}="),
            format!("{APPEAL_COOKIE}=short"),
            format!("{APPEAL_COOKIE}={}", "a".repeat(257)),
            format!("{APPEAL_COOKIE}={token}/bad"),
        ] {
            headers.insert(header::COOKIE, HeaderValue::from_str(&invalid).unwrap());
            assert_eq!(proof_cookie(&headers), Err(IdentityError::Unauthorized));
        }
        headers.insert(
            header::COOKIE,
            HeaderValue::from_str(&format!("{APPEAL_COOKIE}={token}")).unwrap(),
        );
        headers.append(
            header::COOKIE,
            HeaderValue::from_str(&format!("{APPEAL_COOKIE}={token}")).unwrap(),
        );
        assert_eq!(proof_cookie(&headers), Err(IdentityError::Unauthorized));
    }

    #[test]
    fn writes_require_exact_configured_origin_without_referer_fallback() {
        let config = config();
        let mut headers = HeaderMap::new();
        headers.insert(
            header::REFERER,
            HeaderValue::from_static("https://app.example/sign-in"),
        );
        assert!(!valid_write_origin(&headers, &config));
        for origin in [
            "null",
            "https://evil.example",
            "https://app.example/",
            "https://app.example.evil",
        ] {
            headers.insert(header::ORIGIN, HeaderValue::from_str(origin).unwrap());
            assert!(!valid_write_origin(&headers, &config));
        }
        headers.insert(
            header::ORIGIN,
            HeaderValue::from_static("https://app.example"),
        );
        assert!(valid_write_origin(&headers, &config));
        headers.append(
            header::ORIGIN,
            HeaderValue::from_static("https://app.example"),
        );
        assert!(!valid_write_origin(&headers, &config));
    }

    #[test]
    fn submission_rejects_authority_and_token_in_json() {
        for extra in ["token", "user_id", "status", "proof_kind", "public_reason"] {
            let mut value = json!({"reason":"Please review"});
            value[extra] = json!("untrusted");
            assert!(serde_json::from_value::<Submission>(value).is_err());
        }
        assert!(serde_json::from_value::<Submission>(json!({"reason":"Please review"})).is_ok());
        assert!(
            serde_json::from_value::<Submission>(
                json!({"username":"owner","password":"secret","reason":"Please review"})
            )
            .is_ok()
        );
    }

    #[test]
    fn proof_cookie_is_scoped_and_clears_all_login_flows() {
        let config = config();
        let response = set_proof_cookie(
            clear_login_cookies(success(None), &config),
            &"a".repeat(64),
            &config,
        );
        let cookies: Vec<_> = response
            .headers()
            .get_all(header::SET_COOKIE)
            .iter()
            .map(|v| v.to_str().unwrap())
            .collect();
        let proof = cookies
            .iter()
            .find(|v| v.starts_with("lmm_account_appeal="))
            .unwrap();
        assert!(proof.contains(
            "Path=/api/user/account-action-requests; Secure; HttpOnly; SameSite=Lax; Max-Age=600"
        ));
        for name in [
            "new_api_refresh",
            "__Host-lmm_mfa_flow",
            "__Host-lmm_web_google_flow",
            "__Host-lmm_external_github_flow",
            "__Host-lmm_external_linuxdo_flow",
            "__Host-lmm_external_discord_flow",
            "__Host-lmm_external_mfa_pending",
            "__Host-lmm_telegram_login_flow",
            "__Host-lmm_passkey_login",
        ] {
            assert!(
                cookies
                    .iter()
                    .any(|value| value.starts_with(&format!("{name}="))
                        && value.contains("Max-Age=0")),
                "{name}"
            );
        }
        let local = LegacyAuthConfig::new("http://127.0.0.1:8080", &[42; 32])
            .unwrap()
            .with_insecure_local_http()
            .unwrap();
        let response = set_proof_cookie(success(None), &"a".repeat(64), &local);
        assert!(
            !response.headers()[header::SET_COOKIE]
                .to_str()
                .unwrap()
                .contains("Secure;")
        );
    }
}

//! Host-owned Turnstile configuration and bounded server-side Siteverify calls.
use super::{absolute_path, invalid_config, private_file, setting, setting_os};
use axum::{
    Json,
    http::StatusCode,
    response::{IntoResponse, Response},
};
use serde::Deserialize;
use std::{io, path::PathBuf, sync::Arc, time::Duration};
use zeroize::Zeroizing;

const SITEVERIFY: &str = "https://challenges.cloudflare.com/turnstile/v0/siteverify";

#[derive(Deserialize, serde::Serialize)]
#[serde(deny_unknown_fields)]
pub(super) struct TurnstileFile {
    pub site_key: String,
    pub secret_key_file: PathBuf,
}
impl TurnstileFile {
    pub(super) fn validate(&self) -> io::Result<()> {
        validate_site_key(&self.site_key)?;
        absolute_path(&self.secret_key_file)
    }

    /// Load this verifier without changing the authentication-wide configuration.
    pub(super) fn load(&self, origin: &str) -> io::Result<Turnstile> {
        self.validate()?;
        let secret = Zeroizing::new(
            String::from_utf8(private_file(&self.secret_key_file, 4096)?)
                .map_err(|_| invalid_config("invalid Turnstile secret file"))?,
        );
        let secret = Zeroizing::new(secret.trim().to_owned());
        if secret.is_empty() || secret.chars().any(char::is_whitespace) {
            return Err(invalid_config("invalid Turnstile secret file"));
        }
        let url = reqwest::Url::parse(origin).map_err(|_| invalid_config("invalid Web origin"))?;
        let hostname = url
            .host_str()
            .ok_or_else(|| invalid_config("invalid Web origin"))?
            .to_owned();
        let client = reqwest::Client::builder()
            .redirect(reqwest::redirect::Policy::none())
            .timeout(Duration::from_secs(10))
            .build()
            .map_err(|_| invalid_config("unable to configure Turnstile HTTP client"))?;
        Ok(Turnstile {
            client,
            secret: Arc::new(secret),
            hostname,
            site_key: self.site_key.clone(),
            #[cfg(test)]
            test_endpoint: None,
        })
    }
}
fn validate_site_key(key: &str) -> io::Result<()> {
    if key.is_empty()
        || key.len() > 256
        || !key
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'-' | b'_'))
    {
        return Err(invalid_config("invalid Turnstile site key"));
    }
    Ok(())
}

/// A private secret and reusable HTTP client; neither is projected to public status.
#[derive(Clone)]
pub struct Turnstile {
    client: reqwest::Client,
    secret: Arc<Zeroizing<String>>,
    hostname: String,
    site_key: String,
    #[cfg(test)]
    test_endpoint: Option<String>,
}

/// Challenge rejection is distinct from an unavailable verification service.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum VerifyError {
    Rejected,
    Unavailable,
}
impl VerifyError {
    pub(crate) fn response(self) -> Response {
        let (status, code) = match self {
            Self::Rejected => (StatusCode::FORBIDDEN, "AUTH_TURNSTILE_REJECTED"),
            Self::Unavailable => (
                StatusCode::SERVICE_UNAVAILABLE,
                "AUTH_TURNSTILE_UNAVAILABLE",
            ),
        };
        (status, Json(serde_json::json!({"success":false,"message":"Unable to verify challenge","error":{"code":code}}))).into_response()
    }
}

pub(super) fn load() -> io::Result<Option<Turnstile>> {
    match (
        setting("LMM_CORE_TURNSTILE_SITE_KEY"),
        setting_os("LMM_CORE_TURNSTILE_SECRET_KEY_FILE"),
    ) {
        (Err(std::env::VarError::NotPresent), None) => Ok(None),
        (Ok(site_key), Some(path)) => {
            let origin = setting("LMM_CORE_WEB_ORIGIN")
                .map_err(|_| invalid_config("Turnstile requires Web origin"))?;
            TurnstileFile {
                site_key,
                secret_key_file: path.into(),
            }
            .load(&origin)
            .map(Some)
        }
        _ => Err(invalid_config(
            "configure both Turnstile site key and secret key file",
        )),
    }
}

/// Apply the same challenge verifier to canonical and browser email paths.
pub fn email_runtime(
    store: crate::identity::IdentityStore,
    pool: sqlx::PgPool,
) -> io::Result<Option<super::EmailRuntime>> {
    let challenge = load()?;
    let mut runtime = super::email_config::email_runtime(store, pool)?;
    if let Some(runtime) = runtime.as_mut() {
        runtime.http = runtime.http.clone().with_turnstile(challenge);
    }
    Ok(runtime)
}

/// Only public widget material is read here; startup validates the paired secret.
pub(crate) fn public_site_key() -> Option<String> {
    let key = setting("LMM_CORE_TURNSTILE_SITE_KEY").ok()?;
    validate_site_key(&key).ok()?;
    Some(key)
}

pub(crate) async fn verify(config: Option<&Turnstile>, token: &str) -> Result<(), VerifyError> {
    let Some(config) = config else { return Ok(()) };
    config.verify_at(token, SITEVERIFY).await
}

impl Turnstile {
    pub fn site_key(&self) -> &str {
        &self.site_key
    }

    /// A check-in token is valid only for this purpose and server-issued challenge.
    pub(crate) async fn verify_checkin(
        &self,
        token: &str,
        binding: &str,
    ) -> Result<(), VerifyError> {
        let endpoint = SITEVERIFY;
        #[cfg(test)]
        let endpoint = self.test_endpoint.as_deref().unwrap_or(endpoint);
        self.verify_context_at(token, endpoint, Some(binding)).await
    }

    async fn verify_at(&self, token: &str, endpoint: &str) -> Result<(), VerifyError> {
        self.verify_context_at(token, endpoint, None).await
    }

    async fn verify_context_at(
        &self,
        token: &str,
        endpoint: &str,
        checkin_binding: Option<&str>,
    ) -> Result<(), VerifyError> {
        if token.is_empty() || token.len() > 2048 || token.chars().any(char::is_whitespace) {
            return Err(VerifyError::Rejected);
        }
        #[derive(Deserialize)]
        struct Verification {
            success: bool,
            hostname: Option<String>,
            action: Option<String>,
            cdata: Option<String>,
        }
        let mut response = self
            .client
            .post(endpoint)
            .form(&[("secret", self.secret.as_str()), ("response", token)])
            .send()
            .await
            .map_err(|_| VerifyError::Unavailable)?;
        if !response.status().is_success() {
            return Err(VerifyError::Unavailable);
        }
        // Siteverify is a small JSON response. Bound the decoded body as well as time.
        let mut bytes = Vec::new();
        while let Some(chunk) = response
            .chunk()
            .await
            .map_err(|_| VerifyError::Unavailable)?
        {
            if bytes.len() + chunk.len() > 16 * 1024 {
                return Err(VerifyError::Unavailable);
            }
            bytes.extend_from_slice(&chunk);
        }
        let result: Verification =
            serde_json::from_slice(&bytes).map_err(|_| VerifyError::Unavailable)?;
        if !result.success || result.hostname.as_deref() != Some(self.hostname.as_str()) {
            return Err(VerifyError::Rejected);
        }
        if let Some(binding) = checkin_binding
            && (result.action.as_deref() != Some("daily_checkin")
                || result.cdata.as_deref() != Some(binding))
        {
            return Err(VerifyError::Rejected);
        }
        Ok(())
    }
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use axum::{Router, routing::post};

    pub(crate) fn verifier() -> Turnstile {
        Turnstile {
            client: reqwest::Client::builder()
                .no_proxy()
                .timeout(Duration::from_millis(100))
                .redirect(reqwest::redirect::Policy::none())
                .build()
                .unwrap(),
            secret: Arc::new(Zeroizing::new("test-private-secret".into())),
            hostname: "app.example.test".into(),
            site_key: "test-public-site-key".into(),
            test_endpoint: None,
        }
    }

    pub(crate) fn verifier_at(endpoint: String) -> Turnstile {
        Turnstile {
            test_endpoint: Some(endpoint),
            ..verifier()
        }
    }

    #[tokio::test]
    async fn siteverify_checks_real_http_form_and_fails_closed() {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let endpoint = format!("http://{}", listener.local_addr().unwrap());
        let app = Router::new().route("/{case}", post(|axum::extract::Path(case): axum::extract::Path<String>, axum::Form(form): axum::Form<std::collections::HashMap<String, String>>| async move {
            assert_eq!(form["secret"], "test-private-secret");
            assert_eq!(form["response"], "test-token");
            match case.as_str() {
                "ok" => (StatusCode::OK, r#"{"success":true,"hostname":"app.example.test"}"#),
                "denied" => (StatusCode::OK, r#"{"success":false,"error-codes":["timeout-or-duplicate"]}"#),
                "wrong-host" => (StatusCode::OK, r#"{"success":true,"hostname":"other.test"}"#),
                "missing-host" => (StatusCode::OK, r#"{"success":true}"#),
                "bad-json" => (StatusCode::OK, "not json"),
                "redirect" => (StatusCode::FOUND, r#"{"success":true,"hostname":"app.example.test"}"#),
                "slow" => { tokio::time::sleep(Duration::from_millis(200)).await; (StatusCode::OK, "{}") },
                _ => (StatusCode::SERVICE_UNAVAILABLE, r#"{"success":true,"hostname":"app.example.test"}"#),
            }
        }));
        let task = tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
        let config = verifier();
        for (case, expected) in [
            ("ok", Ok(())),
            ("denied", Err(VerifyError::Rejected)),
            ("wrong-host", Err(VerifyError::Rejected)),
            ("missing-host", Err(VerifyError::Rejected)),
            ("bad-json", Err(VerifyError::Unavailable)),
            ("slow", Err(VerifyError::Unavailable)),
            ("status", Err(VerifyError::Unavailable)),
            ("redirect", Err(VerifyError::Unavailable)),
        ] {
            assert_eq!(
                config
                    .verify_at("test-token", &format!("{endpoint}/{case}"))
                    .await,
                expected,
                "{case}"
            );
        }
        for token in [String::new(), "x".repeat(2049), "white space".into()] {
            assert_eq!(
                config.verify_at(&token, &endpoint).await,
                Err(VerifyError::Rejected)
            );
        }
        task.abort();
        assert_eq!(
            config.verify_at("test-token", &endpoint).await,
            Err(VerifyError::Unavailable)
        );
        assert!(verify(None, "").await.is_ok());
    }

    #[tokio::test]
    #[ignore = "contacts Cloudflare using its public always-failing test secret"]
    async fn external_siteverify_failure_is_rejected() {
        let config = Turnstile {
            client: reqwest::Client::builder()
                .timeout(Duration::from_secs(10))
                .redirect(reqwest::redirect::Policy::none())
                .build()
                .unwrap(),
            secret: Arc::new(Zeroizing::new("2x0000000000000000000000000000000AA".into())),
            hostname: "example.com".into(),
            site_key: "test-public-site-key".into(),
            test_endpoint: None,
        };
        assert_eq!(
            verify(Some(&config), "XXXX.DUMMY.TOKEN.XXXX").await,
            Err(VerifyError::Rejected)
        );
    }

    #[tokio::test]
    async fn email_paths_reject_missing_proof_before_database_or_mail() {
        use crate::{
            email_delivery::{MailKeys, Outbox},
            email_http::EmailHttp,
            identity::{IdentityStore, email::EmailChallenges},
        };
        let pool = sqlx::postgres::PgPoolOptions::new()
            .connect_lazy("postgres://unused:unused@127.0.0.1:1/unused")
            .unwrap();
        let store = IdentityStore::from_pool(pool.clone()).with_registration_enabled(true);
        let challenges = EmailChallenges::new(pool.clone(), "test", [1; 32], [2; 32]).unwrap();
        let keys = MailKeys::new("test", vec![("test".into(), vec![3; 32])]).unwrap();
        let mail = EmailHttp::new(store, challenges, Outbox::new(pool, keys))
            .with_turnstile(Some(verifier()));
        let verification = serde_json::from_value(
            serde_json::json!({"email":"user@example.test","purpose":"register"}),
        )
        .unwrap();
        let reset =
            serde_json::from_value(serde_json::json!({"email":"user@example.test"})).unwrap();
        let registration = serde_json::from_value(serde_json::json!({"username":"test-user","password":"12345678","email":"user@example.test","verification_code":"123456","accepted_legal":true})).unwrap();
        assert!(matches!(
            mail.request_verification(&axum::http::HeaderMap::new(), verification)
                .await,
            Err(crate::email_http::Failure::Forbidden)
        ));
        assert!(matches!(
            mail.request_reset(reset).await,
            Err(crate::email_http::Failure::Forbidden)
        ));
        assert!(matches!(
            mail.register(&registration, true).await,
            Err(crate::email_http::Failure::Forbidden)
        ));
    }

    #[test]
    fn configuration_requires_web_and_an_absolute_secret_path() {
        let base = "version=1\n[database]\nurl_file='/nonexistent-test/db'\n[identity.turnstile]\nsite_key='public-site-key'\nsecret_key_file='/nonexistent-test/turnstile'\n";
        assert!(super::super::parse_core_file(base.as_bytes()).is_err());
        let configured = format!(
            "{base}[identity.web]\norigin='https://app.example.test'\nrefresh_key_file='/nonexistent-test/refresh'\nuser_agreement_enabled=false\nprivacy_policy_enabled=false\n"
        );
        let values = super::super::parse_core_file(configured.as_bytes()).unwrap();
        assert_eq!(values["LMM_CORE_TURNSTILE_SITE_KEY"], "public-site-key");
        for bad in [
            configured.replace("public-site-key", ""),
            configured.replace("/nonexistent-test/turnstile", "relative"),
        ] {
            assert!(super::super::parse_core_file(bad.as_bytes()).is_err());
        }
    }
}

//! Compatibility for the console settings API.
//!
//! Runtime flags must come from the same parsed configuration used at startup.
//! The console exposes an authenticated projection of the effective settings.

use axum::{
    Json, Router,
    body::Bytes,
    extract::State,
    http::{HeaderMap, StatusCode, header},
    response::{IntoResponse, Response},
    routing::{any, get, post},
};
use serde::{Deserialize, Serialize};
use serde_json::json;
use sqlx::PgPool;
use std::collections::BTreeMap;

use crate::identity::{CredentialKind, IdentityError, IdentityStore};

mod exchange_rate;
mod ip_access;

/// Public, effective configuration; never pass raw TOML or secret references.
#[derive(Clone)]
pub struct RuntimeOptions {
    pub server_address: String,
    pub password_login_enabled: bool,
    pub email_verification_enabled: bool,
    pub passkey_login_enabled: bool,
    pub registration_enabled: bool,
    pub password_register_enabled: bool,
    pub oauth_register_enabled: bool,
    pub registration_disabled_methods: Vec<String>,
}

#[derive(Clone)]
pub struct OptionsHttp {
    pool: PgPool,
    identity: IdentityStore,
    runtime: RuntimeOptions,
    exchange_rates: exchange_rate::Reader,
}

impl OptionsHttp {
    pub fn new(pool: PgPool, identity: IdentityStore, runtime: RuntimeOptions) -> Self {
        Self {
            pool,
            identity,
            runtime,
            exchange_rates: exchange_rate::Reader::new(),
        }
    }
}

pub fn router(state: OptionsHttp) -> Router {
    Router::new()
        .route(
            "/api/option/",
            get(read).fallback(|| async { file_managed("GET, HEAD") }),
        )
        .route(
            "/api/option",
            get(read).fallback(|| async { file_managed("GET, HEAD") }),
        )
        .route("/api/option/bulk", any(|| async { file_managed("") }))
        .route("/api/option/validate", any(|| async { file_managed("") }))
        .route("/api/option/exchange-rate", get(exchange_rate::read))
        .route("/api/option/ip-access/preview", post(ip_access::preview))
        .with_state(state)
}

// Reject legacy mutations at the HTTP boundary, before parsing a body or opening
// a database transaction. No identity, including a root session, can write here.
fn file_managed(allow: &'static str) -> Response {
    let mut denied = response(
        StatusCode::METHOD_NOT_ALLOWED,
        json!({"success":false,"code":"SYSTEM_OPTIONS_READ_ONLY",
               "message":"Core system settings are managed by configuration files."}),
    );
    denied
        .headers_mut()
        .insert(header::ALLOW, header::HeaderValue::from_static(allow));
    denied
}

#[derive(Serialize)]
struct SystemOption {
    key: String,
    value: String,
    source: &'static str,
    revision: Option<String>,
}

fn effective_option(
    key: &'static str,
    default: &str,
    rows: &BTreeMap<String, (String, i64)>,
) -> SystemOption {
    let stored = match key {
        "Announcements" | "console_setting.announcements" => rows
            .get("console_setting.announcements")
            .or_else(|| rows.get("Announcements")),
        _ => rows.get(key),
    };
    let (value, source, revision) = if let Some(value) = crate::config::console_option(key) {
        (
            value,
            "core_file",
            crate::config::configuration_revision().map(str::to_owned),
        )
    } else if let Some((value, revision)) = stored {
        (value.clone(), "legacy_database", Some(revision.to_string()))
    } else {
        (default.to_owned(), "default", None)
    };
    SystemOption {
        key: key.into(),
        value,
        source,
        revision,
    }
}

fn bearer(headers: &HeaderMap) -> Result<&str, IdentityError> {
    let mut values = headers.get_all(header::AUTHORIZATION).iter();
    let value = values.next().ok_or(IdentityError::Unauthorized)?;
    if values.next().is_some() {
        return Err(IdentityError::Unauthorized);
    }
    let mut fields = value
        .to_str()
        .map_err(|_| IdentityError::Unauthorized)?
        .split_whitespace();
    Ok(match (fields.next(), fields.next(), fields.next()) {
        (Some(scheme), Some(secret), None) if scheme.eq_ignore_ascii_case("bearer") => secret,
        _ => return Err(IdentityError::Unauthorized),
    })
}

async fn authorize(state: &OptionsHttp, headers: &HeaderMap) -> Result<(), IdentityError> {
    let actor = state.identity.authorize(bearer(headers)?).await?;
    if actor.credential_kind != CredentialKind::Session || actor.platform_level < 6 {
        return Err(IdentityError::Forbidden);
    }
    Ok(())
}

fn runtime_options(runtime: &RuntimeOptions) -> Vec<SystemOption> {
    // @context-fold begin effective-public-runtime-options
    [
        (
            "ServerAddress",
            reqwest::Url::parse(&runtime.server_address)
                .ok()
                .filter(|url| matches!(url.scheme(), "http" | "https"))
                .map(|url| url.origin().ascii_serialization())
                .unwrap_or_default(),
        ),
        (
            "PasswordLoginEnabled",
            runtime.password_login_enabled.to_string(),
        ),
        (
            "EmailVerificationEnabled",
            runtime.email_verification_enabled.to_string(),
        ),
        ("passkey.enabled", runtime.passkey_login_enabled.to_string()),
        ("RegisterEnabled", runtime.registration_enabled.to_string()),
        (
            "PasswordRegisterEnabled",
            runtime.password_register_enabled.to_string(),
        ),
        (
            "OAuthRegisterEnabled",
            runtime.oauth_register_enabled.to_string(),
        ),
        (
            "RegistrationDisabledMethods",
            runtime.registration_disabled_methods.join(","),
        ),
    ]
    .into_iter()
    .map(|(key, value)| SystemOption {
        key: key.into(),
        value,
        source: "core_runtime",
        revision: crate::config::configuration_revision().map(str::to_owned),
    })
    .collect()
    // @context-fold end effective-public-runtime-options
}

/// This value controls CNY display only; no financial calculation consumes it.
pub(crate) fn display_exchange_rate(raw: &str) -> Option<f64> {
    if raw.len() > 32 || raw.trim() != raw {
        return None;
    }
    serde_json::from_str::<f64>(raw)
        .ok()
        .filter(|value| value.is_finite() && (0.0001..=1e9).contains(value))
}

async fn read(State(state): State<OptionsHttp>, headers: HeaderMap) -> Response {
    if let Err(error) = authorize(&state, &headers).await {
        return auth_error(error);
    }
    // Only these persisted options are consumed by the current Core console.
    // @context-fold begin runtime-settings-projection
    let stored = sqlx::query_as::<_, (String, String, i64)>(
        "SELECT key,value,revision FROM core_console.options
         WHERE key IN ('Notice','Announcements','console_setting.announcements','USDExchangeRate','IPAccessRoutingRules',
                       'payment_setting.compliance_confirmed','payment_setting.compliance_terms_version')
         ORDER BY key",
    )
    .fetch_all(&state.pool)
    .await;
    let stored = match stored {
        Ok(rows) => rows,
        Err(_) => {
            return response(
                StatusCode::SERVICE_UNAVAILABLE,
                json!({"success":false,"code":"SYSTEM_OPTIONS_STORAGE_UNAVAILABLE",
                    "message":"Core console settings could not be read"}),
            );
        }
    };
    let mut data = runtime_options(&state.runtime);
    let rows = stored
        .into_iter()
        .map(|(key, value, revision)| (key, (value, revision)))
        .collect();
    let rate = effective_option("USDExchangeRate", "", &rows);
    if rate.source != "default" && display_exchange_rate(&rate.value).is_none() {
        return response(
            StatusCode::SERVICE_UNAVAILABLE,
            json!({"success":false,"code":"SYSTEM_OPTIONS_INVALID_STORED_RATE",
                "message":"The stored display exchange rate is invalid"}),
        );
    }
    let mut confirmed = effective_option("payment_setting.compliance_confirmed", "false", &rows);
    confirmed.value = (confirmed.value == "true").to_string();
    data.extend([
        confirmed,
        effective_option("payment_setting.compliance_terms_version", "", &rows),
        effective_option("IPAccessRoutingRules", "", &rows),
        rate,
        effective_option("Notice", "", &rows),
        effective_option("Announcements", "[]", &rows),
        effective_option("console_setting.announcements", "[]", &rows),
    ]);
    response(
        StatusCode::OK,
        json!({
            "success":true,
            "message":"Core system settings are read-only in the console.",
            "data":data,
            "capabilities":{
                "read_only":true,
                "partial":true,
                "writable_keys":[],
                "model_price_locks":false
            }
        }),
    )
    // @context-fold end runtime-settings-projection
}

fn auth_error(error: IdentityError) -> Response {
    let (status, code, message) = match error {
        IdentityError::Unauthorized => (
            StatusCode::UNAUTHORIZED,
            "AUTH_UNAUTHORIZED",
            "Authentication is required",
        ),
        IdentityError::Forbidden => (
            StatusCode::FORBIDDEN,
            "SYSTEM_OPTIONS_FORBIDDEN",
            "A superadministrator console session is required",
        ),
        _ => (
            StatusCode::SERVICE_UNAVAILABLE,
            "SYSTEM_OPTIONS_AUTH_UNAVAILABLE",
            "Console authorization is unavailable",
        ),
    };
    response(
        status,
        json!({"success":false,"code":code,"message":message}),
    )
}

fn response(status: StatusCode, body: serde_json::Value) -> Response {
    let mut response = (status, Json(body)).into_response();
    response
        .headers_mut()
        .insert(header::CACHE_CONTROL, "no-store".parse().unwrap());
    response
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::{
        body::{Body, to_bytes},
        http::Request,
    };
    use tower::ServiceExt;

    fn runtime() -> RuntimeOptions {
        RuntimeOptions {
            server_address: "https://console.example.test".into(),
            password_login_enabled: true,
            email_verification_enabled: true,
            passkey_login_enabled: true,
            registration_enabled: false,
            password_register_enabled: false,
            oauth_register_enabled: true,
            registration_disabled_methods: vec!["github".into(), "linuxdo".into()],
        }
    }

    async fn call(
        app: &Router,
        method: &str,
        path: &str,
        secret: Option<&str>,
    ) -> (StatusCode, serde_json::Value) {
        let mut request = Request::builder().method(method).uri(path);
        if let Some(secret) = secret {
            request = request.header(header::AUTHORIZATION, format!("Bearer {secret}"));
        }
        let response = app
            .clone()
            .oneshot(request.body(Body::empty()).unwrap())
            .await
            .unwrap();
        let status = response.status();
        assert_eq!(response.headers()[header::CACHE_CONTROL], "no-store");
        let bytes = to_bytes(response.into_body(), 1 << 20).await.unwrap();
        (status, serde_json::from_slice(&bytes).unwrap())
    }

    #[test]
    fn server_address_projects_only_the_public_origin() {
        let mut configured = runtime();
        configured.server_address =
            "https://user:credential@console.example.test:8443/path?token=private#fragment".into();
        let projected = runtime_options(&configured);
        let origin = projected
            .iter()
            .find(|item| item.key == "ServerAddress")
            .unwrap();
        assert_eq!(origin.value, "https://console.example.test:8443");
    }

    #[tokio::test]
    async fn missing_authentication_does_not_touch_storage() {
        let pool = sqlx::postgres::PgPoolOptions::new()
            .connect_lazy("postgres://localhost/unused")
            .unwrap();
        let app = router(OptionsHttp::new(
            pool.clone(),
            IdentityStore::from_pool(pool),
            runtime(),
        ));
        for (method, path) in [
            ("GET", "/api/option/"),
            ("GET", "/api/option/exchange-rate?currency=CNY"),
            ("POST", "/api/option/ip-access/preview"),
        ] {
            let (status, _) = call(&app, method, path, None).await;
            assert_eq!(status, StatusCode::UNAUTHORIZED);
        }
    }

    #[tokio::test]
    async fn retired_trust_policy_routes_are_not_registered() {
        let pool = sqlx::postgres::PgPoolOptions::new()
            .connect_lazy("postgres://localhost/unused")
            .unwrap();
        let app = router(OptionsHttp::new(
            pool.clone(),
            IdentityStore::from_pool(pool),
            runtime(),
        ));
        for (method, path) in [
            ("GET", "/core/v1/admin/trust-policy"),
            ("POST", "/api/option/trust-policy/preview"),
            ("POST", "/api/option/trust-policy/challenge"),
            ("POST", "/api/option/trust-policy/confirm"),
            ("POST", "/api/option/trust-policy/resolve"),
        ] {
            let response = app
                .clone()
                .oneshot(
                    Request::builder()
                        .method(method)
                        .uri(path)
                        .body(Body::empty())
                        .unwrap(),
                )
                .await
                .unwrap();
            assert_eq!(response.status(), StatusCode::NOT_FOUND, "{method} {path}");
        }
    }

    #[sqlx::test(migrations = false)]
    async fn authentic_settings_are_allowlisted(pool: PgPool) {
        crate::core_schema::init(&pool).await.unwrap();
        let identity = IdentityStore::from_pool(pool.clone());
        let root = identity.bootstrap_user(1, 6).await.unwrap();
        let admin = identity.bootstrap_user(2, 5).await.unwrap();
        sqlx::query("INSERT INTO core_console.options(key,value) VALUES ('Notice','Existing notice'),('OAuthSecret','never expose'),('RegisterEnabled','true'),('EmailVerificationEnabled','false'),('passkey.enabled','false'),('payment_setting.compliance_confirmed','true'),('payment_setting.compliance_terms_version','old'),('payment_setting.compliance_confirmed_ip','192.0.2.1')")
            .execute(&pool).await.unwrap();
        let app = router(OptionsHttp::new(pool.clone(), identity, runtime()));
        let (status, _) = call(&app, "GET", "/api/option/", Some(&admin.secret)).await;
        assert_eq!(status, StatusCode::FORBIDDEN);
        let (status, body) = call(&app, "GET", "/api/option/", Some(&root.secret)).await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(body["capabilities"]["partial"], true);
        assert!(body["capabilities"].get("trust_level_benefits").is_none());
        assert_eq!(body["capabilities"]["read_only"], true);
        assert_eq!(body["capabilities"]["writable_keys"], json!([]));
        let (status, denied) = call(&app, "PUT", "/api/option/", Some(&root.secret)).await;
        assert_eq!(status, StatusCode::METHOD_NOT_ALLOWED);
        assert_eq!(denied["code"], "SYSTEM_OPTIONS_READ_ONLY");
        let options: std::collections::BTreeMap<String, String> =
            serde_json::from_value::<Vec<serde_json::Value>>(body["data"].clone())
                .unwrap()
                .into_iter()
                .map(|item| {
                    (
                        item["key"].as_str().unwrap().into(),
                        item["value"].as_str().unwrap().into(),
                    )
                })
                .collect();
        let item = |key: &str| {
            body["data"]
                .as_array()
                .unwrap()
                .iter()
                .find(|row| row["key"] == key)
                .unwrap()
        };
        assert_eq!(item("Notice")["source"], "legacy_database");
        assert_eq!(item("Notice")["revision"], "1");
        assert_eq!(item("USDExchangeRate")["source"], "default");
        assert_eq!(item("USDExchangeRate")["revision"], serde_json::Value::Null);
        assert_eq!(item("EmailVerificationEnabled")["source"], "core_runtime");
        assert_eq!(options["Notice"], "Existing notice");
        assert_eq!(options["RegisterEnabled"], "false");
        assert_eq!(options["PasswordLoginEnabled"], "true");
        assert_eq!(options["EmailVerificationEnabled"], "true");
        assert_eq!(options["passkey.enabled"], "true");
        assert_eq!(options["payment_setting.compliance_confirmed"], "true");
        assert_eq!(options["payment_setting.compliance_terms_version"], "old");
        assert!(!options.contains_key("payment_setting.compliance_confirmed_ip"));
        assert_eq!(options["PasswordRegisterEnabled"], "false");
        assert_eq!(options["OAuthRegisterEnabled"], "true");
        assert_eq!(options["RegistrationDisabledMethods"], "github,linuxdo");
        assert!(!options.contains_key("OAuthSecret"));
        assert_eq!(options["ServerAddress"], runtime().server_address);
        let notice: String =
            sqlx::query_scalar("SELECT value FROM core_console.options WHERE key='Notice'")
                .fetch_one(&pool)
                .await
                .unwrap();
        assert_eq!(notice, "Existing notice");
    }
}

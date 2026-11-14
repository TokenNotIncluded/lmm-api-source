//! Synthetic provider replies, real provider callbacks, browser authority and PostgreSQL.
use super::*;
use crate::{
    http::{Capabilities, PublicStatus},
    identity::{
        BrowserAuth, BrowserBundle, BrowserLoginOutcome, BrowserSessionMetadata,
        FederatedBindOutcome, FederatedCallbackInput, FederatedLoginOutcome, IdentityError,
        IdentityStore, mfa::MfaError,
    },
};
use axum::{
    Json, Router,
    body::{Body, to_bytes},
    extract::{Form, State},
    http::{HeaderMap, Request, StatusCode},
    routing::{get, post},
};
use serde_json::{Value, json};
use sqlx::PgPool;
use std::{collections::HashMap, sync::Arc};
use tower::ServiceExt;

const ORIGIN: &str = "https://registration.example.test";
const BINDING: &str = "1111111111111111111111111111111111111111111111111111111111111111";

struct ProviderServer {
    origin: String,
    task: tokio::task::JoinHandle<()>,
}
impl Drop for ProviderServer {
    fn drop(&mut self) {
        self.task.abort();
    }
}
impl ProviderServer {
    async fn start() -> Self {
        Self::start_with_profiles(HashMap::new()).await
    }

    async fn start_with_profiles(profiles: HashMap<String, Value>) -> Self {
        async fn token(Form(form): Form<HashMap<String, String>>) -> Json<Value> {
            Json(json!({"access_token":form["code"],"token_type":"Bearer"}))
        }
        async fn user(
            State(profiles): State<Arc<HashMap<String, Value>>>,
            headers: HeaderMap,
        ) -> Json<Value> {
            let subject = headers["authorization"]
                .to_str()
                .unwrap()
                .strip_prefix("Bearer ")
                .unwrap();
            Json(profiles.get(subject).cloned().unwrap_or_else(|| {
                json!({"sub":subject,"email":format!("{subject}@example.test"),"email_verified":true})
            }))
        }
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let origin = format!("http://{}", listener.local_addr().unwrap());
        let task = tokio::spawn(async move {
            axum::serve(
                listener,
                Router::new()
                    .route("/token", post(token))
                    .route("/user", get(user))
                    .with_state(Arc::new(profiles)),
            )
            .await
            .unwrap();
        });
        Self { origin, task }
    }
}

async fn provider(
    store: &IdentityStore,
    server: &ProviderServer,
    kind: ProviderKind,
    slug: &str,
    context: u8,
) -> i64 {
    let mut configuration = ProviderConfiguration {
        client_id: "synthetic-client".into(),
        authorization_endpoint: format!("{}/authorize", server.origin),
        token_endpoint: format!("{}/token", server.origin),
        user_info_endpoint: format!("{}/user", server.origin),
        bridge_url: format!("{}/bridge", server.origin),
        issuer: "https://issuer.example.test".into(),
        jwks_uri: "https://issuer.example.test/jwks".into(),
        id_token_algorithms: vec!["RS256".into()],
        ..Default::default()
    };
    store
        .validate_federated_configuration(kind, &mut configuration)
        .unwrap();
    let (key, sealed) = store
        .seal_federated_secret(&[context; 32], 1, "synthetic-secret")
        .unwrap();
    sqlx::query_scalar("INSERT INTO core_federated_auth.providers(kind,slug,name,enabled,configuration,secret_context,secret_key_id,sealed_secret) VALUES($1,$2,$2,true,$3,$4,$5,$6) RETURNING id")
        .bind(kind.as_str()).bind(slug).bind(sqlx::types::Json(configuration)).bind(vec![context;32]).bind(key).bind(sealed).fetch_one(&store.pool).await.unwrap()
}

fn browser(store: &IdentityStore) -> BrowserAuth {
    BrowserAuth::new(store.clone(), &[43; 32]).unwrap()
}
fn metadata() -> BrowserSessionMetadata {
    BrowserSessionMetadata {
        ip: "192.0.2.42".into(),
        user_agent: "registration policy regression".into(),
    }
}
fn callback<'a>(
    reference: &'a str,
    start: &'a FederatedStart,
    code: &'a str,
) -> FederatedCallbackInput<'a> {
    FederatedCallbackInput {
        reference,
        state: &start.state,
        binding: BINDING,
        origin: ORIGIN,
        code,
        issuer: None,
    }
}
async fn login(browser: &BrowserAuth, reference: &str, code: &str) -> BrowserBundle {
    // Existing identities can log in even without accepting registration terms.
    match attempt_login(browser, reference, code, code != "known")
        .await
        .unwrap()
    {
        FederatedLoginOutcome::Login(BrowserLoginOutcome::Session(bundle)) => bundle,
        _ => panic!("expected an ordinary browser session"),
    }
}

async fn attempt_login(
    browser: &BrowserAuth,
    reference: &str,
    code: &str,
    accepted_legal: bool,
) -> crate::identity::mfa::MfaResult<FederatedLoginOutcome> {
    let start = browser
        .begin_federated_login(reference, BINDING, ORIGIN, accepted_legal, "")
        .await?;
    browser
        .finish_federated_login(callback(reference, &start, code), &metadata(), BINDING)
        .await
}
async fn counts(pool: &PgPool) -> (i64, i64, i64, i64, i64) {
    sqlx::query_as("SELECT (SELECT count(*) FROM core_identity.users),(SELECT count(*) FROM core_identity.accounts),(SELECT count(*) FROM core_federated_auth.identities),(SELECT count(*) FROM core_identity.credentials),(SELECT count(*) FROM core_identity.web_sessions)")
        .fetch_one(pool).await.unwrap()
}

async fn public_capabilities(pool: &PgPool, store: &IdentityStore, expected: &[bool; 4]) {
    let status = PublicStatus {
        registration_enabled: store.registration_enabled(),
        oauth_register_enabled: store.oauth_register_enabled(),
        oauth_registration_disabled_methods: store.oauth_registration_disabled_methods().to_vec(),
        federated_auth: true,
        ..Default::default()
    };
    let app = crate::http::configured_router_with_runtime_status(
        pool.clone(),
        Capabilities::default(),
        None,
        None,
        status,
    )
    .merge(crate::legacy_auth::router(
        store.clone(),
        crate::legacy_auth::LegacyAuthConfig::new(ORIGIN, &[43; 32]).unwrap(),
    ));
    for path in ["/api/status", "/api/oauth/federated/providers"] {
        let response = app
            .clone()
            .oneshot(Request::builder().uri(path).body(Body::empty()).unwrap())
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::OK, "{path}");
        let body: Value =
            serde_json::from_slice(&to_bytes(response.into_body(), 65536).await.unwrap()).unwrap();
        let catalog = if path == "/api/status" {
            assert_eq!(
                body["data"]["oauth_register_enabled"],
                expected.iter().any(|enabled| *enabled)
            );
            assert_eq!(body["data"]["telegram_register_enabled"], false);
            assert_eq!(body["data"]["oidc_enabled"], true);
            assert_eq!(body["data"]["wechat_login"], true);
            assert_eq!(
                body["data"]["custom_oauth_providers"]
                    .as_array()
                    .unwrap()
                    .len(),
                2
            );
            &body["data"]["federated_oauth_providers"]
        } else {
            &body["data"]
        };
        let providers = catalog.as_array().unwrap();
        assert_eq!(providers.len(), 4);
        for (provider, enabled) in providers.iter().zip(expected) {
            assert_eq!(
                provider["registration_enabled"], *enabled,
                "{path}: {provider}"
            );
            assert_eq!(provider["native_federated"], true);
            for secret in [
                "client_secret",
                "sealed_secret",
                "secret_key_id",
                "secret_context",
            ] {
                assert!(provider.get(secret).is_none(), "catalog leaked {secret}");
            }
        }
    }
}

#[sqlx::test(migrations = false)]
async fn registration_policy_catalog_and_callbacks_preserve_existing_login_and_binding(
    pool: PgPool,
) {
    crate::core_schema::init(&pool).await.unwrap();
    let server = ProviderServer::start().await;
    let mut store = IdentityStore::from_pool(pool.clone())
        .with_registration_enabled(true)
        .with_oauth_register_enabled(true)
        .with_account_links(true)
        .with_federated_keys(
            "test".into(),
            vec![("test".into(), vec![47; 32])],
            vec![48; 32],
        )
        .unwrap();
    Arc::get_mut(store.federated.as_mut().unwrap())
        .unwrap()
        .network = network::ProviderNetwork::local_for_test();
    let first = provider(&store, &server, ProviderKind::CustomOauth, "example", 1).await;
    let second = provider(&store, &server, ProviderKind::CustomOauth, "other", 2).await;
    provider(&store, &server, ProviderKind::Oidc, "oidc", 3).await;
    provider(&store, &server, ProviderKind::Wechat, "wechat", 4).await;
    let reference = first.to_string();
    let bind_reference = second.to_string();
    let initial_browser = browser(&store);
    let start = initial_browser
        .begin_federated_login(&reference, BINDING, ORIGIN, true, "")
        .await
        .unwrap();
    let original = match initial_browser
        .finish_federated_login(callback(&reference, &start, "known"), &metadata(), BINDING)
        .await
        .unwrap()
    {
        FederatedLoginOutcome::Login(BrowserLoginOutcome::Session(bundle)) => bundle,
        _ => panic!("initial registration failed"),
    };
    let user = original.data["user"]["id"].clone();
    let restrict = |methods: Vec<String>| {
        store
            .clone()
            .with_oauth_registration_disabled_methods(&methods)
            .unwrap()
    };
    for (case, closed, expected) in [
        ("open", store.clone(), [true, true, true, true]),
        (
            "global",
            store.clone().with_registration_enabled(false),
            [false; 4],
        ),
        (
            "oauth",
            store.clone().with_oauth_register_enabled(false),
            [false; 4],
        ),
        (
            "id",
            restrict(vec![format!("federated:{first}")]),
            [false, true, true, true],
        ),
        (
            "slug",
            restrict(vec!["custom:example".into()]),
            [false, true, true, true],
        ),
        (
            "kind",
            restrict(vec!["custom_oauth".into()]),
            [false, false, true, true],
        ),
        (
            "oidc",
            restrict(vec!["oidc".into()]),
            [true, true, false, true],
        ),
        (
            "wechat",
            restrict(vec!["wechat".into()]),
            [true, true, true, false],
        ),
        (
            "all",
            restrict(vec![
                format!("federated:{first}"),
                format!("federated:{second}"),
                "oidc".into(),
                "wechat".into(),
            ]),
            [false; 4],
        ),
    ] {
        public_capabilities(&pool, &closed, &expected).await;
        let auth = browser(&closed);
        let known = login(&auth, &reference, "known").await;
        assert_eq!(known.data["user"]["id"], user, "{case}");
        if case == "id" {
            let other = login(&auth, &bind_reference, "separate-provider").await;
            assert_ne!(
                other.data["user"]["id"], user,
                "an exact provider restriction blocked another provider"
            );
        }
        let before = counts(&pool).await;
        let start = auth
            .begin_federated_login(&reference, BINDING, ORIGIN, true, "")
            .await
            .unwrap();
        let unknown = format!("unknown-{case}");
        let result = auth
            .finish_federated_login(callback(&reference, &start, &unknown), &metadata(), BINDING)
            .await;
        if expected[0] {
            assert!(
                matches!(
                    result,
                    Ok(FederatedLoginOutcome::Login(BrowserLoginOutcome::Session(
                        _
                    )))
                ),
                "{case}"
            );
            assert_eq!(counts(&pool).await.0, before.0 + 1, "{case}");
        } else {
            assert!(
                matches!(result, Err(MfaError::Identity(IdentityError::Forbidden))),
                "{case}"
            );
            assert_eq!(
                counts(&pool).await,
                before,
                "denial created partial identity/session: {case}"
            );
        }
        assert!(
            auth.finish_federated_login(
                callback(&reference, &start, "known"),
                &metadata(),
                BINDING
            )
            .await
            .is_err(),
            "flow replay: {case}"
        );

        // Also disable registration for the binding destination explicitly.
        // This must not register another Core user or obstruct later login.
        let mut disabled = closed.oauth_registration_disabled_methods().to_vec();
        disabled.push(format!("federated:{second}"));
        let bind_auth = browser(
            &closed
                .clone()
                .with_oauth_registration_disabled_methods(&disabled)
                .unwrap(),
        );
        let before = counts(&pool).await.0;
        let access = known.data["access_token"].as_str().unwrap();
        let links = bind_auth.account_links(access).await.unwrap();
        let version = links["expected_auth_version"].as_str().unwrap();
        let start = bind_auth
            .begin_federated_bind(access, &bind_reference, BINDING, ORIGIN, version)
            .await
            .unwrap();
        let code = format!("bound-{case}");
        let bundle = match bind_auth
            .finish_federated_bind(access, callback(&bind_reference, &start, &code))
            .await
            .unwrap()
        {
            FederatedBindOutcome::Session(bundle) => bundle,
            _ => panic!("binding denied: {case}"),
        };
        assert_eq!(bundle.data["user"]["id"], user, "{case}");
        assert_eq!(
            counts(&pool).await.0,
            before,
            "binding registered a user: {case}"
        );
        let bound = login(&bind_auth, &bind_reference, &code).await;
        assert_eq!(bound.data["user"]["id"], user, "bound login: {case}");
    }
}

#[sqlx::test(migrations = false)]
async fn registration_email_policy_checks_only_new_federated_subjects(pool: PgPool) {
    crate::core_schema::init(&pool).await.unwrap();
    let mut profiles = HashMap::from([
        (
            "known-unverified".into(),
            json!({"sub":"known","email":"known@outside.test","email_verified":false}),
        ),
        ("known-missing".into(), json!({"sub":"known"})),
        (
            "known-wrong-domain".into(),
            json!({"sub":"known","email":"known@outside.test","email_verified":true}),
        ),
    ]);
    for policy in ["verification", "domain", "both"] {
        for (variant, domain, verified) in [
            ("verified", Some("EXAMPLE.TEST"), true),
            ("unverified", Some("example.test"), false),
            ("missing", None, false),
            ("wrong-domain", Some("outside.test"), true),
            ("subdomain", Some("sub.example.test"), true),
            ("suffix", Some("notexample.test"), true),
        ] {
            let code = format!("new-{policy}-{variant}");
            let mut profile = json!({"sub":code});
            if let Some(domain) = domain {
                profile["email"] = json!(format!("{code}@{domain}"));
                profile["email_verified"] = json!(verified);
            }
            profiles.insert(code, profile);
        }
        let code = format!("collision-{policy}");
        profiles.insert(
            code.clone(),
            json!({"sub":code,"email":"known@example.test","email_verified":true}),
        );
    }
    let server = ProviderServer::start_with_profiles(profiles).await;
    let mut store = IdentityStore::from_pool(pool.clone())
        .with_registration_enabled(true)
        .with_oauth_register_enabled(true)
        .with_federated_keys(
            "test".into(),
            vec![("test".into(), vec![47; 32])],
            vec![48; 32],
        )
        .unwrap();
    Arc::get_mut(store.federated.as_mut().unwrap())
        .unwrap()
        .network = network::ProviderNetwork::local_for_test();
    let reference = provider(
        &store,
        &server,
        ProviderKind::CustomOauth,
        "email-policy",
        1,
    )
    .await
    .to_string();
    let original = match attempt_login(&browser(&store), &reference, "known", true)
        .await
        .unwrap()
    {
        FederatedLoginOutcome::Login(BrowserLoginOutcome::Session(bundle)) => bundle,
        _ => panic!("initial registration failed"),
    };
    let original_user = original.data["user"]["id"].clone();
    for (case, verification, domain_restriction) in [
        ("verification", true, false),
        ("domain", false, true),
        ("both", true, true),
    ] {
        let restricted = store
            .clone()
            .with_registration_email_verification(verification)
            .with_registration_email_policy(domain_restriction, &[" Example.TEST ".into()])
            .unwrap();
        let auth = browser(&restricted);
        for code in ["known-unverified", "known-missing", "known-wrong-domain"] {
            let bundle = match attempt_login(&auth, &reference, code, false).await.unwrap() {
                FederatedLoginOutcome::Login(BrowserLoginOutcome::Session(bundle)) => bundle,
                _ => panic!("existing identity was blocked: {case}/{code}"),
            };
            assert_eq!(bundle.data["user"]["id"], original_user, "{case}/{code}");
        }
        let user_id = original_user.as_i64().unwrap();
        let saved_email: String =
            sqlx::query_scalar("SELECT email FROM core_identity.user_profiles WHERE user_id=$1")
                .bind(user_id)
                .fetch_one(&pool)
                .await
                .unwrap();
        assert_eq!(saved_email, "known@example.test");

        for variant in [
            "verified",
            "unverified",
            "missing",
            "wrong-domain",
            "subdomain",
            "suffix",
        ] {
            let before = counts(&pool).await;
            let code = format!("new-{case}-{variant}");
            let result = attempt_login(&auth, &reference, &code, true).await;
            let allowed = variant == "verified"
                || (!domain_restriction
                    && matches!(variant, "wrong-domain" | "subdomain" | "suffix"));
            if allowed {
                let bundle = match result.unwrap() {
                    FederatedLoginOutcome::Login(BrowserLoginOutcome::Session(bundle)) => bundle,
                    _ => panic!("verified new identity denied: {case}/{variant}"),
                };
                assert_ne!(bundle.data["user"]["id"], original_user);
                assert_eq!(counts(&pool).await.0, before.0 + 1);
            } else {
                assert!(
                    matches!(result, Err(MfaError::Identity(IdentityError::Forbidden))),
                    "{case}/{variant}"
                );
                assert_eq!(counts(&pool).await, before, "{case}/{variant}");
            }
        }

        // Neither missing legal consent nor a matching email establishes a new owner.
        for (code, expected) in [
            (format!("without-legal-{case}"), IdentityError::Forbidden),
            (format!("collision-{case}"), IdentityError::Conflict),
        ] {
            let before = counts(&pool).await;
            let accepted_legal = expected == IdentityError::Conflict;
            assert!(matches!(
                attempt_login(&auth, &reference, &code, accepted_legal).await,
                Err(MfaError::Identity(error)) if error == expected
            ));
            assert_eq!(counts(&pool).await, before, "{case}/{code}");
        }
    }
}

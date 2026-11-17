//! Real HTTP and restricted-runtime PostgreSQL checks for account source networks.
use axum::{
    Router,
    body::{Body, to_bytes},
    extract::ConnectInfo,
    http::{HeaderMap, HeaderValue, Request, StatusCode, header},
};
use lmm_core::{
    core_schema,
    funding::FundingPolicy,
    identity::{IdentityError, IdentityStore, LoginRequest},
    identity_http,
    legacy_auth::{self, LegacyAuthConfig},
    native_key_read,
    network_access::{NetworkAccess, TypedConfig},
};
use serde_json::{Value, json};
use sqlx::PgPool;
use std::net::SocketAddr;
use tower::ServiceExt;

#[path = "common/runtime.rs"]
mod runtime;

const ORIGIN: &str = "https://network.example.test";
const PASSWORD: &str = "network-test-pass";
const ALLOWED: &str = "192.0.2.7";
const BLOCKED: &str = "198.51.100.8";
const POLICY_PATH: &str = "/api/user/self/ip-allowlist";

async fn setup(owner: &PgPool) -> (runtime::RuntimeRole, IdentityStore, Router) {
    core_schema::init(owner).await.unwrap();
    let runtime = runtime::RuntimeRole::create(owner).await;
    runtime.optional("core_console").await;
    runtime.optional("web_identity").await;
    let store = IdentityStore::from_pool(runtime.pool.clone()).with_registration_enabled(true);
    let routes = legacy_auth::router(
        store.clone(),
        LegacyAuthConfig::new(ORIGIN, &[43; 32]).unwrap(),
    )
    .merge(identity_http::router(store.clone()))
    .merge(native_key_read::router(runtime.pool.clone()));
    let app = NetworkAccess::new(Some(runtime.pool.clone()), TypedConfig::default())
        .unwrap()
        .protect(routes);
    (runtime, store, app)
}

fn request(
    method: &str,
    path: &str,
    peer: Option<&str>,
    secret: Option<&str>,
    body: Option<Value>,
) -> Request<Body> {
    let mut builder = Request::builder()
        .method(method)
        .uri(path)
        .header(header::ORIGIN, ORIGIN);
    if let Some(secret) = secret {
        builder = builder.header(header::AUTHORIZATION, format!("Bearer {secret}"));
    }
    let body = match body {
        Some(body) => {
            builder = builder.header(header::CONTENT_TYPE, "application/json");
            Body::from(body.to_string())
        }
        None => Body::empty(),
    };
    let mut request = builder.body(body).unwrap();
    if let Some(peer) = peer {
        request
            .extensions_mut()
            .insert(ConnectInfo(SocketAddr::new(peer.parse().unwrap(), 12345)));
    }
    request
}

async fn call(app: &Router, request: Request<Body>) -> (StatusCode, HeaderMap, Value) {
    let response = app.clone().oneshot(request).await.unwrap();
    let status = response.status();
    let headers = response.headers().clone();
    assert_eq!(headers[header::CACHE_CONTROL], "no-store");
    let bytes = to_bytes(response.into_body(), 65536).await.unwrap();
    (status, headers, serde_json::from_slice(&bytes).unwrap())
}

fn refresh_cookie(headers: &HeaderMap) -> String {
    headers
        .get_all(header::SET_COOKIE)
        .iter()
        .map(|value| value.to_str().unwrap())
        .find(|value| value.starts_with("new_api_refresh="))
        .unwrap()
        .split(';')
        .next()
        .unwrap()
        .to_owned()
}

async fn credential_snapshot(owner: &PgPool) -> Value {
    sqlx::query_scalar(
        "SELECT jsonb_build_object(
            'credentials',(SELECT jsonb_agg(to_jsonb(c) ORDER BY id) FROM core_identity.credentials c),
            'families',(SELECT jsonb_agg(to_jsonb(s) ORDER BY sid) FROM core_identity.web_sessions s))",
    )
    .fetch_one(owner)
    .await
    .unwrap()
}

#[sqlx::test(migrations = false)]
async fn user_network_policy_http_preserves_settings_and_isolates_accounts(owner: PgPool) {
    let (runtime, _, app) = setup(&owner).await;
    let installer = IdentityStore::from_pool(owner.clone()).with_registration_enabled(true);
    let alice = installer
        .register(&LoginRequest {
            login_name: "network-alice".into(),
            password: PASSWORD.into(),
        })
        .await
        .unwrap();
    let bob = installer
        .register(&LoginRequest {
            login_name: "network-bob".into(),
            password: PASSWORD.into(),
        })
        .await
        .unwrap();
    let user = installer.authorize(&alice.secret).await.unwrap().user_id;
    sqlx::query("UPDATE core_identity.user_profiles SET setting='' WHERE user_id=$1")
        .bind(user)
        .execute(&owner)
        .await
        .unwrap();
    for path in [POLICY_PATH, "/core/v1/identity"] {
        assert_eq!(
            call(
                &app,
                request("GET", path, Some(ALLOWED), Some(&alice.secret), None)
            )
            .await
            .0,
            StatusCode::OK
        );
    }
    sqlx::query("UPDATE core_identity.user_profiles SET setting=$2 WHERE user_id=$1")
        .bind(user)
        .bind(r#"{"language":"zh-CN","sidebar_modules":"kept"}"#)
        .execute(&owner)
        .await
        .unwrap();
    let input =
        json!({"allow_ips":["192.0.2.7","::ffff:192.0.2.7","192.0.2.99/24","2001:db8::a/64"]});
    let expected = json!(["192.0.2.7", "192.0.2.0/24", "2001:db8::/64"]);
    let (status, _, saved) = call(
        &app,
        request(
            "PUT",
            POLICY_PATH,
            Some(ALLOWED),
            Some(&alice.secret),
            Some(input),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{saved}");
    assert_eq!(saved["data"]["allow_ips"], expected);
    assert_eq!(saved["data"]["client_ip"], ALLOWED);
    for peer in ["::ffff:192.0.2.7", "2001:db8::123"] {
        let (status, _, loaded) = call(
            &app,
            request("GET", POLICY_PATH, Some(peer), Some(&alice.secret), None),
        )
        .await;
        assert_eq!(status, StatusCode::OK, "{loaded}");
        assert_eq!(loaded["data"]["allow_ips"], expected);
    }
    let (status, _, other) = call(
        &app,
        request("GET", POLICY_PATH, Some(BLOCKED), Some(&bob.secret), None),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(other["data"]["allow_ips"], json!([]));
    for (body, code) in [
        (
            json!({"allow_ips":["198.51.100.0/24"]}),
            "USER_IP_ALLOWLIST_LOCKOUT",
        ),
        (
            json!({"allow_ips":["invalid"]}),
            "USER_IP_ALLOWLIST_INVALID",
        ),
    ] {
        let (status, _, denied) = call(
            &app,
            request(
                "PUT",
                POLICY_PATH,
                Some(ALLOWED),
                Some(&alice.secret),
                Some(body),
            ),
        )
        .await;
        assert_eq!(status, StatusCode::BAD_REQUEST);
        assert_eq!(denied["code"], code);
    }
    let mut cross_origin = request(
        "PUT",
        POLICY_PATH,
        Some(ALLOWED),
        Some(&alice.secret),
        Some(json!({"allow_ips":[]})),
    );
    cross_origin.headers_mut().insert(
        header::ORIGIN,
        HeaderValue::from_static("https://other.example.test"),
    );
    let (status, _, denied) = call(&app, cross_origin).await;
    assert_eq!(status, StatusCode::FORBIDDEN);
    assert_eq!(denied["code"], "AUTH_ORIGIN_FORBIDDEN");
    let raw: String =
        sqlx::query_scalar("SELECT setting FROM core_identity.user_profiles WHERE user_id=$1")
            .bind(user)
            .fetch_one(&owner)
            .await
            .unwrap();
    assert_eq!(
        serde_json::from_str::<Value>(&raw).unwrap(),
        json!({
            "language":"zh-CN", "sidebar_modules":"kept", "ip_allowlist":expected,
        })
    );
    let audits: Vec<Value> = sqlx::query_scalar("SELECT target FROM core_identity.audit WHERE action='user.ip_allowlist.updated' ORDER BY id")
        .fetch_all(&owner).await.unwrap();
    assert_eq!(audits, vec![json!({"user_id":user,"network_count":3})]);
    assert_eq!(
        call(
            &app,
            request("GET", POLICY_PATH, Some(BLOCKED), Some(&alice.secret), None)
        )
        .await
        .0,
        StatusCode::FORBIDDEN
    );
    assert_eq!(
        call(
            &app,
            request(
                "PUT",
                POLICY_PATH,
                Some(ALLOWED),
                Some(&alice.secret),
                Some(json!({"allow_ips":[]}))
            )
        )
        .await
        .0,
        StatusCode::OK
    );
    assert_eq!(
        call(
            &app,
            request(
                "GET",
                "/core/v1/identity",
                Some(BLOCKED),
                Some(&alice.secret),
                None
            )
        )
        .await
        .0,
        StatusCode::OK
    );
    drop(app);
    runtime.cleanup().await;
}

#[sqlx::test(migrations = false)]
async fn user_network_policy_blocks_credentials_and_cookie_recovery(owner: PgPool) {
    let (runtime, _, app) = setup(&owner).await;
    let installer = IdentityStore::from_pool(owner.clone()).with_registration_enabled(true);
    installer
        .register(&LoginRequest {
            login_name: "network-browser".into(),
            password: PASSWORD.into(),
        })
        .await
        .unwrap();
    let login_body = json!({"username":"network-browser","password":PASSWORD});
    let (status, headers, signed_in) = call(
        &app,
        request(
            "POST",
            "/api/user/login",
            Some(ALLOWED),
            None,
            Some(login_body.clone()),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{signed_in}");
    let access = signed_in["data"]["access_token"].as_str().unwrap();
    let cookie = refresh_cookie(&headers);
    assert_eq!(
        call(
            &app,
            request(
                "PUT",
                POLICY_PATH,
                Some(ALLOWED),
                Some(access),
                Some(json!({"allow_ips":["192.0.2.0/24"]}))
            )
        )
        .await
        .0,
        StatusCode::OK
    );
    let before = credential_snapshot(&owner).await;
    for peer in [Some(BLOCKED), None] {
        let mut rejected = request("GET", "/core/v1/identity", peer, Some(access), None);
        rejected
            .headers_mut()
            .insert("x-forwarded-for", HeaderValue::from_static(ALLOWED));
        let (status, _, value) = call(&app, rejected).await;
        assert_eq!(status, StatusCode::FORBIDDEN);
        assert_eq!(value["error"]["code"], "identity_forbidden");
    }
    for (path, body) in [
        ("/api/user/login", login_body),
        (
            "/core/v1/auth/login",
            json!({"login_name":"network-browser","password":PASSWORD}),
        ),
    ] {
        let (status, headers, _) =
            call(&app, request("POST", path, Some(BLOCKED), None, Some(body))).await;
        assert_eq!(status, StatusCode::FORBIDDEN);
        assert!(!headers.contains_key(header::SET_COOKIE));
    }
    for (path, expected) in [
        ("/api/user/auth/refresh", StatusCode::FORBIDDEN),
        ("/api/user/auth/reconcile", StatusCode::SERVICE_UNAVAILABLE),
    ] {
        let mut rejected = request("POST", path, Some(BLOCKED), None, None);
        rejected
            .headers_mut()
            .insert(header::COOKIE, cookie.parse().unwrap());
        let (status, headers, body) = call(&app, rejected).await;
        assert_eq!(status, expected, "{body}");
        assert!(!headers.contains_key(header::SET_COOKIE));
        assert!(body.get("data").is_none());
    }
    assert_eq!(credential_snapshot(&owner).await, before);
    // The same current cookie remains usable after rejection; then exercise its predecessor.
    let mut allowed = request("POST", "/api/user/auth/refresh", Some(ALLOWED), None, None);
    allowed
        .headers_mut()
        .insert(header::COOKIE, cookie.parse().unwrap());
    let (status, _, rotated) = call(&app, allowed).await;
    assert_eq!(status, StatusCode::OK, "{rotated}");
    let after_rotation = credential_snapshot(&owner).await;
    let mut rejected = request("POST", "/api/user/auth/refresh", Some(BLOCKED), None, None);
    rejected
        .headers_mut()
        .insert(header::COOKIE, cookie.parse().unwrap());
    let (status, headers, _) = call(&app, rejected).await;
    assert_eq!(status, StatusCode::FORBIDDEN);
    assert!(!headers.contains_key(header::SET_COOKIE));
    assert_eq!(credential_snapshot(&owner).await, after_rotation);
    let mut allowed = request("POST", "/api/user/auth/refresh", Some(ALLOWED), None, None);
    allowed
        .headers_mut()
        .insert(header::COOKIE, cookie.parse().unwrap());
    let (status, _, recovered) = call(&app, allowed).await;
    assert_eq!(status, StatusCode::OK, "{recovered}");
    assert_eq!(
        recovered["data"]["access_token"],
        rotated["data"]["access_token"]
    );
    drop(app);
    runtime.cleanup().await;
}

#[sqlx::test(migrations = false)]
async fn user_network_policy_intersects_api_key_networks(owner: PgPool) {
    let (runtime, store, app) = setup(&owner).await;
    let installer = IdentityStore::from_pool(owner.clone());
    let session = installer.bootstrap_user(7, 1).await.unwrap();
    let actor = installer.authorize(&session.secret).await.unwrap();
    let key = installer
        .issue_key(
            &session.secret,
            actor.owner,
            FundingPolicy::new_personal_key(),
            3600,
        )
        .await
        .unwrap();
    sqlx::query("INSERT INTO core_identity.user_routing_groups(user_id,group_name,source_reference) VALUES (7,'default','network-test-fixture')")
        .execute(&owner).await.unwrap();
    sqlx::query("INSERT INTO core_identity.api_key_settings(credential_id,name,group_name,allow_ips) VALUES ($1,'network-test','default',ARRAY['192.0.2.0/25'::inet,'198.51.100.0/24'::inet])")
        .bind(key.id).execute(&owner).await.unwrap();
    assert_eq!(
        call(
            &app,
            request(
                "PUT",
                POLICY_PATH,
                Some(ALLOWED),
                Some(&session.secret),
                Some(json!({"allow_ips":["192.0.2.0/24"]}))
            )
        )
        .await
        .0,
        StatusCode::OK
    );
    for peer in [ALLOWED, "::ffff:192.0.2.7"] {
        assert!(
            store
                .authorize_model_request(
                    &key.secret,
                    "model",
                    Some(peer.parse().unwrap()),
                    "default"
                )
                .await
                .is_ok()
        );
        let (status, _, body) = call(
            &app,
            request("GET", "/v1/balance", Some(peer), Some(&key.secret), None),
        )
        .await;
        assert_eq!(status, StatusCode::FORBIDDEN);
        // Network admission succeeds; the existing separate balance-consent gate remains closed.
        assert_eq!(body["error"], "account_balance_access_required");
    }
    for peer in [Some("192.0.2.200"), Some(BLOCKED), None] {
        assert!(matches!(
            store
                .authorize_model_request(
                    &key.secret,
                    "model",
                    peer.map(|p| p.parse().unwrap()),
                    "default"
                )
                .await,
            Err(IdentityError::Forbidden)
        ));
        if let Some(peer) = peer {
            let (status, _, body) = call(
                &app,
                request("GET", "/v1/balance", Some(peer), Some(&key.secret), None),
            )
            .await;
            assert_eq!(status, StatusCode::FORBIDDEN);
            assert_eq!(body["error"], "ip_not_allowed");
        }
    }
    assert_eq!(
        call(
            &app,
            request("GET", POLICY_PATH, Some(ALLOWED), Some(&key.secret), None)
        )
        .await
        .0,
        StatusCode::FORBIDDEN
    );
    drop(app);
    drop(store);
    runtime.cleanup().await;
}

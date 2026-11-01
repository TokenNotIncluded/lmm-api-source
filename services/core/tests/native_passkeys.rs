//! Real WebAuthn signatures from the upstream software authenticator, persisted
//! through the restricted Core runtime role in SQLx-owned disposable databases.
//! SoftPasskey's simulated UV is strictly a test fixture, never server behavior.
use axum::{
    Router,
    body::{Body, to_bytes},
    http::{HeaderMap, Request, StatusCode},
};
use base64::{Engine, engine::general_purpose::URL_SAFE_NO_PAD};
use lmm_core::{
    core_schema,
    identity::{
        BrowserAuth, BrowserBundle, BrowserLoginOutcome, BrowserRegistration,
        BrowserSessionMetadata, IdentityError, IdentityStore, MfaCompletion, PasskeyChallenge,
        PasskeyConfig,
    },
    legacy_auth::{self, LegacyAuthConfig},
};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use sqlx::PgPool;
use totp_rs::{Algorithm, Builder, Secret};
use tower::ServiceExt;
use webauthn_authenticator_rs::{WebauthnAuthenticator, softpasskey::SoftPasskey};
use webauthn_rs::prelude::{Url, Uuid};

#[path = "common/runtime.rs"]
mod runtime;

const ORIGIN: &str = "https://passkeys.example.test";
const RP: &str = "passkeys.example.test";
const PASSWORD: &str = "passkey fixture";
const BINDING: &str = "1111111111111111111111111111111111111111111111111111111111111111";
const OTHER: &str = "2222222222222222222222222222222222222222222222222222222222222222";

struct Fixture {
    role: runtime::RuntimeRole,
    store: IdentityStore,
    browser: BrowserAuth,
    config: PasskeyConfig,
    app: Router,
    original: BrowserBundle,
}
async fn fixture(owner: &PgPool) -> Fixture {
    core_schema::init(owner).await.unwrap();
    let role = runtime::RuntimeRole::create(owner).await;
    role.optional("web_identity").await;
    let store = IdentityStore::from_pool(role.pool.clone())
        .with_registration_enabled(true)
        .with_mfa_keys("test".into(), vec![("test".into(), vec![47; 32])])
        .unwrap();
    let browser = BrowserAuth::new(store.clone(), &[43; 32]).unwrap();
    browser
        .register(&BrowserRegistration {
            username: "alice",
            password: PASSWORD,
            email: "",
        })
        .await
        .unwrap();
    let original = browser
        .login("alice", PASSWORD, &BrowserSessionMetadata::default())
        .await
        .unwrap();
    let config = PasskeyConfig::new(RP, ORIGIN).unwrap();
    let app = legacy_auth::router(
        store.clone(),
        LegacyAuthConfig::new(ORIGIN, &[43; 32])
            .unwrap()
            .with_passkeys(config.clone())
            .unwrap(),
    );
    Fixture {
        role,
        store,
        browser,
        config,
        app,
        original,
    }
}
fn access(bundle: &BrowserBundle) -> &str {
    bundle.data["access_token"].as_str().unwrap()
}
fn sid(bundle: &BrowserBundle) -> &str {
    bundle.data["session"]["sid"].as_str().unwrap()
}
fn user(bundle: &BrowserBundle) -> i64 {
    bundle.data["user"]["id"].as_i64().unwrap()
}
fn device() -> WebauthnAuthenticator<SoftPasskey> {
    WebauthnAuthenticator::new(SoftPasskey::new(true))
}
fn attestation(
    device: &mut WebauthnAuthenticator<SoftPasskey>,
    challenge: &PasskeyChallenge,
) -> Value {
    serde_json::to_value(
        device
            .do_registration(
                Url::parse(ORIGIN).unwrap(),
                serde_json::from_value(challenge.options.clone()).unwrap(),
            )
            .unwrap(),
    )
    .unwrap()
}
fn assertion(
    device: &mut WebauthnAuthenticator<SoftPasskey>,
    challenge: &PasskeyChallenge,
) -> Value {
    serde_json::to_value(
        device
            .do_authentication(
                Url::parse(ORIGIN).unwrap(),
                serde_json::from_value(challenge.options.clone()).unwrap(),
            )
            .unwrap(),
    )
    .unwrap()
}
// SoftPasskey does not implement resident-key discovery. Simulate only the
// client's key selection/userHandle; it still signs the server's original
// challenge, and the server retains its discoverable state and full verifier.
fn discoverable_assertion(
    device: &mut WebauthnAuthenticator<SoftPasskey>,
    challenge: &PasskeyChallenge,
    credential_id: &[u8],
    user_handle: Uuid,
) -> Value {
    let mut options = challenge.options.clone();
    assert!(
        options["publicKey"]["allowCredentials"]
            .as_array()
            .is_none_or(Vec::is_empty)
    );
    options["publicKey"]["allowCredentials"] = json!([{
        "type":"public-key", "id":URL_SAFE_NO_PAD.encode(credential_id),
    }]);
    let mut credential = serde_json::to_value(
        device
            .do_authentication(
                Url::parse(ORIGIN).unwrap(),
                serde_json::from_value(options).unwrap(),
            )
            .unwrap(),
    )
    .unwrap();
    credential["response"]["userHandle"] = json!(URL_SAFE_NO_PAD.encode(user_handle.as_bytes()));
    credential
}

async fn enroll(f: &Fixture, device: &mut WebauthnAuthenticator<SoftPasskey>) -> BrowserBundle {
    let flow = f
        .browser
        .begin_passkey_registration(&f.config, access(&f.original), BINDING)
        .await
        .unwrap();
    let credential = attestation(device, &flow);
    f.browser
        .finish_passkey_registration(
            &f.config,
            access(&f.original),
            BINDING,
            &flow.flow_token,
            credential,
            "Synthetic security key",
        )
        .await
        .unwrap()
}
async fn bounds(owner: &PgPool, sid: &str) -> Value {
    sqlx::query_scalar("SELECT jsonb_build_array(created_at,expires_at,weekly_logout,ip,user_agent) FROM core_identity.web_sessions WHERE sid=$1")
        .bind(sid).fetch_one(owner).await.unwrap()
}
async fn request(
    app: &Router,
    method: &str,
    path: &str,
    token: Option<&str>,
    origin: Option<&str>,
    cookie: Option<&str>,
    body: Value,
) -> (StatusCode, HeaderMap, Value) {
    let mut req = Request::builder()
        .method(method)
        .uri(path)
        .header("content-type", "application/json");
    if let Some(token) = token {
        req = req.header("authorization", format!("Bearer {token}"));
    }
    if let Some(origin) = origin {
        req = req.header("origin", origin);
    }
    if let Some(cookie) = cookie {
        req = req.header("cookie", cookie);
    }
    let response = app
        .clone()
        .oneshot(req.body(Body::from(body.to_string())).unwrap())
        .await
        .unwrap();
    let status = response.status();
    let headers = response.headers().clone();
    assert_eq!(headers["cache-control"], "no-store");
    let value = serde_json::from_slice(&to_bytes(response.into_body(), 65536).await.unwrap())
        .unwrap_or(Value::Null);
    (status, headers, value)
}

#[test]
fn rp_and_origin_are_exact_and_transport_secure() {
    assert!(PasskeyConfig::new(RP, ORIGIN).is_ok());
    assert!(PasskeyConfig::new("localhost", "http://localhost:3333").is_ok());
    for (rp, origin) in [
        (RP, "http://passkeys.example.test"),
        ("example.test", ORIGIN),
        (RP, "https://passkeys.example.test/path"),
        (RP, "https://passkeys.example.test?query=1"),
        (RP, "https://name@passkeys.example.test"),
    ] {
        assert!(PasskeyConfig::new(rp, origin).is_err());
    }
}

#[sqlx::test(migrations = false)]
async fn real_registration_login_and_removal_preserve_family_bounds_and_revoke_old_authority(
    pool: PgPool,
) {
    let f = fixture(&pool).await;
    let other_family = f
        .browser
        .login("alice", PASSWORD, &BrowserSessionMetadata::default())
        .await
        .unwrap();
    let before = bounds(&pool, sid(&f.original)).await;
    let mut device = device();
    let registered = enroll(&f, &mut device).await;
    assert_eq!(sid(&registered), sid(&f.original));
    assert_eq!(bounds(&pool, sid(&registered)).await, before);
    assert_eq!(
        f.store.authorize(access(&f.original)).await.err(),
        Some(IdentityError::Unauthorized)
    );
    assert_eq!(
        f.store.authorize(access(&other_family)).await.err(),
        Some(IdentityError::Unauthorized)
    );
    assert!(
        f.browser
            .refresh(&f.original.refresh_token, None)
            .await
            .is_err()
    );
    assert!(
        f.browser
            .refresh(&other_family.refresh_token, None)
            .await
            .is_err()
    );
    let status = f.browser.passkey_status(access(&registered)).await.unwrap();
    assert_eq!(status["management"], "core_recent_session");
    assert_eq!(status["credentials"].as_array().unwrap().len(), 1);
    let id = status["credentials"][0]["id"]
        .as_str()
        .unwrap()
        .parse::<i64>()
        .unwrap();

    let flow = f
        .browser
        .begin_passkey_login(&f.config, BINDING, Some("alice"))
        .await
        .unwrap();
    let signed = assertion(&mut device, &flow);
    let outcome = f
        .browser
        .finish_passkey_login(
            &f.config,
            BINDING,
            &flow.flow_token,
            signed.clone(),
            &BrowserSessionMetadata::default(),
            OTHER,
        )
        .await
        .unwrap();
    let BrowserLoginOutcome::Session(logged_in) = outcome else {
        panic!("unexpected MFA requirement")
    };
    assert_eq!(user(&logged_in), user(&registered));
    assert_eq!(
        logged_in.data["user"]["role"],
        registered.data["user"]["role"]
    );
    assert_eq!(logged_in.data["session"]["login_method"], "passkey");
    assert!(
        f.browser
            .finish_passkey_login(
                &f.config,
                BINDING,
                &flow.flow_token,
                signed,
                &BrowserSessionMetadata::default(),
                OTHER
            )
            .await
            .is_err()
    );

    let pending = f
        .browser
        .begin_passkey_login(&f.config, BINDING, Some("alice"))
        .await
        .unwrap();
    let pending_assertion = assertion(&mut device, &pending);
    let removed = f
        .browser
        .remove_passkey(access(&registered), id)
        .await
        .unwrap();
    assert_eq!(sid(&removed), sid(&registered));
    assert_eq!(bounds(&pool, sid(&removed)).await, before);
    assert_eq!(
        f.browser.passkey_status(access(&removed)).await.unwrap()["enabled"],
        false
    );
    assert_eq!(
        f.store.authorize(access(&logged_in)).await.err(),
        Some(IdentityError::Unauthorized)
    );
    assert!(
        f.browser
            .finish_passkey_login(
                &f.config,
                BINDING,
                &pending.flow_token,
                pending_assertion,
                &BrowserSessionMetadata::default(),
                OTHER
            )
            .await
            .is_err()
    );
    let history: (i64, i64) = sqlx::query_as("SELECT count(*),count(*) FILTER(WHERE revoked_at IS NOT NULL) FROM core_passkeys.credentials").fetch_one(&pool).await.unwrap();
    assert_eq!(history, (1, 1));
    f.role.cleanup().await;
}

#[sqlx::test(migrations = false)]
async fn ceremonies_require_exact_browser_binding_origin_and_one_use_signature(pool: PgPool) {
    let f = fixture(&pool).await;
    for origin in [None, Some("https://evil.example.test"), Some("null")] {
        assert_eq!(
            request(
                &f.app,
                "POST",
                "/api/user/passkey/register/begin",
                Some(access(&f.original)),
                origin,
                None,
                json!({})
            )
            .await
            .0,
            StatusCode::FORBIDDEN
        );
    }
    let (status, headers, body) = request(
        &f.app,
        "POST",
        "/api/user/passkey/register/begin",
        Some(access(&f.original)),
        Some(ORIGIN),
        None,
        json!({}),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    let cookie = headers
        .get_all("set-cookie")
        .iter()
        .map(|v| v.to_str().unwrap())
        .find(|v| v.starts_with("__Host-lmm_passkey_register="))
        .unwrap();
    assert!(
        cookie.contains("HttpOnly")
            && cookie.contains("Secure")
            && cookie.contains("SameSite=Strict")
    );
    let cookie = cookie.split(';').next().unwrap();
    let flow = body["data"]["flow_token"].as_str().unwrap();
    let mut device = device();
    let credential = serde_json::to_value(
        device
            .do_registration(
                Url::parse(ORIGIN).unwrap(),
                serde_json::from_value(body["data"]["options"].clone()).unwrap(),
            )
            .unwrap(),
    )
    .unwrap();
    let body = json!({"flow_token":flow,"credential":credential,"name":"HTTP fixture"});
    assert_eq!(
        request(
            &f.app,
            "POST",
            "/api/user/passkey/register/finish",
            Some(access(&f.original)),
            Some(ORIGIN),
            None,
            body.clone()
        )
        .await
        .0,
        StatusCode::UNAUTHORIZED
    );
    let (status, _, response) = request(
        &f.app,
        "POST",
        "/api/user/passkey/register/finish",
        Some(access(&f.original)),
        Some(ORIGIN),
        Some(cookie),
        body,
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    let access = response["data"]["access_token"].as_str().unwrap();
    let flow = f
        .browser
        .begin_passkey_login(&f.config, BINDING, Some("alice"))
        .await
        .unwrap();
    let valid = assertion(&mut device, &flow);
    assert!(
        f.browser
            .finish_passkey_login(
                &f.config,
                OTHER,
                &flow.flow_token,
                valid.clone(),
                &BrowserSessionMetadata::default(),
                OTHER
            )
            .await
            .is_err()
    );
    let mut invalid = valid.clone();
    let bytes = URL_SAFE_NO_PAD
        .decode(invalid["response"]["clientDataJSON"].as_str().unwrap())
        .unwrap();
    let mut client_data: Value = serde_json::from_slice(&bytes).unwrap();
    client_data["origin"] = json!("https://evil.example.test");
    invalid["response"]["clientDataJSON"] =
        json!(URL_SAFE_NO_PAD.encode(serde_json::to_vec(&client_data).unwrap()));
    assert!(
        f.browser
            .finish_passkey_login(
                &f.config,
                BINDING,
                &flow.flow_token,
                invalid,
                &BrowserSessionMetadata::default(),
                OTHER
            )
            .await
            .is_err()
    );
    // An invalid signed ceremony consumes its challenge; replacing the forged
    // payload with the real signed result cannot replay it.
    assert!(
        f.browser
            .finish_passkey_login(
                &f.config,
                BINDING,
                &flow.flow_token,
                valid,
                &BrowserSessionMetadata::default(),
                OTHER
            )
            .await
            .is_err()
    );
    assert!(
        f.browser.passkey_status(access).await.unwrap()["enabled"]
            .as_bool()
            .unwrap()
    );
    f.role.cleanup().await;
}

#[sqlx::test(migrations = false)]
async fn passkey_authentication_preserves_local_mfa_and_rejects_stale_counter_snapshots(
    pool: PgPool,
) {
    let f = fixture(&pool).await;
    let mut device = device();
    let registered = enroll(&f, &mut device).await;
    let first = f
        .browser
        .begin_passkey_login(&f.config, BINDING, Some("alice"))
        .await
        .unwrap();
    let second = f
        .browser
        .begin_passkey_login(&f.config, OTHER, Some("alice"))
        .await
        .unwrap();
    let first_signed = assertion(&mut device, &first);
    let second_signed = assertion(&mut device, &second);
    assert!(matches!(
        f.browser
            .finish_passkey_login(
                &f.config,
                BINDING,
                &first.flow_token,
                first_signed,
                &BrowserSessionMetadata::default(),
                OTHER
            )
            .await
            .unwrap(),
        BrowserLoginOutcome::Session(_)
    ));
    assert!(
        f.browser
            .finish_passkey_login(
                &f.config,
                OTHER,
                &second.flow_token,
                second_signed,
                &BrowserSessionMetadata::default(),
                OTHER
            )
            .await
            .is_err()
    );

    let setup = f.browser.mfa_setup(access(&registered)).await.unwrap();
    let epoch: i64 =
        sqlx::query_scalar("SELECT floor(extract(epoch FROM clock_timestamp()))::bigint")
            .fetch_one(&pool)
            .await
            .unwrap();
    let secret = Secret::try_from_base32(&setup.secret).unwrap();
    let code = Builder::new()
        .with_algorithm(Algorithm::SHA1)
        .with_digits(6)
        .with_step_duration(30)
        .with_skew(1)
        .with_secret(secret.as_bytes())
        .build()
        .unwrap()
        .generate(epoch as u64)
        .to_string();
    let enabled = f
        .browser
        .mfa_enable(access(&registered), &code)
        .await
        .unwrap();
    let families: i64 = sqlx::query_scalar("SELECT count(*) FROM core_identity.web_sessions")
        .fetch_one(&pool)
        .await
        .unwrap();
    let challenge = f
        .browser
        .begin_passkey_login(&f.config, BINDING, Some("alice"))
        .await
        .unwrap();
    let signed = assertion(&mut device, &challenge);
    let result = f
        .browser
        .finish_passkey_login(
            &f.config,
            BINDING,
            &challenge.flow_token,
            signed,
            &BrowserSessionMetadata::default(),
            OTHER,
        )
        .await
        .unwrap();
    let BrowserLoginOutcome::Challenge(mfa) = result else {
        panic!("Passkey bypassed the existing local factor")
    };
    assert_eq!(
        sqlx::query_scalar::<_, i64>("SELECT count(*) FROM core_identity.web_sessions")
            .fetch_one(&pool)
            .await
            .unwrap(),
        families
    );
    let input = MfaCompletion {
        flow_token: mfa.flow_token,
        code: setup.backup_codes[0].clone(),
    };
    assert!(f.browser.complete_mfa_login(&input, BINDING).await.is_err());
    let complete = f.browser.complete_mfa_login(&input, OTHER).await.unwrap();
    assert_eq!(user(&complete), user(&enabled));
    assert_eq!(complete.data["session"]["login_method"], "2fa");
    assert!(f.browser.complete_mfa_login(&input, OTHER).await.is_err());
    f.role.cleanup().await;
}

#[sqlx::test(migrations = false)]
async fn last_primary_method_and_inactive_accounts_cannot_be_used_to_escape_management_policy(
    pool: PgPool,
) {
    let f = fixture(&pool).await;
    let mut device = device();
    let registered = enroll(&f, &mut device).await;
    let id = f.browser.passkey_status(access(&registered)).await.unwrap()["credentials"][0]["id"]
        .as_str()
        .unwrap()
        .parse::<i64>()
        .unwrap();
    sqlx::query(
        "UPDATE core_identity.user_profiles SET password_login_allowed=false WHERE user_id=$1",
    )
    .bind(user(&registered))
    .execute(&pool)
    .await
    .unwrap();
    let flow = f
        .browser
        .begin_passkey_login(&f.config, BINDING, Some("alice"))
        .await
        .unwrap();
    let signed = assertion(&mut device, &flow);
    let BrowserLoginOutcome::Session(current) = f
        .browser
        .finish_passkey_login(
            &f.config,
            BINDING,
            &flow.flow_token,
            signed,
            &BrowserSessionMetadata::default(),
            OTHER,
        )
        .await
        .unwrap()
    else {
        panic!("unexpected MFA")
    };
    assert!(
        f.browser
            .remove_passkey(access(&current), id)
            .await
            .is_err()
    );
    assert_eq!(
        f.browser.passkey_status(access(&current)).await.unwrap()["enabled"],
        true
    );

    // Owner-only synthetic fixture: create a valid, eleven-minute-old primary
    // browser family by appending evidence, without disabling immutable guards.
    let old_access = format!("lmms_{}", "a".repeat(64));
    let old_sid = "b".repeat(64);
    let principal = f.store.authorize(access(&current)).await.unwrap();
    let current_version: i64 =
        sqlx::query_scalar("SELECT auth_version FROM core_identity.users WHERE id=$1")
            .bind(principal.user_id)
            .fetch_one(&pool)
            .await
            .unwrap();
    let id: i64 = sqlx::query_scalar("INSERT INTO core_identity.credentials(digest,kind,user_id,user_version,owner_account_id,expires_at,session_started_at) SELECT $1,'session',user_id,user_version,owner_account_id,clock_timestamp()+interval '1 hour',clock_timestamp()-interval '11 minutes' FROM core_identity.credentials WHERE id=$2 RETURNING id")
        .bind(Sha256::digest(old_access.as_bytes()).to_vec()).bind(principal.credential_id).fetch_one(&pool).await.unwrap();
    sqlx::query("INSERT INTO core_identity.session_authentication(credential_id,user_id,user_version,factor_revision,factor_level,method,authenticated_at) VALUES($1,$2,$3,0,1,'primary',clock_timestamp()-interval '11 minutes')")
        .bind(id).bind(principal.user_id).bind(current_version).execute(&pool).await.unwrap();
    sqlx::query("INSERT INTO core_identity.web_sessions(sid,user_id,user_version,current_credential_id,refresh_digest,ip,user_agent,created_at,expires_at) VALUES($1,$2,$3,$4,$5,'','synthetic old family',clock_timestamp()-interval '11 minutes',clock_timestamp()+interval '1 hour')")
        .bind(&old_sid).bind(principal.user_id).bind(current_version).bind(id).bind(vec![42_u8;32]).execute(&pool).await.unwrap();
    sqlx::query(
        "INSERT INTO core_identity.web_session_credentials(credential_id,sid) VALUES($1,$2)",
    )
    .bind(id)
    .bind(&old_sid)
    .execute(&pool)
    .await
    .unwrap();
    assert!(f.store.authorize(&old_access).await.is_ok());
    assert!(f.browser.passkey_status(&old_access).await.is_ok());
    assert!(
        f.browser
            .begin_passkey_registration(&f.config, &old_access, BINDING)
            .await
            .is_err()
    );

    let flow = f
        .browser
        .begin_passkey_login(&f.config, BINDING, Some("alice"))
        .await
        .unwrap();
    let signed = assertion(&mut device, &flow);
    sqlx::query("UPDATE core_identity.users SET active=false WHERE id=$1")
        .bind(user(&current))
        .execute(&pool)
        .await
        .unwrap();
    assert!(
        f.browser
            .finish_passkey_login(
                &f.config,
                BINDING,
                &flow.flow_token,
                signed,
                &BrowserSessionMetadata::default(),
                OTHER
            )
            .await
            .is_err()
    );
    f.role.cleanup().await;
}

#[sqlx::test(migrations = false)]
async fn passkey_only_disabled_named_and_discoverable_signatures_grant_only_appeals(pool: PgPool) {
    let f = fixture(&pool).await;
    let mut device = device();
    let registered = enroll(&f, &mut device).await;
    let user = user(&registered);
    let (credential_id, handle): (Vec<u8>, String) = sqlx::query_as("SELECT k.credential_id,h.user_handle::text FROM core_passkeys.credentials k JOIN core_passkeys.user_handles h ON h.user_id=k.user_id WHERE k.user_id=$1 AND k.revoked_at IS NULL")
        .bind(user).fetch_one(&pool).await.unwrap();
    let handle = Uuid::parse_str(&handle).unwrap();
    sqlx::query("UPDATE core_identity.user_profiles SET password_login_allowed=FALSE,second_factor_required=TRUE WHERE user_id=$1")
        .bind(user).execute(&pool).await.unwrap();
    sqlx::query(
        "UPDATE core_identity.users SET active=FALSE,auth_version=auth_version+1 WHERE id=$1",
    )
    .bind(user)
    .execute(&pool)
    .await
    .unwrap();
    assert!(matches!(
        f.browser.password_account_appeal("alice", PASSWORD).await,
        Err(IdentityError::Unauthorized)
    ));
    let snapshot_sql = "SELECT (SELECT count(*) FROM core_identity.credentials),(SELECT count(*) FROM core_identity.web_sessions),(SELECT count(*) FROM core_identity.mfa_challenges),(SELECT count(*) FROM core_identity.mfa_factors)";
    let before: (i64, i64, i64, i64) = sqlx::query_as(snapshot_sql).fetch_one(&pool).await.unwrap();
    for username in [Some("alice"), None] {
        let flow = f
            .browser
            .begin_passkey_login(&f.config, BINDING, username)
            .await
            .unwrap();
        let signed = if username.is_some() {
            assertion(&mut device, &flow)
        } else {
            discoverable_assertion(&mut device, &flow, &credential_id, handle)
        };
        let grant = match f
            .browser
            .finish_passkey_login(
                &f.config,
                BINDING,
                &flow.flow_token,
                signed.clone(),
                &BrowserSessionMetadata::default(),
                OTHER,
            )
            .await
            .unwrap()
        {
            BrowserLoginOutcome::Disabled(grant) => grant,
            _ => panic!("verified disabled Passkey-only ownership must only issue an appeal proof"),
        };
        assert_eq!(grant.context.username, "alice");
        assert!(grant.context.request.is_none());
        assert!(matches!(
            f.store.authorize(&grant.token).await,
            Err(IdentityError::Unauthorized)
        ));
        let kind: String =
            sqlx::query_scalar("SELECT proof_kind FROM core_account_appeals.flows WHERE digest=$1")
                .bind(Sha256::digest(grant.token.as_bytes()).to_vec())
                .fetch_one(&pool)
                .await
                .unwrap();
        assert_eq!(kind, "passkey");
        let receipt = f
            .browser
            .submit_account_appeal(
                &grant.token,
                "Please review my Passkey-only account suspension",
            )
            .await
            .unwrap();
        assert_eq!(receipt.status, "pending");
        assert_eq!(
            f.browser
                .account_appeal_context(&grant.token)
                .await
                .unwrap()
                .request
                .unwrap()
                .id,
            receipt.id
        );
        assert!(
            f.browser
                .finish_passkey_login(
                    &f.config,
                    BINDING,
                    &flow.flow_token,
                    signed,
                    &BrowserSessionMetadata::default(),
                    OTHER
                )
                .await
                .is_err()
        );
        let after: (i64, i64, i64, i64) =
            sqlx::query_as(snapshot_sql).fetch_one(&pool).await.unwrap();
        assert_eq!(after, before);
    }
    let policy: (bool,bool) = sqlx::query_as("SELECT password_login_allowed,second_factor_required FROM core_identity.user_profiles WHERE user_id=$1")
        .bind(user).fetch_one(&pool).await.unwrap();
    assert_eq!(policy, (false, true));
    f.role.cleanup().await;
}

#[sqlx::test(migrations = false)]
async fn disabled_passkey_proof_still_requires_signature_binding_and_live_account(pool: PgPool) {
    let f = fixture(&pool).await;
    let mut device = device();
    let registered = enroll(&f, &mut device).await;
    let user = user(&registered);
    let (credential_id, handle): (Vec<u8>, String) = sqlx::query_as("SELECT k.credential_id,h.user_handle::text FROM core_passkeys.credentials k JOIN core_passkeys.user_handles h ON h.user_id=k.user_id WHERE k.user_id=$1 AND k.revoked_at IS NULL")
        .bind(user).fetch_one(&pool).await.unwrap();
    let handle = Uuid::parse_str(&handle).unwrap();
    sqlx::query(
        "UPDATE core_identity.users SET active=FALSE,auth_version=auth_version+1 WHERE id=$1",
    )
    .bind(user)
    .execute(&pool)
    .await
    .unwrap();
    let before: i64 = sqlx::query_scalar("SELECT count(*) FROM core_account_appeals.flows")
        .fetch_one(&pool)
        .await
        .unwrap();
    for username in [Some("alice"), None] {
        let flow = f
            .browser
            .begin_passkey_login(&f.config, BINDING, username)
            .await
            .unwrap();
        let mut signed = if username.is_some() {
            assertion(&mut device, &flow)
        } else {
            discoverable_assertion(&mut device, &flow, &credential_id, handle)
        };
        // A genuine assertion with the wrong browser binding cannot mint proof.
        assert!(
            f.browser
                .finish_passkey_login(
                    &f.config,
                    OTHER,
                    &flow.flow_token,
                    signed.clone(),
                    &BrowserSessionMetadata::default(),
                    OTHER
                )
                .await
                .is_err()
        );
        let mut signature = URL_SAFE_NO_PAD
            .decode(signed["response"]["signature"].as_str().unwrap())
            .unwrap();
        signature[0] ^= 1;
        signed["response"]["signature"] = json!(URL_SAFE_NO_PAD.encode(signature));
        assert!(
            f.browser
                .finish_passkey_login(
                    &f.config,
                    BINDING,
                    &flow.flow_token,
                    signed,
                    &BrowserSessionMetadata::default(),
                    OTHER
                )
                .await
                .is_err()
        );
    }
    assert_eq!(
        sqlx::query_scalar::<_, i64>("SELECT count(*) FROM core_account_appeals.flows")
            .fetch_one(&pool)
            .await
            .unwrap(),
        before
    );
    // Start a valid named ceremony before closure to exercise finish's account
    // guard too; a new named start must be rejected after closure.
    let named = f
        .browser
        .begin_passkey_login(&f.config, BINDING, Some("alice"))
        .await
        .unwrap();
    let named_signed = assertion(&mut device, &named);
    sqlx::query("UPDATE core_identity.accounts SET active=FALSE WHERE id=(SELECT personal_account_id FROM core_identity.users WHERE id=$1)")
        .bind(user).execute(&pool).await.unwrap();
    assert!(
        f.browser
            .begin_passkey_login(&f.config, BINDING, Some("alice"))
            .await
            .is_err()
    );
    assert!(
        f.browser
            .finish_passkey_login(
                &f.config,
                BINDING,
                &named.flow_token,
                named_signed,
                &BrowserSessionMetadata::default(),
                OTHER
            )
            .await
            .is_err()
    );
    let discoverable = f
        .browser
        .begin_passkey_login(&f.config, BINDING, None)
        .await
        .unwrap();
    let signed = discoverable_assertion(&mut device, &discoverable, &credential_id, handle);
    assert!(
        f.browser
            .finish_passkey_login(
                &f.config,
                BINDING,
                &discoverable.flow_token,
                signed,
                &BrowserSessionMetadata::default(),
                OTHER
            )
            .await
            .is_err()
    );
    assert_eq!(
        sqlx::query_scalar::<_, i64>("SELECT count(*) FROM core_account_appeals.flows")
            .fetch_one(&pool)
            .await
            .unwrap(),
        before
    );
    f.role.cleanup().await;
}

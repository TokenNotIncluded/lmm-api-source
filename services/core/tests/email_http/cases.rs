//! Actual Axum -> business -> proof -> encrypted outbox -> restricted PG calls.
use super::*;
use aes_gcm::{
    Aes256Gcm, KeyInit, Nonce,
    aead::{Aead, Payload},
};
use axum::{
    Router,
    body::{Body, to_bytes},
    http::{HeaderMap, Request as HttpRequest, StatusCode},
};
use lmm_core::{
    email_delivery::{MailKeys, Outbox},
    email_http::EmailHttp,
    identity_http,
    legacy_auth::{self, LegacyAuthConfig},
};
use serde_json::{Value, json};
use tower::ServiceExt;
use zeroize::Zeroizing;

const WEB: &str = "https://email.example.test";
const VERIFY: &str = "/api/identity/email/verification/request";
const RESET_REQUEST: &str = "/api/identity/email/password-reset/request";
const RESET: &str = "/api/user/reset";
const BIND: &str = "/api/oauth/email/bind";
const REGISTER: &str = "/api/user/register";

async fn setup() -> (Fixture, Router) {
    let f = fixture().await;
    let outbox = Outbox::new(
        f.runtime.clone(),
        MailKeys::new("http-test", vec![("http-test".into(), vec![79; 32])]).unwrap(),
    );
    let mail = EmailHttp::new(f.store.clone(), f.challenges.clone(), outbox);
    let router = legacy_auth::router_with_email(
        f.store.clone(),
        LegacyAuthConfig::new(WEB, &[43; 32]).unwrap(),
        mail.clone(),
    )
    .merge(identity_http::router_with_email(f.store.clone(), mail));
    (f, router)
}

async fn call(
    router: &Router,
    path: &str,
    body: Value,
    auth: Option<(&str, &str)>,
) -> (StatusCode, Value) {
    let mut request = HttpRequest::builder()
        .method("POST")
        .uri(path)
        .header("origin", WEB)
        .header("content-type", "application/json");
    if let Some((access, sid)) = auth {
        request = request
            .header("authorization", format!("Bearer {access}"))
            .header("x-auth-session", sid);
    }
    let reply = router
        .clone()
        .oneshot(request.body(Body::from(body.to_string())).unwrap())
        .await
        .unwrap();
    assert_eq!(reply.headers().get("cache-control").unwrap(), "no-store");
    assert_eq!(
        reply.headers().get("referrer-policy").unwrap(),
        "no-referrer"
    );
    assert!(
        !reply.headers().contains_key("set-cookie"),
        "email writes mint no cookie"
    );
    let status = reply.status();
    let bytes = to_bytes(reply.into_body(), 16 * 1024).await.unwrap();
    let value: Value = serde_json::from_slice(&bytes).expect("fixed JSON envelope");
    (status, value)
}

async fn issue_window(f: &Fixture) {
    // Only the synthetic owner's issuance timestamps move. Proof TTL is untouched.
    sqlx::query(
        "UPDATE core_identity.email_rate_events SET occurred_at=occurred_at-interval '61 seconds'
         WHERE kind='issue'",
    )
    .execute(&f.owner)
    .await
    .unwrap();
}

async fn count(f: &Fixture, sql: &str) -> i64 {
    sqlx::query_scalar(sql).fetch_one(&f.owner).await.unwrap()
}

async fn current_code(f: &Fixture, email: &str, purpose: &str) -> Zeroizing<String> {
    let row = sqlx::query(
        "SELECT o.* FROM core_identity.email_delivery_outbox o
         JOIN core_identity.email_challenges c ON c.id=o.challenge_id
         WHERE c.email_key=$1 AND c.purpose=$2 AND c.state='active' AND o.kind='challenge'
             AND o.payload IS NOT NULL ORDER BY o.created_at DESC LIMIT 1",
    )
    .bind(email)
    .bind(purpose)
    .fetch_one(&f.owner)
    .await
    .expect("HTTP issuance must have a recoverable encrypted payload");
    let mut aad = Vec::new();
    for field in [
        "email-outbox-v1".to_owned(),
        row.get::<String, _>("id"),
        row.get::<String, _>("challenge_id"),
        "challenge".to_owned(),
        "http-test".to_owned(),
    ] {
        aad.extend_from_slice(&(field.len() as u64).to_be_bytes());
        aad.extend_from_slice(field.as_bytes());
    }
    let encrypted: Vec<u8> = row.get("payload");
    assert_eq!(encrypted[0], 1);
    let plaintext = Zeroizing::new(
        Aes256Gcm::new_from_slice(&[79; 32])
            .unwrap()
            .decrypt(
                Nonce::from_slice(&encrypted[1..13]),
                Payload {
                    msg: &encrypted[13..],
                    aad: &aad,
                },
            )
            .expect("actual encrypted HTTP outbox"),
    );
    let mut bytes = plaintext.as_slice();
    let mut fields = Vec::new();
    for _ in 0..4 {
        let length = u16::from_be_bytes([bytes[0], bytes[1]]) as usize;
        fields.push(Zeroizing::new(
            String::from_utf8(bytes[2..2 + length].to_vec()).unwrap(),
        ));
        bytes = &bytes[2 + length..];
    }
    assert!(bytes.is_empty());
    assert!(fields[0].as_str() == email, "recipient remains bound");
    fields.remove(2)
}

async fn register_http(f: &Fixture, app: &Router, name: &str, email: &str) -> i64 {
    issue_window(f).await;
    let sent = call(
        app,
        VERIFY,
        json!({"email":email,"purpose":"register"}),
        None,
    )
    .await;
    assert_eq!(sent.0, StatusCode::ACCEPTED);
    let code = current_code(f, email, "register").await;
    let completed = call(
        app,
        REGISTER,
        json!({"username":name,"password":PASSWORD,"email":email,
               "verification_code":code.as_str(),"accepted_legal":true}),
        None,
    )
    .await;
    assert_eq!(completed.0, StatusCode::OK, "real registration route");
    assert_eq!(
        completed.1,
        json!({"success":true,"message":"","data":{"registered":true,"login_required":true}})
    );
    sqlx::query_scalar("SELECT user_id FROM core_identity.email_addresses WHERE email_key=$1")
        .bind(email)
        .fetch_one(&f.owner)
        .await
        .unwrap()
}

#[tokio::test]
#[ignore = "requires private HTTP integration runner"]
async fn web_register_bind_and_reset_use_real_transactions_and_revoke_credentials() {
    let (f, app) = setup().await;
    let old_email = "http-old@example.test";
    let new_email = "http-new@example.test";
    let user = register_http(&f, &app, "http-flow", old_email).await;
    assert_eq!(
        count(
            &f,
            "SELECT count(*) FROM core_identity.email_delivery_outbox WHERE kind='security_notice'"
        )
        .await,
        0
    );
    assert_eq!(
        count(
            &f,
            "SELECT count(*) FROM core_identity.email_challenges WHERE state='consumed'"
        )
        .await,
        1
    );
    let bundle = login(&f, "http-flow", PASSWORD).await;
    let second = login(&f, "http-flow", PASSWORD).await;
    let auth = Some((access(&bundle), sid(&bundle)));
    // Enable key creation only in this synthetic fixture, never by registration.
    sqlx::query("INSERT INTO core_identity.developer_access(user_id,granted,source_reference) VALUES ($1,TRUE,'email-http-test')")
        .bind(user).execute(&f.owner).await.unwrap();
    let key = f
        .store
        .issue_key(
            access(&bundle),
            Account {
                kind: AccountKind::Personal,
                id: user,
            },
            FundingPolicy::new_personal_key(),
            3600,
        )
        .await
        .unwrap();
    issue_window(&f).await;
    assert_eq!(
        call(
            &app,
            VERIFY,
            json!({"email":new_email,"purpose":"bind"}),
            auth
        )
        .await
        .0,
        StatusCode::ACCEPTED
    );
    let bind_code = current_code(&f, new_email, "bind").await;
    let binding = json!({"email":new_email,"code":bind_code.as_str()});
    assert_eq!(
        call(&app, BIND, binding.clone(), None).await.0,
        StatusCode::UNAUTHORIZED
    );
    assert_eq!(
        call(
            &app,
            BIND,
            binding.clone(),
            Some((access(&bundle), sid(&second)))
        )
        .await
        .0,
        StatusCode::UNAUTHORIZED
    );
    assert_eq!(
        call(&app, BIND, binding.clone(), auth).await.0,
        StatusCode::OK
    );
    assert_eq!(
        call(&app, BIND, binding, auth).await.0,
        StatusCode::BAD_REQUEST
    );
    let bound = state(&f, user).await;
    assert!(bound.3 == new_email);
    assert_eq!(bound.4, 2);
    assert_eq!(count(&f, "SELECT count(*) FROM core_identity.email_delivery_outbox WHERE kind='security_notice' AND recipient_slot='current'").await, 1);
    assert_eq!(count(&f, "SELECT count(*) FROM core_identity.email_delivery_outbox WHERE kind='security_notice' AND recipient_slot='previous'").await, 1);
    issue_window(&f).await;
    let before = count(&f, "SELECT count(*) FROM core_identity.email_challenges").await;
    assert_eq!(
        call(&app, RESET_REQUEST, json!({"email":old_email}), None)
            .await
            .0,
        StatusCode::ACCEPTED
    );
    assert_eq!(
        count(&f, "SELECT count(*) FROM core_identity.email_challenges").await,
        before
    );
    issue_window(&f).await;
    assert_eq!(
        call(&app, RESET_REQUEST, json!({"email":new_email}), None)
            .await
            .0,
        StatusCode::ACCEPTED
    );
    let token = current_code(&f, new_email, "password_reset").await;
    let wrong_purpose = call(
        &app,
        RESET,
        json!({"email":new_email,"token":bind_code.as_str(),"new_password":NEW_PASSWORD}),
        None,
    )
    .await;
    let unknown = call(&app, RESET, json!({"email":"http-missing@example.test","token":token.as_str(),"new_password":NEW_PASSWORD}), None).await;
    assert_eq!(wrong_purpose, unknown, "no existence/purpose detail");
    let reset = json!({"email":new_email,"token":token.as_str(),"new_password":NEW_PASSWORD});
    let (a, b) = tokio::join!(
        call(&app, RESET, reset.clone(), None),
        call(&app, RESET, reset, None)
    );
    assert!(
        (a.0 == StatusCode::OK && b.0 == StatusCode::BAD_REQUEST)
            || (b.0 == StatusCode::OK && a.0 == StatusCode::BAD_REQUEST),
        "only one concurrent reset commits"
    );
    let success = if a.0 == StatusCode::OK { a.1 } else { b.1 };
    assert_eq!(
        success,
        json!({"success":true,"message":"","data":{"password_changed":true,"login_required":true}})
    );
    assert_eq!(state(&f, user).await.1, bound.1 + 1);
    assert_eq!(
        count(
            &f,
            "SELECT count(*) FROM core_identity.email_delivery_outbox WHERE kind='security_notice'"
        )
        .await,
        3
    );
    assert!(f.store.authorize(access(&bundle)).await.is_err());
    assert!(f.store.authorize(access(&second)).await.is_err());
    assert!(f.store.authorize(&key.secret).await.is_err());
    assert!(
        f.browser
            .refresh(&bundle.refresh_token, Some(sid(&bundle)))
            .await
            .is_err()
    );
    assert!(
        f.browser
            .login("http-flow", PASSWORD, &BrowserSessionMetadata::default())
            .await
            .is_err()
    );
    assert!(
        f.browser
            .login(
                "http-flow",
                NEW_PASSWORD,
                &BrowserSessionMetadata::default()
            )
            .await
            .is_ok()
    );
}

#[tokio::test]
#[ignore = "requires private HTTP integration runner"]
async fn send_envelopes_hide_eligibility_budget_and_enqueue_failure() {
    let (f, app) = setup().await;
    let email = "http-known@example.test";
    register_http(&f, &app, "http-known", email).await;
    f.browser
        .register(&BrowserRegistration {
            username: "http-legacy",
            password: PASSWORD,
            email: "",
        })
        .await
        .unwrap();
    sqlx::query("UPDATE core_identity.user_profiles SET email='http-legacy@example.test' WHERE username='http-legacy'")
        .execute(&f.owner).await.unwrap();
    issue_window(&f).await;
    let accepted = call(&app, RESET_REQUEST, json!({"email":email}), None).await;
    for target in [
        email,
        "http-absent@example.test",
        "http-legacy@example.test",
    ] {
        assert_eq!(
            call(&app, RESET_REQUEST, json!({"email":target}), None).await,
            accepted
        );
    }
    assert_eq!(accepted.0, StatusCode::ACCEPTED);
    assert_eq!(
        accepted.1,
        json!({"success":true,"message":"","data":{"status":"accepted_if_eligible"}})
    );
    // A database failure only reached by eligible targets cannot become an oracle.
    let before = count(&f, "SELECT count(*) FROM core_identity.email_challenges").await;
    sqlx::raw_sql(
        "CREATE FUNCTION core_identity.http_fail_challenge() RETURNS trigger LANGUAGE plpgsql AS $$
         BEGIN RAISE EXCEPTION 'injected enqueue failure'; END $$;
         CREATE TRIGGER http_fail_challenge BEFORE INSERT ON core_identity.email_delivery_outbox
         FOR EACH ROW EXECUTE FUNCTION core_identity.http_fail_challenge();",
    )
    .execute(&f.owner)
    .await
    .unwrap();
    issue_window(&f).await;
    assert_eq!(
        call(
            &app,
            VERIFY,
            json!({"email":"http-new-failure@example.test","purpose":"register"}),
            None
        )
        .await,
        accepted
    );
    assert_eq!(
        count(&f, "SELECT count(*) FROM core_identity.email_challenges").await,
        before
    );
    for target in [email, "http-free@example.test"] {
        assert_eq!(
            call(
                &app,
                VERIFY,
                json!({"email":target,"purpose":"register"}),
                None
            )
            .await,
            accepted
        );
    }
    let disabled = f.store.clone().with_registration_enabled(false);
    let outbox = Outbox::new(
        f.runtime.clone(),
        MailKeys::new("http-test", vec![("http-test".into(), vec![79; 32])]).unwrap(),
    );
    let blocked = identity_http::router_with_email(
        disabled.clone(),
        EmailHttp::new(disabled, f.challenges.clone(), outbox),
    );
    assert_eq!(
        call(
            &blocked,
            "/core/v1/identity/email/verification/request",
            json!({"email":"http-disabled@example.test","purpose":"register"}),
            None
        )
        .await,
        accepted
    );
    assert_eq!(
        count(&f, "SELECT count(*) FROM core_identity.email_challenges").await,
        before
    );
}

#[tokio::test]
#[ignore = "requires private HTTP integration runner"]
async fn notice_failure_rolls_back_binding_and_password_reset_through_http() {
    let (f, app) = setup().await;
    let old_email = "rollback-old@example.test";
    let new_email = "rollback-new@example.test";
    let user = register_http(&f, &app, "http-rollback", old_email).await;
    let bundle = login(&f, "http-rollback", PASSWORD).await;
    let auth = Some((access(&bundle), sid(&bundle)));
    let original = state(&f, user).await;
    issue_window(&f).await;
    assert_eq!(
        call(
            &app,
            VERIFY,
            json!({"email":new_email,"purpose":"bind"}),
            auth
        )
        .await
        .0,
        StatusCode::ACCEPTED
    );
    let code = current_code(&f, new_email, "bind").await;
    sqlx::raw_sql(
        "CREATE FUNCTION core_identity.http_fail_notice() RETURNS trigger LANGUAGE plpgsql AS $$
         BEGIN IF NEW.kind='security_notice' AND NEW.recipient_slot='previous' THEN
             RAISE EXCEPTION 'injected second notice failure'; END IF; RETURN NEW; END $$;
         CREATE TRIGGER http_fail_notice BEFORE INSERT ON core_identity.email_delivery_outbox
         FOR EACH ROW EXECUTE FUNCTION core_identity.http_fail_notice();",
    )
    .execute(&f.owner)
    .await
    .unwrap();
    let binding = json!({"email":new_email,"code":code.as_str()});
    assert_eq!(
        call(&app, BIND, binding.clone(), auth).await.0,
        StatusCode::SERVICE_UNAVAILABLE
    );
    assert!(
        state(&f, user).await == original,
        "binding and email generation roll back"
    );
    assert_eq!(count(&f,"SELECT count(*) FROM core_identity.email_challenges WHERE state='active' AND purpose='bind'").await,1);
    assert_eq!(
        count(
            &f,
            "SELECT count(*) FROM core_identity.email_delivery_outbox WHERE kind='security_notice'"
        )
        .await,
        0
    );
    sqlx::query("DROP TRIGGER http_fail_notice ON core_identity.email_delivery_outbox")
        .execute(&f.owner)
        .await
        .unwrap();
    assert_eq!(call(&app, BIND, binding, auth).await.0, StatusCode::OK);
    issue_window(&f).await;
    assert_eq!(
        call(&app, RESET_REQUEST, json!({"email":new_email}), None)
            .await
            .0,
        StatusCode::ACCEPTED
    );
    let token = current_code(&f, new_email, "password_reset").await;
    let before = state(&f, user).await;
    sqlx::raw_sql(
        "CREATE OR REPLACE FUNCTION core_identity.http_fail_notice() RETURNS trigger LANGUAGE plpgsql AS $$
         BEGIN IF NEW.kind='security_notice' THEN RAISE EXCEPTION 'injected reset notice failure';
         END IF; RETURN NEW; END $$;
         CREATE TRIGGER http_fail_notice BEFORE INSERT ON core_identity.email_delivery_outbox
         FOR EACH ROW EXECUTE FUNCTION core_identity.http_fail_notice();"
    ).execute(&f.owner).await.unwrap();
    let reset = json!({"email":new_email,"token":token.as_str(),"new_password":NEW_PASSWORD});
    assert_eq!(
        call(&app, RESET, reset.clone(), None).await.0,
        StatusCode::SERVICE_UNAVAILABLE
    );
    assert!(
        state(&f, user).await == before,
        "password, generations and legacy policy roll back"
    );
    assert!(f.store.authorize(access(&bundle)).await.is_ok());
    assert_eq!(count(&f,"SELECT count(*) FROM core_identity.email_challenges WHERE state='active' AND purpose='password_reset'").await,1);
    assert_eq!(
        count(
            &f,
            "SELECT count(*) FROM core_identity.email_delivery_outbox WHERE kind='security_notice'"
        )
        .await,
        2
    );
    sqlx::query("DROP TRIGGER http_fail_notice ON core_identity.email_delivery_outbox")
        .execute(&f.owner)
        .await
        .unwrap();
    assert_eq!(call(&app, RESET, reset, None).await.0, StatusCode::OK);
}

#[tokio::test]
#[ignore = "requires private HTTP integration runner"]
async fn canonical_contract_rejects_authority_injection_and_preserves_web_gates() {
    let (f, app) = setup().await;
    let core_verify = "/core/v1/identity/email/verification/request";
    for body in [
        json!({"email":"canonical@example.test","purpose":"password_reset"}),
        json!({"email":"canonical@example.test","purpose":"register","user_id":1}),
        json!({"email":"canonical@example.test","purpose":"bind","sid":"caller-supplied"}),
        json!({"email":"canonical@example.test","purpose":"register","challenge_id":"detached-proof"}),
    ] {
        assert_eq!(
            call(&app, core_verify, body, None).await.0,
            StatusCode::BAD_REQUEST
        );
    }
    assert_eq!(
        count(&f, "SELECT count(*) FROM core_identity.email_challenges").await,
        0
    );
    let email = "canonical@example.test";
    assert_eq!(
        call(
            &app,
            core_verify,
            json!({"email":email,"purpose":"register"}),
            None
        )
        .await
        .0,
        StatusCode::ACCEPTED
    );
    let code = current_code(&f, email, "register").await;
    let registration = json!({"username":"http-canonical","password":PASSWORD,"email":email,"verification_code":code.as_str(),"accepted_legal":true});
    let mut no_legal = registration.clone();
    no_legal["accepted_legal"] = json!(false);
    assert_eq!(
        call(&app, "/core/v1/identity/email/register", no_legal, None)
            .await
            .0,
        StatusCode::BAD_REQUEST
    );
    assert_eq!(
        call(
            &app,
            "/core/v1/identity/email/register",
            registration.clone(),
            None
        )
        .await
        .0,
        StatusCode::CREATED
    );
    assert_eq!(
        call(&app, "/core/v1/identity/email/register", registration, None)
            .await
            .0,
        StatusCode::BAD_REQUEST
    );
    let bundle = login(&f, "http-canonical", PASSWORD).await;
    let auth = Some((access(&bundle), sid(&bundle)));
    issue_window(&f).await;
    assert_eq!(
        call(
            &app,
            core_verify,
            json!({"email":"canonical-bound@example.test","purpose":"bind"}),
            auth
        )
        .await
        .0,
        StatusCode::ACCEPTED
    );
    let binding_code = current_code(&f, "canonical-bound@example.test", "bind").await;
    assert_eq!(
        call(
            &app,
            "/core/v1/identity/email/bind",
            json!({"email":"canonical-bound@example.test","code":binding_code.as_str()}),
            auth
        )
        .await
        .0,
        StatusCode::OK
    );
    issue_window(&f).await;
    assert_eq!(
        call(
            &app,
            "/core/v1/identity/email/password-reset/request",
            json!({"email":"canonical-bound@example.test"}),
            None
        )
        .await
        .0,
        StatusCode::ACCEPTED
    );
    let token = current_code(&f, "canonical-bound@example.test", "password_reset").await;
    assert_eq!(call(&app,"/core/v1/identity/email/password-reset/consume",json!({"email":"canonical-bound@example.test","token":token.as_str(),"new_password":NEW_PASSWORD}),None).await.0,StatusCode::OK);
    assert_eq!(call(&app,VERIFY,json!({"email":"gate@example.test","purpose":"register","turnstile":"unsupported-synthetic"}),None).await.0,StatusCode::SERVICE_UNAVAILABLE);
    let plain = legacy_auth::router(
        f.store.clone(),
        LegacyAuthConfig::new(WEB, &[43; 32]).unwrap(),
    );
    let disabled = plain
        .oneshot(
            HttpRequest::builder()
                .method("POST")
                .uri(RESET_REQUEST)
                .header("origin", WEB)
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(
        disabled.status(),
        StatusCode::NOT_FOUND,
        "default constructor enables no email routes"
    );
    let forbidden = app
        .clone()
        .oneshot(
            HttpRequest::builder()
                .method("POST")
                .uri(VERIFY)
                .header("origin", "https://foreign.example.test")
                .header("content-type", "application/json")
                .body(Body::from(
                    json!({"email":"gate@example.test","purpose":"register"}).to_string(),
                ))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(forbidden.status(), StatusCode::FORBIDDEN);
    let get = app
        .clone()
        .oneshot(
            HttpRequest::builder()
                .uri(RESET)
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(
        get.status(),
        StatusCode::METHOD_NOT_ALLOWED,
        "GET cannot consume reset tokens"
    );
    let mut duplicate_headers = HeaderMap::new();
    duplicate_headers.insert("origin", WEB.parse().unwrap());
    duplicate_headers.insert("content-type", "application/json".parse().unwrap());
    duplicate_headers.append(
        "authorization",
        format!("Bearer {}", access(&bundle)).parse().unwrap(),
    );
    duplicate_headers.append("authorization", "Bearer synthetic-other".parse().unwrap());
    duplicate_headers.insert("x-auth-session", sid(&bundle).parse().unwrap());
    let mut request = HttpRequest::builder()
        .method("POST")
        .uri(BIND)
        .body(Body::from(
            json!({"email":"canonical-extra@example.test","code":"000000"}).to_string(),
        ))
        .unwrap();
    *request.headers_mut() = duplicate_headers;
    assert_eq!(
        app.oneshot(request).await.unwrap().status(),
        StatusCode::UNAUTHORIZED
    );
}

#[tokio::test]
#[ignore = "requires private HTTP integration runner"]
async fn binding_rejects_other_users_keys_revocation_and_stale_primary_authentication() {
    let (f, app) = setup().await;
    let first_user = register_http(&f, &app, "http-first", "http-first@example.test").await;
    register_http(&f, &app, "http-second", "http-second@example.test").await;
    let first = login(&f, "http-first", PASSWORD).await;
    let second = login(&f, "http-second", PASSWORD).await;
    let target = "http-private-target@example.test";
    let first_auth = Some((access(&first), sid(&first)));
    let second_auth = Some((access(&second), sid(&second)));
    let input = json!({"email":target,"purpose":"bind"});
    let before = count(&f, "SELECT count(*) FROM core_identity.email_challenges").await;
    assert_eq!(
        call(&app, VERIFY, input.clone(), None).await.0,
        StatusCode::UNAUTHORIZED
    );
    assert_eq!(
        call(
            &app,
            VERIFY,
            input.clone(),
            Some((access(&first), sid(&second)))
        )
        .await
        .0,
        StatusCode::UNAUTHORIZED
    );
    sqlx::query("INSERT INTO core_identity.developer_access(user_id,granted,source_reference) VALUES ($1,TRUE,'email-http-test')")
        .bind(first_user).execute(&f.owner).await.unwrap();
    let key = f
        .store
        .issue_key(
            access(&first),
            Account {
                kind: AccountKind::Personal,
                id: first_user,
            },
            FundingPolicy::new_personal_key(),
            3600,
        )
        .await
        .unwrap();
    assert_eq!(
        call(
            &app,
            VERIFY,
            input.clone(),
            Some((&key.secret, sid(&first)))
        )
        .await
        .0,
        StatusCode::FORBIDDEN
    );
    assert_eq!(
        count(&f, "SELECT count(*) FROM core_identity.email_challenges").await,
        before
    );
    issue_window(&f).await;
    assert_eq!(
        call(&app, VERIFY, input.clone(), first_auth).await.0,
        StatusCode::ACCEPTED
    );
    let code = current_code(&f, target, "bind").await;
    let consume = json!({"email":target,"code":code.as_str()});
    assert_eq!(
        call(&app, BIND, consume.clone(), second_auth).await.0,
        StatusCode::BAD_REQUEST
    );
    // Synthetic owner-only aging exercises the persisted recency check without sleeps.
    sqlx::raw_sql(
        "ALTER TABLE core_identity.session_authentication DISABLE TRIGGER session_authentication_immutable;
         UPDATE core_identity.session_authentication SET authenticated_at=clock_timestamp()-interval '6 minutes';
         ALTER TABLE core_identity.session_authentication ENABLE TRIGGER session_authentication_immutable;"
    ).execute(&f.owner).await.unwrap();
    assert_eq!(
        call(&app, BIND, consume.clone(), first_auth).await.0,
        StatusCode::BAD_REQUEST
    );
    issue_window(&f).await;
    assert_eq!(
        call(
            &app,
            VERIFY,
            json!({"email":"http-stale@example.test","purpose":"bind"}),
            first_auth
        )
        .await
        .0,
        StatusCode::ACCEPTED
    );
    assert_eq!(
        count(&f, "SELECT count(*) FROM core_identity.email_challenges").await,
        before + 1
    );
    f.browser
        .logout(
            Some(&first.refresh_token),
            Some(access(&first)),
            Some(sid(&first)),
        )
        .await
        .unwrap();
    assert_eq!(
        call(&app, BIND, consume, first_auth).await.0,
        StatusCode::UNAUTHORIZED
    );
    assert_eq!(count(&f,"SELECT count(*) FROM core_identity.email_challenges WHERE purpose='bind' AND state='consumed'").await,0);
}

#[tokio::test]
#[ignore = "requires private HTTP integration runner"]
async fn password_reset_does_not_turn_email_proof_into_mfa_or_a_login() {
    let (f, app) = setup().await;
    let email = "http-mfa@example.test";
    let user = register_http(&f, &app, "http-mfa", email).await;
    let primary = login(&f, "http-mfa", PASSWORD).await;
    let enrollment = f.browser.mfa_setup(access(&primary)).await.unwrap();
    let epoch: i64 =
        sqlx::query_scalar("SELECT floor(extract(epoch FROM clock_timestamp()))::bigint")
            .fetch_one(&f.owner)
            .await
            .unwrap();
    let secret = Secret::try_from_base32(&enrollment.secret).unwrap();
    let totp = Builder::new()
        .with_algorithm(Algorithm::SHA1)
        .with_digits(6)
        .with_step_duration(30)
        .with_skew(1)
        .with_secret(secret.as_bytes())
        .build()
        .unwrap()
        .generate(u64::try_from(epoch).unwrap())
        .to_string();
    let assured = f.browser.mfa_enable(access(&primary), &totp).await.unwrap();
    let before: (String, i64, bool) = sqlx::query_as(
        "SELECT u.platform_role,f.revision,f.enabled FROM core_identity.users u
         JOIN core_identity.mfa_factors f ON f.user_id=u.id WHERE u.id=$1",
    )
    .bind(user)
    .fetch_one(&f.owner)
    .await
    .unwrap();
    issue_window(&f).await;
    assert_eq!(
        call(
            &app,
            VERIFY,
            json!({"email":"http-mfa-new@example.test","purpose":"bind"}),
            Some((access(&assured), sid(&assured))),
        )
        .await
        .0,
        StatusCode::ACCEPTED
    );
    let binding_code = current_code(&f, "http-mfa-new@example.test", "bind").await;
    issue_window(&f).await;
    assert_eq!(
        call(&app, RESET_REQUEST, json!({"email":email}), None)
            .await
            .0,
        StatusCode::ACCEPTED
    );
    let token = current_code(&f, email, "password_reset").await;
    assert_eq!(
        call(
            &app,
            RESET,
            json!({"email":email,"token":token.as_str(),"new_password":NEW_PASSWORD}),
            None,
        )
        .await
        .0,
        StatusCode::OK
    );
    let after: (String, i64, bool) = sqlx::query_as(
        "SELECT u.platform_role,f.revision,f.enabled FROM core_identity.users u
         JOIN core_identity.mfa_factors f ON f.user_id=u.id WHERE u.id=$1",
    )
    .bind(user)
    .fetch_one(&f.owner)
    .await
    .unwrap();
    assert!(
        before == after && after.2,
        "role and MFA authority remain unchanged"
    );
    assert!(
        state(&f, user).await.5,
        "password reset preserves required MFA"
    );
    assert!(matches!(
        f.browser
            .login_with_mfa(
                "http-mfa",
                NEW_PASSWORD,
                &BrowserSessionMetadata::default(),
                BINDING,
            )
            .await
            .unwrap(),
        BrowserLoginOutcome::Challenge(_)
    ));
    assert_eq!(
        call(
            &app,
            BIND,
            json!({"email":"http-mfa-new@example.test","code":binding_code.as_str()}),
            Some((access(&assured), sid(&assured))),
        )
        .await
        .0,
        StatusCode::UNAUTHORIZED
    );
}

#[path = "reset_link_cases.rs"]
mod reset_links;

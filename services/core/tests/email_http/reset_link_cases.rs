//! Link production and the real reset consumer share the existing encrypted proof.
use super::*;
use lmm_core::email_delivery::ResetSite;
use reqwest::Url;

#[test]
fn renderer_uses_the_configured_web_origin_and_encodes_the_existing_proof() {
    let vectors: Value = serde_json::from_str(include_str!("reset-vectors.json")).unwrap();
    for entry in vectors.as_array().unwrap() {
        let mut config =
            LegacyAuthConfig::new(entry["origin"].as_str().unwrap(), &[43; 32]).unwrap();
        if entry["local_http"] == true {
            config = config.with_insecure_local_http().unwrap();
        }
        let site = config.email_reset_site().unwrap();
        let rendered = site
            .render(
                &address(entry["email"].as_str().unwrap()),
                entry["token"].as_str().unwrap(),
            )
            .unwrap();
        let link = rendered
            .lines()
            .find(|line| line.starts_with("http"))
            .unwrap();
        assert!(
            link == entry["link"].as_str().unwrap(),
            "encoded URL contract vector"
        );
        let url = Url::parse(link).unwrap();
        assert!(url.query().is_none(), "proof must not be an HTTP query");
        assert_eq!(url.path(), "/user/reset");
        assert!(rendered.contains("original request") && rendered.contains("can be used once"));
    }
}

#[test]
fn reset_sites_and_proof_shape_fail_closed_without_request_header_fallback() {
    for invalid in [
        "http://foreign.example.test",
        "//foreign.example.test",
        "https:foreign.example.test",
        "https:/foreign.example.test",
        "javascript:alert(1)",
        "https://trusted.example.test@foreign.example.test",
        "https://user:pass@console.example.test",
        "https://console.example.test/base",
        "https://console.example.test?next=foreign",
        "https://console.example.test#foreign",
        "https://console.example.test:0",
        "https://console.example.test\r\nX-Injected: value",
        " https://console.example.test",
        "https:\\foreign.example.test",
    ] {
        assert!(
            ResetSite::https(invalid).is_err(),
            "invalid configured origin rejected"
        );
    }
    assert!(ResetSite::local_http("http://foreign.example.test").is_err());
    let insecure = LegacyAuthConfig::new("http://foreign.example.test", &[43; 32]).unwrap();
    assert!(
        insecure.email_reset_site().is_err(),
        "secure Web profile cannot enable HTTP links"
    );
    let site = ResetSite::https(WEB).unwrap();
    for invalid in [
        "",
        "abcdef",
        "ABCDEF0123456789ABCDEF0123456789",
        "0123456789abcdef0123456789abcde&x",
    ] {
        assert!(
            site.render(&address("scope@example.test"), invalid)
                .is_err()
        );
    }
}

fn consume_fields(message: &str) -> (Zeroizing<String>, Zeroizing<String>) {
    let link = message
        .lines()
        .find(|line| line.starts_with("https://"))
        .unwrap();
    let url = Url::parse(link).unwrap();
    assert!(
        url.origin().ascii_serialization() == WEB,
        "configured Web site only"
    );
    assert!(url.query().is_none());
    let mut decoder = Url::parse("https://decode.example.test").unwrap();
    decoder.set_query(url.fragment());
    let fields = decoder.query_pairs().collect::<Vec<_>>();
    assert_eq!(fields.len(), 2);
    let email = fields
        .iter()
        .find(|(key, _)| key == "email")
        .unwrap()
        .1
        .to_string();
    let token = fields
        .iter()
        .find(|(key, _)| key == "token")
        .unwrap()
        .1
        .to_string();
    (Zeroizing::new(email), Zeroizing::new(token))
}

#[tokio::test]
#[ignore = "requires private HTTP integration runner"]
async fn encrypted_issuance_link_and_real_reset_keep_original_deadline_and_single_consumption() {
    let (f, app) = setup().await;
    let email = "reset+tag/o'hare?x=1&y@example.test";
    let user = register_http(&f, &app, "http-reset-link", email).await;
    let old_session = login(&f, "http-reset-link", PASSWORD).await;
    issue_window(&f).await;
    // Poisoned request headers are deliberately absent from trusted site configuration.
    let request = HttpRequest::builder()
        .method("POST")
        .uri(RESET_REQUEST)
        .header("origin", WEB)
        .header("host", "foreign.example.test")
        .header("x-forwarded-host", "foreign.example.test")
        .header("forwarded", "host=foreign.example.test;proto=http")
        .header("content-type", "application/json")
        .body(Body::from(json!({"email":email}).to_string()))
        .unwrap();
    let reply = app.clone().oneshot(request).await.unwrap();
    assert_eq!(reply.status(), StatusCode::ACCEPTED);
    let token = current_code(&f, email, "password_reset").await;
    let expiry: f64 = sqlx::query_scalar(
        "SELECT extract(epoch FROM expires_at)::float8 FROM core_identity.email_challenges
         WHERE email_key=$1 AND purpose='password_reset' AND state='active'",
    )
    .bind(email)
    .fetch_one(&f.owner)
    .await
    .unwrap();
    issue_window(&f).await;
    assert_eq!(
        call(&app, RESET_REQUEST, json!({"email":email}), None)
            .await
            .0,
        StatusCode::ACCEPTED
    );
    assert!(
        current_code(&f, email, "password_reset").await.as_str() == token.as_str(),
        "pending request preserves proof"
    );
    let after: f64 = sqlx::query_scalar(
        "SELECT extract(epoch FROM expires_at)::float8 FROM core_identity.email_challenges
         WHERE email_key=$1 AND purpose='password_reset' AND state='active'",
    )
    .bind(email)
    .fetch_one(&f.owner)
    .await
    .unwrap();
    assert!(
        expiry == after,
        "render/resend cannot extend the original deadline"
    );
    let site = LegacyAuthConfig::new(WEB, &[43; 32])
        .unwrap()
        .email_reset_site()
        .unwrap();
    let message = site.render(&address(email), &token).unwrap();
    let (parsed_email, parsed_token) = consume_fields(&message);
    assert!(parsed_email.as_str() == email && parsed_token.as_str() == token.as_str());
    let before = state(&f, user).await;
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
    assert_eq!(get.status(), StatusCode::METHOD_NOT_ALLOWED);
    assert!(
        state(&f, user).await == before,
        "opening a link never chooses a password"
    );
    let reset = json!({"email":parsed_email.as_str(),"token":parsed_token.as_str(),"new_password":NEW_PASSWORD});
    assert_eq!(
        call(&app, RESET, reset.clone(), None).await.0,
        StatusCode::OK
    );
    assert_eq!(
        call(&app, RESET, reset, None).await.0,
        StatusCode::BAD_REQUEST
    );
    assert!(f.store.authorize(access(&old_session)).await.is_err());
    assert!(
        f.browser
            .login(
                "http-reset-link",
                NEW_PASSWORD,
                &BrowserSessionMetadata::default()
            )
            .await
            .is_ok()
    );
    issue_window(&f).await;
    assert_eq!(
        call(&app, RESET_REQUEST, json!({"email":email}), None)
            .await
            .0,
        StatusCode::ACCEPTED
    );
    let expired_token = current_code(&f, email, "password_reset").await;
    // Only the private install owner may age this synthetic immutable fixture.
    sqlx::raw_sql(
        "ALTER TABLE core_identity.email_challenges DISABLE TRIGGER email_challenge_guard;
         UPDATE core_identity.email_challenges SET issued_at=issued_at-interval '11 minutes',
             expires_at=expires_at-interval '11 minutes' WHERE state='active' AND purpose='password_reset';
         ALTER TABLE core_identity.email_challenges ENABLE TRIGGER email_challenge_guard;"
    ).execute(&f.owner).await.unwrap();
    let expired_message = site.render(&address(email), &expired_token).unwrap();
    let (parsed_email, parsed_token) = consume_fields(&expired_message);
    let unchanged = state(&f, user).await;
    assert_eq!(call(&app,RESET,json!({"email":parsed_email.as_str(),"token":parsed_token.as_str(),"new_password":PASSWORD}),None).await.0,StatusCode::BAD_REQUEST);
    assert!(
        state(&f, user).await == unchanged,
        "rendering a URL cannot revive an expired proof"
    );
}

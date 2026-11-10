use super::*;
use jsonwebtoken::{EncodingKey, Header, encode};
use serde_json::json;

const SECRET: &str = "synthetic-confidential-client-secret";
fn fixture() -> (ProviderConfiguration, Value, Vec<u8>) {
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_secs();
    let configuration = ProviderConfiguration {
        issuer: "https://issuer.example.test".into(),
        client_id: "native-client".into(),
        id_token_algorithms: vec!["HS256".into()],
        ..Default::default()
    };
    let claims = json!({"iss":configuration.issuer,"sub":"subject","aud":"native-client","iat":now,"exp":now+300,"nonce":"browser-bound-nonce"});
    (
        configuration,
        claims,
        Sha256::digest(b"browser-bound-nonce").to_vec(),
    )
}
fn signed(claims: &Value) -> String {
    encode(
        &Header::new(Algorithm::HS256),
        claims,
        &EncodingKey::from_secret(SECRET.as_bytes()),
    )
    .unwrap()
}

#[test]
fn oidc_verifies_signature_issuer_audience_nonce_and_time() {
    let (configuration, claims, nonce) = fixture();
    assert!(verify_id_token(&signed(&claims), &json!({}), &configuration, &nonce, SECRET).is_ok());
    for (field, value) in [
        ("iss", json!("https://other.example.test")),
        ("aud", json!("other-client")),
        ("nonce", json!("another-nonce")),
        ("exp", json!(1)),
        ("iat", json!(4102444800_u64)),
    ] {
        let mut changed = claims.clone();
        changed[field] = value;
        assert!(
            verify_id_token(
                &signed(&changed),
                &json!({}),
                &configuration,
                &nonce,
                SECRET
            )
            .is_err(),
            "{field}"
        );
    }
    assert!(
        verify_id_token(
            &signed(&claims),
            &json!({}),
            &configuration,
            &nonce,
            "wrong-secret"
        )
        .is_err()
    );
    let mut missing = claims;
    missing.as_object_mut().unwrap().remove("nonce");
    assert!(
        verify_id_token(
            &signed(&missing),
            &json!({}),
            &configuration,
            &nonce,
            SECRET
        )
        .is_err()
    );
}

#[test]
fn oidc_rejects_unconfigured_algorithms_and_hmac_multiple_audiences() {
    let (mut configuration, mut claims, nonce) = fixture();
    configuration.id_token_algorithms = vec!["RS256".into()];
    assert!(verify_id_token(&signed(&claims), &json!({}), &configuration, &nonce, SECRET).is_err());
    configuration.id_token_algorithms = vec!["HS256".into()];
    claims["aud"] = json!(["native-client", "another-client"]);
    claims["azp"] = json!("native-client");
    assert!(verify_id_token(&signed(&claims), &json!({}), &configuration, &nonce, SECRET).is_err());
}

#[test]
fn old_custom_token_json_and_form_formats_are_supported_without_retry() {
    assert_eq!(
        token_reply(r#"{"access_token":"token","token_type":"bearer"}"#)
            .unwrap()
            .access_token,
        "token"
    );
    assert_eq!(
        token_reply("access_token=form%2Btoken&token_type=Bearer")
            .unwrap()
            .access_token,
        "form+token"
    );
    assert!(token_reply("access_token=first&access_token=second").is_err());
    assert!(token_reply(r#"{"access_token":"token","error":"denied"}"#).is_err());
    assert!(token_reply(r#"{"access_token":"token","token_type":"Bearer\r\nother"}"#).is_err());
}

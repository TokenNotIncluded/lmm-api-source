use super::*;

fn request(version: &str, action: SecurityAction) -> SecurityRequest {
    SecurityRequest {
        expected_auth_version: version.to_owned(),
        action,
        reason: "Synthetic administrator recovery".to_owned(),
        public_reason: None,
    }
}

#[test]
fn security_requests_require_fresh_generation_and_a_reason() {
    assert!(request("1", SecurityAction::Disable).validate().is_ok());
    for generation in ["", "0", "01", "-1", "+1", "1.0", "9223372036854775807"] {
        assert_eq!(
            request(generation, SecurityAction::Disable).validate(),
            Err(ProfileError::Invalid)
        );
    }
    let mut input = request("1", SecurityAction::ResetMfa);
    input.reason = " ".into();
    assert_eq!(input.validate(), Err(ProfileError::Invalid));
    assert!(
        serde_json::from_str::<SecurityRequest>(
            r#"{"expected_auth_version":"1","action":"promote","reason":"test"}"#
        )
        .is_err()
    );
    assert!(
        serde_json::from_str::<SecurityRequest>(
            r#"{"expected_auth_version":"1","action":"enable","reason":"test","role":"superadmin"}"#
        )
        .is_err()
    );
}

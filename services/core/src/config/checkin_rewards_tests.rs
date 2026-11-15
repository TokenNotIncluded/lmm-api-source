use super::*;
#[test]
fn checkin_rewards_default_to_explicit_user_verification() {
    let config: RewardFundingConfig =
        serde_json::from_str(r#"{"version":1,"max_daily_units_by_trust":[0,1,2,3,4]}"#).unwrap();
    assert!(config.require_passkey);
    let config: RewardFundingConfig = serde_json::from_str(
        r#"{"version":1,"max_daily_units_by_trust":[0,1,2,3,4],"require_passkey":false}"#,
    )
    .unwrap();
    assert!(!config.require_passkey);
    assert!(
        serde_json::from_str::<RewardFundingConfig>(
            r#"{"version":1,"max_daily_units_by_trust":[0,1,2,3,4],"require_passkey":null}"#
        )
        .is_err()
    );
}
#[test]
fn checkin_passkeys_share_native_web_origin_validation() {
    assert!(
        web_passkeys("https://example.test", None)
            .unwrap()
            .is_some()
    );
    assert!(
        web_passkeys("http://localhost:3000", None)
            .unwrap()
            .is_some()
    );
    assert!(web_passkeys("http://example.test", None).unwrap().is_none());
    assert!(web_passkeys("https://example.test", Some("elsewhere.test")).is_err());
}

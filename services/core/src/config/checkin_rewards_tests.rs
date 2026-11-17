use super::*;
#[test]
fn checkin_rewards_default_to_explicit_user_verification() {
    let config: RewardFundingConfig =
        serde_json::from_str(r#"{"version":1,"max_daily_units":4}"#).unwrap();
    assert!(config.require_passkey);
    let config: RewardFundingConfig = serde_json::from_str(
        r#"{"version":1,"max_daily_units":4,"require_passkey":false}"#,
    )
    .unwrap();
    assert!(!config.require_passkey);
    assert!(
        serde_json::from_str::<RewardFundingConfig>(
            r#"{"version":1,"max_daily_units":4,"require_passkey":null}"#
        )
        .is_err()
    );
}
#[test]
fn checkin_reward_limits_read_legacy_caps_and_write_only_the_scalar() {
    let config: RewardFundingConfig = serde_json::from_str(
        r#"{"version":1,"max_daily_units_by_trust":[100,200,300,400,4]}"#,
    )
    .unwrap();
    assert_eq!(config.max_daily_units, 4);
    assert!(config.require_passkey);
    assert_eq!(
        serde_json::to_value(&config).unwrap(),
        serde_json::json!({"version":1,"max_daily_units":4,"require_passkey":true})
    );

    for maximum in [0, i64::MAX] {
        let input = serde_json::json!({"version":1,"max_daily_units":maximum});
        let config: RewardFundingConfig = serde_json::from_value(input).unwrap();
        assert_eq!(config.max_daily_units, maximum);
    }
}

#[test]
fn checkin_reward_limits_reject_conflicting_or_invalid_amounts() {
    for document in [
        r#"{"version":1}"#,
        r#"{"version":1,"max_daily_units":null}"#,
        r#"{"version":1,"max_daily_units":-1}"#,
        r#"{"version":1,"max_daily_units":0.5}"#,
        r#"{"version":1,"max_daily_units":"4"}"#,
        r#"{"version":1,"max_daily_units":9223372036854775808}"#,
        r#"{"version":1,"max_daily_units":4,"unexpected":true}"#,
        r#"{"version":1,"max_daily_units":4,"max_daily_units":4}"#,
        r#"{"version":1,"max_daily_units_by_trust":null}"#,
        r#"{"version":1,"max_daily_units_by_trust":[0,1,2,3]}"#,
        r#"{"version":1,"max_daily_units_by_trust":[0,1,2,3,4,5]}"#,
        r#"{"version":1,"max_daily_units_by_trust":[-1,1,2,3,4]}"#,
        r#"{"version":1,"max_daily_units_by_trust":[0,1,2,3,-1]}"#,
        r#"{"version":1,"max_daily_units_by_trust":[9223372036854775808,1,2,3,4]}"#,
        r#"{"version":1,"max_daily_units_by_trust":[0,1,2,3,9223372036854775808]}"#,
        r#"{"version":1,"max_daily_units":4,"max_daily_units_by_trust":[0,1,2,3,4]}"#,
        r#"{"version":1,"max_daily_units":null,"max_daily_units_by_trust":[0,1,2,3,4]}"#,
    ] {
        assert!(
            serde_json::from_str::<RewardFundingConfig>(document).is_err(),
            "{document}"
        );
    }
}

#[test]
fn checkin_reward_limits_validate_the_production_toml_boundary() {
    for (document, expected) in [
        ("version = 1\nmax_daily_units = 0\n", 0),
        ("version = 1\nmax_daily_units = 9223372036854775807\n", i64::MAX),
        ("version = 1\nmax_daily_units_by_trust = [900, 80, 70, 60, 4]\n", 4),
    ] {
        let config: RewardFundingConfig = parse_document(document.as_bytes()).unwrap();
        assert_eq!(config.max_daily_units, expected);
        assert!(config.require_passkey);
    }
    for document in [
        "version = 1\nmax_daily_units = 4\nmax_daily_units_by_trust = [0,1,2,3,4]\n",
        "version = 1\nmax_daily_units_by_trust = [0,1,2,3,4]\nmax_daily_units = 4\n",
        "version = 1\nmax_daily_units = 4\nmax_daily_units = 4\n",
        "version = 1\nmax_daily_units = -1\n",
        "version = 1\nmax_daily_units = 9223372036854775808\n",
        "version = 1\nmax_daily_units = 0.5\n",
        "version = 1\nmax_daily_units_by_trust = [-1,1,2,3,4]\n",
        "version = 1\nmax_daily_units_by_trust = [0,1,2,3]\n",
        "version = 1\nmax_daily_units = 4\nunexpected = true\n",
    ] {
        assert!(parse_document::<RewardFundingConfig>(document.as_bytes()).is_err(), "{document}");
    }
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

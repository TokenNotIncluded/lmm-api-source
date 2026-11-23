use super::*;
#[test]
fn checkin_rewards_default_to_explicit_user_verification() {
    let config: RewardFundingConfig =
        serde_json::from_str(r#"{"version":1,"max_daily_units":4}"#).unwrap();
    assert!(config.require_passkey);
    let config: RewardFundingConfig =
        serde_json::from_str(r#"{"version":1,"max_daily_units":4,"require_passkey":false}"#)
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
    let config: RewardFundingConfig =
        serde_json::from_str(r#"{"version":1,"max_daily_units_by_trust":[100,200,300,400,4]}"#)
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
        (
            "version = 1\nmax_daily_units = 9223372036854775807\n",
            i64::MAX,
        ),
        (
            "version = 1\nmax_daily_units_by_trust = [900, 80, 70, 60, 4]\n",
            4,
        ),
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
        assert!(
            parse_document::<RewardFundingConfig>(document.as_bytes()).is_err(),
            "{document}"
        );
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

#[test]
fn checkin_turnstile_configuration_is_strict_and_preserves_legacy_limits() {
    for limit in [
        "max_daily_units = 4",
        "max_daily_units_by_trust = [0, 1, 2, 3, 4]",
    ] {
        let document = format!(
            "version = 1\n{limit}\n[turnstile]\nsite_key = 'checkin-public-key'\nsecret_key_file = '/nonexistent-test/checkin-secret'\n"
        );
        let config: RewardFundingConfig = parse_document(document.as_bytes()).unwrap();
        assert_eq!(config.max_daily_units, 4);
        assert!(config.require_passkey);
        let verifier = config.turnstile.as_ref().unwrap();
        assert_eq!(verifier.site_key, "checkin-public-key");
        verifier.validate().unwrap();
        assert_eq!(
            serde_json::to_value(&config).unwrap(),
            serde_json::json!({
                "version": 1, "max_daily_units": 4, "require_passkey": true,
                "turnstile": {
                    "site_key": "checkin-public-key",
                    "secret_key_file": "/nonexistent-test/checkin-secret"
                }
            })
        );
        for invalid in [
            document.replace("site_key = 'checkin-public-key'\n", ""),
            document.replace("secret_key_file = '/nonexistent-test/checkin-secret'\n", ""),
            format!("{document}unexpected = true\n"),
        ] {
            assert!(parse_document::<RewardFundingConfig>(invalid.as_bytes()).is_err());
        }
        for invalid in [
            document.replace("checkin-public-key", "invalid public key"),
            document.replace("/nonexistent-test/checkin-secret", "relative-secret"),
        ] {
            let config: RewardFundingConfig = parse_document(invalid.as_bytes()).unwrap();
            assert!(config.turnstile.unwrap().validate().is_err());
        }
    }
}

#[cfg(unix)]
#[test]
fn checkin_turnstile_scope_preserves_authentication_configuration() {
    use std::os::unix::fs::PermissionsExt;
    const CASE: &str = "LMM_TEST_CHECKIN_TURNSTILE_SCOPE";
    if let Ok(case) = env::var(CASE) {
        let (global, scoped) = match case.as_str() {
            "scoped-only" => (false, true),
            "separate" => (true, true),
            "legacy-fallback" => (true, false),
            "missing" => (false, false),
            _ => panic!("unknown scoped Turnstile test case"),
        };
        let directory = env::temp_dir().join(format!(
            "lmm-checkin-turnstile-{}-{}",
            std::process::id(),
            rand::random::<u64>()
        ));
        fs::create_dir(&directory).unwrap();
        fs::set_permissions(&directory, fs::Permissions::from_mode(0o700)).unwrap();
        let private = |name: &str, value: &str| {
            let path = directory.join(name);
            fs::write(&path, value).unwrap();
            fs::set_permissions(&path, fs::Permissions::from_mode(0o600)).unwrap();
            path
        };
        let database = private("database", "postgres://test:test@127.0.0.1:1/unused");
        let refresh = private("refresh", &"r".repeat(32));
        let token = private("rpc-token", &"t".repeat(64));
        let global_secret = private("global-secret", "synthetic-global-secret");
        let scoped_secret = private("checkin-secret", "synthetic-checkin-secret");
        let mut reward = "version = 1\nmax_daily_units = 4\n".to_owned();
        if scoped {
            reward.push_str(&format!(
                "[turnstile]\nsite_key = 'checkin-public-key'\nsecret_key_file = {scoped_secret:?}\n"
            ));
        }
        let reward_file = private("rewards.toml", &reward);
        let mut core = format!(
            "version = 1\n[database]\nurl_file = {database:?}\n[identity.web]\norigin = 'https://app.example.test'\nrefresh_key_file = {refresh:?}\nuser_agreement_enabled = false\nprivacy_policy_enabled = false\n[checkin_rewards]\nconfig_file = {reward_file:?}\ntoken_file = {token:?}\n"
        );
        if global {
            core.push_str(&format!(
                "[identity.turnstile]\nsite_key = 'global-public-key'\nsecret_key_file = {global_secret:?}\n"
            ));
        }
        let core_file = private("core.toml", &core);
        initialize(Some(&core_file)).unwrap();
        let config: RewardFundingConfig = parse_document(reward.as_bytes()).unwrap();
        let expected_global = global.then_some("global-public-key");
        assert_eq!(
            turnstile_config::public_site_key().as_deref(),
            expected_global
        );
        let loaded = config.load_turnstile("https://app.example.test");
        if scoped || global {
            assert_eq!(
                loaded.unwrap().site_key(),
                if scoped {
                    "checkin-public-key"
                } else {
                    "global-public-key"
                }
            );
        } else {
            assert!(loaded.is_err());
        }
        // Loading the reward verifier cannot opt other authentication paths in,
        // replace their key, or silently fall back from an invalid scoped secret.
        let authentication = turnstile_config::load().unwrap();
        assert_eq!(
            authentication.as_ref().map(|value| value.site_key()),
            expected_global
        );
        assert_eq!(
            turnstile_config::public_site_key().as_deref(),
            expected_global
        );
        if scoped {
            fs::write(&scoped_secret, "").unwrap();
            assert!(config.load_turnstile("https://app.example.test").is_err());
            assert_eq!(
                turnstile_config::load()
                    .unwrap()
                    .as_ref()
                    .map(|value| value.site_key()),
                expected_global
            );
        }
        fs::remove_dir_all(directory).unwrap();
        return;
    }
    for case in ["scoped-only", "separate", "legacy-fallback", "missing"] {
        let status = std::process::Command::new(env::current_exe().unwrap())
            .args([
                "--exact",
                "config::checkin_rewards_tests::checkin_turnstile_scope_preserves_authentication_configuration",
                "--nocapture",
            ])
            .env(CASE, case)
            .status()
            .unwrap();
        assert!(status.success(), "{case}");
    }
}

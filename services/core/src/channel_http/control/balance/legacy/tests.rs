use super::*;

fn config(channel_type: u32, rate: Option<&str>) -> LegacyBalance {
    LegacyBalance {
        channel_type,
        cny_per_usd: rate.map(str::to_owned),
    }
}
fn plan(channel_type: u32, rate: Option<&str>) -> BalanceProbe {
    resolve(
        None,
        Some(&config(channel_type, rate)),
        Protocol::OpenAiChat,
    )
    .unwrap()
    .unwrap()
}

#[test]
fn legacy_balance_exact_mapping() {
    for (kind, path, pointer, unit) in [
        (
            1,
            "/v1/dashboard/billing/subscription",
            "/hard_limit_usd",
            Some("USD"),
        ),
        (
            8,
            "/v1/dashboard/billing/subscription",
            "/hard_limit_usd",
            Some("USD"),
        ),
        (
            10,
            "/api/report/getUserOverview",
            "/data/totalPoints",
            Some("points"),
        ),
        (
            12,
            "/dashboard/billing/credit_grants",
            "/total_remaining",
            None,
        ),
        (
            13,
            "/dashboard/billing/credit_grants",
            "/total_available",
            None,
        ),
        (20, "/api/v1/credits", "/data/total_credits", Some("USD")),
        (
            25,
            "/v1/users/me/balance",
            "/data/available_balance",
            Some("CNY"),
        ),
        (40, "/v1/user/info", "/data/totalBalance", None),
        (43, "/user/balance", "/balance_infos", Some("USD")),
    ] {
        let probe = plan(kind, None);
        assert_eq!(
            (
                probe.path.as_str(),
                probe.json_pointer.as_str(),
                probe.unit_label.as_deref()
            ),
            (path, pointer, unit)
        );
        assert!(probe.cny_per_usd.is_none());
        assert!(probe.validate().is_ok());
    }
    assert_eq!(plan(1, None).openai_subscription, Some(true));
    assert_eq!(plan(8, None).openai_subscription, Some(true));
    let proxy = plan(10, None);
    assert_eq!(proxy.auth, Some(BalanceAuth::ApiKey));
    assert_eq!(proxy.require.unwrap().equals, json!(true));
    assert_eq!(
        plan(20, None).subtract_json_pointer.as_deref(),
        Some("/data/total_usage")
    );
    assert_eq!(plan(40, None).require.unwrap().equals, json!(20000));
    assert_eq!(plan(43, None).deepseek_currency_array, Some(true));
    let moonshot = plan(25, None);
    assert_eq!(
        serde_json::to_value(moonshot.require_all).unwrap(),
        json!([
            {"json_pointer":"/status","equals":true}, {"json_pointer":"/code","equals":0}
        ])
    );
}

#[test]
fn legacy_balance_unknown_types_and_protocols_never_guess() {
    for kind in (0..=64).chain([u32::MAX]) {
        let found = resolve(None, Some(&config(kind, None)), Protocol::OpenAiChat).unwrap();
        assert_eq!(
            found.is_some(),
            matches!(kind, 1 | 8 | 10 | 12 | 13 | 20 | 25 | 40 | 43),
            "type {kind}"
        );
    }
    assert!(resolve(None, None, Protocol::OpenAiChat).unwrap().is_none());
    for protocol in [Protocol::Gemini, Protocol::AnthropicMessages] {
        assert!(
            resolve(None, Some(&config(20, None)), protocol)
                .unwrap()
                .is_none()
        );
    }
}

#[test]
fn legacy_balance_manual_override_is_exact_and_not_fallback_on_error() {
    let mut manual = plan(20, None);
    manual.path = "/operator/balance".into();
    manual.unit_label = Some("operator-unit".into());
    for kind in [0, 1, 25, 43, 61] {
        assert_eq!(
            resolve(
                Some(&manual),
                Some(&config(kind, Some("invalid"))),
                Protocol::Gemini
            )
            .unwrap(),
            Some(manual.clone())
        );
    }
    // Selection never replaces even an invalid manual plan with an automatic
    // one: the caller's existing validation must reject it before I/O.
    manual.path = "https://not-allowed.invalid".into();
    let selected = resolve(Some(&manual), Some(&config(20, None)), Protocol::OpenAiChat)
        .unwrap()
        .unwrap();
    assert_eq!(selected, manual);
    assert!(selected.validate().is_err());
}

#[test]
fn legacy_balance_rates_are_explicit_and_round_trip() {
    for kind in [25, 43] {
        let probe = plan(kind, Some("7.25"));
        assert_eq!(probe.cny_per_usd.as_deref(), Some("7.25"));
        assert_eq!(probe.unit_label.as_deref(), Some("USD"));
        let input = config(kind, Some("7.25"));
        let restored: LegacyBalance =
            serde_json::from_value(serde_json::to_value(&input).unwrap()).unwrap();
        assert_eq!(restored, input);
        for rate in ["0", "-1", "NaN", "inf", " 7", "7 ", "1e999", ""] {
            assert!(resolve(None, Some(&config(kind, Some(rate))), Protocol::OpenAiChat).is_err());
        }
    }
    assert!(config(20, Some("7")).validate().is_err());
    assert!(
        serde_json::from_value::<LegacyBalance>(json!({"channel_type":43,"exchange_rate":7}))
            .is_err()
    );
    assert!(serde_json::from_value::<LegacyBalance>(json!({"channel_type":-1})).is_err());
}

#[test]
fn legacy_balance_configuration_requires_an_explicit_probe_change() {
    use super::super::BalanceConfigInput;
    let version = json!({"routing":1,"pricing":1});
    assert!(
        serde_json::from_value::<BalanceConfigInput>(json!({"expected_version":version})).is_err()
    );
    assert!(
        serde_json::from_value::<BalanceConfigInput>(
            json!({"expected_version":version,"legacy":{"channel_type":25}})
        )
        .is_err()
    );
    assert!(
        serde_json::from_value::<BalanceConfigInput>(
            json!({"expected_version":version,"probe":null})
        )
        .is_ok()
    );
    assert!(serde_json::from_value::<BalanceConfigInput>(json!({"expected_version":version,"probe":null,"legacy":{"channel_type":25,"cny_per_usd":"7"}})).is_ok());
}

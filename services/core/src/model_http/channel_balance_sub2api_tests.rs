use super::*;

const SECRET: &str = "synthetic-sub2api-only";

fn read(body: &[u8]) -> Result<f64, RelayError> {
    parse_balance(
        body,
        "/remaining",
        None,
        &[],
        None,
        BalanceFormat::Sub2Api,
        SECRET,
    )
}

#[test]
fn sub2api_wallet_and_quota_fixtures_keep_usd_amounts() {
    assert_eq!(
        read(include_bytes!(
            "../../tests/fixtures/sub2api_wallet_usage.json"
        ))
        .unwrap(),
        42.5
    );
    assert_eq!(
        read(include_bytes!(
            "../../tests/fixtures/sub2api_quota_usage.json"
        ))
        .unwrap(),
        986.17194888
    );
    for (body, expected) in [
        (
            serde_json::json!({"remaining":0,"balance":20,"unit":"USD","isValid":true}),
            0.0,
        ),
        (
            serde_json::json!({"balance":42.5,"unit":"USD","isValid":true}),
            42.5,
        ),
        (
            serde_json::json!({"remaining":-1,"balance":-1,"unit":"USD","isValid":true,"mode":"unrestricted"}),
            -1.0,
        ),
        (
            serde_json::json!({"remaining":0,"unit":"USD","isValid":true,"mode":"quota_limited","quota":{"limit":10,"used":10,"remaining":0,"unit":"USD"}}),
            0.0,
        ),
        (
            serde_json::json!({"remaining":2,"unit":"USD","isValid":true,"subscription":{"daily_limit_usd":10}}),
            2.0,
        ),
    ] {
        assert_eq!(read(body.to_string().as_bytes()).unwrap(), expected);
    }
}

#[test]
fn sub2api_unknown_invalid_and_unlimited_results_are_unavailable() {
    for body in [
        serde_json::json!({}),
        serde_json::json!({"balance":null,"remaining":null,"unit":"USD","isValid":true}),
        serde_json::json!({"remaining":"12.5","unit":"USD","isValid":true}),
        serde_json::json!({"remaining":true,"unit":"USD","isValid":true}),
        serde_json::json!({"remaining":5,"unit":"USD","isValid":false}),
        serde_json::json!({"remaining":5,"unit":"USD","isValid":"true"}),
        serde_json::json!({"remaining":5,"unit":"USD","error":{"message":"unavailable"}}),
        serde_json::json!({"remaining":5,"unit":"CNY","isValid":true}),
        serde_json::json!({"remaining":5,"isValid":true}),
        serde_json::json!({"remaining":5,"unit":"USD","mode":"unknown"}),
        serde_json::json!({"mode":"quota_limited","unit":"USD","isValid":true,"rate_limits":[{"window":"5h","remaining":5}]}),
        serde_json::json!({"remaining":5,"unit":"USD","mode":"quota_limited"}),
        serde_json::json!({"balance":5,"unit":"USD","mode":"quota_limited","quota":{"unit":"USD"}}),
        serde_json::json!({"remaining":5,"unit":"USD","quota":{"unit":"CNY"}}),
        serde_json::json!({"remaining":-1,"unit":"USD","quota":{"unit":"USD"}}),
        serde_json::json!({"remaining":-1,"unit":"USD","subscription":{"daily_limit_usd":null,"weekly_limit_usd":null,"monthly_limit_usd":null}}),
        serde_json::json!({"remaining":-1,"unit":"USD","subscription":{"daily_limit_usd":0}}),
    ] {
        assert!(read(body.to_string().as_bytes()).is_err(), "{body}");
    }
    for body in [
        b"{".as_slice(),
        br#"{"remaining":1e999,"unit":"USD"}"#,
        br#"[]"#,
    ] {
        assert!(read(body).is_err());
    }
}

#[test]
fn sub2api_rejects_reflected_credentials_in_ignored_and_escaped_fields() {
    let escaped = SECRET
        .chars()
        .map(|ch| format!("\\u{:04x}", ch as u32))
        .collect::<String>();
    for field in [
        format!(r#""metadata":{{"private_key":"{SECRET}"}}"#),
        format!(r#""metadata":{{"private_key":"{escaped}"}}"#),
        format!(r#""{escaped}":"ignored""#),
        format!(r#""metadata":["{escaped}"]"#),
    ] {
        let body = format!(r#"{{"remaining":42.5,"unit":"USD","isValid":true,{field}}}"#);
        assert!(read(body.as_bytes()).is_err());
    }
}

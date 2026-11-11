use super::*;
use crate::relay::{ClientProtocol, Limits, Request};

fn source() -> SourceContract {
    serde_json::from_value(serde_json::json!({
        "source_revision": charging::legacy_precharge::SOURCE_REVISION,
        "upstream_id": 1, "upstream_model": "openrouter/gpt-6.1-sol",
        "model": "gpt-6.1-sol", "preconsumed_tokens": 5000,
        "count_token": true, "get_media_token": true,
        "get_media_token_not_stream": false, "source_free_model": false,
        "operator_max_billable": 100000
    }))
    .unwrap()
}

#[test]
fn ordinary_responses_authorization_uses_original_body_and_explicit_output() {
    for (body, completion, hold) in [
        (
            br#"{"model":"gpt-6.1-sol","input":"hello"}"#.as_slice(),
            0,
            400,
        ),
        (
            br#"{"model":"gpt-6.1-sol","input":"hello","max_output_tokens":100}"#.as_slice(),
            100,
            408,
        ),
    ] {
        let request = Request::parse(ClientProtocol::Responses, body, &Limits::default()).unwrap();
        let authorization =
            authorization(&source(), &request, body, 4_000_000, 40_000, None, 100000).unwrap();
        let charging::contract::LegacyPrechargeAuthorization::Cc52OrdinaryWalletV1 {
            estimated_hold,
            estimate,
            estimate_sha256,
            source_contract_sha256,
            ..
        } = authorization
        else {
            panic!("expected ordinary authorization")
        };
        assert_eq!(estimate.prompt_tokens, 1);
        assert_eq!(estimate.explicit_max_tokens.unwrap_or(0), completion);
        assert_eq!(estimated_hold, hold);
        assert_eq!(estimate.hold().unwrap(), hold);
        assert_eq!(
            estimate_sha256,
            charging::contract::typed_digest(&estimate).unwrap()
        );
        assert_eq!(
            source_contract_sha256,
            charging::contract::typed_digest(&source()).unwrap()
        );
        assert!(estimate.options.get_media_token);
        assert!(!estimate.options.get_media_token_not_stream);
    }
}

#[test]
fn source_contract_requires_explicit_options_and_fixed_revision() {
    let source = source();
    source.validate().unwrap();
    for field in [
        "preconsumed_tokens",
        "count_token",
        "get_media_token",
        "get_media_token_not_stream",
        "source_free_model",
    ] {
        let mut value = serde_json::to_value(&source).unwrap();
        value.as_object_mut().unwrap().remove(field);
        assert!(
            serde_json::from_value::<SourceContract>(value).is_err(),
            "{field}"
        );
    }
    let mut missing_cap = serde_json::to_value(&source).unwrap();
    missing_cap
        .as_object_mut()
        .unwrap()
        .remove("operator_max_billable");
    assert!(
        serde_json::from_value::<SourceContract>(missing_cap)
            .unwrap()
            .validate()
            .is_err()
    );
    let mut invalid = source;
    invalid.source_revision = "unreviewed".into();
    assert!(invalid.validate().is_err());
}

#[test]
fn source_authorization_rejects_media_estimates() {
    let body = br#"{"model":"gpt-6.1-sol","input":[{"role":"user","content":[{"type":"input_image","image_url":"https://example.invalid/image.png"}]}]}"#;
    let request = Request::parse(ClientProtocol::Responses, body, &Limits::default()).unwrap();
    assert!(authorization(&source(), &request, body, 4_000_000, 40_000, None, 100000).is_err());
}

fn fable_source() -> SourceContract {
    let mut source = source();
    source.model = "claude-fable-5".into();
    source.upstream_model = "anthropic/claude-fable-5".into();
    source.source_tariff = Some(serde_json::json!({
        "format_version": 1, "kind": "legacy_ratio_decimal", "source_mode": "ratio",
        "source_revision": charging::legacy_precharge::SOURCE_REVISION,
        "source_options_sha256": "ab".repeat(32), "source_channel_sha256": "cd".repeat(32),
        "contract_revision": "reviewed-fable-test-v1", "oracle_sha256": "ef".repeat(32),
        "source_usage_contract": "cc52-openrouter-chat-fable-usage-v1",
        "model": "claude-fable-5", "group": "allowed",
        "ratios": {"model":"3/2", "completion":"6166667/1000000", "cache":"1/10", "group":"1/25"},
        "cache_ratio_default": null, "adapter_contract":"openrouter-chat-v1",
        "non_audio_contract":"openrouter-chat-non-audio-v1", "source_operations":["chat.completions"],
        "max_input_tokens": 100000, "max_output_tokens": 16384,
        "payload_policy":"legacy_openrouter_chat_usage_v1", "input_basis":"projected-disjoint-sum",
        "creation_ratio":"5/4", "creation_ratio_1h":"2",
        "cost_inference":"cc52_custom_ModelRatio_not_in_default_map",
        "counter_projection":"cc52-DTO-alias-TTL-semantic-priority-v1"
    }));
    source
}

fn fable_price(source: &SourceContract) -> charging::Price {
    serde_json::from_value(serde_json::json!({
        "version":"routing-1-rule-1", "model":source.model, "group":"allowed",
        "input_per_million":0,"cached_per_million":0,"output_per_million":0,
        "tiered": {
            "tariff": {
                "revision":3,
                "source_digest":charging::contract::legacy_decimal::source_digest(source.source_tariff.as_ref().unwrap()).unwrap(),
                "uses_cache_read":true,"uses_cache_write":true,"uses_cache_write_1h":true,
                "tiers":[{"up_to_input_tokens":null,"rates":{
                    "input":{"numerator":3,"denominator":1},
                    "output":{"numerator":18500001,"denominator":1000000},
                    "cache_read":{"numerator":3,"denominator":10},
                    "cache_write":{"numerator":15,"denominator":4},
                    "cache_write_1h":{"numerator":6,"denominator":1}
                }}]
            },
            "input_semantics":"anthropic_text", "group_multiplier_ppm":40000,
            "speed_multiplier_ppm":1000000,"credits_per_usd":500000,
            "cache_creation_mode":"generic_remainder_v1"
        }
    })).unwrap()
}

#[test]
fn reviewed_fable_source_binds_decimal_tariff_and_preserves_actual_usage_price() {
    let source = fable_source();
    let price = fable_price(&source);
    price.validate().unwrap();
    let origin = source.source_tariff.as_ref().unwrap();
    assert!(matches_source_tariff(&source, origin, &price).unwrap());
    let body = br#"{"model":"claude-fable-5","input":"hello","max_output_tokens":100}"#;
    let request = Request::parse(ClientProtocol::Responses, body, &Limits::default()).unwrap();
    let charging::contract::LegacyPrechargeAuthorization::Cc52OrdinaryWalletV1 {
        estimated_hold,
        estimate,
        ..
    } = authorization(&source, &request, body, 0, 40000, None, 100000).unwrap()
    else {
        panic!("expected ordinary decimal authorization")
    };
    assert_eq!(estimated_hold, 306);
    assert_eq!(estimate.hold().unwrap(), estimated_hold);
    assert_eq!(estimate.model_ratio_bits, 1.5_f64.to_bits());
    assert_eq!(estimate.group_ratio_bits, 0.04_f64.to_bits());
    assert_eq!(estimate.source_tariff.as_ref(), Some(origin));
    let mut changed = price.clone();
    changed.tiered.as_mut().unwrap().tariff.source_digest = "00".repeat(32);
    assert!(!matches_source_tariff(&source, origin, &changed).unwrap());
    let mut unreviewed = origin.clone();
    unreviewed["kind"] = "tiered_expr".into();
    assert!(!matches_source_tariff(&source, &unreviewed, &price).unwrap());
    // Settlement continues to use the original decimal tariff and cache rates.
    assert_eq!(
        price.tiered.unwrap().tariff.tiers[0].rates.output.numerator,
        18500001
    );
}

#[test]
fn ordinary_native_chat_authorization_uses_chat_source_counter() {
    let body = br#"{"model":"gpt-6.1-sol","messages":[{"role":"user","content":"hello"}],"max_tokens":100}"#;
    let request = Request::parse(ClientProtocol::Chat, body, &Limits::default()).unwrap();
    let charging::contract::LegacyPrechargeAuthorization::Cc52OrdinaryWalletV1 {
        estimated_hold,
        estimate,
        ..
    } = authorization(&source(), &request, body, 4_000_000, 40_000, None, 100000).unwrap()
    else {
        panic!("expected ordinary chat authorization")
    };
    assert_eq!(estimate.explicit_max_tokens, Some(100));
    assert_eq!(estimated_hold, 408);
    assert_eq!(estimate.hold().unwrap(), estimated_hold);
    assert!(estimate.source_tariff.is_none());
}

#[test]
fn expression_precharge_requires_matching_frozen_source_and_separate_contract() {
    let mut source = fable_source();
    let expression = r#"tier("standard", p * 3 + c * 15)"#;
    let mut price = fable_price(&source);
    price.tiered.as_mut().unwrap().tariff.source_digest =
        charging::contract::digest(expression.as_bytes());
    assert!(matches_source_expression(expression, &price));
    assert!(!matches_source_expression(
        r#"tier("standard", p * 4 + c * 15)"#,
        &price
    ));
    source.source_expression = Some(expression.into());
    assert!(source.validate().is_err());
    source.source_tariff = None;
    source.validate().unwrap();
}

#[test]
fn expression_authorization_preserves_default_and_explicit_source_completion() {
    let mut source = source();
    source.source_expression = Some(r#"tier("standard", p * 2 + c * 4)"#.into());
    for (body, expected) in [
        (br#"{"model":"gpt-6.1-sol","messages":[{"role":"user","content":"hello"}]}"#.as_slice(), 656),
        (br#"{"model":"gpt-6.1-sol","messages":[{"role":"user","content":"hello"}],"max_tokens":3}"#.as_slice(), 1),
    ] {
        let request = Request::parse(ClientProtocol::Chat, body, &Limits::default()).unwrap();
        let charging::contract::LegacyPrechargeAuthorization::Cc52TieredWalletV1 {
            estimated_hold, estimate_sha256, estimate, ..
        } = super::super::legacy_tiered_precharge::authorization(
            &source, &request, 10, 40000, None, 100000,
        ).unwrap() else { panic!("expected expression authorization") };
        assert_eq!(estimated_hold, expected);
        assert_eq!(estimate.hold().unwrap(), expected);
        assert_eq!(estimate_sha256, charging::contract::typed_digest(&estimate).unwrap());
        assert_eq!(estimate.source_expression, r#"tier("standard", p * 2 + c * 4)"#);
    }
}

#[test]
fn observed_fable_expressions_compile_without_calendar_and_reach_chat_authorization() {
    for (model, expression) in [
        (
            "claude-fable-5",
            r#"tier("standard", p * 10 + cr * 1 + cc * 12.5 + cc1h * 20 + c * 50)"#,
        ),
        (
            "claude-fable-5-1",
            r#"tier("standard", p * 10 + cr * 0.25 + cc * 12.5 + cc1h * 20 + c * 50)"#,
        ),
    ] {
        let program = crate::billing::legacy_tiered::Program::compile(expression).unwrap();
        assert!(!program.requires_clock());
        let mut source = source();
        source.model = model.into();
        source.source_expression = Some(expression.into());
        source.validate().unwrap();
        let body = serde_json::to_vec(&serde_json::json!({
            "model":model, "messages":[{"role":"user","content":"hello"}]
        }))
        .unwrap();
        let request = Request::parse(ClientProtocol::Chat, &body, &Limits::default()).unwrap();
        let charging::contract::LegacyPrechargeAuthorization::Cc52TieredWalletV1 {
            estimate,
            estimated_hold,
            ..
        } = authorization(&source, &request, &body, 0, 40000, None, 100000).unwrap()
        else {
            panic!("expected actual Fable expression authorization")
        };
        assert!(estimate.prompt_tokens > 0);
        assert!(estimate.explicit_max_tokens.is_none());
        assert_eq!(estimate.source_expression, expression);
        assert_eq!(estimate.hold().unwrap(), estimated_hold);
        assert!(estimated_hold >= 8192);
    }
}

#[test]
fn derived_source_contract_is_explicit_and_preserves_static_serialization() {
    let original = source();
    let encoded = serde_json::to_value(&original).unwrap();
    assert!(encoded.get("derive_native_reservation").is_none());
    assert!(!original.derive_native_reservation);
    let mut dynamic = encoded.clone();
    dynamic
        .as_object_mut()
        .unwrap()
        .remove("operator_max_billable");
    dynamic["derive_native_reservation"] = true.into();
    let dynamic: SourceContract = serde_json::from_value(dynamic).unwrap();
    dynamic.validate().unwrap();
    assert_eq!(dynamic.operator_max_billable, 0);
    let mut ambiguous = dynamic.clone();
    ambiguous.operator_max_billable = 100000;
    assert!(ambiguous.validate().is_err());
    let mut missing_cap = dynamic;
    missing_cap.derive_native_reservation = false;
    assert!(missing_cap.validate().is_err());
    assert_eq!(serde_json::to_value(original).unwrap(), encoded);
}

#[test]
fn derived_cap_uses_exact_prepared_bounds_selected_price_and_trust() {
    let mut source = fable_source();
    source.derive_native_reservation = true;
    source.operator_max_billable = 0;
    let price = fable_price(&source);
    let trust: crate::billing::trust_pricing::FrozenTrustPrice =
        serde_json::from_value(serde_json::json!({
            "user_id":7, "policy_revision":1, "basis_revision":2,
            "effective_level":1, "level_source":"automatic",
            "discount":{"numerator":9,"denominator":10},
            "evaluated_at":100, "next_decay_at":200, "evidence_digest":vec![1u8;32]
        }))
        .unwrap();
    let mut frozen = price.clone();
    frozen.user_trust = Some(trust.clone());
    for bounds in [(10_000, 100), (20_000, 200)] {
        let cap = source
            .maximum_billable(&price, bounds, Some(&trust))
            .unwrap();
        assert_eq!(
            cap,
            frozen
                .reservation_ceiling(charging::ReservationBasis::Tokens {
                    input: bounds.0,
                    output: bounds.1,
                })
                .unwrap()
        );
        assert!(cap < source.maximum_billable(&price, bounds, None).unwrap());
        let body = br#"{"model":"claude-fable-5","input":"hello","max_output_tokens":100}"#;
        let request = Request::parse(ClientProtocol::Responses, body, &Limits::default()).unwrap();
        let authorization =
            authorization(&source, &request, body, 0, 40000, Some(trust.clone()), cap).unwrap();
        assert!(authorization.derives_native_reservation());
        assert_eq!(authorization.max_billable(), cap);
        let charging::contract::LegacyPrechargeAuthorization::Cc52OrdinaryWalletV1 {
            source_contract_sha256,
            estimate,
            ..
        } = authorization
        else {
            panic!("expected ordinary authorization")
        };
        assert_eq!(
            source_contract_sha256,
            charging::contract::typed_digest(&source).unwrap()
        );
        assert_eq!(estimate.trust.as_ref(), Some(&trust));
    }
    assert_eq!(price.user_trust, None);
}

#[test]
fn expression_derived_cap_is_frozen_and_rejects_underfunded_estimate() {
    let mut source = source();
    source.derive_native_reservation = true;
    source.operator_max_billable = 0;
    source.source_expression = Some(r#"tier("standard", p * 2 + c * 4)"#.into());
    let mut price = fable_price(&fable_source());
    price.tiered.as_mut().unwrap().tariff.source_digest =
        charging::contract::digest(source.source_expression.as_ref().unwrap().as_bytes());
    let body = br#"{"model":"gpt-6.1-sol","messages":[{"role":"user","content":"hello"}]}"#;
    let request = Request::parse(ClientProtocol::Chat, body, &Limits::default()).unwrap();
    let cap = source
        .maximum_billable(&price, (10_000, 10_000), None)
        .unwrap();
    let authorization = super::super::legacy_tiered_precharge::authorization(
        &source, &request, 10, 40000, None, cap,
    )
    .unwrap();
    assert!(authorization.derives_native_reservation());
    assert_eq!(authorization.max_billable(), cap);
    assert!(super::super::legacy_tiered_precharge::authorization(
        &source, &request, 10, 40000, None, 0,
    ).is_err());
}

#[test]
fn ordinary_estimate_cannot_raise_a_smaller_native_cap() {
    let mut source = source();
    source.derive_native_reservation = true;
    source.operator_max_billable = 0;
    let body = br#"{"model":"gpt-6.1-sol","input":"hello","max_output_tokens":100}"#;
    let request = Request::parse(ClientProtocol::Responses, body, &Limits::default()).unwrap();
    assert!(matches!(
        authorization(&source, &request, body, 4_000_000, 40_000, None, 407),
        Err(RelayError::Limit(
            "legacy precharge exceeds operator maximum"
        ))
    ));
}

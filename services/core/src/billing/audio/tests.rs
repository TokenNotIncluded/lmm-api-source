use super::*;
use crate::billing::{Credits, trust_pricing::DiscountRatio};

fn ratio(n: u64, d: u64) -> AudioRatio {
    AudioRatio {
        numerator: n,
        denominator: d,
    }
}
fn usage(ti: u64, to: u64, ai: u64, ao: u64) -> AudioUsage {
    AudioUsage {
        text_input: ti,
        text_output: to,
        audio_input: ai,
        audio_output: ao,
    }
}
fn spec(tariff: AudioTariff) -> AudioQuoteSpec {
    AudioQuoteSpec {
        version: 1,
        price_revision: "synthetic-v1".into(),
        source_sha256: [1; 32],
        binding: AudioBinding {
            request_id: "synthetic-request".into(),
            request_sha256: [2; 32],
            authority_sha256: [3; 32],
            route_id: "synthetic-route".into(),
            upstream_id: 1,
            model: "synthetic-audio".into(),
            group: "synthetic-group".into(),
            provider_contract_revision: "four-counters-v1".into(),
        },
        tariff,
        group_multiplier: ratio(1, 1),
        discount: None,
        rounding: AudioRounding::Ceiling,
        usage_format: AudioUsageFormat::FourCountersV1,
        bounds: usage(100, 100, 100, 100),
        max_budget: Credits::try_from(1_000_000_i64).unwrap(),
    }
}
fn dispatch(q: &AudioQuote) -> AudioDispatch {
    AudioDispatch {
        request_id: q.persisted().binding.request_id.clone(),
        attempt_id: "attempt-1".into(),
        route_id: q.persisted().binding.route_id.clone(),
        wire_sha256: [4; 32],
        quote_sha256: q.digest(),
    }
}
fn cost(q: &AudioQuote, counts: AudioUsage) -> i64 {
    i64::from(
        q.settle(&dispatch(q), &serde_json::to_vec(&counts).unwrap(), true)
            .unwrap()
            .credits(),
    )
}
fn fixed() -> AudioTariff {
    AudioTariff::Fixed {
        usd_per_request: ratio(1, 500_000),
    }
}

#[test]
fn four_modalities_and_discount_are_rounded_once() {
    let mut s = spec(AudioTariff::Multipliers {
        model: ratio(1, 2),
        completion: ratio(2, 1),
        audio: ratio(3, 1),
        audio_completion: ratio(4, 1),
    });
    s.group_multiplier = ratio(3, 2);
    s.discount = Some(DiscountRatio::new(1, 2).unwrap());
    let q = AudioQuote::freeze(s.clone()).unwrap();
    // (1 + 2*2 + 3*3 + 4*3*4) * 1/2 * 3/2 * 1/2 = 23.25.
    assert_eq!(cost(&q, usage(1, 2, 3, 4)), 24);
    s.rounding = AudioRounding::HalfUp;
    assert_eq!(cost(&AudioQuote::freeze(s).unwrap(), usage(1, 2, 3, 4)), 23);
}

#[test]
fn fixed_price_still_requires_complete_terminal_usage() {
    let q = AudioQuote::freeze(spec(fixed())).unwrap();
    assert_eq!(cost(&q, usage(0, 0, 0, 0)), 1);
    let d = dispatch(&q);
    assert!(matches!(
        q.settle(
            &d,
            br#"{"text_input":0,"text_output":0,"audio_input":0}"#,
            true
        ),
        Err(AudioError::UnknownUsage)
    ));
    assert!(matches!(
        q.settle(&d, b"", true),
        Err(AudioError::UnknownUsage)
    ));
    let raw = serde_json::to_vec(&usage(0, 0, 0, 0)).unwrap();
    assert!(matches!(
        q.settle(&d, &raw, false),
        Err(AudioError::UnknownUsage)
    ));
    assert!(matches!(
        q.settle(&d, &serde_json::to_vec(&usage(101, 0, 0, 0)).unwrap(), true),
        Err(AudioError::BoundsExceeded)
    ));
}

#[test]
fn falling_tier_prices_do_not_under_reserve_and_boundary_is_inclusive() {
    let rates = |n| AudioRates {
        text_input: ratio(n, 1),
        text_output: ratio(n * 2, 1),
        audio_input: ratio(n * 3, 1),
        audio_output: ratio(n * 4, 1),
    };
    let q = AudioQuote::freeze(spec(AudioTariff::Tiered {
        tiers: vec![
            AudioTier {
                up_to_input_tokens: Some(10),
                rates: rates(20),
            },
            AudioTier {
                up_to_input_tokens: None,
                rates: rates(2),
            },
        ],
    }))
    .unwrap();
    assert_eq!(i64::from(q.reservation_credits()), 10_000);
    assert_eq!(cost(&q, usage(6, 2, 4, 3)), 340);
    assert_eq!(cost(&q, usage(7, 2, 4, 3)), 35);
}

#[test]
fn original_quote_and_raw_bytes_are_required_for_replay() {
    let q = AudioQuote::freeze(spec(fixed())).unwrap();
    let d = dispatch(&q);
    let raw = serde_json::to_vec(&usage(1, 2, 3, 4)).unwrap();
    let receipt = q.settle(&d, &raw, true).unwrap();
    let restored =
        AudioQuote::restore(q.persisted().clone(), q.digest(), &q.persisted().binding).unwrap();
    assert_eq!(restored.replay(&d, &receipt), Ok(receipt.credits()));
    let mut changed = q.persisted().clone();
    changed.price_revision = "v2".into();
    assert!(matches!(
        AudioQuote::restore(changed, q.digest(), &q.persisted().binding),
        Err(AudioError::BindingMismatch)
    ));
    let mut encoded = serde_json::to_value(&receipt).unwrap();
    encoded["raw_usage"] = serde_json::json!(serde_json::to_vec(&usage(2, 2, 3, 4)).unwrap());
    let corrupted: AudioSettlement = serde_json::from_value(encoded).unwrap();
    assert_eq!(
        restored.replay(&d, &corrupted),
        Err(AudioError::BindingMismatch)
    );
    let mut other_attempt = d;
    other_attempt.attempt_id = "attempt-2".into();
    assert_eq!(
        restored.replay(&other_attempt, &receipt),
        Err(AudioError::BindingMismatch)
    );
}

#[test]
fn provider_totals_must_match_all_four_explicit_counts() {
    for (format, raw) in [
        (
            AudioUsageFormat::OpenAiChatV1,
            serde_json::json!({"prompt_tokens":4,"completion_tokens":6,"total_tokens":10,"prompt_tokens_details":{"text_tokens":1,"audio_tokens":3},"completion_tokens_details":{"text_tokens":2,"audio_tokens":4}}),
        ),
        (
            AudioUsageFormat::OpenAiRealtimeV1,
            serde_json::json!({"input_tokens":4,"output_tokens":6,"total_tokens":10,"input_token_details":{"text_tokens":1,"audio_tokens":3},"output_token_details":{"text_tokens":2,"audio_tokens":4}}),
        ),
    ] {
        let mut s = spec(fixed());
        s.usage_format = format;
        let q = AudioQuote::freeze(s).unwrap();
        let d = dispatch(&q);
        let receipt = q
            .settle(&d, &serde_json::to_vec(&raw).unwrap(), true)
            .unwrap();
        assert_eq!(receipt.usage(), usage(1, 2, 3, 4));
        let mut malformed = raw;
        malformed["total_tokens"] = 9.into();
        assert!(matches!(
            q.settle(&d, &serde_json::to_vec(&malformed).unwrap(), true),
            Err(AudioError::InvalidUsage)
        ));
    }
}

#[test]
fn fractional_fixed_prices_and_limits_never_clamp_or_wrap() {
    let mut s = spec(AudioTariff::Fixed {
        usd_per_request: ratio(3, 1_000_000),
    });
    s.discount = Some(DiscountRatio::new(1, 2).unwrap());
    assert_eq!(
        cost(&AudioQuote::freeze(s.clone()).unwrap(), usage(0, 0, 0, 0)),
        1
    );
    s.max_budget = Credits::try_from(0_i64).unwrap();
    assert!(matches!(
        AudioQuote::freeze(s),
        Err(AudioError::BudgetExceeded)
    ));
    let huge = spec(AudioTariff::Fixed {
        usd_per_request: ratio(u64::MAX, 1),
    });
    assert!(matches!(
        AudioQuote::freeze(huge),
        Err(AudioError::Overflow)
    ));
    let invalid = spec(AudioTariff::Fixed {
        usd_per_request: ratio(1, 0),
    });
    assert!(matches!(
        AudioQuote::freeze(invalid),
        Err(AudioError::InvalidQuote)
    ));
}

#[test]
fn chat_v2_normalizes_official_optional_text_without_changing_v1() {
    let raw=br#"{"prompt_tokens":19,"completion_tokens":10,"total_tokens":29,"prompt_tokens_details":{"cached_tokens":0,"audio_tokens":0},"completion_tokens_details":{"reasoning_tokens":0,"audio_tokens":0,"accepted_prediction_tokens":0,"rejected_prediction_tokens":0}}"#;
    assert!(AudioUsageFormat::OpenAiChatV1.parse(raw).is_err());
    assert_eq!(
        AudioUsageFormat::OpenAiChatV2.parse(raw).unwrap(),
        usage(19, 10, 0, 0)
    );
    let explicit=br#"{"prompt_tokens":4,"completion_tokens":6,"total_tokens":10,"prompt_tokens_details":{"text_tokens":1,"audio_tokens":3,"cache_write_tokens":0,"image_tokens":0},"completion_tokens_details":{"text_tokens":2,"audio_tokens":4}}"#;
    assert_eq!(
        AudioUsageFormat::OpenAiChatV1.parse(explicit).unwrap(),
        AudioUsageFormat::OpenAiChatV2.parse(explicit).unwrap()
    );
    let mut value: serde_json::Value = serde_json::from_slice(explicit).unwrap();
    for bad in [
        serde_json::json!(1),
        serde_json::Value::Null,
        serde_json::json!("0"),
    ] {
        value["prompt_tokens_details"]["image_tokens"] = bad;
        assert!(
            AudioUsageFormat::OpenAiChatV2
                .parse(&serde_json::to_vec(&value).unwrap())
                .is_err()
        );
    }
    value["prompt_tokens_details"]["image_tokens"] = serde_json::json!(0);
    value["prompt_tokens_details"]["text_tokens"] = serde_json::json!(2);
    assert!(
        AudioUsageFormat::OpenAiChatV2
            .parse(&serde_json::to_vec(&value).unwrap())
            .is_err()
    );
    value["prompt_tokens_details"]["text_tokens"] = serde_json::json!(1);
    value["prompt_tokens_details"]
        .as_object_mut()
        .unwrap()
        .remove("audio_tokens");
    assert!(
        AudioUsageFormat::OpenAiChatV2
            .parse(&serde_json::to_vec(&value).unwrap())
            .is_err()
    );
    let duplicate = std::str::from_utf8(explicit).unwrap().replace(
        "\"audio_tokens\":3",
        "\"audio_tokens\":9,\"audio_tokens\":3",
    );
    for format in [
        AudioUsageFormat::OpenAiChatV1,
        AudioUsageFormat::OpenAiChatV2,
    ] {
        assert!(format.parse(duplicate.as_bytes()).is_err());
    }
    let underflow = std::str::from_utf8(explicit)
        .unwrap()
        .replace("\"audio_tokens\":3", "\"audio_tokens\":5");
    assert!(
        AudioUsageFormat::OpenAiChatV2
            .parse(underflow.as_bytes())
            .is_err()
    );
    let invalid_cache = std::str::from_utf8(explicit)
        .unwrap()
        .replace("\"cache_write_tokens\":0", "\"cache_write_tokens\":null");
    assert!(
        AudioUsageFormat::OpenAiChatV2
            .parse(invalid_cache.as_bytes())
            .is_err()
    );
}

#[test]
fn fresh_chat_usage_does_not_persist_body_inside_unknown_fields() {
    let raw=br#"{"prompt_tokens":4,"completion_tokens":6,"prompt_tokens_details":{"text_tokens":1,"audio_tokens":3},"completion_tokens_details":{"text_tokens":2,"audio_tokens":4},"body":"PRIVATE_BODY"}"#;
    // Historical V1 decoding semantics remain separate from new evidence capture.
    assert_eq!(
        AudioUsageFormat::OpenAiChatV1.parse(raw).unwrap(),
        usage(1, 2, 3, 4)
    );
    assert!(
        AudioUsageFormat::OpenAiChatV1
            .validate_retained_chat_usage(raw)
            .is_err()
    );
}

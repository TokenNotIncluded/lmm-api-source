// SPDX-License-Identifier: AGPL-3.0-or-later
use super::*;
use eval::QuotaClamp;

fn money(group: f64) -> Financial {
    Financial::from_bits(500_000_f64.to_bits(), group.to_bits()).unwrap()
}
fn evaluate(expr: &str, raw: &RawUsage) -> Evaluation {
    let p = Program::compile(expr).unwrap();
    let params = project_source(Some(raw), p.used_vars()).unwrap();
    p.evaluate_f64(&params, money(1.0), None).unwrap()
}
#[test]
fn float_conversion_retains_the_source_one_credit_counterexample() {
    let result = evaluate(
        "tier(\"base\",p*2+c*10+cr*0.1+cc*2.5)",
        &RawUsage {
            prompt: 2490,
            cached: 2490,
            ..RawUsage::default()
        },
    );
    assert_eq!(result.raw_expression_bits, 249_f64.to_bits());
    assert_eq!(
        result.before_group_bits,
        (249.0_f64 / 1_000_000.0 * 500_000.0).to_bits()
    );
    assert_eq!(result.quota, 124);
}
#[test]
fn parentheses_and_addition_order_are_not_collected_into_rates() {
    let mut params = TokenParams::default();
    params.values[Variable::P as usize] = 1e16;
    params.values[Variable::C as usize] = 1.0;
    params.values[Variable::Cr as usize] = 1.0;
    let a = Program::compile("tier(\"a\",p+c+cr)")
        .unwrap()
        .evaluate_f64(&params, money(1.0), None)
        .unwrap();
    let b = Program::compile("tier(\"b\",p+(c+cr))")
        .unwrap()
        .evaluate_f64(&params, money(1.0), None)
        .unwrap();
    assert_ne!(a.raw_expression_bits, b.raw_expression_bits);
}
#[test]
fn zero_creation_coefficient_still_changes_source_prompt_projection() {
    let result = evaluate(
        "tier(\"base\",p+cc*0)",
        &RawUsage {
            prompt: 100,
            creation_openai: 20,
            ..RawUsage::default()
        },
    );
    assert_eq!(f64::from_bits(result.raw_expression_bits), 80.0);
}
#[test]
fn claude_argument_and_creation_semantic_are_independent() {
    let program = Program::compile("tier(\"base\",p+cr+cc+cc1h)").unwrap();
    let mut raw = RawUsage {
        prompt: 100,
        cached: 20,
        creation_openai: 40,
        creation_5m: 5,
        creation_1h: 3,
        prompt_semantics: PromptSemantics::ClaudeText,
        ..RawUsage::default()
    };
    let a = project_source(Some(&raw), program.used_vars()).unwrap();
    assert_eq!(a.value(Variable::Len), Some(160.0));
    assert_eq!(a.value(Variable::P), Some(100.0));
    raw.creation_semantics = CreationSemantics::Anthropic;
    let b = project_source(Some(&raw), program.used_vars()).unwrap();
    assert_eq!(b.value(Variable::Len), Some(128.0));
    assert_eq!(b.value(Variable::Cc1h), Some(3.0));
}
#[test]
fn classified_media_union_and_missing_measurement_are_distinct() {
    let p = Program::compile("tier(\"base\",p*5+cr_text+img*8+cr_img*2+img_o*30)").unwrap();
    let mut raw = RawUsage {
        prompt: 100,
        cached: 30,
        input_text: 80,
        input_image: 20,
        completion: 100,
        output_image: 90,
        cached_details: Some(CacheDetails {
            text: 20,
            image: 10,
            audio: 0,
        }),
        ..RawUsage::default()
    };
    let params = project_source(Some(&raw), p.used_vars()).unwrap();
    assert_eq!(params.value(Variable::P), Some(60.0));
    assert_eq!(params.value(Variable::Img), Some(10.0));
    assert_eq!(params.value(Variable::C), Some(10.0));
    raw.cached_details = None;
    assert!(matches!(
        project_source(Some(&raw), p.used_vars()),
        Err(Error::Measurement)
    ));
    raw.cached = 0;
    assert!(project_source(Some(&raw), p.used_vars()).is_ok());
    let nil = project_source(None, p.used_vars()).unwrap();
    assert!(matches!(
        p.evaluate_f64(&nil, money(1.0), None),
        Err(Error::Measurement)
    ));
}
#[test]
fn source_clamps_overlapping_unclassified_input_and_negative_creation() {
    let result = evaluate(
        "tier(\"base\",p+cr+cc)",
        &RawUsage {
            prompt: 100,
            cached: 80,
            creation_openai: 70,
            ..RawUsage::default()
        },
    );
    assert_eq!(f64::from_bits(result.raw_expression_bits), 150.0);
    let result = evaluate(
        "tier(\"base\",p+cc)",
        &RawUsage {
            prompt: 10,
            creation_legacy: -8,
            creation_openai: -2,
            ..RawUsage::default()
        },
    );
    assert_eq!(f64::from_bits(result.raw_expression_bits), 10.0);
}
#[test]
fn quota_rounding_saturation_minimum_and_free_group() {
    assert_eq!(quota_round_checked(-0.5), (-1, None));
    assert_eq!(quota_round_checked(0.5), (1, None));
    assert_eq!(quota_round_checked(f64::NAN), (0, Some(QuotaClamp::NaN)));
    assert_eq!(
        quota_round_checked(f64::INFINITY),
        (i32::MAX, Some(QuotaClamp::Overflow))
    );
    assert_eq!(
        quota_round_checked(f64::NEG_INFINITY),
        (i32::MIN, Some(QuotaClamp::Underflow))
    );
    let p = Program::compile("tier(\"base\",p*0.001)").unwrap();
    let params = project_source(
        Some(&RawUsage {
            prompt: 1,
            ..RawUsage::default()
        }),
        p.used_vars(),
    )
    .unwrap();
    assert_eq!(p.evaluate_f64(&params, money(0.1), None).unwrap().quota, 1);
    assert_eq!(p.evaluate_f64(&params, money(0.0), None).unwrap().quota, 0);
}
#[test]
fn calendar_requires_explicit_clock_and_replays_the_same_clock() {
    let expr = "weekday(\"UTC\")>=1&&weekday(\"UTC\")<=5&&hour(\"UTC\")<4&&!(date(\"Asia/Shanghai\")>=20260101&&date(\"Asia/Shanghai\")<=20260103)?tier(\"peak\",p*.3):tier(\"off_peak\",p*.15)";
    let p = Program::compile(expr).unwrap();
    let params = project_source(
        Some(&RawUsage {
            prompt: 1000,
            ..RawUsage::default()
        }),
        p.used_vars(),
    )
    .unwrap();
    assert!(matches!(
        p.evaluate_f64(&params, money(1.0), None),
        Err(Error::MissingClock)
    ));
    let before = SettlementClock::from_unix_seconds(1794283199).unwrap();
    let after = SettlementClock::from_unix_seconds(1794283201).unwrap();
    assert_eq!(
        p.evaluate_f64(&params, money(1.0), Some(before))
            .unwrap()
            .quota,
        150
    );
    assert_eq!(
        p.evaluate_f64(&params, money(1.0), Some(after))
            .unwrap()
            .quota,
        75
    );
    assert_eq!(
        p.evaluate_f64(&params, money(1.0), Some(before))
            .unwrap()
            .matched_tier,
        "peak"
    );
    let holiday = SettlementClock::from_unix_seconds(1767232800).unwrap();
    assert_eq!(
        p.evaluate_f64(&params, money(1.0), Some(holiday))
            .unwrap()
            .matched_tier,
        "off_peak"
    );
}
#[test]
fn reject_general_dsl_and_unbounded_inputs() {
    for expr in [
        "tier(\"x\",p*c)",
        "tier(\"x\",p+1)",
        "tier(\"x\",p-1)",
        "tier(\"x\",header(\"X\"))",
        "tier(\"x\",p)/2",
        "tier(\"x\",audio_s)",
    ] {
        assert!(Program::compile(expr).is_err(), "accepted {expr}");
    }
    assert!(Program::compile(&"(".repeat(17000)).is_err());
    assert!(SettlementClock::from_unix_seconds(0).is_err());
    assert!(Financial::from_bits(500_000_f64.to_bits(), f64::NAN.to_bits()).is_err());
}

fn integer(value: &str) -> i64 {
    value.parse().unwrap()
}
fn optional(value: &str) -> Option<i64> {
    (value != "-").then(|| integer(value))
}
fn decode_hex(value: &str) -> String {
    let bytes = (0..value.len())
        .step_by(2)
        .map(|i| u8::from_str_radix(&value[i..i + 2], 16).unwrap())
        .collect::<Vec<_>>();
    String::from_utf8(bytes).unwrap()
}
/// Generated outside the repository by the original cc52 GoVM and original DTO
/// projection, not hand-authored expected successes. Includes actual44 source
/// expressions, arithmetic bits, tiers, projections, errors and clock edges.
#[test]
#[ignore = "requires private source-bound GoVM fixture: LMM_LEGACY_TIERED_ORACLE"]
fn actual_old_go_vm_oracle() {
    let path = std::env::var("LMM_LEGACY_TIERED_ORACLE").expect("set actual GoVM fixture path");
    let text = std::fs::read_to_string(path).unwrap();
    let mut count = 0;
    for line in text.lines() {
        let fields = line.split('\t').collect::<Vec<_>>();
        assert_eq!(fields.len(), 40);
        let id = fields[0];
        let expr = decode_hex(fields[1]);
        let program = Program::compile(&expr).expect(id);
        assert_eq!(
            program.used_vars().0,
            fields[39].parse::<u16>().unwrap(),
            "used vars {id}"
        );
        let cached_details = optional(fields[18]).map(|text| CacheDetails {
            text,
            image: integer(fields[19]),
            audio: integer(fields[20]),
        });
        let raw = RawUsage {
            prompt: integer(fields[3]),
            completion: integer(fields[4]),
            cached: integer(fields[5]),
            creation_legacy: integer(fields[6]),
            creation_openai: integer(fields[7]),
            creation_5m: integer(fields[8]),
            creation_1h: integer(fields[9]),
            input_text: integer(fields[10]),
            input_image: integer(fields[11]),
            input_audio: integer(fields[12]),
            output_image: integer(fields[13]),
            prompt_semantics: if fields[14] == "1" {
                PromptSemantics::ClaudeText
            } else {
                PromptSemantics::Inclusive
            },
            creation_semantics: if fields[15] == "1" {
                CreationSemantics::Anthropic
            } else {
                CreationSemantics::Aggregate
            },
            cached_details,
        };
        let projected = project_source((fields[2] == "1").then_some(&raw), program.used_vars());
        let result = projected.as_ref().map_err(|e| *e).and_then(|params| {
            program.evaluate_f64(
                params,
                money(f64::from_bits(fields[16].parse().unwrap())),
                Some(SettlementClock::from_unix_seconds(integer(fields[17])).unwrap()),
            )
        });
        if fields[21] == "error" {
            assert!(result.is_err(), "oracle expected error: {id}");
            count += 1;
            continue;
        }
        let got = result.expect(id);
        assert_eq!(
            got.raw_expression_bits,
            fields[22].parse::<u64>().unwrap(),
            "raw {id}"
        );
        assert_eq!(
            got.before_group_bits,
            fields[23].parse::<u64>().unwrap(),
            "before {id}"
        );
        assert_eq!(
            got.after_group_bits,
            fields[24].parse::<u64>().unwrap(),
            "after {id}"
        );
        assert_eq!(got.quota, fields[25].parse::<i32>().unwrap(), "quota {id}");
        assert_eq!(got.matched_tier, fields[26], "tier {id}");
        assert_eq!(
            got.tier_cost_bits,
            fields[38].parse::<u64>().unwrap(),
            "tier value {id}"
        );
        let params = projected.unwrap();
        for (column, var) in [
            (27, Variable::P),
            (28, Variable::C),
            (29, Variable::Len),
            (30, Variable::Cc),
            (31, Variable::Cc1h),
            (32, Variable::Img),
            (33, Variable::CrText),
            (34, Variable::CrImg),
            (35, Variable::Cr),
            (36, Variable::ImgO),
            (37, Variable::Ai),
        ] {
            let expected = (fields[column] != "-").then(|| fields[column].parse::<u64>().unwrap());
            assert_eq!(
                params.value(var).map(f64::to_bits),
                expected,
                "projection {id}/{var:?}"
            );
        }
        count += 1;
    }
    assert!(count >= 100, "not the actual44 boundary oracle: {count}");
    eprintln!("actual original GoVM oracle cases matched: {count}");
}

// SPDX-License-Identifier: AGPL-3.0-or-later
use super::{Error, Financial, Program, RawUsage, SettlementClock, compat::*};

fn money(group: f64) -> Financial {
    Financial::from_bits(500_000_f64.to_bits(), group.to_bits()).unwrap()
}

#[test]
fn estimate_uses_source_length_completion_and_zero_classified_cache() {
    let program =
        Program::compile(r#"len>4?tier("high",p+2*c+3*cr_text+4*cr_img):tier("low",p)"#).unwrap();
    let estimate = estimate_text(&program, 5, 11, money(1.0), None).unwrap();
    assert_eq!(estimate.matched_tier, "high");
    assert_eq!(estimate.raw_expression_bits, 27_f64.to_bits());
    assert_eq!(estimate.quota, 14);
    // A measured cache hit requires its classification; the estimate's
    // known-zero cache is not evidence for this response.
    let measured = RawUsage {
        prompt: 5,
        cached: 1,
        ..RawUsage::default()
    };
    assert!(matches!(
        settle(&program, &measured, money(1.0), None),
        Err(Error::Measurement)
    ));
}

#[test]
fn estimate_strict_rounding_differs_from_terminal_minimum_and_clamp() {
    let program = Program::compile(r#"tier("standard",p*0.25)"#).unwrap();
    let usage = RawUsage {
        prompt: 1,
        ..RawUsage::default()
    };
    assert_eq!(
        estimate_text(&program, 1, 0, money(1.0), None)
            .unwrap()
            .quota,
        0
    );
    assert_eq!(settle(&program, &usage, money(1.0), None).unwrap().quota, 1);
    assert!(matches!(
        estimate_text(&program, 1, 0, money(1.0e20), None),
        Err(Error::Financial)
    ));
    let settled = settle(&program, &usage, money(1.0e20), None).unwrap();
    assert_eq!(settled.quota, i32::MAX);
    assert!(settled.clamp.is_some());
}

#[test]
fn calendar_uses_explicit_admission_and_terminal_clocks() {
    let program =
        Program::compile(r#"hour("UTC")<4?tier("peak",p*0.3):tier("off_peak",p*0.15)"#).unwrap();
    let usage = RawUsage {
        prompt: 1000,
        ..RawUsage::default()
    };
    let admission = SettlementClock::from_unix_seconds(1_767_239_999).unwrap();
    let terminal = SettlementClock::from_unix_seconds(1_767_240_000).unwrap();
    assert!(matches!(
        estimate_text(&program, 1000, 0, money(1.0), None),
        Err(Error::MissingClock)
    ));
    assert!(matches!(
        settle(&program, &usage, money(1.0), None),
        Err(Error::MissingClock)
    ));
    assert_eq!(
        estimate_text(&program, 1000, 0, money(1.0), Some(admission))
            .unwrap()
            .quota,
        150
    );
    for _ in 0..2 {
        let actual = settle(&program, &usage, money(1.0), Some(terminal)).unwrap();
        assert_eq!(
            (actual.quota, actual.matched_tier.as_str()),
            (75, "off_peak")
        );
    }
}

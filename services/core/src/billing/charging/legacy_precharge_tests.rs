use super::*;

#[test]
fn terminal_supplement_charges_cash_before_debt_within_authorization() {
    let split = |actual, wallet| WalletSupplement::for_terminal(400, actual, 210_944, wallet);
    assert_eq!(split(399, 300), Ok(WalletSupplement { cash: 0, debt: 0 }));
    assert_eq!(split(800, 500), Ok(WalletSupplement { cash: 400, debt: 0 }));
    assert_eq!(
        split(800, 125),
        Ok(WalletSupplement {
            cash: 125,
            debt: 275
        })
    );
    assert_eq!(split(800, 0), Ok(WalletSupplement { cash: 0, debt: 400 }));
    assert_eq!(split(210_945, 500), Err(Error::Invalid));
    assert_eq!(split(800, -1), Err(Error::Invalid));
    assert_eq!(
        WalletSupplement::for_terminal(0, i64::MAX, i64::MAX, 1),
        Ok(WalletSupplement {
            cash: 1,
            debt: i64::MAX - 1
        })
    );
}

fn options() -> SourceOptions {
    SourceOptions {
        // Actual fixed export, not the code's default 500.
        preconsumed_tokens: 5000,
        count_token: true,
        get_media_token: true,
        get_media_token_not_stream: false,
    }
}

#[test]
fn ordinary_missing_max_is_not_native_default_or_tiered_8192() {
    let q = ordinary_text(
        options(),
        7,
        None,
        2_f64.to_bits(),
        0.04_f64.to_bits(),
        false,
    )
    .unwrap();
    assert_eq!(q.hold, 400);
    assert_eq!(q.completion_tokens, 0);
    assert_eq!(
        ordinary_text(
            options(),
            7,
            Some(100),
            2_f64.to_bits(),
            0.04_f64.to_bits(),
            false
        )
        .unwrap()
        .hold,
        408
    );
    assert_eq!(
        ordinary_text(
            options(),
            7,
            Some(10),
            2_f64.to_bits(),
            0.04_f64.to_bits(),
            false
        )
        .unwrap()
        .hold,
        400
    );
}

#[test]
fn source_count_disabled_and_large_prompt_keep_distinct_estimates() {
    let mut off = options();
    off.count_token = false;
    let count = |o| ordinary_text(o, 6000, None, 1_f64.to_bits(), 1_f64.to_bits(), false).unwrap();
    assert_eq!(count(off).prompt_tokens, 0);
    assert_eq!(count(off).hold, 5000);
    assert_eq!(count(options()).hold, 6000);
}

#[test]
fn source_free_flag_is_not_inferred_from_zero_ratio() {
    assert_eq!(
        ordinary_text(options(), 7, None, 0_f64.to_bits(), 1_f64.to_bits(), false)
            .unwrap()
            .hold,
        1
    );
    assert_eq!(
        ordinary_text(options(), 7, None, 0_f64.to_bits(), 1_f64.to_bits(), true)
            .unwrap()
            .hold,
        0
    );
}

#[test]
fn tiered_defaults_and_reselected_group_use_original_base() {
    assert_eq!(tiered_text_completion(None, 1_f64.to_bits()), Ok(8192));
    assert_eq!(tiered_text_completion(Some(0), 1_f64.to_bits()), Ok(8192));
    assert_eq!(tiered_text_completion(Some(123), 1_f64.to_bits()), Ok(123));
    assert_eq!(tiered_text_completion(None, 0_f64.to_bits()), Ok(0));
    assert_eq!(
        refresh_tiered_group(10.1_f64.to_bits(), 0.5_f64.to_bits())
            .unwrap()
            .1,
        5
    );
    assert_eq!(
        refresh_tiered_group(10.1_f64.to_bits(), 0.25_f64.to_bits())
            .unwrap()
            .1,
        3
    );
    assert_eq!(
        refresh_tiered_group(40.5_f64.to_bits(), 1_f64.to_bits())
            .unwrap()
            .1,
        41
    );
}

#[test]
fn strict_precharge_rejects_overflow_instead_of_saturating() {
    assert_eq!(
        ordinary_text(
            options(),
            1,
            None,
            f64::INFINITY.to_bits(),
            1_f64.to_bits(),
            false
        ),
        Err(Error::Invalid)
    );
    assert_eq!(
        ordinary_text(
            options(),
            1,
            None,
            (-1_f64).to_bits(),
            1_f64.to_bits(),
            false
        ),
        Err(Error::Invalid)
    );
    assert_eq!(
        refresh_tiered_group((i32::MAX as f64 + 1.0).to_bits(), 1_f64.to_bits()),
        Err(Error::QuotaRange)
    );
}

#[test]
fn source_env_boolean_is_case_sensitive_and_does_not_trim() {
    for v in ["1", "t", "T", "TRUE", "true", "True"] {
        assert!(source_bool(Some(v), false));
    }
    for v in ["0", "f", "F", "FALSE", "false", "False"] {
        assert!(!source_bool(Some(v), true));
    }
    assert!(source_bool(Some(" false "), true));
    assert!(!source_bool(Some("TrUe"), false));
    assert!(source_bool(None, true));
    assert!(!source_bool(Some(""), false));
}

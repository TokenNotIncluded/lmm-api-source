#[test]
fn invitation_codes_preserve_legacy_case_without_broadening_native_codes() {
    for code in ["Ab09", "aB09", "0000", "0123456789abcdef0123456789abcdef"] {
        assert!(super::valid_invitation_code(code), "{code}");
    }
    for code in [
        "",
        "abc",
        "abcde",
        "Ab_9",
        " Ab9",
        "é123",
        "００００",
        "ABCDEF0123456789abcdef0123456789",
        "0123456789abcdef0123456789abcdeg",
    ] {
        assert!(!super::valid_invitation_code(code), "{code}");
    }
}

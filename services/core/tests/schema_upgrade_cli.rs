//! Real CLI argument/input checks. No configuration or database is opened.
#[path = "common/core_binary.rs"]
mod core_binary_input;

use std::{
    io::Write,
    process::{Command, Output, Stdio},
};
fn invoke(args: &[&str], input: &[u8]) -> Output {
    let mut child = Command::new(core_binary_input::path())
        .args(args)
        .env_clear()
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    if !input.is_empty() {
        child.stdin.take().unwrap().write_all(input).unwrap();
    }
    child.wait_with_output().unwrap()
}
#[test]
fn upgrade_cli_requires_explicit_config_and_never_legacy_environment() {
    for (args, expected) in [
        (vec!["upgrade-db"], "upgrade-db requires explicit"),
        (vec!["upgrade-db", "--legacy-env"], "Usage: lmm-core"),
        (vec!["--legacy-env", "upgrade-db"], "Usage: lmm-core"),
    ] {
        let output = invoke(&args, b"");
        assert!(!output.status.success());
        assert!(String::from_utf8_lossy(&output.stderr).contains(expected));
        assert!(output.stdout.is_empty());
    }
}
#[test]
fn upgrade_cli_rejects_untrusted_json_before_reading_configuration() {
    let marker = "private-input-do-not-echo";
    for input in [
        format!("{{\"arbitrary_sql\":\"{marker}\"}}"),
        format!("{{\"password\":\"{marker}\""),
    ] {
        let output = invoke(
            &["upgrade-db", "--config", "/missing-must-not-be-read.toml"],
            input.as_bytes(),
        );
        assert!(!output.status.success());
        assert!(String::from_utf8_lossy(&output.stderr).contains("UPGRADE_DB_INPUT_INVALID"));
        assert!(!String::from_utf8_lossy(&output.stderr).contains(marker));
        assert!(output.stdout.is_empty());
    }
}
#[test]
fn upgrade_help_is_explicit_and_has_no_startup_side_effect() {
    let output = invoke(&["upgrade-db", "--help"], b"");
    assert!(output.status.success());
    assert!(output.stderr.is_empty());
    let help = String::from_utf8_lossy(&output.stdout);
    assert!(help.contains("Explicit offline exact-target schema upgrade"));
    assert!(help.contains("core-payment-gateways-v1"));
    assert!(help.contains("Additive 25->26 requires the preserved 26-component installer"));
    assert!(
        help.contains("Legacy 24->25 trust-pricing requires the preserved 25-component installer.")
    );
}

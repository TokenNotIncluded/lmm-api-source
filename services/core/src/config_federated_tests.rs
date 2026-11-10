//! AUTH configuration cases run with the existing targeted config tests. They
//! need no PostgreSQL, listener, external provider or real secret material.
use super::*;

const WEB: &str = "[identity.web]\norigin='https://auth.example.test'\nrefresh_key_file='/nonexistent-test/web-key'\nuser_agreement_enabled=false\nprivacy_policy_enabled=false\n";
const DATABASE: &str = "[database]\nurl_file='/nonexistent-test/database-url'\n";

#[test]
fn federated_enable_requires_database_native_web_and_keyring() {
    for (database, web, keyring, enabled, expected) in [
        (false, false, false, false, true),
        (true, true, false, false, true),
        (true, true, true, false, false),
        (false, true, true, true, false),
        (true, false, true, true, false),
        (true, true, false, true, false),
        (true, true, true, true, true),
    ] {
        let document = format!(
            "version=1\n{}{}[identity.federated_auth]\nenabled={enabled}\n{}",
            if database { DATABASE } else { "" },
            if web { WEB } else { "" },
            if keyring {
                "keyring_file='/nonexistent-test/federated-keyring'\n"
            } else {
                ""
            }
        );
        let result = parse_core_file(document.as_bytes());
        assert_eq!(
            result.is_ok(),
            expected,
            "database={database} web={web} keyring={keyring} enabled={enabled}: {:?}",
            result.err()
        );
    }
}

#[test]
fn federated_keyring_requires_stable_replay_key_file() {
    let base = "version=1\nactive_key_id='first'\n";
    let entry = "[[keys]]\nid='first'\nkey_file='/nonexistent-test/federated-encryption-key'\n";
    assert!(parse_document::<FederatedKeyringFile>(format!("{base}{entry}").as_bytes()).is_err());
    let document =
        format!("{base}replay_key_file='/nonexistent-test/federated-replay-key'\n{entry}");
    let ring = parse_document::<FederatedKeyringFile>(document.as_bytes()).unwrap();
    assert_eq!(
        ring.replay_key_file,
        PathBuf::from("/nonexistent-test/federated-replay-key")
    );
    assert!(
        parse_document::<FederatedKeyringFile>(
            document.replace("replay_key_file", "replay_key").as_bytes()
        )
        .is_err()
    );
}

#[test]
fn federated_environment_enable_and_key_must_agree() {
    const CASE: &str = "LMM_TEST_FEDERATED_CONFIG_CASE";
    if let Ok(case) = env::var(CASE) {
        initialize(None, true).unwrap();
        match case.as_str() {
            "default" | "off" => assert!(!federated_auth_configured().unwrap()),
            "implicit" | "on" => assert!(federated_auth_configured().unwrap()),
            "missing" | "disabled_key" | "invalid" => assert!(federated_auth_configured().is_err()),
            _ => panic!("unknown test case"),
        }
        return;
    }
    for (case, enabled, key) in [
        ("default", None, false),
        ("off", Some("false"), false),
        ("implicit", None, true),
        ("on", Some("true"), true),
        ("missing", Some("true"), false),
        ("disabled_key", Some("false"), true),
        ("invalid", Some("yes"), true),
    ] {
        let mut command = std::process::Command::new(env::current_exe().unwrap());
        command
            .args([
                "--exact",
                "config::federated_config_tests::federated_environment_enable_and_key_must_agree",
                "--nocapture",
            ])
            .env_clear()
            .env(CASE, case);
        if let Some(enabled) = enabled {
            command.env("LMM_CORE_FEDERATED_AUTH_ENABLED", enabled);
        }
        if key {
            command.env(
                "LMM_CORE_FEDERATED_AUTH_KEYRING_FILE",
                "/nonexistent-test/federated-keyring",
            );
        }
        let output = command.output().unwrap();
        assert!(
            output.status.success(),
            "case {case}: {}{}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        );
    }
}

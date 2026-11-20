use super::*;

const CLIENTS: [(&str, &str); 6] = [
    ("pi", "LMM_CORE_OAUTH2_PI_MAX_CREDITS"),
    ("dsh", "LMM_CORE_OAUTH2_DSH_MAX_CREDITS"),
    ("codewhale", "LMM_CORE_OAUTH2_CODEWHALE_MAX_CREDITS"),
    ("opencode", "LMM_CORE_OAUTH2_OPENCODE_MAX_CREDITS"),
    ("vscode", "LMM_CORE_OAUTH2_VSCODE_MAX_CREDITS"),
    ("zed", "LMM_CORE_OAUTH2_ZED_MAX_CREDITS"),
];

fn document(ceilings: &str, model: bool) -> String {
    format!(
        r#"version=1
[database]
url_file='/private/test-db'
[identity.web]
origin='https://provider.example.test'
refresh_key_file='/private/web-key'
user_agreement_enabled=false
privacy_policy_enabled=false
[identity.oauth2]
issuer='https://provider.example.test'
groups=['allowed']
access_ttl_seconds=900
family_ttl_seconds=86400
{ceilings}
{}"#,
        if model {
            "[model]\nconfig_file='/private/model'\n"
        } else {
            ""
        }
    )
}

#[test]
fn public_provider_ceilings_are_independent_and_default_off() {
    let disabled = parse_core_file(document("", false).as_bytes()).unwrap();
    for (_, key) in CLIENTS {
        assert!(!disabled.contains_key(key));
    }
    for (name, key) in CLIENTS {
        let values =
            parse_core_file(document(&format!("{name}_max_credits=17"), true).as_bytes()).unwrap();
        assert_eq!(values[key], OsString::from("17"));
        for (_, other) in CLIENTS {
            if other != key {
                assert!(!values.contains_key(other));
            }
        }
        assert!(!values.contains_key("LMM_CORE_OAUTH2_CC_SWITCH_MAX_CREDITS"));
        assert!(!values.contains_key("LMM_CORE_OAUTH2_MAGPIE_MAX_CREDITS"));
        assert!(!values.contains_key("LMM_CORE_OAUTH2_MANAGED_REFRESH_KEYRING_FILE"));
    }
    let ceilings = CLIENTS
        .iter()
        .enumerate()
        .map(|(i, (name, _))| format!("{name}_max_credits={}\n", i + 1))
        .collect::<String>();
    let combined = parse_core_file(document(&ceilings, true).as_bytes()).unwrap();
    for (i, (_, key)) in CLIENTS.iter().enumerate() {
        assert_eq!(combined[*key], OsString::from((i + 1).to_string()));
    }
}

#[test]
fn public_provider_ceilings_require_finite_values_and_a_model_service() {
    for (name, _) in CLIENTS {
        for ceiling in [0, -1, 9_007_199_254_740_992, i64::MAX] {
            assert!(
                parse_core_file(
                    document(&format!("{name}_max_credits={ceiling}"), true).as_bytes()
                )
                .is_err()
            );
        }
        assert!(
            parse_core_file(document(&format!("{name}_max_credits=1"), false).as_bytes()).is_err()
        );
        assert!(
            parse_core_file(
                document(&format!("{name}_max_credits=9007199254740991"), true).as_bytes()
            )
            .is_ok()
        );
        assert!(
            parse_core_file(
                document(&format!("{name}_max_credits=1\n{name}_max_credits=2"), true).as_bytes()
            )
            .is_err()
        );
    }
    for invalid in [
        "unknown_max_credits=1",
        "DSH_max_credits=1",
        "dsh_max_credits=1.5",
        "dsh_max_credits='1'",
    ] {
        assert!(parse_core_file(document(invalid, true).as_bytes()).is_err());
    }
}

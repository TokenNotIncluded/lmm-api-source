//! Shared registration capability for public catalogs and account creation.
pub(crate) struct OAuthRegistrationPolicy<'a> {
    pub enabled: bool,
    pub disabled_methods: &'a [String],
}

impl OAuthRegistrationPolicy<'_> {
    pub(crate) fn allows(&self, method: &str) -> bool {
        self.enabled
            && !self
                .disabled_methods
                .iter()
                .any(|disabled| disabled == method)
    }

    pub(crate) fn allows_federated(&self, id: i64, kind: &str, slug: &str) -> bool {
        self.allows(kind)
            && self.allows(&format!("federated:{id}"))
            && (kind != "custom_oauth" || self.allows(&format!("custom:{slug}")))
    }
}

pub(crate) fn valid_method(method: &str) -> bool {
    matches!(
        method,
        "google"
            | "github"
            | "linuxdo"
            | "discord"
            | "telegram"
            | "oidc"
            | "wechat"
            | "custom_oauth"
    ) || method.strip_prefix("custom:").is_some_and(|slug| {
        !slug.is_empty()
            && slug.len() <= 64
            && slug
                .bytes()
                .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-')
    }) || method.strip_prefix("federated:").is_some_and(|raw| {
        raw.parse::<i64>()
            .is_ok_and(|id| id > 0 && id.to_string() == raw)
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn valid_methods_preserve_legacy_selectors_and_require_canonical_positive_ids() {
        for method in [
            "google",
            "github",
            "linuxdo",
            "discord",
            "telegram",
            "oidc",
            "wechat",
            "custom_oauth",
            "custom:example-login",
            "federated:1",
            "federated:9223372036854775807",
        ] {
            assert!(valid_method(method), "{method}");
        }
        assert!(valid_method(&format!("custom:{}", "a".repeat(64))));
        assert!(!valid_method(&format!("custom:{}", "a".repeat(65))));
        for method in [
            "",
            "unknown",
            "GOOGLE",
            "custom:",
            "custom:Example",
            "custom:非ASCII",
            "federated:0",
            "federated:-1",
            "federated:+1",
            "federated:01",
            "federated:",
            "federated:9223372036854775808",
            "federated:1.0",
        ] {
            assert!(!valid_method(method), "{method}");
        }
    }

    #[test]
    fn effective_global_gate_controls_every_registration_method() {
        for enabled in [false, true] {
            let policy = OAuthRegistrationPolicy {
                enabled,
                disabled_methods: &[],
            };
            assert_eq!(policy.allows("google"), enabled);
            assert_eq!(policy.allows_federated(42, "oidc", "oidc"), enabled);
            assert_eq!(policy.allows_federated(43, "wechat", "wechat"), enabled);
            assert_eq!(
                policy.allows_federated(44, "custom_oauth", "example"),
                enabled
            );
        }
    }

    #[test]
    fn exact_id_restriction_preserves_other_providers_and_builtin_methods() {
        let disabled = ["federated:42".into()];
        let policy = OAuthRegistrationPolicy {
            enabled: true,
            disabled_methods: &disabled,
        };
        for (kind, slug) in [
            ("oidc", "oidc"),
            ("wechat", "wechat"),
            ("custom_oauth", "example"),
        ] {
            assert!(!policy.allows_federated(42, kind, slug), "{kind}");
            assert!(policy.allows_federated(43, kind, slug), "{kind}");
        }
        assert!(policy.allows("google"));
    }

    #[test]
    fn kind_and_custom_slug_restrictions_preserve_their_scope() {
        let disabled = ["custom_oauth".into(), "oidc".into()];
        let policy = OAuthRegistrationPolicy {
            enabled: true,
            disabled_methods: &disabled,
        };
        assert!(!policy.allows_federated(42, "custom_oauth", "example"));
        assert!(!policy.allows_federated(43, "custom_oauth", "other"));
        assert!(!policy.allows_federated(44, "oidc", "oidc"));
        assert!(policy.allows_federated(45, "wechat", "wechat"));
        assert!(policy.allows("github"));

        let disabled = [
            "custom:example".into(),
            "custom:oidc".into(),
            "custom:wechat".into(),
        ];
        let policy = OAuthRegistrationPolicy {
            enabled: true,
            disabled_methods: &disabled,
        };
        assert!(!policy.allows_federated(42, "custom_oauth", "example"));
        assert!(policy.allows_federated(43, "custom_oauth", "other"));
        assert!(policy.allows_federated(44, "oidc", "oidc"));
        assert!(policy.allows_federated(45, "wechat", "wechat"));
    }

    #[test]
    fn combined_restrictions_apply_without_overriding_each_other() {
        let disabled = [
            "google".into(),
            "federated:42".into(),
            "custom:locked".into(),
            "oidc".into(),
        ];
        let policy = OAuthRegistrationPolicy {
            enabled: true,
            disabled_methods: &disabled,
        };
        assert!(!policy.allows("google"));
        assert!(policy.allows("github"));
        assert!(!policy.allows_federated(42, "custom_oauth", "other"));
        assert!(!policy.allows_federated(43, "custom_oauth", "locked"));
        assert!(policy.allows_federated(43, "custom_oauth", "other"));
        assert!(!policy.allows_federated(44, "oidc", "oidc"));
        assert!(policy.allows_federated(45, "wechat", "wechat"));
    }
}

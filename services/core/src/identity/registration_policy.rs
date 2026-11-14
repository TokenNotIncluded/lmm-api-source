//! Shared registration capability for public catalogs and account creation.
use super::{IdentityStore, federated::ProviderKind};

pub(crate) struct OAuthRegistrationPolicy<'a> {
    pub enabled: bool,
    pub disabled_methods: &'a [String],
}

impl OAuthRegistrationPolicy<'_> {
    pub(crate) fn allows(&self, method: &str) -> bool {
        self.enabled && !self.disabled_methods.iter().any(|disabled| disabled == method)
    }

    pub(crate) fn allows_federated(&self, id: i64, kind: ProviderKind, slug: &str) -> bool {
        self.allows(kind.as_str())
            && self.allows(&format!("federated:{id}"))
            && (kind != ProviderKind::CustomOauth || self.allows(&format!("custom:{slug}")))
    }
}

impl IdentityStore {
    pub(crate) fn oauth_registration_policy(&self) -> OAuthRegistrationPolicy<'_> {
        OAuthRegistrationPolicy {
            enabled: self.registration_enabled && self.oauth_register_enabled,
            disabled_methods: &self.oauth_registration_disabled_methods,
        }
    }
}

pub(crate) fn valid_method(method: &str) -> bool {
    matches!(
        method,
        "google" | "github" | "linuxdo" | "discord" | "telegram" | "oidc" | "wechat" | "custom_oauth"
    ) || method.strip_prefix("custom:").is_some_and(|slug| {
        !slug.is_empty()
            && slug.len() <= 64
            && slug.bytes().all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-')
    }) || method.strip_prefix("federated:").is_some_and(|raw| {
        raw.parse::<i64>().is_ok_and(|id| id > 0 && id.to_string() == raw)
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn registration_policy_accepts_precise_ids_and_preserves_legacy_selectors() {
        let pool = sqlx::postgres::PgPoolOptions::new()
            .connect_lazy("postgres://unused@127.0.0.1:1/unused")
            .unwrap();
        let base = IdentityStore::from_pool(pool)
            .with_registration_enabled(true)
            .with_oauth_register_enabled(true);
        let restricted = base.clone().with_oauth_registration_disabled_methods(&[
            " FEDERATED:42 ".into(), "federated:42".into(), "custom:example".into(), "custom_oauth".into(),
        ]).unwrap();
        assert_eq!(restricted.oauth_registration_disabled_methods(), &["custom:example", "custom_oauth", "federated:42"]);
        for invalid in ["federated:0", "federated:-1", "federated:+1", "federated:01", "federated:", "federated:9223372036854775808", "federated:1.0"] {
            assert!(base.clone().with_oauth_registration_disabled_methods(&[invalid.into()]).is_err(), "{invalid}");
        }
        for selector in ["oidc", "wechat", "custom_oauth", "custom:example", "federated:42"] {
            let store = base.clone().with_oauth_registration_disabled_methods(&[selector.into()]).unwrap();
            let policy = store.oauth_registration_policy();
            for (kind, slug) in [(ProviderKind::Oidc, "oidc"), (ProviderKind::Wechat, "wechat"), (ProviderKind::CustomOauth, "example")] {
                let denied = selector == kind.as_str() || selector == "federated:42" || (kind == ProviderKind::CustomOauth && selector == "custom:example");
                assert_eq!(policy.allows_federated(42, kind, slug), !denied, "{selector}, {kind:?}");
            }
            assert!(policy.allows("google"));
        }
        for store in [base.clone().with_registration_enabled(false), base.with_oauth_register_enabled(false)] {
            assert!(!store.oauth_registration_policy().allows("google"));
            assert!(!store.oauth_registration_policy().allows_federated(42, ProviderKind::CustomOauth, "example"));
        }
    }
}

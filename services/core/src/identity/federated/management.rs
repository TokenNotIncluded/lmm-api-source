use super::{ProviderConfiguration, ProviderKind};
use crate::identity::{IdentityError, Result};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::BTreeMap;
use zeroize::Zeroize;

// Never Debug; the one secret field is wiped when the request is dropped.
#[derive(Default, Deserialize, Serialize)]
pub(crate) struct ProviderWrite {
    #[serde(default)]
    pub operation_key: String,
    #[serde(default)]
    pub expected_revision: Option<String>,
    #[serde(default)]
    pub id: Option<String>,
    #[serde(default)]
    pub kind: Option<ProviderKind>,
    #[serde(default)]
    pub name: Option<String>,
    #[serde(default)]
    pub slug: Option<String>,
    #[serde(default)]
    pub icon: Option<String>,
    #[serde(default)]
    pub enabled: Option<bool>,
    #[serde(alias = "server_token")]
    #[serde(default)]
    pub client_secret: String,
    #[serde(flatten)]
    #[serde(default)]
    pub changes: BTreeMap<String, Value>,
}
impl Drop for ProviderWrite {
    fn drop(&mut self) {
        self.client_secret.zeroize();
    }
}

impl ProviderWrite {
    pub(crate) fn validate_operation(&self) -> Result<()> {
        if !(16..=128).contains(&self.operation_key.len())
            || !self
                .operation_key
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'_' | b'-'))
        {
            return Err(IdentityError::Invalid);
        }
        Ok(())
    }

    pub(crate) fn configuration(
        &self,
        current: &ProviderConfiguration,
    ) -> Result<ProviderConfiguration> {
        let mut current = serde_json::to_value(current).map_err(|_| IdentityError::Storage)?;
        for (key, value) in &self.changes {
            let original = current.get_mut(key).ok_or(IdentityError::Invalid)?;
            if !(original.is_string() && value.is_string()
                || original.is_boolean() && value.is_boolean()
                || original.is_array() && value.is_array()
                || original.is_number() && value.is_number())
            {
                return Err(IdentityError::Invalid);
            }
            if value.as_str() == Some("")
                && !matches!(
                    key.as_str(),
                    "well_known" | "access_policy" | "access_denied_message" | "qr_code_url"
                )
            {
                continue;
            }
            *original = value.clone();
        }
        serde_json::from_value(current).map_err(|_| IdentityError::Invalid)
    }
}

pub(crate) fn names(kind: ProviderKind, name: &str, slug: &str, icon: &str) -> Result<()> {
    let reserved = [
        "oidc", "wechat", "github", "discord", "linuxdo", "telegram", "email", "state", "passkey",
    ];
    if name.is_empty()
        || name.chars().count() > 64
        || icon.chars().count() > 128
        || slug.is_empty()
        || slug.len() > 64
        || !slug.as_bytes()[0].is_ascii_alphanumeric()
        || !slug
            .bytes()
            .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-')
        || match kind {
            ProviderKind::Oidc => slug != "oidc",
            ProviderKind::Wechat => slug != "wechat",
            ProviderKind::CustomOauth => reserved.contains(&slug),
        }
    {
        return Err(IdentityError::Invalid);
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn legacy_patch_preserves_empty_required_fields_but_clears_optional_policy() {
        let current = ProviderConfiguration {
            client_id: "prior".into(),
            access_policy: "prior-policy".into(),
            auth_style: 2,
            ..Default::default()
        };
        let patch: ProviderWrite = serde_json::from_value(
            serde_json::json!({"client_id":"","access_policy":"","auth_style":0}),
        )
        .unwrap();
        let next = patch.configuration(&current).unwrap();
        assert_eq!(next.client_id, "prior");
        assert_eq!(next.access_policy, "");
        assert_eq!(next.auth_style, 0);
        let unknown: ProviderWrite =
            serde_json::from_value(serde_json::json!({"sealed_secret":"attacker"})).unwrap();
        assert!(unknown.configuration(&current).is_err());
    }
}

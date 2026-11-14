use super::policy;
use crate::identity::{IdentityError, Result};
use serde::{Deserialize, Serialize};
use serde_json::Value;

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum ProviderKind {
    Oidc,
    Wechat,
    CustomOauth,
}

impl ProviderKind {
    pub(crate) fn as_str(self) -> &'static str {
        match self {
            Self::Oidc => "oidc",
            Self::Wechat => "wechat",
            Self::CustomOauth => "custom_oauth",
        }
    }
    pub(crate) fn parse(value: &str) -> Result<Self> {
        match value {
            "oidc" => Ok(Self::Oidc),
            "wechat" => Ok(Self::Wechat),
            "custom_oauth" => Ok(Self::CustomOauth),
            _ => Err(IdentityError::Invalid),
        }
    }
}

/// Only non-secret protocol configuration is serialized or placed in JSONB.
#[derive(Clone, Default, Deserialize, Serialize)]
#[serde(default, deny_unknown_fields)]
pub(crate) struct ProviderConfiguration {
    pub client_id: String,
    pub authorization_endpoint: String,
    pub token_endpoint: String,
    pub user_info_endpoint: String,
    pub scopes: String,
    pub user_id_field: String,
    pub username_field: String,
    pub display_name_field: String,
    pub email_field: String,
    pub well_known: String,
    pub auth_style: u8,
    pub access_policy: String,
    pub access_denied_message: String,
    pub issuer: String,
    pub jwks_uri: String,
    pub id_token_algorithms: Vec<String>,
    pub pkce_s256: bool,
    pub bridge_url: String,
    pub qr_code_url: String,
}

fn mapped_string(raw: &str, path: &str) -> String {
    let value = gjson::get(raw, path);
    if value.str().is_empty() && value.exists() {
        value.json().trim_matches('"').to_owned()
    } else {
        value.str().to_owned()
    }
}
fn legacy_bool(raw: &str, path: &str) -> bool {
    let value = gjson::get(raw, path);
    if value.kind() == gjson::Kind::Number {
        value.f64() != 0.0
    } else {
        value.bool()
    }
}

impl ProviderConfiguration {
    pub(crate) fn normalize(&mut self, kind: ProviderKind) -> Result<()> {
        if kind == ProviderKind::Wechat {
            if self.bridge_url.is_empty() {
                return Err(IdentityError::Invalid);
            }
            return Ok(());
        }
        if self.client_id.is_empty()
            || self.client_id.len() > 1024
            || self.auth_style > 2
            || self.scopes.len() > 1024
            || self.access_denied_message.chars().count() > 512
        {
            return Err(IdentityError::Invalid);
        }
        if self.scopes.is_empty() {
            self.scopes = "openid profile email".into();
        }
        for (path, default) in [
            (&mut self.user_id_field, "sub"),
            (&mut self.username_field, "preferred_username"),
            (&mut self.display_name_field, "name"),
            (&mut self.email_field, "email"),
        ] {
            if path.is_empty() {
                *path = default.into();
            }
            if path.len() > 512 {
                return Err(IdentityError::Invalid);
            }
            policy::validate_path(path)?;
        }
        policy::parse(&self.access_policy)?;
        if kind == ProviderKind::Oidc
            && (self.issuer.is_empty()
                || self.jwks_uri.is_empty()
                || self.id_token_algorithms.is_empty()
                || self.id_token_algorithms.len() > 16)
        {
            return Err(IdentityError::Invalid);
        }
        Ok(())
    }

    pub(super) fn mapped_user(&self, raw: &str) -> Result<MappedUser> {
        policy::bounded_json(raw)?;
        for path in [
            &self.user_id_field,
            &self.username_field,
            &self.display_name_field,
            &self.email_field,
        ] {
            policy::validate_path(path)?;
        }
        let subject = mapped_string(raw, &self.user_id_field);
        if subject.is_empty() || subject.len() > 65536 || subject.contains('\0') {
            return Err(IdentityError::Unauthorized);
        }
        let email = mapped_string(raw, &self.email_field);
        let email_verified = !email.trim().is_empty()
            && ["email_verified", "emailVerified", "verified_email"]
                .iter()
                .any(|path| legacy_bool(raw, path));
        Ok(MappedUser {
            subject,
            username: mapped_string(raw, &self.username_field),
            display_name: mapped_string(raw, &self.display_name_field),
            email,
            email_verified,
        })
    }
}

// Not a proof: protocol code must verify the token/userinfo/bridge first.
pub(super) struct MappedUser {
    pub subject: String,
    pub username: String,
    pub display_name: String,
    pub email: String,
    pub email_verified: bool,
}

pub(crate) struct FederatedProvider {
    pub id: i64,
    pub kind: ProviderKind,
    pub slug: String,
    pub name: String,
    pub icon: String,
    pub enabled: bool,
    pub configuration: ProviderConfiguration,
    pub secret_context: Vec<u8>,
    pub secret_key_id: String,
    pub sealed_secret: Vec<u8>,
    pub revision: i64,
    pub deleted: bool,
}

impl FederatedProvider {
    pub(crate) fn descriptor(&self) -> Value {
        serde_json::json!({"id":self.id.to_string(),"kind":self.kind,"slug":self.slug,"name":self.name,"icon":self.icon,"enabled":self.enabled && !self.deleted,"revision":self.revision.to_string()})
    }

    pub(crate) fn admin_descriptor(&self) -> Result<Value> {
        let mut result = self.descriptor();
        let Value::Object(fields) =
            serde_json::to_value(&self.configuration).map_err(|_| IdentityError::Storage)?
        else {
            return Err(IdentityError::Storage);
        };
        result
            .as_object_mut()
            .ok_or(IdentityError::Storage)?
            .extend(fields);
        // Empty on update preserves the existing encrypted secret. No secret
        // value, key label or ciphertext is included in any response.
        result["client_secret"] = Value::String(String::new());
        result["secret_configured"] = Value::Bool(true);
        Ok(result)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn mapping_keeps_raw_integer_ids_and_fractional_boolean_truth() {
        let mut configuration = ProviderConfiguration {
            client_id: "client".into(),
            ..ProviderConfiguration::default()
        };
        configuration.normalize(ProviderKind::CustomOauth).unwrap();
        let mapped=configuration.mapped_user(r#"{"sub":9007199254740993,"preferred_username":7,"name":true,"email":"user@example.test","email_verified":0.5}"#).unwrap();
        assert_eq!(mapped.subject, concat!("9007199254", "740993"));
        assert_eq!(mapped.username, "7");
        assert_eq!(mapped.display_name, "true");
        assert!(mapped.email_verified);
        assert!(
            configuration
                .mapped_user(&format!("{}{}", "[".repeat(129), "]".repeat(129)))
                .is_err()
        );
    }
}

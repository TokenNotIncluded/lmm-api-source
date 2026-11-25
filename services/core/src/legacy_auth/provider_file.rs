//! Explicit local OAuth provider changes. Serving Core never applies these files.
use super::{BrowserAuth, BrowserAuthError, IdentityError, IdentityStore, LegacyAuthConfig};
use crate::identity::federated::ProviderWrite;
use serde::Deserialize;
use serde_json::{Value, json};
use std::{
    io::{self, Read},
    path::{Path, PathBuf},
};
use zeroize::Zeroizing;

#[derive(Deserialize)]
#[serde(rename_all = "lowercase")]
enum Change {
    Create,
    Update,
    Delete,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct FederatedProviderFile {
    action: Change,
    session_file: PathBuf,
    client_secret_file: Option<PathBuf>,
    provider: ProviderWrite,
}

fn invalid() -> io::Error {
    io::Error::new(
        io::ErrorKind::InvalidInput,
        "invalid federated provider TOML",
    )
}

fn secret(path: &Path) -> io::Result<String> {
    let bytes = Zeroizing::new(crate::config::private_file(path, 8192)?);
    let raw = std::str::from_utf8(&bytes).map_err(|_| invalid())?;
    // A single trailing LF or CRLF is conventional in an operator-created file.
    let raw = raw
        .strip_suffix("\r\n")
        .or_else(|| raw.strip_suffix('\n'))
        .unwrap_or(raw);
    if raw.is_empty() || raw.chars().any(char::is_control) {
        return Err(invalid());
    }
    Ok(raw.to_owned())
}

impl FederatedProviderFile {
    pub fn read(input: impl Read) -> io::Result<Self> {
        const LIMIT: u64 = 128 * 1024;
        let mut text = Zeroizing::new(String::new());
        input
            .take(LIMIT + 1)
            .read_to_string(&mut text)
            .map_err(|_| invalid())?;
        if text.len() as u64 > LIMIT {
            return Err(invalid());
        }
        let operation: Self = toml::from_str(&text).map_err(|_| invalid())?;
        operation
            .provider
            .validate_operation()
            .map_err(|_| invalid())?;
        if !operation.session_file.is_absolute()
            || !operation.provider.client_secret.is_empty()
            || operation
                .client_secret_file
                .as_ref()
                .is_some_and(|p| !p.is_absolute())
        {
            return Err(invalid());
        }
        let id = operation.target()?;
        match operation.action {
            Change::Create if id.is_some() || operation.provider.expected_revision.is_some() => {
                return Err(invalid());
            }
            Change::Update | Change::Delete
                if id.is_none() || operation.provider.expected_revision.is_none() =>
            {
                return Err(invalid());
            }
            Change::Delete
                if operation.client_secret_file.is_some()
                    || !operation.provider.changes.is_empty()
                    || operation.provider.kind.is_some()
                    || operation.provider.enabled.is_some()
                    || operation.provider.name.is_some()
                    || operation.provider.slug.is_some()
                    || operation.provider.icon.is_some() =>
            {
                return Err(invalid());
            }
            _ => {}
        }
        Ok(operation)
    }

    fn target(&self) -> io::Result<Option<i64>> {
        self.provider
            .id
            .as_deref()
            .map(|raw| {
                let id = raw.parse::<i64>().map_err(|_| invalid())?;
                if id <= 0 || id.to_string() != raw {
                    return Err(invalid());
                }
                Ok(id)
            })
            .transpose()
    }

    pub async fn apply(
        mut self,
        store: IdentityStore,
        config: &LegacyAuthConfig,
    ) -> io::Result<Value> {
        let target = self.target()?;
        let action = match self.action {
            Change::Create => "create",
            Change::Update => "update",
            Change::Delete => "delete",
        };
        let access = Zeroizing::new(secret(&self.session_file)?);
        if let Some(path) = &self.client_secret_file {
            self.provider.client_secret = secret(path)?;
        }
        let browser = BrowserAuth::new(store, &config.refresh_key).map_err(io::Error::other)?;
        let result = browser
            .mutate_federated_provider(&access, action, target, false, &self.provider)
            .await
            .map_err(|err| match err {
                BrowserAuthError::Identity(IdentityError::Storage) => io::Error::other(
                    "provider outcome unknown; reconcile the original operation before retrying",
                ),
                BrowserAuthError::Identity(err) => io::Error::other(err),
                _ => io::Error::new(
                    io::ErrorKind::PermissionDenied,
                    "a recent superadministrator session is required",
                ),
            })?;
        Ok(
            json!({"id":result["id"], "revision":result["applied_revision"], "replayed":result["replayed"]}),
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    const EXAMPLE: &str = r#"
action = "update"
session_file = "/run/secrets/admin-session"
[provider]
id = "17"
operation_key = "local-provider-00017"
expected_revision = "3"
enabled = false
"#;

    #[test]
    fn provider_changes_require_file_references_and_exact_target() {
        let request = FederatedProviderFile::read(EXAMPLE.as_bytes()).unwrap();
        assert_eq!(request.target().unwrap(), Some(17));
        for bad in [
            EXAMPLE.replace("action = \"update\"", "action = \"create\""),
            EXAMPLE.replace("id = \"17\"", "id = \"017\""),
            EXAMPLE.replace("/run/secrets/admin-session", "relative"),
            EXAMPLE.replace("expected_revision", "expected_revisions"),
            format!("{EXAMPLE}\nclient_secret = \"secret\""),
        ] {
            assert!(FederatedProviderFile::read(bad.as_bytes()).is_err());
        }
    }
}

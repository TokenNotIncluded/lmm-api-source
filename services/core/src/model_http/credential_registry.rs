//! Immutable credential revisions shared by current and in-flight releases.
//! Publication may append a revision; it cannot change material behind a ref.
use super::{RelayError, UpstreamCredential};
use std::{collections::BTreeMap, sync::RwLock};
use subtle::ConstantTimeEq;

pub(super) struct CredentialRegistry(RwLock<BTreeMap<(u64, u64), String>>);
impl CredentialRegistry {
    pub(super) fn new(values: BTreeMap<(u64, u64), String>) -> Self {
        Self(RwLock::new(values))
    }
    pub(super) fn get(&self, reference: &(u64, u64)) -> Option<String> {
        self.0.read().ok()?.get(reference).cloned()
    }
    pub(super) fn contains_key(&self, reference: &(u64, u64)) -> bool {
        self.0
            .read()
            .is_ok_and(|values| values.contains_key(reference))
    }
    pub(super) fn snapshot(&self) -> Result<BTreeMap<(u64, u64), String>, RelayError> {
        self.0
            .read()
            .map(|values| values.clone())
            .map_err(|_| RelayError::RouteUnavailable)
    }
    pub(super) fn contains_secret(&self, secret: &str) -> bool {
        let Ok(values) = self.0.read() else {
            return true;
        };
        let mut found = subtle::Choice::from(0);
        for value in values.values() {
            found |= value.as_bytes().ct_eq(secret.as_bytes());
        }
        bool::from(found)
    }
    pub(super) fn append(&self, credentials: Vec<UpstreamCredential>) -> Result<(), RelayError> {
        let incoming = super::routing::credential_map(credentials)?;
        if incoming
            .values()
            .any(|secret| reqwest::header::HeaderValue::from_str(secret).is_err())
        {
            return Err(RelayError::Invalid("invalid upstream credential"));
        }
        let mut values = self.0.write().map_err(|_| RelayError::RouteUnavailable)?;
        if values
            .len()
            .checked_add(incoming.len())
            .is_none_or(|n| n > 100_000)
            || incoming.iter().any(|(reference, secret)| {
                values
                    .get(reference)
                    .is_some_and(|old| !bool::from(old.as_bytes().ct_eq(secret.as_bytes())))
            })
        {
            return Err(RelayError::Conflict);
        }
        values.extend(incoming);
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::routing::CredentialRef;
    #[test]
    fn immutable_revisions_preserve_old_material_and_reject_overwrites_atomically() {
        let registry = CredentialRegistry::new([((1, 1), "old-key".into())].into());
        registry
            .append(vec![UpstreamCredential {
                reference: CredentialRef { id: 1, revision: 2 },
                secret: "new-key".into(),
            }])
            .unwrap();
        assert_eq!(registry.get(&(1, 1)).as_deref(), Some("old-key"));
        assert_eq!(registry.get(&(1, 2)).as_deref(), Some("new-key"));
        assert!(
            registry
                .append(vec![
                    UpstreamCredential {
                        reference: CredentialRef { id: 1, revision: 1 },
                        secret: "replacement".into()
                    },
                    UpstreamCredential {
                        reference: CredentialRef { id: 2, revision: 1 },
                        secret: "must-not-appear".into()
                    }
                ])
                .is_err()
        );
        assert!(!registry.contains_key(&(2, 1)));
        assert!(registry.contains_secret("old-key"));
    }
}

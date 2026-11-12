//! Same pinned AES-256-GCM primitive as identity/mfa/crypto.rs,
//! with an independent key file and mail-only domain separation.
use super::{MailError, Result};
use aes_gcm::{
    Aes256Gcm, KeyInit, Nonce,
    aead::{Aead, Payload},
};
use rand::{RngCore, rngs::OsRng};
use std::{collections::HashMap, io::Read, path::Path};
use zeroize::Zeroizing;

const MAX_PAYLOAD: usize = 2048;

/// No Debug/serialization. Retain previous keys until their payloads are erased.
pub struct MailKeys {
    active: String,
    keys: HashMap<String, Zeroizing<[u8; 32]>>,
}

fn label(id: &str) -> bool {
    (1..=64).contains(&id.len())
        && id
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b"-_".contains(&b))
}

/// Private raw 32-byte key file; never follows a symlink or prints its path.
pub(crate) fn read_private(path: &Path, limit: u64) -> Result<Zeroizing<Vec<u8>>> {
    if !path.is_absolute()
        || path
            .components()
            .any(|p| matches!(p, std::path::Component::ParentDir))
    {
        return Err(MailError::Invalid);
    }
    let mut prefix = std::path::PathBuf::new();
    for part in path.components() {
        prefix.push(part);
        if std::fs::symlink_metadata(&prefix)
            .map_err(|_| MailError::Invalid)?
            .file_type()
            .is_symlink()
        {
            return Err(MailError::Invalid);
        }
    }
    let before = std::fs::symlink_metadata(path).map_err(|_| MailError::Invalid)?;
    if !before.is_file() || before.len() > limit {
        return Err(MailError::Invalid);
    }
    let file = std::fs::File::open(path).map_err(|_| MailError::Invalid)?;
    let after = file.metadata().map_err(|_| MailError::Invalid)?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::{MetadataExt, PermissionsExt};
        if before.dev() != after.dev()
            || before.ino() != after.ino()
            || !matches!(before.permissions().mode() & 0o777, 0o400 | 0o600)
            || !matches!(after.permissions().mode() & 0o777, 0o400 | 0o600)
        {
            return Err(MailError::Invalid);
        }
    }
    let mut secret = Zeroizing::new(Vec::new());
    file.take(limit + 1)
        .read_to_end(&mut secret)
        .map_err(|_| MailError::Invalid)?;
    if secret.is_empty() || secret.len() as u64 > limit {
        return Err(MailError::Invalid);
    }
    Ok(secret)
}

impl MailKeys {
    /// Keys must be independently generated, never refresh/proof/MFA keys.
    /// Every input is wiped even when validation fails.
    pub fn new(active: &str, keys: Vec<(String, Vec<u8>)>) -> Result<Self> {
        let keys: Vec<_> = keys
            .into_iter()
            .map(|(id, key)| (id, Zeroizing::new(key)))
            .collect();
        if !label(active) || !(1..=8).contains(&keys.len()) {
            return Err(MailError::Invalid);
        }
        let mut derived = HashMap::new();
        for (id, key) in keys {
            if !label(&id)
                || key.len() != 32
                || key.iter().all(|b| *b == 0)
                || derived.contains_key(&id)
            {
                return Err(MailError::Invalid);
            }
            let mut material = Zeroizing::new([0_u8; 32]);
            // This independently generated mail key is used directly by
            // AES-GCM; the dedicated key file already provides domain separation.
            material.copy_from_slice(&key);
            derived.insert(id, material);
        }
        if !derived.contains_key(active) {
            return Err(MailError::Invalid);
        }
        Ok(Self {
            active: active.into(),
            keys: derived,
        })
    }

    /// All files are raw bytes, not hex/base64 or an environment secret.
    pub fn from_files(active: &str, files: &[(String, std::path::PathBuf)]) -> Result<Self> {
        // Collect into wiping wrappers before any later read can fail.
        let mut keys = Vec::new();
        for (id, path) in files {
            keys.push((id.clone(), read_private(path, 32)?));
        }
        Self::new(
            active,
            keys.into_iter()
                .map(|(id, mut key)| (id, std::mem::take(&mut *key)))
                .collect(),
        )
    }

    pub(crate) fn active(&self) -> &str {
        &self.active
    }

    fn aad(id: &str, challenge: &str, kind: &str, key_id: &str) -> Vec<u8> {
        let mut aad = Vec::new();
        for field in [
            b"email-outbox-v1".as_slice(),
            id.as_bytes(),
            challenge.as_bytes(),
            kind.as_bytes(),
            key_id.as_bytes(),
        ] {
            aad.extend_from_slice(&(field.len() as u64).to_be_bytes());
            aad.extend_from_slice(field);
        }
        aad
    }

    pub(crate) fn encrypt(
        &self,
        id: &str,
        challenge: &str,
        kind: &str,
        plaintext: &[u8],
    ) -> Result<Vec<u8>> {
        if plaintext.is_empty() || plaintext.len() > MAX_PAYLOAD {
            return Err(MailError::Invalid);
        }
        let key = self.keys.get(&self.active).ok_or(MailError::Unavailable)?;
        let cipher =
            Aes256Gcm::new_from_slice(key.as_slice()).map_err(|_| MailError::Unavailable)?;
        let mut nonce = [0_u8; 12];
        OsRng
            .try_fill_bytes(&mut nonce)
            .map_err(|_| MailError::Unavailable)?;
        let encrypted = cipher
            .encrypt(
                Nonce::from_slice(&nonce),
                Payload {
                    msg: plaintext,
                    aad: &Self::aad(id, challenge, kind, &self.active),
                },
            )
            .map_err(|_| MailError::Unavailable)?;
        let mut envelope = vec![1];
        envelope.extend_from_slice(&nonce);
        envelope.extend_from_slice(&encrypted);
        Ok(envelope)
    }

    pub(crate) fn decrypt(
        &self,
        id: &str,
        challenge: &str,
        kind: &str,
        key_id: &str,
        envelope: &[u8],
    ) -> Result<Zeroizing<Vec<u8>>> {
        if !(30..=MAX_PAYLOAD + 29).contains(&envelope.len()) || envelope[0] != 1 {
            return Err(MailError::Unavailable);
        }
        let key = self.keys.get(key_id).ok_or(MailError::Unavailable)?;
        let plain = Aes256Gcm::new_from_slice(key.as_slice())
            .map_err(|_| MailError::Unavailable)?
            .decrypt(
                Nonce::from_slice(&envelope[1..13]),
                Payload {
                    msg: &envelope[13..],
                    aad: &Self::aad(id, challenge, kind, key_id),
                },
            )
            .map_err(|_| MailError::Unavailable)?;
        Ok(Zeroizing::new(plain))
    }
}

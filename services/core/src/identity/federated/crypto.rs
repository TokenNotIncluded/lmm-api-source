//! Dedicated provider secrets; never use MFA/session/provider credentials as keys.
use crate::identity::{IdentityError, Result};
use aes_gcm::{
    Aes256Gcm, KeyInit, Nonce,
    aead::{Aead, Payload},
};
use hkdf::Hkdf;
use hmac::{Hmac, Mac};
use rand::TryRng as _;
use rand::rngs::SysRng;
use sha2::Sha256;
use std::collections::BTreeMap;
use zeroize::Zeroizing;

const VERSION: u8 = 1;
const DOMAIN: &[u8] = b"lmm/core/federated-auth/v1";

// Deliberately neither Debug nor Serialize.
pub(crate) struct FederatedKeys {
    active: String,
    keys: BTreeMap<String, Zeroizing<[u8; 32]>>,
    claim_key: Zeroizing<[u8; 32]>,
}

impl FederatedKeys {
    pub(crate) fn new(
        active: String,
        inputs: Vec<(String, Vec<u8>)>,
        replay_key: Vec<u8>,
    ) -> Result<Self> {
        let inputs: Vec<_> = inputs
            .into_iter()
            .map(|(id, bytes)| (id, Zeroizing::new(bytes)))
            .collect();
        let replay_key = Zeroizing::new(replay_key);
        if replay_key.len() != 32 {
            return Err(IdentityError::Invalid);
        }
        let mut claim_key = Zeroizing::new([0_u8; 32]);
        Hkdf::<Sha256>::new(Some(DOMAIN), &replay_key)
            .expand(b"wechat-code-claims", &mut *claim_key)
            .map_err(|_| IdentityError::Storage)?;
        let mut keys = BTreeMap::new();
        if inputs.is_empty() || inputs.len() > 8 {
            return Err(IdentityError::Invalid);
        }
        for (id, bytes) in inputs {
            if id.is_empty()
                || id.len() > 64
                || !id
                    .bytes()
                    .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'_' | b'-'))
                || bytes.len() != 32
                || keys.contains_key(&id)
            {
                return Err(IdentityError::Invalid);
            }
            let mut key = Zeroizing::new([0_u8; 32]);
            Hkdf::<Sha256>::new(Some(DOMAIN), &bytes)
                .expand(b"provider-secrets", &mut *key)
                .map_err(|_| IdentityError::Storage)?;
            keys.insert(id, key);
        }
        if !keys.contains_key(&active) {
            return Err(IdentityError::Invalid);
        }
        Ok(Self {
            active,
            keys,
            claim_key,
        })
    }

    pub(crate) fn context() -> Result<[u8; 32]> {
        let mut context = [0_u8; 32];
        SysRng
            .try_fill_bytes(&mut context)
            .map_err(|_| IdentityError::Storage)?;
        Ok(context)
    }

    pub(crate) fn operation_digest(
        &self,
        action: &str,
        target: i64,
        request: &[u8],
    ) -> Result<Vec<u8>> {
        let mut mac = <Hmac<Sha256> as hmac::KeyInit>::new_from_slice(self.claim_key.as_ref())
            .map_err(|_| IdentityError::Storage)?;
        mac.update(b"lmm/federated/provider-operation/v1");
        mac.update(action.as_bytes());
        mac.update(&target.to_be_bytes());
        mac.update(request);
        Ok(mac.finalize().into_bytes().to_vec())
    }

    fn aad(context: &[u8], revision: i64, label: &str) -> Result<Vec<u8>> {
        if context.len() != 32 || revision <= 0 || label.len() > 64 {
            return Err(IdentityError::Invalid);
        }
        let mut aad = Vec::with_capacity(DOMAIN.len() + 105);
        aad.extend_from_slice(DOMAIN);
        aad.push(VERSION);
        aad.extend_from_slice(context);
        aad.extend_from_slice(&revision.to_be_bytes());
        aad.extend_from_slice(&(label.len() as u16).to_be_bytes());
        aad.extend_from_slice(label.as_bytes());
        Ok(aad)
    }

    pub(crate) fn seal(
        &self,
        context: &[u8],
        revision: i64,
        secret: &str,
    ) -> Result<(String, Vec<u8>)> {
        if secret.is_empty() || secret.len() > 65507 {
            return Err(IdentityError::Invalid);
        }
        let aad = Self::aad(context, revision, &self.active)?;
        let key = self.keys.get(&self.active).ok_or(IdentityError::Storage)?;
        let cipher = Aes256Gcm::new_from_slice(key.as_ref()).map_err(|_| IdentityError::Storage)?;
        let mut nonce = [0_u8; 12];
        SysRng
            .try_fill_bytes(&mut nonce)
            .map_err(|_| IdentityError::Storage)?;
        let ciphertext = cipher
            .encrypt(
                &Nonce::from(nonce),
                Payload {
                    msg: secret.as_bytes(),
                    aad: &aad,
                },
            )
            .map_err(|_| IdentityError::Storage)?;
        let mut envelope = Vec::with_capacity(13 + ciphertext.len());
        envelope.push(VERSION);
        envelope.extend_from_slice(&nonce);
        envelope.extend_from_slice(&ciphertext);
        Ok((self.active.clone(), envelope))
    }

    pub(crate) fn open(
        &self,
        context: &[u8],
        revision: i64,
        label: &str,
        envelope: &[u8],
    ) -> Result<Zeroizing<String>> {
        if !(29..=65536).contains(&envelope.len()) || envelope[0] != VERSION {
            return Err(IdentityError::Storage);
        }
        let key = self.keys.get(label).ok_or(IdentityError::Storage)?;
        let cipher = Aes256Gcm::new_from_slice(key.as_ref()).map_err(|_| IdentityError::Storage)?;
        let aad = Self::aad(context, revision, label)?;
        let bytes = Zeroizing::new(
            cipher
                .decrypt(
                    &Nonce::try_from(&envelope[1..13]).map_err(|_| IdentityError::Storage)?,
                    Payload {
                        msg: &envelope[13..],
                        aad: &aad,
                    },
                )
                .map_err(|_| IdentityError::Storage)?,
        );
        let text = std::str::from_utf8(&bytes).map_err(|_| IdentityError::Storage)?;
        Ok(Zeroizing::new(text.to_owned()))
    }

    /// Encryption-key rotation never changes the replay namespace. This key
    /// has a separate offline retirement procedure, not an active-key label.
    pub(crate) fn code_digests(&self, provider: i64, code: &str) -> Result<Vec<[u8; 32]>> {
        if provider <= 0
            || code.is_empty()
            || code.len() > 4096
            || code.chars().any(char::is_control)
        {
            return Err(IdentityError::Invalid);
        }
        let mut mac = <Hmac<Sha256> as hmac::KeyInit>::new_from_slice(self.claim_key.as_ref())
            .map_err(|_| IdentityError::Storage)?;
        mac.update(b"lmm/federated/wechat-code/v1");
        mac.update(&provider.to_be_bytes());
        mac.update(code.as_bytes());
        Ok(vec![mac.finalize().into_bytes().into()])
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn secret_binds_context_revision_and_key() {
        let keys =
            FederatedKeys::new("a".into(), vec![("a".into(), vec![23; 32])], vec![81; 32]).unwrap();
        let (label, envelope) = keys.seal(&[1; 32], 7, "client-secret").unwrap();
        assert_eq!(
            &*keys.open(&[1; 32], 7, &label, &envelope).unwrap(),
            "client-secret"
        );
        assert!(keys.open(&[2; 32], 7, &label, &envelope).is_err());
        assert!(keys.open(&[1; 32], 8, &label, &envelope).is_err());
        assert!(keys.open(&[1; 32], 7, "missing", &envelope).is_err());
        let mut damaged = envelope;
        damaged[15] ^= 1;
        assert!(keys.open(&[1; 32], 7, &label, &damaged).is_err());
    }

    #[test]
    fn stable_claim_key_survives_encryption_key_rotation_and_retirement() {
        let old =
            FederatedKeys::new("a".into(), vec![("a".into(), vec![23; 32])], vec![81; 32]).unwrap();
        let rotated =
            FederatedKeys::new("b".into(), vec![("b".into(), vec![24; 32])], vec![81; 32]).unwrap();
        let prior = old.code_digests(4, "123456").unwrap()[0];
        assert!(rotated.code_digests(4, "123456").unwrap().contains(&prior));
        assert!(!rotated.code_digests(5, "123456").unwrap().contains(&prior));
    }
}

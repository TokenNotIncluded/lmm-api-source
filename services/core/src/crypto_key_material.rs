//! Existing host key derivations shared by consumers and isolation checks.
use hkdf::Hkdf;
use sha2::{Digest, Sha256};
use zeroize::Zeroizing;

#[derive(Clone, Copy)]
pub(crate) enum KeyPurpose {
    ApiVault,
    ChannelVault,
    MfaVault,
    FederatedVault,
    FederatedReplay,
}
impl KeyPurpose {
    pub(crate) fn derive(self, raw: &[u8]) -> Result<Zeroizing<[u8; 32]>, hkdf::InvalidLength> {
        let (salt, info): (&[u8], &[u8]) = match self {
            Self::ApiVault => (b"lmm-api-key-secret-v1", b"aes-256-gcm"),
            Self::ChannelVault => (b"lmm-channel-secret-v1", b"aes-256-gcm"),
            Self::MfaVault => (b"api.lmm.best/core/mfa/v1", b"aes-256-gcm"),
            Self::FederatedVault => (b"lmm/core/federated-auth/v1", b"provider-secrets"),
            Self::FederatedReplay => (b"lmm/core/federated-auth/v1", b"wechat-code-claims"),
        };
        let mut material = Zeroizing::new([0; 32]);
        Hkdf::<Sha256>::new(Some(salt), raw).expand(info, &mut *material)?;
        Ok(material)
    }
}

/// HMAC-SHA256 hashes inputs longer than its block and zero-pads shorter
/// inputs. Compare that actual effective key with a proposed 32-byte key.
pub(crate) fn hmac_sha256_uses(key: &[u8], material: &[u8]) -> bool {
    if material.len() != 32 {
        return false;
    }
    let mut block = Zeroizing::new([0; 64]);
    if key.len() > block.len() {
        block[..32].copy_from_slice(&Sha256::digest(key));
    } else {
        block[..key.len()].copy_from_slice(key);
    }
    &block[..32] == material && block[32..].iter().all(|byte| *byte == 0)
}

//! Channel secrets use their own host-owned key and immutable reference AAD.
//! The SQL surface stores ciphertext only and has no plaintext reveal endpoint.
use super::ManagementError;
use crate::routing::CredentialRef;
use aes_gcm::{
    Aes256Gcm, KeyInit, Nonce,
    aead::{Aead, Payload},
};
use hkdf::Hkdf;
use rand::{RngCore, rngs::OsRng};
use sha2::Sha256;
use zeroize::Zeroizing;

fn cipher(key: &[u8]) -> Result<Aes256Gcm, ManagementError> {
    if key.len() != 32 {
        return Err(ManagementError::Invalid);
    }
    let mut derived = Zeroizing::new([0_u8; 32]);
    Hkdf::<Sha256>::new(Some(b"lmm-channel-secret-v1"), key)
        .expand(b"aes-256-gcm", derived.as_mut())
        .map_err(|_| ManagementError::Storage)?;
    Aes256Gcm::new_from_slice(derived.as_ref()).map_err(|_| ManagementError::Storage)
}
fn aad(reference: CredentialRef) -> [u8; 24] {
    let mut data = [0; 24];
    data[..8].copy_from_slice(&1_u64.to_be_bytes());
    data[8..16].copy_from_slice(&reference.id.to_be_bytes());
    data[16..].copy_from_slice(&reference.revision.to_be_bytes());
    data
}
pub(super) fn encrypt(
    key: &[u8],
    reference: CredentialRef,
    secret: &str,
) -> Result<Vec<u8>, ManagementError> {
    if secret.is_empty() || secret.len() > 4096 || secret.bytes().any(|b| !b.is_ascii_graphic()) {
        return Err(ManagementError::Invalid);
    }
    let mut nonce = [0_u8; 12];
    OsRng
        .try_fill_bytes(&mut nonce)
        .map_err(|_| ManagementError::Storage)?;
    let encrypted = cipher(key)?
        .encrypt(
            Nonce::from_slice(&nonce),
            Payload {
                msg: secret.as_bytes(),
                aad: &aad(reference),
            },
        )
        .map_err(|_| ManagementError::Storage)?;
    let mut result = nonce.to_vec();
    result.extend(encrypted);
    Ok(result)
}
pub(super) fn decrypt(
    key: &[u8],
    reference: CredentialRef,
    bytes: &[u8],
) -> Result<String, ManagementError> {
    if !(29..=4124).contains(&bytes.len()) {
        return Err(ManagementError::Storage);
    }
    let plaintext = cipher(key)?
        .decrypt(
            Nonce::from_slice(&bytes[..12]),
            Payload {
                msg: &bytes[12..],
                aad: &aad(reference),
            },
        )
        .map_err(|_| ManagementError::Storage)?;
    String::from_utf8(plaintext).map_err(|_| ManagementError::Storage)
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn encrypted_keys_bind_the_immutable_reference_and_never_contain_plaintext() {
        let reference = CredentialRef { id: 7, revision: 2 };
        let secret = "channel-synthetic-private-key";
        let encrypted = encrypt(&[41; 32], reference, secret).unwrap();
        assert!(
            !encrypted
                .windows(secret.len())
                .any(|window| window == secret.as_bytes())
        );
        assert_eq!(decrypt(&[41; 32], reference, &encrypted).unwrap(), secret);
        assert!(decrypt(&[41; 32], CredentialRef { id: 8, revision: 2 }, &encrypted).is_err());
        assert!(decrypt(&[42; 32], reference, &encrypted).is_err());
        assert!(encrypt(&[41; 32], reference, "key\nheader").is_err());
    }
}

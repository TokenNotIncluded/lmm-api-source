//! Imported Go recovery verifiers; successful proofs use the normal atomic consumption.
use super::super::{IdentityError, IdentityStore, Result};

const PREFIX: &str = "legacy-bcrypt:";

pub(super) async fn verify(store: &IdentityStore, code: String, verifier: String) -> Result<bool> {
    let Some(hash) = verifier.strip_prefix(PREFIX) else {
        return store.verify_password(code, verifier).await;
    };
    // The fixed Go source generated bcrypt.DefaultCost (10), not arbitrary work factors.
    let hash = hash.to_owned();
    let permit = store
        .password_slots
        .clone()
        .try_acquire_owned()
        .map_err(|_| IdentityError::Storage)?;
    tokio::task::spawn_blocking(move || {
        let _permit = permit;
        verify_legacy(&code, &hash)
    })
    .await
    .map_err(|_| IdentityError::Storage)?
}

fn verify_legacy(code: &str, hash: &str) -> Result<bool> {
    if hash.len() != 60
        || !["$2a$10$", "$2b$10$", "$2y$10$"]
            .iter()
            .any(|p| hash.starts_with(p))
    {
        return Err(IdentityError::Storage);
    }
    bcrypt::verify(code, hash).map_err(|_| IdentityError::Storage)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn legacy_hashes_verify_without_recovering_the_secret() {
        let hash = bcrypt::hash("ABCD-1234", 10).unwrap();
        assert!(verify_legacy("ABCD-1234", &hash).unwrap());
        assert!(!verify_legacy("ABCD-1235", &hash).unwrap());
        let expensive = hash.replacen("$10$", "$31$", 1);
        assert_eq!(
            verify_legacy("ABCD-1234", &expensive),
            Err(IdentityError::Storage)
        );
        assert_eq!(
            verify_legacy("ABCD-1234", "invalid"),
            Err(IdentityError::Storage)
        );
    }
}

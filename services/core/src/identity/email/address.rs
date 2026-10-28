use super::{EmailError, Result};

/// Canonical bare ASCII mailbox; dots and plus tags retain their meaning.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct EmailAddress(String);

impl EmailAddress {
    /// Uses the project's explicit whole-address ASCII lowercase policy.
    pub fn parse(input: &str) -> Result<Self> {
        // Reject controls before trimming so a trailing CR/LF is not erased.
        if !input.is_ascii() || input.bytes().any(|c| c.is_ascii_control()) {
            return Err(EmailError::InvalidAddress);
        }
        let input = input.trim_matches(' ');
        let (local, domain) = input.split_once('@').ok_or(EmailError::InvalidAddress)?;
        if input.len() > 254
            || local.is_empty()
            || local.len() > 64
            || local.starts_with('.')
            || local.ends_with('.')
            || local.contains("..")
            || !local
                .bytes()
                .all(|c| c.is_ascii_alphanumeric() || b".!#$%&'*+-/=?^_`{|}~".contains(&c))
            || domain.is_empty()
            || !domain.split('.').all(|label| {
                !label.is_empty()
                    && label.len() <= 63
                    && !label.starts_with('-')
                    && !label.ends_with('-')
                    && label
                        .bytes()
                        .all(|c| c.is_ascii_alphanumeric() || c == b'-')
            })
        {
            return Err(EmailError::InvalidAddress);
        }
        Ok(Self(input.to_ascii_lowercase()))
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

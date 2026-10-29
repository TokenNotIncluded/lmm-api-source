//! Callable staged mail outbox. Deliberately absent from Core startup and routes.
mod crypto;
mod reset_link;
mod smtp;
mod store;

pub use crypto::MailKeys;
pub use reset_link::ResetSite;
pub use smtp::{SmtpConfig, SmtpMailer, SmtpTls};
pub use store::{Dispatch, Lease, Outbox};

use std::fmt;

/// Contains no SQL, provider response, recipient, token or credential text.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum MailError {
    Invalid,
    Conflict,
    Unavailable,
}

impl fmt::Display for MailError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::Invalid => "invalid mail input",
            Self::Conflict => "mail state conflict",
            Self::Unavailable => "mail unavailable",
        })
    }
}

impl std::error::Error for MailError {}

impl From<sqlx::Error> for MailError {
    fn from(error: sqlx::Error) -> Self {
        match error.as_database_error().and_then(|e| e.code()).as_deref() {
            Some("23505" | "40001" | "40P01" | "55P03") => Self::Conflict,
            _ => Self::Unavailable,
        }
    }
}

type Result<T> = std::result::Result<T, MailError>;

/// Facts about this SMTP attempt; none of these means inbox delivery.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum SmtpOutcome {
    Accepted,
    RejectedTemporary,
    RejectedPermanent,
    Unknown,
}

impl SmtpOutcome {
    fn state(self) -> (&'static str, &'static str) {
        match self {
            Self::Accepted => ("accepted_by_smtp", "smtp_accepted"),
            Self::RejectedTemporary => ("rejected", "smtp_temporary"),
            Self::RejectedPermanent => ("rejected", "smtp_permanent"),
            Self::Unknown => ("unknown", "smtp_uncertain"),
        }
    }
}

fn valid_id(id: &str) -> bool {
    id.len() == 32
        && id
            .bytes()
            .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
}

fn random_id() -> Result<String> {
    use rand::{RngCore, rngs::OsRng};
    use std::fmt::Write;
    let mut bytes = [0_u8; 16];
    OsRng
        .try_fill_bytes(&mut bytes)
        .map_err(|_| MailError::Unavailable)?;
    let mut id = String::with_capacity(32);
    for byte in bytes {
        write!(id, "{byte:02x}").map_err(|_| MailError::Unavailable)?;
    }
    Ok(id)
}

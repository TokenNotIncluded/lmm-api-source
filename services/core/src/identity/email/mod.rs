//! Purpose-bound email proofs. No startup DDL, HTTP adapter or mail transport.
//!
//! A successful verification owns an open transaction. Callers perform their
//! identity mutation through `connection()` and then `finish()`; dropping it
//! rolls back both the mutation and consumption. Rejections commit guessing
//! budgets before returning. Never return an `Issued` secret through HTTP.
mod address;
mod store;

pub use address::EmailAddress;
pub use store::{EmailChallenges, EmailMutation, IssueOutcome, IssuedEmailChallenge, Request};

use std::fmt;

/// Storage failures are deliberately free of SQL, credentials and addresses.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum EmailError {
    InvalidAddress,
    InvalidProof,
    Conflict,
    Unavailable,
}

impl fmt::Display for EmailError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{self:?}")
    }
}

impl std::error::Error for EmailError {}

impl From<sqlx::Error> for EmailError {
    fn from(error: sqlx::Error) -> Self {
        match error.as_database_error().and_then(|e| e.code()).as_deref() {
            Some("23505" | "40001" | "40P01" | "55P03") => Self::Conflict,
            _ => Self::Unavailable,
        }
    }
}

type Result<T> = std::result::Result<T, EmailError>;

#[cfg(test)]
mod tests;

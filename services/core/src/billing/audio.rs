//! Audio billing v1: four disjoint counters, frozen exact prices and raw evidence.
//! Pure calculation only. Errors never authorize releasing a dispatched hold.
//! Admission/dispatch authentication, durable receipts and ledger idempotency
//! remain the caller's responsibility. Do not expose quote construction to HTTP.
mod integrated;
pub(crate) mod journal;
mod price;
mod receipt;

pub use integrated::*;
pub use price::*;
pub use receipt::*;
pub(crate) use receipt::{validate_chat_usage_retention, validate_unique_json};

#[cfg(test)]
mod tests;

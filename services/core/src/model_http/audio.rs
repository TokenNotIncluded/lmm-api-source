//! Audio billing v1: four disjoint counters, frozen exact prices and raw evidence.
//! Pure calculation only. Errors never authorize releasing a dispatched hold.
//! Admission/dispatch authentication, durable receipts and ledger idempotency
//! remain the caller's responsibility. Do not expose quote construction to HTTP.
mod config;
mod integrated;
mod price;
pub use config::AudioRule;
pub(crate) mod http;
pub use integrated::*;
mod receipt;

pub use price::*;
pub use receipt::*;

#[cfg(test)]
mod tests;

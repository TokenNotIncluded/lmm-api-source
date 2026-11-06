//! Reviewed audio provider configuration and HTTP execution.
//! Pricing, usage evidence and journal validation belong to billing::audio.
mod config;
pub use config::AudioRule;
pub(crate) mod http;

//! Reviewed audio provider configuration and HTTP execution.
//! Pricing, usage evidence and journal validation belong to billing::audio.
use crate::billing::audio::AudioPrice;
use serde::{Deserialize, Serialize};

/// Host-only reviewed provider/model contract. A route release must match exactly.
/// No default contract is inferred for local providers or from a model name.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AudioRule {
    pub route_version: u64,
    pub upstream_id: u64,
    pub upstream_model: String,
    pub endpoint: String,
    pub price: AudioPrice,
    /// Explicit review of streaming usage/terminal semantics, disabled for old rules.
    #[serde(default)]
    pub chat_streaming: bool,
}

pub(crate) mod http;

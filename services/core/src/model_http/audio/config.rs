use super::*;
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
}

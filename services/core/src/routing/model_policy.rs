//! Optional settings on an immutable routing release.
use super::RoutingError;
use regex::RegexSet;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct ChatResponsesPolicy {
    pub enabled: bool,
    pub all_channels: bool,
    pub channel_ids: Vec<u64>,
    pub model_patterns: Vec<String>,
    pub overrides: BTreeMap<u64, bool>,
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct ChannelOverrides {
    pub parameters: BTreeMap<String, serde_json::Number>,
    pub headers: BTreeMap<String, String>,
    pub status_codes: BTreeMap<u16, u16>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct GlobalModelPolicy {
    pub pass_through_request_enabled: bool,
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub honor_request_service_tier: bool,
    pub thinking_model_blacklist: Vec<String>,
    pub chat_completions_to_responses_policy: ChatResponsesPolicy,
    pub ping_interval_enabled: bool,
    pub ping_interval_seconds: u32,
}

#[derive(Debug)]
pub(crate) struct Compiled {
    pub config: GlobalModelPolicy,
    patterns: RegexSet,
}
impl GlobalModelPolicy {
    pub(crate) fn compile(
        self,
        channels: &[super::UpstreamConfig],
    ) -> Result<Compiled, RoutingError> {
        let bad = RoutingError::InvalidConfig("global model policy");
        let exists = |id: &u64| channels.iter().any(|u| u.id == *id);
        let chat = &self.chat_completions_to_responses_policy;
        if !(1..=3600).contains(&self.ping_interval_seconds)
            || self.thinking_model_blacklist.len() > 512
            || self
                .thinking_model_blacklist
                .iter()
                .any(|name| !super::config::valid_model(name))
            || chat.model_patterns.len() > 128
            || chat
                .model_patterns
                .iter()
                .any(|s| s.is_empty() || s.len() > 256)
            || !chat
                .channel_ids
                .iter()
                .chain(chat.overrides.keys())
                .all(exists)
        {
            return Err(bad);
        }
        let patterns = RegexSet::new(&chat.model_patterns).map_err(|_| bad)?;
        Ok(Compiled {
            config: self,
            patterns,
        })
    }
}
impl Compiled {
    pub fn upgrade_chat(&self, channel: u64, model: &str) -> bool {
        let chat = &self.config.chat_completions_to_responses_policy;
        chat.overrides.get(&channel).copied().unwrap_or_else(|| {
            chat.enabled && (chat.all_channels || chat.channel_ids.contains(&channel))
        }) && self.patterns.is_match(model)
    }
    pub fn preserve_suffix(&self, name: &str) -> bool {
        self.config
            .thinking_model_blacklist
            .iter()
            .any(|s| s == name)
    }
}

pub(super) fn validate_channel_overrides(
    entries: &BTreeMap<u64, ChannelOverrides>,
    channels: &[super::UpstreamConfig],
) -> Result<(), RoutingError> {
    let ids: std::collections::HashSet<_> = channels.iter().map(|u| u.id).collect();
    if entries.keys().any(|id| !ids.contains(id)) {
        return Err(RoutingError::InvalidConfig("channel overrides"));
    }
    for overrides in entries.values() {
        if overrides.parameters.iter().any(|(name, n)| {
            !n.as_f64().is_some_and(|n| match name.as_str() {
                "temperature" => (0.0..=2.0).contains(&n),
                "top_p" => n > 0.0 && n <= 1.0,
                _ => false,
            })
        }) || overrides.headers.iter().any(|(name, value)| {
            !matches!(name.as_str(), "user-agent" | "x-client-version")
                || value.is_empty()
                || value.len() > 256
                || !value.bytes().all(|b| b.is_ascii_graphic() || b == b' ')
        }) || overrides
            .status_codes
            .iter()
            .any(|(from, to)| !(400..=599).contains(from) || !(400..=599).contains(to))
        {
            return Err(RoutingError::InvalidConfig("channel overrides"));
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::GlobalModelPolicy;
    use serde_json::{from_value, json, to_value};

    #[test]
    fn service_tier_opt_in_preserves_the_previous_policy_wire_format() {
        let previous = json!({
            "pass_through_request_enabled": false,
            "thinking_model_blacklist": [],
            "chat_completions_to_responses_policy": {
                "enabled": false, "all_channels": false,
                "channel_ids": [], "model_patterns": [], "overrides": {}
            },
            "ping_interval_enabled": false,
            "ping_interval_seconds": 60
        });
        let policy: GlobalModelPolicy = from_value(previous.clone()).unwrap();
        assert!(!policy.honor_request_service_tier);
        assert_eq!(to_value(policy).unwrap(), previous);

        for enabled in [true, false] {
            let mut document = previous.clone();
            document["honor_request_service_tier"] = json!(enabled);
            let policy: GlobalModelPolicy = from_value(document.clone()).unwrap();
            assert_eq!(policy.honor_request_service_tier, enabled);
            let expected = if enabled { document } else { previous.clone() };
            assert_eq!(to_value(policy).unwrap(), expected);
        }
    }
}

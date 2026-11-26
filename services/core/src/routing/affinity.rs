//! Request key extraction for success-only channel affinity under an immutable release.
pub(crate) mod store;
use super::RoutingError;
use axum::http::{HeaderMap, HeaderName};
use regex::Regex;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use sha2::{Digest, Sha256};
use std::{borrow::Cow, collections::HashSet, sync::Arc};

#[derive(Clone, Debug, Serialize, Deserialize, Eq, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct Config {
    pub enabled: bool,
    pub switch_on_success: bool,
    pub keep_on_channel_disabled: bool,
    pub max_entries: usize,
    pub default_ttl_seconds: u32,
    pub rules: Vec<Rule>,
}

#[derive(Clone, Debug, Serialize, Deserialize, Eq, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct Rule {
    pub name: String,
    pub model_regex: Vec<String>,
    #[serde(default)]
    pub path_regex: Vec<String>,
    #[serde(default)]
    pub user_agent_include: Vec<String>,
    pub key_sources: Vec<Source>,
    #[serde(default)]
    pub value_regex: String,
    #[serde(default)]
    pub ttl_seconds: u32,
    pub skip_retry_on_failure: bool,
    pub include_using_group: bool,
    pub include_model_name: bool,
    pub include_rule_name: bool,
}

#[derive(Clone, Debug, Serialize, Deserialize, Eq, PartialEq)]
#[serde(tag = "type", rename_all = "snake_case", deny_unknown_fields)]
pub enum Source {
    ContextInt { key: String },
    ContextString { key: String },
    RequestHeader { key: String },
    Gjson { path: String },
}

pub(crate) struct Input<'a> {
    pub model: &'a str,
    pub path: &'a str,
    pub tenant_id: i64,
    pub user_id: i64,
    pub key_id: Option<i64>,
    pub headers: &'a HeaderMap,
    pub body: &'a Value,
}

impl Source {
    fn valid(&self) -> bool {
        match self {
            Self::ContextInt { key } => matches!(key.as_str(), "user_id" | "key_id" | "token_id"),
            Self::ContextString { key } => {
                matches!(key.as_str(), "model" | "group" | "using_group" | "path")
            }
            Self::RequestHeader { key } => {
                HeaderName::from_bytes(key.as_bytes()).is_ok()
                    && !matches!(
                        key.to_ascii_lowercase().as_str(),
                        "authorization"
                            | "proxy-authorization"
                            | "cookie"
                            | "x-api-key"
                            | "x-auth-session"
                            | "x-goog-api-key"
                    )
            }
            Self::Gjson { path } => {
                !path.is_empty()
                    && path.len() <= 256
                    && path.split('.').all(|part| {
                        !part.is_empty()
                            && part.bytes().all(|byte| {
                                byte.is_ascii_alphanumeric() || matches!(byte, b'_' | b'-')
                            })
                    })
            }
        }
    }
    fn read<'a>(&self, input: &Input<'a>, group: &'a str) -> Option<Cow<'a, str>> {
        let value = match self {
            Self::ContextInt { key } => Cow::Owned(match key.as_str() {
                "user_id" => input.user_id.to_string(),
                "key_id" | "token_id" => input.key_id?.to_string(),
                _ => return None,
            }),
            Self::ContextString { key } => Cow::Borrowed(match key.as_str() {
                "model" => input.model,
                "group" | "using_group" => group,
                "path" => input.path,
                _ => return None,
            }),
            Self::RequestHeader { key } => {
                let mut values = input.headers.get_all(key.as_str()).iter();
                let first = values.next()?.to_str().ok()?;
                if values.next().is_some() {
                    return None;
                }
                Cow::Borrowed(first)
            }
            Self::Gjson { path } => {
                let mut node = input.body;
                for part in path.split('.') {
                    node = if node.is_array() {
                        node.get(part.parse::<usize>().ok()?)?
                    } else {
                        node.get(part)?
                    };
                }
                match node {
                    Value::String(value) => Cow::Borrowed(value.as_str()),
                    Value::Number(value) => Cow::Owned(value.to_string()),
                    _ => return None,
                }
            }
        };
        (!value.is_empty()).then_some(value)
    }
}

struct CompiledRule {
    model: Vec<Regex>,
    path: Vec<Regex>,
    value: Option<Regex>,
}
fn patterns(patterns: &[String]) -> Result<Vec<Regex>, RoutingError> {
    patterns
        .iter()
        .map(|p| {
            if p.is_empty() || p.len() > 1024 {
                return Err(RoutingError::InvalidConfig("channel affinity regex"));
            }
            regex::RegexBuilder::new(p)
                .size_limit(256 * 1024)
                .build()
                .map_err(|_| RoutingError::InvalidConfig("channel affinity regex"))
        })
        .collect()
}

pub(crate) struct Runtime {
    pub(crate) config: Option<Config>,
    rules: Vec<CompiledRule>,
}

#[derive(Clone)]
pub(crate) struct Binding {
    pub(crate) key: [u8; 32],
    pub(crate) rule: String,
    pub(crate) ttl: u32,
    pub(crate) max_entries: usize,
    pub(crate) skip_retry: bool,
    pub(crate) switch_on_success: bool,
    pub(crate) keep_invalid: bool,
    pub(crate) preserve: bool,
}
impl Runtime {
    pub(crate) fn compile(
        config: Option<Config>,
        previous: Option<&Arc<Self>>,
    ) -> Result<Arc<Self>, RoutingError> {
        if let Some(previous) = previous.filter(|old| old.config == config) {
            return Ok(previous.clone());
        }
        let mut rules = Vec::new();
        if let Some(config) = &config {
            if !(1..=1_000_000).contains(&config.max_entries)
                || config.default_ttl_seconds == 0
                || config.rules.len() > 128
            {
                return Err(RoutingError::InvalidConfig(
                    "channel affinity capacity, TTL or rules",
                ));
            }
            let mut names = HashSet::new();
            for rule in &config.rules {
                if rule.name.trim().is_empty()
                    || rule.name.len() > 128
                    || rule.name.chars().any(char::is_control)
                    || !names.insert(&rule.name)
                    || rule.model_regex.len() > 32
                    || rule.path_regex.len() > 32
                    || rule.key_sources.is_empty()
                    || rule.key_sources.len() > 16
                    || !rule.key_sources.iter().all(Source::valid)
                    || rule.user_agent_include.len() > 32
                    || rule
                        .user_agent_include
                        .iter()
                        .any(|s| s.is_empty() || s.len() > 256)
                {
                    return Err(RoutingError::InvalidConfig("channel affinity rule"));
                }
                rules.push(CompiledRule {
                    model: patterns(&rule.model_regex)?,
                    path: patterns(&rule.path_regex)?,
                    value: if rule.value_regex.is_empty() {
                        None
                    } else {
                        patterns(std::slice::from_ref(&rule.value_regex))?.pop()
                    },
                });
            }
        }
        Ok(Arc::new(Self { config, rules }))
    }
    pub(crate) fn enabled(&self) -> bool {
        self.config
            .as_ref()
            .is_some_and(|c| c.enabled && !c.rules.is_empty())
    }
    pub(crate) fn bind(&self, input: &Input<'_>, group: &str) -> Option<Binding> {
        let config = self.config.as_ref().filter(|c| c.enabled)?;
        for (rule, matched) in config.rules.iter().zip(&self.rules) {
            if (!matched.model.is_empty()
                && !matched.model.iter().any(|re| re.is_match(input.model)))
                || (!matched.path.is_empty()
                    && !matched.path.iter().any(|re| re.is_match(input.path)))
                || (!rule.user_agent_include.is_empty()
                    && !input
                        .headers
                        .get("user-agent")
                        .and_then(|h| h.to_str().ok())
                        .is_some_and(|ua| {
                            rule.user_agent_include
                                .iter()
                                .any(|s| ua.contains(s.as_str()))
                        }))
            {
                continue;
            }
            let Some(value) = rule
                .key_sources
                .iter()
                .filter_map(|source| source.read(input, group))
                .find(|value| matched.value.as_ref().is_none_or(|re| re.is_match(value)))
            else {
                continue;
            };
            let mut hash = Sha256::new();
            hash.update(b"lmm-channel-affinity-v1\0");
            hash.update(input.tenant_id.to_be_bytes());
            for part in [
                rule.include_rule_name.then_some(rule.name.as_str()),
                rule.include_using_group.then_some(group),
                rule.include_model_name.then_some(input.model),
                Some(value.as_ref()),
            ] {
                if let Some(part) = part {
                    hash.update([1]);
                    hash.update((part.len() as u64).to_be_bytes());
                    hash.update(part.as_bytes());
                } else {
                    hash.update([0]);
                }
            }
            return Some(Binding {
                key: hash.finalize().into(),
                rule: rule.name.clone(),
                ttl: if rule.ttl_seconds == 0 {
                    config.default_ttl_seconds
                } else {
                    rule.ttl_seconds
                },
                max_entries: config.max_entries,
                skip_retry: rule.skip_retry_on_failure,
                switch_on_success: config.switch_on_success,
                keep_invalid: config.keep_on_channel_disabled,
                preserve: false,
            });
        }
        None
    }
}

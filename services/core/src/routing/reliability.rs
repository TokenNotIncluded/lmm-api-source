//! Global retry and health settings in an immutable routing release.
use super::RoutingError;
use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ProbeMode {
    ScheduledAll,
    PassiveRecovery,
}

/// Status ranges are compiled on read; requests never parse configuration strings.
#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(try_from = "String", into = "String")]
pub struct StatusRanges(Vec<(u16, u16)>);

impl TryFrom<String> for StatusRanges {
    type Error = &'static str;

    fn try_from(value: String) -> Result<Self, Self::Error> {
        if value.len() > 4096 {
            return Err("status range input is too long");
        }
        let mut ranges = Vec::new();
        fn code(value: &str) -> Result<u16, &'static str> {
            let s = value.trim();
            let value = s.parse::<u16>().map_err(|_| "invalid HTTP status")?;
            if s.len() != 3 || !(100..=599).contains(&value) {
                return Err("HTTP status must be between 100 and 599");
            }
            Ok(value)
        }
        if !value.trim().is_empty() {
            for part in value.split([',', '，']) {
                let (start, end) = part.split_once('-').unwrap_or((part, part));
                let (start, end) = (code(start)?, code(end)?);
                if start > end {
                    return Err("reversed HTTP status range");
                }
                ranges.push((start, end));
            }
        }
        ranges.sort_unstable();
        let mut merged: Vec<(u16, u16)> = Vec::new();
        for (start, end) in ranges {
            if let Some(last) = merged.last_mut()
                && start <= last.1 + 1
            {
                last.1 = last.1.max(end);
            } else {
                merged.push((start, end));
            }
        }
        Ok(Self(merged))
    }
}

impl From<StatusRanges> for String {
    fn from(value: StatusRanges) -> Self {
        value
            .0
            .into_iter()
            .map(|(s, e)| {
                if s == e {
                    s.to_string()
                } else {
                    format!("{s}-{e}")
                }
            })
            .collect::<Vec<_>>()
            .join(",")
    }
}

impl StatusRanges {
    pub(crate) fn includes(&self, status: u16) -> bool {
        self.0.iter().any(|&(a, b)| (a..=b).contains(&status))
    }
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct RoutingReliability {
    pub retry_times: u8,
    pub retry_status_codes: StatusRanges,
    pub auto_disable: bool,
    pub disable_status_codes: StatusRanges,
    pub disable_keywords: Vec<String>,
    pub disable_threshold_seconds: u32,
    pub probe_enabled: bool,
    pub probe_mode: ProbeMode,
    pub probe_interval_seconds: u32,
    pub auto_recover: bool,
}

impl RoutingReliability {
    pub(crate) fn validate(&self) -> Result<(), RoutingError> {
        if self.retry_times > 10
            || !(1..=86400).contains(&self.probe_interval_seconds)
            || self.disable_keywords.len() > 100
            || self
                .disable_keywords
                .iter()
                .any(|v| v.trim().is_empty() || v.len() > 256)
        {
            return Err(RoutingError::InvalidConfig("routing reliability policy"));
        }
        Ok(())
    }

    pub(crate) fn matches_error(keywords: &[String], bytes: &[u8]) -> bool {
        !keywords.is_empty()
            && serde_json::from_slice::<serde_json::Value>(bytes)
                .ok()
                .is_some_and(|v| {
                    v.pointer("/error/message")
                        .and_then(|v| v.as_str())
                        .is_some_and(|message| {
                            let message = message.to_lowercase();
                            keywords.iter().any(|k| message.contains(&k.to_lowercase()))
                        })
                })
    }

    pub(crate) fn too_slow(&self, elapsed: std::time::Duration) -> bool {
        self.disable_threshold_seconds > 0
            && elapsed > std::time::Duration::from_secs(u64::from(self.disable_threshold_seconds))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn reliability_status_ranges_keywords_bounds_and_delay() {
        let mut policy: RoutingReliability = serde_json::from_value(serde_json::json!({
            "retry_times":10,"retry_status_codes":"502-505，401,501-503, 506",
            "disable_status_codes":"401","auto_disable":true,"disable_keywords":["Invalid KEY"],
            "disable_threshold_seconds":5,"auto_recover":true,"probe_enabled":true,
            "probe_mode":"passive_recovery","probe_interval_seconds":30
        }))
        .unwrap();
        assert!(policy.validate().is_ok());
        assert_eq!(
            serde_json::to_value(&policy).unwrap()["retry_status_codes"],
            "401,501-506"
        );
        for s in ["99", "600", "401-400", "400-401-402", "a", "401,,429"] {
            assert!(StatusRanges::try_from(s.to_string()).is_err());
        }
        assert!(policy.retry_status_codes.includes(501));
        assert!(!policy.retry_status_codes.includes(500));
        assert!(!policy.too_slow(std::time::Duration::from_secs(5)));
        assert!(policy.too_slow(std::time::Duration::from_millis(5001)));
        assert!(RoutingReliability::matches_error(
            &policy.disable_keywords,
            br#"{"error":{"message":"INVALID KEY"}}"#
        ));
        assert!(!RoutingReliability::matches_error(
            &policy.disable_keywords,
            br#"{"message":"invalid key"}"#
        ));
        policy.retry_times = 11;
        assert!(policy.validate().is_err());
        policy.retry_times = 0;
        policy.probe_interval_seconds = 0;
        assert!(policy.validate().is_err());
    }
}

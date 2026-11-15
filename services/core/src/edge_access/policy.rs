//! Bounded edge policy shared by validation, preview and live decisions.
use super::{SetupError, canonical_ip};
use ipnet::IpNet;
use std::net::IpAddr;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Action {
    Direct,
    Reject,
}
impl Action {
    fn parse(raw: &str) -> Result<Self, SetupError> {
        match raw.trim() {
            "direct" => Ok(Self::Direct),
            "reject" => Ok(Self::Reject),
            _ => Err(SetupError),
        }
    }
    fn name(self) -> &'static str {
        match self {
            Self::Direct => "direct",
            Self::Reject => "reject",
        }
    }
}

enum Matcher {
    Prefix(IpNet),
    Range(IpAddr, IpAddr),
    Country(String),
    UnknownCountry,
    Version(bool),
}
impl Matcher {
    fn parse(raw: &str) -> Result<Self, SetupError> {
        let raw = raw.trim();
        if let Some(code) = raw.strip_prefix("geoip:") {
            if code == "unknown" {
                return Ok(Self::UnknownCountry);
            }
            if code.len() != 2 || !code.bytes().all(|b| b.is_ascii_alphabetic()) {
                return Err(SetupError);
            }
            return Ok(Self::Country(code.to_ascii_uppercase()));
        }
        if let Some((start, end)) = raw.split_once('-') {
            let start = canonical_ip(start.trim()).ok_or(SetupError)?;
            let end = canonical_ip(end.trim()).ok_or(SetupError)?;
            if start.is_ipv4() != end.is_ipv4() || start > end {
                return Err(SetupError);
            }
            return Ok(Self::Range(start, end));
        }
        if let Some(ip) = canonical_ip(raw) {
            return Ok(Self::Prefix(IpNet::from(ip)));
        }
        let mut net: IpNet = raw.parse().map_err(|_| SetupError)?;
        // Requests normalize mapped IPv6 to IPv4; normalize matching networks too.
        if let IpNet::V6(v6) = net
            && let Some(v4) = v6.addr().to_ipv4_mapped()
        {
            if v6.prefix_len() < 96 {
                return Err(SetupError);
            }
            net = IpNet::new(IpAddr::V4(v4), v6.prefix_len() - 96).map_err(|_| SetupError)?;
        }
        Ok(Self::Prefix(net))
    }
    fn matches(&self, ip: IpAddr, country: Option<&str>) -> Option<bool> {
        match self {
            Self::Prefix(prefix) => Some(prefix.contains(&ip)),
            Self::Range(start, end) => {
                Some(ip.is_ipv4() == start.is_ipv4() && ip >= *start && ip <= *end)
            }
            Self::Country(expected) => country.map(|actual| actual == expected),
            Self::UnknownCountry => Some(country.is_none()),
            Self::Version(v4) => Some(ip.is_ipv4() == *v4),
        }
    }
}
struct Rule {
    matchers: Vec<Matcher>,
    negated: bool,
    action: Action,
    line: usize,
}
pub(super) struct Policy {
    rules: Vec<Rule>,
    fallback: Action,
    unknown_country: Option<Action>,
}
impl Policy {
    pub(super) fn parse(source: &str) -> Result<Self, SetupError> {
        if source.len() > 16384 {
            return Err(SetupError);
        }
        let mut rules = Vec::new();
        let mut fallback = None;
        let mut unknown_country = None;
        let mut block = false;
        let mut closed = false;
        for (index, raw) in source.lines().enumerate() {
            if raw.len() > 1024 {
                return Err(SetupError);
            }
            let line = raw.split('#').next().unwrap_or("").trim();
            if line.is_empty() {
                continue;
            }
            if closed {
                return Err(SetupError);
            }
            if line == "routing {" {
                if block || !rules.is_empty() || fallback.is_some() || unknown_country.is_some() {
                    return Err(SetupError);
                }
                block = true;
                continue;
            }
            if line == "}" {
                if !block {
                    return Err(SetupError);
                }
                closed = true;
                continue;
            }
            if let Some(value) = line.strip_prefix("fallback:") {
                if fallback.replace(Action::parse(value)?).is_some() {
                    return Err(SetupError);
                }
                continue;
            }
            if let Some(value) = line.strip_prefix("unknown-country:") {
                if unknown_country.replace(Action::parse(value)?).is_some() {
                    return Err(SetupError);
                }
                continue;
            }
            if rules.len() >= 256 {
                return Err(SetupError);
            }
            let (predicate, result) = line.split_once("->").ok_or(SetupError)?;
            let predicate = predicate.trim();
            let negated = predicate.starts_with('!');
            let predicate = predicate.strip_prefix('!').unwrap_or(predicate).trim();
            let (kind, arguments) = predicate.split_once('(').ok_or(SetupError)?;
            let arguments = arguments.strip_suffix(')').ok_or(SetupError)?;
            let matchers = match kind {
                "dip" | "ip" => arguments
                    .split(',')
                    .map(Matcher::parse)
                    .collect::<Result<Vec<_>, _>>()?,
                "ipversion" => vec![Matcher::Version(match arguments.trim() {
                    "4" => true,
                    "6" => false,
                    _ => return Err(SetupError),
                })],
                _ => return Err(SetupError),
            };
            if matchers.len() > 64 {
                return Err(SetupError);
            }
            rules.push(Rule {
                matchers,
                negated,
                action: Action::parse(result)?,
                line: index + 1,
            });
        }
        if (block && !closed) || (rules.is_empty() && fallback.is_none()) {
            return Err(SetupError);
        }
        Ok(Self {
            rules,
            fallback: fallback.unwrap_or(Action::Direct),
            unknown_country,
        })
    }
    fn decision(
        &self,
        ip: IpAddr,
        country: Option<&str>,
    ) -> (Option<Action>, Option<usize>, &'static str) {
        for rule in &self.rules {
            let mut matched = false;
            let mut unknown = false;
            for matcher in &rule.matchers {
                match matcher.matches(ip, country) {
                    Some(true) => {
                        matched = true;
                        break;
                    }
                    Some(false) => (),
                    None => unknown = true,
                }
            }
            // Unknown never becomes true through negation. A known true list
            // member still settles the OR without requiring a country lookup.
            if !matched && unknown {
                return (self.unknown_country, Some(rule.line), "unknown_country");
            }
            if matched != rule.negated {
                return (Some(rule.action), Some(rule.line), "rule");
            }
        }
        (Some(self.fallback), None, "fallback")
    }
    pub(super) fn evaluate(&self, ip: IpAddr, country: Option<&str>) -> Result<Action, SetupError> {
        self.decision(ip, country).0.ok_or(SetupError)
    }
    pub(super) fn preview(&self, ip: IpAddr, country: Option<&str>) -> serde_json::Value {
        let (action, line, reason) = self.decision(ip, country);
        serde_json::json!({"action":action.map_or("unavailable", Action::name), "matched_line":line, "reason":reason})
    }
}

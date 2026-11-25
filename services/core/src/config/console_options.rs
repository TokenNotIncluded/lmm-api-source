use super::{ConsoleFile, invalid_config};
use serde::Deserialize;
use std::{collections::BTreeSet, io};

impl ConsoleFile {
    pub(super) fn options(self) -> io::Result<[(&'static str, Option<String>); 4]> {
        let options = [
            ("Notice", self.notice),
            ("console_setting.announcements", self.announcements),
            ("USDExchangeRate", self.usd_exchange_rate),
            ("IPAccessRoutingRules", self.ip_access_routing_rules),
        ];
        for (key, value) in &options {
            let Some(value) = value else {
                continue;
            };
            if !self.enabled {
                return Err(invalid_config(
                    "file console options require console.enabled",
                ));
            }
            match *key {
                "USDExchangeRate"
                    if crate::options_http::display_exchange_rate(value).is_none() =>
                {
                    return Err(invalid_config("invalid console display exchange rate"));
                }
                "IPAccessRoutingRules"
                    if value.contains('\0')
                        || crate::edge_access::validate_policy(value).is_err() =>
                {
                    return Err(invalid_config("invalid console IP access rules"));
                }
                "console_setting.announcements" => validate_announcements(value)?,
                _ => {}
            }
        }
        Ok(options)
    }
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Announcement {
    id: i64,
    content: String,
    #[serde(rename = "publishDate")]
    publish_date: String,
    #[serde(default)]
    extra: String,
    #[serde(default, rename = "mandatory")]
    _mandatory: bool,
    #[serde(default, rename = "ackRevision")]
    ack_revision: String,
    #[serde(default, rename = "type")]
    _kind: Option<AnnouncementKind>,
}

#[derive(Deserialize)]
#[serde(rename_all = "lowercase")]
enum AnnouncementKind {
    Default,
    Ongoing,
    Success,
    Warning,
    Error,
}

fn validate_announcements(value: &str) -> io::Result<()> {
    let items: Vec<Announcement> = serde_json::from_str(value).map_err(|_| {
        invalid_config("Announcements must be a JSON array of announcement objects")
    })?;
    let mut ids = BTreeSet::new();
    for item in items {
        if item.id <= 0
            || !ids.insert(item.id)
            || item.content.trim().is_empty()
            || item.content.chars().count() > 500
            || item.extra.chars().count() > 100
            || item.ack_revision.len() > 32
            || !item
                .ack_revision
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || b"_.-".contains(&b))
            || chrono::DateTime::parse_from_rfc3339(&item.publish_date).is_err()
        {
            return Err(invalid_config(
                "Invalid announcement ID, content or publication date",
            ));
        }
    }
    Ok(())
}

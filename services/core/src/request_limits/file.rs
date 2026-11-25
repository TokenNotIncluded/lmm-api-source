//! Apply operator-supplied TOML explicitly, under the authenticated runtime role.
use super::{Policy, SettingsError, administrator};
use serde::Deserialize;
use serde_json::{Value, json};
use sqlx::PgPool;
use std::{
    collections::HashSet,
    io::{self, Read},
    path::PathBuf,
};
use zeroize::Zeroizing;

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RoutingPolicyFile {
    session_file: PathBuf,
    request_limits: Option<LimitsUpdate>,
    auto_groups: Option<GroupsUpdate>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct LimitsUpdate {
    expected_revision: i64,
    policy: Policy,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct GroupsUpdate {
    expected_revision: i64,
    groups: Vec<String>,
}

fn invalid() -> io::Error {
    io::Error::new(io::ErrorKind::InvalidInput, "invalid routing policy TOML")
}

fn database(_: sqlx::Error) -> io::Error {
    io::Error::other("routing policy database operation failed")
}

impl RoutingPolicyFile {
    pub fn read(input: impl Read) -> io::Result<Self> {
        const MAX_BYTES: u64 = 65_536;
        let mut text = Zeroizing::new(String::new());
        input
            .take(MAX_BYTES + 1)
            .read_to_string(&mut text)
            .map_err(|_| invalid())?;
        if text.len() as u64 > MAX_BYTES {
            return Err(invalid());
        }
        let file: Self = toml::from_str(&text).map_err(|_| invalid())?;
        if !file.session_file.is_absolute()
            || (file.request_limits.is_none() && file.auto_groups.is_none())
        {
            return Err(invalid());
        }
        if let Some(update) = &file.request_limits {
            let p = &update.policy;
            if update.expected_revision < 1
                || !(1..=86_400).contains(&p.window_seconds)
                || p.user_limit.is_some_and(|n| !(1..=100_000_000).contains(&n))
                || p.group_limits.len() > 1024
                || p.group_limits.iter().any(|(name, n)| {
                    !crate::routing::valid_group_name(name) || !(1..=100_000_000).contains(n)
                })
                || (p.enabled && p.user_limit.is_none() && p.group_limits.is_empty())
            {
                return Err(invalid());
            }
        }
        if let Some(update) = &file.auto_groups {
            let mut seen = HashSet::new();
            if update.expected_revision < 1
                || update.groups.len() > 1024
                || update.groups.iter().any(|name| {
                    !crate::routing::valid_group_name(name) || !seen.insert(name)
                })
            {
                return Err(invalid());
            }
        }
        Ok(file)
    }

    pub async fn apply(self, pool: &PgPool) -> io::Result<Value> {
        let bytes = Zeroizing::new(crate::config::private_file(&self.session_file, 8192)?);
        let secret = std::str::from_utf8(&bytes).map_err(|_| invalid())?;
        let secret = secret
            .strip_suffix("\r\n")
            .or_else(|| secret.strip_suffix('\n'))
            .unwrap_or(secret);
        if secret.is_empty() || secret.chars().any(char::is_control) {
            return Err(invalid());
        }
        let mut tx = pool.begin().await.map_err(database)?;
        let actor = administrator(&mut tx, secret)
            .await
            .map_err(|error| match error {
                SettingsError::Unauthorized | SettingsError::Forbidden => io::Error::new(
                    io::ErrorKind::PermissionDenied,
                    "an active superadministrator session is required",
                ),
                _ => io::Error::other("unable to verify routing policy authorization"),
            })?;
        let mut receipt = json!({});
        if let Some(update) = self.request_limits {
            let p = update.policy;
            let revision: Option<i64> = sqlx::query_scalar(
                "SELECT core_request_limits.write_policy($1,$2,$3,$4,$5,$6)",
            )
            .bind(update.expected_revision)
            .bind(p.enabled)
            .bind(p.window_seconds as i32)
            .bind(p.user_limit.map(|v| v as i32))
            .bind(json!(p.group_limits))
            .bind(actor)
            .fetch_one(&mut *tx)
            .await
            .map_err(database)?;
            let revision = revision.ok_or_else(|| io::Error::other("request limits revision conflict"))?;
            receipt["request_limits_revision"] = revision.into();
        }
        if let Some(update) = self.auto_groups {
            let revision: Option<i64> = sqlx::query_scalar(
                "SELECT core_request_limits.write_auto_groups($1,$2,$3)",
            )
            .bind(update.expected_revision)
            .bind(update.groups)
            .bind(actor)
            .fetch_one(&mut *tx)
            .await
            .map_err(database)?;
            let revision = revision.ok_or_else(|| io::Error::other("Auto groups revision conflict"))?;
            receipt["auto_groups_revision"] = revision.into();
        }
        tx.commit().await.map_err(|_| io::Error::other(
            "routing policy commit unconfirmed; reconcile both revisions before retrying",
        ))?;
        Ok(receipt)
    }
}

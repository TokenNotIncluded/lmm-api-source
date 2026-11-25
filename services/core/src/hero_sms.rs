// Copyright (C) 2026 LIghtJUNction. SPDX-License-Identifier: AGPL-3.0-or-later.
//! Core-owned HeroSMS configuration. Supplier operations remain unavailable until
//! their reservation/receipt implementation is installed.
use serde::{Deserialize, Serialize};
use std::{
    path::{Path, PathBuf},
    time::Duration,
};

pub mod http;
#[cfg(all(test, unix))]
mod tests;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Error {
    Invalid,
    Unauthorized,
    Forbidden,
    FileManaged,
    Unavailable,
    CredentialMissing,
    CredentialRejected,
    Disabled,
    PurchasesUnavailable,
}
type Result<T> = std::result::Result<T, Error>;

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct Policy {
    pub revision: i64,
    pub enabled: bool,
    pub sms_enabled: bool,
    pub email_enabled: bool,
    pub price_multiplier: String,
    #[serde(default)]
    pub updated_by: Option<i64>,
    #[serde(default)]
    pub updated_at: Option<i64>,
}

fn multiplier_valid(value: &str) -> bool {
    if value.is_empty() || value.len() > 11 {
        return false;
    }
    let (whole, fraction) = value.split_once('.').unwrap_or((value, ""));
    if whole.is_empty()
        || !whole.bytes().all(|v| v.is_ascii_digit())
        || (value.contains('.') && fraction.is_empty())
        || fraction.len() > 6
        || !fraction.bytes().all(|v| v.is_ascii_digit())
    {
        return false;
    }
    let Some(integer) = whole.parse::<u64>().ok() else {
        return false;
    };
    let fractional = fraction.parse::<u64>().unwrap_or(0) * 10u64.pow(6 - fraction.len() as u32);
    integer
        .checked_mul(1_000_000)
        .and_then(|v| v.checked_add(fractional))
        .is_some_and(|v| (1..=1_000_000_000).contains(&v))
}

fn read_policy(path: &Path) -> Result<Policy> {
    let bytes = crate::config::private_file(path, 4096).map_err(|_| Error::Unavailable)?;
    let policy: Policy = serde_json::from_slice(&bytes).map_err(|_| Error::Unavailable)?;
    if policy.revision < 0 || !multiplier_valid(&policy.price_multiplier) {
        return Err(Error::Unavailable);
    }
    Ok(policy)
}

#[derive(Clone)]
pub struct PolicyFile {
    path: PathBuf,
}

impl PolicyFile {
    pub fn open(path: PathBuf) -> Result<Self> {
        if !path.is_absolute() {
            return Err(Error::Invalid);
        }
        read_policy(&path)?;
        Ok(Self { path })
    }

    pub async fn read(&self) -> Result<Policy> {
        let path = self.path.clone();
        tokio::task::spawn_blocking(move || read_policy(&path))
            .await
            .map_err(|_| Error::Unavailable)?
    }
}

#[derive(Clone)]
pub struct Service {
    pub(crate) pool: sqlx::PgPool,
    pub(crate) policy: PolicyFile,
    credential_file: Option<PathBuf>,
    client: reqwest::Client,
}

impl Service {
    pub fn new(
        pool: sqlx::PgPool,
        policy: PolicyFile,
        credential_file: Option<PathBuf>,
    ) -> Result<Self> {
        let client = crate::http_client::builder()
            .redirect(reqwest::redirect::Policy::none())
            .no_proxy()
            .connect_timeout(Duration::from_secs(2))
            .timeout(Duration::from_secs(3))
            .build()
            .map_err(|_| Error::Unavailable)?;
        let service = Self {
            pool,
            policy,
            credential_file,
            client,
        };
        if service.credential_file.is_some() {
            service.credential()?;
        }
        Ok(service)
    }

    pub(crate) fn credential(&self) -> Result<String> {
        let path = self
            .credential_file
            .as_ref()
            .ok_or(Error::CredentialMissing)?;
        let raw = crate::config::private_file(path, 4096).map_err(|_| Error::Unavailable)?;
        let value = std::str::from_utf8(&raw)
            .map_err(|_| Error::Invalid)?
            .trim();
        if !(16..=256).contains(&value.len()) || !value.bytes().all(|v| v.is_ascii_graphic()) {
            return Err(Error::Invalid);
        }
        Ok(value.to_owned())
    }

    pub(crate) fn credential_configured(&self) -> Result<bool> {
        match self.credential() {
            Ok(_) => Ok(true),
            Err(Error::CredentialMissing) => Ok(false),
            Err(error) => Err(error),
        }
    }

    pub(crate) async fn test_connection(&self) -> Result<()> {
        // Same read-only SMS-Activate endpoint as the fixed go-v0.2.118 client.
        // Credentials never enter a returned error, log or browser response.
        self.test_connection_at("https://hero-sms.com/stubs/handler_api.php")
            .await
    }

    async fn test_connection_at(&self, endpoint: &str) -> Result<()> {
        let credential = self.credential()?;
        let mut response = self
            .client
            .get(endpoint)
            .query(&[("action", "getBalance"), ("api_key", credential.as_str())])
            .header(reqwest::header::ACCEPT, "text/plain")
            .send()
            .await
            .map_err(|_| Error::Unavailable)?;
        if matches!(response.status().as_u16(), 401 | 403) {
            return Err(Error::CredentialRejected);
        }
        if response.status() != reqwest::StatusCode::OK {
            return Err(Error::Unavailable);
        }
        let mut bytes = Vec::new();
        while let Some(chunk) = response.chunk().await.map_err(|_| Error::Unavailable)? {
            if bytes.len() + chunk.len() > 4096 {
                return Err(Error::Unavailable);
            }
            bytes.extend_from_slice(&chunk);
        }
        let body = std::str::from_utf8(&bytes)
            .map_err(|_| Error::Unavailable)?
            .trim();
        if body == "BAD_KEY" {
            return Err(Error::CredentialRejected);
        }
        let balance = body
            .strip_prefix("ACCESS_BALANCE:")
            .ok_or(Error::Unavailable)?;
        if balance.is_empty()
            || balance.len() > 32
            || !balance.bytes().all(|b| b.is_ascii_digit() || b == b'.')
            || balance.bytes().filter(|b| *b == b'.').count() > 1
            || !balance.bytes().any(|b| b.is_ascii_digit())
        {
            return Err(Error::Unavailable);
        }
        Ok(())
    }
}

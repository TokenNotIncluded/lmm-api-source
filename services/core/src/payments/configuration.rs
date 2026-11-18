//! Owner-managed edits for unused, disabled LDC connections.
use super::*;
use serde::{Deserialize, Serialize};
use std::{path::PathBuf, sync::RwLock};
use tokio::sync::Mutex;
pub mod http;
mod storage;

pub const CONTRACT: &str = "core.epay-configuration.v1";
type ConfigurationResult<T> = std::result::Result<T, ConfigurationError>;
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ConfigurationError {
    Invalid,
    Denied,
    NotFound,
    Conflict,
    Unavailable,
    DurabilityUnknown,
}
impl From<Error> for ConfigurationError {
    fn from(error: Error) -> Self {
        match error {
            Error::Invalid => Self::Invalid,
            Error::Denied => Self::Denied,
            Error::Conflict => Self::Conflict,
            Error::NotFound => Self::NotFound,
            Error::Unavailable | Error::Storage => Self::Unavailable,
        }
    }
}
impl From<sqlx::Error> for ConfigurationError {
    fn from(_: sqlx::Error) -> Self {
        Self::Unavailable
    }
}
impl From<IdentityError> for ConfigurationError {
    fn from(error: IdentityError) -> Self {
        Self::from(Error::from(error))
    }
}
#[derive(Clone, Serialize)]
pub struct EPayBinding {
    pub channel_id: i64,
    pub merchant_id: String,
    pub environment: String,
    pub currency: String,
    pub payment_type: String,
    pub endpoint: String,
    pub notify_url: String,
    pub return_url: String,
    pub key_configured: bool,
    pub enabled: bool,
    pub editable: bool,
    pub credits_per_minor: String,
    pub max_amount_minor: String,
    pub min_amount_minor: String,
}
#[derive(Clone, Serialize)]
pub struct EPayConfiguration {
    pub contract: &'static str,
    pub revision: String,
    pub bindings: Vec<EPayBinding>,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct EPayUpdate {
    pub expected_revision: String,
    pub endpoint: String,
    pub notify_url: String,
    pub return_url: String,
    #[serde(default)]
    pub secret_key: String,
}
#[derive(Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Overlay {
    version: u32,
    base_sha256: String,
    bindings: BTreeMap<i64, Override>,
}
#[derive(Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Override {
    endpoint: String,
    notify_url: String,
    return_url: String,
    secret_file: Option<String>,
}
struct Snapshot {
    channels: Arc<BTreeMap<i64, Channel>>,
    revision: String,
    overlay: Overlay,
    disk_sha256: Option<String>,
}
pub(super) struct Runtime {
    path: PathBuf,
    state_path: PathBuf,
    base_sha256: String,
    writable: bool,
    live: RwLock<Arc<Snapshot>>,
    edits: Mutex<()>,
}
fn eligible(channel: &Channel) -> bool {
    channel.provider == "epay"
        && channel.currency == "LDC"
        && channel.verification.payment_type == "epay"
        && !channel.enabled()
}
fn revision(base: &str, bytes: &[u8], channels: &BTreeMap<i64, Channel>) -> String {
    let mut digest = Sha256::new();
    digest.update(b"core.epay-configuration.v1\0");
    digest.update(base.as_bytes());
    digest.update(bytes);
    for (id, channel) in channels {
        digest.update(
            json!([
                id,
                channel.provider,
                channel.merchant,
                channel.environment,
                channel.currency,
                channel.enabled(),
                channel.credits_per_minor,
                channel.max_amount_minor,
                channel.min_amount_minor(),
                channel.verification.payment_type,
                channel.verification.endpoint,
                channel.verification.notify_url,
                channel.verification.return_url,
            ])
            .to_string()
            .as_bytes(),
        );
        for secret in &channel.webhook_secrets {
            digest.update(Sha256::digest(secret.as_bytes()));
        }
    }
    crate::hex_codec::encode(digest.finalize())
}
async fn history(c: &mut PgConnection, id: i64) -> ConfigurationResult<bool> {
    Ok(sqlx::query_scalar(
        "SELECT EXISTS(SELECT FROM core_payments.intents WHERE channel_id=$1)
         OR EXISTS(SELECT FROM core_payments.legacy_bases WHERE channel_id=$1)
         OR EXISTS(SELECT FROM core_payments.accepted_proofs WHERE channel_id=$1)
         OR EXISTS(SELECT FROM core_payments.provider_events WHERE channel_id=$1)
         OR EXISTS(SELECT FROM core_payments.provider_refunds WHERE channel_id=$1)
         OR EXISTS(SELECT FROM core_payments.refund_dispatch_bindings WHERE channel_id=$1)
         OR EXISTS(SELECT FROM core_subscriptions.external_prices WHERE channel_id=$1)
         OR EXISTS(SELECT FROM core_subscriptions.external_orders WHERE channel_id=$1)",
    )
    .bind(id)
    .fetch_one(c)
    .await?)
}
async fn owner(c: &mut PgConnection, credential: &str) -> ConfigurationResult<identity::Principal> {
    let actor = discounts::session(c, credential, true).await?;
    if actor.platform_level != 6 {
        return Err(ConfigurationError::Denied);
    }
    Ok(actor)
}
impl Runtime {
    fn snapshot(&self) -> ConfigurationResult<Arc<Snapshot>> {
        self.live
            .read()
            .map(|snapshot| snapshot.clone())
            .map_err(|_| ConfigurationError::Unavailable)
    }
}
impl PaymentsStore {
    /// Freeze the shared live Arc once at each HTTP/RPC request boundary.
    /// An admitted operation keeps its original verification key throughout.
    pub fn freeze(&self) -> Result<Self> {
        if self.frozen {
            return Ok(self.clone());
        }
        let mut frozen = self.clone();
        frozen.frozen = true;
        if let Some(runtime) = &self.configuration {
            frozen.channels = runtime
                .snapshot()
                .map_err(|_| Error::Unavailable)?
                .channels
                .clone();
        }
        Ok(frozen)
    }
    /// Load the private overlay before sharing one store with HTTP and RPC.
    pub fn with_configuration_file(self, path: PathBuf, document: &[u8]) -> Result<Self> {
        let runtime =
            Runtime::load(path, document, self.channels.clone()).map_err(|_| Error::Unavailable)?;
        let channels = runtime
            .snapshot()
            .map_err(|_| Error::Unavailable)?
            .channels
            .clone();
        Ok(Self {
            channels,
            configuration: Some(Arc::new(runtime)),
            frozen: false,
            ..self
        })
    }
    async fn configuration_projection_on(
        &self,
        c: &mut PgConnection,
    ) -> ConfigurationResult<EPayConfiguration> {
        let (channels, current_revision, writable) = match &self.configuration {
            Some(runtime) => {
                let snapshot = runtime.snapshot()?;
                (
                    snapshot.channels.clone(),
                    snapshot.revision.clone(),
                    runtime.writable,
                )
            }
            None => (
                self.channels.clone(),
                revision("", b"", &self.channels),
                false,
            ),
        };
        let mut bindings = Vec::new();
        for channel in channels
            .values()
            .filter(|channel| channel.provider == "epay")
        {
            let editable = writable && eligible(channel) && !history(c, channel.id).await?;
            bindings.push(EPayBinding {
                channel_id: channel.id,
                merchant_id: channel.merchant.clone(),
                environment: channel.environment.clone(),
                currency: channel.currency.clone(),
                payment_type: channel.verification.payment_type.clone(),
                endpoint: channel.verification.endpoint.clone(),
                notify_url: channel.verification.notify_url.clone(),
                return_url: channel.verification.return_url.clone(),
                key_configured: !channel.webhook_secrets.is_empty(),
                enabled: channel.enabled(),
                editable,
                credits_per_minor: channel.credits_per_minor.to_string(),
                max_amount_minor: channel.max_amount_minor.to_string(),
                min_amount_minor: channel.min_amount_minor().to_string(),
            });
        }
        Ok(EPayConfiguration {
            contract: CONTRACT,
            revision: current_revision,
            bindings,
        })
    }
    /// Internal read-only service projection; never emits key bytes or paths.
    pub async fn epay_configuration(&self) -> ConfigurationResult<EPayConfiguration> {
        let mut tx = self.begin().await?;
        let projection = self.configuration_projection_on(&mut tx).await?;
        tx.commit().await?;
        Ok(projection)
    }
    pub async fn owner_epay_configuration(
        &self,
        credential: &str,
    ) -> ConfigurationResult<EPayConfiguration> {
        let mut tx = self.begin().await?;
        owner(&mut tx, credential).await?;
        let projection = self.configuration_projection_on(&mut tx).await?;
        tx.commit().await?;
        Ok(projection)
    }
    pub async fn update_epay_configuration(
        &self,
        credential: &str,
        id: i64,
        input: EPayUpdate,
    ) -> ConfigurationResult<EPayConfiguration> {
        let runtime = self
            .configuration
            .as_ref()
            .ok_or(ConfigurationError::Unavailable)?;
        let _edit = runtime.edits.lock().await;
        let current = runtime.snapshot()?;
        let mut tx = self.begin().await?;
        let actor = owner(&mut tx, credential).await?;
        if input.expected_revision != current.revision {
            return Err(ConfigurationError::Conflict);
        }
        let old = current
            .channels
            .get(&id)
            .ok_or(ConfigurationError::NotFound)?;
        if !runtime.writable || !eligible(old) || history(&mut tx, id).await? {
            return Err(ConfigurationError::Conflict);
        }
        let prepared = runtime.prepare(&current, id, input)?;
        // Audit authorization before file publication. Failure here changes nothing.
        identity::audit(
            &mut tx,
            &actor,
            "payment_configuration.prepare",
            json!({"channel_id":id,"before":current.revision,"after":prepared.snapshot.revision}),
        )
        .await?;
        tx.commit().await?;
        runtime.persist(&current, &prepared)?;
        // PostgreSQL and the filesystem cannot share a commit. A failed final
        // receipt explicitly requires readback, never a blind replay of PUT.
        let completion: ConfigurationResult<EPayConfiguration> = async {
            let mut tx = self.begin().await?;
            identity::audit(
                &mut tx,
                &actor,
                "payment_configuration.apply",
                json!({"channel_id":id,"revision":prepared.snapshot.revision}),
            )
            .await?;
            let result = self.configuration_projection_on(&mut tx).await?;
            tx.commit().await?;
            Ok(result)
        }
        .await;
        completion.map_err(|_| ConfigurationError::DurabilityUnknown)
    }
}
/// Offline tools observe the same persisted channel overlay as server startup.
pub fn configured_channels(
    path: PathBuf,
    document: &[u8],
    channels: Vec<Channel>,
) -> Result<Vec<Channel>> {
    let base = PaymentsStore::compile_channels(channels)?;
    let runtime = Runtime::load(path, document, base).map_err(|_| Error::Unavailable)?;
    Ok(runtime
        .snapshot()
        .map_err(|_| Error::Unavailable)?
        .channels
        .values()
        .cloned()
        .collect())
}
#[cfg(test)]
mod tests;

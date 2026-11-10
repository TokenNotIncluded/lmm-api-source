use super::{
    FederatedKeys, FederatedProvider, ProviderConfiguration, ProviderKind, network::ProviderNetwork,
};
use crate::identity::{IdentityError, IdentityStore, Result};
use sqlx::{PgConnection, Row, postgres::PgRow};
use std::sync::Arc;

pub(crate) struct FederatedAuth {
    pub(super) keys: FederatedKeys,
    pub(super) network: ProviderNetwork,
}

pub(super) fn provider_row(row: PgRow) -> Result<FederatedProvider> {
    let configuration = serde_json::from_value(row.try_get("configuration")?)
        .map_err(|_| IdentityError::Storage)?;
    Ok(FederatedProvider {
        id: row.try_get("id")?,
        kind: ProviderKind::parse(&row.try_get::<String, _>("kind")?)?,
        slug: row.try_get("slug")?,
        name: row.try_get("name")?,
        icon: row.try_get("icon")?,
        enabled: row.try_get("enabled")?,
        configuration,
        secret_context: row.try_get("secret_context")?,
        secret_key_id: row.try_get("secret_key_id")?,
        sealed_secret: row.try_get("sealed_secret")?,
        revision: row.try_get("revision")?,
        deleted: row
            .try_get::<Option<chrono::DateTime<chrono::Utc>>, _>("deleted_at")?
            .is_some(),
    })
}

impl IdentityStore {
    pub fn with_federated_keys(
        mut self,
        active: String,
        keys: Vec<(String, Vec<u8>)>,
        replay_key: Vec<u8>,
    ) -> Result<Self> {
        let keys = FederatedKeys::new(active, keys, replay_key)?;
        if self.federated.is_some() {
            return Err(IdentityError::Invalid);
        }
        self.federated = Some(Arc::new(FederatedAuth {
            keys,
            network: ProviderNetwork::new(),
        }));
        Ok(self)
    }

    pub fn federated_auth_enabled(&self) -> bool {
        self.federated.is_some()
    }

    pub(crate) fn federated_auth(&self) -> Result<&FederatedAuth> {
        self.federated.as_deref().ok_or(IdentityError::Forbidden)
    }

    pub(crate) async fn federated_provider_reference(
        &self,
        reference: &str,
    ) -> Result<FederatedProvider> {
        self.federated_auth()?;
        if matches!(reference, "oidc" | "wechat") {
            return self.federated_provider(reference).await;
        }
        let reference = reference.strip_prefix("federated:").unwrap_or(reference);
        let id = reference
            .parse::<i64>()
            .map_err(|_| IdentityError::Invalid)?;
        if id <= 0 || id.to_string() != reference {
            return Err(IdentityError::Invalid);
        }
        let mut connection = self.pool.acquire().await?;
        self.federated_provider_in(&mut connection, id).await
    }

    pub(crate) async fn federated_catalog(&self) -> Result<serde_json::Value> {
        self.federated_auth()?;
        public_catalog(&self.pool).await
    }

    pub(crate) fn validate_federated_configuration(
        &self,
        kind: ProviderKind,
        configuration: &mut ProviderConfiguration,
    ) -> Result<()> {
        configuration.normalize(kind)?;
        let network = &self.federated_auth()?.network;
        if kind == ProviderKind::Wechat {
            let bridge = network.bridge_endpoint(&configuration.bridge_url)?;
            if bridge.query().is_some() {
                return Err(IdentityError::Invalid);
            }
        } else {
            for endpoint in [
                &configuration.authorization_endpoint,
                &configuration.token_endpoint,
                &configuration.user_info_endpoint,
            ] {
                network.endpoint(endpoint)?;
            }
            if !configuration.well_known.is_empty() {
                network.endpoint(&configuration.well_known)?;
            }
            if kind == ProviderKind::Oidc {
                let issuer = network.endpoint(&configuration.issuer)?;
                if issuer.query().is_some() {
                    return Err(IdentityError::Invalid);
                }
                network.endpoint(&configuration.jwks_uri)?;
            }
        }
        Ok(())
    }

    pub async fn check_federated_auth_ready(&self) -> Result<()> {
        if !self.federated_auth_enabled() {
            return Ok(());
        }
        let rows = sqlx::query("SELECT * FROM core_federated_auth.providers WHERE deleted_at IS NULL ORDER BY id LIMIT 1001").fetch_all(&self.pool).await?;
        if rows.len() > 1000 {
            return Err(IdentityError::Storage);
        }
        for row in rows {
            let provider = provider_row(row)?;
            let mut configuration = provider.configuration.clone();
            self.validate_federated_configuration(provider.kind, &mut configuration)?;
            self.federated_secret(&provider)?;
        }
        Ok(())
    }

    pub(crate) fn federated_secret(
        &self,
        provider: &FederatedProvider,
    ) -> Result<zeroize::Zeroizing<String>> {
        self.federated_auth()?.keys.open(
            &provider.secret_context,
            provider.revision,
            &provider.secret_key_id,
            &provider.sealed_secret,
        )
    }

    pub(crate) fn seal_federated_secret(
        &self,
        context: &[u8],
        revision: i64,
        secret: &str,
    ) -> Result<(String, Vec<u8>)> {
        self.federated_auth()?.keys.seal(context, revision, secret)
    }

    pub(crate) async fn managed_federated_provider(
        &self,
        connection: &mut PgConnection,
        id: i64,
    ) -> Result<FederatedProvider> {
        self.federated_auth()?;
        let row = sqlx::query("SELECT * FROM core_federated_auth.providers WHERE id=$1 AND deleted_at IS NULL FOR UPDATE")
            .bind(id).fetch_optional(connection).await?.ok_or(IdentityError::Forbidden)?;
        provider_row(row)
    }

    pub(crate) async fn federated_provider_descriptor(
        &self,
        connection: &mut PgConnection,
        id: i64,
    ) -> Result<serde_json::Value> {
        let row = sqlx::query("SELECT * FROM core_federated_auth.providers WHERE id=$1")
            .bind(id)
            .fetch_optional(connection)
            .await?
            .ok_or(IdentityError::Forbidden)?;
        provider_row(row)?.admin_descriptor()
    }

    pub(crate) fn validate_federated_provider(
        &self,
        kind: ProviderKind,
        name: &str,
        slug: &str,
        icon: &str,
        configuration: &mut ProviderConfiguration,
    ) -> Result<()> {
        super::management::names(kind, name, slug, icon)?;
        self.validate_federated_configuration(kind, configuration)
    }

    pub(crate) fn federated_operation_digest(
        &self,
        action: &str,
        target: i64,
        input: &super::ProviderWrite,
    ) -> Result<Vec<u8>> {
        let bytes =
            zeroize::Zeroizing::new(serde_json::to_vec(input).map_err(|_| IdentityError::Invalid)?);
        self.federated_auth()?
            .keys
            .operation_digest(action, target, &bytes)
    }

    pub(crate) async fn discover_federated(
        &self,
        url: &str,
        issuer: Option<&str>,
    ) -> Result<serde_json::Value> {
        super::protocol::discover(&self.federated_auth()?.network, url, issuer).await
    }

    pub(crate) async fn federated_provider(&self, slug: &str) -> Result<FederatedProvider> {
        self.federated_auth()?;
        if slug.is_empty()
            || slug.len() > 64
            || !slug
                .bytes()
                .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-')
        {
            return Err(IdentityError::Invalid);
        }
        let row = sqlx::query(
            "SELECT * FROM core_federated_auth.providers WHERE slug=$1 AND deleted_at IS NULL",
        )
        .bind(slug)
        .fetch_optional(&self.pool)
        .await?
        .ok_or(IdentityError::Forbidden)?;
        let provider = provider_row(row)?;
        if !provider.enabled {
            return Err(IdentityError::Forbidden);
        }
        Ok(provider)
    }

    pub(crate) async fn federated_provider_in(
        &self,
        connection: &mut PgConnection,
        id: i64,
    ) -> Result<FederatedProvider> {
        self.federated_auth()?;
        let row = sqlx::query("SELECT * FROM core_federated_auth.providers WHERE id=$1 FOR SHARE")
            .bind(id)
            .fetch_optional(connection)
            .await?
            .ok_or(IdentityError::Forbidden)?;
        let provider = provider_row(row)?;
        if !provider.enabled || provider.deleted {
            return Err(IdentityError::Forbidden);
        }
        Ok(provider)
    }
}

pub(crate) async fn public_catalog(pool: &sqlx::PgPool) -> Result<serde_json::Value> {
    let rows = sqlx::query("SELECT id,kind,slug,name,icon,revision,configuration FROM core_federated_auth.providers WHERE enabled AND deleted_at IS NULL ORDER BY id LIMIT 1001")
        .fetch_all(pool).await?;
    if rows.len() > 1000 {
        return Err(IdentityError::Storage);
    }
    let mut providers = Vec::with_capacity(rows.len());
    for row in rows {
        let configuration: ProviderConfiguration =
            serde_json::from_value(row.try_get("configuration")?)
                .map_err(|_| IdentityError::Storage)?;
        providers.push(serde_json::json!({"id":row.try_get::<i64,_>("id")?.to_string(),"kind":row.try_get::<String,_>("kind")?,"slug":row.try_get::<String,_>("slug")?,"name":row.try_get::<String,_>("name")?,"icon":row.try_get::<String,_>("icon")?,"revision":row.try_get::<i64,_>("revision")?.to_string(),"client_id":configuration.client_id,"authorization_endpoint":configuration.authorization_endpoint,"scopes":configuration.scopes,"qr_code_url":configuration.qr_code_url,"native_federated":true}));
    }
    Ok(serde_json::Value::Array(providers))
}

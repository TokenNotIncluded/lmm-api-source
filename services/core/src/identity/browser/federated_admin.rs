//! The old provider manager is root-only, with native recent-Session checks.
use super::*;
use crate::identity::federated::{
    FederatedKeys, ProviderConfiguration, ProviderKind, ProviderWrite,
};
use zeroize::Zeroizing;

async fn bounded(pool: &PgPool) -> Result<sqlx::Transaction<'_, sqlx::Postgres>> {
    let mut tx = pool.begin().await?;
    sqlx::query("SET LOCAL statement_timeout='10s'")
        .execute(&mut *tx)
        .await?;
    sqlx::query("SET LOCAL lock_timeout='500ms'")
        .execute(&mut *tx)
        .await?;
    Ok(tx)
}

impl BrowserAuth {
    async fn federated_root(
        &self,
        connection: &mut PgConnection,
        access: &str,
        recent: bool,
    ) -> BrowserResult<(Principal, String)> {
        self.store.federated_auth()?;
        let (actor, sid) = self.security_actor(connection, access, recent).await?;
        let role: String = sqlx::query_scalar(
            "SELECT platform_role FROM core_identity.users WHERE id=$1 FOR SHARE",
        )
        .bind(actor.user_id)
        .fetch_one(connection)
        .await?;
        if role != "superadmin" {
            return Err(IdentityError::Forbidden.into());
        }
        Ok((actor, sid))
    }

    pub(crate) async fn federated_providers_admin(
        &self,
        access: &str,
        custom_only: bool,
        id: Option<i64>,
    ) -> BrowserResult<Value> {
        let mut tx = bounded(&self.store.pool).await?;
        self.federated_root(&mut tx, access, false).await?;
        let ids: Vec<i64> = sqlx::query_scalar("SELECT id FROM core_federated_auth.providers WHERE deleted_at IS NULL AND (NOT $1 OR kind='custom_oauth') AND ($2::bigint IS NULL OR id=$2) ORDER BY id LIMIT 1001")
            .bind(custom_only).bind(id).fetch_all(&mut *tx).await?;
        if ids.len() > 1000 {
            return Err(IdentityError::Storage.into());
        }
        let mut providers = Vec::with_capacity(ids.len());
        for id in ids {
            providers.push(
                self.store
                    .federated_provider_descriptor(&mut tx, id)
                    .await?,
            );
        }
        tx.commit().await?;
        if id.is_some() {
            providers.pop().ok_or(IdentityError::Forbidden.into())
        } else {
            Ok(json!(providers))
        }
    }

    pub(crate) async fn federated_discovery_admin(
        &self,
        access: &str,
        url: &str,
        issuer: Option<&str>,
    ) -> BrowserResult<Value> {
        let mut tx = bounded(&self.store.pool).await?;
        let (actor, sid) = self.federated_root(&mut tx, access, true).await?;
        tx.commit().await?;
        let result = self.store.discover_federated(url, issuer).await?;
        let mut tx = bounded(&self.store.pool).await?;
        let (current, current_sid) = self.federated_root(&mut tx, access, true).await?;
        if current.user_id != actor.user_id
            || current.user_version != actor.user_version
            || current_sid != sid
        {
            return Err(IdentityError::Unauthorized.into());
        }
        tx.commit().await?;
        Ok(result)
    }

    pub(crate) async fn mutate_federated_provider(
        &self,
        access: &str,
        action: &str,
        target: Option<i64>,
        custom_only: bool,
        input: &ProviderWrite,
    ) -> BrowserResult<Value> {
        input.validate_operation()?;
        if !matches!(action, "create" | "update" | "delete")
            || (action == "create") != target.is_none()
        {
            return Err(IdentityError::Invalid.into());
        }
        if let (Some(id), Some(target)) = (&input.id, target) {
            if id != &target.to_string() {
                return Err(IdentityError::Invalid.into());
            }
        }
        let request_digest =
            self.store
                .federated_operation_digest(action, target.unwrap_or(0), input)?;
        let mut tx = bounded(&self.store.pool).await?;
        let (actor, sid) = self.federated_root(&mut tx, access, true).await?;
        if let Some(receipt) = sqlx::query("SELECT sid,request_digest,action,provider_id,provider_revision FROM core_federated_auth.operations WHERE actor_user_id=$1 AND operation_key=$2")
            .bind(actor.user_id).bind(&input.operation_key).fetch_optional(&mut *tx).await? {
            let id:i64=receipt.try_get("provider_id")?;
            if receipt.try_get::<String,_>("sid")?!=sid || receipt.try_get::<Vec<u8>,_>("request_digest")?!=request_digest
                || receipt.try_get::<String,_>("action")?!=action || target.is_some_and(|target|target!=id) { return Err(IdentityError::Conflict.into()); }
            let mut descriptor = self.store.federated_provider_descriptor(&mut tx,id).await?;
            descriptor["applied_revision"]=json!(receipt.try_get::<i64,_>("provider_revision")?.to_string());
            descriptor["operation_key"]=json!(input.operation_key);
            descriptor["replayed"]=json!(true);
            tx.commit().await?;
            return Ok(descriptor);
        }
        let (id, revision) = if let Some(id) = target {
            let current = self.store.managed_federated_provider(&mut tx, id).await?;
            if custom_only && current.kind != ProviderKind::CustomOauth
                || input.kind.is_some_and(|kind| kind != current.kind)
            {
                return Err(IdentityError::Invalid.into());
            }
            let expected = super::account_links::version(
                input
                    .expected_revision
                    .as_deref()
                    .ok_or(IdentityError::Invalid)?,
                false,
            )?;
            if expected != current.revision {
                return Err(IdentityError::Conflict.into());
            }
            let revision = current
                .revision
                .checked_add(1)
                .ok_or(IdentityError::Conflict)?;
            if action == "delete" {
                let bound:bool=sqlx::query_scalar("SELECT EXISTS(SELECT FROM core_federated_auth.identities WHERE provider_id=$1 AND revoked_at IS NULL)")
                    .bind(id).fetch_one(&mut *tx).await?;
                if bound {
                    return Err(IdentityError::Conflict.into());
                }
                // Reseal even on tombstone: every envelope is revision-bound.
                let secret = self.store.federated_secret(&current)?;
                let (key, envelope) =
                    self.store
                        .seal_federated_secret(&current.secret_context, revision, &secret)?;
                sqlx::query("UPDATE core_federated_auth.providers SET enabled=false,deleted_at=clock_timestamp(),updated_at=clock_timestamp(),revision=$2,secret_key_id=$3,sealed_secret=$4 WHERE id=$1")
                    .bind(id).bind(revision).bind(key).bind(envelope).execute(&mut *tx).await?;
            } else {
                let mut configuration = input.configuration(&current.configuration)?;
                let name = input
                    .name
                    .as_deref()
                    .filter(|value| !value.is_empty())
                    .unwrap_or(&current.name);
                let slug = input
                    .slug
                    .as_deref()
                    .filter(|value| !value.is_empty())
                    .unwrap_or(&current.slug);
                let icon = input.icon.as_deref().unwrap_or(&current.icon);
                self.store.validate_federated_provider(
                    current.kind,
                    name,
                    slug,
                    icon,
                    &mut configuration,
                )?;
                let secret = if input.client_secret.is_empty() {
                    self.store.federated_secret(&current)?
                } else {
                    Zeroizing::new(input.client_secret.clone())
                };
                let (key, envelope) =
                    self.store
                        .seal_federated_secret(&current.secret_context, revision, &secret)?;
                sqlx::query("UPDATE core_federated_auth.providers SET name=$2,slug=$3,icon=$4,enabled=$5,configuration=$6,secret_key_id=$7,sealed_secret=$8,revision=$9,updated_at=clock_timestamp() WHERE id=$1")
                    .bind(id).bind(name).bind(slug).bind(icon).bind(input.enabled.unwrap_or(current.enabled))
                    .bind(serde_json::to_value(configuration).map_err(|_|IdentityError::Invalid)?)
                    .bind(key).bind(envelope).bind(revision).execute(&mut *tx).await?;
            }
            (id, revision)
        } else {
            let kind = input.kind.unwrap_or(ProviderKind::CustomOauth);
            if custom_only && kind != ProviderKind::CustomOauth {
                return Err(IdentityError::Invalid.into());
            }
            let name = input.name.as_deref().ok_or(IdentityError::Invalid)?;
            let slug = input.slug.as_deref().ok_or(IdentityError::Invalid)?;
            let icon = input.icon.as_deref().unwrap_or("");
            let mut configuration = input.configuration(&ProviderConfiguration::default())?;
            self.store
                .validate_federated_provider(kind, name, slug, icon, &mut configuration)?;
            let context = FederatedKeys::context()?;
            let (key, envelope) =
                self.store
                    .seal_federated_secret(&context, 1, &input.client_secret)?;
            let id:i64=sqlx::query_scalar("INSERT INTO core_federated_auth.providers(kind,slug,name,icon,enabled,configuration,secret_context,secret_key_id,sealed_secret) VALUES($1,$2,$3,$4,$5,$6,$7,$8,$9) RETURNING id")
                .bind(kind.as_str()).bind(slug).bind(name).bind(icon).bind(input.enabled.unwrap_or(false))
                .bind(serde_json::to_value(configuration).map_err(|_|IdentityError::Invalid)?)
                .bind(context.as_slice()).bind(key).bind(envelope).fetch_one(&mut *tx).await?;
            (id, 1)
        };
        sqlx::query("INSERT INTO core_federated_auth.operations(actor_user_id,operation_key,sid,request_digest,action,provider_id,provider_revision) VALUES($1,$2,$3,$4,$5,$6,$7)")
            .bind(actor.user_id).bind(&input.operation_key).bind(sid).bind(request_digest).bind(action).bind(id).bind(revision).execute(&mut *tx).await?;
        audit(&mut tx,&actor,"auth.provider_configuration",json!({"provider_id":id.to_string(),"revision":revision.to_string(),"operation":action})).await?;
        let mut descriptor = self
            .store
            .federated_provider_descriptor(&mut tx, id)
            .await?;
        descriptor["applied_revision"] = json!(revision.to_string());
        descriptor["operation_key"] = json!(input.operation_key);
        descriptor["replayed"] = json!(false);
        tx.commit().await?;
        Ok(descriptor)
    }
}

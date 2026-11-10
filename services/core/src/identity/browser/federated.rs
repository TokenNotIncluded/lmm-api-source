//! Federated login/binding uses the native browser family and local MFA.
use super::account_links::PrimaryExclusion;
use super::*;
use crate::identity::federated::{
    ExchangeResult, FederatedStart, FlowOwner, FlowPurpose, ProviderKind,
};

pub(crate) enum FederatedLoginOutcome {
    Login(BrowserLoginOutcome),
    Denied(String),
}
pub(crate) enum FederatedBindOutcome {
    Session(BrowserBundle),
    Denied(String),
}

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
    fn federated_binding(&self, provider: i64, binding: &str) -> Result<String> {
        if binding.len() != 64 || !binding.bytes().all(|b| b.is_ascii_hexdigit()) {
            return Err(IdentityError::Unauthorized);
        }
        self.derived_secret(
            b"browser-federated-flow-v1",
            &provider.to_string(),
            binding,
            "",
        )
    }

    pub(crate) async fn begin_federated_login(
        &self,
        reference: &str,
        binding: &str,
        origin: &str,
        accepted_legal: bool,
        affiliate: &str,
    ) -> BrowserResult<FederatedStart> {
        let provider = self.store.federated_provider_reference(reference).await?;
        let binding = self.federated_binding(provider.id, binding)?;
        let mut tx = bounded(&self.store.pool).await?;
        let start = self
            .store
            .create_federated_flow(
                &mut tx,
                &provider,
                &binding,
                origin,
                FlowPurpose::Login,
                None,
                accepted_legal,
                affiliate,
            )
            .await?;
        tx.commit().await?;
        Ok(start)
    }

    pub(crate) async fn finish_federated_login(
        &self,
        reference: &str,
        state: &str,
        binding: &str,
        origin: &str,
        code: &str,
        issuer: Option<&str>,
        metadata: &BrowserSessionMetadata,
        mfa_binding: &str,
    ) -> super::super::mfa::MfaResult<FederatedLoginOutcome> {
        let provider = self.store.federated_provider_reference(reference).await?;
        let binding = self.federated_binding(provider.id, binding)?;
        let mut tx = bounded(&self.store.pool).await?;
        let flow = self
            .store
            .consume_federated_flow(
                &mut tx,
                &provider.slug,
                state,
                &binding,
                FlowPurpose::Login,
                None,
                code,
            )
            .await?;
        tx.commit().await?;
        let exchange = match self
            .store
            .exchange_federated_flow(flow, origin, code, issuer)
            .await?
        {
            ExchangeResult::Denied(message) => return Ok(FederatedLoginOutcome::Denied(message)),
            ExchangeResult::Verified(exchange) => exchange,
        };
        let source = metadata.ip.parse().ok().map(|address| {
            crate::abuse::registration::RegistrationSource {
                address,
                kind: crate::abuse::registration::RegistrationSourceKind::Socket,
            }
        });
        let mut tx = bounded(&self.store.pool).await?;
        let user = self
            .store
            .federated_user(&mut tx, &exchange, source)
            .await?;
        let action = match exchange.kind() {
            ProviderKind::Oidc => "session.oidc_browser_login",
            ProviderKind::Wechat => "session.wechat_browser_login",
            ProviderKind::CustomOauth => "session.custom_oauth_browser_login",
        };
        let outcome = self
            .provider_login_outcome(&mut tx, user, metadata, mfa_binding, action)
            .await?;
        tx.commit().await?;
        Ok(FederatedLoginOutcome::Login(outcome))
    }

    pub(crate) async fn begin_federated_bind(
        &self,
        access: &str,
        reference: &str,
        binding: &str,
        origin: &str,
        expected_auth_version: &str,
    ) -> BrowserResult<FederatedStart> {
        if !self.store.account_links_enabled {
            return Err(IdentityError::Forbidden.into());
        }
        let expected = super::account_links::version(expected_auth_version, false)?;
        let provider = self.store.federated_provider_reference(reference).await?;
        let binding = self.federated_binding(provider.id, binding)?;
        let mut tx = bounded(&self.store.pool).await?;
        let (actor, sid) = self.security_actor(&mut tx, access, true).await?;
        if actor.user_version != expected {
            return Err(IdentityError::Conflict.into());
        }
        let owner = FlowOwner {
            user: actor.user_id,
            version: actor.user_version,
            sid,
        };
        let start = self
            .store
            .create_federated_flow(
                &mut tx,
                &provider,
                &binding,
                origin,
                FlowPurpose::Bind,
                Some(&owner),
                false,
                "",
            )
            .await?;
        tx.commit().await?;
        Ok(start)
    }

    pub(crate) async fn finish_federated_bind(
        &self,
        access: &str,
        reference: &str,
        state: &str,
        binding: &str,
        origin: &str,
        code: &str,
        issuer: Option<&str>,
    ) -> BrowserResult<FederatedBindOutcome> {
        if !self.store.account_links_enabled {
            return Err(IdentityError::Forbidden.into());
        }
        let provider = self.store.federated_provider_reference(reference).await?;
        let binding = self.federated_binding(provider.id, binding)?;
        let mut tx = bounded(&self.store.pool).await?;
        let (actor, sid) = self.security_actor(&mut tx, access, true).await?;
        let owner = FlowOwner {
            user: actor.user_id,
            version: actor.user_version,
            sid,
        };
        let flow = self
            .store
            .consume_federated_flow(
                &mut tx,
                &provider.slug,
                state,
                &binding,
                FlowPurpose::Bind,
                Some(&owner),
                code,
            )
            .await?;
        tx.commit().await?;
        let exchange = match self
            .store
            .exchange_federated_flow(flow, origin, code, issuer)
            .await?
        {
            ExchangeResult::Denied(message) => return Ok(FederatedBindOutcome::Denied(message)),
            ExchangeResult::Verified(exchange) => exchange,
        };
        let mut tx = bounded(&self.store.pool).await?;
        let (actor, sid) = self.security_actor(&mut tx, access, true).await?;
        if exchange.owner()
            != Some(&FlowOwner {
                user: actor.user_id,
                version: actor.user_version,
                sid: sid.clone(),
            })
        {
            return Err(IdentityError::Unauthorized.into());
        }
        auth::lock_registration(&mut tx).await?;
        self.store
            .verify_federated_exchange(&mut tx, &exchange)
            .await?;
        let digest = Sha256::digest(exchange.subject().as_bytes()).to_vec();
        let existing = sqlx::query("SELECT subject,user_id,revoked_at IS NULL AS active FROM core_federated_auth.identities WHERE provider_id=$1 AND subject_digest=$2 FOR UPDATE")
            .bind(exchange.provider_id()).bind(&digest).fetch_optional(&mut *tx).await?;
        if let Some(row) = &existing {
            if row.try_get::<String, _>("subject")? != exchange.subject()
                || row.try_get::<i64, _>("user_id")? != actor.user_id
            {
                return Err(IdentityError::Conflict.into());
            }
        }
        sqlx::query("UPDATE core_federated_auth.identities SET revoked_at=clock_timestamp(),revision=revision+1 WHERE user_id=$1 AND provider_id=$2 AND subject_digest<>$3 AND revoked_at IS NULL")
            .bind(actor.user_id).bind(exchange.provider_id()).bind(&digest).execute(&mut *tx).await?;
        if let Some(row) = existing {
            if !row.try_get::<bool, _>("active")? {
                sqlx::query("UPDATE core_federated_auth.identities SET revoked_at=NULL,revision=revision+1 WHERE provider_id=$1 AND subject_digest=$2")
                    .bind(exchange.provider_id()).bind(&digest).execute(&mut *tx).await?;
            }
        } else {
            sqlx::query("INSERT INTO core_federated_auth.identities(provider_id,subject,subject_digest,user_id) VALUES($1,$2,$3,$4)")
                .bind(exchange.provider_id()).bind(exchange.subject()).bind(&digest).bind(actor.user_id).execute(&mut *tx).await?;
        }
        let bundle = self
            .rotate_security_authority(&mut tx, &actor, &sid, "account.federated_link")
            .await?;
        tx.commit().await?;
        Ok(FederatedBindOutcome::Session(bundle))
    }

    pub(crate) async fn unlink_federated(
        &self,
        access: &str,
        provider_id: i64,
        input: &AccountUnlinkRequest,
        passkeys_enabled: bool,
    ) -> BrowserResult<BrowserBundle> {
        if !self.store.account_links_enabled || !self.store.federated_auth_enabled() {
            return Err(IdentityError::Forbidden.into());
        }
        let revision = super::account_links::version(&input.expected_revision, false)?;
        let expected = super::account_links::version(&input.expected_auth_version, false)?;
        let mut tx = bounded(&self.store.pool).await?;
        let (actor, sid) = self.security_actor(&mut tx, access, true).await?;
        if actor.user_version != expected {
            return Err(IdentityError::Conflict.into());
        }
        auth::lock_registration(&mut tx).await?;
        if !self
            .retains_primary_login(
                &mut tx,
                actor.user_id,
                PrimaryExclusion::Federated(provider_id, &input.subject),
                passkeys_enabled,
            )
            .await?
        {
            return Err(IdentityError::Forbidden.into());
        }
        let changed = sqlx::query("UPDATE core_federated_auth.identities SET revoked_at=clock_timestamp(),revision=revision+1 WHERE provider_id=$1 AND subject_digest=$2 AND subject=$3 AND user_id=$4 AND revision=$5 AND revoked_at IS NULL")
            .bind(provider_id).bind(Sha256::digest(input.subject.as_bytes()).as_slice()).bind(&input.subject).bind(actor.user_id).bind(revision)
            .execute(&mut *tx).await?.rows_affected();
        if changed != 1 {
            return Err(IdentityError::Conflict.into());
        }
        let bundle = self
            .rotate_security_authority(&mut tx, &actor, &sid, "account.federated_unlink")
            .await?;
        tx.commit().await?;
        Ok(bundle)
    }
}

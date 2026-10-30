//! Native account binding reuses verified provider exchanges and the existing
//! browser Session authority. Provider email/profile fields never select users.
use super::*;
use serde::{Deserialize, Serialize};
pub const ACCOUNT_LINKS_SCHEMA: &str = include_str!("../../../schema/account_links.sql");
const GOOGLE: &str = "https://accounts.google.com";
const TELEGRAM: &str = "https://telegram.org";

#[derive(Clone, Copy)]
pub enum AccountLinkProvider {
    Google,
    External(ExternalProvider),
}
impl AccountLinkProvider {
    pub fn parse(value: &str) -> Result<Self> {
        if value == "google" {
            Ok(Self::Google)
        } else {
            Ok(Self::External(ExternalProvider::parse(value)?))
        }
    }
    pub fn slug(self) -> &'static str {
        match self {
            Self::Google => "google",
            Self::External(p) => p.slug(),
        }
    }
    pub fn issuer(self) -> &'static str {
        match self {
            Self::Google => GOOGLE,
            Self::External(p) => p.issuer(),
        }
    }
}
#[derive(Serialize)]
pub struct AccountBindStart {
    pub authorization_url: String,
    pub state: String,
    pub expires_at: i64,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AccountBindFinish {
    pub state: String,
    pub code: String,
    pub iss: Option<String>,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AccountUnlinkRequest {
    pub issuer: String,
    pub subject: String,
    pub expected_revision: String,
    pub expected_auth_version: String,
}
pub(in crate::identity::browser) enum PrimaryExclusion<'a> {
    Passkey(i64),
    OAuth(&'a str, &'a str),
    Telegram(&'a str),
}
fn version(value: &str, zero: bool) -> Result<i64> {
    let number = value.parse::<i64>().map_err(|_| IdentityError::Invalid)?;
    if number < if zero { 0 } else { 1 } || number.to_string() != value {
        return Err(IdentityError::Invalid);
    }
    Ok(number)
}
async fn begin(pool: &PgPool) -> Result<sqlx::Transaction<'_, sqlx::Postgres>> {
    let mut tx = pool.begin().await?;
    sqlx::query("SET LOCAL statement_timeout='10s'")
        .execute(&mut *tx)
        .await?;
    sqlx::query("SET LOCAL lock_timeout='500ms'")
        .execute(&mut *tx)
        .await?;
    Ok(tx)
}
impl IdentityStore {
    pub fn with_account_links(mut self, enabled: bool) -> Self {
        self.account_links_enabled = enabled;
        self
    }
    pub fn account_links_enabled(&self) -> bool {
        self.account_links_enabled
    }
    pub(in crate::identity) async fn oauth_link_active(
        &self,
        c: &mut PgConnection,
        issuer: &str,
        subject: &str,
    ) -> Result<bool> {
        if !self.account_links_enabled {
            return Ok(true);
        }
        Ok(sqlx::query_scalar("SELECT NOT EXISTS(SELECT 1 FROM core_account_links.oauth_states WHERE issuer=$1 AND subject=$2 AND revoked_at IS NOT NULL)")
            .bind(issuer).bind(subject).fetch_one(c).await?)
    }
}
impl BrowserAuth {
    fn require_account_links(&self) -> Result<()> {
        if self.store.account_links_enabled {
            Ok(())
        } else {
            Err(IdentityError::Forbidden)
        }
    }
    fn account_bind_binding(&self, provider: AccountLinkProvider, binding: &str) -> Result<String> {
        if binding.len() != 64 || !binding.bytes().all(|byte| byte.is_ascii_hexdigit()) {
            return Err(IdentityError::Unauthorized);
        }
        self.derived_secret(b"browser-account-bind-v1", provider.slug(), binding, "")
    }
    pub async fn account_links(&self, access: &str) -> BrowserResult<Value> {
        self.require_account_links()?;
        let mut tx = begin(&self.store.pool).await?;
        let (actor, _) = self.security_actor(&mut tx, access, false).await?;
        let rows=sqlx::query("SELECT i.issuer,i.subject,COALESCE(s.revision,0) AS revision,s.revoked_at IS NULL AS active FROM core_identity.oauth_identities i LEFT JOIN core_account_links.oauth_states s USING(issuer,subject) WHERE i.user_id=$1 ORDER BY i.issuer,i.subject LIMIT 101")
            .bind(actor.user_id).fetch_all(&mut *tx).await?;
        if rows.len() > 100 {
            return Err(IdentityError::Conflict.into());
        }
        let mut links = Vec::new();
        for row in rows {
            links.push(json!({"issuer":row.try_get::<String,_>("issuer")?,"subject":row.try_get::<String,_>("subject")?,"revision":row.try_get::<i64,_>("revision")?.to_string(),"active":row.try_get::<bool,_>("active")?}));
        }
        let telegram:Option<String>=sqlx::query_scalar("SELECT subject FROM core_telegram_auth.bindings WHERE user_id=$1 AND revoked_at IS NULL").bind(actor.user_id).fetch_optional(&mut *tx).await?;
        if let Some(subject) = telegram {
            links.push(json!({"issuer":TELEGRAM,"subject":subject,"revision":"0","active":true}));
        }
        tx.commit().await?;
        Ok(
            json!({"contract":"core.account-links.v1","expected_auth_version":actor.user_version.to_string(),"links":links}),
        )
    }
    pub async fn begin_account_bind(
        &self,
        access: &str,
        provider: AccountLinkProvider,
        binding: &str,
        expected_auth_version: &str,
    ) -> BrowserResult<AccountBindStart> {
        self.require_account_links()?;
        let binding = self.account_bind_binding(provider, binding)?;
        let mut tx = begin(&self.store.pool).await?;
        let (actor, sid) = self.security_actor(&mut tx, access, true).await?;
        if actor.user_version != version(expected_auth_version, false)? {
            return Err(IdentityError::Conflict.into());
        }
        tx.commit().await?;
        // Existing provider starters acquire their own pooled connection. Do
        // not retain authority locks or a pool slot while they create state.
        let url = match provider {
            AccountLinkProvider::Google => self.store.begin_google_login(&binding).await?,
            AccountLinkProvider::External(p) => {
                self.store.begin_external_login(p, &binding).await?
            }
        };
        let url = reqwest::Url::parse(&url).map_err(|_| IdentityError::Storage)?;
        let state = url
            .query_pairs()
            .find(|(key, _)| key == "state")
            .map(|(_, value)| value.into_owned())
            .ok_or(IdentityError::Storage)?;
        let mut tx = begin(&self.store.pool).await?;
        let (current, current_sid) = self.security_actor(&mut tx, access, true).await?;
        if current.user_id != actor.user_id
            || current.user_version != actor.user_version
            || current_sid != sid
        {
            return Err(IdentityError::Unauthorized.into());
        }
        let expires_at:i64=sqlx::query_scalar("INSERT INTO core_account_links.bind_flows(state_digest,issuer,binding_digest,user_id,user_version,sid) VALUES($1,$2,$3,$4,$5,$6) RETURNING floor(extract(epoch FROM expires_at))::bigint")
            .bind(digest(&state)?).bind(provider.issuer()).bind(digest(&binding)?).bind(actor.user_id).bind(actor.user_version).bind(sid).fetch_one(&mut *tx).await?;
        sqlx::query("DELETE FROM core_account_links.bind_flows WHERE state_digest IN (SELECT state_digest FROM core_account_links.bind_flows WHERE expires_at<=clock_timestamp() ORDER BY expires_at LIMIT 100)").execute(&mut *tx).await?;
        tx.commit().await?;
        Ok(AccountBindStart {
            authorization_url: url.into(),
            state,
            expires_at,
        })
    }
    /// Redirect only an already recorded bind state from a fixed direct API
    /// callback to the existing popup page. The redirect grants no authority.
    pub async fn is_account_bind_state(
        &self,
        provider: AccountLinkProvider,
        state: &str,
    ) -> BrowserResult<bool> {
        if !self.store.account_links_enabled {
            return Ok(false);
        }
        let state = digest(state)?;
        Ok(sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM core_account_links.bind_flows WHERE state_digest=$1 AND issuer=$2 AND expires_at>clock_timestamp())")
            .bind(state).bind(provider.issuer()).fetch_one(&self.store.pool).await?)
    }
    pub async fn finish_account_bind(
        &self,
        access: &str,
        provider: AccountLinkProvider,
        binding: &str,
        input: &AccountBindFinish,
    ) -> BrowserResult<BrowserBundle> {
        self.require_account_links()?;
        if input
            .iss
            .as_deref()
            .is_some_and(|issuer| issuer != provider.issuer())
            || matches!(
                provider,
                AccountLinkProvider::External(ExternalProvider::Discord)
            ) && input.iss.as_deref() != Some(provider.issuer())
        {
            return Err(IdentityError::Unauthorized.into());
        }
        let binding = self.account_bind_binding(provider, binding)?;
        let mut tx = begin(&self.store.pool).await?;
        let (actor, sid) = self.security_actor(&mut tx, access, true).await?;
        let changed=sqlx::query("DELETE FROM core_account_links.bind_flows WHERE state_digest=$1 AND issuer=$2 AND binding_digest=$3 AND user_id=$4 AND user_version=$5 AND sid=$6 AND expires_at>clock_timestamp()")
            .bind(digest(&input.state)?).bind(provider.issuer()).bind(digest(&binding)?).bind(actor.user_id).bind(actor.user_version).bind(&sid).execute(&mut *tx).await?.rows_affected();
        if changed != 1 {
            return Err(IdentityError::Unauthorized.into());
        }
        tx.commit().await?;
        // No authority locks are held across the provider network exchange.
        let subject = match provider {
            AccountLinkProvider::Google => self
                .store
                .exchange_google_code(&input.state, &binding, &input.code)
                .await?
                .subject()
                .to_owned(),
            AccountLinkProvider::External(p) => self
                .store
                .exchange_external_code(p, &input.state, &binding, &input.code)
                .await?
                .subject()
                .to_owned(),
        };
        let mut tx = begin(&self.store.pool).await?;
        // Existing login/registration resolves the same immutable identity
        // under this gate. Keep lock order gate -> current browser authority.
        auth::lock_registration(&mut tx).await?;
        let (current, current_sid) = self.security_actor(&mut tx, access, true).await?;
        if current.user_id != actor.user_id
            || current.user_version != actor.user_version
            || current_sid != sid
        {
            return Err(IdentityError::Unauthorized.into());
        }
        let issuer = provider.issuer();
        let owner: Option<i64> = sqlx::query_scalar(
            "SELECT user_id FROM core_identity.oauth_identities WHERE issuer=$1 AND subject=$2",
        )
        .bind(issuer)
        .bind(&subject)
        .fetch_optional(&mut *tx)
        .await?;
        if owner.is_some_and(|owner| owner != current.user_id) {
            return Err(IdentityError::Conflict.into());
        }
        let active:bool=sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM core_identity.oauth_identities i LEFT JOIN core_account_links.oauth_states s USING(issuer,subject) WHERE i.user_id=$1 AND i.issuer=$2 AND s.revoked_at IS NULL)")
            .bind(current.user_id).bind(issuer).fetch_one(&mut *tx).await?;
        if active {
            return Err(IdentityError::Conflict.into());
        }
        if owner.is_none() {
            sqlx::query("INSERT INTO core_identity.oauth_identities(issuer,subject,user_id) VALUES($1,$2,$3)").bind(issuer).bind(&subject).bind(current.user_id).execute(&mut *tx).await?;
        }
        sqlx::query("INSERT INTO core_account_links.oauth_states(issuer,subject,user_id) VALUES($1,$2,$3) ON CONFLICT(issuer,subject) DO UPDATE SET revoked_at=NULL,revision=core_account_links.oauth_states.revision+1")
            .bind(issuer).bind(&subject).bind(current.user_id).execute(&mut *tx).await?;
        audit(
            &mut tx,
            &current,
            "identity.oauth_bind",
            json!({"user_id":current.user_id,"provider":provider.slug()}),
        )
        .await?;
        let bundle = self
            .rotate_security_authority(&mut tx, &current, &current_sid, "session.oauth_management")
            .await?;
        tx.commit().await?;
        Ok(bundle)
    }
    pub async fn unlink_account(
        &self,
        access: &str,
        input: &AccountUnlinkRequest,
        passkeys: Option<&PasskeyConfig>,
    ) -> BrowserResult<BrowserBundle> {
        self.require_account_links()?;
        let revision = version(&input.expected_revision, true)?;
        let expected = version(&input.expected_auth_version, false)?;
        if input.issuer.is_empty()
            || input.issuer.len() > 512
            || input.subject.is_empty()
            || input.subject.len() > 512
            || input
                .issuer
                .chars()
                .chain(input.subject.chars())
                .any(char::is_control)
        {
            return Err(IdentityError::Invalid.into());
        }
        let mut tx = begin(&self.store.pool).await?;
        auth::lock_registration(&mut tx).await?;
        let (actor, sid) = self.security_actor(&mut tx, access, true).await?;
        if actor.user_version != expected {
            return Err(IdentityError::Conflict.into());
        }
        if input.issuer == TELEGRAM {
            if revision != 0
                || !self
                    .retains_primary_login(
                        &mut tx,
                        actor.user_id,
                        PrimaryExclusion::Telegram(&input.subject),
                        passkeys.is_some(),
                    )
                    .await?
            {
                return Err(IdentityError::Forbidden.into());
            }
            let changed=sqlx::query("UPDATE core_telegram_auth.bindings SET revoked_at=clock_timestamp() WHERE subject=$1 AND user_id=$2 AND revoked_at IS NULL").bind(&input.subject).bind(actor.user_id).execute(&mut *tx).await?.rows_affected();
            if changed != 1 {
                return Err(IdentityError::Conflict.into());
            }
        } else {
            let stored:Option<i64>=sqlx::query_scalar("SELECT COALESCE(s.revision,0) FROM core_identity.oauth_identities i LEFT JOIN core_account_links.oauth_states s USING(issuer,subject) WHERE i.user_id=$1 AND i.issuer=$2 AND i.subject=$3 AND s.revoked_at IS NULL")
                .bind(actor.user_id).bind(&input.issuer).bind(&input.subject).fetch_optional(&mut *tx).await?;
            if stored != Some(revision) {
                return Err(IdentityError::Conflict.into());
            }
            if !self
                .retains_primary_login(
                    &mut tx,
                    actor.user_id,
                    PrimaryExclusion::OAuth(&input.issuer, &input.subject),
                    passkeys.is_some(),
                )
                .await?
            {
                return Err(IdentityError::Forbidden.into());
            }
            sqlx::query("INSERT INTO core_account_links.oauth_states(issuer,subject,user_id,revoked_at) VALUES($1,$2,$3,clock_timestamp()) ON CONFLICT(issuer,subject) DO UPDATE SET revoked_at=clock_timestamp(),revision=core_account_links.oauth_states.revision+1")
                .bind(&input.issuer).bind(&input.subject).bind(actor.user_id).execute(&mut *tx).await?;
        }
        audit(
            &mut tx,
            &actor,
            "identity.account_unlink",
            json!({"user_id":actor.user_id,"issuer":input.issuer}),
        )
        .await?;
        let bundle = self
            .rotate_security_authority(&mut tx, &actor, &sid, "session.oauth_management")
            .await?;
        tx.commit().await?;
        Ok(bundle)
    }
    pub(in crate::identity::browser) async fn retains_primary_login(
        &self,
        c: &mut PgConnection,
        user: i64,
        excluding: PrimaryExclusion<'_>,
        passkeys_enabled: bool,
    ) -> Result<bool> {
        let (passkey, issuer, subject, telegram) = match excluding {
            PrimaryExclusion::Passkey(id) => (id, "", "", ""),
            PrimaryExclusion::OAuth(issuer, subject) => (0, issuer, subject, ""),
            PrimaryExclusion::Telegram(subject) => (0, "", "", subject),
        };
        let mut issuers: Vec<&str> = self.store.external.keys().map(|p| p.issuer()).collect();
        if self.store.google.is_some() {
            issuers.push(GOOGLE);
        }
        let local:bool=sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM core_identity.password_logins l JOIN core_identity.user_profiles p ON p.user_id=l.user_id WHERE l.user_id=$1 AND p.password_login_allowed) OR ($2 AND EXISTS(SELECT 1 FROM core_passkeys.credentials WHERE user_id=$1 AND id<>$3 AND revoked_at IS NULL)) OR ($4 AND EXISTS(SELECT 1 FROM core_telegram_auth.bindings WHERE user_id=$1 AND subject<>$5 AND revoked_at IS NULL))")
            .bind(user).bind(passkeys_enabled).bind(passkey).bind(self.store.telegram.is_some()).bind(telegram).fetch_one(&mut *c).await?;
        if local {
            return Ok(true);
        }
        let sql = if self.store.account_links_enabled {
            "SELECT EXISTS(SELECT 1 FROM core_identity.oauth_identities i LEFT JOIN core_account_links.oauth_states s USING(issuer,subject) WHERE i.user_id=$1 AND i.issuer=ANY($2) AND NOT(i.issuer=$3 AND i.subject=$4) AND s.revoked_at IS NULL)"
        } else {
            "SELECT EXISTS(SELECT 1 FROM core_identity.oauth_identities i WHERE i.user_id=$1 AND i.issuer=ANY($2) AND NOT(i.issuer=$3 AND i.subject=$4))"
        };
        Ok(sqlx::query_scalar(sql)
            .bind(user)
            .bind(issuers)
            .bind(issuer)
            .bind(subject)
            .fetch_one(c)
            .await?)
    }
}

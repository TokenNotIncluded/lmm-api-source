//! Passkey ceremonies use the unmodified MPL-2.0 webauthn-rs 0.5.5 verifier.
//! <https://docs.rs/webauthn-rs/0.5.5/webauthn_rs/>
//! Only public browser options leave Core; challenge state remains in PostgreSQL.
use super::*;
use serde::{Deserialize, Serialize};
use sqlx::types::Json;
use std::collections::BTreeMap;
use webauthn_rs::prelude::{
    DiscoverableAuthentication, DiscoverableKey, Passkey, PasskeyAuthentication,
    PasskeyRegistration, PublicKeyCredential, RegisterPublicKeyCredential, Url, Uuid, Webauthn,
    WebauthnBuilder,
};

pub const PASSKEY_SCHEMA: &str = include_str!("../../../schema/passkeys.sql");

#[derive(Clone)]
pub struct PasskeyConfig {
    webauthn: Webauthn,
    origin: String,
}
impl PasskeyConfig {
    pub fn new(rp_id: &str, origin: &str) -> Result<Self> {
        let url = Url::parse(origin).map_err(|_| IdentityError::Invalid)?;
        if url.host_str() != Some(rp_id)
            || !url.username().is_empty()
            || url.password().is_some()
            || url.query().is_some()
            || url.fragment().is_some()
            || url.path() != "/"
            || (url.scheme() != "https" && !(url.scheme() == "http" && rp_id == "localhost"))
            || url.origin().ascii_serialization() != origin.trim_end_matches('/')
        {
            return Err(IdentityError::Invalid);
        }
        let webauthn = WebauthnBuilder::new(rp_id, &url)
            .map_err(|_| IdentityError::Invalid)?
            .allow_subdomains(false)
            .allow_any_port(false)
            .rp_name("LMM")
            .timeout(Duration::from_secs(300))
            .build()
            .map_err(|_| IdentityError::Invalid)?;
        Ok(Self {
            webauthn,
            origin: url.origin().ascii_serialization(),
        })
    }
    pub fn origin(&self) -> &str {
        &self.origin
    }
}

#[derive(Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
enum State {
    Register {
        challenge: PasskeyRegistration,
    },
    Named {
        challenge: PasskeyAuthentication,
        revisions: BTreeMap<String, i64>,
    },
    Discoverable {
        challenge: DiscoverableAuthentication,
    },
}
#[derive(Serialize)]
pub struct PasskeyChallenge {
    pub flow_token: String,
    pub options: Value,
    pub expires_at: i64,
}
fn label(input: &str) -> Result<&str> {
    let input = input.trim();
    if input.is_empty()
        || input.chars().count() > 64
        || input.len() > 256
        || input.chars().any(char::is_control)
    {
        Err(IdentityError::Invalid)
    } else {
        Ok(input)
    }
}
fn binding_digest(value: &str) -> Result<Vec<u8>> {
    if value.len() != 64 || !value.bytes().all(|byte| byte.is_ascii_hexdigit()) {
        return Err(IdentityError::Unauthorized);
    }
    digest(value)
}
async fn bounded_begin(pool: &PgPool) -> Result<sqlx::Transaction<'_, sqlx::Postgres>> {
    let mut tx = pool.begin().await?;
    for statement in [
        "SET LOCAL statement_timeout='10s'",
        "SET LOCAL lock_timeout='500ms'",
    ] {
        sqlx::query(statement).execute(&mut *tx).await?;
    }
    Ok(tx)
}

impl BrowserAuth {
    pub(in crate::identity::browser) async fn security_actor(
        &self,
        c: &mut PgConnection,
        access: &str,
        recent: bool,
    ) -> BrowserResult<(Principal, String)> {
        let actor = session(c, access).await?;
        // NOWAIT avoids shared-lock upgrades racing refresh or a security reset.
        let row=sqlx::query("SELECT s.sid,sa.authenticated_at>clock_timestamp()-interval '10 minutes' AS recent FROM core_identity.web_sessions s JOIN core_identity.session_authentication sa ON sa.credential_id=s.current_credential_id JOIN core_identity.users u ON u.id=s.user_id JOIN core_identity.user_profiles p ON p.user_id=u.id WHERE s.current_credential_id=$1 AND s.user_id=$2 AND s.user_version=$3 AND s.revoked_at IS NULL AND s.expires_at>clock_timestamp() AND (NOT s.weekly_logout OR s.created_at>clock_timestamp()-interval '7 days') FOR UPDATE OF s,u,p NOWAIT")
            .bind(actor.credential_id).bind(actor.user_id).bind(actor.user_version).fetch_optional(c).await?
            .ok_or(IdentityError::Unauthorized)?;
        if recent && !row.try_get::<bool, _>("recent")? {
            return Err(IdentityError::Forbidden.into());
        }
        Ok((actor, row.try_get("sid")?))
    }

    pub async fn passkey_status(&self, access: &str) -> BrowserResult<Value> {
        let mut tx = bounded_begin(&self.store.pool).await?;
        let (actor, _) = self.security_actor(&mut tx, access, false).await?;
        let rows=sqlx::query("SELECT id,name,to_char(created_at AT TIME ZONE 'UTC','YYYY-MM-DD\"T\"HH24:MI:SS\"Z\"') AS created_at,to_char(last_used_at AT TIME ZONE 'UTC','YYYY-MM-DD\"T\"HH24:MI:SS\"Z\"') AS last_used_at FROM core_passkeys.credentials WHERE user_id=$1 AND revoked_at IS NULL ORDER BY id LIMIT 20")
            .bind(actor.user_id).fetch_all(&mut *tx).await?;
        let mut credentials = Vec::new();
        let mut last_used_at: Option<String> = None;
        for row in rows {
            let used = row.try_get::<Option<String>, _>("last_used_at")?;
            last_used_at = last_used_at.max(used.clone());
            credentials.push(json!({"id":row.try_get::<i64,_>("id")?.to_string(),"name":row.try_get::<String,_>("name")?,"created_at":row.try_get::<String,_>("created_at")?,"last_used_at":used}));
        }
        tx.commit().await?;
        Ok(
            json!({"enabled":!credentials.is_empty(),"credentials":credentials,"management":"core_recent_session","last_used_at":last_used_at}),
        )
    }

    pub async fn begin_passkey_registration(
        &self,
        config: &PasskeyConfig,
        access: &str,
        binding: &str,
    ) -> BrowserResult<PasskeyChallenge> {
        let binding = binding_digest(binding)?;
        let mut tx = bounded_begin(&self.store.pool).await?;
        let (actor, sid) = self.security_actor(&mut tx, access, true).await?;
        let active:i64=sqlx::query_scalar("SELECT count(*) FROM core_passkeys.credentials WHERE user_id=$1 AND revoked_at IS NULL")
            .bind(actor.user_id).fetch_one(&mut *tx).await?;
        if active >= 10 {
            return Err(IdentityError::Conflict.into());
        }
        sqlx::query("INSERT INTO core_passkeys.user_handles(user_id) VALUES($1) ON CONFLICT(user_id) DO NOTHING")
            .bind(actor.user_id).execute(&mut *tx).await?;
        let row=sqlx::query("SELECT h.user_handle::text,p.username,p.display_name FROM core_passkeys.user_handles h JOIN core_identity.user_profiles p ON p.user_id=h.user_id WHERE h.user_id=$1")
            .bind(actor.user_id).fetch_one(&mut *tx).await?;
        let handle = Uuid::parse_str(&row.try_get::<String, _>("user_handle")?)
            .map_err(|_| IdentityError::Storage)?;
        let keys:Vec<Json<Passkey>>=sqlx::query_scalar("SELECT passkey FROM core_passkeys.credentials WHERE user_id=$1 ORDER BY id DESC LIMIT 100")
            .bind(actor.user_id).fetch_all(&mut *tx).await?;
        let excluded = keys
            .into_iter()
            .map(|key| key.0.cred_id().clone())
            .collect();
        let (options, challenge) = config
            .webauthn
            .start_passkey_registration(
                handle,
                &row.try_get::<String, _>("username")?,
                &row.try_get::<String, _>("display_name")?,
                Some(excluded),
            )
            .map_err(|_| IdentityError::Invalid)?;
        let flow_token = new_secret("lmmp_")?;
        let expires_at:i64=sqlx::query_scalar("INSERT INTO core_passkeys.flows(digest,binding_digest,purpose,user_id,user_version,sid,state) VALUES($1,$2,'register',$3,$4,$5,$6) RETURNING floor(extract(epoch FROM expires_at))::bigint")
            .bind(digest(&flow_token)?).bind(binding).bind(actor.user_id).bind(actor.user_version).bind(sid)
            .bind(Json(State::Register{challenge})).fetch_one(&mut *tx).await?;
        cleanup(&mut tx).await?;
        tx.commit().await?;
        Ok(PasskeyChallenge {
            flow_token,
            options: serde_json::to_value(options).map_err(|_| IdentityError::Storage)?,
            expires_at,
        })
    }

    pub async fn finish_passkey_registration(
        &self,
        config: &PasskeyConfig,
        access: &str,
        binding: &str,
        flow: &str,
        credential: Value,
        name: &str,
    ) -> BrowserResult<BrowserBundle> {
        let name = if name.trim().is_empty() {
            "Passkey"
        } else {
            label(name)?
        };
        let reg: RegisterPublicKeyCredential =
            serde_json::from_value(credential).map_err(|_| IdentityError::Invalid)?;
        let mut tx = bounded_begin(&self.store.pool).await?;
        let (actor, sid) = self.security_actor(&mut tx, access, true).await?;
        let Json(state):Json<State>=sqlx::query_scalar("DELETE FROM core_passkeys.flows WHERE digest=$1 AND binding_digest=$2 AND purpose='register' AND user_id=$3 AND user_version=$4 AND sid=$5 AND expires_at>clock_timestamp() RETURNING state")
            .bind(digest(flow)?).bind(binding_digest(binding)?).bind(actor.user_id).bind(actor.user_version).bind(&sid)
            .fetch_optional(&mut *tx).await?.ok_or(IdentityError::Unauthorized)?;
        let State::Register { challenge } = state else {
            return Err(IdentityError::Unauthorized.into());
        };
        let passkey = match config
            .webauthn
            .finish_passkey_registration(&reg, &challenge)
        {
            Ok(passkey) => passkey,
            Err(_) => {
                tx.commit().await?;
                return Err(IdentityError::Unauthorized.into());
            }
        };
        let active:i64=sqlx::query_scalar("SELECT count(*) FROM core_passkeys.credentials WHERE user_id=$1 AND revoked_at IS NULL")
            .bind(actor.user_id).fetch_one(&mut *tx).await?;
        if active >= 10 {
            tx.commit().await?;
            return Err(IdentityError::Conflict.into());
        }
        let credential_id: &[u8] = passkey.cred_id().as_ref();
        // Never resurrect a revoked CredentialID or transfer one between accounts.
        let id:Option<i64>=sqlx::query_scalar("INSERT INTO core_passkeys.credentials(user_id,credential_id,passkey,name) VALUES($1,$2,$3,$4) ON CONFLICT(credential_id) DO NOTHING RETURNING id")
            .bind(actor.user_id).bind(credential_id).bind(Json(&passkey)).bind(name).fetch_optional(&mut *tx).await?;
        let Some(id) = id else {
            tx.commit().await?;
            return Err(IdentityError::Conflict.into());
        };
        audit(
            &mut tx,
            &actor,
            "passkey.register",
            json!({"passkey_id":id}),
        )
        .await?;
        let bundle = self
            .rotate_security_authority(&mut tx, &actor, &sid, "session.passkey_management")
            .await?;
        tx.commit().await?;
        Ok(bundle)
    }

    pub async fn remove_passkey(&self, access: &str, id: i64) -> BrowserResult<BrowserBundle> {
        if id <= 0 {
            return Err(IdentityError::Invalid.into());
        }
        let mut tx = bounded_begin(&self.store.pool).await?;
        let (actor, sid) = self.security_actor(&mut tx, access, true).await?;
        // Retain at least one usable primary login method. A profile email alone
        // is not evidence that email login is enabled or a password exists.
        let alternative = self
            .retains_primary_login(
                &mut tx,
                actor.user_id,
                super::account_links::PrimaryExclusion::Passkey(id),
                true,
            )
            .await?;
        if !alternative {
            return Err(IdentityError::Forbidden.into());
        }
        let changed=sqlx::query("UPDATE core_passkeys.credentials SET revoked_at=clock_timestamp(),revision=revision+1 WHERE id=$1 AND user_id=$2 AND revoked_at IS NULL")
            .bind(id).bind(actor.user_id).execute(&mut *tx).await?.rows_affected();
        if changed != 1 {
            return Err(IdentityError::Conflict.into());
        }
        audit(&mut tx, &actor, "passkey.revoke", json!({"passkey_id":id})).await?;
        let bundle = self
            .rotate_security_authority(&mut tx, &actor, &sid, "session.passkey_management")
            .await?;
        tx.commit().await?;
        Ok(bundle)
    }

    pub(in crate::identity::browser) async fn rotate_security_authority(
        &self,
        c: &mut PgConnection,
        actor: &Principal,
        sid: &str,
        action: &str,
    ) -> BrowserResult<BrowserBundle> {
        let assurance = super::super::mfa::from_session(
            c,
            actor.credential_id,
            actor.user_id,
            actor.user_version,
        )
        .await?;
        let version:i64=sqlx::query_scalar("UPDATE core_identity.users SET auth_version=auth_version+1 WHERE id=$1 AND auth_version<9223372036854775807 RETURNING auth_version")
            .bind(actor.user_id).fetch_optional(&mut *c).await?.ok_or(IdentityError::Conflict)?;
        sqlx::query("UPDATE core_identity.credentials SET revoked_at=clock_timestamp() WHERE user_id=$1 AND revoked_at IS NULL")
            .bind(actor.user_id).execute(&mut *c).await?;
        sqlx::query("UPDATE core_identity.web_sessions SET revoked_at=clock_timestamp() WHERE user_id=$1 AND sid<>$2 AND revoked_at IS NULL")
            .bind(actor.user_id).bind(sid).execute(&mut *c).await?;
        sqlx::query("DELETE FROM core_passkeys.flows WHERE user_id=$1")
            .bind(actor.user_id)
            .execute(&mut *c)
            .await?;
        // Reuse the established security-rotation writer: retain this family's
        // absolute/weekly deadlines and assurance, and clear refresh grace.
        self.rotate_mfa_family(c, actor, sid, &assurance.rebind_version(version), action)
            .await
            .map_err(|error| match error {
                super::super::mfa::MfaError::Identity(error) => error.into(),
                _ => IdentityError::Unauthorized.into(),
            })
    }

    pub async fn begin_passkey_login(
        &self,
        config: &PasskeyConfig,
        binding: &str,
        username: Option<&str>,
    ) -> BrowserResult<PasskeyChallenge> {
        let binding = binding_digest(binding)?;
        let mut tx = bounded_begin(&self.store.pool).await?;
        let (options, state, user, version) = if let Some(username) =
            username.filter(|name| !name.trim().is_empty())
        {
            if username.len() > 320 {
                return Err(IdentityError::Invalid.into());
            }
            let users=sqlx::query("SELECT u.id,u.auth_version FROM core_identity.users u JOIN core_identity.user_profiles p ON p.user_id=u.id JOIN core_identity.accounts a ON a.id=u.personal_account_id WHERE (p.username=$1 OR (p.email_login_allowed AND p.email<>'' AND p.email=$1)) AND a.active LIMIT 2")
                .bind(username.trim()).fetch_all(&mut *tx).await?;
            if users.len() != 1 {
                return Err(IdentityError::Unauthorized.into());
            }
            let user: i64 = users[0].try_get("id")?;
            let version: i64 = users[0].try_get("auth_version")?;
            let rows=sqlx::query("SELECT id,revision,passkey FROM core_passkeys.credentials WHERE user_id=$1 AND revoked_at IS NULL ORDER BY id LIMIT 10")
                .bind(user).fetch_all(&mut *tx).await?;
            let mut keys = Vec::new();
            let mut revisions = BTreeMap::new();
            for row in rows {
                let Json(key): Json<Passkey> = row.try_get("passkey")?;
                keys.push(key);
                revisions.insert(
                    row.try_get::<i64, _>("id")?.to_string(),
                    row.try_get::<i64, _>("revision")?,
                );
            }
            if keys.is_empty() {
                return Err(IdentityError::Unauthorized.into());
            }
            let (options, challenge) = config
                .webauthn
                .start_passkey_authentication(&keys)
                .map_err(|_| IdentityError::Unauthorized)?;
            (
                options,
                State::Named {
                    challenge,
                    revisions,
                },
                Some(user),
                Some(version),
            )
        } else {
            let (options, challenge) = config
                .webauthn
                .start_discoverable_authentication()
                .map_err(|_| IdentityError::Invalid)?;
            (options, State::Discoverable { challenge }, None, None)
        };
        let flow_token = new_secret("lmmp_")?;
        let expires_at:i64=sqlx::query_scalar("INSERT INTO core_passkeys.flows(digest,binding_digest,purpose,user_id,user_version,state) VALUES($1,$2,'login',$3,$4,$5) RETURNING floor(extract(epoch FROM expires_at))::bigint")
            .bind(digest(&flow_token)?).bind(binding).bind(user).bind(version).bind(Json(state)).fetch_one(&mut *tx).await?;
        cleanup(&mut tx).await?;
        tx.commit().await?;
        Ok(PasskeyChallenge {
            flow_token,
            options: serde_json::to_value(options).map_err(|_| IdentityError::Storage)?,
            expires_at,
        })
    }

    pub async fn finish_passkey_login(
        &self,
        config: &PasskeyConfig,
        binding: &str,
        flow: &str,
        credential: Value,
        metadata: &BrowserSessionMetadata,
        mfa_binding: &str,
    ) -> super::super::mfa::MfaResult<BrowserLoginOutcome> {
        let reg: PublicKeyCredential =
            serde_json::from_value(credential).map_err(|_| IdentityError::Invalid)?;
        let mut tx = bounded_begin(&self.store.pool).await?;
        let row=sqlx::query("DELETE FROM core_passkeys.flows WHERE digest=$1 AND binding_digest=$2 AND purpose='login' AND expires_at>clock_timestamp() RETURNING user_id,user_version,state")
            .bind(digest(flow)?).bind(binding_digest(binding)?).fetch_optional(&mut *tx).await?.ok_or(IdentityError::Unauthorized)?;
        let Json(state): Json<State> = row.try_get("state")?;
        let credential_id: &[u8] = reg.raw_id.as_ref();
        let credential=sqlx::query("SELECT k.id,k.user_id,k.passkey,k.revision,h.user_handle::text,u.auth_version FROM core_passkeys.credentials k JOIN core_passkeys.user_handles h ON h.user_id=k.user_id JOIN core_identity.users u ON u.id=k.user_id JOIN core_identity.accounts a ON a.id=u.personal_account_id WHERE k.credential_id=$1 AND k.revoked_at IS NULL AND a.active FOR UPDATE OF k NOWAIT FOR SHARE OF u,a NOWAIT")
            .bind(credential_id).fetch_optional(&mut *tx).await?;
        let Some(credential) = credential else {
            tx.commit().await?;
            return Err(IdentityError::Unauthorized.into());
        };
        let id: i64 = credential.try_get("id")?;
        let user: i64 = credential.try_get("user_id")?;
        let Json(mut key): Json<Passkey> = credential.try_get("passkey")?;
        let result = match state {
            State::Named {
                challenge,
                revisions,
            } => {
                if row.try_get::<Option<i64>, _>("user_id")? != Some(user)
                    || row.try_get::<Option<i64>, _>("user_version")?
                        != Some(credential.try_get("auth_version")?)
                    || revisions.get(&id.to_string())
                        != Some(&credential.try_get::<i64, _>("revision")?)
                {
                    tx.commit().await?;
                    return Err(IdentityError::Unauthorized.into());
                }
                config
                    .webauthn
                    .finish_passkey_authentication(&reg, &challenge)
            }
            State::Discoverable { challenge } => {
                let Ok((handle, reported_id)) =
                    config.webauthn.identify_discoverable_authentication(&reg)
                else {
                    tx.commit().await?;
                    return Err(IdentityError::Unauthorized.into());
                };
                if handle.to_string() != credential.try_get::<String, _>("user_handle")?
                    || reported_id != credential_id
                {
                    tx.commit().await?;
                    return Err(IdentityError::Unauthorized.into());
                }
                config.webauthn.finish_discoverable_authentication(
                    &reg,
                    challenge,
                    &[DiscoverableKey::from(&key)],
                )
            }
            State::Register { .. } => {
                tx.commit().await?;
                return Err(IdentityError::Unauthorized.into());
            }
        };
        let result = match result {
            Ok(result) => result,
            Err(_) => {
                tx.commit().await?;
                return Err(IdentityError::Unauthorized.into());
            }
        };
        if key.update_credential(&result).is_none() {
            tx.commit().await?;
            return Err(IdentityError::Unauthorized.into());
        }
        sqlx::query("UPDATE core_passkeys.credentials SET passkey=$2,last_used_at=clock_timestamp(),revision=revision+1 WHERE id=$1")
            .bind(id).bind(Json(key)).execute(&mut *tx).await?;
        let outcome = self
            .provider_login_outcome(
                &mut tx,
                user,
                metadata,
                mfa_binding,
                "session.passkey_browser_login",
            )
            .await?;
        tx.commit().await?;
        Ok(outcome)
    }
}

async fn cleanup(c: &mut PgConnection) -> Result<()> {
    sqlx::query("DELETE FROM core_passkeys.flows WHERE digest IN (SELECT digest FROM core_passkeys.flows WHERE expires_at<=clock_timestamp() ORDER BY expires_at LIMIT 100)")
        .execute(c).await?;
    Ok(())
}

//! Independent installed-application grants and durable refresh recovery.
//! Secrets deliberately have neither Debug nor ordinary receipt serialization.
use super::issuer::{scope_groups, scope_subset_is_current};
use super::{protocol, *};
use aes_gcm::{
    Aes256Gcm, KeyInit, Nonce,
    aead::{Aead, Payload},
};
use rand::{TryRng as _, rngs::SysRng};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use zeroize::{Zeroize, Zeroizing};

const APPLICATION: &str = "cc-switch";
const MAX_CIPHERTEXT: usize = 16_384;

fn is_lower_hex64(value: &str) -> bool {
    value.len() == 64
        && value
            .bytes()
            .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
}
fn key_id(value: &str) -> bool {
    (1..=64).contains(&value.len())
        && value
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b"-_".contains(&b))
}

/// Validated installation identity, constructed only by managed form validation.
#[derive(Clone)]
pub struct InstallationId(String);
impl InstallationId {
    fn parse(value: String) -> Result<Self> {
        if !is_lower_hex64(&value) {
            return Err(IdentityError::Invalid);
        }
        Ok(Self(value))
    }
    fn as_str(&self) -> &str {
        &self.0
    }
}

/// A rotation operation is distinct from its installation, despite equal wire syntax.
#[derive(Clone)]
pub struct RotationId(String);
impl RotationId {
    fn parse(value: String) -> Result<Self> {
        if !is_lower_hex64(&value) {
            return Err(IdentityError::Invalid);
        }
        Ok(Self(value))
    }
    fn as_str(&self) -> &str {
        &self.0
    }
}

/// Separate strict wire type: old clients cannot silently supply app fields.
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ManagedAuthorizationRequest {
    pub response_type: String,
    pub client_id: String,
    pub redirect_uri: String,
    pub scope: String,
    pub resource: String,
    pub code_challenge: String,
    pub code_challenge_method: String,
    pub state: String,
    pub installation_id: String,
    pub application: String,
    pub display_label: String,
}
impl ManagedAuthorizationRequest {
    fn validate(self, profile: &LmmProfile) -> Result<(AuthorizationRequest, Installation)> {
        if self.application != APPLICATION
            || self.display_label.trim().is_empty()
            || self.display_label.len() > 160
            || self.display_label.chars().any(char::is_control)
        {
            return Err(IdentityError::Invalid);
        }
        let installation_id = InstallationId::parse(self.installation_id)?;
        let request = profile.validate_managed_authorization(AuthorizationRequest {
            response_type: self.response_type,
            client_id: self.client_id,
            redirect_uri: self.redirect_uri,
            scope: self.scope,
            resource: self.resource,
            code_challenge: self.code_challenge,
            code_challenge_method: self.code_challenge_method,
            state: self.state,
        })?;
        Ok((
            request,
            Installation {
                id: installation_id,
                label: self.display_label,
            },
        ))
    }
}
struct Installation {
    id: InstallationId,
    label: String,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ManagedTokenForm {
    pub grant_type: String,
    pub client_id: String,
    pub resource: String,
    pub installation_id: String,
    pub code: Option<String>,
    pub redirect_uri: Option<String>,
    pub code_verifier: Option<String>,
    pub refresh_token: Option<String>,
    pub scope: Option<String>,
    pub rotation_id: Option<String>,
}
pub enum ManagedTokenRequest {
    Code {
        installation_id: InstallationId,
        code: String,
        redirect: String,
        verifier: String,
    },
    Refresh {
        installation_id: InstallationId,
        refresh: String,
        rotation_id: RotationId,
    },
}
impl ManagedTokenForm {
    pub fn validate(self, profile: &LmmProfile) -> Result<ManagedTokenRequest> {
        if self.client_id != protocol::MANAGED_CLIENT {
            return Err(IdentityError::Invalid);
        }
        let installation_id = InstallationId::parse(self.installation_id)?;
        profile.binding(&self.client_id, &self.resource)?;
        match self.grant_type.as_str() {
            "authorization_code"
                if self.refresh_token.is_none()
                    && self.scope.is_none()
                    && self.rotation_id.is_none() =>
            {
                let code = self.code.ok_or(IdentityError::Invalid)?;
                let redirect = self.redirect_uri.ok_or(IdentityError::Invalid)?;
                let verifier = self.code_verifier.ok_or(IdentityError::Invalid)?;
                protocol::canonical(&code, "lmm_oc_")?;
                super::redirect(&redirect)?;
                if !(43..=128).contains(&verifier.len())
                    || !verifier
                        .bytes()
                        .all(|b| b.is_ascii_alphanumeric() || b"-._~".contains(&b))
                {
                    return Err(IdentityError::Invalid);
                }
                Ok(ManagedTokenRequest::Code {
                    installation_id,
                    code,
                    redirect,
                    verifier,
                })
            }
            "refresh_token"
                if self.code.is_none()
                    && self.redirect_uri.is_none()
                    && self.code_verifier.is_none()
                    && self.scope.is_none() =>
            {
                let refresh = self.refresh_token.ok_or(IdentityError::Invalid)?;
                let rotation_id =
                    RotationId::parse(self.rotation_id.ok_or(IdentityError::Invalid)?)?;
                protocol::canonical(&refresh, "lmm_rt_")?;
                Ok(ManagedTokenRequest::Refresh {
                    installation_id,
                    refresh,
                    rotation_id,
                })
            }
            _ => Err(IdentityError::Invalid),
        }
    }
}
#[derive(Serialize)]
pub struct ManagedTokenWire {
    pub access_token: String,
    pub refresh_token: String,
    pub token_type: &'static str,
    pub expires_in: i64,
    pub scope: String,
    pub client_id: &'static str,
    pub installation_id: String,
    pub grant_id: String,
    pub budget_credits: i64,
}

/// Private, independently generated recovery keys; never Debug or serializable.
pub struct ManagedRecoveryKeys {
    active: String,
    keys: BTreeMap<String, Zeroizing<[u8; 32]>>,
}
impl ManagedRecoveryKeys {
    pub fn new(active: &str, keys: Vec<(String, Vec<u8>)>) -> Result<Self> {
        let keys: Vec<_> = keys
            .into_iter()
            .map(|(id, key)| (id, Zeroizing::new(key)))
            .collect();
        if !key_id(active) || !(1..=8).contains(&keys.len()) {
            return Err(IdentityError::Invalid);
        }
        let mut stored: BTreeMap<String, Zeroizing<[u8; 32]>> = BTreeMap::new();
        for (id, key) in keys {
            if !key_id(&id)
                || key.len() != 32
                || key.iter().all(|b| *b == 0)
                || stored.contains_key(&id)
                || stored
                    .values()
                    .any(|value| value.as_slice() == key.as_slice())
            {
                return Err(IdentityError::Invalid);
            }
            let mut material = Zeroizing::new([0u8; 32]);
            material.copy_from_slice(&key);
            stored.insert(id, material);
        }
        if !stored.contains_key(active) {
            return Err(IdentityError::Invalid);
        }
        Ok(Self {
            active: active.into(),
            keys: stored,
        })
    }
    fn seal(&self, aad: &[u8], plaintext: &[u8]) -> Result<(String, Vec<u8>)> {
        if plaintext.len() + 29 > MAX_CIPHERTEXT {
            return Err(IdentityError::Storage);
        }
        let key = self.keys.get(&self.active).ok_or(IdentityError::Storage)?;
        let cipher =
            Aes256Gcm::new_from_slice(key.as_slice()).map_err(|_| IdentityError::Storage)?;
        let mut nonce = [0u8; 12];
        SysRng
            .try_fill_bytes(&mut nonce)
            .map_err(|_| IdentityError::Storage)?;
        let encrypted = cipher
            .encrypt(
                &Nonce::from(nonce),
                Payload {
                    msg: plaintext,
                    aad,
                },
            )
            .map_err(|_| IdentityError::Storage)?;
        let mut result = Vec::with_capacity(encrypted.len() + 13);
        result.push(1);
        result.extend_from_slice(&nonce);
        result.extend_from_slice(&encrypted);
        Ok((self.active.clone(), result))
    }
    fn open(&self, id: &str, aad: &[u8], ciphertext: &[u8]) -> Result<Zeroizing<Vec<u8>>> {
        if !(29..=MAX_CIPHERTEXT).contains(&ciphertext.len()) || ciphertext[0] != 1 {
            return Err(IdentityError::Storage);
        }
        let key = self.keys.get(id).ok_or(IdentityError::Storage)?;
        let cipher =
            Aes256Gcm::new_from_slice(key.as_slice()).map_err(|_| IdentityError::Storage)?;
        let nonce: [u8; 12] = ciphertext[1..13]
            .try_into()
            .map_err(|_| IdentityError::Storage)?;
        let raw = cipher
            .decrypt(
                &Nonce::from(nonce),
                Payload {
                    msg: &ciphertext[13..],
                    aad,
                },
            )
            .map_err(|_| IdentityError::Storage)?;
        Ok(Zeroizing::new(raw))
    }
}

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Successor {
    access_token: String,
    refresh_token: String,
    access_expires_at: i64,
}
impl Drop for Successor {
    fn drop(&mut self) {
        self.access_token.zeroize();
        self.refresh_token.zeroize();
    }
}

impl LmmIssuer {
    /// Caller holds the existing family lock; immutable sidecars need no UPDATE right.
    pub(super) async fn validate_managed_family_binding(
        &self,
        c: &mut PgConnection,
        f: &PgRow,
    ) -> Result<()> {
        if f.try_get::<String, _>("client_id")? != protocol::MANAGED_CLIENT
            || f.try_get::<String, _>("issuer")? != self.profile.issuer()
            || f.try_get::<String, _>("resource")? != self.profile.resource()
            || f.try_get::<String, _>("issuance_kind")? != "browser"
            || f.try_get::<String, _>("purpose")? != "normal_model"
        {
            return Err(IdentityError::Unauthorized);
        }
        let valid: bool = sqlx::query_scalar(
            "SELECT b.client_id='lmm-cc-switch' AND b.application='cc-switch'
                AND b.installation_id ~ '^[0-9a-f]{64}$'
                AND b.client_id=a.client_id AND b.application=a.application
                AND b.installation_id=a.installation_id AND b.display_label=a.display_label
                AND r.client_id=b.client_id AND r.user_id=$2 AND r.owner_account_id=$3
                AND r.user_version=$4 AND r.issuer=$5 AND r.resource=$6
                AND r.consumed_at IS NOT NULL AND $7::text[] <@ r.granted_scopes
             FROM core_oauth.managed_family_bindings b
             JOIN core_oauth.managed_authorization_bindings a ON a.binding_digest=b.authorization_binding_digest
             JOIN core_oauth.authorization_requests r ON r.binding_digest=a.binding_digest
             WHERE b.family_id=$1",
        )
        .bind(f.try_get::<String, _>("id")?)
        .bind(f.try_get::<i64, _>("user_id")?)
        .bind(f.try_get::<i64, _>("owner_account_id")?)
        .bind(f.try_get::<i64, _>("user_version")?)
        .bind(self.profile.issuer())
        .bind(self.profile.resource())
        .bind(f.try_get::<Vec<String>, _>("scopes")?)
        .fetch_optional(c)
        .await?
        .unwrap_or(false);
        if !valid {
            return Err(IdentityError::Unauthorized);
        }
        Ok(())
    }
}

impl LmmIssuer {
    pub async fn with_managed_recovery_keys(mut self, keys: ManagedRecoveryKeys) -> Result<Self> {
        crate::core_schema::managed_applications_ready(&self.identity.pool).await?;
        self.managed_keys = Some(std::sync::Arc::new(keys));
        Ok(self)
    }
    fn has_managed_issuance_configuration(&self) -> bool {
        self.profile
            .model_budget_limit(protocol::MANAGED_CLIENT)
            .is_some()
            && self.managed_keys.is_some()
            && self.catalog_surface.is_some()
    }
    pub(crate) async fn managed_discovery_ready(&self) -> Result<bool> {
        if !self.has_managed_issuance_configuration() {
            return Ok(false);
        }
        // Check the actual installed contract; a configuration flag alone is
        // insufficient, and discovery must never install or repair a schema.
        crate::core_schema::managed_applications_ready(&self.identity.pool).await?;
        crate::runtime_database::check(
            &self.identity.pool,
            crate::runtime_database::RequiredCapabilities {
                models: true,
                oauth2: true,
                managed_applications: true,
                web_identity: true,
                ..Default::default()
            },
        )
        .await
        .map_err(|_| IdentityError::Storage)?;
        Ok(true)
    }
    pub(super) fn require_managed_issuance(&self) -> Result<()> {
        if !self.has_managed_issuance_configuration() {
            return Err(IdentityError::Forbidden);
        }
        Ok(())
    }
    pub async fn begin_managed(&self, input: ManagedAuthorizationRequest) -> Result<ConsentPage> {
        self.require_managed_issuance()?;
        let (request, installation) = input.validate(&self.profile)?;
        let mut tx = transaction(&self.identity.pool).await?;
        let mut page = self.pending(&mut tx, &request, None, &[], None).await?;
        sqlx::query("INSERT INTO core_oauth.managed_authorization_bindings(binding_digest,client_id,application,installation_id,display_label) VALUES($1,'lmm-cc-switch','cc-switch',$2,$3)")
            .bind(digest(&page.binding)?).bind(installation.id.as_str()).bind(&installation.label).execute(&mut *tx).await?;
        page.installation_id = Some(installation.id.0);
        page.display_label = Some(installation.label);
        tx.commit().await?;
        Ok(page)
    }
    pub(super) async fn copy_managed_authorization(
        &self,
        c: &mut PgConnection,
        previous: &str,
        page: &mut ConsentPage,
    ) -> Result<()> {
        let row = sqlx::query("INSERT INTO core_oauth.managed_authorization_bindings(binding_digest,client_id,application,installation_id,display_label) SELECT $1,client_id,application,installation_id,display_label FROM core_oauth.managed_authorization_bindings WHERE binding_digest=$2 RETURNING installation_id,display_label")
            .bind(digest(&page.binding)?).bind(digest(previous)?).fetch_optional(c).await?.ok_or(IdentityError::Unauthorized)?;
        page.installation_id = Some(row.try_get("installation_id")?);
        page.display_label = Some(row.try_get("display_label")?);
        Ok(())
    }
    pub(super) async fn bind_managed_family(
        &self,
        c: &mut PgConnection,
        family_id: &str,
        binding: &str,
    ) -> Result<()> {
        let count = sqlx::query("INSERT INTO core_oauth.managed_family_bindings(family_id,authorization_binding_digest,client_id,application,installation_id,display_label) SELECT $1,binding_digest,client_id,application,installation_id,display_label FROM core_oauth.managed_authorization_bindings WHERE binding_digest=$2")
            .bind(family_id).bind(digest(binding)?).execute(c).await?.rows_affected();
        if count != 1 {
            return Err(IdentityError::Unauthorized);
        }
        Ok(())
    }
}

impl LmmIssuer {
    async fn managed_installation(
        &self,
        c: &mut PgConnection,
        f: &PgRow,
        expected: &InstallationId,
    ) -> Result<()> {
        self.validate_managed_family_binding(c, f).await?;
        let matches: bool = sqlx::query_scalar(
            "SELECT installation_id=$2 FROM core_oauth.managed_family_bindings WHERE family_id=$1",
        )
        .bind(f.try_get::<String, _>("id")?)
        .bind(expected.as_str())
        .fetch_optional(c)
        .await?
        .unwrap_or(false);
        if !matches {
            return Err(IdentityError::Unauthorized);
        }
        Ok(())
    }
    async fn managed_successor(
        &self,
        c: &mut PgConnection,
        f: &PgRow,
        scopes: &[String],
    ) -> Result<(Successor, String)> {
        self.profile
            .validate_model_budget(protocol::MANAGED_CLIENT, f.try_get("budget_credits")?)?;
        let wire = self.issue(c, f, scopes).await?;
        let row = sqlx::query("SELECT id,floor(extract(epoch FROM expires_at))::bigint AS expiry FROM core_oauth.access_tokens WHERE digest=$1 AND family_id=$2")
            .bind(digest(&wire.access_token)?).bind(f.try_get::<String, _>("id")?).fetch_one(c).await?;
        Ok((
            Successor {
                access_token: wire.access_token,
                refresh_token: wire.refresh_token,
                access_expires_at: row.try_get("expiry")?,
            },
            row.try_get("id")?,
        ))
    }
    async fn managed_wire(
        &self,
        c: &mut PgConnection,
        f: &PgRow,
        installation_id: &InstallationId,
        successor: &Successor,
        access_id: &str,
    ) -> Result<ManagedTokenWire> {
        let row = sqlx::query("SELECT scopes,floor(extract(epoch FROM expires_at-clock_timestamp()))::bigint AS remaining,floor(extract(epoch FROM expires_at))::bigint AS expiry FROM core_oauth.access_tokens WHERE id=$1 AND family_id=$2 AND digest=$3")
            .bind(access_id).bind(f.try_get::<String, _>("id")?).bind(digest(&successor.access_token)?).fetch_one(c).await?;
        let remaining: i64 = row.try_get("remaining")?;
        if !(1..=86400).contains(&remaining)
            || row.try_get::<i64, _>("expiry")? != successor.access_expires_at
        {
            return Err(IdentityError::Unauthorized);
        }
        Ok(ManagedTokenWire {
            access_token: successor.access_token.clone(),
            refresh_token: successor.refresh_token.clone(),
            token_type: "Bearer",
            expires_in: remaining,
            scope: row.try_get::<Vec<String>, _>("scopes")?.join(" "),
            client_id: protocol::MANAGED_CLIENT,
            installation_id: installation_id.as_str().to_owned(),
            grant_id: f.try_get("id")?,
            budget_credits: f
                .try_get::<Option<i64>, _>("budget_credits")?
                .ok_or(IdentityError::Storage)?,
        })
    }
    pub async fn token_managed(&self, input: ManagedTokenRequest) -> Result<ManagedTokenWire> {
        match input {
            ManagedTokenRequest::Code {
                installation_id,
                code,
                redirect,
                verifier,
            } => {
                self.exchange_managed(&installation_id, &code, &redirect, &verifier)
                    .await
            }
            ManagedTokenRequest::Refresh {
                installation_id,
                refresh,
                rotation_id,
            } => {
                self.refresh_managed(&installation_id, &refresh, &rotation_id)
                    .await
            }
        }
    }
    async fn exchange_managed(
        &self,
        installation_id: &InstallationId,
        code: &str,
        redirect: &str,
        verifier: &str,
    ) -> Result<ManagedTokenWire> {
        self.require_managed_issuance()?;
        protocol::canonical(code, "lmm_oc_")?;
        super::redirect(redirect)?;
        let hash = digest(code)?;
        let mut tx = transaction(&self.identity.pool).await?;
        let id: String = sqlx::query_scalar(
            "SELECT family_id FROM core_oauth.authorization_codes WHERE digest=$1",
        )
        .bind(&hash)
        .fetch_optional(&mut *tx)
        .await?
        .ok_or(IdentityError::Unauthorized)?;
        let f = family(&mut tx, &id, true).await?;
        self.managed_installation(&mut tx, &f, installation_id)
            .await?;
        let row = sqlx::query("SELECT *,consumed_at IS NULL AND expires_at>clock_timestamp() AS live FROM core_oauth.authorization_codes WHERE digest=$1 FOR UPDATE")
            .bind(&hash).fetch_one(&mut *tx).await?;
        if !row.try_get::<bool, _>("live")?
            || row.try_get::<String, _>("client_id")? != protocol::MANAGED_CLIENT
            || row.try_get::<String, _>("issuer")? != self.profile.issuer()
            || row.try_get::<String, _>("resource")? != self.profile.resource()
            || row.try_get::<String, _>("redirect_uri")? != redirect
            || row.try_get::<i64, _>("user_id")? != f.try_get::<i64, _>("user_id")?
            || row.try_get::<i64, _>("owner_account_id")?
                != f.try_get::<i64, _>("owner_account_id")?
            || row.try_get::<i64, _>("user_version")? != f.try_get::<i64, _>("user_version")?
        {
            return Err(IdentityError::Unauthorized);
        }
        super::pkce(
            &row.try_get::<String, _>("pkce_challenge")?,
            Some("S256"),
            Some(verifier),
        )?;
        let current = self
            .live(&mut tx, &f, row.try_get("consent_version")?)
            .await?;
        let scopes: Vec<String> = row.try_get("scopes")?;
        let allowed: Vec<String> = f.try_get("scopes")?;
        if !scope_subset_is_current(&scopes, &allowed, &current)? {
            return Err(IdentityError::Forbidden);
        }
        sqlx::query("UPDATE core_oauth.authorization_codes SET consumed_at=clock_timestamp() WHERE digest=$1 AND consumed_at IS NULL AND expires_at>clock_timestamp() RETURNING digest")
            .bind(hash).fetch_optional(&mut *tx).await?.ok_or(IdentityError::Unauthorized)?;
        let (successor, access_id) = self.managed_successor(&mut tx, &f, &scopes).await?;
        let wire = self
            .managed_wire(&mut tx, &f, installation_id, &successor, &access_id)
            .await?;
        tx.commit().await?;
        Ok(wire)
    }
    async fn refresh_managed(
        &self,
        installation_id: &InstallationId,
        refresh: &str,
        rotation_id: &RotationId,
    ) -> Result<ManagedTokenWire> {
        protocol::canonical(refresh, "lmm_rt_")?;
        let old = digest(refresh)?;
        let mut tx = transaction(&self.identity.pool).await?;
        let id: String =
            sqlx::query_scalar("SELECT family_id FROM core_oauth.refresh_tokens WHERE digest=$1")
                .bind(&old)
                .fetch_optional(&mut *tx)
                .await?
                .ok_or(IdentityError::Unauthorized)?;
        let f = family(&mut tx, &id, true).await?;
        // No functional refusal may mutate a foreign client's installation.
        self.managed_installation(&mut tx, &f, installation_id)
            .await?;
        let token = sqlx::query("SELECT *,consumed_at IS NOT NULL AS consumed,expires_at>clock_timestamp() AS live FROM core_oauth.refresh_tokens WHERE digest=$1 FOR UPDATE")
            .bind(&old).fetch_one(&mut *tx).await?;
        let request_digest = Sha256::digest(
            serde_json::to_vec(&[
                self.profile.issuer(),
                self.profile.resource(),
                protocol::MANAGED_CLIENT,
                &id,
                installation_id.as_str(),
                &crate::hex_codec::encode(&old),
                rotation_id.as_str(),
            ])
            .map_err(|_| IdentityError::Storage)?,
        )
        .to_vec();
        if token.try_get::<bool, _>("consumed")? {
            let lineage = sqlx::query("SELECT l.*,extract(epoch FROM l.created_at)::text AS created_epoch,extract(epoch FROM l.recover_until)::text AS recover_epoch,l.recover_until>clock_timestamp() AS recoverable,r.key_id,r.encrypted_response,s.consumed_at IS NULL AND s.expires_at>clock_timestamp() AS successor_live FROM core_oauth.managed_refresh_lineage l LEFT JOIN core_oauth.managed_refresh_responses r USING(old_refresh_digest) JOIN core_oauth.refresh_tokens s ON s.digest=l.successor_refresh_digest WHERE l.old_refresh_digest=$1")
                .bind(&old).fetch_optional(&mut *tx).await?.ok_or(IdentityError::Storage)?;
            if lineage.try_get::<String, _>("family_id")? != id {
                return Err(IdentityError::Storage);
            }
            if lineage.try_get::<String, _>("rotation_id")? != rotation_id.as_str() {
                sqlx::query("UPDATE core_oauth.families SET revoked_at=COALESCE(revoked_at,clock_timestamp()) WHERE id=$1").bind(&id).execute(&mut *tx).await?;
                tx.commit().await?;
                return Err(IdentityError::Unauthorized);
            }
            if lineage.try_get::<Vec<u8>, _>("request_digest")? != request_digest {
                return Err(IdentityError::Unauthorized);
            }
            if !lineage.try_get::<bool, _>("recoverable")?
                || !lineage.try_get::<bool, _>("successor_live")?
            {
                return Err(IdentityError::Unauthorized);
            }
            let current = self
                .live(&mut tx, &f, token.try_get("consent_version")?)
                .await?;
            let access_id: String = lineage.try_get("successor_access_id")?;
            let scopes: Vec<String> = sqlx::query_scalar(
                "SELECT scopes FROM core_oauth.access_tokens WHERE id=$1 AND family_id=$2",
            )
            .bind(&access_id)
            .bind(&id)
            .fetch_one(&mut *tx)
            .await?;
            let allowed: Vec<String> = f.try_get("scopes")?;
            if !scope_subset_is_current(&scopes, &allowed, &current)? {
                return Err(IdentityError::Forbidden);
            }
            let keys = self.managed_keys.as_ref().ok_or(IdentityError::Storage)?;
            let aad = self.recovery_aad(&lineage, installation_id)?;
            let plaintext = keys.open(
                &lineage
                    .try_get::<Option<String>, _>("key_id")?
                    .ok_or(IdentityError::Storage)?,
                &aad,
                &lineage
                    .try_get::<Option<Vec<u8>>, _>("encrypted_response")?
                    .ok_or(IdentityError::Storage)?,
            )?;
            let successor: Successor =
                serde_json::from_slice(&plaintext).map_err(|_| IdentityError::Storage)?;
            protocol::canonical(&successor.access_token, "lmm_at_")?;
            protocol::canonical(&successor.refresh_token, "lmm_rt_")?;
            if digest(&successor.refresh_token)?
                != lineage.try_get::<Vec<u8>, _>("successor_refresh_digest")?
            {
                return Err(IdentityError::Storage);
            }
            let wire = self
                .managed_wire(&mut tx, &f, installation_id, &successor, &access_id)
                .await?;
            // Recheck after authority and key work, using the database clock.
            let live: bool = sqlx::query_scalar("SELECT recover_until>clock_timestamp() FROM core_oauth.managed_refresh_lineage WHERE old_refresh_digest=$1")
                .bind(&old).fetch_one(&mut *tx).await?;
            if !live {
                return Err(IdentityError::Unauthorized);
            }
            tx.commit().await?;
            return Ok(wire);
        }
        self.require_managed_issuance()?;
        if !token.try_get::<bool, _>("live")? {
            return Err(IdentityError::Unauthorized);
        }
        let current = self
            .live(&mut tx, &f, token.try_get("consent_version")?)
            .await?;
        let scopes: Vec<String> = token.try_get("scopes")?;
        let allowed: Vec<String> = f.try_get("scopes")?;
        if !scope_subset_is_current(&scopes, &allowed, &current)? {
            return Err(IdentityError::Forbidden);
        }
        sqlx::query("UPDATE core_oauth.refresh_tokens SET consumed_at=clock_timestamp() WHERE digest=$1 AND consumed_at IS NULL AND expires_at>clock_timestamp() RETURNING digest")
            .bind(&old).fetch_optional(&mut *tx).await?.ok_or(IdentityError::Unauthorized)?;
        let (successor, access_id) = self.managed_successor(&mut tx, &f, &scopes).await?;
        let lineage = sqlx::query("INSERT INTO core_oauth.managed_refresh_lineage(old_refresh_digest,family_id,rotation_id,request_digest,successor_access_id,successor_refresh_digest) VALUES($1,$2,$3,$4,$5,$6) RETURNING *,extract(epoch FROM created_at)::text AS created_epoch,extract(epoch FROM recover_until)::text AS recover_epoch")
            .bind(&old).bind(&id).bind(rotation_id.as_str()).bind(&request_digest).bind(&access_id).bind(digest(&successor.refresh_token)?).fetch_one(&mut *tx).await?;
        let plaintext =
            Zeroizing::new(serde_json::to_vec(&successor).map_err(|_| IdentityError::Storage)?);
        let aad = self.recovery_aad(&lineage, installation_id)?;
        let keys = self.managed_keys.as_ref().ok_or(IdentityError::Storage)?;
        let (key_id, ciphertext) = keys.seal(&aad, &plaintext)?;
        sqlx::query("INSERT INTO core_oauth.managed_refresh_responses(old_refresh_digest,key_id,encrypted_response) VALUES($1,$2,$3)")
            .bind(&old).bind(key_id).bind(ciphertext).execute(&mut *tx).await?;
        let wire = self
            .managed_wire(&mut tx, &f, installation_id, &successor, &access_id)
            .await?;
        tx.commit().await?;
        Ok(wire)
    }
    fn recovery_aad(&self, row: &PgRow, installation_id: &InstallationId) -> Result<Vec<u8>> {
        serde_json::to_vec(&[
            "lmm.managed-refresh.v1".to_owned(),
            self.profile.issuer().to_owned(),
            self.profile.resource().to_owned(),
            protocol::MANAGED_CLIENT.to_owned(),
            installation_id.as_str().to_owned(),
            row.try_get("family_id")?,
            crate::hex_codec::encode(row.try_get::<Vec<u8>, _>("old_refresh_digest")?),
            row.try_get("rotation_id")?,
            crate::hex_codec::encode(row.try_get::<Vec<u8>, _>("request_digest")?),
            row.try_get("created_epoch")?,
            row.try_get("recover_epoch")?,
            row.try_get("successor_access_id")?,
            crate::hex_codec::encode(row.try_get::<Vec<u8>, _>("successor_refresh_digest")?),
        ])
        .map_err(|_| IdentityError::Storage)
    }
}

impl LmmIssuer {
    pub async fn managed_application_status(&self, access: &str) -> Result<serde_json::Value> {
        protocol::canonical(access, "lmm_at_")?;
        let hash = digest(access)?;
        let mut tx = transaction(&self.identity.pool).await?;
        let token = sqlx::query("SELECT * FROM core_oauth.access_tokens WHERE digest=$1")
            .bind(&hash)
            .fetch_optional(&mut *tx)
            .await?
            .ok_or(IdentityError::Unauthorized)?;
        let f = family(&mut tx, &token.try_get::<String, _>("family_id")?, false).await?;
        if f.try_get::<String, _>("client_id")? != protocol::MANAGED_CLIENT {
            return Err(IdentityError::Unauthorized);
        }
        let current = self
            .live(&mut tx, &f, token.try_get("consent_version")?)
            .await?;
        let allowed: Vec<String> = f.try_get("scopes")?;
        let scopes: Vec<String> = token
            .try_get::<Vec<String>, _>("scopes")?
            .into_iter()
            .filter(|s| {
                allowed.contains(s)
                    && (!s.starts_with("group:")
                        || protocol::decode_group(s).is_ok_and(|g| current.contains(&g)))
            })
            .collect();
        if !scopes.iter().any(|s| s == "catalog:read")
            || !scopes.iter().any(|s| s == "models:invoke")
        {
            return Err(IdentityError::Forbidden);
        }
        scope_groups(&scopes)?;
        let installation_id: String = sqlx::query_scalar(
            "SELECT installation_id FROM core_oauth.managed_family_bindings WHERE family_id=$1",
        )
        .bind(f.try_get::<String, _>("id")?)
        .fetch_one(&mut *tx)
        .await?;
        let expiry: i64 = sqlx::query_scalar("SELECT floor(extract(epoch FROM expires_at))::bigint FROM core_oauth.access_tokens WHERE digest=$1 AND expires_at>clock_timestamp()")
            .bind(hash).fetch_optional(&mut *tx).await?.ok_or(IdentityError::Unauthorized)?;
        let result = serde_json::json!({
            "contract":"lmm.application-status.v1","client_id":protocol::MANAGED_CLIENT,
            "application":APPLICATION,"installation_id":installation_id,
            "grant_id":f.try_get::<String,_>("id")?,
            "budget_credits":f.try_get::<Option<i64>,_>("budget_credits")?.ok_or(IdentityError::Storage)?,
            "scopes":scopes,"access_expires_at":expiry,
        });
        tx.commit().await?;
        Ok(result)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn recovery_key_ids_cannot_alias_the_same_material() {
        assert!(
            ManagedRecoveryKeys::new(
                "current",
                vec![
                    ("current".into(), vec![83; 32]),
                    ("retired".into(), vec![83; 32])
                ],
            )
            .is_err()
        );
        assert!(
            ManagedRecoveryKeys::new(
                "current",
                vec![
                    ("current".into(), vec![83; 32]),
                    ("retired".into(), vec![84; 32])
                ],
            )
            .is_ok()
        );
    }
    #[test]
    fn dedicated_keys_authenticate_binding_and_survive_key_rotation() {
        let old = ManagedRecoveryKeys::new("old", vec![("old".into(), vec![3; 32])]).unwrap();
        let (id, mut ciphertext) = old.seal(b"original binding", b"secret successor").unwrap();
        let rotated = ManagedRecoveryKeys::new(
            "new",
            vec![("new".into(), vec![4; 32]), ("old".into(), vec![3; 32])],
        )
        .unwrap();
        assert_eq!(
            &*rotated.open(&id, b"original binding", &ciphertext).unwrap(),
            b"secret successor"
        );
        assert!(
            rotated
                .open(&id, b"foreign installation", &ciphertext)
                .is_err()
        );
        let missing = ManagedRecoveryKeys::new("new", vec![("new".into(), vec![4; 32])]).unwrap();
        assert!(missing.open(&id, b"original binding", &ciphertext).is_err());
        let last = ciphertext.len() - 1;
        ciphertext[last] ^= 1;
        assert!(rotated.open(&id, b"original binding", &ciphertext).is_err());
        assert!(ManagedRecoveryKeys::new("zero", vec![("zero".into(), vec![0; 32])]).is_err());
        assert!(ManagedRecoveryKeys::new("absent", vec![("other".into(), vec![1; 32])]).is_err());
    }
    #[test]
    fn managed_codec_does_not_turn_optional_scope_or_foreign_client_into_authority() {
        let profile =
            LmmProfile::new("https://managed.example.test", vec!["allowed".into()]).unwrap();
        let form = |client: &str, scope: Option<String>| ManagedTokenForm {
            grant_type: "refresh_token".into(),
            client_id: client.into(),
            resource: profile.resource().into(),
            installation_id: "a".repeat(64),
            code: None,
            redirect_uri: None,
            code_verifier: None,
            refresh_token: Some(protocol::secret("lmm_rt_").unwrap()),
            scope,
            rotation_id: Some("b".repeat(64)),
        };
        assert!(
            form(protocol::MANAGED_CLIENT, None)
                .validate(&profile)
                .is_ok()
        );
        assert!(form("lmm-pi", None).validate(&profile).is_err());
        assert!(
            form(protocol::MANAGED_CLIENT, Some("models:invoke".into()))
                .validate(&profile)
                .is_err()
        );
        let revoke = protocol::RevokeForm {
            client_id: protocol::MANAGED_CLIENT.into(),
            token: protocol::secret("lmm_rt_").unwrap(),
            token_type_hint: None,
        };
        assert!(revoke.validate().is_ok());
        assert!(
            profile
                .model_budget_limit(protocol::MANAGED_CLIENT)
                .is_none()
        );
    }
}

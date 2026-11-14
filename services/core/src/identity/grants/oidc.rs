//! OIDC code-flow overlay on the existing native consent and grant lifecycle.
use super::*;
use base64::{Engine, engine::general_purpose::URL_SAFE_NO_PAD};
use jsonwebtoken::{Algorithm, EncodingKey, Header, encode};
use rsa::{
    PrivateKey, PublicKey,
    pkcs1::EncodeRsaPrivateKey,
    pkcs8::{DecodePrivateKey, DecodePublicKey},
    traits::PublicKeyParts,
};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::{collections::BTreeMap, sync::Arc};

/// Private signing material is never serialized or included in Debug output.
pub struct OidcSigningKeys {
    active: String,
    signing: EncodingKey,
    public: Value,
}
impl OidcSigningKeys {
    /// Reuses the existing RSA/JWT implementation. Retired keys are public only.
    pub fn from_pem(active: &str, private: &[u8], retired: Vec<(String, Vec<u8>)>) -> Result<Self> {
        fn jwk(id: &str, key: &PublicKey) -> Result<Value> {
            if id.is_empty()
                || id.len() > 64
                || !id
                    .bytes()
                    .all(|b| b.is_ascii_alphanumeric() || b"-_".contains(&b))
                || !(2048..=8192).contains(&key.n().bit_width())
            {
                return Err(IdentityError::Invalid);
            }
            Ok(json!({"kty":"RSA","use":"sig","alg":"RS256","kid":id,
                "n":URL_SAFE_NO_PAD.encode(key.n().to_be_bytes_trimmed_vartime()),"e":URL_SAFE_NO_PAD.encode(key.e().to_be_bytes_trimmed_vartime())}))
        }
        if retired.len() > 15 {
            return Err(IdentityError::Invalid);
        }
        let pem = std::str::from_utf8(private).map_err(|_| IdentityError::Invalid)?;
        let key = PrivateKey::from_pkcs8_pem(pem).map_err(|_| IdentityError::Invalid)?;
        let mut keys = BTreeMap::new();
        keys.insert(active.to_owned(), jwk(active, &key.to_public_key())?);
        for (id, pem) in retired {
            let pem = std::str::from_utf8(&pem).map_err(|_| IdentityError::Invalid)?;
            let key = PublicKey::from_public_key_pem(pem).map_err(|_| IdentityError::Invalid)?;
            let value = jwk(&id, &key)?;
            if keys.insert(id, value).is_some() {
                return Err(IdentityError::Invalid);
            }
        }
        Ok(Self {
            active: active.into(),
            signing: EncodingKey::from_rsa_der(
                key.to_pkcs1_der()
                    .map_err(|_| IdentityError::Invalid)?
                    .as_bytes(),
            ),
            public: json!({"keys":keys.into_values().collect::<Vec<_>>()}),
        })
    }
    fn sign(&self, claims: IdTokenClaims<'_>) -> Result<String> {
        let mut header = Header::new(Algorithm::RS256);
        header.kid = Some(self.active.clone());
        encode(&header, &claims, &self.signing).map_err(|_| IdentityError::Storage)
    }
}

#[derive(Serialize)]
struct IdTokenClaims<'a> {
    iss: &'a str,
    sub: String,
    aud: &'a str,
    iat: i64,
    exp: i64,
    auth_time: i64,
    #[serde(skip_serializing_if = "Option::is_none")]
    nonce: Option<&'a str>,
}

/// Strict OIDC request; ordinary OAuth requests retain their original wire type.
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct OidcAuthorizationRequest {
    pub response_type: String,
    pub client_id: String,
    pub redirect_uri: String,
    #[serde(deserialize_with = "openid_scope")]
    pub scope: String,
    pub resource: String,
    pub code_challenge: String,
    pub code_challenge_method: String,
    pub state: String,
    pub nonce: Option<String>,
}
fn openid_scope<'de, D: serde::Deserializer<'de>>(
    deserializer: D,
) -> std::result::Result<String, D::Error> {
    let scope = String::deserialize(deserializer)?;
    if scope.split(' ').filter(|s| *s == "openid").count() != 1 {
        return Err(serde::de::Error::custom("one openid scope is required"));
    }
    Ok(scope)
}
impl OidcAuthorizationRequest {
    fn native(self) -> Result<(AuthorizationRequest, Option<String>)> {
        let mut scopes: Vec<_> = self.scope.split(' ').collect();
        if self.client_id != protocol::CLIENT
            || scopes.iter().filter(|s| **s == "openid").count() != 1
            || self
                .nonce
                .as_ref()
                .is_some_and(|n| n.is_empty() || n.len() > 256 || n.chars().any(char::is_control))
        {
            return Err(IdentityError::Invalid);
        }
        scopes.retain(|s| *s != "openid");
        Ok((
            AuthorizationRequest {
                response_type: self.response_type,
                client_id: self.client_id,
                redirect_uri: self.redirect_uri,
                scope: scopes.join(" "),
                resource: self.resource,
                code_challenge: self.code_challenge,
                code_challenge_method: self.code_challenge_method,
                state: self.state,
            },
            self.nonce,
        ))
    }
}
impl LmmIssuer {
    pub async fn with_oidc_signing_keys(mut self, keys: OidcSigningKeys) -> Result<Self> {
        // A configured signer never installs or repairs its persistent state.
        sqlx::query("SELECT r.nonce,f.auth_time,t.access_digest FROM core_oidc.authorization_requests r CROSS JOIN core_oidc.families f CROSS JOIN core_oidc.tokens t LIMIT 0").execute(&self.identity.pool).await?;
        self.oidc_keys = Some(Arc::new(keys));
        Ok(self)
    }
    pub fn oidc_metadata(&self) -> Option<Value> {
        self.oidc_keys.as_ref()?;
        let issuer = self.profile.issuer();
        let resource = self.profile.resource();
        Some(
            json!({"issuer":issuer,"authorization_endpoint":format!("{resource}/authorize"),"token_endpoint":format!("{resource}/token"),"revocation_endpoint":format!("{resource}/revoke"),"jwks_uri":format!("{resource}/jwks"),"userinfo_endpoint":format!("{resource}/userinfo"),"response_types_supported":["code"],"response_modes_supported":["query"],"grant_types_supported":["authorization_code","refresh_token"],"subject_types_supported":["public"],"id_token_signing_alg_values_supported":["RS256"],"token_endpoint_auth_methods_supported":["none"],"code_challenge_methods_supported":["S256"],"scopes_supported":["openid","catalog:read","balance:read"],"claims_supported":["iss","sub","aud","iat","exp","nonce","auth_time"],"claims_parameter_supported":false,"request_parameter_supported":false,"request_uri_parameter_supported":false,"authorization_response_iss_parameter_supported":true,"lmm_client_ids_supported":["lmm"]}),
        )
    }
    pub fn oidc_jwks(&self) -> Option<Value> {
        self.oidc_keys.as_ref().map(|keys| keys.public.clone())
    }
    pub async fn begin_oidc(&self, input: OidcAuthorizationRequest) -> Result<ConsentPage> {
        self.oidc_keys.as_ref().ok_or(IdentityError::Forbidden)?;
        let (request, nonce) = input.native()?;
        let request = self.profile.validate_authorization(request)?;
        let mut tx = transaction(&self.identity.pool).await?;
        let mut page = self.pending(&mut tx, &request, None, &[], None).await?;
        sqlx::query(
            "INSERT INTO core_oidc.authorization_requests(binding_digest,nonce) VALUES($1,$2)",
        )
        .bind(digest(&page.binding)?)
        .bind(nonce)
        .execute(&mut *tx)
        .await?;
        page.openid = true;
        tx.commit().await?;
        Ok(page)
    }
    pub(super) async fn copy_oidc_request(
        &self,
        c: &mut PgConnection,
        predecessor: &str,
        page: &mut ConsentPage,
    ) -> Result<()> {
        if self.oidc_keys.is_none() {
            return Ok(());
        }
        let changed=sqlx::query("INSERT INTO core_oidc.authorization_requests(binding_digest,nonce) SELECT $1,nonce FROM core_oidc.authorization_requests WHERE binding_digest=$2").bind(digest(&page.binding)?).bind(digest(predecessor)?).execute(c).await?;
        page.openid = changed.rows_affected() == 1;
        Ok(())
    }
    pub(super) async fn bind_oidc_family(
        &self,
        c: &mut PgConnection,
        binding: &str,
        family: &str,
        sid: &str,
    ) -> Result<()> {
        if self.oidc_keys.is_none() {
            return Ok(());
        }
        sqlx::query("INSERT INTO core_oidc.families(family_id,nonce,auth_time) SELECT $1,r.nonce,floor(extract(epoch FROM s.created_at))::bigint FROM core_oidc.authorization_requests r CROSS JOIN core_identity.web_sessions s WHERE r.binding_digest=$2 AND s.sid=$3").bind(family).bind(digest(binding)?).bind(sid).execute(c).await?;
        Ok(())
    }
    pub(super) async fn issue_oidc(
        &self,
        c: &mut PgConnection,
        f: &PgRow,
        wire: &mut TokenWire,
        enabled: bool,
    ) -> Result<()> {
        let Some(keys) = &self.oidc_keys else {
            return Ok(());
        };
        if !enabled {
            return Ok(());
        }
        let Some(binding) =
            sqlx::query("SELECT nonce,auth_time FROM core_oidc.families WHERE family_id=$1")
                .bind(f.try_get::<String, _>("id")?)
                .fetch_optional(&mut *c)
                .await?
        else {
            return Ok(());
        };
        let (now,expires):(i64,i64)=sqlx::query_as("SELECT floor(extract(epoch FROM clock_timestamp()))::bigint,floor(extract(epoch FROM expires_at))::bigint FROM core_oauth.access_tokens WHERE digest=$1").bind(digest(&wire.access_token)?).fetch_one(&mut *c).await?;
        wire.id_token = Some(keys.sign(IdTokenClaims {
            iss: self.profile.issuer(),
            aud: &f.try_get::<String, _>("client_id")?,
            sub: f.try_get::<i64, _>("user_id")?.to_string(),
            iat: now,
            exp: expires,
            nonce: binding.try_get::<Option<String>, _>("nonce")?.as_deref(),
            auth_time: binding.try_get("auth_time")?,
        })?);
        wire.scope.push_str(" openid");
        sqlx::query(
            "INSERT INTO core_oidc.tokens(access_digest,refresh_digest,family_id) VALUES($1,$2,$3)",
        )
        .bind(digest(&wire.access_token)?)
        .bind(digest(&wire.refresh_token)?)
        .bind(f.try_get::<String, _>("id")?)
        .execute(c)
        .await?;
        Ok(())
    }
    pub(super) async fn oidc_refresh_scope(
        &self,
        c: &mut PgConnection,
        refresh: &str,
        scope: Option<&str>,
    ) -> Result<(bool, Option<String>)> {
        if self.oidc_keys.is_none() {
            return Ok((false, scope.map(str::to_owned)));
        }
        let previous: bool = sqlx::query_scalar(
            "SELECT EXISTS(SELECT 1 FROM core_oidc.tokens WHERE refresh_digest=$1)",
        )
        .bind(digest(refresh)?)
        .fetch_one(c)
        .await?;
        let enabled = scope
            .map(|s| s.split(' ').any(|s| s == "openid"))
            .unwrap_or(previous);
        if enabled && !previous {
            return Err(IdentityError::Forbidden);
        }
        let native = scope.map(|s| {
            s.split(' ')
                .filter(|s| *s != "openid")
                .collect::<Vec<_>>()
                .join(" ")
        });
        if scope.is_some_and(|s| s.split(' ').filter(|s| *s == "openid").count() > 1) {
            return Err(IdentityError::Invalid);
        }
        Ok((enabled, native))
    }
    pub async fn userinfo(&self, access: &str) -> std::result::Result<Value, ReadError> {
        self.oidc_keys.as_ref().ok_or(IdentityError::Forbidden)?;
        protocol::canonical(access, "lmm_at_")?;
        let mut tx = transaction(&self.identity.pool).await?;
        let hash = digest(access)?;
        let token=sqlx::query("SELECT family_id,consent_version,scopes FROM core_oauth.access_tokens WHERE digest=$1 AND expires_at>clock_timestamp()").bind(&hash).fetch_optional(&mut *tx).await?.ok_or(IdentityError::Unauthorized)?;
        let family = family(&mut tx, &token.try_get::<String, _>("family_id")?, false).await?;
        let current = self
            .live(&mut tx, &family, token.try_get("consent_version")?)
            .await?;
        if !super::issuer::scope_subset_is_current(
            &token.try_get::<Vec<String>, _>("scopes")?,
            &family.try_get::<Vec<String>, _>("scopes")?,
            &current,
        )? {
            return Err(IdentityError::Forbidden.into());
        }
        let granted: bool = sqlx::query_scalar(
            "SELECT EXISTS(SELECT 1 FROM core_oidc.tokens WHERE access_digest=$1 AND family_id=$2)",
        )
        .bind(hash)
        .bind(family.try_get::<String, _>("id")?)
        .fetch_one(&mut *tx)
        .await?;
        if !granted {
            return Err(IdentityError::Forbidden.into());
        }
        let result = json!({"sub":family.try_get::<i64,_>("user_id")?.to_string()});
        tx.commit().await?;
        Ok(result)
    }
}

use super::{
    network::ProviderNetwork,
    policy,
    provider::{FederatedProvider, MappedUser, ProviderConfiguration, ProviderKind},
};
use crate::identity::{IdentityError, Result};
use base64::{Engine, engine::general_purpose::STANDARD};
use jsonwebtoken::{Algorithm, DecodingKey, Validation, decode, decode_header};
use serde::Deserialize;
use serde_json::Value;
use sha2::{Digest, Sha256};
use std::time::{Duration, SystemTime, UNIX_EPOCH};
use subtle::ConstantTimeEq;
use tokio::time::Instant;
use zeroize::Zeroize;

pub(super) const FLOW_BUDGET: Duration = Duration::from_secs(20);

pub(super) enum ExchangeOutcome {
    Identity(VerifiedFederatedIdentity),
    Denied(String),
}

/// Only successful token/JWT/userinfo or bridge verification constructs this.
pub(super) struct VerifiedFederatedIdentity {
    provider_id: i64,
    revision: i64,
    kind: ProviderKind,
    mapped: MappedUser,
}

impl VerifiedFederatedIdentity {
    pub(super) fn provider_id(&self) -> i64 {
        self.provider_id
    }
    pub(super) fn revision(&self) -> i64 {
        self.revision
    }
    pub(super) fn kind(&self) -> ProviderKind {
        self.kind
    }
    pub(super) fn user(&self) -> &MappedUser {
        &self.mapped
    }
}

struct TokenForm(Vec<(String, String)>);
impl Drop for TokenForm {
    fn drop(&mut self) {
        for (_, value) in &mut self.0 {
            value.zeroize();
        }
    }
}

#[derive(Deserialize)]
struct TokenReply {
    #[serde(default)]
    access_token: String,
    #[serde(default)]
    token_type: String,
    #[serde(default)]
    id_token: String,
    #[serde(default)]
    error: String,
}
impl Drop for TokenReply {
    fn drop(&mut self) {
        self.access_token.zeroize();
        self.id_token.zeroize();
    }
}

fn token_reply(raw: &str) -> Result<TokenReply> {
    if raw.len() > policy::MAX_JSON_BYTES {
        return Err(IdentityError::Unauthorized);
    }
    let reply = if raw.trim_start().starts_with('{') {
        serde_json::from_str::<TokenReply>(raw).map_err(|_| IdentityError::Unauthorized)?
    } else {
        let mut reply = TokenReply {
            access_token: String::new(),
            token_type: String::new(),
            id_token: String::new(),
            error: String::new(),
        };
        let mut seen = std::collections::BTreeSet::new();
        for (name, value) in form_urlencoded::parse(raw.as_bytes()) {
            let target = match name.as_ref() {
                "access_token" => &mut reply.access_token,
                "token_type" => &mut reply.token_type,
                "id_token" => &mut reply.id_token,
                "error" => &mut reply.error,
                _ => continue,
            };
            if !seen.insert(name.into_owned()) {
                return Err(IdentityError::Unauthorized);
            }
            *target = value.into_owned();
        }
        reply
    };
    if !reply.error.is_empty()
        || reply.access_token.is_empty()
        || reply.access_token.len() > 65536
        || reply.access_token.chars().any(char::is_control)
        || reply.token_type.len() > 64
        || reply
            .token_type
            .chars()
            .any(|c| !c.is_ascii_alphanumeric() && c != '_' && c != '-')
    {
        return Err(IdentityError::Unauthorized);
    }
    Ok(reply)
}

#[derive(Deserialize)]
#[serde(untagged)]
enum Audience {
    One(String),
    Many(Vec<String>),
}

impl Audience {
    fn valid(&self, client: &str, azp: Option<&str>) -> bool {
        match self {
            Self::One(aud) => aud == client && azp.is_none_or(|party| party == client),
            Self::Many(aud) => {
                !aud.is_empty()
                    && aud.len() <= 16
                    && aud.iter().any(|aud| aud == client)
                    && (aud.len() == 1 || azp == Some(client))
                    && azp.is_none_or(|party| party == client)
            }
        }
    }
}

#[derive(Deserialize)]
struct Claims {
    iss: String,
    sub: String,
    aud: Audience,
    exp: u64,
    iat: u64,
    nonce: String,
    azp: Option<String>,
}

fn verify_id_token(
    token: &str,
    keys: &Value,
    config: &ProviderConfiguration,
    nonce: &[u8],
    secret: &str,
) -> Result<Claims> {
    if token.len() > 16384 || nonce.len() != 32 {
        return Err(IdentityError::Unauthorized);
    }
    let header = decode_header(token).map_err(|_| IdentityError::Unauthorized)?;
    let algorithm = serde_json::to_value(header.alg).map_err(|_| IdentityError::Unauthorized)?;
    let algorithm = algorithm.as_str().ok_or(IdentityError::Unauthorized)?;
    if !config.id_token_algorithms.iter().any(|a| a == algorithm) {
        return Err(IdentityError::Unauthorized);
    }
    let key = if matches!(
        header.alg,
        Algorithm::HS256 | Algorithm::HS384 | Algorithm::HS512
    ) {
        DecodingKey::from_secret(secret.as_bytes())
    } else {
        let candidates = keys
            .get("keys")
            .and_then(Value::as_array)
            .ok_or(IdentityError::Unauthorized)?;
        if candidates.is_empty() || candidates.len() > 32 {
            return Err(IdentityError::Unauthorized);
        }
        let candidates: Vec<_> = candidates
            .iter()
            .filter(|key| {
                header
                    .kid
                    .as_deref()
                    .is_none_or(|kid| key.get("kid").and_then(Value::as_str) == Some(kid))
                    && key
                        .get("alg")
                        .and_then(Value::as_str)
                        .is_none_or(|alg| alg == algorithm)
                    && key
                        .get("use")
                        .and_then(Value::as_str)
                        .is_none_or(|usage| usage == "sig")
                    && key.get("key_ops").is_none_or(|ops| {
                        ops.as_array()
                            .is_some_and(|ops| ops.iter().any(|op| op.as_str() == Some("verify")))
                    })
                    && key
                        .get("kty")
                        .and_then(Value::as_str)
                        .is_some_and(|kind| match header.alg {
                            Algorithm::RS256
                            | Algorithm::RS384
                            | Algorithm::RS512
                            | Algorithm::PS256
                            | Algorithm::PS384
                            | Algorithm::PS512 => kind == "RSA",
                            Algorithm::ES256 | Algorithm::ES384 => kind == "EC",
                            Algorithm::EdDSA => kind == "OKP",
                            _ => false,
                        })
                    && !["n", "e", "x", "y"].iter().any(|field| {
                        key.get(field)
                            .and_then(Value::as_str)
                            .is_some_and(|value| value.len() > 2048)
                    })
            })
            .collect();
        if candidates.len() != 1 {
            return Err(IdentityError::Unauthorized);
        }
        let jwk = serde_json::from_value(candidates[0].clone())
            .map_err(|_| IdentityError::Unauthorized)?;
        DecodingKey::from_jwk(&jwk).map_err(|_| IdentityError::Unauthorized)?
    };
    let mut validation = Validation::new(header.alg);
    validation.set_issuer(&[&config.issuer]);
    validation.set_audience(&[&config.client_id]);
    validation.set_required_spec_claims(&["iss", "sub", "aud", "exp", "iat", "nonce"]);
    validation.validate_nbf = true;
    validation.leeway = 60;
    let claims = decode::<Claims>(token, &key, &validation)
        .map_err(|_| IdentityError::Unauthorized)?
        .claims;
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_err(|_| IdentityError::Storage)?
        .as_secs();
    if claims.iss != config.issuer
        || !claims.aud.valid(&config.client_id, claims.azp.as_deref())
        || claims.sub.is_empty()
        || claims.sub.len() > 255
        || claims.sub.chars().any(char::is_control)
        || claims.iat >= claims.exp
        || claims.iat > now.saturating_add(60)
        || !bool::from(
            Sha256::digest(claims.nonce.as_bytes())
                .as_slice()
                .ct_eq(nonce),
        )
    {
        return Err(IdentityError::Unauthorized);
    }
    if matches!(
        header.alg,
        Algorithm::HS256 | Algorithm::HS384 | Algorithm::HS512
    ) && matches!(&claims.aud, Audience::Many(aud) if aud.len()!=1)
    {
        return Err(IdentityError::Unauthorized);
    }
    Ok(claims)
}

pub(super) async fn discover(
    network: &ProviderNetwork,
    raw: &str,
    expected_issuer: Option<&str>,
) -> Result<Value> {
    let url = network.endpoint(raw)?;
    let body = network
        .get(
            &url,
            None,
            Instant::now() + FLOW_BUDGET,
            policy::MAX_JSON_BYTES,
        )
        .await?;
    let value: Value = serde_json::from_str(&body).map_err(|_| IdentityError::Unauthorized)?;
    let issuer = value
        .get("issuer")
        .and_then(Value::as_str)
        .ok_or(IdentityError::Invalid)?;
    let issuer_url = network.endpoint(issuer)?;
    if issuer_url.query().is_some() || expected_issuer.is_some_and(|expected| issuer != expected) {
        return Err(IdentityError::Invalid);
    }
    for field in [
        "authorization_endpoint",
        "token_endpoint",
        "userinfo_endpoint",
        "jwks_uri",
    ] {
        if let Some(endpoint) = value.get(field).and_then(Value::as_str) {
            network.endpoint(endpoint)?;
        }
    }
    Ok(value)
}

pub(super) struct CodeExchangeInput<'a> {
    pub(super) code: &'a str,
    pub(super) redirect: &'a str,
    pub(super) verifier: Option<&'a str>,
    pub(super) nonce: Option<&'a [u8]>,
    pub(super) response_issuer: Option<&'a str>,
}
pub(super) async fn exchange(
    network: &ProviderNetwork,
    provider: &FederatedProvider,
    secret: &str,
    input: CodeExchangeInput<'_>,
) -> Result<ExchangeOutcome> {
    let CodeExchangeInput {
        code,
        redirect,
        verifier,
        nonce,
        response_issuer,
    } = input;
    if code.is_empty() || code.len() > 4096 || code.chars().any(char::is_control) {
        return Err(IdentityError::Unauthorized);
    }
    let deadline = Instant::now() + FLOW_BUDGET;
    let config = &provider.configuration;
    if provider.kind == ProviderKind::Wechat {
        let mut url = network.bridge_endpoint(&config.bridge_url)?;
        if url.query().is_some() {
            return Err(IdentityError::Invalid);
        }
        let path = format!("{}/api/wechat/user", url.path().trim_end_matches('/'));
        url.set_path(&path);
        url.query_pairs_mut().append_pair("code", code);
        // The old bridge uses its raw configured Authorization token.
        let raw = network.bridge_get(&url, secret, deadline).await?;
        #[derive(Deserialize)]
        struct Bridge {
            success: bool,
            data: String,
        }
        let bridge: Bridge = serde_json::from_str(&raw).map_err(|_| IdentityError::Unauthorized)?;
        if !bridge.success
            || bridge.data.is_empty()
            || bridge.data.len() > 65536
            || bridge.data.contains('\0')
        {
            return Err(IdentityError::Unauthorized);
        }
        return Ok(ExchangeOutcome::Identity(VerifiedFederatedIdentity {
            provider_id: provider.id,
            revision: provider.revision,
            kind: provider.kind,
            mapped: MappedUser {
                subject: bridge.data,
                username: String::new(),
                display_name: String::new(),
                email: String::new(),
                email_verified: false,
            },
        }));
    }
    if provider.kind == ProviderKind::Oidc
        && response_issuer.is_some_and(|issuer| issuer != config.issuer)
    {
        return Err(IdentityError::Unauthorized);
    }
    let mut form = TokenForm(vec![
        ("grant_type".into(), "authorization_code".into()),
        ("code".into(), code.into()),
        ("redirect_uri".into(), redirect.into()),
    ]);
    let authorization = if config.auth_style == 2 {
        Some(zeroize::Zeroizing::new(format!(
            "Basic {}",
            STANDARD.encode(
                zeroize::Zeroizing::new(format!("{}:{secret}", config.client_id)).as_bytes()
            )
        )))
    } else {
        // Legacy auto=0 explicitly means params; do not probe/fallback/replay.
        form.0.push(("client_id".into(), config.client_id.clone()));
        form.0.push(("client_secret".into(), secret.into()));
        None
    };
    if config.pkce_s256 {
        form.0.push((
            "code_verifier".into(),
            verifier.ok_or(IdentityError::Unauthorized)?.into(),
        ));
    }
    let token_url = network.endpoint(&config.token_endpoint)?;
    let reply = network
        .form(
            &token_url,
            authorization.as_deref().map(String::as_str),
            &form.0,
            deadline,
        )
        .await;
    let tokens = token_reply(&reply?)?;
    let claims = if provider.kind == ProviderKind::Oidc {
        if tokens.id_token.is_empty() {
            return Err(IdentityError::Unauthorized);
        }
        let url = network.endpoint(&config.jwks_uri)?;
        let raw = network
            .get(&url, None, deadline, policy::MAX_JSON_BYTES)
            .await?;
        let keys = serde_json::from_str(&raw).map_err(|_| IdentityError::Unauthorized)?;
        Some(verify_id_token(
            &tokens.id_token,
            &keys,
            config,
            nonce.ok_or(IdentityError::Unauthorized)?,
            secret,
        )?)
    } else {
        None
    };
    let token_type =
        if tokens.token_type.is_empty() || tokens.token_type.eq_ignore_ascii_case("Bearer") {
            "Bearer"
        } else {
            &tokens.token_type
        };
    let authorization = zeroize::Zeroizing::new(format!("{token_type} {}", tokens.access_token));
    let user_url = network.endpoint(&config.user_info_endpoint)?;
    let raw = network
        .get(
            &user_url,
            Some(&authorization),
            deadline,
            policy::MAX_JSON_BYTES,
        )
        .await?;
    let permit = tokio::time::timeout_at(deadline, network.profile_slots.clone().acquire_owned())
        .await
        .map_err(|_| IdentityError::Unauthorized)?
        .map_err(|_| IdentityError::Storage)?;
    let config = config.clone();
    let (id, revision, kind, name) = (
        provider.id,
        provider.revision,
        provider.kind,
        provider.name.clone(),
    );
    // A cancelled caller cannot release the worker permit while gjson is still
    // evaluating. One bounded worker protects the async runtime and pool.
    let work = tokio::task::spawn_blocking(move || {
        let _permit = permit;
        let mut mapped = config.mapped_user(&raw)?;
        if let Some(claims) = claims {
            // Builtin OIDC's stable subject and email are strict strings, unlike
            // custom OAuth's explicitly configured gjson mapping/coercion rules.
            #[derive(Deserialize)]
            struct Info {
                sub: String,
                email: String,
                email_verified: Option<bool>,
            }
            let info: Info = serde_json::from_str(&raw).map_err(|_| IdentityError::Unauthorized)?;
            if info.sub != claims.sub || info.email.is_empty() {
                return Err(IdentityError::Unauthorized);
            }
            mapped.subject = claims.sub;
            mapped.email = info.email;
            mapped.email_verified = info.email_verified == Some(true);
        }
        if let Some(policy) = policy::parse(&config.access_policy)?
            && let Some(failure) = policy.evaluate(&raw)?
        {
            return Ok(ExchangeOutcome::Denied(failure.message(
                &config.access_denied_message,
                &name,
                &raw,
            )?));
        }
        Ok(ExchangeOutcome::Identity(VerifiedFederatedIdentity {
            provider_id: id,
            revision,
            kind,
            mapped,
        }))
    });
    tokio::time::timeout_at(deadline, work)
        .await
        .map_err(|_| IdentityError::Unauthorized)?
        .map_err(|_| IdentityError::Storage)?
}

#[cfg(test)]
#[path = "protocol_tests.rs"]
mod tests;

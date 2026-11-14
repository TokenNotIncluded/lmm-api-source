//! Durable, browser-bound and purpose-specific ceremonies. Network I/O only
//! starts after consumption commits; authority is checked again afterwards.
use super::{
    FederatedProvider, ProviderKind,
    protocol::{self, ExchangeOutcome, VerifiedFederatedIdentity},
};
use crate::identity::{IdentityError, IdentityStore, Result, auth, digest, new_secret};
use base64::{Engine, engine::general_purpose::URL_SAFE_NO_PAD};
use serde::Serialize;
use sha2::{Digest, Sha256};
use sqlx::{PgConnection, Row};

#[derive(Clone, Copy, Eq, PartialEq)]
pub(crate) enum FlowPurpose {
    Login,
    Bind,
}
impl FlowPurpose {
    fn as_str(self) -> &'static str {
        match self {
            Self::Login => "login",
            Self::Bind => "bind",
        }
    }
}

#[derive(Clone, Eq, PartialEq)]
pub(crate) struct FlowOwner {
    pub user: i64,
    pub version: i64,
    pub sid: String,
}

#[derive(Serialize)]
pub struct FederatedStart {
    pub authorization_url: String,
    pub state: String,
    pub expires_at: i64,
}

pub(crate) struct FlowStartInput<'a> {
    pub(crate) provider: &'a FederatedProvider,
    pub(crate) binding: &'a str,
    pub(crate) origin: &'a str,
    pub(crate) purpose: FlowPurpose,
    pub(crate) owner: Option<&'a FlowOwner>,
    pub(crate) accepted_legal: bool,
    pub(crate) affiliate: &'a str,
}
pub(crate) struct FlowConsumeInput<'a> {
    pub(crate) slug: &'a str,
    pub(crate) state: &'a str,
    pub(crate) binding: &'a str,
    pub(crate) purpose: FlowPurpose,
    pub(crate) owner: Option<&'a FlowOwner>,
    pub(crate) code: &'a str,
}
pub(crate) struct ConsumedFlow {
    provider: FederatedProvider,
    nonce: Option<Vec<u8>>,
    verifier: Option<String>,
    owner: Option<FlowOwner>,
    accepted_legal: bool,
    affiliate: String,
}

pub(crate) struct FederatedExchange {
    flow: ConsumedFlow,
    proof: VerifiedFederatedIdentity,
}
impl FederatedExchange {
    pub(crate) fn owner(&self) -> Option<&FlowOwner> {
        self.flow.owner.as_ref()
    }
    pub(crate) fn provider_id(&self) -> i64 {
        self.flow.provider.id
    }
    pub(crate) fn subject(&self) -> &str {
        &self.proof.user().subject
    }
    pub(crate) fn kind(&self) -> ProviderKind {
        self.proof.kind()
    }
}
pub(crate) enum ExchangeResult {
    Verified(Box<FederatedExchange>),
    Denied(String),
}

impl IdentityStore {
    pub(crate) async fn create_federated_flow(
        &self,
        connection: &mut PgConnection,
        input: FlowStartInput<'_>,
    ) -> Result<FederatedStart> {
        let FlowStartInput {
            provider,
            binding,
            origin,
            purpose,
            owner,
            accepted_legal,
            affiliate,
        } = input;
        self.federated_auth()?;
        if (purpose == FlowPurpose::Bind) != owner.is_some() || affiliate.len() > 256 {
            return Err(IdentityError::Invalid);
        }
        let current = self.federated_provider_in(connection, provider.id).await?;
        if current.revision != provider.revision {
            return Err(IdentityError::Conflict);
        }
        let state = new_secret("lmmfe_")?;
        let nonce = if provider.kind == ProviderKind::Oidc {
            Some(new_secret("")?)
        } else {
            None
        };
        let verifier = if provider.configuration.pkce_s256 {
            Some(new_secret("")?)
        } else {
            None
        };
        let callback = format!("{}/oauth/{}", origin.trim_end_matches('/'), provider.slug);
        let authorization_url = if provider.kind == ProviderKind::Wechat {
            String::new()
        } else {
            let mut url = self
                .federated_auth()?
                .network
                .endpoint(&provider.configuration.authorization_endpoint)?;
            let retained: Vec<(String, String)> = url
                .query_pairs()
                .filter(|(key, _)| {
                    !matches!(
                        key.as_ref(),
                        "client_id"
                            | "response_type"
                            | "redirect_uri"
                            | "scope"
                            | "state"
                            | "nonce"
                            | "code_challenge"
                            | "code_challenge_method"
                    )
                })
                .map(|(key, value)| (key.into_owned(), value.into_owned()))
                .collect();
            url.set_query(None);
            let mut pairs = url.query_pairs_mut();
            pairs
                .extend_pairs(retained)
                .append_pair("client_id", &provider.configuration.client_id)
                .append_pair("response_type", "code")
                .append_pair("redirect_uri", &callback)
                .append_pair("scope", &provider.configuration.scopes)
                .append_pair("state", &state);
            if let Some(nonce) = &nonce {
                pairs.append_pair("nonce", nonce);
            }
            if let Some(verifier) = &verifier {
                pairs
                    .append_pair(
                        "code_challenge",
                        &URL_SAFE_NO_PAD.encode(Sha256::digest(verifier.as_bytes())),
                    )
                    .append_pair("code_challenge_method", "S256");
            }
            drop(pairs);
            url.to_string()
        };
        let expires_at: i64 = sqlx::query_scalar("INSERT INTO core_federated_auth.flows(digest,provider_id,provider_revision,purpose,binding_digest,nonce_digest,pkce_verifier,user_id,user_version,sid,accepted_legal,affiliate) VALUES($1,$2,$3,$4,$5,$6,$7,$8,$9,$10,$11,$12) RETURNING floor(extract(epoch FROM expires_at))::bigint")
            .bind(digest(&state)?).bind(provider.id).bind(provider.revision).bind(purpose.as_str())
            .bind(digest(binding)?).bind(nonce.as_ref().map(|value| Sha256::digest(value.as_bytes()).to_vec()))
            .bind(verifier).bind(owner.map(|owner| owner.user)).bind(owner.map(|owner| owner.version))
            .bind(owner.map(|owner| owner.sid.as_str())).bind(accepted_legal).bind(affiliate)
            .fetch_one(&mut *connection).await?;
        sqlx::query("DELETE FROM core_federated_auth.flows WHERE digest IN (SELECT digest FROM core_federated_auth.flows WHERE expires_at<=clock_timestamp() ORDER BY expires_at LIMIT 100)")
            .execute(connection).await?;
        Ok(FederatedStart {
            authorization_url,
            state,
            expires_at,
        })
    }

    pub(crate) async fn consume_federated_flow(
        &self,
        connection: &mut PgConnection,
        input: FlowConsumeInput<'_>,
    ) -> Result<ConsumedFlow> {
        let FlowConsumeInput {
            slug,
            state,
            binding,
            purpose,
            owner,
            code,
        } = input;
        if !state.starts_with("lmmfe_")
            || state.len() != 70
            || code.is_empty()
            || code.len() > 4096
            || code.chars().any(char::is_control)
            || (purpose == FlowPurpose::Bind) != owner.is_some()
        {
            return Err(IdentityError::Unauthorized);
        }
        let row = sqlx::query("UPDATE core_federated_auth.flows SET consumed_at=clock_timestamp() WHERE digest=$1 AND binding_digest=$2 AND purpose=$3 AND user_id IS NOT DISTINCT FROM $4 AND user_version IS NOT DISTINCT FROM $5 AND sid IS NOT DISTINCT FROM $6 AND consumed_at IS NULL AND expires_at>clock_timestamp() RETURNING *")
            .bind(digest(state)?).bind(digest(binding)?).bind(purpose.as_str())
            .bind(owner.map(|owner| owner.user)).bind(owner.map(|owner| owner.version)).bind(owner.map(|owner| owner.sid.as_str()))
            .fetch_optional(&mut *connection).await?.ok_or(IdentityError::Unauthorized)?;
        let provider = self
            .federated_provider_in(connection, row.try_get("provider_id")?)
            .await?;
        if provider.slug != slug
            || provider.revision != row.try_get::<i64, _>("provider_revision")?
        {
            return Err(IdentityError::Unauthorized);
        }
        if provider.kind == ProviderKind::Wechat {
            sqlx::query("DELETE FROM core_federated_auth.assertions WHERE digest IN (SELECT digest FROM core_federated_auth.assertions WHERE expires_at<=clock_timestamp() ORDER BY expires_at LIMIT 100)")
                .execute(&mut *connection).await?;
            for claim in self
                .federated_auth()?
                .keys
                .code_digests(provider.id, code)?
            {
                let inserted = sqlx::query("INSERT INTO core_federated_auth.assertions(digest) VALUES($1) ON CONFLICT DO NOTHING")
                    .bind(claim.as_slice()).execute(&mut *connection).await?.rows_affected();
                if inserted != 1 {
                    return Err(IdentityError::Unauthorized);
                }
            }
        }
        Ok(ConsumedFlow {
            provider,
            nonce: row.try_get("nonce_digest")?,
            verifier: row.try_get("pkce_verifier")?,
            owner: owner.cloned(),
            accepted_legal: row.try_get("accepted_legal")?,
            affiliate: row.try_get("affiliate")?,
        })
    }

    pub(crate) async fn exchange_federated_flow(
        &self,
        flow: ConsumedFlow,
        origin: &str,
        code: &str,
        issuer: Option<&str>,
    ) -> Result<ExchangeResult> {
        let auth = self.federated_auth()?;
        let secret = auth.keys.open(
            &flow.provider.secret_context,
            flow.provider.revision,
            &flow.provider.secret_key_id,
            &flow.provider.sealed_secret,
        )?;
        let callback = format!(
            "{}/oauth/{}",
            origin.trim_end_matches('/'),
            flow.provider.slug
        );
        match protocol::exchange(
            &auth.network,
            &flow.provider,
            &secret,
            protocol::CodeExchangeInput {
                code,
                redirect: &callback,
                verifier: flow.verifier.as_deref(),
                nonce: flow.nonce.as_deref(),
                response_issuer: issuer,
            },
        )
        .await?
        {
            ExchangeOutcome::Identity(proof) => {
                Ok(ExchangeResult::Verified(Box::new(FederatedExchange {
                    flow,
                    proof,
                })))
            }
            ExchangeOutcome::Denied(message) => Ok(ExchangeResult::Denied(message)),
        }
    }

    pub(crate) async fn verify_federated_exchange(
        &self,
        connection: &mut PgConnection,
        exchange: &FederatedExchange,
    ) -> Result<()> {
        let provider = self
            .federated_provider_in(connection, exchange.proof.provider_id())
            .await?;
        if provider.revision != exchange.proof.revision() || provider.kind != exchange.proof.kind()
        {
            return Err(IdentityError::Unauthorized);
        }
        Ok(())
    }

    pub(crate) async fn federated_user(
        &self,
        connection: &mut PgConnection,
        exchange: &FederatedExchange,
        source: Option<crate::abuse::registration::RegistrationSource>,
    ) -> Result<i64> {
        auth::lock_registration(connection).await?;
        self.verify_federated_exchange(connection, exchange).await?;
        let mapped = exchange.proof.user();
        let row = sqlx::query("SELECT subject,user_id,revoked_at IS NULL AS active FROM core_federated_auth.identities WHERE provider_id=$1 AND subject_digest=$2 FOR SHARE")
            .bind(exchange.provider_id()).bind(Sha256::digest(mapped.subject.as_bytes()).as_slice()).fetch_optional(&mut *connection).await?;
        if let Some(row) = row {
            if row.try_get::<String, _>("subject")? != mapped.subject
                || !row.try_get::<bool, _>("active")?
            {
                return Err(IdentityError::Unauthorized);
            }
            return Ok(row.try_get("user_id")?);
        }
        if !exchange.flow.accepted_legal {
            return Err(IdentityError::Forbidden);
        }
        let method = if exchange.kind() == ProviderKind::CustomOauth {
            format!("custom:{}", exchange.flow.provider.slug)
        } else {
            exchange.kind().as_str().to_owned()
        };
        self.require_oauth_registration(&method)?;
        self.require_registration_email(if mapped.email_verified {
            Some(&mapped.email)
        } else {
            None
        })?;
        let user = auth::create_user(connection).await?;
        // go-v0.2.118: prefer the mapped username only when it fits the
        // byte-based 20-byte limit and has never been occupied (including
        // disabled/imported users). The generated name remains provider-scoped.
        let occupied: bool = sqlx::query_scalar(
            "SELECT EXISTS(SELECT FROM core_identity.user_profiles WHERE username=$1)",
        )
        .bind(&mapped.username)
        .fetch_one(&mut *connection)
        .await?;
        let (username, display) = registration_names(
            &exchange.flow.provider.slug,
            &exchange.flow.provider.name,
            user,
            &mapped.username,
            &mapped.display_name,
            occupied,
        );
        let email = mapped.email.trim().to_lowercase();
        if !email.is_empty() {
            let occupied: bool = sqlx::query_scalar("SELECT EXISTS(SELECT FROM core_identity.user_profiles WHERE lower(btrim(email))=$1)")
                .bind(&email).fetch_one(&mut *connection).await?;
            if occupied {
                return Err(IdentityError::Conflict);
            }
        }
        // Profile text cannot choose an owner or a Core role. Verified email is
        // a registration attribute, never an account-link lookup key.
        sqlx::query("INSERT INTO core_identity.user_profiles(user_id,username,display_name,password_login_allowed,email) VALUES($1,$2,$3,FALSE,$4)")
            .bind(user).bind(&username).bind(&display).bind(&email).execute(&mut *connection).await?;
        sqlx::query("INSERT INTO core_federated_auth.identities(provider_id,subject,subject_digest,user_id) VALUES($1,$2,$3,$4)")
            .bind(exchange.provider_id()).bind(&mapped.subject).bind(Sha256::digest(mapped.subject.as_bytes()).as_slice()).bind(user).execute(&mut *connection).await?;
        crate::abuse::registration::record(connection, user, source)
            .await
            .map_err(|_| IdentityError::Storage)?;
        // The consumed flow retains the affiliate code for the existing
        // acquisition writer; no arbitrary Core capability comes from it.
        let _affiliate = &exchange.flow.affiliate;
        Ok(user)
    }
}

fn registration_names(
    slug: &str,
    provider_name: &str,
    user_id: i64,
    mapped_username: &str,
    mapped_display: &str,
    occupied: bool,
) -> (String, String) {
    let username = if !mapped_username.is_empty() && mapped_username.len() <= 20 && !occupied {
        mapped_username.to_owned()
    } else {
        format!("{slug}_{user_id}")
    };
    let display = if !mapped_display.is_empty() {
        mapped_display.to_owned()
    } else if !mapped_username.is_empty() {
        mapped_username.to_owned()
    } else {
        format!("{provider_name} User")
    };
    (username, display)
}

#[cfg(test)]
mod registration_tests {
    use super::registration_names;

    #[test]
    fn legacy_registration_names_preserve_preference_and_display_fallbacks() {
        for (name, display, occupied, expected_name, expected_display) in [
            ("alice", "Alice", false, "alice", "Alice"),
            ("alice", "", true, "example_42", "alice"),
            (
                "123456789012345678901",
                "",
                false,
                "example_42",
                "123456789012345678901",
            ),
            ("你好你好你好你", "", false, "example_42", "你好你好你好你"),
            ("你好你好你好", "", false, "你好你好你好", "你好你好你好"),
            ("", "", false, "example_42", "Example User"),
        ] {
            assert_eq!(
                registration_names("example", "Example", 42, name, display, occupied),
                (expected_name.to_owned(), expected_display.to_owned())
            );
        }
    }
}

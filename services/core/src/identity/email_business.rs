//! Identity mutations on the real email proof transaction.
//!
//! These functions deliberately return an uncommitted transaction. Composition
//! must enqueue the security notice on that connection before calling
//! `EmailMutation::finish`. No HTTP route or mail delivery is enabled here.
use super::{
    IdentityError, IdentityStore, auth,
    email::{EmailAddress, EmailChallenges, EmailError, EmailMutation, Request},
    new_secret,
};
use sqlx::Row;
use std::fmt;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum EmailBusinessError {
    Identity(IdentityError),
    Proof(EmailError),
}

impl fmt::Display for EmailBusinessError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{self:?}")
    }
}

impl std::error::Error for EmailBusinessError {}

impl From<IdentityError> for EmailBusinessError {
    fn from(error: IdentityError) -> Self {
        Self::Identity(error)
    }
}

impl From<EmailError> for EmailBusinessError {
    fn from(error: EmailError) -> Self {
        Self::Proof(error)
    }
}

impl From<sqlx::Error> for EmailBusinessError {
    fn from(error: sqlx::Error) -> Self {
        Self::Identity(auth::password_change_error(error))
    }
}

type Result<T> = std::result::Result<T, EmailBusinessError>;

/// No Debug or serialization: `code` is a delivery secret.
pub struct EmailConfirmation<'a> {
    pub email: &'a EmailAddress,
    pub code: &'a str,
}

pub struct EmailRegistration<'a> {
    pub username: &'a str,
    pub password: &'a str,
    pub confirmation: EmailConfirmation<'a>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum EmailSecurityChange {
    Registered,
    Bound,
    PasswordReset,
}

/// Intent for the future durable notification writer, not a queued/sent fact.
/// There is no password or proof secret in this value.
pub struct EmailSecurityNotice {
    pub change: EmailSecurityChange,
    pub user_id: i64,
    pub challenge_id: String,
    pub email: EmailAddress,
    pub previous_verified_email: Option<EmailAddress>,
}

#[must_use = "the open email transaction must be completed or dropped to roll back"]
pub struct PendingEmailBusiness {
    mutation: EmailMutation,
    notice: EmailSecurityNotice,
}

impl PendingEmailBusiness {
    /// Enqueue the notice through `mutation.connection()`, then call
    /// `mutation.finish(user_id)`. Dropping any part before finish rolls back.
    /// A notice value alone is not evidence of a durable outbox write.
    pub fn into_parts(self) -> (EmailMutation, EmailSecurityNotice) {
        (self.mutation, self.notice)
    }
}

fn web_password_policy(password: &str) -> Result<()> {
    // Match BrowserAuth::register/change_password, including Unicode scalars.
    if !(8..=20).contains(&password.chars().count()) || password.len() > 1024 {
        return Err(IdentityError::Invalid.into());
    }
    Ok(())
}

fn pending(
    mutation: EmailMutation,
    confirmation: &EmailConfirmation<'_>,
    change: EmailSecurityChange,
    user_id: i64,
    previous_verified_email: Option<EmailAddress>,
) -> PendingEmailBusiness {
    let challenge_id = mutation.challenge_id().to_owned();
    PendingEmailBusiness {
        mutation,
        notice: EmailSecurityNotice {
            change,
            user_id,
            challenge_id,
            email: confirmation.email.clone(),
            previous_verified_email,
        },
    }
}

impl IdentityStore {
    /// Prepare a browser registration without issuing any session or authority
    /// from an unverified legacy profile. HTTP still owns legal/abuse gates.
    pub async fn register_with_email(
        &self,
        challenges: &EmailChallenges,
        input: &EmailRegistration<'_>,
    ) -> Result<PendingEmailBusiness> {
        self.require_password_registration()?;
        self.require_registration_email(Some(input.confirmation.email.as_str()))?;
        let username = input.username.trim();
        if username.is_empty() || username.chars().count() > 20 || username.len() > 128 {
            return Err(IdentityError::Invalid.into());
        }
        web_password_policy(input.password)?;
        let hash = self.hash_password(input.password.to_owned()).await?;
        let mut mutation = challenges
            .verify_current(
                Request::Register,
                input.confirmation.email,
                input.confirmation.code,
            )
            .await?;
        let c = mutation.connection();
        // Reuse registration serialization, personal account and billing defaults.
        let user = auth::create_user(c).await?;
        // Proof verification precedes the registration lock. Recheck namespace
        // occupancy after acquiring it, including a concurrent registration.
        let occupied: bool = sqlx::query_scalar(
            "SELECT EXISTS(SELECT 1 FROM core_identity.user_profiles
                 WHERE username=$1 OR (email_login_allowed AND email<>'' AND email=$1)
                     OR lower(username)=$2 OR lower(btrim(email))=$2)
             OR EXISTS(SELECT 1 FROM core_identity.email_addresses
                 WHERE email_key=$2 OR email_key=$3)",
        )
        .bind(username)
        .bind(input.confirmation.email.as_str())
        .bind(username.to_ascii_lowercase())
        .fetch_one(&mut *c)
        .await?;
        if occupied {
            return Err(IdentityError::Conflict.into());
        }
        let canonical = format!("web_{}", &new_secret("")?[..32]);
        sqlx::query(
            "INSERT INTO core_identity.password_logins(login_name,user_id,password_hash)
             VALUES ($1,$2,$3)",
        )
        .bind(canonical)
        .bind(user)
        .bind(hash)
        .execute(&mut *c)
        .await?;
        sqlx::query(
            "INSERT INTO core_identity.user_profiles
                 (user_id,username,display_name,email,email_login_allowed,email_version)
             VALUES ($1,$2,$2,$3,TRUE,1)",
        )
        .bind(user)
        .bind(username)
        .bind(input.confirmation.email.as_str())
        .execute(&mut *c)
        .await?;
        sqlx::query(
            "INSERT INTO core_identity.email_addresses
                 (email_key,user_id,state,email_version,verified_at)
             VALUES ($1,$2,'verified',1,clock_timestamp())",
        )
        .bind(input.confirmation.email.as_str())
        .bind(user)
        .execute(&mut *c)
        .await?;
        sqlx::query(
            "INSERT INTO core_identity.audit(actor_user_id,action,target)
             VALUES ($1,'user.register',jsonb_build_object('user_id',$1::bigint))",
        )
        .bind(user)
        .execute(c)
        .await?;
        Ok(pending(
            mutation,
            &input.confirmation,
            EmailSecurityChange::Registered,
            user,
            None,
        ))
    }

    /// Requires the current browser bearer, stable family and recent primary/MFA
    /// evidence at both issuance and verification, enforced by EmailChallenges.
    pub async fn bind_verified_email(
        &self,
        challenges: &EmailChallenges,
        access: &str,
        sid: &str,
        confirmation: &EmailConfirmation<'_>,
    ) -> Result<PendingEmailBusiness> {
        let mut mutation = challenges
            .verify_current(
                Request::Bind { access, sid },
                confirmation.email,
                confirmation.code,
            )
            .await?;
        let challenge_id = mutation.challenge_id().to_owned();
        let c = mutation.connection();
        // Verification holds shared authority locks. Never wait to upgrade one:
        // refresh/MFA/profile writers have different existing lock orders.
        let row = sqlx::query(
            "SELECT pr.user_id,pr.email_version FROM core_identity.user_profiles pr
             JOIN core_identity.users u ON u.id=pr.user_id
             JOIN core_identity.email_challenges ch ON ch.user_id=u.id
             WHERE ch.id=$1 AND ch.purpose='bind' AND u.auth_version=ch.user_version
                 AND pr.email_version=ch.email_version
             FOR UPDATE OF pr NOWAIT",
        )
        .bind(&challenge_id)
        .fetch_optional(&mut *c)
        .await?
        .ok_or(EmailError::InvalidProof)?;
        let user: i64 = row.try_get("user_id")?;
        let version: i64 = row.try_get("email_version")?;
        let previous = sqlx::query(
            "SELECT email_key,state,email_version FROM core_identity.email_addresses
             WHERE user_id=$1 FOR UPDATE NOWAIT",
        )
        .bind(user)
        .fetch_optional(&mut *c)
        .await?;
        let previous_verified_email = match previous {
            Some(row)
                if row.try_get::<String, _>("state")? == "verified"
                    && row.try_get::<i64, _>("email_version")? == version
                    && row.try_get::<String, _>("email_key")? != confirmation.email.as_str() =>
            {
                Some(EmailAddress::parse(
                    &row.try_get::<String, _>("email_key")?,
                )?)
            }
            _ => None,
        };
        let next: i64 = sqlx::query_scalar(
            "UPDATE core_identity.user_profiles SET email=$2,email_login_allowed=TRUE,
                 email_version=email_version+1 WHERE user_id=$1 RETURNING email_version",
        )
        .bind(user)
        .bind(confirmation.email.as_str())
        .fetch_one(&mut *c)
        .await?;
        sqlx::query(
            "INSERT INTO core_identity.email_addresses
                 (email_key,user_id,state,email_version,verified_at)
             VALUES ($1,$2,'verified',$3,clock_timestamp())
             ON CONFLICT (user_id) DO UPDATE SET email_key=EXCLUDED.email_key,
                 state='verified',email_version=EXCLUDED.email_version,
                 verified_at=EXCLUDED.verified_at",
        )
        .bind(confirmation.email.as_str())
        .bind(user)
        .bind(next)
        .execute(&mut *c)
        .await?;
        sqlx::query(
            "INSERT INTO core_identity.audit(actor_user_id,action,target)
             VALUES ($1,'identity.email_bound',
                 jsonb_build_object('challenge_id',$2::text,'email_version',$3::bigint))",
        )
        .bind(user)
        .bind(&challenge_id)
        .bind(next)
        .execute(c)
        .await?;
        Ok(pending(
            mutation,
            confirmation,
            EmailSecurityChange::Bound,
            user,
            previous_verified_email,
        ))
    }

    /// Browser password policy, existing password method only. Completion never
    /// issues a session, disables MFA, changes role or activates an account.
    pub async fn reset_password_with_email(
        &self,
        challenges: &EmailChallenges,
        confirmation: &EmailConfirmation<'_>,
        new_password: &str,
    ) -> Result<PendingEmailBusiness> {
        web_password_policy(new_password)?;
        // Use the existing bounded Argon2 worker before taking proof/identity locks.
        let hash = self.hash_password(new_password.to_owned()).await?;
        let mut mutation = challenges
            .verify_current(
                Request::PasswordReset,
                confirmation.email,
                confirmation.code,
            )
            .await?;
        let challenge_id = mutation.challenge_id().to_owned();
        let c = mutation.connection();
        let row = sqlx::query(
            "SELECT u.id,u.auth_version FROM core_identity.users u
             JOIN core_identity.accounts a ON a.id=u.personal_account_id
             JOIN core_identity.user_profiles pr ON pr.user_id=u.id
             JOIN core_identity.password_logins p ON p.user_id=u.id
             JOIN core_identity.email_addresses e ON e.user_id=u.id
             JOIN core_identity.email_challenges ch ON ch.user_id=u.id
             WHERE ch.id=$1 AND ch.purpose='password_reset'
                 AND e.email_key=$2 AND e.state='verified'
                 AND e.email_version=pr.email_version AND pr.email_version=ch.email_version
                 AND u.auth_version=ch.user_version AND u.active AND a.active
                 AND pr.password_login_allowed
             FOR UPDATE OF u,p,pr NOWAIT FOR SHARE OF a,e NOWAIT",
        )
        .bind(&challenge_id)
        .bind(confirmation.email.as_str())
        .fetch_optional(&mut *c)
        .await?
        .ok_or(EmailError::InvalidProof)?;
        let user: i64 = row.try_get("id")?;
        let version: i64 = row.try_get("auth_version")?;
        sqlx::query("UPDATE core_identity.password_logins SET password_hash=$2 WHERE user_id=$1")
            .bind(user)
            .bind(hash)
            .execute(&mut *c)
            .await?;
        sqlx::query(
            "UPDATE core_identity.user_profiles SET legacy_bcrypt_allowed=FALSE WHERE user_id=$1",
        )
        .bind(user)
        .execute(&mut *c)
        .await?;
        // Core session/API-key authorization, Web refresh, OAuth and pending MFA
        // all compare this generation. No old family is rebound to the new one.
        let changed = sqlx::query(
            "UPDATE core_identity.users SET auth_version=auth_version+1
             WHERE id=$1 AND auth_version=$2",
        )
        .bind(user)
        .bind(version)
        .execute(&mut *c)
        .await?
        .rows_affected();
        if changed != 1 {
            return Err(IdentityError::Conflict.into());
        }
        sqlx::query(
            "INSERT INTO core_identity.audit(actor_user_id,action,target)
             VALUES ($1,'identity.email_password_reset',
                 jsonb_build_object('challenge_id',$2::text,'user_version',$3::bigint))",
        )
        .bind(user)
        .bind(&challenge_id)
        .bind(version + 1)
        .execute(c)
        .await?;
        Ok(pending(
            mutation,
            confirmation,
            EmailSecurityChange::PasswordReset,
            user,
            None,
        ))
    }
}

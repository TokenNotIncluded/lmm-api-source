//! Email business requests composed with encrypted durable delivery.
use crate::{
    email_delivery::Outbox,
    identity::email_notice,
    identity::{
        IdentityError, IdentityStore,
        email::{EmailAddress, EmailChallenges, EmailError, Request},
        email_business::{EmailBusinessError, EmailConfirmation, EmailRegistration},
    },
};
use axum::{
    Json,
    http::{HeaderMap, StatusCode, header},
    response::{IntoResponse, Response},
};
use serde::Deserialize;
use serde_json::{Value, json};

/// Explicit composition of already configured stores, without SMTP or startup.
#[derive(Clone)]
pub struct EmailHttp {
    store: IdentityStore,
    challenges: EmailChallenges,
    outbox: Outbox,
    turnstile: Option<crate::config::turnstile_config::Turnstile>,
}

#[derive(Clone, Copy)]
pub(crate) enum Failure {
    Invalid,
    Proof,
    Unauthorized,
    Forbidden,
    Unavailable,
}

type Result<T> = std::result::Result<T, Failure>;

#[derive(Deserialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum VerificationPurpose {
    Register,
    Bind,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct VerificationRequest {
    email: String,
    purpose: VerificationPurpose,
    turnstile: Option<String>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct ResetRequest {
    email: String,
    turnstile: Option<String>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Registration {
    pub(crate) username: String,
    pub(crate) password: String,
    pub(crate) email: String,
    pub(crate) verification_code: String,
    #[serde(default)]
    pub(crate) aff_code: String,
    pub(crate) accepted_legal: Option<bool>,
    #[serde(default)]
    pub(crate) turnstile: String,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Binding {
    email: String,
    code: String,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Reset {
    email: String,
    token: String,
    new_password: String,
}

fn address(raw: &str) -> Result<EmailAddress> {
    EmailAddress::parse(raw).map_err(|_| Failure::Invalid)
}

fn identity_error(error: IdentityError) -> Failure {
    match error {
        IdentityError::Unauthorized => Failure::Unauthorized,
        IdentityError::Forbidden => Failure::Forbidden,
        IdentityError::Storage => Failure::Unavailable,
        IdentityError::Invalid | IdentityError::Conflict => Failure::Proof,
    }
}

fn business_error(error: EmailBusinessError) -> Failure {
    match error {
        EmailBusinessError::Identity(IdentityError::Storage)
        | EmailBusinessError::Proof(EmailError::Unavailable) => Failure::Unavailable,
        _ => Failure::Proof,
    }
}

fn completion_error(error: email_notice::CompletionError) -> Failure {
    match error {
        email_notice::CompletionError::Proof(EmailError::Unavailable)
        | email_notice::CompletionError::Mail(_) => Failure::Unavailable,
        email_notice::CompletionError::Proof(_) => Failure::Proof,
    }
}

fn one_header<'a>(headers: &'a HeaderMap, name: &str) -> Result<&'a str> {
    let mut values = headers.get_all(name).iter();
    let value = values.next().ok_or(Failure::Unauthorized)?;
    if values.next().is_some() {
        return Err(Failure::Unauthorized);
    }
    value.to_str().map_err(|_| Failure::Unauthorized)
}

fn credentials(headers: &HeaderMap) -> Result<(&str, &str)> {
    let authorization = one_header(headers, "authorization")?;
    let mut parts = authorization.split_whitespace();
    let scheme = parts.next().ok_or(Failure::Unauthorized)?;
    let access = parts.next().ok_or(Failure::Unauthorized)?;
    let sid = one_header(headers, "x-auth-session")?;
    if !scheme.eq_ignore_ascii_case("bearer")
        || parts.next().is_some()
        || access.len() > 256
        || sid.len() != 64
        || !sid
            .bytes()
            .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
    {
        return Err(Failure::Unauthorized);
    }
    Ok((access, sid))
}

impl EmailHttp {
    pub(crate) fn with_turnstile(
        mut self,
        turnstile: Option<crate::config::turnstile_config::Turnstile>,
    ) -> Self {
        self.turnstile = turnstile;
        self
    }

    async fn verify_challenge(&self, token: Option<&str>) -> Result<()> {
        crate::config::turnstile_config::verify(self.turnstile.as_ref(), token.unwrap_or_default())
            .await
            .map_err(|error| match error {
                crate::config::turnstile_config::VerifyError::Rejected => Failure::Forbidden,
                crate::config::turnstile_config::VerifyError::Unavailable => Failure::Unavailable,
            })
    }
    pub(crate) fn with_identity_store(mut self, store: IdentityStore) -> Self {
        self.store = store;
        self
    }

    /// Caller owns schema/ACL/readiness and independent secret configuration.
    pub fn new(store: IdentityStore, challenges: EmailChallenges, outbox: Outbox) -> Self {
        Self {
            store,
            challenges,
            outbox,
            turnstile: None,
        }
    }

    pub(crate) async fn request_verification(
        &self,
        headers: &HeaderMap,
        body: VerificationRequest,
    ) -> Result<()> {
        let email = address(&body.email)?;
        self.verify_challenge(body.turnstile.as_deref()).await?;
        let request = match body.purpose {
            VerificationPurpose::Register => {
                if !self.store.email_registration_allowed(&email) {
                    return Ok(());
                }
                Request::Register
            }
            VerificationPurpose::Bind => {
                let (access, sid) = credentials(headers)?;
                self.store
                    .authorize_email_session(access, sid)
                    .await
                    .map_err(identity_error)?;
                Request::Bind { access, sid }
            }
        };
        // A per-target enqueue failure must not reveal that this address exists.
        // "accepted_if_eligible" is deliberately not a durable-queue receipt.
        let _ = self.outbox.issue(&self.challenges, request, &email).await;
        Ok(())
    }

    pub(crate) async fn request_reset(&self, body: ResetRequest) -> Result<()> {
        let email = address(&body.email)?;
        self.verify_challenge(body.turnstile.as_deref()).await?;
        let result = self
            .outbox
            .issue(&self.challenges, Request::PasswordReset, &email)
            .await;
        if let Some(event) = reset_enqueue_failure(result) {
            eprintln!("{event}");
        }
        Ok(())
    }

    pub(crate) async fn register(&self, body: &Registration, legal_required: bool) -> Result<()> {
        if legal_required && body.accepted_legal != Some(true) {
            return Err(Failure::Invalid);
        }
        self.verify_challenge(Some(&body.turnstile)).await?;
        let email = address(&body.email)?;
        let pending = self
            .store
            .register_with_email_and_affiliate(
                &self.challenges,
                &EmailRegistration {
                    username: &body.username,
                    password: &body.password,
                    confirmation: EmailConfirmation {
                        email: &email,
                        code: &body.verification_code,
                    },
                },
                &body.aff_code,
            )
            .await
            .map_err(business_error)?;
        email_notice::finish(pending, &self.outbox)
            .await
            .map_err(completion_error)?;
        Ok(())
    }

    pub(crate) async fn bind(&self, headers: &HeaderMap, body: Binding) -> Result<()> {
        let (access, sid) = credentials(headers)?;
        self.store
            .authorize_email_session(access, sid)
            .await
            .map_err(identity_error)?;
        let email = address(&body.email)?;
        let pending = self
            .store
            .bind_verified_email(
                &self.challenges,
                access,
                sid,
                &EmailConfirmation {
                    email: &email,
                    code: &body.code,
                },
            )
            .await
            .map_err(business_error)?;
        email_notice::finish(pending, &self.outbox)
            .await
            .map_err(completion_error)?;
        Ok(())
    }

    pub(crate) async fn reset(&self, body: Reset) -> Result<()> {
        let email = address(&body.email)?;
        let pending = self
            .store
            .reset_password_with_email(
                &self.challenges,
                &EmailConfirmation {
                    email: &email,
                    code: &body.token,
                },
                &body.new_password,
            )
            .await
            .map_err(business_error)?;
        email_notice::finish(pending, &self.outbox)
            .await
            .map_err(completion_error)?;
        Ok(())
    }
}

// Only fixed operational classes reach stderr. Successful and silently suppressed
// issuance both return Ok(()), and the public reply is identical for every outcome.
fn reset_enqueue_failure(
    result: std::result::Result<(), crate::email_delivery::MailError>,
) -> Option<&'static str> {
    use crate::email_delivery::MailError;
    match result {
        Ok(()) => None,
        Err(MailError::Invalid) => Some("core_password_reset_enqueue_failed class=invalid"),
        Err(MailError::Conflict) => Some("core_password_reset_enqueue_failed class=conflict"),
        Err(MailError::Unavailable) => Some("core_password_reset_enqueue_failed class=unavailable"),
    }
}

pub(crate) fn failure(error: Failure) -> Response {
    let (status, code) = match error {
        Failure::Invalid => (StatusCode::BAD_REQUEST, "EMAIL_INVALID_REQUEST"),
        Failure::Proof => (StatusCode::BAD_REQUEST, "EMAIL_PROOF_INVALID"),
        Failure::Unauthorized => (StatusCode::UNAUTHORIZED, "AUTH_UNAUTHORIZED"),
        Failure::Forbidden => (StatusCode::FORBIDDEN, "AUTH_FORBIDDEN"),
        Failure::Unavailable => (StatusCode::SERVICE_UNAVAILABLE, "EMAIL_UNAVAILABLE"),
    };
    let mut response = (
        status,
        Json(
            json!({"success":false,"message":"Unable to complete email request",
                    "error":{"code":code}}),
        ),
    )
        .into_response();
    if status == StatusCode::UNAUTHORIZED {
        response.headers_mut().insert(
            header::WWW_AUTHENTICATE,
            "Bearer".parse().expect("static header"),
        );
    }
    response
}

pub(crate) fn response(result: Result<()>, status: StatusCode, data: Value) -> Response {
    match result {
        Ok(()) => (
            status,
            Json(json!({"success":true,"message":"","data":data})),
        )
            .into_response(),
        Err(error) => failure(error),
    }
}

pub(crate) fn accepted(result: Result<()>) -> Response {
    response(
        result,
        StatusCode::ACCEPTED,
        json!({"status":"accepted_if_eligible"}),
    )
}

#[cfg(test)]
mod reset_enqueue_tests {
    use super::reset_enqueue_failure;
    use crate::email_delivery::MailError;

    #[test]
    fn reset_enqueue_logs_only_static_failure_classes() {
        assert_eq!(reset_enqueue_failure(Ok(())), None);
        for (error, expected) in [
            (
                MailError::Invalid,
                "core_password_reset_enqueue_failed class=invalid",
            ),
            (
                MailError::Conflict,
                "core_password_reset_enqueue_failed class=conflict",
            ),
            (
                MailError::Unavailable,
                "core_password_reset_enqueue_failed class=unavailable",
            ),
        ] {
            assert_eq!(reset_enqueue_failure(Err(error)), Some(expected));
        }
    }
}

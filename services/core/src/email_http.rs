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
}

#[derive(Clone, Copy)]
pub(crate) enum Failure {
    Invalid,
    Proof,
    Unauthorized,
    Forbidden,
    Unavailable,
    FeatureUnavailable,
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

fn unsupported_challenge(turnstile: Option<&str>) -> Result<()> {
    if turnstile.is_some_and(|value| !value.is_empty()) {
        return Err(Failure::FeatureUnavailable);
    }
    Ok(())
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
        }
    }

    pub(crate) async fn request_verification(
        &self,
        headers: &HeaderMap,
        body: VerificationRequest,
    ) -> Result<()> {
        let email = address(&body.email)?;
        unsupported_challenge(body.turnstile.as_deref())?;
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
        unsupported_challenge(body.turnstile.as_deref())?;
        let _ = self
            .outbox
            .issue(&self.challenges, Request::PasswordReset, &email)
            .await;
        Ok(())
    }

    pub(crate) async fn register(&self, body: &Registration, legal_required: bool) -> Result<()> {
        if legal_required && body.accepted_legal != Some(true) {
            return Err(Failure::Invalid);
        }
        unsupported_challenge(Some(&body.turnstile))?;
        if !body.aff_code.is_empty() {
            return Err(Failure::FeatureUnavailable);
        }
        let email = address(&body.email)?;
        let pending = self
            .store
            .register_with_email(
                &self.challenges,
                &EmailRegistration {
                    username: &body.username,
                    password: &body.password,
                    confirmation: EmailConfirmation {
                        email: &email,
                        code: &body.verification_code,
                    },
                },
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

pub(crate) fn failure(error: Failure) -> Response {
    let (status, code) = match error {
        Failure::Invalid => (StatusCode::BAD_REQUEST, "EMAIL_INVALID_REQUEST"),
        Failure::Proof => (StatusCode::BAD_REQUEST, "EMAIL_PROOF_INVALID"),
        Failure::Unauthorized => (StatusCode::UNAUTHORIZED, "AUTH_UNAUTHORIZED"),
        Failure::Forbidden => (StatusCode::FORBIDDEN, "AUTH_FORBIDDEN"),
        Failure::Unavailable => (StatusCode::SERVICE_UNAVAILABLE, "EMAIL_UNAVAILABLE"),
        Failure::FeatureUnavailable => (
            StatusCode::SERVICE_UNAVAILABLE,
            "AUTH_REGISTRATION_FEATURE_UNAVAILABLE",
        ),
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

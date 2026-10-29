//! Atomic email business completion and encrypted security notices.
use crate::{
    email_delivery::{MailError, NoticeRecipient, Outbox},
    identity::{
        email::EmailError,
        email_business::{EmailSecurityChange, PendingEmailBusiness},
    },
};
use std::fmt;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum CompletionError {
    Proof(EmailError),
    Mail(MailError),
}

impl fmt::Display for CompletionError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{self:?}")
    }
}

impl std::error::Error for CompletionError {}

impl From<EmailError> for CompletionError {
    fn from(error: EmailError) -> Self {
        Self::Proof(error)
    }
}

impl From<MailError> for CompletionError {
    fn from(error: MailError) -> Self {
        Self::Mail(error)
    }
}

/// Commits business changes, consumption and required encrypted notices together.
///
/// Registration retains the existing policy: the proof email is required, but
/// no additional registration security notice is defined by the current outbox.
/// Binding notifies the current and, if different and verified, previous email.
/// Reset notifies the current verified email. Success means durable enqueue,
/// not SMTP acceptance or inbox delivery. Any error drops the whole transaction.
pub async fn finish(
    pending: PendingEmailBusiness,
    outbox: &Outbox,
) -> Result<i64, CompletionError> {
    let (mutation, notice) = pending.into_parts();
    let mut consumed = mutation.consume(notice.user_id).await?;
    let challenge = consumed.challenge_id().to_owned();
    if notice.change != EmailSecurityChange::Registered {
        outbox
            .enqueue_notice_slot_in(
                consumed.transaction(),
                &challenge,
                notice.email.as_str(),
                NoticeRecipient::Current,
            )
            .await?;
        if notice.change == EmailSecurityChange::Bound
            && let Some(previous) = notice.previous_verified_email
        {
            outbox
                .enqueue_notice_slot_in(
                    consumed.transaction(),
                    &challenge,
                    previous.as_str(),
                    NoticeRecipient::Previous,
                )
                .await?;
        }
    }
    consumed.commit().await?;
    Ok(notice.user_id)
}

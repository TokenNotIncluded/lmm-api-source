//! Session preflight; proof issuance/consumption rechecks authority under locks.
use super::{AccountKind, CredentialKind, IdentityError, IdentityStore, email::EmailAddress};

impl IdentityStore {
    pub(crate) fn email_registration_allowed(&self, email: &EmailAddress) -> bool {
        self.require_password_registration().is_ok()
            && self
                .require_registration_email(Some(email.as_str()))
                .is_ok()
    }

    pub(crate) async fn authorize_email_session(
        &self,
        access: &str,
        sid: &str,
    ) -> Result<(), IdentityError> {
        let mut tx = self.pool.begin().await?;
        sqlx::query("SET LOCAL lock_timeout='1s'")
            .execute(&mut *tx)
            .await?;
        sqlx::query("SET LOCAL statement_timeout='5s'")
            .execute(&mut *tx)
            .await?;
        let actor = super::session(&mut tx, access).await?;
        if actor.credential_kind != CredentialKind::Session
            || actor.owner.kind != AccountKind::Personal
            || actor.owner.id != actor.user_id
        {
            return Err(IdentityError::Forbidden);
        }
        let live: bool = sqlx::query_scalar(
            "SELECT EXISTS(SELECT 1 FROM core_identity.web_sessions
             WHERE sid=$1 AND current_credential_id=$2 AND user_id=$3 AND user_version=$4
                 AND revoked_at IS NULL AND expires_at>clock_timestamp()
                 AND (NOT weekly_logout OR created_at>clock_timestamp()-interval '7 days'))",
        )
        .bind(sid)
        .bind(actor.credential_id)
        .bind(actor.user_id)
        .bind(actor.user_version)
        .fetch_one(&mut *tx)
        .await?;
        if !live {
            return Err(IdentityError::Unauthorized);
        }
        tx.commit().await?;
        Ok(())
    }
}

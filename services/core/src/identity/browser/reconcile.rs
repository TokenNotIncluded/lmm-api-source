//! Read-only recovery of an existing browser access credential, never refresh.
//! Client writer/document isolation is a separate prerequisite for installation.
use super::*;
use serde::Serialize;

const PROTOCOL: &str = "core.browser-session-reconciliation.v1";

#[derive(Clone, Copy, Serialize)]
#[serde(rename_all = "snake_case")]
enum Status {
    Current,
    Previous,
    ExpiredPrevious,
    UnknownSid,
    UnknownCookie,
    NoCookie,
    SidMismatch,
    FamilyInactive,
    CurrentAccessExpired,
    AuthorityUnavailable,
}

fn reply(status: Status, bundle: Option<Value>) -> Value {
    let mut value = json!({"protocol": PROTOCOL, "status": status});
    if let Some(bundle) = bundle.filter(|_| matches!(status, Status::Current)) {
        value["bundle"] = bundle;
    }
    value
}

impl BrowserAuth {
    /// Returns no refresh secret. Every database operation is in a read-only
    /// snapshot; even invalid or expired previous cookies cannot revoke a family.
    pub async fn reconcile_session(
        &self,
        cookie: Option<&str>,
        expected_sid: Option<&str>,
    ) -> BrowserResult<Value> {
        if expected_sid.is_some_and(|sid| {
            sid.len() != 64
                || !sid
                    .bytes()
                    .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
        }) {
            return Err(IdentityError::Invalid.into());
        }
        let Some(cookie) = cookie else {
            return Ok(reply(Status::NoCookie, None));
        };
        let Ok((sid, secret)) = parse_refresh(cookie) else {
            return Ok(reply(Status::UnknownCookie, None));
        };
        if expected_sid.is_some_and(|expected| expected != sid) {
            return Ok(reply(Status::SidMismatch, None));
        }
        let mut tx = self.store.pool.begin().await?;
        sqlx::query("SET TRANSACTION ISOLATION LEVEL REPEATABLE READ READ ONLY")
            .execute(&mut *tx)
            .await?;
        let result = self.reconcile_snapshot(&mut tx, sid, secret).await?;
        tx.commit().await?;
        Ok(result)
    }

    async fn reconcile_snapshot(
        &self,
        c: &mut PgConnection,
        sid: &str,
        secret: &str,
    ) -> BrowserResult<Value> {
        let row = sqlx::query(
            "SELECT s.*,c.id AS selected_credential_id,c.kind AS credential_kind,
                c.digest AS credential_digest,c.user_id AS credential_user_id,
                c.owner_account_id AS credential_owner_account_id,
                c.user_version AS credential_user_version,
                c.revoked_at IS NULL AS credential_not_revoked,
                EXISTS(SELECT FROM core_identity.web_session_credentials m
                       WHERE m.sid=s.sid AND m.credential_id=c.id) AS credential_family_link,
                COALESCE(c.expires_at>clock_timestamp(),FALSE) AS access_live,
                u.personal_account_id,u.auth_version AS current_user_version,
                COALESCE(s.previous_valid_until>=clock_timestamp(),FALSE) AS previous_live,
                (s.revoked_at IS NULL AND s.expires_at>clock_timestamp()
                 AND (NOT s.weekly_logout OR s.created_at>clock_timestamp()-interval '7 days')
                 AND u.active AND a.active AND s.user_version=u.auth_version) AS family_live
             FROM core_identity.web_sessions s
             JOIN core_identity.users u ON u.id=s.user_id
             JOIN core_identity.accounts a ON a.id=u.personal_account_id
             LEFT JOIN core_identity.credentials c ON c.id=s.current_credential_id
             WHERE s.sid=$1",
        )
        .bind(sid)
        .fetch_optional(&mut *c)
        .await?;
        let Some(row) = row else {
            return Ok(reply(Status::UnknownSid, None));
        };
        let hash = digest(secret)?;
        let current: Vec<u8> = row.try_get("refresh_digest")?;
        let previous: Option<Vec<u8>> = row.try_get("previous_refresh_digest")?;
        let is_current = constant_digest(&current, &hash);
        let is_previous = previous.as_ref().is_some_and(|p| constant_digest(p, &hash));
        if !is_current && !is_previous {
            return Ok(reply(Status::UnknownCookie, None));
        }
        if !row.try_get::<bool, _>("family_live")? {
            return Ok(reply(Status::FamilyInactive, None));
        }
        // A predecessor never derives the next secret, even during its grace
        // window. Its classification is not permission to call normal refresh.
        if !is_current {
            return Ok(reply(
                if row.try_get::<bool, _>("previous_live")? {
                    Status::Previous
                } else {
                    Status::ExpiredPrevious
                },
                None,
            ));
        }
        let user: i64 = row.try_get("user_id")?;
        network_policy::authorize_request_ip(c, user).await?;
        let version: i64 = row.try_get("user_version")?;
        let Some(credential): Option<i64> = row.try_get("selected_credential_id")? else {
            return Ok(reply(Status::AuthorityUnavailable, None));
        };
        if row.try_get::<String, _>("credential_kind")? != "session"
            || row.try_get::<i64, _>("credential_user_id")? != user
            || row.try_get::<i64, _>("credential_owner_account_id")?
                != row.try_get::<i64, _>("personal_account_id")?
            || row.try_get::<i64, _>("credential_user_version")? != version
            || !row.try_get::<bool, _>("credential_not_revoked")?
            || !row.try_get::<bool, _>("credential_family_link")?
        {
            return Ok(reply(Status::AuthorityUnavailable, None));
        }
        // This is exactly the access secret already persisted by login/refresh.
        // A refresh-cookie proof cannot mint a different credential or authority.
        let access = self.derived_secret(b"browser-access-v1", sid, secret, "lmms_")?;
        let stored_access: Vec<u8> = row.try_get("credential_digest")?;
        if !constant_digest(&stored_access, &digest(&access)?) {
            return Ok(reply(Status::AuthorityUnavailable, None));
        }
        match super::super::mfa::require_session_assurance_in_mode(
            c,
            credential,
            user,
            version,
            super::super::mfa::AssuranceRead::Snapshot,
        )
        .await
        {
            Ok(()) => {}
            Err(error @ (IdentityError::Storage | IdentityError::Conflict)) => {
                return Err(error.into());
            }
            Err(_) => return Ok(reply(Status::AuthorityUnavailable, None)),
        }
        if !row.try_get::<bool, _>("access_live")? {
            return Ok(reply(Status::CurrentAccessExpired, None));
        }
        let actor = match super::super::authenticate_readonly_in_transaction(c, &access).await {
            Ok(actor) => actor,
            Err(error @ (IdentityError::Storage | IdentityError::Conflict)) => {
                return Err(error.into());
            }
            Err(_) => return Ok(reply(Status::AuthorityUnavailable, None)),
        };
        if actor.credential_id != credential
            || actor.user_id != user
            || actor.credential_kind != CredentialKind::Session
            || actor.owner.kind != AccountKind::Personal
            || actor.owner.id != user
            || actor.user_version != version
        {
            return Ok(reply(Status::AuthorityUnavailable, None));
        }
        // make_bundle only reads; do not expose its refresh field and never call
        // the HTTP bundle_response helper, which would write browser cookies.
        let bundle = make_bundle(
            c,
            sid,
            &access,
            String::new(),
            self.store.financial_profile_enabled,
        )
        .await?;
        Ok(reply(Status::Current, Some(bundle.data)))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn classifications_never_contain_authority() {
        for status in [
            Status::Previous,
            Status::ExpiredPrevious,
            Status::UnknownSid,
            Status::UnknownCookie,
            Status::NoCookie,
            Status::SidMismatch,
            Status::FamilyInactive,
            Status::CurrentAccessExpired,
            Status::AuthorityUnavailable,
        ] {
            let value = reply(status, None);
            assert_eq!(value["protocol"], PROTOCOL);
            assert_eq!(value.as_object().unwrap().len(), 2);
            assert!(value.get("bundle").is_none());
        }
    }
}

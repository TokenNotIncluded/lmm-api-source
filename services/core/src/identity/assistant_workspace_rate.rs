//! One fixed, user-scoped browser-confirmation budget across workspace tools.
//! No model argument, IP address or bearer digest determines the bucket.
//! This transaction commits before a separate HumanGrant/action transaction.
use super::{BrowserAuth, IdentityError};
use sha2::{Digest, Sha256};
use sqlx::{PgPool, Row};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct Decision {
    pub(crate) allowed: bool,
    pub(crate) retry_after_seconds: i64,
}

fn bucket(user: i64) -> Result<[u8; 32], IdentityError> {
    if user <= 0 {
        return Err(IdentityError::Invalid);
    }
    Ok(Sha256::digest(format!("core.assistant-workspace-rate.v1\0user:{user}").as_bytes()).into())
}

// Existing auth_attempts storage, separate from all original login/OAuth buckets.
// The fixed 120/300 policy is bound to the observed source configuration. No
// runtime/model caller can choose its window or ceiling. Saturation avoids overflow.
const TAKE: &str = r#"
INSERT INTO core_identity.auth_attempts AS a(bucket,started_at,attempts)
VALUES($1,clock_timestamp(),1)
ON CONFLICT(bucket) DO UPDATE SET
 attempts=CASE WHEN a.started_at<=EXCLUDED.started_at-interval '300 seconds'
  THEN 1 ELSE least(a.attempts+1,121) END,
 started_at=CASE WHEN a.started_at<=EXCLUDED.started_at-interval '300 seconds'
  THEN EXCLUDED.started_at ELSE a.started_at END
RETURNING attempts,
 greatest(1,least(300,ceil(extract(epoch FROM
  started_at+interval '300 seconds'-clock_timestamp()))))::bigint AS retry_after_seconds
"#;

/// Authenticate the actual HttpOnly browser Session independently; an expired,
/// revoked, replaced or IP-disallowed Session cannot charge another user's bucket.
/// No confirmation, consent, domain action or financial authority is produced.
pub(crate) async fn take(pool: &PgPool, browser_cookie: &str) -> Result<Decision, IdentityError> {
    let mut tx = pool.begin().await?;
    sqlx::query("SET LOCAL synchronous_commit=on")
        .execute(&mut *tx)
        .await?;
    let actor = BrowserAuth::oauth_identity_in_transaction(&mut tx, browser_cookie).await?;
    let row = sqlx::query(TAKE)
        .bind(bucket(actor.user)?.to_vec())
        .fetch_one(&mut *tx)
        .await?;
    let attempts: i32 = row.try_get("attempts")?;
    let retry_after_seconds: i64 = row.try_get("retry_after_seconds")?;
    if !(1..=121).contains(&attempts) || !(1..=300).contains(&retry_after_seconds) {
        return Err(IdentityError::Storage);
    }
    // A lost COMMIT acknowledgement fails closed; never proceed to the action.
    tx.commit().await?;
    Ok(Decision {
        allowed: attempts <= 120,
        retry_after_seconds,
    })
}

#[cfg(test)]
mod tests;

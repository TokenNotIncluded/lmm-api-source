use super::{
    MailError, MailKeys, Result, SmtpMailer, SmtpOutcome, random_id, smtp::MailPayload, valid_id,
};
use crate::identity::email::{EmailAddress, EmailChallenges, Request, TransactionalIssue};
use sqlx::{PgPool, Postgres, Row, Transaction, postgres::PgRow};
use std::{sync::Arc, time::Duration};
use zeroize::Zeroizing;

/// Opaque, non-cloneable ownership token. Does not carry plaintext.
pub struct Lease {
    id: String,
    challenge: String,
    token: String,
}

impl Lease {
    pub fn attempt_id(&self) -> &str {
        &self.id
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Dispatch {
    Idle,
    Finished(SmtpOutcome),
    Canceled,
    LostLease,
}

/// Identity-derived recipient roles, not arbitrary deduplication labels.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum NoticeRecipient {
    Current,
    Previous,
}

impl NoticeRecipient {
    fn slot(self) -> &'static str {
        match self {
            Self::Current => "current",
            Self::Previous => "previous",
        }
    }
}

#[derive(Clone)]
pub struct Outbox {
    pool: PgPool,
    keys: Arc<MailKeys>,
}

fn email_error(_: crate::identity::email::EmailError) -> MailError {
    MailError::Unavailable
}

impl Outbox {
    pub fn new(pool: PgPool, keys: MailKeys) -> Self {
        Self {
            pool,
            keys: Arc::new(keys),
        }
    }

    async fn transaction(&self) -> Result<Transaction<'static, Postgres>> {
        let mut tx = self.pool.begin().await?;
        sqlx::query("SET LOCAL lock_timeout='1s';")
            .execute(&mut *tx)
            .await?;
        sqlx::query("SET LOCAL statement_timeout='5s';")
            .execute(&mut *tx)
            .await?;
        Ok(tx)
    }

    /// Owns one transaction for target budgets, challenge and encrypted outbox.
    /// No challenge ID or eligibility status is returned. The Core verifier
    /// selects the current scope from trusted request context. This staged API
    /// is not registered with application routes.
    pub async fn issue(
        &self,
        challenges: &EmailChallenges,
        request: Request<'_>,
        email: &EmailAddress,
    ) -> Result<()> {
        let mut tx = self.transaction().await?;
        self.issue_in(&mut tx, challenges, request, email).await?;
        tx.commit().await?;
        Ok(())
    }

    /// Allows a larger caller transaction to own the same atomic boundary.
    /// All three writes use `tx`, even if the stores were built with other pools.
    pub async fn issue_in(
        &self,
        tx: &mut Transaction<'_, Postgres>,
        challenges: &EmailChallenges,
        request: Request<'_>,
        email: &EmailAddress,
    ) -> Result<()> {
        let issue = challenges
            .issue_in(tx, request, email)
            .await
            .map_err(email_error)?;
        match issue {
            TransactionalIssue::Suppressed => Ok(()),
            TransactionalIssue::Issued(proof) => {
                let sid = match request {
                    Request::Bind { sid, .. } => sid,
                    _ => "",
                };
                self.enqueue_challenge_in(tx, &proof.id, &proof.code, sid)
                    .await?;
                Ok(())
            }
            TransactionalIssue::Pending { id } => {
                // issue_in still holds the target/authority/challenge locks.
                let previous = sqlx::query(
                    "SELECT * FROM core_identity.email_delivery_outbox
                     WHERE challenge_id=$1 AND kind='challenge' AND payload IS NOT NULL
                     ORDER BY created_at DESC,id DESC LIMIT 1 FOR UPDATE",
                )
                .bind(&id)
                .fetch_optional(&mut **tx)
                .await?;
                let Some(previous) = previous else {
                    return Ok(());
                };
                if matches!(
                    previous.try_get::<String, _>("status")?.as_str(),
                    "pending" | "leased"
                ) {
                    return Ok(());
                }
                // An explicit request, through the persistent issue budget,
                // creates a new attempt/message ID with the SAME secret and TTL.
                let payload = self.open_payload(&previous)?;
                self.enqueue_challenge_in(tx, &id, &payload.code, &payload.bind_sid)
                    .await?;
                Ok(())
            }
        }
    }

    /// Trusted issuer primitive. Requires an active caller transaction, never
    /// accepts a post-commit `issue()` result as an atomic issuance workflow.
    pub async fn enqueue_challenge_in(
        &self,
        tx: &mut Transaction<'_, Postgres>,
        challenge: &str,
        code: &str,
        bind_sid: &str,
    ) -> Result<String> {
        let row = lock_challenge(tx, challenge).await?;
        if row.try_get::<String, _>("state")? != "active" || !row.try_get::<bool, _>("live")? {
            return Err(MailError::Conflict);
        }
        let payload = MailPayload {
            recipient: Zeroizing::new(row.try_get("email_key")?),
            purpose: Zeroizing::new(row.try_get("purpose")?),
            code: Zeroizing::new(code.to_owned()),
            bind_sid: Zeroizing::new(bind_sid.to_owned()),
        };
        payload.validate("challenge")?;
        self.insert(tx, challenge, "challenge", "current", &payload)
            .await
    }

    /// Caller must perform its authorized mutation/consumption in this tx first.
    /// A notification contains no token/password, and survives proof invalidation.
    pub async fn enqueue_notice_in(
        &self,
        tx: &mut Transaction<'_, Postgres>,
        challenge: &str,
        recipient: &str,
    ) -> Result<String> {
        let row = lock_challenge(tx, challenge).await?;
        let slot = if recipient == row.try_get::<String, _>("email_key")? {
            NoticeRecipient::Current
        } else {
            NoticeRecipient::Previous
        };
        self.enqueue_notice_slot_in(tx, challenge, recipient, slot)
            .await
    }

    /// Enqueue once per recipient slot after consumption in the same tx.
    /// Previous recipients are supplied only by trusted binding business data.
    pub async fn enqueue_notice_slot_in(
        &self,
        tx: &mut Transaction<'_, Postgres>,
        challenge: &str,
        recipient: &str,
        slot: NoticeRecipient,
    ) -> Result<String> {
        let row = lock_challenge(tx, challenge).await?;
        if row.try_get::<String, _>("state")? != "consumed"
            || row.try_get::<String, _>("purpose")? == "register"
        {
            return Err(MailError::Conflict);
        }
        let email: String = row.try_get("email_key")?;
        if match slot {
            NoticeRecipient::Current => recipient != email,
            NoticeRecipient::Previous => {
                row.try_get::<String, _>("purpose")? != "bind" || recipient == email
            }
        } {
            return Err(MailError::Invalid);
        }
        let payload = MailPayload {
            recipient: Zeroizing::new(recipient.into()),
            purpose: Zeroizing::new(row.try_get("purpose")?),
            code: Zeroizing::new(String::new()),
            bind_sid: Zeroizing::new(String::new()),
        };
        payload.validate("security_notice")?;
        self.insert(tx, challenge, "security_notice", slot.slot(), &payload)
            .await
    }

    async fn insert(
        &self,
        tx: &mut Transaction<'_, Postgres>,
        challenge: &str,
        kind: &str,
        recipient_slot: &str,
        payload: &MailPayload,
    ) -> Result<String> {
        let id = random_id()?;
        let ciphertext = self.keys.encrypt(&id, challenge, kind, &payload.encode())?;
        // Compute the deadline in PostgreSQL, without timestamp truncation.
        let inserted = sqlx::query(
            "INSERT INTO core_identity.email_delivery_outbox(id,challenge_id,kind,key_id,payload,recipient_slot,expires_at)
             SELECT $1,id,$3,$4,$5,$6,
                 CASE WHEN $3='challenge' THEN expires_at ELSE clock_timestamp()+interval '24 hours' END
             FROM core_identity.email_challenges WHERE id=$2
                 AND (($3='challenge' AND state='active' AND expires_at>clock_timestamp())
                     OR ($3='security_notice' AND state='consumed'))
             RETURNING id",
        ).bind(&id).bind(challenge).bind(kind).bind(self.keys.active()).bind(ciphertext).bind(recipient_slot)
            .fetch_optional(&mut **tx).await?;
        if inserted.is_none() {
            return Err(MailError::Conflict);
        }
        Ok(id)
    }

    fn open_payload(&self, row: &PgRow) -> Result<MailPayload> {
        let plain = self.keys.decrypt(
            &row.try_get::<String, _>("id")?,
            &row.try_get::<String, _>("challenge_id")?,
            &row.try_get::<String, _>("kind")?,
            &row.try_get::<String, _>("key_id")?,
            &row.try_get::<Vec<u8>, _>("payload")?,
        )?;
        MailPayload::decode(&plain, &row.try_get::<String, _>("kind")?)
    }

    /// Claims a single attempt in a short durable transaction. Never steals a
    /// lease; expired ownership is terminal UNKNOWN, not an automatic retry.
    pub async fn claim(&self) -> Result<Option<Lease>> {
        let mut tx = self.transaction().await?;
        let row = sqlx::query(
            "SELECT id,challenge_id FROM core_identity.email_delivery_outbox
             WHERE status='pending' AND expires_at>clock_timestamp()
             ORDER BY created_at,id LIMIT 1 FOR UPDATE SKIP LOCKED",
        )
        .fetch_optional(&mut *tx)
        .await?;
        let Some(row) = row else {
            tx.commit().await?;
            return Ok(None);
        };
        let id: String = row.try_get("id")?;
        let token = random_id()?;
        sqlx::query(
            "UPDATE core_identity.email_delivery_outbox SET status='leased',
                 lease_token=$2,lease_until=clock_timestamp()+interval '60 seconds'
             WHERE id=$1",
        )
        .bind(&id)
        .bind(&token)
        .execute(&mut *tx)
        .await?;
        let lease = Lease {
            id,
            challenge: row.try_get("challenge_id")?,
            token,
        };
        tx.commit().await?;
        Ok(Some(lease))
    }

    async fn prepare(&self, lease: &Lease) -> Result<Option<(String, MailPayload)>> {
        let mut tx = self.transaction().await?;
        let challenge = lock_challenge(&mut tx, &lease.challenge).await?;
        let row = sqlx::query(
            "SELECT * FROM core_identity.email_delivery_outbox WHERE id=$1
                 AND status='leased' AND lease_token=$2 AND lease_until>clock_timestamp()
                 AND smtp_started_at IS NULL FOR UPDATE",
        )
        .bind(&lease.id)
        .bind(&lease.token)
        .fetch_optional(&mut *tx)
        .await?;
        let Some(row) = row else {
            tx.commit().await?;
            return Ok(None);
        };
        let kind: String = row.try_get("kind")?;
        let payload = self.open_payload(&row)?;
        let valid = eligible(&mut tx, &challenge, &payload, &kind).await?;
        if !valid {
            terminal(&mut tx, lease, "canceled", "invalidated", true).await?;
            tx.commit().await?;
            return Ok(None);
        }
        // Last check after decryption and policy locks, immediately before IO.
        let changed = sqlx::query(
            "UPDATE core_identity.email_delivery_outbox SET smtp_started_at=clock_timestamp()
             WHERE id=$1 AND status='leased' AND lease_token=$2
                 AND lease_until>clock_timestamp() AND expires_at>clock_timestamp()
                 AND smtp_started_at IS NULL",
        )
        .bind(&lease.id)
        .bind(&lease.token)
        .execute(&mut *tx)
        .await?
        .rows_affected();
        tx.commit().await?;
        Ok((changed == 1).then_some((kind, payload)))
    }

    async fn finish(&self, lease: &Lease, outcome: SmtpOutcome) -> Result<bool> {
        let mut tx = self.transaction().await?;
        lock_challenge(&mut tx, &lease.challenge).await?;
        let (status, class) = outcome.state();
        let changed = terminal(&mut tx, lease, status, class, false).await?;
        tx.commit().await?;
        Ok(changed)
    }

    /// A claimed-but-paused worker is checked again before any SMTP connection.
    /// If the final DB write fails, return an error and leave the lease for the
    /// reaper; never report acceptance or requeue based on the network result.
    pub async fn dispatch(&self, lease: Lease, mailer: &SmtpMailer) -> Result<Dispatch> {
        let Some((kind, payload)) = self.prepare(&lease).await? else {
            return Ok(Dispatch::LostLease);
        };
        let outcome = mailer.send(&lease.id, &kind, &payload).await?;
        if self.finish(&lease, outcome).await? {
            Ok(Dispatch::Finished(outcome))
        } else {
            Ok(Dispatch::LostLease)
        }
    }

    /// Bounded startup/periodic reaper and retention sweep. Terminal attempts
    /// never reopen. Does not delete audit facts or extend any deadline.
    pub async fn maintain(&self) -> Result<u64> {
        let mut tx = self.transaction().await?;
        let ids: Vec<String> = sqlx::query_scalar(
            "SELECT c.id FROM core_identity.email_challenges c WHERE EXISTS (
                SELECT 1 FROM core_identity.email_delivery_outbox o WHERE o.challenge_id=c.id AND (
                    (o.status='leased' AND o.lease_until<=clock_timestamp())
                    OR (o.payload IS NOT NULL AND (o.expires_at<=clock_timestamp()
                        OR (o.kind='challenge' AND c.state<>'active')))))
             ORDER BY c.id LIMIT 32 FOR UPDATE OF c SKIP LOCKED",
        )
        .fetch_all(&mut *tx)
        .await?;
        let mut changed = 0;
        for id in ids {
            changed += sqlx::query(
                "UPDATE core_identity.email_delivery_outbox o SET
                    payload=CASE WHEN o.expires_at<=clock_timestamp() OR
                        (o.kind='challenge' AND c.state<>'active') THEN NULL ELSE o.payload END,
                    status=CASE WHEN o.status='leased' AND o.lease_until<=clock_timestamp() THEN 'unknown'
                        WHEN o.status IN ('pending','leased') THEN
                            CASE WHEN o.smtp_started_at IS NULL THEN 'canceled' ELSE 'unknown' END
                        ELSE o.status END,
                    error_class=CASE WHEN o.status='leased' AND o.lease_until<=clock_timestamp() THEN 'lease_expired'
                        WHEN o.status IN ('pending','leased') THEN 'expired' ELSE o.error_class END,
                    finished_at=CASE WHEN o.status IN ('pending','leased') THEN clock_timestamp() ELSE o.finished_at END,
                    lease_token=NULL,lease_until=NULL
                 FROM core_identity.email_challenges c WHERE c.id=$1 AND o.challenge_id=c.id AND (
                    (o.status='leased' AND o.lease_until<=clock_timestamp())
                    OR (o.payload IS NOT NULL AND (o.expires_at<=clock_timestamp()
                        OR (o.kind='challenge' AND c.state<>'active'))))",
            ).bind(id).execute(&mut *tx).await?.rows_affected();
        }
        tx.commit().await?;
        Ok(changed)
    }

    pub async fn dispatch_one(&self, mailer: &SmtpMailer) -> Result<Dispatch> {
        self.maintain().await?;
        match self.claim().await? {
            Some(lease) => self.dispatch(lease, mailer).await,
            None => Ok(Dispatch::Idle),
        }
    }

    /// Host must explicitly start and join this bounded worker. Shutdown stops
    /// new claims and drains at most one in-flight send (30-second IO bound).
    /// No raw errors, payloads or transport diagnostics are logged.
    pub async fn run(&self, mailer: &SmtpMailer, mut shutdown: tokio::sync::watch::Receiver<bool>) {
        loop {
            if *shutdown.borrow() {
                break;
            }
            let result = self.dispatch_one(mailer).await;
            let wait = if matches!(result, Ok(Dispatch::Finished(_))) {
                Duration::from_millis(100)
            } else {
                Duration::from_secs(2)
            };
            tokio::select! {
                _ = tokio::time::sleep(wait) => {}
                changed = shutdown.changed() => {
                    if changed.is_err() || *shutdown.borrow() { break; }
                }
            }
        }
    }
}

#[cfg(test)]
#[path = "store/tests.rs"]
mod tests;

async fn lock_challenge(tx: &mut Transaction<'_, Postgres>, id: &str) -> Result<PgRow> {
    if !valid_id(id) {
        return Err(MailError::Invalid);
    }
    sqlx::query("SELECT *,expires_at>clock_timestamp() AS live FROM core_identity.email_challenges WHERE id=$1 FOR UPDATE")
        .bind(id).fetch_one(&mut **tx).await.map_err(Into::into)
}

async fn terminal(
    tx: &mut Transaction<'_, Postgres>,
    lease: &Lease,
    status: &str,
    class: &str,
    erase: bool,
) -> Result<bool> {
    Ok(sqlx::query(
        "UPDATE core_identity.email_delivery_outbox SET status=$3,error_class=$4,
             payload=CASE WHEN $5 THEN NULL ELSE payload END,finished_at=clock_timestamp(),
             lease_token=NULL,lease_until=NULL
         WHERE id=$1 AND status='leased' AND lease_token=$2 AND lease_until>clock_timestamp()",
    )
    .bind(&lease.id)
    .bind(&lease.token)
    .bind(status)
    .bind(class)
    .bind(erase)
    .execute(&mut **tx)
    .await?
    .rows_affected()
        == 1)
}

async fn eligible(
    tx: &mut Transaction<'_, Postgres>,
    c: &PgRow,
    payload: &MailPayload,
    kind: &str,
) -> Result<bool> {
    let state: String = c.try_get("state")?;
    let purpose: String = c.try_get("purpose")?;
    if payload.purpose.as_str() != purpose {
        return Ok(false);
    }
    if kind == "security_notice" {
        return Ok(state == "consumed");
    }
    if state != "active"
        || !c.try_get::<bool, _>("live")?
        || payload.recipient.as_str() != c.try_get::<String, _>("email_key")?
    {
        return Ok(false);
    }
    if purpose == "register" {
        let occupied: bool = sqlx::query_scalar(
            "SELECT EXISTS(SELECT 1 FROM core_identity.email_addresses WHERE email_key=$1)
                 OR EXISTS(SELECT 1 FROM core_identity.user_profiles
                     WHERE lower(btrim(email))=$1 OR lower(username)=$1)",
        )
        .bind(payload.recipient.as_str())
        .fetch_one(&mut **tx)
        .await?;
        return Ok(!occupied);
    }
    let user: i64 = c.try_get("user_id")?;
    let current = sqlx::query(
        "SELECT u.auth_version,p.email_version,p.password_login_allowed,p.second_factor_required
         FROM core_identity.users u JOIN core_identity.accounts a ON a.id=u.personal_account_id
         JOIN core_identity.user_profiles p ON p.user_id=u.id
         WHERE u.id=$1 AND u.active AND a.active FOR SHARE OF u,a,p NOWAIT",
    )
    .bind(user)
    .fetch_optional(&mut **tx)
    .await?;
    let Some(current) = current else {
        return Ok(false);
    };
    let version: i64 = c.try_get("user_version")?;
    let email_version: i64 = c.try_get("email_version")?;
    if current.try_get::<i64, _>("auth_version")? != version
        || current.try_get::<i64, _>("email_version")? != email_version
    {
        return Ok(false);
    }
    if purpose == "password_reset" {
        if !current.try_get::<bool, _>("password_login_allowed")? {
            return Ok(false);
        }
        let authority = sqlx::query(
            "SELECT e.user_id FROM core_identity.email_addresses e
             JOIN core_identity.password_logins p ON p.user_id=e.user_id
             WHERE e.user_id=$1 AND e.email_key=$2 AND e.state='verified' AND e.email_version=$3
             FOR SHARE OF e,p NOWAIT",
        )
        .bind(user)
        .bind(payload.recipient.as_str())
        .bind(email_version)
        .fetch_optional(&mut **tx)
        .await?;
        return Ok(authority.is_some());
    }
    let session = sqlx::query(
        "SELECT s.sid FROM core_identity.web_sessions s
         JOIN core_identity.credentials c ON c.id=s.current_credential_id
         JOIN core_identity.session_authentication a ON a.credential_id=c.id AND a.user_id=s.user_id
         JOIN core_identity.user_profiles p ON p.user_id=s.user_id
         LEFT JOIN core_identity.mfa_factors m ON m.user_id=s.user_id
         WHERE s.sid=$1 AND s.user_id=$2 AND s.user_version=$3 AND s.revoked_at IS NULL
             AND s.expires_at>clock_timestamp()
             AND (NOT s.weekly_logout OR s.created_at>clock_timestamp()-interval '7 days')
             AND c.kind='session' AND c.revoked_at IS NULL AND c.expires_at>clock_timestamp()
             AND c.user_id=s.user_id AND c.user_version=$3 AND a.user_version=$3
             AND a.authenticated_at>clock_timestamp()-interval '5 minutes'
             AND a.authenticated_at<=clock_timestamp()
             AND p.second_factor_required=coalesce(m.enabled,false)
             AND (NOT p.second_factor_required OR (a.factor_level=2 AND a.factor_revision=m.revision
                 AND a.mfa_verified_at>clock_timestamp()-interval '5 minutes'
                 AND a.mfa_verified_at<=clock_timestamp()))
         FOR SHARE OF s,c,a NOWAIT",
    )
    .bind(payload.bind_sid.as_str())
    .bind(user)
    .bind(version)
    .fetch_optional(&mut **tx)
    .await?;
    if session.is_none() {
        return Ok(false);
    }
    let occupied: bool = sqlx::query_scalar(
        "SELECT EXISTS(SELECT 1 FROM core_identity.email_addresses
             WHERE email_key=$1 AND (user_id IS DISTINCT FROM $2 OR state='conflict'))
         OR EXISTS(SELECT 1 FROM core_identity.user_profiles
             WHERE user_id<>$2 AND (lower(btrim(email))=$1 OR lower(username)=$1))",
    )
    .bind(payload.recipient.as_str())
    .bind(user)
    .fetch_one(&mut **tx)
    .await?;
    Ok(!occupied)
}

use super::{EmailAddress, EmailError, Result};
use hmac::{Hmac, Mac};
use rand::{RngCore, rngs::OsRng};
use sha2::{Digest, Sha256};
use sqlx::{PgConnection, PgPool, Postgres, Row, Transaction, postgres::PgRow};
use std::sync::Arc;
use zeroize::Zeroizing;

/// Requests carry current browser credentials, never caller-declared versions.
#[derive(Clone, Copy)]
pub enum Request<'a> {
    Register,
    Bind { access: &'a str, sid: &'a str },
    PasswordReset,
}

impl Request<'_> {
    fn purpose(self) -> &'static str {
        match self {
            Self::Register => "register",
            Self::Bind { .. } => "bind",
            Self::PasswordReset => "password_reset",
        }
    }
}

struct Keys {
    id: String,
    proof: Zeroizing<[u8; 32]>,
    rate: Zeroizing<[u8; 32]>,
}

/// No Debug or serialization: the code is a one-time delivery secret.
pub struct IssuedEmailChallenge {
    pub id: String,
    pub code: Zeroizing<String>,
    pub expires_at: i64,
}

/// Internal delivery result. Public adapters must hide eligibility differences.
pub enum IssueOutcome {
    Issued(IssuedEmailChallenge),
    Pending,
    Suppressed,
}

/// Delivery composition keeps the transaction open, including target budgets.
/// A pending proof has no new secret and must have a recoverable outbox payload
/// before a caller can promise durable mail acceptance.
pub enum TransactionalIssue {
    Issued(IssuedEmailChallenge),
    Pending { id: String },
    Suppressed,
}

#[derive(Clone)]
pub struct EmailChallenges {
    pool: PgPool,
    keys: Arc<Keys>,
}

#[derive(Default, Eq, PartialEq)]
struct Subject {
    user: Option<i64>,
    version: Option<i64>,
    email_version: Option<i64>,
    sid: Option<Vec<u8>>,
}

impl Subject {
    fn from_row(row: &PgRow) -> Result<Self> {
        Ok(Self {
            user: row.try_get("user_id")?,
            version: row.try_get("user_version")?,
            email_version: row.try_get("email_version")?,
            sid: row.try_get("sid_digest")?,
        })
    }
}

fn mac(key: &[u8], parts: &[&[u8]]) -> Vec<u8> {
    let mut mac = Hmac::<Sha256>::new_from_slice(key).expect("HMAC accepts 32-byte keys");
    for part in parts {
        mac.update(&(part.len() as u64).to_be_bytes());
        mac.update(part);
    }
    mac.finalize().into_bytes().to_vec()
}

fn random_hex(bytes: usize) -> Result<Zeroizing<String>> {
    let mut random = Zeroizing::new(vec![0_u8; bytes]);
    OsRng
        .try_fill_bytes(&mut random)
        .map_err(|_| EmailError::Unavailable)?;
    let mut text = Zeroizing::new(String::with_capacity(bytes * 2));
    for byte in random.iter() {
        use std::fmt::Write;
        write!(text, "{byte:02x}").map_err(|_| EmailError::Unavailable)?;
    }
    Ok(text)
}

impl EmailChallenges {
    /// Requires an explicitly installed delta. Does not run SQL or enable routes.
    /// Keys must come from separate persistent secrets; rate-key rotation needs
    /// a coordinated window migration and is intentionally not implemented.
    pub fn new(pool: PgPool, key_id: &str, proof: [u8; 32], rate: [u8; 32]) -> Result<Self> {
        if key_id.is_empty()
            || key_id.len() > 64
            || !key_id
                .bytes()
                .all(|c| c.is_ascii_alphanumeric() || c == b'-' || c == b'_')
            || proof == [0; 32]
            || rate == [0; 32]
            || proof == rate
        {
            return Err(EmailError::Unavailable);
        }
        Ok(Self {
            pool,
            keys: Arc::new(Keys {
                id: key_id.to_owned(),
                proof: Zeroizing::new(proof),
                rate: Zeroizing::new(rate),
            }),
        })
    }

    async fn transaction(&self) -> Result<Transaction<'static, Postgres>> {
        let mut tx = self.pool.begin().await?;
        sqlx::query("SET LOCAL lock_timeout='1s'")
            .execute(&mut *tx)
            .await?;
        sqlx::query("SET LOCAL statement_timeout='5s'")
            .execute(&mut *tx)
            .await?;
        Ok(tx)
    }

    fn bucket(&self, email: &EmailAddress) -> Vec<u8> {
        mac(
            self.keys.rate.as_ref(),
            &[b"email-target-v1", email.as_str().as_bytes()],
        )
    }

    // Every purpose and process locks the same target before reading its windows.
    async fn budget(&self, c: &mut PgConnection, email: &EmailAddress, kind: &str) -> Result<bool> {
        let bucket = self.bucket(email);
        sqlx::query("INSERT INTO core_identity.email_rate_buckets(bucket) VALUES ($1) ON CONFLICT DO NOTHING")
            .bind(&bucket).execute(&mut *c).await?;
        sqlx::query(
            "SELECT bucket FROM core_identity.email_rate_buckets WHERE bucket=$1 FOR UPDATE",
        )
        .bind(&bucket)
        .fetch_one(&mut *c)
        .await?;
        let limits = if kind == "issue" {
            (60_i64, 1_i64, 3600_i64, 5_i64, 86400_i64, 10_i64)
        } else {
            (600, 10, 3600, 30, 3600, 30)
        };
        let allowed: bool = sqlx::query_scalar(
            "SELECT count(*) FILTER (WHERE occurred_at>clock_timestamp()-$3::bigint*interval '1 second')<$4::bigint
                AND count(*) FILTER (WHERE occurred_at>clock_timestamp()-$5::bigint*interval '1 second')<$6::bigint
                AND count(*)<$8::bigint
             FROM core_identity.email_rate_events
             WHERE bucket=$1 AND kind=$2 AND occurred_at>clock_timestamp()-$7::bigint*interval '1 second'",
        )
        .bind(&bucket).bind(kind).bind(limits.0).bind(limits.1)
        .bind(limits.2).bind(limits.3).bind(limits.4).bind(limits.5)
        .fetch_one(&mut *c).await?;
        // Denied attempts do not create unbounded rows or extend a denial window.
        if allowed {
            sqlx::query("INSERT INTO core_identity.email_rate_events(bucket,kind) VALUES ($1,$2)")
                .bind(&bucket)
                .bind(kind)
                .execute(c)
                .await?;
        }
        Ok(allowed)
    }

    fn scope(&self, request: Request<'_>, email: &EmailAddress, subject: &Subject) -> Vec<u8> {
        mac(
            self.keys.rate.as_ref(),
            &[
                b"email-scope-v1",
                request.purpose().as_bytes(),
                email.as_str().as_bytes(),
                &subject.user.unwrap_or(0).to_be_bytes(),
                subject.sid.as_deref().unwrap_or_default(),
            ],
        )
    }

    fn proof_mac(
        &self,
        id: &str,
        purpose: &str,
        email: &EmailAddress,
        subject: &Subject,
        code: &str,
    ) -> Vec<u8> {
        mac(
            self.keys.proof.as_ref(),
            &[
                b"email-proof-v1",
                id.as_bytes(),
                purpose.as_bytes(),
                email.as_str().as_bytes(),
                &subject.user.unwrap_or(0).to_be_bytes(),
                &subject.version.unwrap_or(0).to_be_bytes(),
                &subject.email_version.unwrap_or(0).to_be_bytes(),
                subject.sid.as_deref().unwrap_or_default(),
                code.as_bytes(),
            ],
        )
    }

    /// Resolves authority only from current database facts, never profile email.
    async fn subject(
        &self,
        c: &mut PgConnection,
        request: Request<'_>,
        email: &EmailAddress,
    ) -> Result<Option<Subject>> {
        match request {
            Request::Register => {
                let occupied: bool = sqlx::query_scalar(
                    "SELECT EXISTS(SELECT 1 FROM core_identity.email_addresses WHERE email_key=$1)
                     OR EXISTS(SELECT 1 FROM core_identity.user_profiles
                         WHERE lower(btrim(email))=$1 OR lower(username)=$1)",
                )
                .bind(email.as_str())
                .fetch_one(c)
                .await?;
                Ok((!occupied).then(Subject::default))
            }
            Request::PasswordReset => {
                let row = sqlx::query(
                    "SELECT u.id AS user_id,u.auth_version AS user_version,pr.email_version
                     FROM core_identity.email_addresses e
                     JOIN core_identity.users u ON u.id=e.user_id
                     JOIN core_identity.accounts a ON a.id=u.personal_account_id
                     JOIN core_identity.user_profiles pr ON pr.user_id=u.id
                     JOIN core_identity.password_logins p ON p.user_id=u.id
                     WHERE e.email_key=$1 AND e.state='verified' AND e.email_version=pr.email_version
                         AND u.active AND a.active AND pr.password_login_allowed
                     FOR SHARE OF e,u,a,pr,p NOWAIT",
                ).bind(email.as_str()).fetch_optional(c).await?;
                row.map(|r| {
                    Ok(Subject {
                        user: Some(r.try_get("user_id")?),
                        version: Some(r.try_get("user_version")?),
                        email_version: Some(r.try_get("email_version")?),
                        sid: None,
                    })
                })
                .transpose()
            }
            Request::Bind { access, sid } => {
                if access.len() > 256 || sid.len() != 64 {
                    return Ok(None);
                }
                let row = sqlx::query(
                    "SELECT u.id AS user_id,u.auth_version AS user_version,pr.email_version,
                         pr.second_factor_required,sa.factor_level,sa.factor_revision,
                         sa.mfa_verified_at>clock_timestamp()-interval '5 minutes'
                             AND sa.mfa_verified_at<=clock_timestamp() AS recent_mfa
                     FROM core_identity.credentials c
                     JOIN core_identity.users u ON u.id=c.user_id
                     JOIN core_identity.accounts a ON a.id=u.personal_account_id
                     JOIN core_identity.user_profiles pr ON pr.user_id=u.id
                     JOIN core_identity.web_sessions s ON s.current_credential_id=c.id AND s.user_id=u.id
                     JOIN core_identity.session_authentication sa ON sa.credential_id=c.id
                         AND sa.user_id=u.id AND sa.user_version=u.auth_version
                     WHERE c.digest=$1 AND c.kind='session' AND s.sid=$2
                         AND c.owner_account_id=u.personal_account_id
                         AND c.user_version=u.auth_version AND s.user_version=u.auth_version
                         AND u.active AND a.active AND c.revoked_at IS NULL AND s.revoked_at IS NULL
                         AND c.expires_at>clock_timestamp() AND s.expires_at>clock_timestamp()
                         AND (NOT s.weekly_logout OR s.created_at>clock_timestamp()-interval '7 days')
                         AND sa.authenticated_at>clock_timestamp()-interval '5 minutes'
                         AND sa.authenticated_at<=clock_timestamp()
                     FOR SHARE OF c,u,a,pr,s,sa NOWAIT",
                ).bind(Sha256::digest(access.as_bytes()).to_vec()).bind(sid)
                .fetch_optional(&mut *c).await?;
                let Some(row) = row else { return Ok(None) };
                let user: i64 = row.try_get("user_id")?;
                let factor = sqlx::query("SELECT enabled,revision FROM core_identity.mfa_factors WHERE user_id=$1 FOR SHARE NOWAIT")
                    .bind(user).fetch_optional(&mut *c).await?;
                let required: bool = row.try_get("second_factor_required")?;
                let enabled = factor
                    .as_ref()
                    .map(|r| r.try_get::<bool, _>("enabled"))
                    .transpose()?
                    .unwrap_or(false);
                if required != enabled
                    || (required
                        && (row.try_get::<i16, _>("factor_level")? != 2
                            || row.try_get::<Option<bool>, _>("recent_mfa")? != Some(true)
                            || Some(row.try_get::<i64, _>("factor_revision")?)
                                != factor
                                    .as_ref()
                                    .map(|r| r.try_get::<i64, _>("revision"))
                                    .transpose()?))
                {
                    return Ok(None);
                }
                let occupied: bool = sqlx::query_scalar(
                    "SELECT EXISTS(SELECT 1 FROM core_identity.email_addresses
                         WHERE email_key=$1 AND (user_id IS DISTINCT FROM $2 OR state='conflict'))
                     OR EXISTS(SELECT 1 FROM core_identity.user_profiles
                         WHERE user_id<>$2 AND (lower(btrim(email))=$1 OR lower(username)=$1))",
                )
                .bind(email.as_str())
                .bind(user)
                .fetch_one(c)
                .await?;
                let subject = Subject {
                    user: Some(user),
                    version: Some(row.try_get("user_version")?),
                    email_version: Some(row.try_get("email_version")?),
                    sid: Some(mac(
                        self.keys.proof.as_ref(),
                        &[b"email-sid-v1", sid.as_bytes()],
                    )),
                };
                Ok((!occupied).then_some(subject))
            }
        }
    }

    /// Issues only a proof; this does not register, bind, reset or send mail.
    /// Repeated live requests never rotate a code or reset its attempt budget.
    pub async fn issue(&self, request: Request<'_>, email: &EmailAddress) -> Result<IssueOutcome> {
        let mut tx = self.transaction().await?;
        let result = self.issue_in(&mut tx, request, email).await?;
        tx.commit().await?;
        Ok(match result {
            TransactionalIssue::Issued(proof) => IssueOutcome::Issued(proof),
            TransactionalIssue::Pending { .. } => IssueOutcome::Pending,
            TransactionalIssue::Suppressed => IssueOutcome::Suppressed,
        })
    }

    /// Does not begin or commit. The caller must enqueue encrypted delivery in
    /// this same transaction before committing an issued secret. Never bridge
    /// the standalone `issue()` result into a later mail transaction.
    pub async fn issue_in(
        &self,
        tx: &mut Transaction<'_, Postgres>,
        request: Request<'_>,
        email: &EmailAddress,
    ) -> Result<TransactionalIssue> {
        if !self.budget(tx, email, "issue").await? {
            return Ok(TransactionalIssue::Suppressed);
        }
        let Some(subject) = self.subject(tx, request, email).await? else {
            return Ok(TransactionalIssue::Suppressed);
        };
        let scope = self.scope(request, email, &subject);
        let old = sqlx::query("SELECT *,expires_at>clock_timestamp() AS live FROM core_identity.email_challenges WHERE scope_key=$1 AND state='active' FOR UPDATE")
            .bind(&scope).fetch_optional(&mut **tx).await?;
        if let Some(old) = old {
            if old.try_get::<bool, _>("live")? && Subject::from_row(&old)? == subject {
                return Ok(TransactionalIssue::Pending {
                    id: old.try_get("id")?,
                });
            }
            sqlx::query("UPDATE core_identity.email_challenges SET state=CASE WHEN expires_at<=clock_timestamp() THEN 'expired' ELSE 'superseded' END WHERE id=$1")
                .bind(old.try_get::<String,_>("id")?).execute(&mut **tx).await?;
        }
        let id = random_hex(16)?.to_string();
        let code = random_hex(if matches!(request, Request::PasswordReset) {
            16
        } else {
            3
        })?;
        let expires_at = sqlx::query_scalar(
            "INSERT INTO core_identity.email_challenges(id,scope_key,purpose,email_key,user_id,
                 user_version,email_version,sid_digest,key_id,token_mac)
             VALUES ($1,$2,$3,$4,$5,$6,$7,$8,$9,$10)
             RETURNING floor(extract(epoch FROM expires_at))::bigint",
        )
        .bind(&id)
        .bind(scope)
        .bind(request.purpose())
        .bind(email.as_str())
        .bind(subject.user)
        .bind(subject.version)
        .bind(subject.email_version)
        .bind(&subject.sid)
        .bind(&self.keys.id)
        .bind(self.proof_mac(&id, request.purpose(), email, &subject, &code))
        .fetch_one(&mut **tx)
        .await?;
        Ok(TransactionalIssue::Issued(IssuedEmailChallenge {
            id,
            code,
            expires_at,
        }))
    }

    /// Rejections commit counters; success retains all locks for the mutation.
    /// `Request` must name the endpoint's expected purpose, never untrusted JSON.
    pub async fn verify(
        &self,
        request: Request<'_>,
        email: &EmailAddress,
        id: &str,
        code: &str,
    ) -> Result<EmailMutation> {
        self.verify_selected(request, email, ChallengeLookup::Id(id), code)
            .await
    }

    /// Verifies without requiring a challenge identifier from a client.
    ///
    /// The trusted purpose, canonical email and current authenticated subject
    /// select the unique active scope. Target guessing budget is committed
    /// before resolving the subject or looking for a challenge, including when
    /// no matching challenge exists.
    pub async fn verify_current(
        &self,
        request: Request<'_>,
        email: &EmailAddress,
        code: &str,
    ) -> Result<EmailMutation> {
        self.verify_selected(request, email, ChallengeLookup::CurrentScope, code)
            .await
    }

    async fn verify_selected(
        &self,
        request: Request<'_>,
        email: &EmailAddress,
        lookup: ChallengeLookup<'_>,
        code: &str,
    ) -> Result<EmailMutation> {
        let mut tx = self.transaction().await?;
        if !self.budget(&mut tx, email, "verify").await? {
            tx.commit().await?;
            return Err(EmailError::InvalidProof);
        }
        let row = match lookup {
            ChallengeLookup::Id(id) => {
                sqlx::query(
                    "SELECT * FROM core_identity.email_challenges
                     WHERE id=$1 AND email_key=$2 AND purpose=$3 FOR UPDATE",
                )
                .bind(id)
                .bind(email.as_str())
                .bind(request.purpose())
                .fetch_optional(&mut *tx)
                .await?
            }
            ChallengeLookup::CurrentScope => {
                let subject = self
                    .subject(&mut tx, request, email)
                    .await?
                    .unwrap_or_default();
                let scope = self.scope(request, email, &subject);
                sqlx::query(
                    "SELECT * FROM core_identity.email_challenges
                     WHERE scope_key=$1 AND email_key=$2 AND purpose=$3
                         AND state='active' FOR UPDATE",
                )
                .bind(scope)
                .bind(email.as_str())
                .bind(request.purpose())
                .fetch_optional(&mut *tx)
                .await?
            }
        };
        let Some(row) = row else {
            tx.commit().await?;
            return Err(EmailError::InvalidProof);
        };
        let id: String = row.try_get("id")?;
        if row.try_get::<String, _>("state")? != "active" {
            tx.commit().await?;
            return Err(EmailError::InvalidProof);
        }
        // Read wall-clock time after obtaining both the target and challenge locks.
        let live: bool = sqlx::query_scalar(
            "SELECT expires_at>clock_timestamp() FROM core_identity.email_challenges WHERE id=$1",
        )
        .bind(&id)
        .fetch_one(&mut *tx)
        .await?;
        let expected = Subject::from_row(&row)?;
        let current = self.subject(&mut tx, request, email).await?;
        // An unauthenticated or different browser cannot retire another user's
        // pending proof. Changed generations are rejected without reviving it.
        if current.as_ref() != Some(&expected) {
            tx.commit().await?;
            return Err(EmailError::InvalidProof);
        }
        if !live {
            sqlx::query("UPDATE core_identity.email_challenges SET state=$2 WHERE id=$1")
                .bind(&id)
                .bind("expired")
                .execute(&mut *tx)
                .await?;
            tx.commit().await?;
            return Err(EmailError::InvalidProof);
        }
        let expected_len = if matches!(request, Request::PasswordReset) {
            32
        } else {
            6
        };
        let valid_shape = code.len() == expected_len
            && code
                .bytes()
                .all(|c| c.is_ascii_digit() || (b'a'..=b'f').contains(&c));
        let actual = self.proof_mac(
            &id,
            request.purpose(),
            email,
            &expected,
            if valid_shape { code } else { "" },
        );
        use subtle::ConstantTimeEq;
        if row.try_get::<String, _>("key_id")? != self.keys.id
            || !valid_shape
            || !bool::from(actual.ct_eq(&row.try_get::<Vec<u8>, _>("token_mac")?))
        {
            sqlx::query("UPDATE core_identity.email_challenges SET attempts=attempts+1,state=CASE WHEN attempts+1>=5 THEN 'locked' ELSE 'active' END WHERE id=$1")
                .bind(id).execute(&mut *tx).await?;
            tx.commit().await?;
            return Err(EmailError::InvalidProof);
        }
        Ok(EmailMutation {
            tx,
            id,
            purpose: request.purpose(),
            user: expected.user,
        })
    }
}

enum ChallengeLookup<'a> {
    Id(&'a str),
    CurrentScope,
}

/// A proof is usable only while its transaction and authority locks are held.
#[must_use = "dropping the mutation rolls back all changes"]
pub struct EmailMutation {
    tx: Transaction<'static, Postgres>,
    id: String,
    purpose: &'static str,
    user: Option<i64>,
}

impl EmailMutation {
    /// Business SQL, authority projection and notification enqueue belong here.
    pub fn connection(&mut self) -> &mut PgConnection {
        &mut self.tx
    }

    /// Challenge identity selected and verified by Core; callers must not
    /// accept this value from request data.
    pub fn challenge_id(&self) -> &str {
        &self.id
    }

    /// Compatibility completion for callers with no required notification.
    pub async fn finish(self, result_user_id: i64) -> Result<()> {
        self.consume(result_user_id).await?.commit().await
    }

    /// Consumes and audits in the business transaction without committing.
    /// Required notices must be enqueued before the returned guard commits.
    pub async fn consume(mut self, result_user_id: i64) -> Result<ConsumedEmailMutation> {
        if result_user_id <= 0 || self.user.is_some_and(|user| user != result_user_id) {
            return Err(EmailError::InvalidProof);
        }
        let changed = sqlx::query(
            "UPDATE core_identity.email_challenges SET state='consumed',
                 consumed_at=clock_timestamp(),result_user_id=$2
             WHERE id=$1 AND state='active' AND attempts<5 AND expires_at>clock_timestamp()",
        )
        .bind(&self.id)
        .bind(result_user_id)
        .execute(&mut *self.tx)
        .await?
        .rows_affected();
        if changed != 1 {
            return Err(EmailError::InvalidProof);
        }
        sqlx::query(
            "INSERT INTO core_identity.audit(actor_user_id,action,target)
             VALUES ($1,'identity.email_proof_consumed',
                 jsonb_build_object('challenge_id',$2::text,'purpose',$3::text))",
        )
        .bind(result_user_id)
        .bind(&self.id)
        .bind(self.purpose)
        .execute(&mut *self.tx)
        .await?;
        Ok(ConsumedEmailMutation {
            tx: self.tx,
            id: self.id,
        })
    }
}

/// Consumed proof and business writes remain invisible until commit.
#[must_use = "dropping the guard rolls back business, proof and notices"]
pub struct ConsumedEmailMutation {
    tx: Transaction<'static, Postgres>,
    id: String,
}

impl ConsumedEmailMutation {
    /// The same transaction required by encrypted outbox enqueue APIs.
    pub fn transaction(&mut self) -> &mut Transaction<'static, Postgres> {
        &mut self.tx
    }

    pub fn challenge_id(&self) -> &str {
        &self.id
    }

    /// Call only after every required notice has been enqueued successfully.
    pub async fn commit(self) -> Result<()> {
        self.tx.commit().await?;
        Ok(())
    }
}

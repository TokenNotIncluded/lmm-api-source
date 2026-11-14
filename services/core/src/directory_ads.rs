//! Platform advertisements: personal spend, browser confirmation, original-charge refunds.
use crate::{
    accounts::AccountKind,
    billing::{
        Credits, charging,
        ledger::{Action, Outcome, Request as LedgerRequest},
    },
    identity::{self, CredentialKind, Principal},
    internal_rpc::pb,
};
use rand::Rng as _;
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use sqlx::{PgConnection, PgPool, Postgres, Row, Transaction, types::Json};
mod content;
pub mod http;
mod permissions;
pub mod rpc;
pub use content::content_digest;
pub(crate) use permissions::check_privileges;
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Error {
    Invalid,
    Unauthorized,
    Forbidden,
    Conflict,
    Expired,
    ConfirmationRequired,
    Storage,
    Retry,
}
pub type Result<T> = std::result::Result<T, Error>;
impl From<sqlx::Error> for Error {
    fn from(e: sqlx::Error) -> Self {
        if e.as_database_error()
            .is_some_and(|e| matches!(e.code().as_deref(), Some("40001" | "40P01" | "23505")))
        {
            Self::Retry
        } else {
            Self::Storage
        }
    }
}
impl From<identity::IdentityError> for Error {
    fn from(e: identity::IdentityError) -> Self {
        match e {
            identity::IdentityError::Unauthorized => Self::Unauthorized,
            identity::IdentityError::Forbidden => Self::Forbidden,
            identity::IdentityError::Invalid => Self::Invalid,
            identity::IdentityError::Conflict => Self::Retry,
            _ => Self::Storage,
        }
    }
}
#[derive(Clone)]
pub struct Store {
    pub(crate) pool: PgPool,
}
fn hash(bytes: &[u8]) -> String {
    crate::hex_codec::encode(Sha256::digest(bytes))
}
fn random_id() -> String {
    let mut b = [0u8; 32];
    rand::rand_core::UnwrapErr(rand::rngs::SysRng).fill_bytes(&mut b);
    hash(&b)
}
fn key(v: &str) -> bool {
    (16..=80).contains(&v.len())
        && v.bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'_' || b == b'-')
}
fn payment_command(v: &pb::PayDirectoryAdRequest) -> Value {
    json!([
        v.protocol_major,
        v.request_id,
        v.quote_id,
        v.expected_quota,
        v.content_sha256
    ])
}
fn refund_command(v: &pb::RefundDirectoryAdRequest) -> Value {
    json!([v.protocol_major, v.request_id, v.payment_id])
}
fn encode_receipt(r: &pb::DirectoryAdReceipt) -> Value {
    json!({"request_id":r.request_id,"command_sha256":r.command_sha256,"outcome":r.outcome,"rejection_code":r.rejection_code,"payment_id":r.payment_id,"charged_quota":r.charged_quota,"paid_at":r.paid_at,"ad_expires_at":r.ad_expires_at,"refund_id":r.refund_id,"refunded_quota":r.refunded_quota,"refunded_at":r.refunded_at})
}
fn decode_receipt(v: Value) -> Result<pb::DirectoryAdReceipt> {
    let s = |k: &str| {
        v.get(k)
            .and_then(Value::as_str)
            .map(str::to_owned)
            .ok_or(Error::Storage)
    };
    let n = |k: &str| v.get(k).and_then(Value::as_i64).ok_or(Error::Storage);
    Ok(pb::DirectoryAdReceipt {
        request_id: s("request_id")?,
        command_sha256: s("command_sha256")?,
        outcome: s("outcome")?,
        rejection_code: s("rejection_code")?,
        payment_id: s("payment_id")?,
        charged_quota: n("charged_quota")?,
        paid_at: n("paid_at")?,
        ad_expires_at: n("ad_expires_at")?,
        refund_id: s("refund_id")?,
        refunded_quota: n("refunded_quota")?,
        refunded_at: n("refunded_at")?,
    })
}
async fn actor(c: &mut PgConnection, secret: &str) -> Result<Principal> {
    let a = identity::authorize_in_transaction(c, secret).await?;
    if a.credential_kind != CredentialKind::Session
        || a.owner.kind != AccountKind::Personal
        || a.owner.id != a.user_id
    {
        return Err(Error::Forbidden);
    }
    Ok(a)
}
async fn root(c: &mut PgConnection, secret: &str) -> Result<Principal> {
    let a = actor(c, secret).await?;
    let (role, version): (String, i64) = sqlx::query_as(
        "SELECT platform_role,auth_version FROM core_identity.users WHERE id=$1 FOR SHARE",
    )
    .bind(a.user_id)
    .fetch_one(&mut *c)
    .await?;
    if role != "superadmin" {
        return Err(Error::Forbidden);
    }
    use identity::permissions as p;
    let loaded = match identity::load_complete_policy(c, a.user_id).await {
        Ok(policy) => policy,
        Err(p::PolicyLoadError::Missing)
            if !p::has_privileged_import_facts(c, a.user_id)
                .await
                .map_err(|_| Error::Storage)? =>
        {
            return Ok(a);
        }
        Err(p::PolicyLoadError::Storage) => return Err(Error::Storage),
        Err(_) => return Err(Error::Forbidden),
    };
    if !p::decide(
        &p::Actor {
            user_id: a.user_id,
            personal_account_id: a.owner_account_id,
            role: p::PlatformRole::Root,
            credential: p::Credential::Session,
            active: true,
            account_active: true,
            auth_version: version,
            credential_auth_version: version,
            developer: p::DeveloperState::Granted,
        },
        &loaded.as_policy(),
        p::Action::DirectoryModerate,
        p::Scope::Platform(p::PlatformResource::Directory),
        None,
    )
    .allowed()
    {
        return Err(Error::Forbidden);
    }
    Ok(a)
}
async fn now(c: &mut PgConnection) -> Result<i64> {
    Ok(
        sqlx::query_scalar("SELECT floor(extract(epoch FROM clock_timestamp()))::bigint")
            .fetch_one(c)
            .await?,
    )
}
async fn lock(c: &mut PgConnection, principal: &str, k: &str) -> Result<()> {
    sqlx::query("SELECT pg_advisory_xact_lock(hashtextextended($1,0))")
        .bind(format!("directory-ad:{principal}:{k}"))
        .execute(c)
        .await?;
    Ok(())
}
impl Store {
    pub fn new(pool: PgPool) -> Self {
        Self { pool }
    }
    async fn begin(&self) -> Result<Transaction<'_, Postgres>> {
        let mut t = self.pool.begin().await?;
        for statement in [
            "SET TRANSACTION ISOLATION LEVEL SERIALIZABLE",
            "SET LOCAL synchronous_commit=on",
            "SET LOCAL lock_timeout='500ms'",
            "SET LOCAL statement_timeout='3s'",
        ] {
            sqlx::query(statement).execute(&mut *t).await?;
        }
        Ok(t)
    }
    pub async fn prepare(
        &self,
        principal: &str,
        secret: &str,
        v: &pb::PrepareDirectoryAdRequest,
    ) -> Result<pb::DirectoryAdQuote> {
        content::validate_consent(v)?;
        let mut t = self.begin().await?;
        let a = actor(&mut t, secret).await?;
        lock(&mut t, principal, &v.request_id).await?;
        let command = json!([
            v.content_version,
            v.name,
            v.url,
            v.summary,
            v.description,
            v.bid_cents,
            v.content_sha256
        ]);
        let old = sqlx::query(
            "SELECT * FROM core_directory_ads.quotes WHERE principal=$1 AND request_id=$2",
        )
        .bind(principal)
        .bind(&v.request_id)
        .fetch_optional(&mut *t)
        .await?;
        let (id, expires, quota) = if let Some(r) = old {
            if r.try_get::<i64, _>("user_id")? != a.user_id
                || r.try_get::<Value, _>("command")? != command
            {
                return Err(Error::Conflict);
            }
            (
                r.try_get("id")?,
                r.try_get("expires_at")?,
                r.try_get("quota")?,
            )
        } else {
            identity::reject_marked_new_action(
                &mut t,
                a.user_id,
                identity::NewResourceAction::CommercePayment,
            )
            .await?;
            let id = random_id();
            let at = now(&mut t).await?;
            let quota = v
                .bid_cents
                .checked_mul(crate::billing::CREDITS_PER_USD)
                .ok_or(Error::Invalid)?
                / 100;
            sqlx::query("INSERT INTO core_directory_ads.quotes(id,principal,request_id,user_id,account_id,command,content_sha256,quota,created_at,expires_at) VALUES($1,$2,$3,$4,$5,$6,$7,$8,$9,$10)").bind(&id).bind(principal).bind(&v.request_id).bind(a.user_id).bind(a.owner_account_id).bind(command).bind(&v.content_sha256).bind(quota).bind(at).bind(at+600).execute(&mut *t).await?;
            (id, at + 600, quota)
        };
        t.commit().await?;
        Ok(pb::DirectoryAdQuote {
            confirmation_path: format!("/core/v1/directory-ads/confirm/{id}"),
            quote_id: id,
            request_id: v.request_id.clone(),
            content_sha256: v.content_sha256.clone(),
            bid_cents: v.bid_cents,
            quota,
            currency: "USD".into(),
            pricing_schema_version: 2,
            quote_contract_version: 1,
            expires_at: expires,
            duration_days: 30,
            min_bid_cents: 100,
            max_bid_cents: 1_000_000,
        })
    }
    pub async fn query_payment(
        &self,
        p: &str,
        secret: Option<&str>,
        v: &pb::PayDirectoryAdRequest,
    ) -> Result<Option<pb::DirectoryAdReceipt>> {
        if v.protocol_major != 1 || !key(&v.request_id) {
            return Err(Error::Invalid);
        }
        let mut t = self.begin().await?;
        let user = if let Some(s) = secret {
            Some(actor(&mut t, s).await?.user_id)
        } else {
            None
        };
        let row=sqlx::query("SELECT user_id,command,receipt FROM core_directory_ads.payments WHERE principal=$1 AND request_id=$2").bind(p).bind(&v.request_id).fetch_optional(&mut *t).await?;
        let out = if let Some(r) = row {
            if user.is_some_and(|u| r.get::<i64, _>("user_id") != u) {
                return Err(Error::Forbidden);
            }
            if r.try_get::<Value, _>("command")? != payment_command(v) {
                return Err(Error::Conflict);
            }
            Some(decode_receipt(r.try_get("receipt")?)?)
        } else {
            None
        };
        t.commit().await?;
        Ok(out)
    }
    pub async fn pay(
        &self,
        p: &str,
        secret: &str,
        v: &pb::PayDirectoryAdRequest,
    ) -> Result<pb::DirectoryAdReceipt> {
        if v.protocol_major != 1 || !key(&v.request_id) {
            return Err(Error::Invalid);
        }
        let mut t = self.begin().await?;
        let a = actor(&mut t, secret).await?;
        lock(&mut t, p, &v.request_id).await?;
        let cmd = payment_command(v);
        if let Some(r)=sqlx::query("SELECT user_id,command,receipt FROM core_directory_ads.payments WHERE principal=$1 AND request_id=$2").bind(p).bind(&v.request_id).fetch_optional(&mut *t).await? {if r.try_get::<i64,_>("user_id")?!=a.user_id{return Err(Error::Forbidden)}
            if r.try_get::<Value,_>("command")?!=cmd{return Err(Error::Conflict)}return decode_receipt(r.try_get("receipt")?)}
        let q=sqlx::query("SELECT * FROM core_directory_ads.quotes WHERE id=$1 AND principal=$2 AND request_id=$3 FOR UPDATE").bind(&v.quote_id).bind(p).bind(&v.request_id).fetch_optional(&mut *t).await?.ok_or(Error::Invalid)?;
        if q.try_get::<i64, _>("user_id")? != a.user_id
            || q.try_get::<i64, _>("account_id")? != a.owner_account_id
        {
            return Err(Error::Forbidden);
        }
        if q.try_get::<i64, _>("quota")? != v.expected_quota
            || q.try_get::<String, _>("content_sha256")? != v.content_sha256
        {
            return Err(Error::Conflict);
        }
        let at = now(&mut t).await?;
        if q.try_get::<i64, _>("expires_at")? < at {
            return Err(Error::Expired);
        }
        let sid = q
            .try_get::<Option<String>, _>("confirmed_sid")?
            .ok_or(Error::ConfirmationRequired)?;
        let approved:bool=sqlx::query_scalar("SELECT EXISTS(SELECT FROM core_identity.web_sessions WHERE sid=$1 AND user_id=$2 AND current_credential_id=$3 AND revoked_at IS NULL AND expires_at>clock_timestamp())").bind(sid).bind(a.user_id).bind(a.credential_id).fetch_one(&mut *t).await?;
        if !approved {
            return Err(Error::ConfirmationRequired);
        }
        identity::reject_marked_new_action(
            &mut t,
            a.user_id,
            identity::NewResourceAction::CommercePayment,
        )
        .await?;
        let consumption = format!("directory-ad:{}", v.quote_id);
        sqlx::query("SAVEPOINT ad_pay").execute(&mut *t).await?;
        sqlx::query("INSERT INTO core_charging.external_consumption(id,domain,actor_user_id,key_id,payer_account_id,authorized_at,amount_credits) VALUES($1,'directory_ad',$2,$3,$4,$5,$6)").bind(&consumption).bind(a.user_id).bind(a.credential_id).bind(a.owner_account_id).bind(at).bind(v.expected_quota).execute(&mut *t).await?;
        let budget = charging::external_budgets(&mut t, &consumption).await;
        let mut receipt = pb::DirectoryAdReceipt {
            request_id: v.request_id.clone(),
            command_sha256: hash(cmd.to_string().as_bytes()),
            payment_id: v.quote_id.clone(),
            ..Default::default()
        };
        let mut journal = None;
        match budget {
            Err(charging::Error::BudgetExceeded) => {
                receipt.rejection_code = "budget_exceeded".into()
            }
            Err(_) => return Err(Error::Retry),
            Ok(()) => {
                sqlx::query("SELECT core_billing.open_ledger_wallet($1)")
                    .bind(a.owner_account_id)
                    .execute(&mut *t)
                    .await?;
                let action = Action::Charge {
                    account_id: a.owner_account_id,
                    amount_units: Credits::try_from(v.expected_quota)
                        .map_err(|_| Error::Invalid)?,
                };
                match post(&mut t, p, &v.request_id, a.user_id, action).await? {
                    Outcome::Posted { journal_id, .. } => journal = Some(journal_id),
                    Outcome::Rejected { code } => {
                        receipt.rejection_code = serde_json::to_value(code)
                            .map_err(|_| Error::Storage)?
                            .as_str()
                            .ok_or(Error::Storage)?
                            .into()
                    }
                }
            }
        }
        if journal.is_none() {
            sqlx::query("ROLLBACK TO SAVEPOINT ad_pay")
                .execute(&mut *t)
                .await?;
            receipt.outcome = "rejected".into();
        } else {
            receipt.outcome = "succeeded".into();
            receipt.charged_quota = v.expected_quota;
            receipt.paid_at = at;
            receipt.ad_expires_at = at + 30 * 86400;
        }
        sqlx::query("INSERT INTO core_directory_ads.payments(id,principal,request_id,user_id,command,receipt,journal_id,paid_at,expires_at) VALUES($1,$2,$3,$4,$5,$6,$7,$8,$9)").bind(&v.quote_id).bind(p).bind(&v.request_id).bind(a.user_id).bind(cmd).bind(encode_receipt(&receipt)).bind(journal).bind(if journal.is_some(){Some(at)}else{None}).bind(if journal.is_some(){Some(receipt.ad_expires_at)}else{None}).execute(&mut *t).await?;
        t.commit().await?;
        Ok(receipt)
    }
    pub async fn query_refund(
        &self,
        p: &str,
        secret: Option<&str>,
        v: &pb::RefundDirectoryAdRequest,
    ) -> Result<Option<pb::DirectoryAdReceipt>> {
        if v.protocol_major != 1 || !key(&v.request_id) {
            return Err(Error::Invalid);
        }
        let mut t = self.begin().await?;
        if let Some(s) = secret {
            root(&mut t, s).await?;
        }
        let r=sqlx::query("SELECT command,receipt FROM core_directory_ads.refunds WHERE principal=$1 AND request_id=$2").bind(p).bind(&v.request_id).fetch_optional(&mut *t).await?;
        let out = if let Some(r) = r {
            if r.try_get::<Value, _>("command")? != refund_command(v) {
                return Err(Error::Conflict);
            }
            Some(decode_receipt(r.try_get("receipt")?)?)
        } else {
            None
        };
        t.commit().await?;
        Ok(out)
    }
    pub async fn refund(
        &self,
        p: &str,
        secret: &str,
        v: &pb::RefundDirectoryAdRequest,
    ) -> Result<pb::DirectoryAdReceipt> {
        if v.protocol_major != 1 || !key(&v.request_id) {
            return Err(Error::Invalid);
        }
        let mut t = self.begin().await?;
        let a = root(&mut t, secret).await?;
        lock(&mut t, p, &v.payment_id).await?;
        let cmd = refund_command(v);
        let old=sqlx::query("SELECT * FROM core_directory_ads.refunds WHERE principal=$1 AND request_id=$2 FOR UPDATE").bind(p).bind(&v.request_id).fetch_optional(&mut *t).await?;
        if let Some(r) = &old {
            if r.try_get::<Value, _>("command")? != cmd {
                return Err(Error::Conflict);
            }
            let receipt = decode_receipt(r.try_get("receipt")?)?;
            if receipt.outcome != "pending" {
                return Ok(receipt);
            }
        }
        let payment =
            sqlx::query("SELECT * FROM core_directory_ads.payments WHERE id=$1 AND principal=$2")
                .bind(&v.payment_id)
                .bind(p)
                .fetch_optional(&mut *t)
                .await?
                .ok_or(Error::Forbidden)?;
        let original = decode_receipt(payment.try_get("receipt")?)?;
        let journal = payment
            .try_get::<Option<i64>, _>("journal_id")?
            .ok_or(Error::Forbidden)?;
        let at = now(&mut t).await?;
        if old.is_none() {
            if original.ad_expires_at <= at {
                return Err(Error::Expired);
            }
            let exists: bool = sqlx::query_scalar(
                "SELECT EXISTS(SELECT FROM core_directory_ads.refunds WHERE payment_id=$1)",
            )
            .bind(&v.payment_id)
            .fetch_one(&mut *t)
            .await?;
            if exists {
                return Err(Error::Conflict);
            }
        }
        let mut receipt = pb::DirectoryAdReceipt {
            request_id: v.request_id.clone(),
            command_sha256: hash(cmd.to_string().as_bytes()),
            outcome: "pending".into(),
            payment_id: v.payment_id.clone(),
            charged_quota: original.charged_quota,
            paid_at: original.paid_at,
            ad_expires_at: original.ad_expires_at,
            ..Default::default()
        };
        if old.is_none() {
            sqlx::query("INSERT INTO core_directory_ads.refunds(principal,request_id,payment_id,command,accepted_at,accepted_by,receipt) VALUES($1,$2,$3,$4,$5,$6,$7)").bind(p).bind(&v.request_id).bind(&v.payment_id).bind(cmd).bind(at).bind(a.user_id).bind(encode_receipt(&receipt)).execute(&mut *t).await?;
        }
        sqlx::query("SAVEPOINT ad_refund").execute(&mut *t).await?;
        let outcome = post(
            &mut t,
            p,
            &format!("refund-{}", v.request_id),
            a.user_id,
            Action::Refund {
                journal_id: journal,
                amount_units: Credits::try_from(original.charged_quota)
                    .map_err(|_| Error::Invalid)?,
            },
        )
        .await?;
        match outcome {
            Outcome::Posted { journal_id, .. } => {
                receipt.outcome = "succeeded".into();
                receipt.refund_id = journal_id.to_string();
                receipt.refunded_quota = original.charged_quota;
                receipt.refunded_at = at;
                sqlx::query("UPDATE core_charging.external_consumption SET refunded=amount_credits WHERE id=$1").bind(format!("directory-ad:{}",v.payment_id)).execute(&mut *t).await?;
                sqlx::query("UPDATE core_directory_ads.refunds SET receipt=$3,journal_id=$4 WHERE principal=$1 AND request_id=$2").bind(p).bind(&v.request_id).bind(encode_receipt(&receipt)).bind(journal_id).execute(&mut *t).await?;
            }
            Outcome::Rejected { .. } => {
                sqlx::query("ROLLBACK TO SAVEPOINT ad_refund")
                    .execute(&mut *t)
                    .await?;
            }
        }
        t.commit().await?;
        Ok(receipt)
    }
}
async fn post(
    c: &mut PgConnection,
    p: &str,
    key: &str,
    user: i64,
    action: Action,
) -> Result<Outcome> {
    let request = LedgerRequest {
        scope: "core-directory-ads".into(),
        operation_key: hash(format!("{p}:{key}").as_bytes()),
        actor_user_id: Some(user),
        reason: "Directory advertisement".into(),
        action,
    };
    let Json(out) = sqlx::query_scalar("SELECT core_billing.post_ledger($1)")
        .bind(Json(request))
        .fetch_one(c)
        .await?;
    Ok(out)
}

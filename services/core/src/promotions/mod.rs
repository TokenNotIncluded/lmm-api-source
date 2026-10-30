//! Bounded Core promotional grants. No request selects an amount or recipient;
//! budget reservation, claim uniqueness and canonical Gift commit together.
mod http;
mod types;
use crate::{
    accounts::AccountKind,
    billing::{
        Credits,
        ledger::{Action, Outcome, Request as LedgerRequest},
    },
    identity::{self, CredentialKind, Principal},
};
use base64::{Engine, engine::general_purpose::URL_SAFE_NO_PAD};
use hmac::{Hmac, Mac};
pub use http::router;
use rand::{RngCore, rngs::OsRng};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use sqlx::{PgConnection, PgPool, Postgres, Row, Transaction, types::Json};
pub use types::*;

pub const SCHEMA: &str = include_str!("../../schema/promotions.sql");
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Error {
    Invalid,
    Unauthorized,
    Forbidden,
    NotFound,
    Conflict,
    Exhausted,
    Unavailable,
}
pub type Result<T> = std::result::Result<T, Error>;
impl From<sqlx::Error> for Error {
    fn from(e: sqlx::Error) -> Self {
        if e.as_database_error()
            .is_some_and(|e| matches!(e.code().as_deref(), Some("23505" | "P1001")))
        {
            Self::Conflict
        } else {
            Self::Unavailable
        }
    }
}
impl From<identity::IdentityError> for Error {
    fn from(e: identity::IdentityError) -> Self {
        match e {
            identity::IdentityError::Unauthorized => Self::Unauthorized,
            identity::IdentityError::Forbidden => Self::Forbidden,
            identity::IdentityError::Invalid => Self::Invalid,
            identity::IdentityError::Conflict => Self::Conflict,
            identity::IdentityError::Storage => Self::Unavailable,
        }
    }
}
#[derive(Clone)]
pub struct PromotionStore {
    pool: PgPool,
}
async fn actor(c: &mut PgConnection, secret: &str, admin: bool) -> Result<Principal> {
    let a = identity::authorize_in_transaction(c, secret).await?;
    if a.credential_kind != CredentialKind::Session
        || a.owner.kind != AccountKind::Personal
        || a.owner.id != a.user_id
    {
        return Err(Error::Forbidden);
    }
    if admin {
        let role: String = sqlx::query_scalar(
            "SELECT platform_role FROM core_identity.users WHERE id=$1 FOR SHARE",
        )
        .bind(a.user_id)
        .fetch_one(&mut *c)
        .await?;
        if role != "superadmin" {
            return Err(Error::Forbidden);
        }
        if identity::has_privileged_import_facts(c, a.user_id)
            .await
            .map_err(|_| Error::Unavailable)?
        {
            identity::load_complete_policy(c, a.user_id)
                .await
                .map_err(|_| Error::Forbidden)?;
        }
    }
    Ok(a)
}
async fn lock(c: &mut PgConnection, user: i64) -> Result<()> {
    sqlx::query("SELECT pg_advisory_xact_lock(hashtextextended('core-promotions:'||$1::text,0))")
        .bind(user)
        .execute(c)
        .await?;
    Ok(())
}
async fn now(c: &mut PgConnection) -> Result<i64> {
    Ok(
        sqlx::query_scalar("SELECT extract(epoch FROM clock_timestamp())::bigint")
            .fetch_one(c)
            .await?,
    )
}
fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|v| format!("{v:02x}")).collect()
}
fn random_id() -> String {
    let mut b = [0; 32];
    OsRng.fill_bytes(&mut b);
    hex(&b)
}
fn code_digest(value: &str) -> Result<Vec<u8>> {
    if value.len() != 48
        || !value.starts_with("lmmr_")
        || !value[5..]
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b"_-".contains(&b))
    {
        return Err(Error::Invalid);
    }
    Ok(Sha256::digest(value.as_bytes()).to_vec())
}
fn parse_hex(value: &str) -> Result<Vec<u8>> {
    id(value)?;
    (0..value.len())
        .step_by(2)
        .map(|i| u8::from_str_radix(&value[i..i + 2], 16).map_err(|_| Error::Invalid))
        .collect()
}
fn code(secret: &[u8], issue: &str, position: i32) -> Result<String> {
    let mut mac = Hmac::<Sha256>::new_from_slice(secret).map_err(|_| Error::Unavailable)?;
    mac.update(b"lmm-promotional-code-v1\0");
    mac.update(issue.as_bytes());
    mac.update(&position.to_be_bytes());
    Ok(format!(
        "lmmr_{}",
        URL_SAFE_NO_PAD.encode(mac.finalize().into_bytes())
    ))
}
async fn prior(
    c: &mut PgConnection,
    user: i64,
    key: &str,
    request: &Value,
) -> Result<Option<Value>> {
    let row:Option<(Value,Value)>=sqlx::query_as("SELECT request,result FROM core_promotions.commands WHERE actor_user_id=$1 AND operation_key=$2").bind(user).bind(key).fetch_optional(c).await?;
    match row {
        Some((old, result)) if old == *request => Ok(Some(result)),
        Some(_) => Err(Error::Conflict),
        None => Ok(None),
    }
}
async fn remember(
    c: &mut PgConnection,
    user: i64,
    key: &str,
    request: &Value,
    result: Value,
) -> Result<()> {
    sqlx::query("INSERT INTO core_promotions.commands(actor_user_id,operation_key,request,result) VALUES($1,$2,$3,$4)").bind(user).bind(key).bind(request).bind(result).execute(c).await?;
    Ok(())
}
fn decode<T: serde::de::DeserializeOwned>(v: Value) -> Result<T> {
    serde_json::from_value(v).map_err(|_| Error::Unavailable)
}
fn record(r: &sqlx::postgres::PgRow) -> Result<Campaign> {
    Ok(Campaign {
        id: r.try_get("id")?,
        revision: r.try_get("revision")?,
        spec: CampaignSpec {
            kind: r.try_get("kind")?,
            title: r.try_get("title")?,
            award_units: r.try_get::<i64, _>("award_units")?.to_string(),
            budget_units: r.try_get::<i64, _>("budget_units")?.to_string(),
            max_claims: r.try_get("max_claims")?,
            max_claims_per_user: r.try_get("max_claims_per_user")?,
            starts_at: r.try_get("starts_at")?,
            ends_at: r.try_get("ends_at")?,
            enabled: r.try_get("enabled")?,
        },
        spent_units: r.try_get::<i64, _>("spent_units")?.to_string(),
        reserved_units: r.try_get::<i64, _>("reserved_units")?.to_string(),
        claims: r.try_get("claims")?,
        reserved_claims: r.try_get("reserved_claims")?,
    })
}
async fn campaign(c: &mut PgConnection, value: &str) -> Result<Campaign> {
    let r = sqlx::query("SELECT * FROM core_promotions.campaigns WHERE id=$1 FOR UPDATE")
        .bind(value)
        .fetch_optional(c)
        .await?
        .ok_or(Error::NotFound)?;
    record(&r)
}
async fn issue(c: &mut PgConnection, value: &str) -> Result<Issue> {
    let (campaign, quantity): (String, i32) =
        sqlx::query_as("SELECT campaign_id,quantity FROM core_promotions.issues WHERE id=$1")
            .bind(value)
            .fetch_optional(&mut *c)
            .await?
            .ok_or(Error::NotFound)?;
    let secret: Vec<u8> =
        sqlx::query_scalar("SELECT code_key FROM core_promotions.secrets WHERE singleton")
            .fetch_one(c)
            .await?;
    let mut codes = Vec::with_capacity(quantity as usize);
    for i in 0..quantity {
        let code = code(&secret, value, i)?;
        codes.push(IssuedCode {
            id: hex(&code_digest(&code)?),
            code,
        });
    }
    Ok(Issue {
        id: value.into(),
        campaign_id: campaign,
        codes,
    })
}
fn receipt(r: &sqlx::postgres::PgRow) -> Result<Receipt> {
    Ok(Receipt {
        id: r.try_get("id")?,
        campaign_id: r.try_get("campaign_id")?,
        amount_units: r.try_get::<i64, _>("amount_units")?.to_string(),
        journal_id: r.try_get("journal_id")?,
        claimed_at: r.try_get("created_at")?,
    })
}
impl PromotionStore {
    pub fn new(pool: PgPool) -> Self {
        Self { pool }
    }
    async fn begin(&self) -> Result<Transaction<'_, Postgres>> {
        let mut tx = self.pool.begin().await?;
        for sql in [
            "SET TRANSACTION ISOLATION LEVEL SERIALIZABLE",
            "SET LOCAL statement_timeout='10s'",
            "SET LOCAL lock_timeout='1s'",
            "SET LOCAL synchronous_commit=on",
        ] {
            sqlx::query(sql).execute(&mut *tx).await?;
        }
        Ok(tx)
    }
    pub async fn campaigns(&self, secret: &str, admin: bool) -> Result<Vec<Campaign>> {
        let mut tx = self.begin().await?;
        actor(&mut tx, secret, admin).await?;
        let time = now(&mut tx).await?;
        let rows=sqlx::query("SELECT * FROM core_promotions.campaigns WHERE $1 OR (enabled AND starts_at<=$2 AND ends_at>$2) ORDER BY starts_at DESC,id LIMIT 1000").bind(admin).bind(time).fetch_all(&mut *tx).await?;
        let result = rows.iter().map(record).collect::<Result<Vec<_>>>()?;
        tx.commit().await?;
        Ok(result)
    }
    pub async fn save_campaign(
        &self,
        secret: &str,
        campaign_id: Option<&str>,
        input: SaveCampaign,
    ) -> Result<Campaign> {
        key(&input.operation_key)?;
        input.spec.validate()?;
        if let Some(value) = campaign_id {
            id(value)?;
        }
        if campaign_id.is_some() != input.expected_revision.is_some()
            || input.expected_revision.is_some_and(|n| n < 1)
        {
            return Err(Error::Invalid);
        }
        let mut tx = self.begin().await?;
        let a = actor(&mut tx, secret, true).await?;
        lock(&mut tx, a.user_id).await?;
        let request = json!(["campaign", campaign_id, input]);
        if let Some(result) = prior(&mut tx, a.user_id, &input.operation_key, &request).await? {
            let r = decode(result)?;
            tx.commit().await?;
            return Ok(r);
        }
        let award = units(&input.spec.award_units)?;
        let budget = units(&input.spec.budget_units)?;
        let value = if let Some(value) = campaign_id {
            let old = campaign(&mut tx, value).await?;
            if Some(old.revision) != input.expected_revision
                || old.spec.kind != input.spec.kind
                || old.spec.award_units != input.spec.award_units
            {
                return Err(Error::Conflict);
            }
            let allocated = old
                .spent_units
                .parse::<i64>()
                .map_err(|_| Error::Unavailable)?
                + old
                    .reserved_units
                    .parse::<i64>()
                    .map_err(|_| Error::Unavailable)?;
            if allocated > budget || old.claims + old.reserved_claims > input.spec.max_claims {
                return Err(Error::Conflict);
            }
            sqlx::query("UPDATE core_promotions.campaigns SET revision=revision+1,title=$2,budget_units=$3,max_claims=$4,max_claims_per_user=$5,starts_at=$6,ends_at=$7,enabled=$8 WHERE id=$1")
                .bind(value).bind(&input.spec.title).bind(budget).bind(input.spec.max_claims).bind(input.spec.max_claims_per_user).bind(input.spec.starts_at).bind(input.spec.ends_at).bind(input.spec.enabled).execute(&mut *tx).await?;
            value.to_owned()
        } else {
            let value = random_id();
            sqlx::query("INSERT INTO core_promotions.campaigns(id,revision,kind,title,award_units,budget_units,max_claims,max_claims_per_user,starts_at,ends_at,enabled) VALUES($1,1,$2,$3,$4,$5,$6,$7,$8,$9,$10)")
                .bind(&value).bind(&input.spec.kind).bind(&input.spec.title).bind(award).bind(budget).bind(input.spec.max_claims).bind(input.spec.max_claims_per_user).bind(input.spec.starts_at).bind(input.spec.ends_at).bind(input.spec.enabled).execute(&mut *tx).await?;
            value
        };
        let result = campaign(&mut tx, &value).await?;
        remember(
            &mut tx,
            a.user_id,
            &input.operation_key,
            &request,
            json!(&result),
        )
        .await?;
        identity::audit(
            &mut tx,
            &a,
            "promotion.campaign.save",
            json!({"campaign_id":value,"revision":result.revision}),
        )
        .await?;
        tx.commit().await?;
        Ok(result)
    }
    pub async fn issue_codes(
        &self,
        secret: &str,
        campaign_id: &str,
        input: IssueRequest,
    ) -> Result<Issue> {
        id(campaign_id)?;
        key(&input.operation_key)?;
        if !(1..=100).contains(&input.quantity) {
            return Err(Error::Invalid);
        }
        let mut tx = self.begin().await?;
        let a = actor(&mut tx, secret, true).await?;
        lock(&mut tx, a.user_id).await?;
        let request = json!(["issue", campaign_id, input]);
        if let Some(saved) = prior(&mut tx, a.user_id, &input.operation_key, &request).await? {
            let result = issue(&mut tx, saved.as_str().ok_or(Error::Unavailable)?).await?;
            tx.commit().await?;
            return Ok(result);
        }
        let c = campaign(&mut tx, campaign_id).await?;
        let time = now(&mut tx).await?;
        if c.spec.kind != "redemption" || !c.spec.enabled || c.spec.ends_at <= time {
            return Err(Error::Conflict);
        }
        let amount = units(&c.spec.award_units)?
            .checked_mul(i64::from(input.quantity))
            .ok_or(Error::Invalid)?;
        let spent = c
            .spent_units
            .parse::<i64>()
            .map_err(|_| Error::Unavailable)?;
        let reserved = c
            .reserved_units
            .parse::<i64>()
            .map_err(|_| Error::Unavailable)?;
        if spent + reserved + amount > units(&c.spec.budget_units)?
            || c.claims + c.reserved_claims + i64::from(input.quantity) > c.spec.max_claims
        {
            return Err(Error::Exhausted);
        }
        let issue_id = random_id();
        sqlx::query("INSERT INTO core_promotions.issues(id,campaign_id,actor_user_id,quantity,created_at) VALUES($1,$2,$3,$4,$5)").bind(&issue_id).bind(campaign_id).bind(a.user_id).bind(input.quantity).bind(time).execute(&mut *tx).await?;
        let result = issue(&mut tx, &issue_id).await?;
        for (position, item) in result.codes.iter().enumerate() {
            let position = i32::try_from(position).map_err(|_| Error::Unavailable)?;
            sqlx::query("INSERT INTO core_promotions.codes(digest,issue_id,position,campaign_id) VALUES($1,$2,$3,$4)").bind(code_digest(&item.code)?).bind(&issue_id).bind(position).bind(campaign_id).execute(&mut *tx).await?;
        }
        sqlx::query("UPDATE core_promotions.campaigns SET reserved_units=reserved_units+$2,reserved_claims=reserved_claims+$3 WHERE id=$1").bind(campaign_id).bind(amount).bind(i64::from(input.quantity)).execute(&mut *tx).await?;
        // Persist only the issue identity, never the plaintext redemption codes.
        remember(
            &mut tx,
            a.user_id,
            &input.operation_key,
            &request,
            json!(issue_id),
        )
        .await?;
        identity::audit(
            &mut tx,
            &a,
            "promotion.codes.issue",
            json!({"campaign_id":campaign_id,"issue_id":result.id,"quantity":input.quantity}),
        )
        .await?;
        tx.commit().await?;
        Ok(result)
    }
    pub async fn codes(&self, secret: &str, campaign_id: &str) -> Result<Vec<CodeStatus>> {
        id(campaign_id)?;
        let mut tx = self.begin().await?;
        actor(&mut tx, secret, true).await?;
        let rows=sqlx::query("SELECT encode(c.digest,'hex') AS id,c.revoked,EXISTS(SELECT 1 FROM core_promotions.claims l WHERE l.code_digest=c.digest) AS claimed FROM core_promotions.codes c WHERE c.campaign_id=$1 ORDER BY c.issue_id DESC,c.position LIMIT 1000").bind(campaign_id).fetch_all(&mut *tx).await?;
        let result = rows
            .iter()
            .map(|r| {
                Ok(CodeStatus {
                    id: r.try_get("id")?,
                    revoked: r.try_get("revoked")?,
                    claimed: r.try_get("claimed")?,
                })
            })
            .collect::<Result<Vec<_>>>()?;
        tx.commit().await?;
        Ok(result)
    }
    pub async fn revoke_code(&self, secret: &str, code_id: &str) -> Result<CodeStatus> {
        let digest = parse_hex(code_id)?;
        let mut tx = self.begin().await?;
        let a = actor(&mut tx, secret, true).await?;
        lock(&mut tx, a.user_id).await?;
        let campaign_id: String =
            sqlx::query_scalar("SELECT campaign_id FROM core_promotions.codes WHERE digest=$1")
                .bind(&digest)
                .fetch_optional(&mut *tx)
                .await?
                .ok_or(Error::NotFound)?;
        let c = campaign(&mut tx, &campaign_id).await?;
        let (revoked,claimed):(bool,bool)=sqlx::query_as("SELECT revoked,EXISTS(SELECT 1 FROM core_promotions.claims WHERE code_digest=$1) FROM core_promotions.codes WHERE digest=$1 FOR UPDATE").bind(&digest).fetch_one(&mut *tx).await?;
        if claimed {
            return Err(Error::Conflict);
        }
        if !revoked {
            sqlx::query("UPDATE core_promotions.codes SET revoked=true WHERE digest=$1")
                .bind(&digest)
                .execute(&mut *tx)
                .await?;
            sqlx::query("UPDATE core_promotions.campaigns SET reserved_units=reserved_units-$2,reserved_claims=reserved_claims-1 WHERE id=$1").bind(&campaign_id).bind(units(&c.spec.award_units)?).execute(&mut *tx).await?;
            identity::audit(
                &mut tx,
                &a,
                "promotion.code.revoke",
                json!({"campaign_id":campaign_id,"code_id":code_id}),
            )
            .await?;
        }
        tx.commit().await?;
        Ok(CodeStatus {
            id: code_id.into(),
            revoked: true,
            claimed: false,
        })
    }
    pub async fn redeem(&self, secret: &str, input: RedeemRequest) -> Result<Receipt> {
        let digest = code_digest(&input.code)?;
        self.grant(
            secret,
            &input.operation_key,
            "redemption",
            &hex(&digest),
            Some(digest),
        )
        .await
    }
    pub async fn checkin(
        &self,
        secret: &str,
        campaign_id: &str,
        input: ClaimRequest,
    ) -> Result<Receipt> {
        id(campaign_id)?;
        self.grant(secret, &input.operation_key, "checkin", campaign_id, None)
            .await
    }
    pub async fn envelope(
        &self,
        secret: &str,
        campaign_id: &str,
        input: ClaimRequest,
    ) -> Result<Receipt> {
        id(campaign_id)?;
        self.grant(secret, &input.operation_key, "envelope", campaign_id, None)
            .await
    }
    async fn grant(
        &self,
        secret: &str,
        operation: &str,
        kind: &str,
        target: &str,
        digest: Option<Vec<u8>>,
    ) -> Result<Receipt> {
        key(operation)?;
        let mut tx = self.begin().await?;
        let a = actor(&mut tx, secret, false).await?;
        lock(&mut tx, a.user_id).await?;
        let request = json!([kind, target]);
        if let Some(saved) = prior(&mut tx, a.user_id, operation, &request).await? {
            let result = decode(saved)?;
            tx.commit().await?;
            return Ok(result);
        }
        let campaign_id = if let Some(digest) = &digest {
            sqlx::query_scalar("SELECT campaign_id FROM core_promotions.codes WHERE digest=$1")
                .bind(digest)
                .fetch_optional(&mut *tx)
                .await?
                .ok_or(Error::NotFound)?
        } else {
            target.to_owned()
        };
        let c = campaign(&mut tx, &campaign_id).await?;
        let time = now(&mut tx).await?;
        if c.spec.kind != kind {
            return Err(Error::Conflict);
        }
        let subject = match kind {
            "redemption" => target.to_owned(),
            "checkin" => format!("{}:{}", a.user_id, time / 86400),
            "envelope" => a.user_id.to_string(),
            _ => return Err(Error::Invalid),
        };
        if let Some(row) = sqlx::query(
            "SELECT * FROM core_promotions.claims WHERE campaign_id=$1 AND subject_key=$2",
        )
        .bind(&campaign_id)
        .bind(&subject)
        .fetch_optional(&mut *tx)
        .await?
        {
            if row.try_get::<i64, _>("actor_user_id")? != a.user_id {
                return Err(Error::Conflict);
            }
            let result = receipt(&row)?;
            remember(&mut tx, a.user_id, operation, &request, json!(&result)).await?;
            tx.commit().await?;
            return Ok(result);
        }
        if !c.spec.enabled || time < c.spec.starts_at || time >= c.spec.ends_at {
            return Err(Error::Conflict);
        }
        if crate::abuse::is_marked_in_transaction(&mut tx, a.user_id)
            .await
            .map_err(|_| Error::Unavailable)?
        {
            return Err(Error::Forbidden);
        }
        if let Some(digest) = &digest {
            let revoked: bool = sqlx::query_scalar(
                "SELECT revoked FROM core_promotions.codes WHERE digest=$1 FOR UPDATE",
            )
            .bind(digest)
            .fetch_one(&mut *tx)
            .await?;
            if revoked {
                return Err(Error::Conflict);
            }
        }
        let count: i64 = sqlx::query_scalar(
            "SELECT count(*) FROM core_promotions.claims WHERE campaign_id=$1 AND actor_user_id=$2",
        )
        .bind(&campaign_id)
        .bind(a.user_id)
        .fetch_one(&mut *tx)
        .await?;
        if count >= c.spec.max_claims_per_user {
            return Err(Error::Exhausted);
        }
        let amount = units(&c.spec.award_units)?;
        if digest.is_none() {
            let spent = c
                .spent_units
                .parse::<i64>()
                .map_err(|_| Error::Unavailable)?;
            let reserved = c
                .reserved_units
                .parse::<i64>()
                .map_err(|_| Error::Unavailable)?;
            if spent + reserved + amount > units(&c.spec.budget_units)?
                || c.claims + c.reserved_claims >= c.spec.max_claims
            {
                return Err(Error::Exhausted);
            }
        }
        crate::financial_profile::read_wallet(&mut tx, a.user_id)
            .await
            .map_err(|_| Error::Unavailable)?;
        sqlx::query("SELECT core_billing.open_ledger_wallet($1)")
            .bind(a.owner_account_id)
            .execute(&mut *tx)
            .await?;
        let claim_id = random_id();
        let ledger = LedgerRequest {
            scope: "core-promotion-rewards".into(),
            operation_key: format!("grant:{claim_id}"),
            actor_user_id: Some(a.user_id),
            reason: format!("Promotional {kind} campaign {campaign_id}"),
            action: Action::Gift {
                account_id: a.owner_account_id,
                amount_units: Credits::try_from(amount).map_err(|_| Error::Invalid)?,
            },
        };
        ledger.validate().map_err(|_| Error::Invalid)?;
        let Json(outcome): Json<Outcome> =
            sqlx::query_scalar("SELECT core_billing.post_ledger($1)")
                .bind(Json(ledger))
                .fetch_one(&mut *tx)
                .await?;
        let Outcome::Posted { journal_id, .. } = outcome else {
            return Err(Error::Conflict);
        };
        sqlx::query("INSERT INTO core_promotions.claims(id,campaign_id,subject_key,actor_user_id,account_id,amount_units,code_digest,journal_id,created_at) VALUES($1,$2,$3,$4,$5,$6,$7,$8,$9)")
            .bind(&claim_id).bind(&campaign_id).bind(subject).bind(a.user_id).bind(a.owner_account_id).bind(amount).bind(&digest).bind(journal_id).bind(time).execute(&mut *tx).await?;
        sqlx::query("UPDATE core_promotions.campaigns SET spent_units=spent_units+$2,claims=claims+1,reserved_units=reserved_units-CASE WHEN $3 THEN $2 ELSE 0 END,reserved_claims=reserved_claims-CASE WHEN $3 THEN 1 ELSE 0 END WHERE id=$1")
            .bind(&campaign_id).bind(amount).bind(digest.is_some()).execute(&mut *tx).await?;
        let result = Receipt {
            id: claim_id,
            campaign_id,
            amount_units: amount.to_string(),
            journal_id,
            claimed_at: time,
        };
        remember(&mut tx, a.user_id, operation, &request, json!(&result)).await?;
        identity::audit(&mut tx, &a, "promotion.claim", json!(&result)).await?;
        tx.commit().await?;
        Ok(result)
    }
    pub async fn history(&self, secret: &str) -> Result<Vec<Receipt>> {
        let mut tx = self.begin().await?;
        let a = actor(&mut tx, secret, false).await?;
        let rows=sqlx::query("SELECT * FROM core_promotions.claims WHERE actor_user_id=$1 ORDER BY created_at DESC,id LIMIT 1000").bind(a.user_id).fetch_all(&mut *tx).await?;
        let result = rows.iter().map(receipt).collect::<Result<Vec<_>>>()?;
        tx.commit().await?;
        Ok(result)
    }
}

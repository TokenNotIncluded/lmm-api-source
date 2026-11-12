//! A purpose-bound promotional funding capability, not a business-policy store.
//! Core owns live principals, monetary ceilings and immutable daily posting keys.
use crate::{
    accounts::AccountKind,
    billing::{
        Credits,
        ledger::{Action, Outcome, Request as Posting},
    },
    identity::{self, CredentialKind, Principal},
    internal_rpc::pb,
};
use chrono::{DateTime, Utc};
use sha2::{Digest, Sha256};
use sqlx::{PgConnection, PgPool, Row, types::Json};
use subtle::ConstantTimeEq;
use tonic::{Request, Response, Status, metadata::MetadataMap};
#[derive(Clone)]
pub struct CheckinLedger {
    pool: PgPool,
    digest: [u8; 32],
    caps: [i64; 5],
}
fn unavailable(_: impl std::fmt::Display) -> Status {
    Status::unavailable("funding outcome unconfirmed; retry original daily intention")
}
fn single<'a>(m: &'a MetadataMap, key: &'static str) -> Result<&'a str, Status> {
    let mut values = m.get_all(key).iter();
    let value = values
        .next()
        .ok_or_else(|| Status::unauthenticated("missing funding credential"))?;
    if values.next().is_some() {
        return Err(Status::unauthenticated("duplicate funding credential"));
    };
    value
        .to_str()
        .map_err(|_| Status::unauthenticated("invalid funding credential"))
}
impl CheckinLedger {
    pub fn new(pool: PgPool, token: &[u8], caps: [i64; 5]) -> Result<Self, Status> {
        if !(32..=256).contains(&token.len())
            || !token.iter().all(u8::is_ascii_graphic)
            || caps.iter().any(|v| *v < 0)
        {
            return Err(Status::invalid_argument("invalid funding authority"));
        };
        Ok(Self {
            pool,
            digest: Sha256::digest(token).into(),
            caps,
        })
    }
    fn credential<'a>(&self, m: &'a MetadataMap) -> Result<&'a str, Status> {
        let token = single(m, "authorization")?
            .strip_prefix("Bearer ")
            .ok_or_else(|| Status::unauthenticated("invalid funding service credential"))?;
        let digest: [u8; 32] = Sha256::digest(token.as_bytes()).into();
        if !bool::from(self.digest.ct_eq(&digest)) {
            return Err(Status::unauthenticated(
                "invalid funding service credential",
            ));
        };
        if single(m, "x-lmm-protocol")? != "1" {
            return Err(Status::failed_precondition("unsupported protocol"));
        };
        single(m, "x-lmm-user-credential")
    }
    async fn actor(c: &mut PgConnection, secret: &str) -> Result<Principal, Status> {
        let a = identity::authorize_in_transaction(c, secret)
            .await
            .map_err(|e| match e {
                identity::IdentityError::Unauthorized => {
                    Status::unauthenticated("invalid current user credential")
                }
                identity::IdentityError::Forbidden => {
                    Status::permission_denied("current principal denied")
                }
                _ => unavailable(e),
            })?;
        if a.credential_kind != CredentialKind::Session
            || a.owner.kind != AccountKind::Personal
            || a.owner.id != a.user_id
        {
            return Err(Status::permission_denied("personal session required"));
        };
        let bound:bool=sqlx::query_scalar("SELECT EXISTS(SELECT FROM core_identity.users u JOIN core_identity.accounts a ON a.id=u.personal_account_id WHERE u.id=$1 AND u.personal_account_id=$2 AND a.kind='personal' AND u.active AND a.active)").bind(a.user_id).bind(a.owner_account_id).fetch_one(c).await.map_err(unavailable)?;
        if !bound {
            return Err(Status::permission_denied("personal account binding denied"));
        };
        Ok(a)
    }
    async fn context(
        &self,
        c: &mut PgConnection,
        a: &Principal,
    ) -> Result<pb::CheckinLedgerContext, Status> {
        let timestamp: i64 =
            sqlx::query_scalar("SELECT floor(extract(epoch FROM transaction_timestamp()))::bigint")
                .fetch_one(&mut *c)
                .await
                .map_err(unavailable)?;
        let day = DateTime::<Utc>::from_timestamp(
            timestamp
                .checked_add(28800)
                .ok_or_else(|| Status::internal("invalid funding clock"))?,
            0,
        )
        .ok_or_else(|| Status::internal("invalid funding clock"))?
        .format("%Y-%m-%d")
        .to_string();
        let blocked = crate::abuse::is_marked_in_transaction(c, a.user_id)
            .await
            .map_err(unavailable)?;
        let authority = crate::billing::trust_pricing::quote_in_transaction(c, a.user_id)
            .await
            .map_err(unavailable)?;
        let level = u32::from(authority.evidence().level());
        Ok(pb::CheckinLedgerContext {
            user_id: a.user_id,
            account_id: a.owner_account_id,
            day,
            timestamp,
            trust_level: level,
            max_daily_units: self.caps[level.min(4) as usize],
            blocked,
        })
    }
    async fn receipt(
        c: &mut PgConnection,
        a: &Principal,
        date: &str,
        amount: i64,
        journal: Option<i64>,
        replayed: bool,
    ) -> Result<pb::CheckinLedgerReceipt, Status> {
        let timestamp = if let Some(id) = journal {
            sqlx::query_scalar("SELECT floor(extract(epoch FROM created_at))::bigint FROM core_billing.ledger_journals WHERE id=$1").bind(id).fetch_one(c).await.map_err(unavailable)?
        } else {
            0
        };
        Ok(pb::CheckinLedgerReceipt {
            user_id: a.user_id,
            account_id: a.owner_account_id,
            day: date.into(),
            amount_units: amount,
            journal_id: journal,
            timestamp,
            replayed,
        })
    }
}
#[tonic::async_trait]
impl pb::core_checkin_ledger_server::CoreCheckinLedger for CheckinLedger {
    async fn inspect(
        &self,
        r: Request<pb::CheckinLedgerInspect>,
    ) -> Result<Response<pb::CheckinLedgerContext>, Status> {
        let user = self.credential(r.metadata())?;
        let mut tx = self.pool.begin().await.map_err(unavailable)?;
        sqlx::query("SET TRANSACTION ISOLATION LEVEL SERIALIZABLE")
            .execute(&mut *tx)
            .await
            .map_err(unavailable)?;
        let a = Self::actor(&mut tx, user).await?;
        let result = self.context(&mut tx, &a).await?;
        tx.commit().await.map_err(unavailable)?;
        Ok(Response::new(result))
    }
    async fn post(
        &self,
        r: Request<pb::CheckinLedgerPost>,
    ) -> Result<Response<pb::CheckinLedgerReceipt>, Status> {
        let user = self.credential(r.metadata())?;
        let input = r.get_ref();
        if input.amount_units < 0
            || input.day.len() != 10
            || chrono::NaiveDate::parse_from_str(&input.day, "%Y-%m-%d")
                .ok()
                .is_none_or(|d| d.to_string() != input.day)
        {
            return Err(Status::invalid_argument("invalid funding intention"));
        };
        let mut tx = self.pool.begin().await.map_err(unavailable)?;
        sqlx::query("SET TRANSACTION ISOLATION LEVEL SERIALIZABLE")
            .execute(&mut *tx)
            .await
            .map_err(unavailable)?;
        sqlx::query("SET LOCAL synchronous_commit=on")
            .execute(&mut *tx)
            .await
            .map_err(unavailable)?;
        let a = Self::actor(&mut tx, user).await?;
        sqlx::query("SELECT pg_advisory_xact_lock(hashtextextended('core-checkins:'||$1::text,0))")
            .bind(a.owner_account_id)
            .execute(&mut *tx)
            .await
            .map_err(unavailable)?;
        let key = format!("daily:{}:{}", a.owner_account_id, input.day);
        let request = Posting {
            scope: "core-checkins".into(),
            operation_key: key.clone(),
            actor_user_id: Some(a.user_id),
            reason: "Daily check-in reward".into(),
            action: Action::Gift {
                account_id: a.owner_account_id,
                amount_units: Credits::try_from(input.amount_units).map_err(unavailable)?,
            },
        };
        // Authenticate first, then replay an immutable result before changed ceilings,
        // trust, abuse status or clock. Never execute a second credit for an old day.
        if let Some(row)=sqlx::query("SELECT request,result FROM core_billing.ledger_operations WHERE scope='core-checkins' AND operation_key=$1").bind(&key).fetch_optional(&mut *tx).await.map_err(unavailable)?{let prior:Json<Posting>=row.try_get("request").map_err(unavailable)?;if prior.0!=request{return Err(Status::already_exists("daily funding intention conflict"))};let result:Json<Outcome>=row.try_get("result").map_err(unavailable)?;let Outcome::Posted{journal_id,..}=result.0 else{return Err(Status::already_exists("daily funding rejected"))};let receipt=Self::receipt(&mut tx,&a,&input.day,input.amount_units,Some(journal_id),true).await?;tx.commit().await.map_err(unavailable)?;return Ok(Response::new(receipt))}
        let authority = self.context(&mut tx, &a).await?;
        if authority.blocked
            || authority.trust_level != input.trust_level
            || input.amount_units > authority.max_daily_units
        {
            return Err(Status::permission_denied("funding authority denied"));
        };
        if input.day != authority.day && input.amount_units != 0 {
            return Err(Status::failed_precondition(
                "funding daily authorization expired",
            ));
        }
        let journal = if input.amount_units == 0 {
            None
        } else {
            crate::financial_profile::read_wallet(&mut tx, a.user_id)
                .await
                .map_err(unavailable)?;
            sqlx::query("SELECT core_billing.open_ledger_wallet($1)")
                .bind(a.owner_account_id)
                .execute(&mut *tx)
                .await
                .map_err(unavailable)?;
            request.validate().map_err(unavailable)?;
            let Json(outcome): Json<Outcome> =
                sqlx::query_scalar("SELECT core_billing.post_ledger($1)")
                    .bind(Json(request))
                    .fetch_one(&mut *tx)
                    .await
                    .map_err(unavailable)?;
            match outcome {
                Outcome::Posted { journal_id, .. } => Some(journal_id),
                _ => return Err(Status::already_exists("daily funding rejected")),
            }
        };
        let mut receipt =
            Self::receipt(&mut tx, &a, &input.day, input.amount_units, journal, false).await?;
        if journal.is_none() {
            receipt.timestamp = authority.timestamp
        };
        tx.commit().await.map_err(unavailable)?;
        Ok(Response::new(receipt))
    }
}

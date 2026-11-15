//! Short-lived check-in authorization owned by Core, independent of Go records.
use crate::{
    config::turnstile_config::{Turnstile, VerifyError},
    identity::{IdentityError, PasskeyConfig, Principal},
    internal_rpc::pb,
};
use rand::{TryRng as _, rngs::SysRng};
use serde_json::Value;
use sha2::{Digest, Sha256};
use sqlx::{PgConnection, Row, types::Json};
use tonic::Status;

pub const SCHEMA: &str = include_str!("../schema/checkin_challenges.sql");
const ACTION: &str = "daily_checkin";

#[derive(Clone)]
pub struct ChallengeConfig {
    turnstile: Turnstile,
    passkey: Option<PasskeyConfig>,
    require_passkey: bool,
}

pub(crate) fn required() -> Status {
    Status::failed_precondition("CHECKIN_VERIFICATION_REQUIRED")
}
fn failed() -> Status {
    Status::permission_denied("CHECKIN_VERIFICATION_FAILED")
}
fn unavailable(_: impl std::fmt::Display) -> Status {
    Status::unavailable("CHECKIN_VERIFICATION_UNAVAILABLE")
}
fn identity_error(error: IdentityError) -> Status {
    match error {
        IdentityError::Storage => unavailable(error),
        _ => failed(),
    }
}
pub(crate) fn valid_operation(value: &str) -> bool {
    (16..=80).contains(&value.len())
        && value
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'_' | b'-' | b':' | b'.'))
}
fn challenge_digest(value: &str) -> Result<Vec<u8>, Status> {
    if value.len() != 64
        || !value
            .bytes()
            .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
    {
        return Err(required());
    }
    Ok(Sha256::digest(value.as_bytes()).to_vec())
}
async fn session(c: &mut PgConnection, actor: &Principal) -> Result<String, Status> {
    // Refresh locks the family before rotating its current credential. Actor
    // authorization locks the credential first, so never wait in reverse order.
    sqlx::query_scalar("SELECT sid FROM core_identity.web_sessions WHERE current_credential_id=$1 AND user_id=$2 AND user_version=$3 AND revoked_at IS NULL AND expires_at>clock_timestamp() AND (NOT weekly_logout OR created_at>clock_timestamp()-interval '7 days') FOR SHARE NOWAIT")
        .bind(actor.credential_id).bind(actor.user_id).bind(actor.credential_generation())
        .fetch_optional(c).await.map_err(unavailable)?.ok_or_else(|| Status::unauthenticated("personal browser session required"))
}

impl ChallengeConfig {
    pub fn new(
        turnstile: Turnstile,
        passkey: Option<PasskeyConfig>,
        require_passkey: bool,
    ) -> Result<Self, Status> {
        if require_passkey && passkey.is_none() {
            return Err(Status::failed_precondition(
                "check-in Passkey configuration required",
            ));
        }
        Ok(Self {
            turnstile,
            passkey,
            require_passkey,
        })
    }

    pub(crate) async fn begin(
        &self,
        c: &mut PgConnection,
        actor: &Principal,
        day: &str,
        operation: &str,
    ) -> Result<pb::CheckinChallenge, Status> {
        if !valid_operation(operation) {
            return Err(Status::invalid_argument("invalid checkin operation"));
        }
        let sid = session(c, actor).await?;
        sqlx::query(
            "SELECT pg_advisory_xact_lock(hashtextextended('core-checkin-challenge:'||$1::text,0))",
        )
        .bind(actor.user_id)
        .execute(&mut *c)
        .await
        .map_err(unavailable)?;
        let recent: i64 = sqlx::query_scalar("SELECT count(*) FROM core_checkin_challenges.challenges WHERE user_id=$1 AND created_at>clock_timestamp()-interval '1 minute'")
            .bind(actor.user_id).fetch_one(&mut *c).await.map_err(unavailable)?;
        if recent >= 5 {
            return Err(Status::resource_exhausted("CHECKIN_RATE_LIMITED"));
        }
        let (options, state) = if self.require_passkey {
            let config = self
                .passkey
                .as_ref()
                .ok_or_else(|| unavailable("missing Passkey config"))?;
            let (options, state) = config
                .begin_checkin(c, actor.user_id)
                .await
                .map_err(identity_error)?
                .ok_or_else(|| Status::failed_precondition("CHECKIN_PASSKEY_REQUIRED"))?;
            (
                serde_json::to_string(&options).map_err(unavailable)?,
                Some(Json(state)),
            )
        } else {
            (String::new(), None)
        };
        let mut random = [0; 32];
        SysRng.try_fill_bytes(&mut random).map_err(unavailable)?;
        let token = crate::hex_codec::encode(random);
        let expires_at: i64 = sqlx::query_scalar("INSERT INTO core_checkin_challenges.challenges(digest,user_id,account_id,sid,user_version,operation_key,day,passkey_required,passkey_state) VALUES($1,$2,$3,$4,$5,$6,$7::date,$8,$9) RETURNING floor(extract(epoch FROM expires_at))::bigint")
            .bind(challenge_digest(&token)?).bind(actor.user_id).bind(actor.owner_account_id).bind(sid)
            .bind(actor.credential_generation()).bind(operation).bind(day).bind(self.require_passkey).bind(state)
            .fetch_one(&mut *c).await.map_err(unavailable)?;
        sqlx::query("DELETE FROM core_checkin_challenges.challenges WHERE digest IN (SELECT digest FROM core_checkin_challenges.challenges WHERE expires_at<=clock_timestamp() ORDER BY expires_at LIMIT 100 FOR UPDATE SKIP LOCKED)")
            .execute(c).await.map_err(unavailable)?;
        Ok(pb::CheckinChallenge {
            challenge_id: token.clone(),
            operation_key: operation.into(),
            day: day.into(),
            expires_at,
            turnstile_site_key: self.turnstile.site_key().into(),
            turnstile_action: ACTION.into(),
            turnstile_cdata: token,
            passkey_required: self.require_passkey,
            passkey_options_json: options,
        })
    }

    async fn bound_row(
        &self,
        c: &mut PgConnection,
        actor: &Principal,
        day: &str,
        operation: &str,
        id: &str,
    ) -> Result<sqlx::postgres::PgRow, Status> {
        if !valid_operation(operation) {
            return Err(required());
        }
        let sid = session(c, actor).await?;
        sqlx::query("SELECT day::text,passkey_state,attempts,verified_at IS NOT NULL AS verified,consumed_at IS NOT NULL AS consumed,floor(extract(epoch FROM expires_at))::bigint AS expires_at FROM core_checkin_challenges.challenges WHERE digest=$1 AND user_id=$2 AND account_id=$3 AND sid=$4 AND user_version=$5 AND operation_key=$6 AND day=$7::date AND passkey_required=$8 AND expires_at>clock_timestamp() FOR UPDATE")
            .bind(challenge_digest(id)?).bind(actor.user_id).bind(actor.owner_account_id).bind(sid)
            .bind(actor.credential_generation()).bind(operation).bind(day).bind(self.require_passkey)
            .fetch_optional(c).await.map_err(unavailable)?.ok_or_else(required)
    }

    /// Count the attempt before the external call, without holding database locks.
    pub(crate) async fn prepare(
        &self,
        c: &mut PgConnection,
        actor: &Principal,
        day: &str,
        input: &pb::CheckinChallengeAuthorize,
    ) -> Result<Option<pb::CheckinAuthorization>, Status> {
        let row = self
            .bound_row(c, actor, day, &input.operation_key, &input.challenge_id)
            .await?;
        if row.try_get::<bool, _>("consumed").map_err(unavailable)? {
            return Err(required());
        }
        if row.try_get::<bool, _>("verified").map_err(unavailable)? {
            return Ok(Some(pb::CheckinAuthorization {
                authorization_id: input.challenge_id.clone(),
                day: day.into(),
                expires_at: row.try_get("expires_at").map_err(unavailable)?,
            }));
        }
        if row.try_get::<i16, _>("attempts").map_err(unavailable)? >= 3 {
            return Err(Status::resource_exhausted("CHECKIN_RATE_LIMITED"));
        }
        sqlx::query(
            "UPDATE core_checkin_challenges.challenges SET attempts=attempts+1 WHERE digest=$1",
        )
        .bind(challenge_digest(&input.challenge_id)?)
        .execute(c)
        .await
        .map_err(unavailable)?;
        Ok(None)
    }

    pub(crate) async fn verify_turnstile(
        &self,
        input: &pb::CheckinChallengeAuthorize,
    ) -> Result<(), Status> {
        self.turnstile
            .verify_checkin(&input.turnstile_token, &input.challenge_id)
            .await
            .map_err(|error| match error {
                VerifyError::Rejected => failed(),
                VerifyError::Unavailable => unavailable("verification provider unavailable"),
            })
    }

    pub(crate) async fn finish(
        &self,
        c: &mut PgConnection,
        actor: &Principal,
        day: &str,
        input: &pb::CheckinChallengeAuthorize,
    ) -> Result<pb::CheckinAuthorization, Status> {
        let row = self
            .bound_row(c, actor, day, &input.operation_key, &input.challenge_id)
            .await?;
        if row.try_get::<bool, _>("consumed").map_err(unavailable)? {
            return Err(required());
        }
        if !row.try_get::<bool, _>("verified").map_err(unavailable)? {
            if self.require_passkey {
                let Json(state): Json<Value> = row.try_get("passkey_state").map_err(unavailable)?;
                let assertion =
                    serde_json::from_str(&input.passkey_assertion_json).map_err(|_| failed())?;
                self.passkey
                    .as_ref()
                    .ok_or_else(|| unavailable("missing Passkey config"))?
                    .finish_checkin(c, actor.user_id, state, assertion)
                    .await
                    .map_err(identity_error)?;
            }
            sqlx::query("UPDATE core_checkin_challenges.challenges SET verified_at=clock_timestamp() WHERE digest=$1")
                .bind(challenge_digest(&input.challenge_id)?).execute(c).await.map_err(unavailable)?;
        }
        Ok(pb::CheckinAuthorization {
            authorization_id: input.challenge_id.clone(),
            day: day.into(),
            expires_at: row.try_get("expires_at").map_err(unavailable)?,
        })
    }

    pub(crate) async fn consume(
        &self,
        c: &mut PgConnection,
        actor: &Principal,
        input: &pb::CheckinLedgerPost,
    ) -> Result<Vec<u8>, Status> {
        let row = self
            .bound_row(
                c,
                actor,
                &input.day,
                &input.operation_key,
                &input.authorization_id,
            )
            .await?;
        if !row.try_get::<bool, _>("verified").map_err(unavailable)?
            || row.try_get::<bool, _>("consumed").map_err(unavailable)?
        {
            return Err(required());
        }
        let digest = challenge_digest(&input.authorization_id)?;
        sqlx::query("UPDATE core_checkin_challenges.challenges SET consumed_at=clock_timestamp() WHERE digest=$1")
            .bind(&digest).execute(c).await.map_err(unavailable)?;
        Ok(digest)
    }
}

pub(crate) async fn prior_receipt(
    c: &mut PgConnection,
    actor: &Principal,
    input: &pb::CheckinLedgerPost,
) -> Result<Option<pb::CheckinLedgerReceipt>, Status> {
    let row=sqlx::query("SELECT user_id,amount_units,journal_id,timestamp FROM core_checkin_challenges.receipts WHERE account_id=$1 AND day=$2::date")
        .bind(actor.owner_account_id).bind(&input.day).fetch_optional(c).await.map_err(unavailable)?;
    let Some(row) = row else { return Ok(None) };
    if row.try_get::<i64, _>("user_id").map_err(unavailable)? != actor.user_id
        || row.try_get::<i64, _>("amount_units").map_err(unavailable)? != input.amount_units
    {
        return Err(Status::already_exists("daily funding intention conflict"));
    }
    Ok(Some(pb::CheckinLedgerReceipt {
        user_id: actor.user_id,
        account_id: actor.owner_account_id,
        day: input.day.clone(),
        amount_units: input.amount_units,
        journal_id: row.try_get("journal_id").map_err(unavailable)?,
        timestamp: row.try_get("timestamp").map_err(unavailable)?,
        replayed: true,
    }))
}

pub(crate) async fn record_receipt(
    c: &mut PgConnection,
    input: &pb::CheckinLedgerPost,
    digest: &[u8],
    receipt: &pb::CheckinLedgerReceipt,
) -> Result<(), Status> {
    sqlx::query("INSERT INTO core_checkin_challenges.receipts(account_id,user_id,day,amount_units,challenge_digest,operation_key,journal_id,timestamp) VALUES($1,$2,$3::date,$4,$5,$6,$7,$8)")
        .bind(receipt.account_id).bind(receipt.user_id).bind(&receipt.day).bind(receipt.amount_units)
        .bind(digest).bind(&input.operation_key).bind(receipt.journal_id).bind(receipt.timestamp)
        .execute(c).await.map_err(unavailable)?;
    Ok(())
}

#[cfg(test)]
mod tests;

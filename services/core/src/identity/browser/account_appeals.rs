//! A verified primary identity may request review without receiving a Session.
use super::*;
use sqlx::{PgConnection, Row};

pub const ACCOUNT_APPEALS_SCHEMA: &str = include_str!("../../../schema/account_appeals.sql");

/// The proof is transported only in an HttpOnly Cookie, never in a JSON DTO.
pub struct AccountAppealGrant {
    pub token: String,
    pub context: AccountAppealContext,
}
#[derive(Serialize)]
pub struct AccountAppealContext {
    pub username: String,
    pub public_reason: Option<String>,
    pub public_reason_available: bool,
    pub expires_at: i64,
    pub request: Option<AccountAppealReceipt>,
}
#[derive(Serialize)]
pub struct AccountAppealReceipt {
    pub id: i64,
    pub status: String,
    pub reason: String,
    pub created_at: i64,
    pub reviewed_at: Option<i64>,
    pub already_pending: bool,
}

fn proof_digest(token: &str) -> Result<Vec<u8>> {
    if !token.starts_with("lmma_") || token.len() != 69 {
        return Err(IdentityError::Unauthorized);
    }
    digest(token)
}
async fn public_reason(c: &mut PgConnection, user: i64, version: i64) -> Result<Option<String>> {
    // Credential revocation advances the identity generation without a new ban.
    // An intervening enable prevents reuse of an older public explanation.
    // Private operation reasons and reviewer notes never enter this projection.
    Ok(sqlx::query_scalar("SELECT CASE WHEN action='admin.user.disable' THEN NULLIF(btrim(target->>'public_reason'),'') END FROM core_identity.audit WHERE action IN ('admin.user.disable','admin.user.enable','admin.user.appeal.approve') AND target->>'user_id'=$1 AND COALESCE((target->>'auth_version')::numeric,(target->>'previous_auth_version')::numeric+1)<=$2::text::numeric ORDER BY id DESC LIMIT 1")
        .bind(user.to_string()).bind(version.to_string()).fetch_optional(c).await?.flatten())
}
async fn locked_user(c: &mut PgConnection, user: i64) -> Result<sqlx::postgres::PgRow> {
    sqlx::query("SELECT u.id,u.active,u.auth_version,u.platform_role,a.active AS account_active,COALESCE(p.username,'') AS username,EXISTS(SELECT 1 FROM core_account_lifecycle.closures cl WHERE cl.user_id=u.id) AS closed,EXISTS(SELECT 1 FROM core_identity.privileged_import_facts f WHERE f.user_id=u.id) AS privileged FROM core_identity.users u JOIN core_identity.accounts a ON a.id=u.personal_account_id LEFT JOIN core_identity.user_profiles p ON p.user_id=u.id WHERE u.id=$1 FOR UPDATE OF u FOR SHARE OF a")
        .bind(user).fetch_optional(c).await?.ok_or(IdentityError::Unauthorized)
}
fn may_appeal(user: &sqlx::postgres::PgRow) -> Result<bool> {
    Ok(!user.try_get::<bool, _>("active")?
        && user.try_get::<bool, _>("account_active")?
        && !user.try_get::<bool, _>("closed")?
        && user.try_get::<String, _>("platform_role")? == "user"
        && !user.try_get::<bool, _>("privileged")?)
}
async fn receipt(
    c: &mut PgConnection,
    id: i64,
    user: i64,
    version: i64,
    current_version: i64,
    actionable: bool,
) -> Result<AccountAppealReceipt> {
    let row = sqlx::query("SELECT id,status,reason,floor(extract(epoch FROM created_at))::bigint AS created_at,floor(extract(epoch FROM reviewed_at))::bigint AS reviewed_at FROM core_account_appeals.requests WHERE id=$1 AND user_id=$2 AND user_version=$3")
        .bind(id).bind(user).bind(version).fetch_optional(c).await?.ok_or(IdentityError::Unauthorized)?;
    let stored: String = row.try_get("status")?;
    Ok(AccountAppealReceipt {
        id: row.try_get("id")?,
        status: if stored == "pending" && (version != current_version || !actionable) {
            "superseded".into()
        } else {
            stored
        },
        reason: row.try_get("reason")?,
        created_at: row.try_get("created_at")?,
        reviewed_at: row.try_get("reviewed_at")?,
        already_pending: true,
    })
}
async fn lock_flow(
    c: &mut PgConnection,
    token: &str,
) -> Result<(sqlx::postgres::PgRow, sqlx::postgres::PgRow)> {
    let hash = proof_digest(token)?;
    let user: i64 = sqlx::query_scalar("SELECT user_id FROM core_account_appeals.flows WHERE digest=$1 AND expires_at>clock_timestamp()")
        .bind(&hash).fetch_optional(&mut *c).await?.ok_or(IdentityError::Unauthorized)?;
    // Admin review, suspension, submission and closure serialize on the user.
    let current = locked_user(c, user).await?;
    let flow = sqlx::query("SELECT user_id,user_version,request_id,floor(extract(epoch FROM expires_at))::bigint AS expires_at FROM core_account_appeals.flows WHERE digest=$1 AND expires_at>clock_timestamp() FOR UPDATE")
        .bind(hash).fetch_optional(&mut *c).await?.ok_or(IdentityError::Unauthorized)?;
    Ok((current, flow))
}

impl BrowserAuth {
    pub(super) async fn disabled_account_appeal(
        &self,
        c: &mut PgConnection,
        user: i64,
        proof_kind: &str,
    ) -> Result<Option<AccountAppealGrant>> {
        let active: bool =
            sqlx::query_scalar("SELECT active FROM core_identity.users WHERE id=$1 FOR UPDATE")
                .bind(user)
                .fetch_optional(&mut *c)
                .await?
                .ok_or(IdentityError::Unauthorized)?;
        if active {
            return Ok(None);
        }
        let current = locked_user(c, user).await?;
        if !may_appeal(&current)? {
            return Err(IdentityError::Unauthorized);
        }
        let version = current.try_get::<i64, _>("auth_version")?;
        let token = new_secret("lmma_")?;
        let expires_at: i64 = sqlx::query_scalar("INSERT INTO core_account_appeals.flows(digest,user_id,user_version,proof_kind,expires_at) VALUES($1,$2,$3,$4,clock_timestamp()+interval '10 minutes') RETURNING floor(extract(epoch FROM expires_at))::bigint")
            .bind(proof_digest(&token)?).bind(user).bind(version).bind(proof_kind).fetch_one(&mut *c).await?;
        sqlx::query("DELETE FROM core_account_appeals.flows WHERE digest IN (SELECT digest FROM core_account_appeals.flows WHERE user_id=$1 AND expires_at<=clock_timestamp() LIMIT 1000)")
            .bind(user).execute(&mut *c).await?;
        let public_reason = public_reason(c, user, version).await?;
        Ok(Some(AccountAppealGrant {
            token,
            context: AccountAppealContext {
                username: current.try_get("username")?,
                public_reason_available: public_reason.is_some(),
                public_reason,
                expires_at,
                request: None,
            },
        }))
    }

    pub async fn account_appeal_context(&self, token: &str) -> Result<AccountAppealContext> {
        let mut tx = self.store.pool.begin().await?;
        let (current, flow) = lock_flow(&mut tx, token).await?;
        let user = flow.try_get::<i64, _>("user_id")?;
        let version = flow.try_get::<i64, _>("user_version")?;
        let current_version = current.try_get::<i64, _>("auth_version")?;
        let actionable = may_appeal(&current)?;
        let request = if let Some(id) = flow.try_get::<Option<i64>, _>("request_id")? {
            Some(receipt(&mut tx, id, user, version, current_version, actionable).await?)
        } else {
            if version != current_version || !actionable {
                return Err(IdentityError::Unauthorized);
            }
            None
        };
        let public_reason = public_reason(&mut tx, user, version).await?;
        let view = AccountAppealContext {
            username: current.try_get("username")?,
            public_reason_available: public_reason.is_some(),
            public_reason,
            expires_at: flow.try_get("expires_at")?,
            request,
        };
        tx.commit().await?;
        Ok(view)
    }

    pub async fn submit_account_appeal(
        &self,
        token: &str,
        reason: &str,
    ) -> Result<AccountAppealReceipt> {
        let reason = reason.trim();
        if !(5..=2000).contains(&reason.chars().count())
            || reason.len() > 8192
            || reason.contains('\0')
        {
            return Err(IdentityError::Invalid);
        }
        let mut tx = self.store.pool.begin().await?;
        let (current, flow) = lock_flow(&mut tx, token).await?;
        let user = flow.try_get::<i64, _>("user_id")?;
        let version = flow.try_get::<i64, _>("user_version")?;
        let current_version = current.try_get::<i64, _>("auth_version")?;
        let actionable = may_appeal(&current)?;
        if let Some(id) = flow.try_get::<Option<i64>, _>("request_id")? {
            let previous = receipt(&mut tx, id, user, version, current_version, actionable).await?;
            if previous.reason != reason {
                return Err(IdentityError::Conflict);
            }
            tx.commit().await?;
            return Ok(previous);
        }
        if version != current_version || !actionable {
            return Err(IdentityError::Unauthorized);
        }
        let existing: Option<i64> = sqlx::query_scalar("SELECT id FROM core_account_appeals.requests WHERE user_id=$1 AND user_version=$2 AND status='pending'")
            .bind(user).bind(version).fetch_optional(&mut *tx).await?;
        let id = match existing {
            Some(id) => id,
            None => sqlx::query_scalar("INSERT INTO core_account_appeals.requests(user_id,user_version,reason) VALUES($1,$2,$3) RETURNING id")
                .bind(user).bind(version).bind(reason).fetch_one(&mut *tx).await?,
        };
        sqlx::query("UPDATE core_account_appeals.flows SET request_id=$2 WHERE digest=$1")
            .bind(proof_digest(token)?)
            .bind(id)
            .execute(&mut *tx)
            .await?;
        let mut view = receipt(&mut tx, id, user, version, current_version, true).await?;
        view.already_pending = existing.is_some();
        tx.commit().await?;
        Ok(view)
    }
}

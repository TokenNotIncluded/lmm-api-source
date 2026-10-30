//! Explicit account closure preserves financial and authorization history.
use super::*;
use serde::{Deserialize, Serialize};

pub const ACCOUNT_LIFECYCLE_SCHEMA: &str = include_str!("../../../schema/account_lifecycle.sql");
const CONTRACT: &str = "core.account-closure.v1";

#[derive(Serialize)]
pub struct AccountClosurePreview {
    pub contract: &'static str,
    pub expected_auth_version: String,
    pub username: String,
    pub allowed: bool,
    pub blockers: Vec<&'static str>,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AccountClosureRequest {
    pub expected_auth_version: String,
    pub confirmation: String,
    pub operation_key: String,
}
#[derive(Serialize)]
pub struct AccountClosureReceipt {
    pub contract: &'static str,
    pub operation_key: String,
    pub closed: bool,
}
pub enum AccountClosureOutcome {
    Closed(AccountClosureReceipt),
    Blocked(AccountClosurePreview),
}
fn operation_digest(key: &str) -> Result<Vec<u8>> {
    if key.len() != 64
        || !key
            .bytes()
            .all(|c| c.is_ascii_digit() || (b'a'..=b'f').contains(&c))
    {
        return Err(IdentityError::Invalid);
    }
    digest(key)
}
async fn begin(pool: &PgPool) -> Result<sqlx::Transaction<'_, sqlx::Postgres>> {
    let mut tx = pool.begin().await?;
    sqlx::query("SET TRANSACTION ISOLATION LEVEL SERIALIZABLE")
        .execute(&mut *tx)
        .await?;
    sqlx::query("SET LOCAL statement_timeout='10s'")
        .execute(&mut *tx)
        .await?;
    sqlx::query("SET LOCAL lock_timeout='500ms'")
        .execute(&mut *tx)
        .await?;
    Ok(tx)
}
async fn preview(c: &mut PgConnection, actor: &Principal) -> Result<AccountClosurePreview> {
    // The same live user/account authority used by admission is locked before
    // ledger rows. Existing reservations must finish before closure can pass.
    let row=sqlx::query("SELECT p.username,u.platform_role,EXISTS(SELECT 1 FROM core_identity.privileged_import_facts f WHERE f.user_id=u.id) AS imported_privileged FROM core_identity.users u JOIN core_identity.accounts a ON a.id=u.personal_account_id JOIN core_identity.user_profiles p ON p.user_id=u.id WHERE u.id=$1 AND u.active AND a.active FOR UPDATE OF a NOWAIT")
        .bind(actor.user_id).fetch_optional(&mut *c).await?.ok_or(IdentityError::Unauthorized)?;
    let mut blockers = Vec::new();
    if row.try_get::<String, _>("platform_role")? != "user"
        || row.try_get::<bool, _>("imported_privileged")?
    {
        blockers.push("privileged_account");
    }
    match crate::financial_profile::read_wallet(c, actor.user_id).await {
        Ok(_) => {
            let empty: bool =
                sqlx::query_scalar("SELECT core_account_lifecycle.wallet_is_empty($1)")
                    .bind(actor.owner_account_id)
                    .fetch_one(&mut *c)
                    .await?;
            if !empty {
                blockers.push("unsettled_balance");
            }
        }
        Err(crate::financial_profile::ProfileError::Unavailable) => {
            blockers.push("financial_history_unavailable")
        }
        Err(_) => return Err(IdentityError::Storage),
    }
    let pending=sqlx::query("SELECT EXISTS(SELECT 1 FROM core_identity.teams t WHERE t.owner_user_id=$1 AND (t.active OR EXISTS(SELECT 1 FROM core_billing.ledger_accounts l JOIN core_billing.balance_state b ON b.ledger_account_id=l.id WHERE l.owner_account_id=t.account_id AND b.balance_units<>0))) AS owned_team,EXISTS(SELECT 1 FROM core_charging.requests WHERE (actor_user_id=$1 OR owner_account_id=$2 OR payer_account_id=$2) AND state IN ('reserved','streaming','reconcile')) AS model_request,EXISTS(SELECT 1 FROM core_charging.subscriptions WHERE account_id=$2 AND active AND ends_at>floor(extract(epoch FROM clock_timestamp()))::bigint) AS subscription,EXISTS(SELECT 1 FROM core_payments.intents i WHERE (i.account_id=$2 OR i.created_by_user_id=$1) AND (i.credit_journal_id IS NULL OR EXISTS(SELECT 1 FROM core_payments.refund_holds h WHERE h.intent_id=i.id AND h.state='held') OR EXISTS(SELECT 1 FROM core_payments.blocked_refunds b WHERE b.intent_id=i.id))) AS payment,EXISTS(SELECT 1 FROM core_commerce.orders WHERE (buyer_account_id=$2 OR seller_account_id=$2 OR actor_user_id=$1) AND state='paid') AS commerce,EXISTS(SELECT 1 FROM core_wallet.transfers WHERE (owner_user=$1 OR payer_account=$2) AND state='pending') AS transfer")
        .bind(actor.user_id).bind(actor.owner_account_id).fetch_one(&mut *c).await?;
    for (field, reason) in [
        ("owned_team", "owned_team"),
        ("model_request", "model_request_in_progress"),
        ("subscription", "active_subscription"),
        ("payment", "payment_or_refund_pending"),
        ("commerce", "order_in_progress"),
        ("transfer", "transfer_pending"),
    ] {
        if pending.try_get::<bool, _>(field)? {
            blockers.push(reason);
        }
    }
    Ok(AccountClosurePreview {
        contract: CONTRACT,
        expected_auth_version: actor.user_version.to_string(),
        username: row.try_get("username")?,
        allowed: blockers.is_empty(),
        blockers,
    })
}

impl BrowserAuth {
    pub async fn account_closure_preview(
        &self,
        access: &str,
    ) -> BrowserResult<AccountClosurePreview> {
        let mut tx = begin(&self.store.pool).await?;
        let (actor, _) = self.security_actor(&mut tx, access, true).await?;
        let preview = preview(&mut tx, &actor).await?;
        tx.commit().await?;
        Ok(preview)
    }

    pub async fn close_account(
        &self,
        access: &str,
        input: &AccountClosureRequest,
    ) -> BrowserResult<AccountClosureOutcome> {
        let operation = operation_digest(&input.operation_key)?;
        let version = input
            .expected_auth_version
            .parse::<i64>()
            .map_err(|_| IdentityError::Invalid)?;
        if version <= 0
            || version.to_string() != input.expected_auth_version
            || input.confirmation.is_empty()
            || input.confirmation.len() > 128
        {
            return Err(IdentityError::Invalid.into());
        }
        let mut tx = begin(&self.store.pool).await?;
        let (actor, _) = self.security_actor(&mut tx, access, true).await?;
        if actor.user_version != version {
            return Err(IdentityError::Conflict.into());
        }
        let preview = preview(&mut tx, &actor).await?;
        if input.confirmation != preview.username {
            return Err(IdentityError::Invalid.into());
        }
        if !preview.allowed {
            tx.commit().await?;
            return Ok(AccountClosureOutcome::Blocked(preview));
        }
        sqlx::query("INSERT INTO core_account_lifecycle.closures(user_id,account_id,credential_id,previous_auth_version,operation_digest) VALUES($1,$2,$3,$4,$5)")
            .bind(actor.user_id).bind(actor.owner_account_id).bind(actor.credential_id).bind(actor.user_version).bind(&operation).execute(&mut *tx).await?;
        sqlx::query("UPDATE core_identity.credentials SET revoked_at=clock_timestamp() WHERE user_id=$1 AND revoked_at IS NULL")
            .bind(actor.user_id).execute(&mut *tx).await?;
        sqlx::query("UPDATE core_identity.web_sessions SET revoked_at=clock_timestamp() WHERE user_id=$1 AND revoked_at IS NULL")
            .bind(actor.user_id).execute(&mut *tx).await?;
        sqlx::query("UPDATE core_identity.memberships SET active=false,can_spend=false,version=version+1 WHERE user_id=$1 AND active")
            .bind(actor.user_id).execute(&mut *tx).await?;
        sqlx::query("UPDATE core_identity.invites SET rejected_at=clock_timestamp() WHERE recipient_user_id=$1 AND accepted_at IS NULL AND rejected_at IS NULL AND withdrawn_at IS NULL")
            .bind(actor.user_id).execute(&mut *tx).await?;
        // Changing active advances the established user generation trigger.
        sqlx::query("UPDATE core_identity.users SET active=false WHERE id=$1")
            .bind(actor.user_id)
            .execute(&mut *tx)
            .await?;
        sqlx::query("UPDATE core_identity.accounts SET active=false WHERE id=$1")
            .bind(actor.owner_account_id)
            .execute(&mut *tx)
            .await?;
        audit(&mut tx, &actor, "account.close", json!({"user_id":actor.user_id,"account_id":actor.owner_account_id,"operation_digest":operation.iter().map(|byte| format!("{byte:02x}")).collect::<String>()})).await?;
        tx.commit().await?;
        Ok(AccountClosureOutcome::Closed(AccountClosureReceipt {
            contract: CONTRACT,
            operation_key: input.operation_key.clone(),
            closed: true,
        }))
    }

    /// A random 256-bit receipt capability reveals only terminal completion;
    /// it grants no identity, profile, login, balance or mutation authority.
    pub async fn account_closure_receipt(
        &self,
        operation_key: &str,
    ) -> BrowserResult<Option<AccountClosureReceipt>> {
        let operation = operation_digest(operation_key)?;
        let found: bool=sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM core_account_lifecycle.closures WHERE operation_digest=$1)")
            .bind(operation).fetch_one(&self.store.pool).await?;
        Ok(found.then(|| AccountClosureReceipt {
            contract: CONTRACT,
            operation_key: operation_key.into(),
            closed: true,
        }))
    }
}

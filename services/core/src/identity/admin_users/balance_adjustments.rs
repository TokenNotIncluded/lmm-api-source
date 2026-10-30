//! Explicit additive administrator corrections through the canonical ledger.
//! Positive corrections are gifts, never claims of an external payment.
use super::{AdminUsersStore, ProfileError};
use crate::{
    billing::{
        Credits,
        ledger::{Action, Outcome, Request as LedgerRequest, Wallet},
    },
    identity::{self, CredentialKind},
};
use axum::{
    Json, Router,
    extract::{DefaultBodyLimit, Path, State, rejection::JsonRejection},
    http::HeaderMap,
    routing::{get, post},
};
use serde::{Deserialize, Serialize};
use sqlx::{PgConnection, Row, types::Json as SqlJson};

const SCOPE: &str = "core-admin-user-adjustment";
const CONTRACT: &str = "core.admin-user-adjustment.v1";
type Result<T> = std::result::Result<T, ProfileError>;

#[derive(Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AdjustmentRequest {
    pub operation_key: String,
    /// Signed integer Core credit units, encoded as a decimal string.
    pub delta_units: String,
    pub reason: String,
}
fn valid_operation_key(value: &str) -> bool {
    (32..=64).contains(&value.len())
        && value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || b"-_".contains(&byte))
}
impl AdjustmentRequest {
    fn validate(&self) -> Result<i64> {
        if !valid_operation_key(&self.operation_key)
            || self.reason.trim().is_empty()
            || self.reason.len() > 1000
            || self.reason.chars().count() > 500
            || self.reason.chars().any(char::is_control)
            || self.delta_units.len() > 20
        {
            return Err(ProfileError::Invalid);
        }
        let amount = self
            .delta_units
            .parse::<i64>()
            .map_err(|_| ProfileError::Invalid)?;
        if amount == 0 || amount == i64::MIN || amount.to_string() != self.delta_units {
            return Err(ProfileError::Invalid);
        }
        Ok(amount)
    }
}

#[derive(Debug, Serialize, PartialEq, Eq)]
pub struct AdjustmentReceipt {
    pub contract: &'static str,
    pub operation_key: String,
    pub user_id: String,
    pub delta_units: String,
    pub status: &'static str,
    pub journal_id: Option<String>,
    pub rejection: Option<String>,
}
fn receipt(
    user_id: i64,
    key: &str,
    request: &LedgerRequest,
    outcome: &Outcome,
) -> Result<AdjustmentReceipt> {
    let delta = match request.action {
        Action::Gift { amount_units, .. } => i64::from(amount_units),
        Action::Charge { amount_units, .. } => -i64::from(amount_units),
        _ => return Err(ProfileError::Unavailable),
    };
    let (status, journal_id, rejection) = match outcome {
        Outcome::Posted { journal_id, .. } => ("posted", Some(journal_id.to_string()), None),
        Outcome::Rejected { code } => (
            "rejected",
            None,
            Some(
                serde_json::to_value(code)
                    .map_err(|_| ProfileError::Unavailable)?
                    .as_str()
                    .ok_or(ProfileError::Unavailable)?
                    .to_owned(),
            ),
        ),
    };
    Ok(AdjustmentReceipt {
        contract: CONTRACT,
        operation_key: key.to_owned(),
        user_id: user_id.to_string(),
        delta_units: delta.to_string(),
        status,
        journal_id,
        rejection,
    })
}

async fn subject(c: &mut PgConnection, user_id: i64, write: bool) -> Result<i64> {
    if user_id <= 0 {
        return Err(ProfileError::Invalid);
    }
    let lock = if write { " FOR SHARE OF u,a" } else { "" };
    let row = sqlx::query(&format!(
        "SELECT u.personal_account_id,u.platform_role,u.active,a.active AS account_active,
         EXISTS(SELECT 1 FROM core_identity.privileged_import_facts p WHERE p.user_id=u.id) AS imported_admin
         FROM core_identity.users u JOIN core_identity.accounts a ON a.id=u.personal_account_id
         WHERE u.id=$1 AND a.kind='personal'{lock}"
    )).bind(user_id).fetch_optional(c).await?.ok_or(ProfileError::NotFound)?;
    if row.try_get::<String, _>("platform_role")? != "user"
        || row.try_get::<bool, _>("imported_admin")?
        || (write
            && (!row.try_get::<bool, _>("active")? || !row.try_get::<bool, _>("account_active")?))
    {
        return Err(ProfileError::Forbidden);
    }
    Ok(row.try_get("personal_account_id")?)
}

impl AdminUsersStore {
    pub async fn adjust_user_balance(
        &self,
        secret: &str,
        user_id: i64,
        input: &AdjustmentRequest,
    ) -> Result<AdjustmentReceipt> {
        let delta = input.validate()?;
        let mut tx = self.identity.pool.begin().await?;
        for statement in [
            "SET LOCAL statement_timeout='10s'",
            "SET LOCAL lock_timeout='500ms'",
            "SET LOCAL synchronous_commit=on",
        ] {
            sqlx::query(statement).execute(&mut *tx).await?;
        }
        let actor = identity::authorize_in_transaction(&mut tx, secret).await?;
        if actor.credential_kind != CredentialKind::Session || actor.user_id == user_id {
            return Err(ProfileError::Forbidden);
        }
        Self::authorize(&mut tx, secret).await?;
        let account_id = subject(&mut tx, user_id, true).await?;
        let amount_units = Credits::try_from(delta.abs()).map_err(|_| ProfileError::Invalid)?;
        let request = LedgerRequest {
            scope: SCOPE.to_owned(),
            operation_key: format!("{user_id}:{}", input.operation_key),
            actor_user_id: Some(actor.user_id),
            reason: input.reason.clone(),
            action: if delta > 0 {
                Action::Gift {
                    account_id,
                    amount_units,
                }
            } else {
                Action::Charge {
                    account_id,
                    amount_units,
                }
            },
        };
        request.validate().map_err(|_| ProfileError::Invalid)?;
        // Use the ledger's identical operation gate before the prior-result read.
        // Holding it through commit also prevents duplicate identity audit rows.
        sqlx::query("SELECT pg_advisory_xact_lock(hashtextextended(jsonb_build_array('lmm-ledger-v1',$1::text,$2::text)::TEXT,0))")
            .bind(SCOPE).bind(&request.operation_key).execute(&mut *tx).await?;
        if let Some((SqlJson(prior), SqlJson(outcome))) = sqlx::query_as::<_, (SqlJson<LedgerRequest>, SqlJson<Outcome>)>(
            "SELECT request,result FROM core_billing.ledger_operations WHERE scope=$1 AND operation_key=$2"
        ).bind(SCOPE).bind(&request.operation_key).fetch_optional(&mut *tx).await? {
            if prior != request { return Err(ProfileError::Conflict); }
            let response = receipt(user_id, &input.operation_key, &prior, &outcome)?;
            tx.commit().await?;
            return Ok(response);
        }
        // This existing reader verifies native zero provenance or an established
        // four-bucket wallet. An absent historical opening never becomes zero.
        crate::financial_profile::read_wallet(&mut tx, user_id)
            .await
            .map_err(|_| ProfileError::Unavailable)?;
        let _: SqlJson<Wallet> = sqlx::query_scalar("SELECT core_billing.open_ledger_wallet($1)")
            .bind(account_id)
            .fetch_one(&mut *tx)
            .await?;
        let SqlJson(outcome): SqlJson<Outcome> =
            sqlx::query_scalar("SELECT core_billing.post_ledger($1)")
                .bind(SqlJson(&request))
                .fetch_one(&mut *tx)
                .await?;
        identity::audit(
            &mut tx,
            &actor,
            "admin.user.balance.adjust",
            serde_json::json!({
                "user_id":user_id,"account_id":account_id,"operation_key":input.operation_key,
                "delta_units":input.delta_units,"reason":input.reason,"outcome":outcome,
            }),
        )
        .await?;
        let response = receipt(user_id, &input.operation_key, &request, &outcome)?;
        tx.commit().await?;
        Ok(response)
    }

    pub async fn user_balance_adjustment(
        &self,
        secret: &str,
        user_id: i64,
        key: &str,
    ) -> Result<AdjustmentReceipt> {
        if !valid_operation_key(key) {
            return Err(ProfileError::Invalid);
        }
        let mut tx = self.begin().await?;
        Self::authorize(&mut tx, secret).await?;
        let account = subject(&mut tx, user_id, false).await?;
        let (SqlJson(request), SqlJson(outcome)) = sqlx::query_as::<_, (SqlJson<LedgerRequest>, SqlJson<Outcome>)>(
            "SELECT request,result FROM core_billing.ledger_operations WHERE scope=$1 AND operation_key=$2"
        ).bind(SCOPE).bind(format!("{user_id}:{key}")).fetch_optional(&mut *tx).await?.ok_or(ProfileError::NotFound)?;
        if !matches!(request.action, Action::Gift { account_id, .. } | Action::Charge { account_id, .. } if account_id == account)
        {
            return Err(ProfileError::Unavailable);
        }
        let response = receipt(user_id, key, &request, &outcome)?;
        tx.commit().await?;
        Ok(response)
    }
}

pub(super) fn routes() -> Router<AdminUsersStore> {
    Router::new()
        .route(
            "/core/v1/admin/user-balances/{id}/adjustments",
            post(adjust),
        )
        .route(
            "/core/v1/admin/user-balances/{id}/adjustments/{key}",
            get(read),
        )
        .layer(DefaultBodyLimit::max(4096))
}
async fn adjust(
    State(store): State<AdminUsersStore>,
    headers: HeaderMap,
    Path(id): Path<i64>,
    input: std::result::Result<Json<AdjustmentRequest>, JsonRejection>,
) -> Result<Json<AdjustmentReceipt>> {
    let Json(input) = input.map_err(|_| ProfileError::Invalid)?;
    Ok(Json(
        store
            .adjust_user_balance(super::security::bearer(&headers)?, id, &input)
            .await?,
    ))
}
async fn read(
    State(store): State<AdminUsersStore>,
    headers: HeaderMap,
    Path((id, key)): Path<(i64, String)>,
) -> Result<Json<AdjustmentReceipt>> {
    Ok(Json(
        store
            .user_balance_adjustment(super::security::bearer(&headers)?, id, &key)
            .await?,
    ))
}

#[cfg(test)]
#[path = "balance_adjustments_tests.rs"]
mod tests;

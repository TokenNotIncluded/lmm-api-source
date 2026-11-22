//! Closed, current-owner invitation observations. No code or reward is created.
use super::{Error, context};
use serde::Serialize;
use sqlx::{PgPool, Row};

pub(crate) const FEATURE: &str = "assistant.invitation_facts.v1";

#[derive(Clone, Debug, Serialize)]
pub(crate) struct RewardBalances {
    pub wallet_signed_credits: i64,
    pub affiliate_available_credits: i64,
    pub affiliate_debt_credits: i64,
    pub opening_affiliate_history_credits: i64,
    pub history_scope: &'static str,
}

#[derive(Clone, Debug, Serialize)]
pub(crate) struct InvitationFacts {
    pub version: u32,
    pub ok: bool,
    pub scope: &'static str,
    pub actor_user_id: i64,
    pub auth_version: i64,
    pub checked_at_unix: i64,
    pub invitation_code: Option<String>,
    /// Private Core-to-host observation used only to reject an invitation to
    /// the inviter's own address. The model projection deliberately omits it.
    pub actor_email: Option<String>,
    pub native_registered_referrals: i64,
    pub legacy_invited_count_available: bool,
    pub legacy_invited_count: Option<i64>,
    pub automatic_signup_rewards_enabled: bool,
    pub automatic_signup_rewards_reason: &'static str,
    pub balance_status: &'static str,
    pub balances: Option<RewardBalances>,
    pub credit_unit: &'static str,
    pub financial_authority: bool,
}

pub(crate) async fn check_ready(pool: &PgPool) -> Result<(), Error> {
    let mut tx = pool.begin().await?;
    sqlx::query("SET TRANSACTION READ ONLY")
        .execute(&mut *tx)
        .await?;
    for query in [
        "SELECT code,user_id FROM core_identity.legacy_affiliate_aliases WHERE FALSE",
        "SELECT code,user_id FROM core_identity.affiliate_codes WHERE FALSE",
        "SELECT user_id,inviter_id FROM core_identity.affiliate_registrations WHERE FALSE",
        "SELECT user_id,email FROM core_identity.user_profiles WHERE FALSE",
        "SELECT owner_account_id,bucket FROM core_billing.ledger_accounts WHERE FALSE",
        "SELECT ledger_account_id,balance_units FROM core_billing.balance_state WHERE FALSE",
        "SELECT user_id,account_id,aff_history FROM core_billing.account_opening_counters WHERE FALSE",
        "SELECT migration_id,source_entity_key,account_id,source_row_sha256 FROM core_billing.opening_imports WHERE FALSE",
        "SELECT migration_id,source_archive_sha256,mapping_sha256,observed_at FROM core_billing.opening_import_runs WHERE FALSE",
        "SELECT actor_user_id,state,settled,refunded FROM core_charging.requests WHERE FALSE",
    ] {
        sqlx::query(query).fetch_all(&mut *tx).await?;
    }
    tx.commit().await?;
    Ok(())
}

pub(crate) async fn read(pool: &PgPool, original_session: &str) -> Result<InvitationFacts, Error> {
    let mut tx = pool.begin().await?;
    sqlx::query("SET TRANSACTION ISOLATION LEVEL REPEATABLE READ")
        .execute(&mut *tx)
        .await?;
    let actor = context::from_session(&mut tx, original_session).await?;
    let code = crate::affiliate::existing_invitation_code(&mut tx, actor.user_id()).await?;
    let row = sqlx::query(
        "SELECT p.email,
         (SELECT count(*) FROM core_identity.affiliate_registrations r WHERE r.inviter_id=$1) AS native_count,
         floor(extract(epoch FROM transaction_timestamp()))::bigint AS checked
         FROM core_identity.user_profiles p WHERE p.user_id=$1",
    )
    .bind(actor.user_id()).fetch_one(&mut *tx).await?;
    let email: String = row.try_get("email")?;
    let native_count: i64 = row.try_get("native_count")?;
    if !(0..=9_007_199_254_740_991).contains(&native_count) {
        return Err(Error::Unavailable);
    }
    let (balances, balance_status) =
        match crate::financial_profile::read(&mut tx, actor.user_id()).await {
            Ok(profile) => (
                Some(RewardBalances {
                    wallet_signed_credits: profile.quota,
                    affiliate_available_credits: profile.aff_quota,
                    affiliate_debt_credits: profile.aff_debt,
                    opening_affiliate_history_credits: profile.aff_history_quota,
                    history_scope: "validated_opening_or_native_zero_basis_only",
                }),
                "available",
            ),
            Err(crate::financial_profile::ProfileError::Unavailable) => {
                (None, "financial_projection_unavailable")
            }
            Err(_) => return Err(Error::Unavailable),
        };
    let authority = actor.revalidate(&mut tx).await?;
    let result = InvitationFacts {
        version: 1,
        ok: true,
        scope: "self",
        actor_user_id: actor.user_id(),
        auth_version: authority.auth_version,
        checked_at_unix: row.try_get("checked")?,
        invitation_code: code,
        actor_email: (!email.is_empty()).then_some(email),
        native_registered_referrals: native_count,
        // Native registrations are not a replacement for the old aggregate.
        // No runtime-approved old invited-count projection currently exists.
        legacy_invited_count_available: false,
        legacy_invited_count: None,
        // The current registration path only records attribution; it does not
        // issue a new monetary referral contract. Old option values cannot enable it.
        automatic_signup_rewards_enabled: false,
        automatic_signup_rewards_reason: "registration_records_attribution_without_issuing_reward",
        balance_status,
        balances,
        credit_unit: "core_credit",
        financial_authority: false,
    };
    tx.commit().await?;
    Ok(result)
}

#[derive(serde::Deserialize)]
#[serde(deny_unknown_fields)]
struct InvitationSource {
    email: String,
}
#[derive(serde::Deserialize)]
#[serde(deny_unknown_fields)]
struct InvitationDraft {
    kind: String,
    email: String,
    invitation_code: String,
}

/// The private host cannot alter the actual provider's recipient or borrow
/// another account's code when asking Core for a real browser confirmation.
pub(crate) async fn validate_draft(
    c: &mut sqlx::PgConnection,
    context: &super::context::VerifiedToolContext,
    source: &serde_json::Value,
    draft: &serde_json::Value,
) -> Result<(), Error> {
    let source: InvitationSource =
        serde_json::from_value(source.clone()).map_err(|_| Error::Invalid)?;
    let draft: InvitationDraft =
        serde_json::from_value(draft.clone()).map_err(|_| Error::Invalid)?;
    let recipient =
        crate::identity::email::EmailAddress::parse(&source.email).map_err(|_| Error::Invalid)?;
    let code = crate::affiliate::existing_invitation_code(c, context.user_id())
        .await?
        .ok_or(Error::Conflict)?;
    let own: String =
        sqlx::query_scalar("SELECT email FROM core_identity.user_profiles WHERE user_id=$1")
            .bind(context.user_id())
            .fetch_one(&mut *c)
            .await?;
    if draft.kind != "invitation_email_v1"
        || draft.email != recipient.as_str()
        || draft.invitation_code != code
        || own.trim().to_lowercase() == recipient.as_str()
    {
        return Err(Error::Conflict);
    }
    context.revalidate(c).await?;
    Ok(())
}

#[cfg(test)]
mod tests;

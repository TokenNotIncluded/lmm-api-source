//! Core assistant business reads. Model/Go input never supplies authority.
//! The worker authenticates its original Session and delegates only a typed call.
pub mod account;
pub mod confirmation;
pub mod context;
pub mod facts;
pub mod keys;
pub mod models;
mod normalization;
pub mod redaction;
pub mod registration {
    use super::{Error, context::VerifiedToolContext};
    use serde_json::{Value, json};
    use sqlx::{PgConnection, Row};
    /// This projection contains no other person's identifiers, IP, email or text.
    /// Missing immutable evidence is unavailable, not a guessed low-risk outcome.
    pub async fn risk_in_transaction(
        c: &mut PgConnection,
        context: &VerifiedToolContext,
    ) -> Result<Value, Error> {
        let actor = context.revalidate(c).await?;
        if actor.developer || actor.role != "user" {
            return Err(Error::Forbidden);
        }
        let r=sqlx::query("SELECT o.source_policy_version,o.source_kind,o.source_ip IS NOT NULL AS network_available,q.evidence_kind,q.registration_hold,q.reward_used FROM core_abuse.registration_observations o LEFT JOIN core_gift.qualifications q ON q.user_id=o.user_id WHERE o.user_id=$1")
            .bind(actor.user).fetch_optional(&mut *c).await?;
        let Some(r) = r else {
            return Ok(json!({"ok":false,"status":"verification_unavailable"}));
        };
        if r.try_get::<Option<i32>, _>("source_policy_version")? != Some(1)
            || !r.try_get::<bool, _>("network_available")?
            || r.try_get::<Option<String>, _>("evidence_kind")?.as_deref()
                != Some("google_verified_email_v1")
        {
            return Ok(json!({"ok":false,"status":"verification_unavailable"}));
        }
        let marked = crate::abuse::is_marked_in_transaction(c, actor.user)
            .await
            .map_err(|_| Error::Unavailable)?;
        Ok(
            json!({"ok":true,"status":"checked","risk":{"verified_email":true,"registration_network_evidence":true,"registration_hold":r.try_get::<Option<bool>,_>("registration_hold")?,"reward_used":r.try_get::<Option<bool>,_>("reward_used")?,"current_abuse_mark":marked,"source":"core verified registration v1"},"untrusted_own_history":[],"own_history_available":false,"message":"Use these as fallible observations, not proof of a person's identity or a tool authorization."}),
        )
    }
}
pub mod support {
    use super::{Error, context::VerifiedToolContext};
    use serde_json::{Value, json};
    use sqlx::{PgConnection, Row};
    /// Eligibility is external recharge evidence, never a wallet/gift balance.
    pub async fn eligibility_in_transaction(
        c: &mut PgConnection,
        context: &VerifiedToolContext,
    ) -> Result<Value, Error> {
        let actor = context.revalidate(c).await?;
        let r=sqlx::query("SELECT count(*)::bigint AS payments,COALESCE(sum(amount_minor),0)::text AS amount_minor FROM core_payments.intents i WHERE i.account_id=$1 AND i.created_by_user_id=$2 AND i.provider='stripe' AND i.environment='live' AND i.amount_minor>0 AND i.credit_journal_id IS NOT NULL AND i.provider_transaction IS NOT NULL AND i.refunded_minor=0 AND NOT i.refund_fenced AND EXISTS(SELECT 1 FROM core_payments.accepted_proofs p WHERE p.intent_id=i.id) AND NOT EXISTS(SELECT 1 FROM core_payments.refund_holds h WHERE h.intent_id=i.id AND h.state IN ('held','committed')) AND NOT EXISTS(SELECT 1 FROM core_payments.blocked_refunds b WHERE b.intent_id=i.id)")
            .bind(actor.account).bind(actor.user).fetch_one(c).await?;
        let payments: i64 = r.try_get("payments")?;
        Ok(
            json!({"ok":true,"scope":"self","eligible":payments>0,"successful_external_payments":payments,"unrefunded_amount_minor":r.try_get::<String,_>("amount_minor")?,"currency":"USD","evidence":"core verified external payment receipts","wallet_balance_is_not_qualification":true}),
        )
    }
}

use serde::Serialize;
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Error {
    Invalid,
    Unauthorized,
    Forbidden,
    NotFound,
    Conflict,
    Unavailable,
}
impl From<sqlx::Error> for Error {
    fn from(_: sqlx::Error) -> Self {
        Self::Unavailable
    }
}
impl From<crate::identity::IdentityError> for Error {
    fn from(e: crate::identity::IdentityError) -> Self {
        use crate::identity::IdentityError as I;
        match e {
            I::Invalid => Self::Invalid,
            I::Unauthorized => Self::Unauthorized,
            I::Forbidden => Self::Forbidden,
            I::Conflict => Self::Conflict,
            _ => Self::Unavailable,
        }
    }
}

use context::VerifiedToolContext;
use serde::Deserialize;
use serde_json::Value;
use sqlx::PgConnection;
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct EmptyArgs {}
/// Only these implemented tools are dispatchable; discovery must not advertise
/// the other main catalogue names until their real domain adapters exist.
pub enum TypedReadArgs {
    AccountAccess,
    AvailableModels,
    ModelPricing(models::ModelPricingArgs),
    UserOverview(account::UserTarget),
    UserUsage(account::UsageSummaryArgs),
    OwnUsage(account::OwnUsageArgs),
    ListMyApiKeys(keys::ListMyApiKeys),
    NewUserGiftStatus,
    RegistrationRisk,
    TechnicalSupportEligibility,
}
impl TypedReadArgs {
    pub fn parse(name: &str, bytes: &[u8]) -> Result<Self, Error> {
        if bytes.len() > 16_384 {
            return Err(Error::Invalid);
        }
        fn empty(bytes: &[u8]) -> Result<(), Error> {
            serde_json::from_slice::<EmptyArgs>(bytes)
                .map(|_| ())
                .map_err(|_| Error::Invalid)
        }
        Ok(match name {
            "get_available_models" => {
                empty(bytes)?;
                Self::AvailableModels
            }
            "get_model_pricing" => {
                Self::ModelPricing(serde_json::from_slice(bytes).map_err(|_| Error::Invalid)?)
            }
            "get_account_access" => {
                empty(bytes)?;
                Self::AccountAccess
            }
            "get_user_overview" => {
                Self::UserOverview(serde_json::from_slice(bytes).map_err(|_| Error::Invalid)?)
            }
            "get_user_usage_summary" => {
                Self::UserUsage(serde_json::from_slice(bytes).map_err(|_| Error::Invalid)?)
            }
            "get_usage_summary" => {
                Self::OwnUsage(serde_json::from_slice(bytes).map_err(|_| Error::Invalid)?)
            }
            "list_my_api_keys" => {
                Self::ListMyApiKeys(serde_json::from_slice(bytes).map_err(|_| Error::Invalid)?)
            }
            "get_new_user_gift_status" => {
                empty(bytes)?;
                Self::NewUserGiftStatus
            }
            "get_registration_risk" => {
                empty(bytes)?;
                Self::RegistrationRisk
            }
            _ => return Err(Error::Invalid),
        })
    }
    pub fn name(&self) -> &'static str {
        match self {
            Self::AccountAccess => "get_account_access",
            Self::AvailableModels => "get_available_models",
            Self::ModelPricing(_) => "get_model_pricing",
            Self::UserOverview(_) => "get_user_overview",
            Self::UserUsage(_) => "get_user_usage_summary",
            Self::OwnUsage(_) => "get_usage_summary",
            Self::ListMyApiKeys(_) => "list_my_api_keys",
            Self::NewUserGiftStatus => "get_new_user_gift_status",
            Self::RegistrationRisk => "get_registration_risk",
            Self::TechnicalSupportEligibility => "core_technical_support_eligibility",
        }
    }
}
#[derive(Serialize)]
pub struct TypedReadResult {
    pub tool_name: &'static str,
    pub output: Value,
}
pub async fn read_in_transaction(
    c: &mut PgConnection,
    context: &VerifiedToolContext,
    args: &TypedReadArgs,
) -> Result<TypedReadResult, Error> {
    let output = match args {
        TypedReadArgs::AvailableModels | TypedReadArgs::ModelPricing(_) => {
            return Err(Error::Unavailable);
        }
        TypedReadArgs::AccountAccess => account::access_in_transaction(c, context).await?,
        TypedReadArgs::UserOverview(input) => {
            account::overview_in_transaction(c, context, input).await?
        }
        TypedReadArgs::UserUsage(input) => {
            crate::usage::assistant_summary_in_transaction(c, context, input, false).await?
        }
        TypedReadArgs::OwnUsage(input) => {
            crate::usage::assistant_summary_in_transaction(
                c,
                context,
                &account::UsageSummaryArgs {
                    user_id: None,
                    identifier: None,
                    days: input.days,
                },
                true,
            )
            .await?
        }
        TypedReadArgs::ListMyApiKeys(input) => keys::list_in_transaction(c, context, input).await?,
        TypedReadArgs::NewUserGiftStatus => account::gift_status_in_transaction(c, context).await?,
        TypedReadArgs::RegistrationRisk => registration::risk_in_transaction(c, context).await?,
        TypedReadArgs::TechnicalSupportEligibility => {
            support::eligibility_in_transaction(c, context).await?
        }
    };
    Ok(TypedReadResult {
        tool_name: args.name(),
        output,
    })
}
#[cfg(test)]
mod tests;

/// Core's own implemented adapters. Host discovery intersects these immutable
/// definitions with its explicit approved scope; Go metadata adds no authority.
pub fn implemented_manifest() -> Result<Value, Error> {
    serde_json::from_str(include_str!("assistant_tools_core/manifest.json"))
        .map_err(|_| Error::Unavailable)
}

/// The real worker injects its serving gateway; the generic DB-only entry cannot
/// fabricate a model catalogue when this dependency is absent.
pub async fn read_with_gateway_in_transaction(
    c: &mut PgConnection,
    context: &VerifiedToolContext,
    args: &TypedReadArgs,
    gateway: &crate::model_http::ModelGateway,
) -> Result<TypedReadResult, Error> {
    let surface = models::ModelsProjection::from_gateway(gateway);
    let output = match args {
        TypedReadArgs::AvailableModels => surface.available_tool_in_transaction(c, context).await?,
        TypedReadArgs::ModelPricing(input) => {
            surface.pricing_in_transaction(c, context, input).await?
        }
        _ => return read_in_transaction(c, context, args).await,
    };
    Ok(TypedReadResult {
        tool_name: args.name(),
        output,
    })
}

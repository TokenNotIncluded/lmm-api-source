//! Native consent creation for one browser-confirmed assistant connection.
//! The model chooses only published IDs. Limits arrive from the original
//! browser, and this module never commits or contacts a remote tool.
use super::{
    Error, Owner, Result,
    assistant_consent::{self, MarketConsentPreview},
    assistant_runtime::AssistantMarketRuntime,
    credit_access::{ConsentReceipt, ConsentRequest, CreditStore},
    credits::{self, Price, Quote},
    hash, name,
};
use crate::{
    assistant_model::agent::{FrozenProposal, market_policy::AssistantMarketPolicy},
    assistant_tools_core::{confirmation::BrowserActionProof, context::VerifiedToolContext},
};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use sqlx::{PgConnection, Row, types::Json};

pub(crate) const TOOL: &str = "connect_market_tool";
pub(crate) const VERSION: &str = "core-confirmed-actions-v1";
pub(crate) const SCHEMA: &str = r#"{"type":"object","additionalProperties":false,"properties":{"service_id":{"type":"string","minLength":1,"maxLength":128},"tool_id":{"type":"string","minLength":1,"maxLength":128},"version_id":{"type":"string","minLength":1,"maxLength":128}},"required":["service_id","tool_id","version_id"]}"#;

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub(crate) struct ConnectArguments {
    pub service_id: String,
    pub tool_id: String,
    pub version_id: String,
}
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub(crate) struct MarketConsentDraft {
    pub preview: MarketConsentPreview,
    pub price: Price,
}
/// These are browser fields. They are not part of the model tool schema.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub(crate) struct BrowserLimits {
    pub max_price_quota: i64,
    pub max_total_quota: i64,
    pub max_calls: i64,
    pub lifetime_seconds: i64,
}
impl BrowserLimits {
    fn validate(&self) -> Result<()> {
        if !(0..=credits::MAX_CREDITS).contains(&self.max_price_quota)
            || !(0..=credits::MAX_CREDITS).contains(&self.max_total_quota)
            || self.max_total_quota < self.max_price_quota
            || !(1..=100).contains(&self.max_calls)
            || !(60..=86_400).contains(&self.lifetime_seconds)
        {
            return Err(Error::Invalid);
        }
        Ok(())
    }
}
fn context_error(error: crate::assistant_tools_core::Error) -> Error {
    match error {
        crate::assistant_tools_core::Error::Invalid => Error::Invalid,
        crate::assistant_tools_core::Error::Unauthorized => Error::Unauthorized,
        crate::assistant_tools_core::Error::Conflict => Error::Conflict,
        crate::assistant_tools_core::Error::Unavailable => Error::Storage,
        _ => Error::Forbidden,
    }
}
fn policy_from_proposal(proof: &FrozenProposal) -> Result<AssistantMarketPolicy> {
    if proof.tool_name() != TOOL
        || proof.tool_version() != VERSION
        || proof.domain() != "registered_core"
        || !proof
            .host_policy()
            .get("allowed_tools")
            .and_then(Value::as_array)
            .is_some_and(|tools| tools.iter().any(|v| v.as_str() == Some(TOOL)))
    {
        return Err(Error::Forbidden);
    }
    serde_json::from_value(
        proof
            .host_policy()
            .get("market")
            .cloned()
            .ok_or(Error::Forbidden)?,
    )
    .map_err(|_| Error::Forbidden)
}
pub(crate) async fn prepare(
    c: &mut PgConnection,
    context: &VerifiedToolContext,
    proof: &FrozenProposal,
) -> Result<MarketConsentDraft> {
    let facts = context.revalidate(c).await.map_err(context_error)?;
    if facts.browser_session.is_none()
        || facts.browser_session.as_deref() != proof.source_web_session_sid()
        || proof.arguments_json().len() > 2048
    {
        return Err(Error::Forbidden);
    }
    let policy = policy_from_proposal(proof)?;
    let args: ConnectArguments =
        serde_json::from_slice(proof.arguments_json()).map_err(|_| Error::Invalid)?;
    read_draft(c, context, proof.operation_id(), &args, &policy).await
}
async fn read_draft(
    c: &mut PgConnection,
    context: &VerifiedToolContext,
    operation: &str,
    args: &ConnectArguments,
    policy: &AssistantMarketPolicy,
) -> Result<MarketConsentDraft> {
    if ![operation, &args.service_id, &args.tool_id, &args.version_id]
        .into_iter()
        .all(name)
    {
        return Err(Error::Invalid);
    }
    // A connection is to an actual remote publisher, never an invented builtin.
    // The underlying quote has one immutable row per exact tool/version.
    let row = sqlx::query(
        "SELECT q.snapshot FROM core_tool_market.credit_quotes q
         JOIN core_tool_market.publishers p ON p.user_id=q.seller_user_id
         JOIN core_identity.users u ON u.id=q.seller_user_id
         JOIN core_identity.accounts a ON a.id=q.seller_account_id
         WHERE q.snapshot->>'service_id'=$1 AND q.snapshot->>'tool_id'=$2
           AND q.snapshot->>'version_id'=$3 AND q.approved AND NOT q.suspended
           AND p.active AND u.active AND a.active FOR SHARE OF q",
    )
    .bind(&args.service_id)
    .bind(&args.tool_id)
    .bind(&args.version_id)
    .fetch_optional(&mut *c)
    .await?
    .ok_or(Error::Forbidden)?;
    let Json(quote): Json<Quote> = row.try_get("snapshot")?;
    let ceiling = quote.price.ceiling()?;
    let request = ConsentRequest {
        owner: Owner {
            user: context.user_id().to_string(),
            account: context.user_id().to_string(),
        },
        operation_id: operation.into(),
        client_id: "builtin-assistant".into(),
        tool_id: args.tool_id.clone(),
        version_id: args.version_id.clone(),
        max_price_quota: ceiling,
        max_total_quota: ceiling,
        max_calls: 1,
        // This preview names the immutable quote's maximum expiry. It creates
        // no grant. A separate required browser lifetime replaces it at commit.
        expires_at: quote.expires,
    };
    let preview = assistant_consent::build_preview(c, context, &request, policy).await?;
    if preview.service_id != args.service_id || preview.quote_id != quote.id {
        return Err(Error::Conflict);
    }
    Ok(MarketConsentDraft {
        preview,
        price: quote.price,
    })
}

/// BrowserActionProof has private fields and no Deserialize. Only the browser
/// coordinator constructs it after original Session, CSRF, prepared provider
/// evidence, expiry and grant revocation checks. The pending row is still locked
/// here; its approved state and native receipt commit with the consent below.
pub(crate) async fn apply(
    c: &mut PgConnection,
    context: &VerifiedToolContext,
    authority: &BrowserActionProof,
    draft: &MarketConsentDraft,
    limits: &BrowserLimits,
    runtime: &AssistantMarketRuntime,
) -> Result<ConsentReceipt> {
    limits.validate()?;
    let facts = context.revalidate(c).await.map_err(context_error)?;
    let policy = runtime.require(TOOL)?;
    let row = sqlx::query(
        "SELECT t.host_policy,o.arguments_json,q.preview
         FROM core_assistant_tools.confirmations q
         JOIN core_assistant.agent_tool_operations o ON o.id=q.operation_id
         JOIN core_assistant.agent_turns t ON t.id=o.turn_id
         WHERE q.id=$1 AND q.operation_id=$2 AND q.state='pending'
           AND q.tool_name='connect_market_tool' AND o.tool_name=q.tool_name
           AND o.domain='registered_core' AND o.state='completed'
           AND q.actor_user_id=$3 AND q.account_id=$4 AND q.credential_id=$5
           AND q.auth_version=$6 AND q.web_session_sid=$7
           AND t.owner_user_id=$3 AND t.owner_account_id=$4 AND t.session_id=$5
           AND t.auth_version=$6 AND t.source_web_session_sid=$7 AND q.expires_at>$8",
    )
    .bind(authority.confirmation_id())
    .bind(authority.operation_id())
    .bind(facts.user)
    .bind(facts.account)
    .bind(context.credential_id())
    .bind(facts.auth_version)
    .bind(facts.browser_session.as_deref())
    .bind(authority.confirmed_at_unix())
    .fetch_optional(&mut *c)
    .await?
    .ok_or(Error::Forbidden)?;
    let frozen: Value = row.try_get("host_policy")?;
    let current_policy = serde_json::to_value(policy).map_err(|_| Error::Storage)?;
    if frozen.get("market") != Some(&current_policy)
        || !frozen
            .get("allowed_tools")
            .and_then(Value::as_array)
            .is_some_and(|tools| tools.iter().any(|v| v.as_str() == Some(TOOL)))
        || row.try_get::<Value, _>("preview")?.get("market")
            != Some(&serde_json::to_value(draft).map_err(|_| Error::Storage)?)
    {
        return Err(Error::Forbidden);
    }
    let args: ConnectArguments =
        serde_json::from_slice(&row.try_get::<Vec<u8>, _>("arguments_json")?)
            .map_err(|_| Error::Storage)?;
    if read_draft(c, context, authority.operation_id(), &args, policy).await? != *draft {
        return Err(Error::Conflict);
    }
    let ceiling = draft.price.ceiling()?;
    let expires = authority
        .confirmed_at_unix()
        .checked_add(limits.lifetime_seconds)
        .ok_or(Error::Invalid)?;
    if limits.max_price_quota < ceiling
        || expires > draft.preview.request.expires_at
        || authority.operation_id() != draft.preview.request.operation_id
    {
        return Err(Error::Forbidden);
    }
    let mut request = draft.preview.request.clone();
    request.max_price_quota = limits.max_price_quota;
    request.max_total_quota = limits.max_total_quota;
    request.max_calls = limits.max_calls;
    request.expires_at = expires;
    sqlx::query("SELECT pg_advisory_xact_lock(hashtextextended($1,0))")
        .bind(format!(
            "credit-access:{}:{}",
            facts.user,
            authority.operation_id()
        ))
        .execute(&mut *c)
        .await?;
    if CreditStore::prior_in_connection(
        c,
        facts.user,
        authority.operation_id(),
        "consent-create",
        &request,
    )
    .await?
    .is_some()
    {
        // A committed approval replays its immutable coordinator receipt. A
        // prior grant while this row is pending is not a valid partial retry.
        return Err(Error::Conflict);
    }
    let id = hash(&[
        "credit-consent-v2",
        &request.owner.user,
        &request.operation_id,
    ])?;
    sqlx::query(
        "INSERT INTO core_tool_market.credit_consents
         (id,actor_user_id,payer_account_id,quote_id,request,created_at)
         VALUES($1,$2,$3,$4,$5,$6)",
    )
    .bind(&id)
    .bind(facts.user)
    .bind(facts.account)
    .bind(&draft.preview.quote_id)
    .bind(Json(&request))
    .bind(authority.confirmed_at_unix())
    .execute(&mut *c)
    .await?;
    CreditStore::record_in_connection(
        c,
        facts.user,
        authority.operation_id(),
        "consent-create",
        &request,
        &id,
        authority.confirmed_at_unix(),
        Some(json!({
            "assistant_confirmation_id": authority.confirmation_id(),
            "assistant_tool_operation_id": authority.operation_id(),
            "policy_revision": policy.revision,
        })),
    )
    .await?;
    let receipt = CreditStore::consent_in_connection(c, &id, facts.user).await?;
    context.revalidate(c).await.map_err(context_error)?;
    Ok(receipt)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn browser_limits_and_model_identifiers_are_separate_closed_inputs() {
        let valid = BrowserLimits {
            max_price_quota: 3,
            max_total_quota: 9,
            max_calls: 3,
            lifetime_seconds: 600,
        };
        assert!(valid.validate().is_ok());
        for limits in [
            BrowserLimits {
                max_price_quota: -1,
                ..valid.clone()
            },
            BrowserLimits {
                max_total_quota: 2,
                ..valid.clone()
            },
            BrowserLimits {
                max_calls: 0,
                ..valid.clone()
            },
            BrowserLimits {
                max_calls: 101,
                ..valid.clone()
            },
            BrowserLimits {
                lifetime_seconds: 59,
                ..valid.clone()
            },
            BrowserLimits {
                lifetime_seconds: 86_401,
                ..valid.clone()
            },
        ] {
            assert!(limits.validate().is_err());
        }
        assert!(
            serde_json::from_str::<ConnectArguments>(
                r#"{"service_id":"s","tool_id":"t","version_id":"v","max_calls":1}"#,
            )
            .is_err()
        );
        assert!(serde_json::from_str::<BrowserLimits>(
            r#"{"max_price_quota":3.5,"max_total_quota":9,"max_calls":3,"lifetime_seconds":600}"#,
        ).is_err());
        assert!(serde_json::from_str::<BrowserLimits>(
            r#"{"max_price_quota":3,"max_total_quota":null,"max_calls":3,"lifetime_seconds":600}"#,
        ).is_err());
    }
}

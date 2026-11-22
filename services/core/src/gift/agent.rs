//! Gift issuance from a real, settled Agent model proposal. No caller can
//! construct model provenance, choose the recipient, or supply a ledger result.
use super::{Error, GiftDecision};
use crate::{
    assistant_model::{
        self,
        agent::{
            FrozenProposal,
            registry::{InstalledTool, ToolDomain},
        },
    },
    assistant_tools_core::context::VerifiedToolContext,
};
use serde::Deserialize;
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use sqlx::{PgConnection, Row};

pub const SCHEMA: &str = include_str!("../../schema/assistant_agent_gift.sql");
pub(crate) const TOOL_NAME: &str = "prepare_new_user_gift";
pub(crate) const TOOL_VERSION: &str = "core-agent-gift-v1";
pub(crate) const TOOL_SCHEMA: &str = r#"{"type":"object","additionalProperties":false,"properties":{"amount_credits":{"type":"integer","minimum":0,"maximum":9007199254740991},"reason":{"type":"string","maxLength":240}},"required":["amount_credits"]}"#;

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Arguments {
    amount_credits: i64,
    #[serde(default)]
    reason: String,
}

fn arguments(bytes: &[u8]) -> Result<Arguments, Error> {
    let args: Arguments = serde_json::from_slice(bytes).map_err(|_| Error::Invalid)?;
    if !(0..=9_007_199_254_740_991).contains(&args.amount_credits)
        || args.reason.chars().count() > 240
        || args.reason.chars().any(char::is_control)
    {
        return Err(Error::Invalid);
    }
    Ok(args)
}

pub(crate) struct ToolResult {
    pub result: Value,
    pub domain_receipt_id: String,
}
impl ToolResult {
    fn not_issued(operation: &str, reason: &str) -> Self {
        Self {
            result: json!({"ok":false,"issued":false,"error":"gift_not_issued","reason":reason}),
            domain_receipt_id: format!("core-gift-denied-{operation}"),
        }
    }
}

pub(crate) fn validate_installed_tool(tool: &InstalledTool) -> Result<(), assistant_model::Error> {
    if tool.name != TOOL_NAME
        || tool.domain != ToolDomain::RegisteredCore
        || tool.version != TOOL_VERSION
        || tool.schema_sha256 != <[u8; 32]>::from(Sha256::digest(TOOL_SCHEMA.as_bytes()))
    {
        return Err(assistant_model::Error::Forbidden);
    }
    Ok(())
}

pub(crate) async fn check_ready(c: &mut PgConnection) -> Result<(), assistant_model::Error> {
    let triggers: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM pg_catalog.pg_trigger t
         JOIN pg_catalog.pg_class r ON r.oid=t.tgrelid
         JOIN pg_catalog.pg_namespace n ON n.oid=r.relnamespace
         WHERE NOT t.tgisinternal AND t.tgenabled='O' AND t.tgdeferrable AND t.tginitdeferred
           AND ((n.nspname='core_gift' AND r.relname='decisions' AND t.tgname='agent_gift_decision_binding')
             OR (n.nspname='core_assistant' AND r.relname='agent_tool_operations' AND t.tgname='agent_gift_tool_binding'))",
    ).fetch_one(&mut *c).await?;
    let unique: bool = sqlx::query_scalar(
        "SELECT EXISTS(SELECT FROM pg_catalog.pg_index i
         JOIN pg_catalog.pg_class x ON x.oid=i.indexrelid
         JOIN pg_catalog.pg_namespace n ON n.oid=x.relnamespace
         WHERE n.nspname='core_assistant' AND x.relname='agent_gift_one_issued_receipt'
           AND i.indisunique AND i.indisvalid AND i.indisready AND i.indpred IS NOT NULL)",
    )
    .fetch_one(&mut *c)
    .await?;
    if triggers != 2 || !unique {
        return Err(assistant_model::Error::Unavailable);
    }
    Ok(())
}

/// The worker owns this transaction. Its tool completion is committed together
/// with the gift decision and canonical journal, enforced again by component83.
pub(crate) async fn execute(
    c: &mut PgConnection,
    context: &VerifiedToolContext,
    proposal: &FrozenProposal,
) -> Result<ToolResult, assistant_model::Error> {
    sqlx::query("SAVEPOINT agent_gift_action")
        .execute(&mut *c)
        .await?;
    let attempted = issue(c, context, proposal).await;
    match attempted {
        Ok(result) => {
            sqlx::query("RELEASE SAVEPOINT agent_gift_action")
                .execute(&mut *c)
                .await?;
            Ok(result)
        }
        Err(
            Error::Invalid
            | Error::Unauthorized
            | Error::Forbidden
            | Error::NotFound
            | Error::Conflict,
        ) => {
            sqlx::query("ROLLBACK TO SAVEPOINT agent_gift_action")
                .execute(&mut *c)
                .await?;
            sqlx::query("RELEASE SAVEPOINT agent_gift_action")
                .execute(&mut *c)
                .await?;
            Ok(ToolResult::not_issued(
                proposal.operation_id(),
                "authority_or_eligibility_denied",
            ))
        }
        Err(Error::Unavailable) => Err(assistant_model::Error::Unavailable),
    }
}

async fn issue(
    c: &mut PgConnection,
    context: &VerifiedToolContext,
    proposal: &FrozenProposal,
) -> Result<ToolResult, Error> {
    if proposal.tool_name() != TOOL_NAME
        || proposal.tool_version() != TOOL_VERSION
        || proposal.domain() != "registered_core"
        || !proposal.resource_kind().is_empty()
        || !proposal.resource_id().is_empty()
        || !proposal.scope_grant_id().is_empty()
    {
        return Err(Error::Forbidden);
    }
    let args = arguments(proposal.arguments_json())?;
    context.revalidate(c).await.map_err(|error| match error {
        crate::assistant_tools_core::Error::Unavailable => Error::Unavailable,
        _ => Error::Forbidden,
    })?;
    // This is deliberately separate from PersonalText's streaming source. The
    // round is already settled, and the exact paid provider call is immutable.
    let row = sqlx::query(
        "SELECT t.conversation_id,r.model_request_id
         FROM core_assistant.agent_tool_operations o
         JOIN core_assistant.agent_turns t ON t.id=o.turn_id
         JOIN core_assistant.agent_rounds r ON r.id=o.round_id AND r.turn_id=t.id
         JOIN core_charging.requests q ON q.id=r.model_request_id
         JOIN core_assistant.grants g ON g.id=q.assistant_grant_id
         JOIN core_assistant.operations original ON original.request_id=q.id AND original.grant_id=g.id
         WHERE o.id=$1 AND o.turn_id=$2 AND o.state='pending'
           AND o.domain='registered_core' AND o.tool_name='prepare_new_user_gift' AND o.tool_version=$3
           AND o.schema_sha256=$4 AND o.arguments_json=$5 AND o.arguments_sha256=sha256(o.arguments_json)
           AND o.resource_kind='' AND o.resource_id='' AND o.resource_version='' AND o.scope_grant_id=''
           AND o.resource_schema_sha256 IS NULL AND o.resource_price_sha256 IS NULL
           AND t.owner_user_id=$6 AND t.owner_account_id=$7 AND t.session_id=$8
           AND t.phase='tools_pending' AND t.expires_at>extract(epoch FROM clock_timestamp())::bigint
           AND t.host_policy=$9 AND (t.host_policy->'allowed_tools') ? 'prepare_new_user_gift'
           AND (t.host_policy->'model'->>'gift_tools')::boolean IS TRUE
           AND r.state='settled' AND q.state='settled' AND q.outcome='completed'
           AND r.provider_response_sha256=sha256(r.provider_response_json)
           AND q.actor_user_id=t.owner_user_id AND q.owner_account_id=t.owner_account_id
           AND g.conversation_id=t.conversation_id AND g.session_id=t.session_id AND g.auth_version=t.auth_version
           AND NOT g.revoked AND g.expires_at>extract(epoch FROM clock_timestamp())::bigint
           AND g.policy=t.host_policy->'model' AND (g.policy->>'gift_tools')::boolean IS TRUE
           AND EXISTS(SELECT FROM jsonb_array_elements(convert_from(r.provider_response_json,'UTF8')::jsonb #> '{choices,0,message,tool_calls}') call
             WHERE call->>'id'=o.provider_call_id AND call#>>'{function,name}'=o.tool_name
               AND convert_to(call#>>'{function,arguments}','UTF8')=o.arguments_json)
           AND (q.payer_account_id=t.owner_account_id
             OR q.payer_account_id=(g.policy->'sponsor'->>'treasury_account_id')::bigint
             OR (q.payer_account_id=(g.policy->'service_funding'->>'payer_account_id')::bigint AND EXISTS(
               SELECT FROM core_identity.users payer JOIN core_identity.accounts account ON account.id=payer.personal_account_id
               WHERE payer.id=(g.policy->'service_funding'->>'payer_user_id')::bigint
                 AND account.id=q.payer_account_id AND account.kind='personal')))
         FOR SHARE OF g",
    ).bind(proposal.operation_id()).bind(proposal.turn_id()).bind(TOOL_VERSION)
        .bind(Sha256::digest(TOOL_SCHEMA.as_bytes()).to_vec()).bind(proposal.arguments_json())
        .bind(context.user_id()).bind(context.account_id()).bind(context.credential_id())
        .bind(proposal.host_policy()).fetch_optional(&mut *c).await?.ok_or(Error::Forbidden)?;
    super::lock(c).await?;
    if super::read(c, context.user_id()).await?.is_some() {
        return Ok(ToolResult::not_issued(
            proposal.operation_id(),
            "already_exists",
        ));
    }
    if args.amount_credits == 0 {
        return Ok(ToolResult::not_issued(
            proposal.operation_id(),
            "zero_amount",
        ));
    }
    let conversation: String = row.try_get("conversation_id")?;
    let request: String = row.try_get("model_request_id")?;
    let receipt = super::issue_locked(
        c,
        context.user_id(),
        &conversation,
        &request,
        GiftDecision {
            amount_credits: args.amount_credits,
            reason: args.reason,
        },
    )
    .await?
    .ok_or(Error::Unavailable)?;
    let journal = receipt.ledger_transaction_id.ok_or(Error::Unavailable)?;
    let claimed = receipt.claimed_at_unix.ok_or(Error::Unavailable)?;
    if receipt.owner_user_id != context.user_id() || receipt.credit_amount != args.amount_credits {
        return Err(Error::Unavailable);
    }
    Ok(ToolResult {
        result: json!({"ok":true,"issued":true,"gift_id":receipt.gift_id,
            "credit_amount":receipt.credit_amount,"ledger_transaction_id":journal,"claimed_at_unix":claimed}),
        domain_receipt_id: format!("core-gift-{}", receipt.gift_id),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn typed_agent_gift_accepts_only_bounded_amount_and_optional_plain_reason() {
        assert_eq!(
            arguments(br#"{"amount_credits":0}"#)
                .unwrap()
                .amount_credits,
            0
        );
        assert_eq!(
            arguments(br#"{"amount_credits":25,"reason":"welcome"}"#)
                .unwrap()
                .amount_credits,
            25
        );
        for raw in [
            br#"{"amount_credits":25,"owner_user_id":8}"#.as_slice(),
            br#"{"amount_credits":25,"payer_account_id":3}"#,
            br#"{"amount_credits":25,"ledger_transaction_id":1}"#,
            br#"{"amount_credits":25,"amount_credits":50}"#,
            br#"{"amount_credits":1.5}"#,
            br#"{"amount_credits":-1}"#,
            br#"{"amount_credits":9007199254740992}"#,
            br#"{"amount_credits":25,"reason":"hidden\ncommand"}"#,
            br#"{"reason":"missing amount"}"#,
        ] {
            assert!(
                arguments(raw).is_err(),
                "must reject {}",
                String::from_utf8_lossy(raw)
            );
        }
        let too_long =
            serde_json::to_vec(&json!({"amount_credits":25,"reason":"x".repeat(241)})).unwrap();
        assert!(arguments(&too_long).is_err());
    }
}

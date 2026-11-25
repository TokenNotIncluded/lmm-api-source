//! Only the real durable worker can create a weekly decision.
use super::{Error, VerifiedToolContext, lock_owner, now, policy, projection, week_start};
use crate::assistant_model::{
    self,
    agent::{
        FrozenProposal,
        registry::{InstalledTool, ToolDomain},
    },
};
use futures_util::TryStreamExt;
use rand::Rng;
use serde::Deserialize;
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use sqlx::{PgConnection, Row};

pub(crate) const TOOL_NAME: &str = "prepare_weekly_discount";
pub(crate) const TOOL_VERSION: &str = "core-weekly-discount-v1";
pub(crate) const TOOL_SCHEMA: &str = r#"{"type":"object","additionalProperties":false,"properties":{"discount_percent":{"type":"integer","minimum":0,"maximum":99},"reason":{"type":"string","minLength":2,"maxLength":240}},"required":["discount_percent","reason"]}"#;
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Arguments {
    discount_percent: i16,
    reason: String,
}
fn arguments(bytes: &[u8]) -> Result<Arguments, Error> {
    let mut args: Arguments = serde_json::from_slice(bytes).map_err(|_| Error::Invalid)?;
    args.reason = args.reason.trim().to_owned();
    if !(0..=99).contains(&args.discount_percent)
        || !(2..=240).contains(&args.reason.chars().count())
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
pub(crate) async fn execute(
    c: &mut PgConnection,
    context: &VerifiedToolContext,
    proposal: &FrozenProposal,
) -> Result<ToolResult, assistant_model::Error> {
    sqlx::query("SAVEPOINT agent_weekly_discount")
        .execute(&mut *c)
        .await?;
    match prepare(c, context, proposal).await {
        Ok(result) => {
            sqlx::query("RELEASE SAVEPOINT agent_weekly_discount")
                .execute(c)
                .await?;
            Ok(result)
        }
        Err(Error::Unavailable) => Err(assistant_model::Error::Unavailable),
        Err(_) => {
            sqlx::query("ROLLBACK TO SAVEPOINT agent_weekly_discount")
                .execute(&mut *c)
                .await?;
            sqlx::query("RELEASE SAVEPOINT agent_weekly_discount")
                .execute(c)
                .await?;
            Ok(ToolResult {
                result: json!({"ok":false,"created":false,"status":"not_prepared","error":"weekly_discount_not_eligible"}),
                domain_receipt_id: format!("core-weekly-denied-{}", proposal.operation_id()),
            })
        }
    }
}
async fn prepare(
    c: &mut PgConnection,
    context: &VerifiedToolContext,
    proposal: &FrozenProposal,
) -> Result<ToolResult, Error> {
    if proposal.tool_name() != TOOL_NAME
        || proposal.tool_version() != TOOL_VERSION
        || proposal.domain() != "registered_core"
        || !proposal.resource_kind().is_empty()
        || !proposal.resource_id().is_empty()
        || !proposal.resource_version().is_empty()
        || !proposal.scope_grant_id().is_empty()
    {
        return Err(Error::Forbidden);
    }
    let args = arguments(proposal.arguments_json())?;
    let actor = context.revalidate(c).await?;
    let family = actor.browser_session.as_deref().ok_or(Error::Forbidden)?;
    if proposal.source_web_session_sid() != Some(family) {
        return Err(Error::Forbidden);
    }
    let row=sqlx::query(
        "SELECT t.conversation_id,t.original_text
         FROM core_assistant.agent_tool_operations o
         JOIN core_assistant.agent_turns t ON t.id=o.turn_id
         JOIN core_assistant.agent_rounds r ON r.id=o.round_id AND r.turn_id=t.id
         JOIN core_charging.requests q ON q.id=r.model_request_id
         JOIN core_assistant.grants g ON g.id=q.assistant_grant_id
         JOIN core_assistant.operations original ON original.request_id=q.id AND original.grant_id=g.id
         WHERE o.id=$1 AND o.turn_id=$2 AND o.state='pending'
           AND o.domain='registered_core' AND o.tool_name='prepare_weekly_discount' AND o.tool_version=$3
           AND o.schema_sha256=$4 AND o.arguments_json=$5 AND o.arguments_sha256=sha256(o.arguments_json)
           AND o.resource_kind='' AND o.resource_id='' AND o.resource_version='' AND o.scope_grant_id=''
           AND o.resource_schema_sha256 IS NULL AND o.resource_price_sha256 IS NULL
           AND t.owner_user_id=$6 AND t.owner_account_id=$7 AND t.session_id=$8
           AND t.source_web_session_sid=$10 AND t.auth_version=$11
           AND t.phase='tools_pending' AND t.expires_at>extract(epoch FROM clock_timestamp())::bigint
           AND t.host_policy=$9 AND (t.host_policy->'allowed_tools') ? 'prepare_weekly_discount'
           AND r.state='settled' AND q.state='settled' AND q.outcome='completed'
           AND r.provider_response_sha256=sha256(r.provider_response_json)
           AND q.actor_user_id=t.owner_user_id AND q.owner_account_id=t.owner_account_id
           AND g.conversation_id=t.conversation_id AND g.session_id=t.session_id AND g.auth_version=t.auth_version
           AND NOT g.revoked AND g.expires_at>extract(epoch FROM clock_timestamp())::bigint
           AND g.policy=t.host_policy->'model'
           AND EXISTS(SELECT FROM jsonb_array_elements(convert_from(r.provider_response_json,'UTF8')::jsonb #> '{choices,0,message,tool_calls}') call
              WHERE call->>'id'=o.provider_call_id AND call#>>'{function,name}'=o.tool_name
                AND convert_to(call#>>'{function,arguments}','UTF8')=o.arguments_json)
           AND (q.payer_account_id=t.owner_account_id
             OR q.payer_account_id=(g.policy->'sponsor'->>'treasury_account_id')::bigint
             OR (q.payer_account_id=(g.policy->'service_funding'->>'payer_account_id')::bigint AND EXISTS(
                SELECT FROM core_identity.users payer JOIN core_identity.accounts account ON account.id=payer.personal_account_id
                WHERE payer.id=(g.policy->'service_funding'->>'payer_user_id')::bigint
                  AND account.id=q.payer_account_id AND account.kind='personal')))
         FOR SHARE OF g")
        .bind(proposal.operation_id()).bind(proposal.turn_id()).bind(TOOL_VERSION)
        .bind(Sha256::digest(TOOL_SCHEMA.as_bytes()).to_vec()).bind(proposal.arguments_json())
        .bind(actor.user).bind(actor.account).bind(context.credential_id()).bind(proposal.host_policy())
        .bind(family).bind(actor.auth_version).fetch_optional(&mut *c).await?.ok_or(Error::Forbidden)?;
    let conversation: String = row.try_get("conversation_id")?;
    let original_text: String = row.try_get("original_text")?;
    if super::intent::preserves_status_only(&original_text) {
        return Ok(ToolResult {
            result: json!({"ok":false,"created":false,"read_only":true,"error":"weekly_status_read_required"}),
            domain_receipt_id: format!("core-weekly-denied-{}", proposal.operation_id()),
        });
    }
    lock_owner(c, actor.user).await?;
    let stamp = now(c).await?;
    let week = week_start(stamp);
    if let Some(old) =
        sqlx::query("SELECT * FROM core_weekly.weekly_discounts WHERE user_id=$1 AND week_start=$2")
            .bind(actor.user)
            .bind(week)
            .fetch_optional(&mut *c)
            .await?
    {
        return Ok(ToolResult {
            result: json!({"ok":true,"created":false,"decision":projection(&old)?}),
            domain_receipt_id: format!("core-weekly-existing-{}", proposal.operation_id()),
        });
    }
    let (enabled, maximum) = policy(c).await?;
    if actor.role != "user" || !enabled || args.discount_percent > maximum {
        return Err(Error::Forbidden);
    }
    // Count original owned user turns once, never summaries, provider messages,
    // claimed turn counts, duplicate model rounds or an unsettled request.
    let mut texts=sqlx::query_scalar::<_,String>(
        "SELECT t.original_text FROM core_assistant.agent_turns t
         WHERE t.conversation_id=$1 AND t.owner_user_id=$2 AND t.owner_account_id=$3
           AND EXISTS(SELECT FROM core_assistant.agent_rounds r
             JOIN core_charging.requests q ON q.id=r.model_request_id
             JOIN core_assistant.operations original ON original.request_id=q.id
             WHERE r.turn_id=t.id AND r.state='settled' AND q.state='settled' AND q.outcome='completed'
               AND q.actor_user_id=t.owner_user_id AND q.owner_account_id=t.owner_account_id)
         ORDER BY t.history_position DESC")
        .bind(&conversation).bind(actor.user).bind(actor.account).fetch(&mut *c);
    let mut substantive = 0;
    while let Some(text) = texts.try_next().await? {
        if text.trim().chars().count() >= 4 {
            substantive += 1;
        }
        if substantive == 2 {
            break;
        }
    }
    drop(texts);
    if substantive < 2 {
        return Err(Error::Forbidden);
    }
    let mut bytes = [0u8; 32];
    rand::rng().fill_bytes(&mut bytes);
    let id = crate::hex_codec::encode(bytes);
    let status = if args.discount_percent == 0 {
        "declined"
    } else {
        "offered"
    };
    let decision=sqlx::query(
        "INSERT INTO core_weekly.weekly_discounts(id,user_id,account_id,week_start,status,discount_percent,reason,
          conversation_id,operation_id,credential_id,auth_version,web_session_sid,created_at)
         VALUES($1,$2,$3,$4,$5,$6,$7,$8,$9,$10,$11,$12,$13) RETURNING *")
        .bind(&id).bind(actor.user).bind(actor.account).bind(week).bind(status).bind(args.discount_percent)
        .bind(args.reason).bind(conversation).bind(proposal.operation_id()).bind(context.credential_id())
        .bind(actor.auth_version).bind(family).bind(stamp).fetch_one(c).await?;
    Ok(ToolResult {
        result: json!({"ok":true,"created":true,"decision":projection(&decision)?}),
        domain_receipt_id: format!("core-weekly-{id}"),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn typed_weekly_arguments_include_zero_but_never_actor_or_turn_claims() {
        assert_eq!(
            arguments(br#"{"discount_percent":0,"reason":"No offer"}"#)
                .unwrap()
                .discount_percent,
            0
        );
        for raw in [
            br#"{"discount_percent":100,"reason":"Too high"}"#.as_slice(),
            br#"{"discount_percent":-1,"reason":"Negative"}"#,
            br#"{"discount_percent":5.5,"reason":"Fraction"}"#,
            br#"{"discount_percent":5,"reason":"x"}"#,
            br#"{"discount_percent":5,"reason":"Valid","user_id":1}"#,
            br#"{"discount_percent":5,"reason":"Valid","substantive_turns":2}"#,
            br#"{"discount_percent":5,"discount_percent":10,"reason":"Duplicate"}"#,
        ] {
            assert!(arguments(raw).is_err());
        }
    }
}

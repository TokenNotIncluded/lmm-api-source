//! Fixed identity preparations. A model result only creates a preview;
//! the original HttpOnly browser Session executes it later in one transaction.
mod prepared;
#[cfg(test)]
mod tests;
use super::{Error, confirmation, context::VerifiedToolContext};
use crate::{
    assistant_model::{
        self,
        agent::{
            FrozenProposal,
            registry::{InstalledTool, ToolDomain},
        },
    },
    identity::assistant_actions::{self, Draft, KeyAction},
};
use serde::Deserialize;
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use sqlx::{PgConnection, Row};

pub(crate) const VERSION: &str = "core-confirmed-actions-v1";
pub(crate) const KEY_TOOL: &str = "prepare_api_key_action";
pub(crate) const CREATE_TOOL: &str = "request_create_key";
pub(crate) const CREATE_VERSION: &str = "core-key-create-v1";
pub(crate) const CREATE_SCHEMA: &str = r#"{"type":"object","additionalProperties":false,"properties":{"name":{"type":"string","minLength":1,"maxLength":128},"group":{"type":"string","minLength":1,"maxLength":128}},"required":["group"]}"#;
pub(crate) const PROFILE_TOOL: &str = "prepare_user_action";
pub(crate) const KEY_SCHEMA: &str = r#"{"type":"object","additionalProperties":false,"properties":{"action":{"type":"string","enum":["disable","delete"]},"token_id":{"type":"integer","minimum":1,"maximum":9007199254740991}},"required":["action","token_id"]}"#;
pub(crate) const PROFILE_SCHEMA: &str = r#"{"type":"object","additionalProperties":false,"properties":{"action":{"type":"string","const":"change_display_name"},"display_name":{"type":"string","minLength":1,"maxLength":20}},"required":["action","display_name"]}"#;

pub(crate) fn is_tool(name: &str) -> bool {
    matches!(
        name,
        KEY_TOOL | PROFILE_TOOL | CREATE_TOOL | crate::tool_market::assistant_connect::TOOL
    )
}
pub(crate) fn validate_installed_tool(tool: &InstalledTool) -> Result<(), assistant_model::Error> {
    let (schema, version) = descriptor(&tool.name).ok_or(assistant_model::Error::Forbidden)?;
    if tool.domain != ToolDomain::RegisteredCore
        || tool.version != version
        || tool.schema_sha256 != <[u8; 32]>::from(Sha256::digest(schema.as_bytes()))
    {
        return Err(assistant_model::Error::Forbidden);
    }
    Ok(())
}
pub(super) fn descriptor(name: &str) -> Option<(&'static str, &'static str)> {
    match name {
        KEY_TOOL => Some((KEY_SCHEMA, VERSION)),
        PROFILE_TOOL => Some((PROFILE_SCHEMA, VERSION)),
        CREATE_TOOL => Some((CREATE_SCHEMA, CREATE_VERSION)),
        crate::tool_market::assistant_connect::TOOL => Some((
            crate::tool_market::assistant_connect::SCHEMA,
            crate::tool_market::assistant_connect::VERSION,
        )),
        _ => None,
    }
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct CreateArgs {
    name: Option<String>,
    group: String,
}
impl CreateArgs {
    fn name(&self) -> &str {
        self.name.as_deref().unwrap_or("AI assistant key")
    }
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct KeyArgs {
    action: KeyAction,
    token_id: i64,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct ProfileArgs {
    action: String,
    display_name: String,
}

pub(crate) fn prepared_result(challenge: &confirmation::Challenge) -> Value {
    json!({
        "ok": true, "status": "prepared", "executed": false,
        "confirmation_id": challenge.challenge_id,
        "browser_path": challenge.browser_path,
        "draft_sha256": challenge.draft_sha256,
        "expires_at_unix": challenge.expires_at_unix
    })
}
pub(crate) async fn prepare(
    c: &mut PgConnection,
    context: &VerifiedToolContext,
    proof: &FrozenProposal,
) -> Result<Value, Error> {
    let draft = build_draft(c, context, proof).await?;
    let challenge = confirmation::prepare_in_transaction(c, context, proof, &draft).await?;
    Ok(prepared_result(&challenge))
}
pub(crate) async fn validate_draft(
    c: &mut PgConnection,
    context: &VerifiedToolContext,
    proof: &FrozenProposal,
    draft: &Value,
) -> Result<(), Error> {
    if &build_draft(c, context, proof).await? != draft {
        return Err(Error::Conflict);
    }
    Ok(())
}
async fn build_draft(
    c: &mut PgConnection,
    context: &VerifiedToolContext,
    proof: &FrozenProposal,
) -> Result<Value, Error> {
    if !is_tool(proof.tool_name())
        || descriptor(proof.tool_name()).is_none_or(|(_, v)| proof.tool_version() != v)
        || proof.domain() != "registered_core"
        || !proof
            .host_policy()
            .get("allowed_tools")
            .and_then(Value::as_array)
            .is_some_and(|names| names.iter().any(|n| n.as_str() == Some(proof.tool_name())))
        || proof.arguments_json().len() > 4096
    {
        return Err(Error::Forbidden);
    }
    if proof.tool_name() == crate::tool_market::assistant_connect::TOOL {
        let market = crate::tool_market::assistant_connect::prepare(c, context, proof)
            .await
            .map_err(confirmation::market::map_error)?;
        return serde_json::to_value(confirmation::market::Preview::MarketTool { market })
            .map_err(|_| Error::Unavailable);
    }
    let draft = match proof.tool_name() {
        CREATE_TOOL => {
            let args: CreateArgs =
                serde_json::from_slice(proof.arguments_json()).map_err(|_| Error::Invalid)?;
            assistant_actions::prepare_create_key(c, context, args.name(), &args.group).await?
        }
        KEY_TOOL => {
            let args: KeyArgs =
                serde_json::from_slice(proof.arguments_json()).map_err(|_| Error::Invalid)?;
            let draft =
                assistant_actions::prepare_key(c, context, args.token_id, args.action).await?;
            let Draft::ApiKey { before, .. } = &draft else {
                return Err(Error::Unavailable);
            };
            require_list_and_selection(c, context, proof, before).await?;
            draft
        }
        PROFILE_TOOL => {
            let args: ProfileArgs =
                serde_json::from_slice(proof.arguments_json()).map_err(|_| Error::Invalid)?;
            if args.action != "change_display_name" {
                return Err(Error::Invalid);
            }
            assistant_actions::prepare_display_name(c, context, &args.display_name).await?
        }
        _ => return Err(Error::Forbidden),
    };
    serde_json::to_value(draft).map_err(|_| Error::Unavailable)
}
fn selected_id(text: &str, id: i64) -> bool {
    let text = text.trim().to_lowercase();
    if text == id.to_string() {
        return true;
    }
    let pattern = format!(
        r"(?:\bid\s*[:：#]?\s*|#\s*|编号\s*[:：]?\s*|(?:删除|删掉|停用|禁用)\s*){id}(?:[^0-9]|$)"
    );
    regex::Regex::new(&pattern).is_ok_and(|r| r.is_match(&text))
}
fn selected_name(text: &str, name: &str) -> bool {
    let text = text.trim();
    if name.is_empty() {
        return false;
    }
    if text == name {
        return true;
    }
    for (left, right) in [
        ("\"", "\""),
        ("'", "'"),
        ("`", "`"),
        ("“", "”"),
        ("‘", "’"),
        ("「", "」"),
        ("『", "』"),
        ("《", "》"),
    ] {
        if text.contains(&format!("{left}{name}{right}")) {
            return true;
        }
    }
    if !name.is_ascii() {
        return false;
    }
    regex::Regex::new(&format!(r"(?:^|\s){}(?:\s|$)", regex::escape(name)))
        .is_ok_and(|r| r.is_match(text))
}
fn explicitly_declined(text: &str) -> bool {
    let lower = text.trim().to_lowercase();
    [
        "不要",
        "取消",
        "别删除",
        "别停用",
        "别禁用",
        "do not",
        "don't",
        "cancel",
        "never",
    ]
    .iter()
    .any(|word| lower.contains(word))
}
async fn require_list_and_selection(
    c: &mut PgConnection,
    context: &VerifiedToolContext,
    proof: &FrozenProposal,
    current: &assistant_actions::KeyState,
) -> Result<(), Error> {
    let row = sqlx::query(
        "SELECT old.result_json,old.result_sha256,t.original_text
         FROM core_assistant.agent_tool_operations current
         JOIN core_assistant.agent_turns t ON t.id=current.turn_id
         JOIN core_assistant.agent_rounds cr ON cr.id=current.round_id
         JOIN core_assistant.agent_turns previous ON previous.conversation_id=t.conversation_id
           AND previous.session_id=t.session_id AND previous.auth_version=t.auth_version
           AND previous.owner_user_id=t.owner_user_id AND previous.owner_account_id=t.owner_account_id
         JOIN core_assistant.agent_tool_operations old ON old.turn_id=previous.id
         JOIN core_assistant.agent_rounds pr ON pr.id=old.round_id
         WHERE current.id=$1 AND t.owner_user_id=$2 AND t.session_id=$3
           AND old.tool_name='list_my_api_keys' AND old.domain='registered_core' AND old.state='completed'
           AND (previous.history_position<t.history_position OR
             (previous.id=t.id AND (pr.round<cr.round OR (pr.id=cr.id AND old.ordinal<current.ordinal))))
         ORDER BY previous.history_position DESC,pr.round DESC,old.ordinal DESC LIMIT 1"
    ).bind(proof.operation_id()).bind(context.user_id()).bind(context.credential_id())
        .fetch_optional(&mut *c).await?.ok_or(Error::Forbidden)?;
    let raw: Vec<u8> = row.try_get("result_json")?;
    if Sha256::digest(&raw).as_slice() != row.try_get::<Vec<u8>, _>("result_sha256")? {
        return Err(Error::Unavailable);
    }
    let envelope: Value = serde_json::from_slice(&raw).map_err(|_| Error::Unavailable)?;
    if envelope.get("tool_name").and_then(Value::as_str) != Some("list_my_api_keys") {
        return Err(Error::Unavailable);
    }
    let result = envelope.get("output").ok_or(Error::Unavailable)?;
    if result.get("ok") != Some(&Value::Bool(true))
        || result.get("credentials_returned") != Some(&Value::Bool(false))
        || !result
            .get("keys")
            .and_then(Value::as_array)
            .is_some_and(|keys| {
                keys.iter().any(|key| {
                    key.get("id").and_then(Value::as_i64) == Some(current.id)
                        && key.get("name").and_then(Value::as_str) == Some(current.name.as_str())
                        && key.get("group").and_then(Value::as_str) == Some(current.group.as_str())
                        && key.get("status").and_then(Value::as_i64)
                            == Some(i64::from(current.status))
                })
            })
    {
        return Err(Error::Conflict);
    }
    let text: String = row.try_get("original_text")?;
    if explicitly_declined(&text) {
        return Err(Error::Forbidden);
    }
    if selected_id(&text, current.id) {
        return Ok(());
    }
    let count: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM core_identity.credentials k JOIN core_identity.api_key_settings s ON s.credential_id=k.id
         WHERE k.kind='api_key' AND k.revoked_at IS NULL AND k.user_id=$1 AND k.owner_account_id=$2 AND s.name=$3"
    ).bind(context.user_id()).bind(context.account_id()).bind(&current.name).fetch_one(c).await?;
    if count != 1 || !selected_name(&text, &current.name) {
        return Err(Error::Forbidden);
    }
    Ok(())
}
pub(crate) use prepared::verify as verify_prepared;
pub(crate) async fn check_ready(c: &mut PgConnection) -> Result<(), assistant_model::Error> {
    let valid: bool = sqlx::query_scalar(
        "SELECT has_table_privilege('core_assistant_tools.confirmations','SELECT')
          AND has_table_privilege('core_assistant_tools.confirmations','INSERT')
          AND NOT EXISTS(SELECT FROM unnest(ARRAY['execution_receipt','execution_sha256','executed_at']) col
            WHERE NOT has_column_privilege('core_assistant_tools.confirmations',col,'UPDATE'))
          AND (SELECT count(*) FROM pg_catalog.pg_trigger
            WHERE tgrelid='core_assistant_tools.confirmations'::regclass AND NOT tgisinternal AND tgenabled='O'
              AND tgname IN ('assistant_confirmation_immutable','assistant_confirmation_no_truncate','assistant_confirmation_execution_binding'))=3
          AND has_table_privilege('core_assistant_tools.key_creation_cards','SELECT')
          AND has_table_privilege('core_assistant_tools.key_creation_cards','INSERT')
          AND NOT EXISTS(SELECT FROM unnest(ARRAY['nonce_sha256','ciphertext','consumed_at']) col
            WHERE NOT has_column_privilege('core_assistant_tools.key_creation_cards',col,'UPDATE'))
          AND (SELECT count(*) FROM pg_catalog.pg_trigger
            WHERE tgrelid='core_assistant_tools.key_creation_cards'::regclass AND NOT tgisinternal AND tgenabled='O'
              AND tgname IN ('assistant_key_card_immutable','assistant_key_card_no_truncate','assistant_key_card_execution_binding'))=3
          AND EXISTS(SELECT FROM pg_catalog.pg_constraint
            WHERE conrelid='core_assistant_tools.confirmations'::regclass AND conname='assistant_confirmation_execution_shape' AND convalidated)"
    ).fetch_one(&mut *c).await?;
    sqlx::query("SELECT execution_receipt,execution_sha256,executed_at FROM core_assistant_tools.confirmations WHERE FALSE")
        .execute(c).await?;
    if !valid {
        return Err(assistant_model::Error::Unavailable);
    }
    Ok(())
}

pub(crate) async fn check_market_ready(c: &mut PgConnection) -> Result<(), assistant_model::Error> {
    let valid: bool = sqlx::query_scalar(
        "SELECT has_function_privilege('core_assistant_tools.market_confirmation_bound(text)','EXECUTE')
          AND has_table_privilege('core_tool_market.credit_consents','SELECT')
          AND has_table_privilege('core_tool_market.credit_consents','INSERT')
          AND has_table_privilege('core_tool_market.credit_access_operations','SELECT')
          AND has_table_privilege('core_tool_market.credit_access_operations','INSERT')
          AND (SELECT count(*) FROM pg_catalog.pg_trigger
             WHERE NOT tgisinternal AND tgenabled='O' AND
              ((tgrelid='core_tool_market.credit_consents'::regclass AND
                tgname='assistant_market_consent_confirmation_binding') OR
               (tgrelid='core_tool_market.credit_access_operations'::regclass AND
                tgname='assistant_market_access_confirmation_binding')))=2"
    ).fetch_one(c).await?;
    if !valid {
        return Err(assistant_model::Error::Unavailable);
    }
    Ok(())
}

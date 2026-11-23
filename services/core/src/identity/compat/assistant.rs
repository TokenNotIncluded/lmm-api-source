//! Fixed nonfinancial identity mutations for a Core browser-confirmed proposal.
//! This module owns no transaction and never accepts a bearer, role or payer
//! from model arguments. The browser coordinator commits its receipt with us.
use super::*;
mod create_key;
use crate::assistant_tools_core::context::VerifiedToolContext;
pub(crate) use create_key::{
    CreationState, consume_key_card, inspect_key_card, prepare_create_key,
};

#[derive(Clone, Copy, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub(crate) enum KeyAction {
    Disable,
    Delete,
}
impl KeyAction {
    pub(crate) fn name(self) -> &'static str {
        match self {
            Self::Disable => "disable",
            Self::Delete => "delete",
        }
    }
}
#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub(crate) struct KeyState {
    pub id: i64,
    pub name: String,
    pub group: String,
    pub enabled: bool,
    pub revoked: bool,
    pub status: i32,
    pub expires_at_unix: i64,
    pub requires_two_factor: bool,
    pub mfa_revision: i64,
}
#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub(crate) enum Draft {
    CreateKey {
        before: CreationState,
    },
    ApiKey {
        action: KeyAction,
        before: KeyState,
    },
    DisplayName {
        user_id: i64,
        before_display_name: String,
        display_name: String,
    },
}
pub(crate) fn valid_display_name(value: &str) -> bool {
    value == value.trim()
        && (1..=20).contains(&value.chars().count())
        && !value.chars().any(char::is_control)
}
async fn original_actor(c: &mut PgConnection, context: &VerifiedToolContext) -> Result<Principal> {
    context.revalidate(c).await.map_err(|e| match e {
        crate::assistant_tools_core::Error::Unauthorized => IdentityError::Unauthorized,
        crate::assistant_tools_core::Error::Conflict => IdentityError::Conflict,
        _ => IdentityError::Forbidden,
    })?;
    let row = sqlx::query(sqlx::AssertSqlSafe(format!(
        "{AUTH} AND c.id=$1 FOR SHARE OF c,u,pa,own"
    )))
    .bind(context.credential_id())
    .fetch_optional(&mut *c)
    .await?
    .ok_or(IdentityError::Unauthorized)?;
    let actor = principal(c, row).await?;
    if actor.credential_kind != CredentialKind::Session
        || actor.user_id != context.user_id()
        || actor.owner_account_id != context.account_id()
        || actor.owner.kind != AccountKind::Personal
        || actor.owner.id != actor.user_id
    {
        return Err(IdentityError::Forbidden);
    }
    Ok(actor)
}
async fn key_state(c: &mut PgConnection, actor: &Principal, id: i64) -> Result<KeyState> {
    let marked = key_management(c, actor).await?;
    let row = owned_key(c, actor.user_id, id, true).await?;
    if row.try_get::<bool, _>("revoked")? {
        return Err(IdentityError::Conflict);
    }
    if marked {
        personal_key_funding(c, actor, id).await?;
    }
    let enabled: bool = row.try_get("enabled")?;
    let (_, remaining, unlimited) = quota(c, id, actor.user_id).await?;
    let status = if !enabled {
        2
    } else if row.try_get::<bool, _>("expired")? {
        3
    } else if !unlimited && remaining <= 0 {
        4
    } else {
        1
    };
    let (_, required, revision) = mfa::assistant_policy(c, actor.user_id).await?;
    Ok(KeyState {
        id,
        name: row.try_get("name")?,
        group: row.try_get("group_name")?,
        enabled,
        revoked: false,
        status,
        expires_at_unix: row.try_get("expires")?,
        requires_two_factor: required,
        mfa_revision: revision,
    })
}
pub(crate) async fn prepare_key(
    c: &mut PgConnection,
    context: &VerifiedToolContext,
    id: i64,
    action: KeyAction,
) -> Result<Draft> {
    if !(1..=9_007_199_254_740_991).contains(&id) {
        return Err(IdentityError::Invalid);
    }
    let actor = original_actor(c, context).await?;
    Ok(Draft::ApiKey {
        action,
        before: key_state(c, &actor, id).await?,
    })
}
pub(crate) async fn prepare_display_name(
    c: &mut PgConnection,
    context: &VerifiedToolContext,
    display_name: &str,
) -> Result<Draft> {
    if !valid_display_name(display_name) {
        return Err(IdentityError::Invalid);
    }
    let actor = original_actor(c, context).await?;
    let before: String = sqlx::query_scalar(
        "SELECT display_name FROM core_identity.user_profiles WHERE user_id=$1 FOR UPDATE",
    )
    .bind(actor.user_id)
    .fetch_one(c)
    .await?;
    Ok(Draft::DisplayName {
        user_id: actor.user_id,
        before_display_name: before,
        display_name: display_name.into(),
    })
}
async fn digest(c: &mut PgConnection, value: &Value) -> Result<String> {
    Ok(
        sqlx::query_scalar("SELECT encode(sha256(convert_to(($1::jsonb)::text,'UTF8')),'hex')")
            .bind(value)
            .fetch_one(c)
            .await?,
    )
}
/// A failed TOTP consumes only the existing factor's retry allowance. The
/// coordinator explicitly commits that failure without approving the action.
pub(crate) async fn apply(
    c: &mut PgConnection,
    store: &IdentityStore,
    context: &VerifiedToolContext,
    draft: &Draft,
    confirmation_id: &str,
    operation_id: &str,
    code: Option<&str>,
) -> MfaResult<Value> {
    let actor = original_actor(c, context).await?;
    let (action, resource_kind, resource_id, before, after) = match draft {
        Draft::CreateKey { before } => {
            let after = create_key::create(c, store, &actor, before, confirmation_id, code).await?;
            let id = after
                .get("id")
                .and_then(Value::as_i64)
                .ok_or(IdentityError::Storage)?;
            (
                "create",
                "api_key",
                id,
                serde_json::to_value(before).map_err(|_| IdentityError::Storage)?,
                after,
            )
        }
        Draft::ApiKey { action, before } => {
            let current = key_state(c, &actor, before.id).await?;
            if &current != before {
                return Err(IdentityError::Conflict.into());
            }
            store
                .verify_assistant_action_mfa(
                    c,
                    actor.user_id,
                    before.requires_two_factor,
                    before.mfa_revision,
                    code,
                )
                .await?;
            let mut after = before.clone();
            match action {
                KeyAction::Disable => {
                    sqlx::query("UPDATE core_identity.api_key_settings SET enabled=FALSE WHERE credential_id=$1")
                        .bind(before.id).execute(&mut *c).await?;
                    after.enabled = false;
                    after.status = 2;
                }
                KeyAction::Delete => {
                    sqlx::query("UPDATE core_identity.credentials SET revoked_at=clock_timestamp() WHERE id=$1 AND revoked_at IS NULL")
                        .bind(before.id).execute(&mut *c).await?;
                    after.revoked = true;
                    after.status = 2;
                }
            }
            (
                action.name(),
                "api_key",
                before.id,
                serde_json::to_value(before).map_err(|_| IdentityError::Storage)?,
                serde_json::to_value(after).map_err(|_| IdentityError::Storage)?,
            )
        }
        Draft::DisplayName {
            user_id,
            before_display_name,
            display_name,
        } => {
            if *user_id != actor.user_id || !valid_display_name(display_name) || code.is_some() {
                return Err(IdentityError::Forbidden.into());
            }
            let current: String = sqlx::query_scalar(
                "SELECT display_name FROM core_identity.user_profiles WHERE user_id=$1 FOR UPDATE",
            )
            .bind(actor.user_id)
            .fetch_one(&mut *c)
            .await?;
            if &current != before_display_name {
                return Err(IdentityError::Conflict.into());
            }
            sqlx::query("UPDATE core_identity.user_profiles SET display_name=$2 WHERE user_id=$1")
                .bind(actor.user_id)
                .bind(display_name)
                .execute(&mut *c)
                .await?;
            (
                "change_display_name",
                "user_profile",
                actor.user_id,
                json!({"display_name": before_display_name}),
                json!({"display_name": display_name}),
            )
        }
    };
    let before_sha = digest(c, &before).await?;
    let after_sha = digest(c, &after).await?;
    let target = json!({
        "version": 1, "operation_id": operation_id, "confirmation_id": confirmation_id,
        "action": action, "resource_kind": resource_kind, "resource_id": resource_id,
        "account_id": actor.owner_account_id, "before_sha256": before_sha,
        "after_sha256": after_sha, "after": after
    });
    let audit_id: i64 = sqlx::query_scalar(
        "INSERT INTO core_identity.audit(actor_user_id,credential_id,action,target) VALUES($1,$2,'assistant.confirmed_action',$3) RETURNING id"
    ).bind(actor.user_id).bind(actor.credential_id).bind(&target).fetch_one(c).await?;
    let mut receipt = target;
    receipt["executed"] = json!(true);
    receipt["actor_user_id"] = json!(actor.user_id);
    receipt["audit_id"] = json!(audit_id);
    Ok(receipt)
}

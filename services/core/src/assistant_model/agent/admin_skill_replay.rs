//! Revalidate private administrator skill observations before active use.
//! Accepted operation/history bytes stay immutable; an old result is not a grant.
use super::{Error, context_error};
use super::{
    registry::{AgentRegistry, FREE_PRICE_VERSION, ToolDomain},
    store::ToolScope,
};
use crate::assistant_tools_core::admin_skills;
use serde_json::Value;
use sqlx::{PgConnection, Row};
use std::collections::BTreeMap;

pub(super) const TOOL_NAME: &str = "get_admin_user_skills";

// Cancellation may abandon only this reviewed, free, read-only definition.
// A new version or schema needs its own review; a matching name is insufficient.
const READ_ONLY_VERSION: &str = "main-3948a0a1";
const READ_ONLY_SCHEMA_SHA256: &str =
    "bdbd0a6b5a2982f3cc0e9a917b1f8fc834156fd3c56506c06cb8ae49315faccb";
const READ_ONLY_PRICE_SHA256: &str =
    "5d6fb460c679922a1976e87ef729440d5394187e9b12331f33faf12904a64192";

fn reviewed_read_only_scope(registry: &AgentRegistry, scope: &ToolScope, arguments: &[u8]) -> bool {
    if scope.name != TOOL_NAME
        || scope.domain != "extension"
        || scope.version != READ_ONLY_VERSION
        || scope.price_version != FREE_PRICE_VERSION
        || crate::hex_codec::encode(&scope.schema_sha256) != READ_ONLY_SCHEMA_SHA256
        || crate::hex_codec::encode(&scope.price_sha256) != READ_ONLY_PRICE_SHA256
        || !scope.resource_kind.is_empty()
        || !scope.resource_id.is_empty()
        || !scope.resource_version.is_empty()
        || !scope.resource_schema_sha256.is_empty()
        || !scope.resource_price_sha256.is_empty()
        || !scope.grant_id.is_empty()
    {
        return false;
    }
    let Ok(tool) = registry.get(TOOL_NAME) else {
        return false;
    };
    tool.domain == ToolDomain::Extension
        && tool.version == scope.version
        && tool.schema_sha256.as_slice() == scope.schema_sha256.as_slice()
        && tool.price_sha256.as_slice() == scope.price_sha256.as_slice()
        && tool.validate_arguments(arguments).is_ok()
}

pub(super) async fn can_abandon_read_only_tools(
    c: &mut PgConnection,
    turn_id: &str,
    registry: &AgentRegistry,
) -> Result<bool, Error> {
    // The caller already holds the original owner's conversation and turn
    // locks. No model uncertainty or financial reservation is waived here.
    let unsettled: bool = sqlx::query_scalar(
        "SELECT EXISTS(SELECT FROM core_assistant.agent_rounds r
         JOIN core_charging.requests q ON q.id=r.model_request_id
         WHERE r.turn_id=$1 AND
           (r.state NOT IN ('settled','released') OR q.state NOT IN ('settled','released')))",
    )
    .bind(turn_id)
    .fetch_one(&mut *c)
    .await?;
    if unsettled {
        return Ok(false);
    }
    // Do not read the unaccepted private result. Claims and observations remain
    // immutable evidence even when the user explicitly gives up this read.
    let rows = sqlx::query(
        "SELECT tool_name,tool_version,schema_sha256,price_version,price_sha256,
                resource_kind,resource_id,resource_version,resource_schema_sha256,
                resource_price_sha256,scope_grant_id,domain,arguments_json
         FROM core_assistant.agent_tool_operations
         WHERE turn_id=$1 AND state IN ('claimed','unknown') ORDER BY id FOR UPDATE",
    )
    .bind(turn_id)
    .fetch_all(&mut *c)
    .await?;
    if rows.is_empty() {
        return Ok(false);
    }
    for row in rows {
        if !reviewed_read_only_scope(
            registry,
            &super::store::scope(&row)?,
            &row.try_get::<Vec<u8>, _>("arguments_json")?,
        ) {
            return Ok(false);
        }
    }
    Ok(true)
}

struct BoundObservation {
    target_user_id: i64,
    binding: String,
}

fn observation(
    operation_id: &str,
    arguments: &[u8],
    result: &[u8],
    domain_receipt_id: &str,
) -> Result<BoundObservation, Error> {
    let args: Value = serde_json::from_slice(arguments).map_err(|_| Error::Invalid)?;
    let target = args
        .get("target_user_id")
        .and_then(Value::as_i64)
        .filter(|target| (1..=i64::from(i32::MAX)).contains(target))
        .ok_or(Error::Invalid)?;
    let value: Value = serde_json::from_slice(result).map_err(|_| Error::Invalid)?;
    let binding = value
        .get("authorization_binding_sha256")
        .and_then(Value::as_str)
        .filter(|binding| {
            binding.len() == 64
                && binding
                    .bytes()
                    .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
        })
        .ok_or(Error::Invalid)?;
    if value.get("ok") != Some(&Value::Bool(true))
        || value.get("target_user_id").and_then(Value::as_i64) != Some(target)
        || domain_receipt_id != format!("service:{operation_id}")
    {
        return Err(Error::Forbidden);
    }
    // checked_at_unix is an observation timestamp, not a lease or authority.
    // A still-valid immutable result is not rejected merely because it is old.
    Ok(BoundObservation {
        target_user_id: target,
        binding: binding.to_owned(),
    })
}

async fn revalidate(
    c: &mut PgConnection,
    original_session: &str,
    saved: BoundObservation,
) -> Result<(), Error> {
    let current = admin_skills::read_in_transaction(c, original_session, saved.target_user_id)
        .await
        .map_err(context_error)?;
    if current.target_user_id != saved.target_user_id
        || current.authorization_binding_sha256 != saved.binding
    {
        return Err(Error::Forbidden);
    }
    Ok(())
}

pub(super) async fn validate_completion(
    c: &mut PgConnection,
    original_session: &str,
    operation_id: &str,
    arguments: &[u8],
    result: &[u8],
    domain_receipt_id: &str,
) -> Result<(), Error> {
    revalidate(
        c,
        original_session,
        observation(operation_id, arguments, result, domain_receipt_id)?,
    )
    .await
}

pub(super) async fn validate_active_results(
    c: &mut PgConnection,
    original_session: &str,
    turn_id: &str,
) -> Result<(), Error> {
    // UNKNOWN is not an accepted private observation. Do not retrieve its body.
    // Only this active turn's settled rounds feed the next provider request.
    let rows = sqlx::query(
        "SELECT o.id,o.domain,o.arguments_json,o.result_json,o.domain_receipt_id
         FROM core_assistant.agent_tool_operations o
         JOIN core_assistant.agent_rounds r ON r.id=o.round_id
         WHERE o.turn_id=$1 AND o.tool_name='get_admin_user_skills'
           AND o.state='completed' AND r.state='settled'
         ORDER BY r.round,o.ordinal",
    )
    .bind(turn_id)
    .fetch_all(&mut *c)
    .await?;
    let mut observations = BTreeMap::new();
    for row in rows {
        if row.try_get::<String, _>("domain")? != "extension" {
            return Err(Error::Unavailable);
        }
        let saved = observation(
            &row.try_get::<String, _>("id")?,
            &row.try_get::<Vec<u8>, _>("arguments_json")?,
            &row.try_get::<Option<Vec<u8>>, _>("result_json")?
                .ok_or(Error::Unavailable)?,
            &row.try_get::<Option<String>, _>("domain_receipt_id")?
                .ok_or(Error::Unavailable)?,
        )?;
        if let Some(previous) = observations.insert(saved.target_user_id, saved.binding.clone())
            && previous != saved.binding
        {
            return Err(Error::Forbidden);
        }
    }
    for (target_user_id, binding) in observations {
        revalidate(
            c,
            original_session,
            BoundObservation {
                target_user_id,
                binding,
            },
        )
        .await?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    use sha2::Digest;

    #[test]
    fn cancellation_requires_the_exact_reviewed_read_only_skill_scope() {
        let bytes = include_bytes!("../../../../../contracts/assistant-tools-v2.json");
        let registry = AgentRegistry::from_reviewed(
            bytes,
            &crate::hex_codec::encode(sha2::Sha256::digest(bytes)),
        )
        .unwrap();
        let tool = registry.get(TOOL_NAME).unwrap();
        let scope = ToolScope {
            name: tool.name.clone(),
            version: tool.version.clone(),
            schema_sha256: tool.schema_sha256.to_vec(),
            price_version: FREE_PRICE_VERSION.into(),
            price_sha256: tool.price_sha256.to_vec(),
            domain: "extension".into(),
            resource_kind: String::new(),
            resource_id: String::new(),
            resource_version: String::new(),
            grant_id: String::new(),
            resource_schema_sha256: Vec::new(),
            resource_price_sha256: Vec::new(),
        };
        let arguments = br#"{"target_user_id":7,"before_id":0,"limit":16}"#;
        assert!(reviewed_read_only_scope(&registry, &scope, arguments));
        for field in 0..12 {
            let mut changed = scope.clone();
            match field {
                0 => changed.name = "send_invitation".into(),
                1 => changed.domain = "registered_core".into(),
                2 => changed.version = "unreviewed-next-version".into(),
                3 => changed.schema_sha256[0] ^= 1,
                4 => changed.price_version = "unreviewed-price".into(),
                5 => changed.price_sha256[0] ^= 1,
                6 => changed.resource_kind = "toolmarket".into(),
                7 => changed.resource_id = "remote-service".into(),
                8 => changed.resource_version = "1".into(),
                9 => changed.grant_id = "other-grant".into(),
                10 => changed.resource_schema_sha256 = vec![1; 32],
                11 => changed.resource_price_sha256 = vec![2; 32],
                _ => unreachable!(),
            }
            assert!(
                !reviewed_read_only_scope(&registry, &changed, arguments),
                "unexpected cancellable scope field {field}"
            );
        }
        for arguments in [
            br#"{}"#.as_slice(),
            br#"{"target_user_id":0}"#.as_slice(),
            br#"{"target_user_id":7,"limit":17}"#.as_slice(),
            br#"{"target_user_id":7,"apply":true}"#.as_slice(),
        ] {
            assert!(!reviewed_read_only_scope(&registry, &scope, arguments));
        }
    }

    fn result(target: i64, binding: &str) -> Value {
        json!({
            "ok": true,
            "target_user_id": target,
            "authorization_binding_sha256": binding,
            "checked_at_unix": 1,
            "profile": null,
            "memories": []
        })
    }

    #[test]
    fn private_skill_result_requires_exact_target_binding_and_original_receipt() {
        let binding = "ab".repeat(32);
        let args = br#"{"target_user_id":7,"before_id":0,"limit":16}"#;
        let valid = serde_json::to_vec(&result(7, &binding)).unwrap();
        let saved = observation("op", args, &valid, "service:op").unwrap();
        assert_eq!(saved.target_user_id, 7);
        assert_eq!(saved.binding, binding);
        assert!(observation("op", args, &valid, "service:other").is_err());
        assert!(observation("op", args, &valid, "unrelated").is_err());
        for target in [0, 8] {
            let bytes = serde_json::to_vec(&result(target, &binding)).unwrap();
            assert!(observation("op", args, &bytes, "service:op").is_err());
        }
        for invalid in ["", "ab", &"AB".repeat(32), &"zz".repeat(32)] {
            let bytes = serde_json::to_vec(&result(7, invalid)).unwrap();
            assert!(observation("op", args, &bytes, "service:op").is_err());
        }
        for bad_args in [
            br#"{"user_id":7}"#.as_slice(),
            br#"{"target_user_id":0}"#.as_slice(),
            br#"{"target_user_id":2147483648}"#.as_slice(),
            br#"{"target_user_id":"7"}"#.as_slice(),
        ] {
            assert!(observation("op", bad_args, &valid, "service:op").is_err());
        }
        let mut bad = result(7, &binding);
        bad["ok"] = Value::Bool(false);
        assert!(observation("op", args, &serde_json::to_vec(&bad).unwrap(), "service:op").is_err());
    }
}

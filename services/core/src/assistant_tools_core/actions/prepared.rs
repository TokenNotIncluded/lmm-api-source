//! Read authority for an already completed *preparation*. Normal model final,
//! settlement and grant expiry do not revoke a five-minute browser action.
//! Explicit grant revocation and an abnormal turn still prohibit first execution.
use super::*;
use sqlx::postgres::PgRow;

pub(crate) async fn verify(
    c: &mut PgConnection,
    context: &VerifiedToolContext,
    confirmation: &PgRow,
) -> Result<(), Error> {
    let facts = context.revalidate(c).await?;
    let replay = confirmation.try_get::<String, _>("state")? == "approved"
        && confirmation
            .try_get::<Option<Value>, _>("execution_receipt")?
            .is_some();
    let operation: String = confirmation.try_get("operation_id")?;
    let r = sqlx::query(
        "SELECT o.*,t.source_web_session_sid,t.host_policy,r.provider_response_json,r.provider_response_sha256
         FROM core_assistant.agent_tool_operations o
         JOIN core_assistant.agent_turns t ON t.id=o.turn_id
         JOIN core_assistant.agent_rounds r ON r.id=o.round_id AND r.turn_id=t.id
         JOIN core_charging.requests paid ON paid.id=r.model_request_id
         JOIN core_assistant.grants g ON g.id=paid.assistant_grant_id
         WHERE o.id=$1 AND t.owner_user_id=$2 AND t.owner_account_id=$3
           AND t.session_id=$4 AND t.auth_version=$5 AND t.source_web_session_sid=$6
           AND o.domain='registered_core' AND o.tool_name IN ('prepare_api_key_action','prepare_user_action','request_create_key','connect_market_tool')
           AND o.state='completed' AND o.claim_key='core-'||o.id
           AND o.domain_receipt_id='core-prepared-'||o.id
           AND r.state='settled' AND paid.state='settled' AND paid.outcome='completed'
           AND paid.actor_user_id=t.owner_user_id AND paid.owner_account_id=t.owner_account_id
           AND g.session_id=t.session_id AND g.auth_version=t.auth_version
           AND g.conversation_id=t.conversation_id AND g.policy=t.host_policy->'model'
           AND EXISTS(SELECT FROM core_assistant.operations original
             WHERE original.request_id=paid.id AND original.grant_id=g.id)
           AND ($7 OR (NOT g.revoked AND t.phase IN ('tools_pending','model_running','completed')
             AND NOT EXISTS(SELECT FROM core_assistant.agent_rounds cancelled
               JOIN core_charging.requests cq ON cq.id=cancelled.model_request_id
               JOIN core_assistant.grants cg ON cg.id=cq.assistant_grant_id
               WHERE cancelled.turn_id=t.id AND cg.revoked)))
         FOR SHARE OF o,t,r,paid,g"
    ).bind(&operation).bind(facts.user).bind(facts.account).bind(context.credential_id())
        .bind(facts.auth_version).bind(facts.browser_session.as_deref()).bind(replay)
        .fetch_optional(&mut *c).await?.ok_or(Error::Forbidden)?;
    let name: String = r.try_get("tool_name")?;
    let args: Vec<u8> = r.try_get("arguments_json")?;
    let args_sha: Vec<u8> = r.try_get("arguments_sha256")?;
    let result: Vec<u8> = r.try_get("result_json")?;
    let result_sha: Vec<u8> = r.try_get("result_sha256")?;
    let provider: Vec<u8> = r.try_get("provider_response_json")?;
    let provider_sha: Vec<u8> = r.try_get("provider_response_sha256")?;
    let preview: Value = confirmation.try_get("preview")?;
    let preview_bytes = serde_json::to_vec(&preview).map_err(|_| Error::Unavailable)?;
    if name != confirmation.try_get::<String, _>("tool_name")?
        || descriptor(&name).is_none_or(|(_, version)| {
            r.try_get::<String, _>("tool_version").ok().as_deref() != Some(version)
        })
        || r.try_get::<String, _>("tool_version")?
            != confirmation.try_get::<String, _>("tool_version")?
        || r.try_get::<String, _>("turn_id")? != confirmation.try_get::<String, _>("turn_id")?
        || args_sha != confirmation.try_get::<Vec<u8>, _>("arguments_sha256")?
        || Sha256::digest(&args).as_slice() != args_sha
        || Sha256::digest(&result).as_slice() != result_sha
        || Sha256::digest(&provider).as_slice() != provider_sha
        || Sha256::digest(&preview_bytes).as_slice()
            != confirmation.try_get::<Vec<u8>, _>("draft_sha256")?
        || !r
            .try_get::<Value, _>("host_policy")?
            .get("allowed_tools")
            .and_then(Value::as_array)
            .is_some_and(|names| names.iter().any(|v| v.as_str() == Some(name.as_str())))
    {
        return Err(Error::Forbidden);
    }
    let (schema, _) = descriptor(&name).ok_or(Error::Forbidden)?;
    if r.try_get::<Vec<u8>, _>("schema_sha256")? != Sha256::digest(schema.as_bytes()).as_slice() {
        return Err(Error::Forbidden);
    }
    let stored: Value = serde_json::from_slice(&result).map_err(|_| Error::Unavailable)?;
    let challenge = confirmation::projection(confirmation)?;
    if !matches_prepared_result(&stored, &challenge) {
        return Err(Error::Forbidden);
    }
    let response: Value = serde_json::from_slice(&provider).map_err(|_| Error::Unavailable)?;
    let call_id: String = r.try_get("provider_call_id")?;
    let matches = response
        .pointer("/choices/0/message/tool_calls")
        .and_then(Value::as_array)
        .is_some_and(|calls| {
            calls.iter().any(|call| {
                call.get("id").and_then(Value::as_str) == Some(call_id.as_str())
                    && call.pointer("/function/name").and_then(Value::as_str) == Some(name.as_str())
                    && call
                        .pointer("/function/arguments")
                        .and_then(Value::as_str)
                        .is_some_and(|source| source.as_bytes() == args)
            })
        });
    if !matches {
        return Err(Error::Forbidden);
    }
    if name == crate::tool_market::assistant_connect::TOOL {
        let draft = confirmation::market::draft(&preview)?;
        let original: crate::tool_market::assistant_connect::ConnectArguments =
            serde_json::from_slice(&args).map_err(|_| Error::Unavailable)?;
        if original.service_id != draft.preview.service_id
            || original.tool_id != draft.preview.request.tool_id
            || original.version_id != draft.preview.request.version_id
        {
            return Err(Error::Forbidden);
        }
        return Ok(());
    }
    let draft: Draft = serde_json::from_value(preview).map_err(|_| Error::Unavailable)?;
    match (name.as_str(), draft) {
        (CREATE_TOOL, Draft::CreateKey { before }) => {
            let original: CreateArgs =
                serde_json::from_slice(&args).map_err(|_| Error::Unavailable)?;
            if original.name() != before.name || original.group != before.group {
                return Err(Error::Forbidden);
            }
        }
        (KEY_TOOL, Draft::ApiKey { action, before }) => {
            let original: KeyArgs =
                serde_json::from_slice(&args).map_err(|_| Error::Unavailable)?;
            if original.action != action || original.token_id != before.id {
                return Err(Error::Forbidden);
            }
        }
        (
            PROFILE_TOOL,
            Draft::DisplayName {
                user_id,
                display_name,
                ..
            },
        ) => {
            let original: ProfileArgs =
                serde_json::from_slice(&args).map_err(|_| Error::Unavailable)?;
            if original.action != "change_display_name"
                || original.display_name != display_name
                || user_id != facts.user
            {
                return Err(Error::Forbidden);
            }
        }
        _ => return Err(Error::Forbidden),
    }
    Ok(())
}

fn matches_prepared_result(stored: &Value, challenge: &confirmation::Challenge) -> bool {
    let mut expected = prepared_result(challenge);
    if stored == &expected {
        return true;
    }
    // Historical operations are immutable. Accept their exact original path
    // without changing any receipt field or authorizing another challenge.
    expected["browser_path"] = Value::String(format!(
        "/api/user/assistant/confirmations/{}",
        challenge.challenge_id
    ));
    stored == &expected
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn legacy_prepared_result_only_accepts_exact_historical_navigation() {
        let id = "a".repeat(64);
        let challenge = confirmation::Challenge {
            challenge_id: id.clone(),
            operation_id: "original-operation".into(),
            tool_name: CREATE_TOOL.into(),
            preview: json!({}),
            draft_sha256: "b".repeat(64),
            arguments_sha256: "c".repeat(64),
            tool_version: "v1".into(),
            resource_kind: "api_key".into(),
            resource_id: String::new(),
            resource_version: String::new(),
            expires_at_unix: 1_795_400_000,
            state: "pending".into(),
            browser_path: format!("/api/user/auth/assistant/confirmations/{id}"),
            executed: false,
            execution_receipt: None,
            requires_two_factor: false,
        };
        let current = prepared_result(&challenge);
        assert!(matches_prepared_result(&current, &challenge));
        let mut legacy = current.clone();
        legacy["browser_path"] = json!(format!("/api/user/assistant/confirmations/{id}"));
        let original_legacy = legacy.clone();
        assert!(matches_prepared_result(&legacy, &challenge));
        assert_eq!(legacy, original_legacy);

        for path in [
            format!("/api/user/assistant/confirmations/{}", "d".repeat(64)),
            format!("/api/user/auth/assistant/confirmations/{}", "d".repeat(64)),
            format!("https://example.invalid/api/user/assistant/confirmations/{id}"),
            format!("/api/user/assistant/confirmations/{id}?confirmed=true"),
            format!("/api/user/assistant/confirmations/{id}#confirm"),
            format!("/api/user/assistant/confirmations/../confirmations/{id}"),
            format!("/api/user/assistant/confirmations/%61{}", &id[1..]),
            format!("/api/user/assistant/confirmations/{id}/"),
        ] {
            let mut altered = legacy.clone();
            altered["browser_path"] = json!(path);
            assert!(!matches_prepared_result(&altered, &challenge), "{path}");
        }
        for (key, value) in [
            ("confirmation_id", json!("d".repeat(64))),
            ("draft_sha256", json!("e".repeat(64))),
            ("expires_at_unix", json!(1_795_400_001)),
            ("status", json!("approved")),
            ("executed", json!(true)),
            ("ok", json!(false)),
            ("extra", json!(true)),
        ] {
            for original in [&current, &legacy] {
                let mut altered = original.clone();
                altered[key] = value.clone();
                assert!(!matches_prepared_result(&altered, &challenge), "{key}");
            }
        }
    }
}

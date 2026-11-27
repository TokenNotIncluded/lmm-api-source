//! Runs inside the administrator publication/recovery PostgreSQL fixture.
use super::*;
use agent::registry::AgentRegistry;
use agent::store::{AgentPolicy, AgentService};
use std::collections::BTreeSet;

pub(crate) async fn check_selected_model_issuance(
    pool: &PgPool,
    gateway: ModelGateway,
    session: &str,
    host: &PersonalTextPolicy,
) {
    let mut expected = host.clone();
    expected.model = "assistant-alternate".into();
    let quote = gateway
        .assistant_quote(&expected.model, &expected.group)
        .unwrap();
    assert_eq!(quote.model, "model-a"); // The alias has the canonical model's real tariff.
    let expected_price = serde_json::to_value(quote).unwrap();
    let base = AssistantModel::new(pool.clone(), gateway, host.clone()).unwrap();
    let personal = base
        .conversation(session, "selection-personal", true)
        .await
        .unwrap();
    let grant = base
        .turn(session, &personal.core_conversation_id, "first", true)
        .await
        .unwrap();
    let frozen: (Value, Value) =
        sqlx::query_as("SELECT policy,price FROM core_assistant.grants WHERE id=$1")
            .bind(&grant.delegation_id)
            .fetch_one(pool)
            .await
            .unwrap();
    assert_eq!(frozen.0, serde_json::to_value(&expected).unwrap());
    assert_eq!(frozen.1, expected_price);

    let tools = BTreeSet::from(["get_account_access".to_owned()]);
    let policy = AgentPolicy {
        model: host.clone(),
        max_rounds: 2,
        budget_credits: 2 * host.budget_credits,
        max_tool_calls: 2,
        ttl_seconds: host.ttl_seconds,
        allowed_tools: tools.clone(),
        initial_tools: tools,
        system_prompt: "Use reviewed account facts.".into(),
        market: None,
    };
    let artifact = include_bytes!("../../../../contracts/assistant-tools-v1.json");
    let registry = AgentRegistry::from_reviewed(
        artifact,
        &crate::hex_codec::encode(Sha256::digest(artifact)),
    )
    .unwrap();
    let service = AgentService::new(base.clone(), policy.clone(), registry).unwrap();
    // Hold the local execution permits so this issuance test cannot send a request.
    let _permits = base
        .permits
        .acquire_many(base.capacity as u32)
        .await
        .unwrap();
    let conversation = base
        .conversation(session, "selection-agent", true)
        .await
        .unwrap();
    let turn = service
        .start(
            session,
            &conversation.core_conversation_id,
            "first",
            "hello",
        )
        .await
        .unwrap();
    assert_eq!(turn.phase, "new");
    let frozen: (Value, Value) = sqlx::query_as("SELECT host_policy,price FROM core_assistant.agent_turns WHERE conversation_id=$1 AND client_turn_id=$2")
        .bind(&conversation.core_conversation_id).bind(&turn.client_turn_id).fetch_one(pool).await.unwrap();
    let mut expected_agent = policy;
    expected_agent.model = expected;
    assert_eq!(frozen.0, serde_json::to_value(&expected_agent).unwrap());
    assert_eq!(frozen.1, expected_price);
    assert_eq!(turn.held_credits, 0);
}

pub(crate) async fn check_replaced_model_grant(
    pool: &PgPool,
    gateway: ModelGateway,
    session: &str,
    host: &PersonalTextPolicy,
) {
    let base = AssistantModel::new(pool.clone(), gateway, host.clone()).unwrap();
    let conversation = base
        .conversation(session, "selection-personal", false)
        .await
        .unwrap();
    let grant = base
        .turn(session, &conversation.core_conversation_id, "first", false)
        .await
        .unwrap();
    let binding = Binding {
        delegation_id: grant.delegation_id.clone(),
        core_conversation_id: conversation.core_conversation_id.clone(),
        turn_id: "first".into(),
        round: 0,
        operation_key: "first:0".into(),
        text_sha256: Sha256::digest(b"hello").to_vec(),
    };
    assert!(matches!(
        base.reserve_text(session, &binding, "hello").await,
        Err(Error::Forbidden)
    ));
    let retired = base
        .turn(session, &conversation.core_conversation_id, "first", false)
        .await
        .unwrap();
    assert!(retired.revoked);
    assert_eq!(retired.delegation_id, grant.delegation_id);

    let next = base
        .turn(session, &conversation.core_conversation_id, "second", true)
        .await
        .unwrap();
    let frozen: Value = sqlx::query_scalar("SELECT policy FROM core_assistant.grants WHERE id=$1")
        .bind(&next.delegation_id)
        .fetch_one(pool)
        .await
        .unwrap();
    assert_eq!(frozen, serde_json::to_value(host).unwrap());
    let dispatched: i64 = sqlx::query_scalar("SELECT count(*) FROM core_charging.requests WHERE assistant_grant_id=$1 OR assistant_grant_id=$2")
        .bind(&grant.delegation_id).bind(&next.delegation_id).fetch_one(pool).await.unwrap();
    assert_eq!(dispatched, 0);
}

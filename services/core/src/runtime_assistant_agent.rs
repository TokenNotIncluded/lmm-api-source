//! Exact authority required by the fresh-installed agent journal and its guards.
use super::RuntimeDatabaseError;
use sqlx::PgConnection;

pub(super) async fn check(
    connection: &mut PgConnection,
    required: bool,
) -> Result<(), RuntimeDatabaseError> {
    if !required {
        return Ok(());
    }
    let visible: bool = sqlx::query_scalar("SELECT has_schema_privilege('core_assistant','USAGE')")
        .fetch_one(&mut *connection)
        .await?;
    if !visible {
        return Err(RuntimeDatabaseError::MissingPrivileges);
    }
    for (table, mutable) in [
        (
            "agent_turns",
            &["phase", "revision", "final_text", "loaded_tools"][..],
        ),
        (
            "agent_rounds",
            &[
                "state",
                "provider_response_json",
                "provider_response_sha256",
            ][..],
        ),
        (
            "agent_tool_operations",
            &[
                "state",
                "claim_key",
                "result_json",
                "result_sha256",
                "domain_receipt_id",
            ][..],
        ),
        ("agent_tool_observations", &[][..]),
    ] {
        let relation = format!("core_assistant.{table}");
        let complete: bool = sqlx::query_scalar(
            "SELECT has_table_privilege($1,'SELECT') AND has_table_privilege($1,'INSERT')
             AND NOT EXISTS(SELECT FROM pg_attribute WHERE attrelid=$1::regclass
               AND attnum>0 AND NOT attisdropped
               AND has_column_privilege($1,attname,'UPDATE') IS DISTINCT FROM (attname=ANY($2::text[])))",
        )
        .bind(relation)
        .bind(mutable)
        .fetch_one(&mut *connection)
        .await?;
        if !complete {
            return Err(RuntimeDatabaseError::MissingPrivileges);
        }
    }
    let dependencies: bool = sqlx::query_scalar(
        "SELECT has_sequence_privilege('core_assistant.operations_position_seq','USAGE')
         AND has_sequence_privilege('core_assistant.operations_position_seq','SELECT')
         AND has_sequence_privilege('core_assistant.agent_tool_observations_position_seq','USAGE')
         AND has_sequence_privilege('core_assistant.agent_tool_observations_position_seq','SELECT')
         AND (SELECT count(*)=10 FROM (VALUES
           ('agent_turns','agent_turn_immutable'),
           ('agent_rounds','agent_round_immutable'),
           ('agent_tool_operations','agent_tool_immutable'),
           ('agent_tool_observations','agent_observation_immutable'),
           ('agent_turns','agent_turn_binding'),
           ('agent_rounds','agent_round_binding'),
           ('agent_tool_operations','agent_tool_binding'),
           ('agent_tool_observations','agent_observation_binding'),
           ('requests','agent_charge_binding'),
           ('requests','agent_legacy_exclusion')) expected(relation,trigger_name)
         JOIN pg_class c ON c.relname=expected.relation
         JOIN pg_namespace n ON n.oid=c.relnamespace
           AND n.nspname=CASE WHEN expected.relation='requests' THEN 'core_charging' ELSE 'core_assistant' END
         JOIN pg_trigger t ON t.tgrelid=c.oid AND t.tgname=expected.trigger_name
           AND NOT t.tgisinternal AND t.tgenabled='O')",
    )
    .fetch_one(connection)
    .await?;
    if !dependencies {
        return Err(RuntimeDatabaseError::MissingPrivileges);
    }
    Ok(())
}

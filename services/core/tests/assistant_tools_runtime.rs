//! Exact restricted runtime authority, using a new SQLx-owned database only.
use lmm_core::{
    core_schema,
    runtime_database::{self, RequiredCapabilities, RuntimeDatabaseError as Error},
};
use sqlx::PgPool;
#[path = "common/runtime.rs"]
mod runtime;

#[sqlx::test(migrations = false)]
async fn assistant_confirmation_permissions_are_explicit_and_column_limited(pool: PgPool) {
    core_schema::init(&pool).await.unwrap();
    let role = runtime::RuntimeRole::create(&pool).await;
    let required = RequiredCapabilities {
        assistant_tools: true,
        ..RequiredCapabilities::default()
    };
    runtime_database::check(&role.pool, RequiredCapabilities::default())
        .await
        .unwrap();
    assert_eq!(
        runtime_database::check(&role.pool, required).await,
        Err(Error::MissingPrivileges)
    );
    role.optional("core_assistant_tools").await;
    // Startup also reads the durable agent journal; check-db must not pass
    // until that exact read authority exists.
    assert_eq!(
        runtime_database::check(&role.pool, required).await,
        Err(Error::MissingPrivileges)
    );
    let agent_reads = format!(
        "GRANT USAGE ON SCHEMA core_assistant TO {0}; GRANT SELECT ON core_assistant.agent_turns,core_assistant.agent_rounds,core_assistant.agent_tool_operations TO {0}",
        role.name
    );
    sqlx::raw_sql(sqlx::AssertSqlSafe::<&str>(&agent_reads))
        .execute(&pool)
        .await
        .unwrap();
    runtime_database::check(&role.pool, required).await.unwrap();
    sqlx::query("SELECT o.id,o.arguments_json,o.resource_schema_sha256,o.resource_price_sha256,t.session_id,t.host_policy,t.source_web_session_sid,r.model_request_id,c.id FROM core_assistant.agent_tool_operations o JOIN core_assistant.agent_turns t ON t.id=o.turn_id JOIN core_assistant.agent_rounds r ON r.id=o.round_id LEFT JOIN core_assistant_tools.confirmations c ON c.operation_id=o.id WHERE FALSE")
        .execute(&role.pool)
        .await
        .unwrap();
    for table in ["agent_turns", "agent_rounds", "agent_tool_operations"] {
        sqlx::query(sqlx::AssertSqlSafe(format!(
            "REVOKE SELECT ON core_assistant.{table} FROM {}",
            role.name
        )))
        .execute(&pool)
        .await
        .unwrap();
        assert_eq!(
            runtime_database::check(&role.pool, required).await,
            Err(Error::MissingPrivileges),
            "{table}"
        );
        sqlx::raw_sql(sqlx::AssertSqlSafe::<&str>(&agent_reads))
            .execute(&pool)
            .await
            .unwrap();
    }
    runtime_database::check(&role.pool, required).await.unwrap();
    sqlx::query("SELECT * FROM core_assistant_tools.confirmations")
        .fetch_all(&role.pool)
        .await
        .unwrap();
    for column in ["nonce_sha256", "state", "confirmed_at"] {
        sqlx::query(sqlx::AssertSqlSafe(format!(
            "UPDATE core_assistant_tools.confirmations SET {column}={column} WHERE FALSE"
        )))
        .execute(&role.pool)
        .await
        .unwrap();
        sqlx::query(sqlx::AssertSqlSafe(format!(
            "REVOKE UPDATE({column}) ON core_assistant_tools.confirmations FROM {}",
            role.name
        )))
        .execute(&pool)
        .await
        .unwrap();
        assert_eq!(
            runtime_database::check(&role.pool, required).await,
            Err(Error::MissingPrivileges)
        );
        sqlx::query(sqlx::AssertSqlSafe(format!(
            "GRANT UPDATE({column}) ON core_assistant_tools.confirmations TO {}",
            role.name
        )))
        .execute(&pool)
        .await
        .unwrap();
    }
    sqlx::query(sqlx::AssertSqlSafe(format!(
        "REVOKE UPDATE(revision) ON core_humangrant.human_grant_gate FROM {}",
        role.name
    )))
    .execute(&pool)
    .await
    .unwrap();
    assert_eq!(
        runtime_database::check(&role.pool, required).await,
        Err(Error::MissingPrivileges)
    );
    role.optional("core_assistant_tools").await;
    for table in [
        "intents",
        "accepted_proofs",
        "refund_holds",
        "blocked_refunds",
    ] {
        sqlx::query(sqlx::AssertSqlSafe(format!(
            "REVOKE SELECT ON core_payments.{table} FROM {}",
            role.name
        )))
        .execute(&pool)
        .await
        .unwrap();
        assert_eq!(
            runtime_database::check(&role.pool, required).await,
            Err(Error::MissingPrivileges)
        );
        sqlx::query(sqlx::AssertSqlSafe(format!(
            "GRANT SELECT ON core_payments.{table} TO {}",
            role.name
        )))
        .execute(&pool)
        .await
        .unwrap();
    }
    let columns:Vec<String>=sqlx::query_scalar("SELECT column_name::text FROM information_schema.columns WHERE table_schema='core_assistant_tools' AND table_name='confirmations' AND column_name NOT IN ('nonce_sha256','state','confirmed_at') ORDER BY ordinal_position").fetch_all(&pool).await.unwrap();
    for column in columns {
        let e = sqlx::query(sqlx::AssertSqlSafe(format!(
            "UPDATE core_assistant_tools.confirmations SET {column}={column} WHERE FALSE"
        )))
        .execute(&role.pool)
        .await
        .unwrap_err();
        assert_eq!(
            e.as_database_error().and_then(|e| e.code()).as_deref(),
            Some("42501")
        );
        sqlx::query(sqlx::AssertSqlSafe(format!(
            "GRANT UPDATE({column}) ON core_assistant_tools.confirmations TO {}",
            role.name
        )))
        .execute(&pool)
        .await
        .unwrap();
        assert_eq!(
            runtime_database::check(&role.pool, required).await,
            Err(Error::UnsafePrivileges),
            "{column}"
        );
        sqlx::query(sqlx::AssertSqlSafe(format!(
            "REVOKE UPDATE({column}) ON core_assistant_tools.confirmations FROM {}",
            role.name
        )))
        .execute(&pool)
        .await
        .unwrap();
    }
    for privilege in [
        "UPDATE",
        "DELETE",
        "TRUNCATE",
        "TRIGGER",
        "REFERENCES",
        "MAINTAIN",
    ] {
        sqlx::query(sqlx::AssertSqlSafe(format!(
            "GRANT {privilege} ON core_assistant_tools.confirmations TO {}",
            role.name
        )))
        .execute(&pool)
        .await
        .unwrap();
        assert_eq!(
            runtime_database::check(&role.pool, required).await,
            Err(Error::UnsafePrivileges),
            "{privilege}"
        );
        sqlx::query(sqlx::AssertSqlSafe(format!(
            "REVOKE {privilege} ON core_assistant_tools.confirmations FROM {}",
            role.name
        )))
        .execute(&pool)
        .await
        .unwrap();
        // Revoking whole UPDATE also removes its column grants in PostgreSQL.
        role.optional("core_assistant_tools").await;
        runtime_database::check(&role.pool, required).await.unwrap();
    }
    sqlx::query(sqlx::AssertSqlSafe(format!(
        "GRANT REFERENCES(state) ON core_assistant_tools.confirmations TO {}",
        role.name
    )))
    .execute(&pool)
    .await
    .unwrap();
    assert_eq!(
        runtime_database::check(&role.pool, required).await,
        Err(Error::UnsafePrivileges)
    );
    sqlx::query(sqlx::AssertSqlSafe(format!(
        "REVOKE REFERENCES(state) ON core_assistant_tools.confirmations FROM {}",
        role.name
    )))
    .execute(&pool)
    .await
    .unwrap();
    // Effective grants inherited from PUBLIC are unsafe too.
    sqlx::query("GRANT UPDATE(preview) ON core_assistant_tools.confirmations TO PUBLIC")
        .execute(&pool)
        .await
        .unwrap();
    assert_eq!(
        runtime_database::check(&role.pool, required).await,
        Err(Error::UnsafePrivileges)
    );
    sqlx::query("REVOKE UPDATE(preview) ON core_assistant_tools.confirmations FROM PUBLIC")
        .execute(&pool)
        .await
        .unwrap();
    runtime_database::check(&role.pool, required).await.unwrap();
    role.cleanup().await;
}

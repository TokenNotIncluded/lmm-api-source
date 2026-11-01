//! New account-security capability grants are narrow and checked before serving.
use lmm_core::{
    core_schema,
    runtime_database::{self, RequiredCapabilities, RuntimeDatabaseError},
};
use sqlx::PgPool;
#[path = "common/runtime.rs"]
mod runtime;

#[sqlx::test(migrations = false)]
async fn account_security_requires_its_minimum_and_rejects_mutable_identity_or_balance_authority(
    pool: PgPool,
) {
    core_schema::init(&pool).await.unwrap();
    let role = runtime::RuntimeRole::create(&pool).await;
    let web = RequiredCapabilities {
        web_identity: true,
        ..Default::default()
    };
    // The new global OAuth revocation read works with Web disabled.
    runtime_database::check(&role.pool, Default::default())
        .await
        .unwrap();
    role.optional("web_identity").await;
    runtime_database::check(&role.pool, web).await.unwrap();
    sqlx::raw_sql(&format!(
        "REVOKE INSERT(binding_digest) ON core_account_links.bind_flows FROM {}",
        role.name
    ))
    .execute(&pool)
    .await
    .unwrap();
    assert_eq!(
        runtime_database::check(&role.pool, web).await,
        Err(RuntimeDatabaseError::MissingPrivileges)
    );
    sqlx::raw_sql(&format!(
        "GRANT INSERT(binding_digest) ON core_account_links.bind_flows TO {}",
        role.name
    ))
    .execute(&pool)
    .await
    .unwrap();
    for privilege in [
        "UPDATE(status) ON core_account_appeals.requests",
        "UPDATE(request_id) ON core_account_appeals.flows",
        "USAGE ON SEQUENCE core_account_appeals.requests_id_seq",
    ] {
        sqlx::raw_sql(&format!("REVOKE {privilege} FROM {}", role.name))
            .execute(&pool)
            .await
            .unwrap();
        assert_eq!(
            runtime_database::check(&role.pool, web).await,
            Err(RuntimeDatabaseError::MissingPrivileges),
            "{privilege}"
        );
        sqlx::raw_sql(&format!("GRANT {privilege} TO {}", role.name))
            .execute(&pool)
            .await
            .unwrap();
    }
    for privilege in [
        "UPDATE(user_id) ON core_passkeys.credentials",
        "UPDATE(reason) ON core_account_appeals.requests",
        "UPDATE(user_id) ON core_account_appeals.requests",
        "DELETE ON core_account_appeals.requests",
        "UPDATE(proof_kind) ON core_account_appeals.flows",
        "SELECT ON SEQUENCE core_account_appeals.requests_id_seq",
        "SELECT ON core_meta.account_appeal_upgrades",
        "DELETE ON core_account_links.oauth_states",
        "UPDATE(operation_digest) ON core_account_lifecycle.closures",
        "UPDATE(balance_units) ON core_billing.balance_state",
    ] {
        sqlx::raw_sql(&format!("GRANT {privilege} TO {}", role.name))
            .execute(&pool)
            .await
            .unwrap();
        assert_eq!(
            runtime_database::check(&role.pool, web).await,
            Err(RuntimeDatabaseError::UnsafePrivileges),
            "{privilege}"
        );
        assert_eq!(
            runtime_database::check(&role.pool, Default::default()).await,
            Err(RuntimeDatabaseError::UnsafePrivileges),
            "{privilege}"
        );
        sqlx::raw_sql(&format!("REVOKE {privilege} FROM {}", role.name))
            .execute(&pool)
            .await
            .unwrap();
    }
    sqlx::raw_sql(&format!(
        "REVOKE EXECUTE ON FUNCTION core_account_lifecycle.wallet_is_empty(bigint) FROM {}",
        role.name
    ))
    .execute(&pool)
    .await
    .unwrap();
    assert_eq!(
        runtime_database::check(&role.pool, web).await,
        Err(RuntimeDatabaseError::MissingPrivileges)
    );
    sqlx::raw_sql(&format!(
        "GRANT EXECUTE ON FUNCTION core_account_lifecycle.wallet_is_empty(bigint) TO {}",
        role.name
    ))
    .execute(&pool)
    .await
    .unwrap();
    runtime_database::check(&role.pool, web).await.unwrap();
    role.cleanup().await;
}

//! Actual shipped optional ACL reset, with a separate NOLOGIN schema owner.
#![cfg(unix)]

use lmm_core::core_schema;
use serde_json::Value;
use sqlx::{PgPool, postgres::PgPoolOptions};
use std::{path::PathBuf, process::Command};

async fn authority(pool: &PgPool, role: &str) -> Value {
    sqlx::query_scalar(
        r#"SELECT jsonb_build_object(
          'confirmation_updates',(SELECT count(*) FROM unnest(ARRAY[
            'nonce_sha256','state','confirmed_at','execution_receipt','execution_sha256','executed_at'
          ]) a(name) WHERE has_column_privilege($1,'core_assistant_tools.confirmations',a.name,'UPDATE')),
          'current_history_reads',(SELECT count(*) FROM (VALUES
            ('core_history.current_connection_snapshot','document'),
            ('core_history.current_connection_snapshot','mapping'),
            ('core_history.current_connection_snapshot','snapshot_observed_at'),
            ('core_history.current_connection_snapshot','target_database'),
            ('core_history.current_connection_snapshot','target_database_oid'),
            ('core_history.current_connection_snapshot','target_nonce'),
            ('core_history.current_connection_credential_map','target_credential_id')
          ) t(relation,column_name) WHERE has_column_privilege($1,t.relation,t.column_name,'SELECT')),
          'confirmation_table_update',has_table_privilege($1,'core_assistant_tools.confirmations','UPDATE'),
          'key_card_updates',(SELECT count(*) FROM unnest(ARRAY['nonce_sha256','ciphertext','consumed_at'])
            a(name) WHERE has_column_privilege($1,'core_assistant_tools.key_creation_cards',a.name,'UPDATE')),
          'key_card_select',has_table_privilege($1,'core_assistant_tools.key_creation_cards','SELECT'),
          'key_card_insert',has_table_privilege($1,'core_assistant_tools.key_creation_cards','INSERT'),
          'key_card_table_update',has_table_privilege($1,'core_assistant_tools.key_creation_cards','UPDATE'),
          'market_confirmation_execute',has_function_privilege($1,'core_assistant_tools.market_confirmation_bound(text)','EXECUTE'),
          'alias_reads',(SELECT count(*) FROM unnest(ARRAY['code','user_id']) c(name)
            WHERE has_column_privilege($1,'core_identity.legacy_affiliate_aliases',c.name,'SELECT')),
          'identity_select',has_table_privilege($1,'core_identity.users','SELECT'),
          'wallet_select',has_table_privilege($1,'core_billing.balance_state','SELECT'))"#,
    )
    .bind(role)
    .fetch_one(pool)
    .await
    .unwrap()
}

#[sqlx::test(migrations = false)]
async fn optional_reset_requires_actual_owner_and_removes_current_column_authority(pool: PgPool) {
    let database = pool.connect_options().get_database().unwrap().to_owned();
    assert!(
        database
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'_')
    );
    let nonce = format!("{:016x}", rand::random::<u64>());
    let owner = format!("optional_reset_owner_{nonce}");
    let runtime = format!("optional_reset_runtime_{nonce}");
    sqlx::raw_sql(sqlx::AssertSqlSafe(format!(
        "CREATE ROLE {owner} NOLOGIN NOSUPERUSER NOCREATEDB NOCREATEROLE NOREPLICATION NOBYPASSRLS;
         CREATE ROLE {runtime} LOGIN NOSUPERUSER NOCREATEDB NOCREATEROLE NOREPLICATION NOBYPASSRLS;
         GRANT CREATE ON DATABASE \"{database}\" TO {owner};
         ALTER SCHEMA public OWNER TO {owner};"
    )))
    .execute(&pool)
    .await
    .unwrap();
    let owner_options = (*pool.connect_options())
        .clone()
        .options([("role", owner.as_str())]);
    let owner_pool = PgPoolOptions::new()
        .max_connections(1)
        .connect_with(owner_options)
        .await
        .unwrap();
    core_schema::init(&owner_pool).await.unwrap();
    let mut url = reqwest::Url::parse(&std::env::var("DATABASE_URL").unwrap()).unwrap();
    url.set_path(&format!("/{database}"));
    let source = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..");
    let apply = |script: &str, supplied_owner: &str| {
        Command::new("psql")
            .args(["-X", "-q", "-v", "ON_ERROR_STOP=1", "--dbname"])
            .arg(url.as_str())
            .arg("-v")
            .arg(format!("core_database={database}"))
            .arg("-v")
            .arg(format!("runtime_role={runtime}"))
            .arg("-v")
            .arg(format!("owner_role={supplied_owner}"))
            .args([
                "-v",
                "enable_community=true",
                "-v",
                "enable_web_identity=true",
                "-v",
                "enable_assistant_tools=true",
                "-v",
                "confirm_revoke=true",
                "-f",
            ])
            .arg(source.join("deployment/postgres").join(script))
            .output()
            .unwrap()
    };
    let granted = apply("grant-core-runtime.sql", &owner);
    assert!(
        granted.status.success(),
        "{}",
        String::from_utf8_lossy(&granted.stderr)
    );
    let before = authority(&pool, &runtime).await;
    assert_eq!(before["confirmation_updates"], 6);
    assert_eq!(before["current_history_reads"], 7);
    assert_eq!(before["confirmation_table_update"], false);
    assert_eq!(before["key_card_updates"], 3);
    assert_eq!(before["key_card_select"], true);
    assert_eq!(before["key_card_insert"], true);
    assert_eq!(before["key_card_table_update"], false);
    assert_eq!(before["market_confirmation_execute"], true);
    assert_eq!(before["alias_reads"], 2);
    assert_eq!(before["identity_select"], true);
    assert_eq!(before["wallet_select"], true);

    let refused = apply("revoke-core-runtime.sql", &format!("wrong_owner_{nonce}"));
    assert!(!refused.status.success());
    assert!(String::from_utf8_lossy(&refused.stderr).contains("Refusing unsafe runtime role"));
    assert_eq!(before, authority(&pool, &runtime).await);

    let revoked = apply("revoke-core-runtime.sql", &owner);
    assert!(
        revoked.status.success(),
        "{}",
        String::from_utf8_lossy(&revoked.stderr)
    );
    let after = authority(&pool, &runtime).await;
    assert_eq!(
        after["confirmation_updates"], 0,
        "table REVOKE must not leave column UPDATE"
    );
    assert_eq!(
        after["current_history_reads"], 0,
        "both current-history readers are optional"
    );
    assert_eq!(after["confirmation_table_update"], false);
    assert_eq!(
        after["key_card_updates"], 0,
        "key card column authority survived revoke"
    );
    assert_eq!(after["key_card_select"], false);
    assert_eq!(after["key_card_insert"], false);
    assert_eq!(after["key_card_table_update"], false);
    assert_eq!(after["market_confirmation_execute"], false);
    assert_eq!(after["alias_reads"], 0);
    assert_eq!(after["identity_select"], true);
    assert_eq!(after["wallet_select"], true);
    core_schema::check(&owner_pool).await.unwrap();

    // Re-provisioning the selected features must restore only their declared
    // column rights, never a blanket UPDATE or access to owner upgrade receipts.
    let regranted = apply("grant-core-runtime.sql", &owner);
    assert!(
        regranted.status.success(),
        "{}",
        String::from_utf8_lossy(&regranted.stderr)
    );
    assert_eq!(before, authority(&pool, &runtime).await);
    let receipts_readable: bool = sqlx::query_scalar(
        "SELECT has_any_column_privilege($1,'core_meta.assistant_confirmed_actions_upgrades','SELECT')
          OR has_any_column_privilege($1,'core_meta.current_connection_history_upgrades','SELECT')",
    ).bind(&runtime).fetch_one(&pool).await.unwrap();
    assert!(!receipts_readable);

    owner_pool.close().await;
    sqlx::raw_sql(sqlx::AssertSqlSafe(format!(
        "DROP OWNED BY {runtime}; DROP ROLE {runtime};
         DROP OWNED BY {owner} CASCADE; DROP ROLE {owner};"
    )))
    .execute(&pool)
    .await
    .unwrap();
}

//! Read-only verification of Passkey, account-link and closure authority.
use super::RuntimeDatabaseError;
use sqlx::PgConnection;

pub(super) async fn check(
    connection: &mut PgConnection,
    required: bool,
) -> Result<(), RuntimeDatabaseError> {
    let (unsafe_privileges, complete): (bool, bool) = sqlx::query_as(
        r#"
WITH expected_schemas(name) AS (
    VALUES ('core_passkeys'),('core_account_links'),('core_account_lifecycle')
), namespaces AS (
    SELECT n.oid,n.nspname FROM pg_catalog.pg_namespace n
    JOIN expected_schemas e ON e.name=n.nspname
), expected_tables(schema_name,name,can_delete) AS (
    VALUES ('core_passkeys','user_handles',false),
           ('core_passkeys','credentials',false),
           ('core_passkeys','flows',true),
           ('core_account_links','oauth_states',false),
           ('core_account_links','bind_flows',true),
           ('core_account_lifecycle','closures',false)
), allowed_columns(schema_name,table_name,column_name,privilege) AS (
    VALUES
      ('core_passkeys','user_handles','user_id','INSERT'),
      ('core_passkeys','credentials','user_id','INSERT'),
      ('core_passkeys','credentials','credential_id','INSERT'),
      ('core_passkeys','credentials','passkey','INSERT'),
      ('core_passkeys','credentials','name','INSERT'),
      ('core_passkeys','credentials','passkey','UPDATE'),
      ('core_passkeys','credentials','revision','UPDATE'),
      ('core_passkeys','credentials','last_used_at','UPDATE'),
      ('core_passkeys','credentials','revoked_at','UPDATE'),
      ('core_passkeys','flows','digest','INSERT'),
      ('core_passkeys','flows','binding_digest','INSERT'),
      ('core_passkeys','flows','purpose','INSERT'),
      ('core_passkeys','flows','user_id','INSERT'),
      ('core_passkeys','flows','user_version','INSERT'),
      ('core_passkeys','flows','sid','INSERT'),
      ('core_passkeys','flows','state','INSERT'),
      ('core_account_links','oauth_states','issuer','INSERT'),
      ('core_account_links','oauth_states','subject','INSERT'),
      ('core_account_links','oauth_states','user_id','INSERT'),
      ('core_account_links','oauth_states','revoked_at','INSERT'),
      ('core_account_links','oauth_states','revision','UPDATE'),
      ('core_account_links','oauth_states','revoked_at','UPDATE'),
      ('core_account_links','bind_flows','state_digest','INSERT'),
      ('core_account_links','bind_flows','issuer','INSERT'),
      ('core_account_links','bind_flows','binding_digest','INSERT'),
      ('core_account_links','bind_flows','user_id','INSERT'),
      ('core_account_links','bind_flows','user_version','INSERT'),
      ('core_account_links','bind_flows','sid','INSERT'),
      ('core_account_lifecycle','closures','user_id','INSERT'),
      ('core_account_lifecycle','closures','account_id','INSERT'),
      ('core_account_lifecycle','closures','credential_id','INSERT'),
      ('core_account_lifecycle','closures','previous_auth_version','INSERT'),
      ('core_account_lifecycle','closures','operation_digest','INSERT')
), relations AS (
    SELECT c.oid,n.nspname,c.relname,e.name AS expected_name,e.can_delete
    FROM pg_catalog.pg_class c JOIN namespaces n ON n.oid=c.relnamespace
    LEFT JOIN expected_tables e ON e.schema_name=n.nspname
        AND e.name=c.relname AND c.relkind='r'
    WHERE c.relkind IN ('r','p','v','m','f')
), functions AS (
    SELECT p.oid, n.nspname='core_account_lifecycle'
        AND p.proname='wallet_is_empty' AND p.pronargs=1
        AND p.proargtypes[0]=20 AND p.prorettype=16 AND p.prokind='f' AS allowed
    FROM pg_catalog.pg_proc p JOIN namespaces n ON n.oid=p.pronamespace
)
SELECT (
    EXISTS(SELECT FROM namespaces n WHERE
        pg_catalog.has_schema_privilege(n.oid,'CREATE,USAGE WITH GRANT OPTION'))
    OR EXISTS(SELECT FROM relations r WHERE
        pg_catalog.has_table_privilege(r.oid,
            'INSERT,UPDATE,TRUNCATE,TRIGGER,REFERENCES,MAINTAIN')
        OR pg_catalog.has_any_column_privilege(r.oid,'REFERENCES')
        OR pg_catalog.has_table_privilege(r.oid,
            'SELECT WITH GRANT OPTION,DELETE WITH GRANT OPTION')
        OR pg_catalog.has_any_column_privilege(r.oid,
            'SELECT WITH GRANT OPTION,INSERT WITH GRANT OPTION,UPDATE WITH GRANT OPTION')
        OR (r.can_delete IS DISTINCT FROM true
            AND pg_catalog.has_table_privilege(r.oid,'DELETE'))
        OR (r.expected_name IS NULL AND (
            pg_catalog.has_table_privilege(r.oid,'SELECT')
            OR pg_catalog.has_any_column_privilege(r.oid,'SELECT'))))
    OR EXISTS(
        SELECT FROM relations r JOIN pg_catalog.pg_attribute a ON a.attrelid=r.oid
        CROSS JOIN (VALUES ('INSERT'),('UPDATE')) p(privilege)
        WHERE a.attnum>0 AND NOT a.attisdropped
            AND pg_catalog.has_column_privilege(r.oid,a.attnum,p.privilege)
            AND NOT EXISTS(SELECT FROM allowed_columns permitted
                WHERE r.expected_name IS NOT NULL
                    AND permitted.schema_name=r.nspname
                    AND permitted.table_name=r.relname
                    AND permitted.column_name=a.attname
                    AND permitted.privilege=p.privilege))
    OR EXISTS(
        SELECT FROM pg_catalog.pg_class c JOIN namespaces n ON n.oid=c.relnamespace
        WHERE CASE WHEN c.relkind='S' THEN
            pg_catalog.has_sequence_privilege(c.oid,
                'SELECT,UPDATE,USAGE WITH GRANT OPTION,SELECT WITH GRANT OPTION,UPDATE WITH GRANT OPTION')
            OR ((n.nspname<>'core_passkeys' OR c.relname<>'credentials_id_seq')
                AND pg_catalog.has_sequence_privilege(c.oid,'USAGE'))
            ELSE FALSE END)
    OR EXISTS(SELECT FROM functions f WHERE
        pg_catalog.has_function_privilege(f.oid,'EXECUTE WITH GRANT OPTION')
        OR (NOT f.allowed AND pg_catalog.has_function_privilege(f.oid,'EXECUTE')))
) AS unsafe_privileges,
(
    NOT EXISTS(
        SELECT FROM expected_schemas e LEFT JOIN namespaces n ON n.nspname=e.name
        WHERE NOT COALESCE(pg_catalog.has_schema_privilege(n.oid,'USAGE'),false))
    AND NOT EXISTS(
        SELECT FROM expected_tables e
        LEFT JOIN namespaces n ON n.nspname=e.schema_name
        LEFT JOIN pg_catalog.pg_class c ON c.relnamespace=n.oid
            AND c.relname=e.name AND c.relkind='r'
        WHERE NOT COALESCE(pg_catalog.has_table_privilege(c.oid,'SELECT'),false)
            OR (e.can_delete
                AND NOT COALESCE(pg_catalog.has_table_privilege(c.oid,'DELETE'),false)))
    AND NOT EXISTS(
        SELECT FROM allowed_columns permitted
        LEFT JOIN namespaces n ON n.nspname=permitted.schema_name
        LEFT JOIN pg_catalog.pg_class c ON c.relnamespace=n.oid
            AND c.relname=permitted.table_name AND c.relkind='r'
        LEFT JOIN pg_catalog.pg_attribute a ON a.attrelid=c.oid
            AND a.attname=permitted.column_name AND a.attnum>0 AND NOT a.attisdropped
        WHERE a.attgenerated IS DISTINCT FROM ''
            OR NOT COALESCE(
                pg_catalog.has_column_privilege(c.oid,a.attnum,permitted.privilege),false))
    AND COALESCE((SELECT pg_catalog.has_sequence_privilege(c.oid,'USAGE')
        FROM namespaces n JOIN pg_catalog.pg_class c ON c.relnamespace=n.oid
        WHERE n.nspname='core_passkeys' AND c.relname='credentials_id_seq' AND c.relkind='S'),false)
    AND COALESCE((SELECT pg_catalog.has_function_privilege(f.oid,'EXECUTE')
        FROM functions f WHERE f.allowed),false)
) AS complete
"#,
    )
    .fetch_one(&mut *connection)
    .await?;
    if unsafe_privileges {
        return Err(RuntimeDatabaseError::UnsafePrivileges);
    }
    if !required {
        return Ok(());
    }
    if !complete {
        return Err(RuntimeDatabaseError::MissingPrivileges);
    }
    // Closure checks the actual ledgers and in-flight liabilities even when a
    // particular product router is disabled. These grants are read-only here.
    let closure_reads: bool = sqlx::query_scalar(
        r#"
WITH expected(schema_name,table_name) AS (
    VALUES ('core_charging','subscriptions'),('core_charging','requests'),
           ('core_commerce','orders'),('core_payments','intents'),
           ('core_payments','refund_holds'),('core_payments','blocked_refunds'),
           ('core_wallet','transfers')
)
SELECT NOT EXISTS(
    SELECT FROM expected e
    LEFT JOIN pg_catalog.pg_namespace n ON n.nspname=e.schema_name
    LEFT JOIN pg_catalog.pg_class c ON c.relnamespace=n.oid
        AND c.relname=e.table_name AND c.relkind='r'
    WHERE NOT COALESCE(pg_catalog.has_schema_privilege(n.oid,'USAGE'),false)
        OR NOT COALESCE(pg_catalog.has_table_privilege(c.oid,'SELECT'),false))
"#,
    )
    .fetch_one(connection)
    .await?;
    if !closure_reads {
        return Err(RuntimeDatabaseError::MissingPrivileges);
    }
    Ok(())
}

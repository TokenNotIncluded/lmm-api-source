//! Read-only verification of native federated provider authority.
use super::RuntimeDatabaseError;
use sqlx::PgConnection;

pub(super) async fn check(
    connection: &mut PgConnection,
    required: bool,
) -> Result<(), RuntimeDatabaseError> {
    // Catalog OIDs keep absent schemas, tables and columns from becoming name
    // resolution errors. Effective privileges include PUBLIC and inherited ACLs.
    let (unsafe_privileges, complete): (bool, bool) = sqlx::query_as(
        r#"
WITH namespace AS (
    SELECT oid FROM pg_catalog.pg_namespace WHERE nspname='core_federated_auth'
), expected_tables(name, can_delete) AS (
    VALUES ('providers',false),('identities',false),('flows',true),('assertions',true),('operations',false)
), allowed_columns(table_name, column_name, privilege) AS (
    VALUES
        ('providers','kind','INSERT'),
        ('providers','slug','INSERT'),
        ('providers','name','INSERT'),
        ('providers','icon','INSERT'),
        ('providers','enabled','INSERT'),
        ('providers','configuration','INSERT'),
        ('providers','secret_context','INSERT'),
        ('providers','secret_key_id','INSERT'),
        ('providers','sealed_secret','INSERT'),
        ('providers','slug','UPDATE'),
        ('providers','name','UPDATE'),
        ('providers','icon','UPDATE'),
        ('providers','enabled','UPDATE'),
        ('providers','configuration','UPDATE'),
        ('providers','secret_key_id','UPDATE'),
        ('providers','sealed_secret','UPDATE'),
        ('providers','revision','UPDATE'),
        ('providers','updated_at','UPDATE'),
        ('providers','deleted_at','UPDATE'),
        ('identities','provider_id','INSERT'),
        ('identities','subject','INSERT'),
        ('identities','subject_digest','INSERT'),
        ('identities','user_id','INSERT'),
        ('identities','revision','UPDATE'),
        ('identities','revoked_at','UPDATE'),
        ('flows','digest','INSERT'),
        ('flows','provider_id','INSERT'),
        ('flows','provider_revision','INSERT'),
        ('flows','purpose','INSERT'),
        ('flows','binding_digest','INSERT'),
        ('flows','nonce_digest','INSERT'),
        ('flows','pkce_verifier','INSERT'),
        ('flows','user_id','INSERT'),
        ('flows','user_version','INSERT'),
        ('flows','sid','INSERT'),
        ('flows','accepted_legal','INSERT'),
        ('flows','affiliate','INSERT'),
        ('flows','consumed_at','UPDATE'),
        ('assertions','digest','INSERT'),
        ('operations','actor_user_id','INSERT'),
        ('operations','operation_key','INSERT'),
        ('operations','sid','INSERT'),
        ('operations','request_digest','INSERT'),
        ('operations','action','INSERT'),
        ('operations','provider_id','INSERT'),
        ('operations','provider_revision','INSERT')
), relations AS (
    SELECT c.oid,c.relname,e.name AS expected_name,e.can_delete
    FROM pg_catalog.pg_class c JOIN namespace n ON n.oid=c.relnamespace
    LEFT JOIN expected_tables e ON e.name=c.relname AND c.relkind='r'
    WHERE c.relkind IN ('r','p','v','m','f')
)
SELECT (
    EXISTS(SELECT FROM namespace n WHERE
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
        SELECT FROM relations r
        JOIN pg_catalog.pg_attribute a ON a.attrelid=r.oid
        CROSS JOIN (VALUES ('INSERT'),('UPDATE')) p(privilege)
        WHERE a.attnum>0 AND NOT a.attisdropped
            AND pg_catalog.has_column_privilege(r.oid,a.attnum,p.privilege)
            AND NOT EXISTS(SELECT FROM allowed_columns permitted
                WHERE r.expected_name IS NOT NULL AND permitted.table_name=r.relname
                    AND permitted.column_name=a.attname
                    AND permitted.privilege=p.privilege))
    OR EXISTS(
        SELECT FROM pg_catalog.pg_class c JOIN namespace n ON n.oid=c.relnamespace
        WHERE CASE WHEN c.relkind='S'
            THEN (pg_catalog.has_sequence_privilege(c.oid,'SELECT,UPDATE,USAGE WITH GRANT OPTION')
                OR (c.relname<>'providers_id_seq' AND pg_catalog.has_sequence_privilege(c.oid,'USAGE')))
            ELSE FALSE END)
    OR EXISTS(
        SELECT FROM pg_catalog.pg_proc p JOIN namespace n ON n.oid=p.pronamespace
        WHERE pg_catalog.has_function_privilege(p.oid,'EXECUTE'))
) AS unsafe_privileges,
(
    COALESCE((SELECT pg_catalog.has_schema_privilege(n.oid,'USAGE')
        FROM namespace n),false)
    AND EXISTS(SELECT FROM pg_catalog.pg_class c JOIN namespace n ON n.oid=c.relnamespace
        WHERE c.relkind='S' AND c.relname='providers_id_seq'
            AND pg_catalog.has_sequence_privilege(c.oid,'USAGE'))
    AND NOT EXISTS(
        SELECT FROM expected_tables e
        LEFT JOIN namespace n ON true
        LEFT JOIN pg_catalog.pg_class c ON c.relnamespace=n.oid
            AND c.relname=e.name AND c.relkind='r'
        WHERE NOT COALESCE(pg_catalog.has_table_privilege(c.oid,'SELECT'),false)
            OR (e.can_delete
                AND NOT COALESCE(pg_catalog.has_table_privilege(c.oid,'DELETE'),false)))
    AND NOT EXISTS(
        SELECT FROM allowed_columns permitted
        LEFT JOIN namespace n ON true
        LEFT JOIN pg_catalog.pg_class c ON c.relnamespace=n.oid
            AND c.relname=permitted.table_name AND c.relkind='r'
        LEFT JOIN pg_catalog.pg_attribute a ON a.attrelid=c.oid
            AND a.attname=permitted.column_name AND a.attnum>0 AND NOT a.attisdropped
        WHERE a.attgenerated IS DISTINCT FROM ''
            OR NOT COALESCE(
                pg_catalog.has_column_privilege(c.oid,a.attnum,permitted.privilege),false))
) AS complete
"#,
    )
    .fetch_one(&mut *connection)
    .await?;
    if unsafe_privileges {
        return Err(RuntimeDatabaseError::UnsafePrivileges);
    }
    if required && !complete {
        return Err(RuntimeDatabaseError::MissingPrivileges);
    }
    Ok(())
}

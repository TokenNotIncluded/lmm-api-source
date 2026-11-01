//! Read-only checks of display facts; no authority over channel ledgers.
use super::RuntimeDatabaseError;
use sqlx::PgConnection;
pub(super) async fn check(
    c: &mut PgConnection,
    required: bool,
) -> Result<(), RuntimeDatabaseError> {
    let (unsafe_privileges, complete): (bool,bool) = sqlx::query_as(r#"
WITH ns AS (
 SELECT oid FROM pg_catalog.pg_namespace WHERE nspname='core_channels'
), balance_lock AS (
 SELECT p.oid,p.prosecdef,p.proconfig,p.prorettype,p.prokind
 FROM pg_catalog.pg_proc p JOIN pg_catalog.pg_namespace n ON n.oid=p.pronamespace
 WHERE n.nspname='public' AND p.proname='routing_control_balance_lock' AND p.pronargs=0
), expected(name) AS (
 VALUES ('identities'),('counter_baselines'),('balance_observations')
), updates(table_name,column_name) AS (
 VALUES ('identities','retired_at'),('balance_observations','balance'),('balance_observations','unit_label'),('balance_observations','observed_at'),('balance_observations','kind'),('balance_observations','credential_id'),('balance_observations','credential_revision'),('balance_observations','source_channel_id'),('balance_observations','recorded_at')
), relations AS (
 SELECT c.oid,c.relname,e.name AS expected_name
 FROM pg_catalog.pg_class c JOIN ns ON ns.oid=c.relnamespace
 LEFT JOIN expected e ON e.name=c.relname AND c.relkind='r'
 WHERE c.relkind IN ('r','p','v','m','f')
 UNION ALL
 SELECT c.oid,c.relname,NULL::text FROM pg_catalog.pg_class c
 JOIN pg_catalog.pg_namespace n ON n.oid=c.relnamespace
 WHERE n.nspname='core_meta' AND c.relname='channel_accounting_upgrades'
)
SELECT (
 EXISTS(SELECT FROM balance_lock f WHERE
   pg_catalog.has_function_privilege(f.oid,'EXECUTE WITH GRANT OPTION')
   OR NOT f.prosecdef OR f.prokind<>'f' OR f.prorettype<>2278
   OR NOT ('search_path=pg_catalog'=ANY(coalesce(f.proconfig,'{}'::text[])))
   OR EXISTS(SELECT FROM pg_catalog.pg_proc p
      CROSS JOIN LATERAL pg_catalog.aclexplode(coalesce(p.proacl,pg_catalog.acldefault('f',p.proowner))) a
      WHERE p.oid=f.oid AND a.grantee=0 AND a.privilege_type='EXECUTE'))
 OR EXISTS(SELECT FROM ns WHERE pg_catalog.has_schema_privilege(oid,'CREATE,USAGE WITH GRANT OPTION'))
 OR EXISTS(SELECT FROM relations r WHERE
   pg_catalog.has_table_privilege(r.oid,'UPDATE,DELETE,TRUNCATE,TRIGGER,REFERENCES,MAINTAIN')
   OR pg_catalog.has_any_column_privilege(r.oid,'REFERENCES')
   OR pg_catalog.has_table_privilege(r.oid,'SELECT WITH GRANT OPTION,INSERT WITH GRANT OPTION,UPDATE WITH GRANT OPTION')
   OR pg_catalog.has_any_column_privilege(r.oid,'SELECT WITH GRANT OPTION,INSERT WITH GRANT OPTION,UPDATE WITH GRANT OPTION')
   OR (r.expected_name IS NULL AND (
       pg_catalog.has_table_privilege(r.oid,'SELECT,INSERT')
       OR pg_catalog.has_any_column_privilege(r.oid,'SELECT,INSERT'))))
 OR EXISTS(SELECT FROM relations r JOIN pg_catalog.pg_attribute a ON a.attrelid=r.oid
   WHERE a.attnum>0 AND NOT a.attisdropped
     AND pg_catalog.has_column_privilege(r.oid,a.attnum,'UPDATE')
     AND NOT EXISTS(SELECT FROM updates u
        WHERE r.expected_name IS NOT NULL AND u.table_name=r.relname AND u.column_name=a.attname))
 OR EXISTS(SELECT FROM pg_catalog.pg_class c JOIN ns ON ns.oid=c.relnamespace
   WHERE CASE WHEN c.relkind='S' THEN pg_catalog.has_sequence_privilege(c.oid,'SELECT,UPDATE,USAGE') ELSE false END)
 OR EXISTS(SELECT FROM pg_catalog.pg_proc p JOIN ns ON ns.oid=p.pronamespace
   WHERE pg_catalog.has_function_privilege(p.oid,'EXECUTE'))
), (
 COALESCE((SELECT pg_catalog.has_function_privilege(oid,'EXECUTE') FROM balance_lock),false)
 AND COALESCE((SELECT pg_catalog.has_schema_privilege(oid,'USAGE') FROM pg_catalog.pg_namespace WHERE nspname='public'),false)
 AND COALESCE((SELECT pg_catalog.has_schema_privilege(oid,'USAGE') FROM ns),false)
 AND NOT EXISTS(SELECT FROM expected e LEFT JOIN relations r ON r.expected_name=e.name
   WHERE NOT COALESCE(pg_catalog.has_table_privilege(r.oid,'SELECT'),false)
      OR NOT COALESCE(pg_catalog.has_table_privilege(r.oid,'INSERT'),false))
 AND NOT EXISTS(SELECT FROM updates u LEFT JOIN relations r ON r.expected_name=u.table_name
   LEFT JOIN pg_catalog.pg_attribute a ON a.attrelid=r.oid AND a.attname=u.column_name AND a.attnum>0 AND NOT a.attisdropped
   WHERE NOT COALESCE(pg_catalog.has_column_privilege(r.oid,a.attnum,'UPDATE'),false))
)
"#).fetch_one(c).await?;
    if unsafe_privileges {
        return Err(RuntimeDatabaseError::UnsafePrivileges);
    }
    if required && !complete {
        return Err(RuntimeDatabaseError::MissingPrivileges);
    }
    Ok(())
}

//! Read-only effective authority for derived projections; no financial writes.
use super::RuntimeDatabaseError;
use sqlx::PgConnection;
pub(super) async fn check(
    c: &mut PgConnection,
    required: bool,
) -> Result<(), RuntimeDatabaseError> {
    let (unsafe_privileges,complete):(bool,bool)=sqlx::query_as(r#"
WITH ns AS (
 SELECT oid FROM pg_catalog.pg_namespace WHERE nspname='core_channels'
), expected(name,can_insert,can_update,can_delete) AS (
 VALUES ('usage_contributions',true,true,true),('usage_rollups',true,true,true),
        ('projection_dirty',false,false,true),('projection_state',false,false,false),
        ('opening_sources',false,false,false),('opening_baselines',true,false,false)
), relations AS (
 SELECT c.oid,c.relname,e.can_insert,e.can_update,e.can_delete,true AS allowed_read
 FROM pg_catalog.pg_class c JOIN ns ON ns.oid=c.relnamespace
 JOIN expected e ON e.name=c.relname WHERE c.relkind='r'
 UNION ALL
 SELECT c.oid,c.relname,false,false,false,false
 FROM pg_catalog.pg_class c JOIN pg_catalog.pg_namespace n ON n.oid=c.relnamespace
 WHERE n.nspname='core_meta' AND c.relname='channel_projection_upgrades'
)
SELECT (
 EXISTS(SELECT FROM pg_catalog.pg_proc p JOIN ns ON ns.oid=p.pronamespace
   WHERE p.proname IN ('mark_projection_dirty','check_opening_copy') AND pg_catalog.has_function_privilege(p.oid,'EXECUTE'))
 OR EXISTS(SELECT FROM relations r WHERE
   pg_catalog.has_table_privilege(r.oid,'TRUNCATE,TRIGGER,REFERENCES,MAINTAIN')
   OR pg_catalog.has_any_column_privilege(r.oid,'REFERENCES')
   OR pg_catalog.has_table_privilege(r.oid,'SELECT WITH GRANT OPTION,INSERT WITH GRANT OPTION,UPDATE WITH GRANT OPTION,DELETE WITH GRANT OPTION')
   OR pg_catalog.has_any_column_privilege(r.oid,'SELECT WITH GRANT OPTION,INSERT WITH GRANT OPTION,UPDATE WITH GRANT OPTION')
   OR (NOT r.can_insert AND (pg_catalog.has_table_privilege(r.oid,'INSERT') OR pg_catalog.has_any_column_privilege(r.oid,'INSERT')))
   OR (NOT r.can_update AND (pg_catalog.has_table_privilege(r.oid,'UPDATE') OR pg_catalog.has_any_column_privilege(r.oid,'UPDATE')))
   OR (NOT r.can_delete AND pg_catalog.has_table_privilege(r.oid,'DELETE'))
   OR (NOT r.allowed_read AND (pg_catalog.has_table_privilege(r.oid,'SELECT') OR pg_catalog.has_any_column_privilege(r.oid,'SELECT'))))
), (
 COALESCE((SELECT pg_catalog.has_schema_privilege(oid,'USAGE') FROM ns),false)
 AND NOT EXISTS(SELECT FROM expected e LEFT JOIN relations r ON r.relname=e.name
   WHERE NOT COALESCE(pg_catalog.has_table_privilege(r.oid,'SELECT'),false)
     OR (e.can_insert AND NOT COALESCE(pg_catalog.has_table_privilege(r.oid,'INSERT'),false))
     OR (e.can_update AND NOT COALESCE(pg_catalog.has_table_privilege(r.oid,'UPDATE'),false))
     OR (e.can_delete AND NOT COALESCE(pg_catalog.has_table_privilege(r.oid,'DELETE'),false)))
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

//! Effective managed-runtime authority, including denied owner maintenance.
use super::RuntimeDatabaseError;
use sqlx::PgConnection;

pub(super) async fn check(
    c: &mut PgConnection,
    required: bool,
) -> Result<(), RuntimeDatabaseError> {
    let unsafe_rights: bool = sqlx::query_scalar(r#"
SELECT EXISTS(SELECT FROM pg_class r JOIN pg_namespace n ON n.oid=r.relnamespace
 WHERE n.nspname='core_oauth' AND r.relname IN
 ('managed_authorization_bindings','managed_family_bindings','managed_refresh_lineage','managed_refresh_responses')
 AND (has_table_privilege(r.oid,'UPDATE,DELETE,TRUNCATE,TRIGGER,REFERENCES,MAINTAIN')
   OR has_any_column_privilege(r.oid,'UPDATE,REFERENCES')
   OR has_table_privilege(r.oid,'SELECT WITH GRANT OPTION,INSERT WITH GRANT OPTION')
   OR has_any_column_privilege(r.oid,'SELECT WITH GRANT OPTION,INSERT WITH GRANT OPTION')))
 OR EXISTS(SELECT FROM pg_proc p JOIN pg_namespace n ON n.oid=p.pronamespace
  WHERE n.nspname='core_oauth' AND p.proname='cleanup_managed_refresh_responses'
  AND has_function_privilege(p.oid,'EXECUTE'))
 OR EXISTS(SELECT FROM pg_class r JOIN pg_namespace n ON n.oid=r.relnamespace
  WHERE n.nspname='core_meta' AND r.relname='managed_applications_upgrades'
  AND (has_table_privilege(r.oid,'SELECT,INSERT,UPDATE,DELETE,TRUNCATE,TRIGGER,REFERENCES,MAINTAIN')
   OR has_any_column_privilege(r.oid,'SELECT,INSERT,UPDATE,REFERENCES')))
"#).fetch_one(&mut *c).await?;
    if unsafe_rights {
        return Err(RuntimeDatabaseError::UnsafePrivileges);
    }
    if required {
        let complete: bool = sqlx::query_scalar(
            r#"
SELECT has_schema_privilege('core_oauth','USAGE')
 AND NOT EXISTS(SELECT FROM (VALUES
  ('managed_authorization_bindings'),('managed_family_bindings'),
  ('managed_refresh_lineage'),('managed_refresh_responses')) t(name)
 CROSS JOIN (VALUES ('SELECT'),('INSERT')) p(privilege)
 WHERE NOT coalesce(has_table_privilege(to_regclass('core_oauth.'||t.name),p.privilege),false))
"#,
        )
        .fetch_one(&mut *c)
        .await?;
        if !complete {
            return Err(RuntimeDatabaseError::MissingPrivileges);
        }
    }
    Ok(())
}

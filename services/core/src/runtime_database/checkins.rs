//! Exact check-in privileges, including owner-only policy and history authority.
use super::RuntimeDatabaseError;
use sqlx::PgConnection;

pub(super) async fn check(
    connection: &mut PgConnection,
    required: bool,
) -> Result<(), RuntimeDatabaseError> {
    let installed: bool = sqlx::query_scalar("SELECT to_regnamespace('core_checkins') IS NOT NULL")
        .fetch_one(&mut *connection)
        .await?;
    if !installed {
        return if required {
            Err(RuntimeDatabaseError::MissingPrivileges)
        } else {
            Ok(())
        };
    }
    let unsafe_privileges:bool=sqlx::query_scalar(r#"
      SELECT EXISTS(SELECT FROM pg_class c JOIN pg_namespace n ON n.oid=c.relnamespace
        WHERE n.nspname='core_checkins' AND c.relkind IN ('r','p') AND (
          has_table_privilege(c.oid,'UPDATE,DELETE,TRUNCATE,REFERENCES,TRIGGER')
          OR (c.relname NOT IN ('days','claim_commands') AND has_table_privilege(c.oid,'INSERT'))
          OR (c.relname IN ('seed_receipts','owner_commands') AND has_table_privilege(c.oid,'SELECT'))))
      OR EXISTS(SELECT FROM pg_attribute a JOIN pg_class c ON c.oid=a.attrelid
        JOIN pg_namespace n ON n.oid=c.relnamespace WHERE n.nspname='core_checkins'
        AND a.attnum>0 AND NOT a.attisdropped AND (
          has_column_privilege(c.oid,a.attname,'UPDATE,REFERENCES')
          OR (c.relname NOT IN ('days','claim_commands') AND has_column_privilege(c.oid,a.attname,'INSERT'))
          OR (c.relname IN ('seed_receipts','owner_commands') AND has_column_privilege(c.oid,a.attname,'SELECT'))))
    "#).fetch_one(&mut *connection).await?;
    if unsafe_privileges {
        return Err(RuntimeDatabaseError::UnsafePrivileges);
    }
    if required {
        let complete:bool=sqlx::query_scalar(r#"
          SELECT has_schema_privilege('core_checkins','USAGE') AND
            (SELECT count(*)=4 AND coalesce(bool_and(has_table_privilege(c.oid,'SELECT')
              AND (c.relname NOT IN ('days','claim_commands') OR has_table_privilege(c.oid,'INSERT'))),false)
             FROM pg_class c JOIN pg_namespace n ON n.oid=c.relnamespace
             WHERE n.nspname='core_checkins' AND c.relkind='r'
             AND c.relname IN ('policy_revisions','active_policy','days','claim_commands'))
        "#).fetch_one(&mut *connection).await?;
        if !complete {
            return Err(RuntimeDatabaseError::MissingPrivileges);
        }
    }
    Ok(())
}

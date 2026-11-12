use super::RuntimeDatabaseError;
use sqlx::PgConnection;

pub(super) async fn check(c: &mut PgConnection) -> Result<(), RuntimeDatabaseError> {
    let unsafe_rights:bool=sqlx::query_scalar(r#"
        SELECT EXISTS(SELECT FROM pg_class c JOIN pg_namespace n ON n.oid=c.relnamespace
          WHERE n.nspname='core_operations' AND c.relkind='r' AND (
            has_table_privilege(c.oid,'TRUNCATE,TRIGGER,REFERENCES,MAINTAIN')
            OR (c.relname='upgrade_receipts' AND (has_table_privilege(c.oid,'SELECT,INSERT,UPDATE,DELETE') OR has_any_column_privilege(c.oid,'SELECT,INSERT,UPDATE,REFERENCES')))
            OR (c.relname='log_cleanup_jobs' AND (has_table_privilege(c.oid,'UPDATE,DELETE') OR has_any_column_privilege(c.oid,'UPDATE,REFERENCES')))))
    "#).fetch_one(&mut *c).await?;
    if unsafe_rights {
        return Err(RuntimeDatabaseError::UnsafePrivileges);
    }
    let complete:bool=sqlx::query_scalar(r#"
        SELECT has_schema_privilege('core_operations','USAGE')
          AND NOT EXISTS(SELECT FROM (VALUES
            ('instances','SELECT'),('instances','INSERT'),('instances','UPDATE'),('instances','DELETE'),
            ('log_cleanup_jobs','SELECT'),('log_cleanup_jobs','INSERT')) p(relation,privilege)
            WHERE NOT has_table_privilege('core_operations.'||p.relation,p.privilege))
          AND has_sequence_privilege('core_operations.log_cleanup_jobs_id_seq','USAGE')
          AND has_function_privilege('core_operations.run_log_cleanup_batch()','EXECUTE')
    "#).fetch_one(&mut *c).await?;
    if !complete {
        return Err(RuntimeDatabaseError::MissingPrivileges);
    }
    let limits_unsafe:bool=sqlx::query_scalar(r#"
      SELECT EXISTS(SELECT FROM pg_class c JOIN pg_namespace n ON n.oid=c.relnamespace
        WHERE n.nspname='core_request_limits' AND c.relkind='r' AND (
          has_table_privilege(c.oid,'INSERT,UPDATE,DELETE,TRUNCATE,TRIGGER,REFERENCES,MAINTAIN')
          OR has_any_column_privilege(c.oid,'INSERT,UPDATE,REFERENCES')
          OR (c.relname NOT IN ('policy','auto_policy') AND (has_table_privilege(c.oid,'SELECT') OR has_any_column_privilege(c.oid,'SELECT')))))
    "#).fetch_one(&mut *c).await?;
    if limits_unsafe {
        return Err(RuntimeDatabaseError::UnsafePrivileges);
    }
    let limits_complete:bool=sqlx::query_scalar(r#"
      SELECT has_schema_privilege('core_request_limits','USAGE')
        AND has_table_privilege('core_request_limits.policy','SELECT')
        AND has_table_privilege('core_request_limits.auto_policy','SELECT')
        AND has_function_privilege('core_request_limits.admit(bigint,text,text)','EXECUTE')
        AND has_function_privilege('core_request_limits.write_policy(bigint,boolean,integer,integer,jsonb,bigint)','EXECUTE')
        AND has_function_privilege('core_request_limits.write_auto_groups(bigint,text[],bigint)','EXECUTE')
    "#).fetch_one(c).await?;
    if !limits_complete {
        return Err(RuntimeDatabaseError::MissingPrivileges);
    }
    Ok(())
}

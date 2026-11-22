//! Exact runtime read surface for imported invitation aliases.
use super::RuntimeDatabaseError;
use sqlx::PgConnection;

pub(super) async fn check(
    connection: &mut PgConnection,
    required: bool,
) -> Result<(), RuntimeDatabaseError> {
    // Use catalog OIDs: a runtime without the optional namespaces still needs
    // its negative privilege check, without being given namespace USAGE.
    let (unsafe_access, readable): (bool, bool) = sqlx::query_as(
        r#"WITH expected(schema_name,table_name) AS (VALUES
             ('core_identity','legacy_affiliate_aliases'),
             ('core_meta','legacy_affiliate_alias_imports'),
             ('core_meta','legacy_affiliate_alias_upgrades')),
           relations AS (SELECT e.table_name,c.oid,n.oid AS schema_oid
             FROM expected e LEFT JOIN pg_catalog.pg_namespace n ON n.nspname=e.schema_name
             LEFT JOIN pg_catalog.pg_class c ON c.relnamespace=n.oid AND c.relname=e.table_name)
           SELECT EXISTS(SELECT FROM relations r WHERE
              has_table_privilege(r.oid,'INSERT,UPDATE,DELETE,TRUNCATE,TRIGGER,REFERENCES,MAINTAIN')
              OR has_any_column_privilege(r.oid,'INSERT,UPDATE,REFERENCES')
              OR EXISTS(SELECT FROM pg_catalog.pg_attribute a
                WHERE a.attrelid=r.oid AND a.attnum>0 AND NOT a.attisdropped
                  AND NOT (r.table_name='legacy_affiliate_aliases' AND a.attname IN ('code','user_id'))
                  AND has_column_privilege(r.oid,a.attnum,'SELECT')))
              OR EXISTS(SELECT FROM pg_catalog.pg_proc p JOIN pg_catalog.pg_namespace n ON n.oid=p.pronamespace
                WHERE (n.nspname='core_meta' AND p.proname IN ('legacy_affiliate_alias_authority',
                   'guard_legacy_affiliate_alias_import','guard_legacy_affiliate_alias_row',
                   'complete_legacy_affiliate_alias_import')
                 OR n.nspname='core_identity' AND p.proname='guard_affiliate_registration_inviter')
                AND has_function_privilege(p.oid,'EXECUTE')),
             EXISTS(SELECT FROM relations r WHERE r.table_name='legacy_affiliate_aliases'
                AND r.oid IS NOT NULL AND has_schema_privilege(r.schema_oid,'USAGE')
                AND has_column_privilege(r.oid,'code','SELECT')
                AND has_column_privilege(r.oid,'user_id','SELECT'))"#,
    )
    .fetch_one(&mut *connection)
    .await?;
    if unsafe_access {
        return Err(RuntimeDatabaseError::UnsafePrivileges);
    }
    if required && !readable {
        return Err(RuntimeDatabaseError::MissingPrivileges);
    }
    Ok(())
}

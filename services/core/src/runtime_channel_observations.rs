//! Display-only observations: exact effective column authority, no private receipt access.
use super::RuntimeDatabaseError;
use sqlx::PgConnection;
pub(super) async fn check(
    c: &mut PgConnection,
    required: bool,
) -> Result<(), RuntimeDatabaseError> {
    let (unsafe_privileges,complete):(bool,bool)=sqlx::query_as(r#"
WITH relation AS (
 SELECT c.oid,c.relowner,c.relacl,c.relkind FROM pg_catalog.pg_class c
 JOIN pg_catalog.pg_namespace n ON n.oid=c.relnamespace
 WHERE n.nspname='core_channels' AND c.relname='catalog_observations'
), expected(name,can_update) AS (VALUES
 ('channel_id',false),('credential_id',true),('credential_revision',true),
 ('binding_sha256',true),('scope',true),('result',true),('started_at',true),
 ('observed_at',true),('duration_ms',true)
), cols AS (
 SELECT a.*,e.name AS expected_name,e.can_update FROM relation r
 JOIN pg_catalog.pg_attribute a ON a.attrelid=r.oid
 LEFT JOIN expected e ON e.name=a.attname WHERE a.attnum>0 AND NOT a.attisdropped
), receipt AS (
 SELECT c.oid,c.relowner,c.relacl FROM pg_catalog.pg_class c
 JOIN pg_catalog.pg_namespace n ON n.oid=c.relnamespace
 WHERE n.nspname='core_meta' AND c.relname='channel_observations_upgrades'
)
SELECT (
 EXISTS(SELECT FROM relation r WHERE r.relkind<>'r'
   OR pg_catalog.has_table_privilege(r.oid,'SELECT,INSERT,UPDATE,DELETE,TRUNCATE,REFERENCES,TRIGGER,MAINTAIN')
   OR EXISTS(SELECT FROM pg_catalog.aclexplode(r.relacl) a WHERE a.grantee=0))
 OR EXISTS(SELECT FROM cols a WHERE
   pg_catalog.has_column_privilege(a.attrelid,a.attnum,'REFERENCES,SELECT WITH GRANT OPTION,INSERT WITH GRANT OPTION,UPDATE WITH GRANT OPTION')
   OR (NOT coalesce(a.can_update,false) AND pg_catalog.has_column_privilege(a.attrelid,a.attnum,'UPDATE'))
   OR (a.expected_name IS NULL AND pg_catalog.has_column_privilege(a.attrelid,a.attnum,'SELECT,INSERT,UPDATE'))
   OR EXISTS(SELECT FROM pg_catalog.aclexplode(a.attacl) p WHERE p.grantee=0))
 OR EXISTS(SELECT FROM receipt r WHERE
   pg_catalog.has_table_privilege(r.oid,'SELECT,INSERT,UPDATE,DELETE,TRUNCATE,REFERENCES,TRIGGER,MAINTAIN')
   OR pg_catalog.has_any_column_privilege(r.oid,'SELECT,INSERT,UPDATE,REFERENCES')
   OR EXISTS(SELECT FROM pg_catalog.aclexplode(r.relacl) a WHERE a.grantee<>r.relowner)
   OR EXISTS(SELECT FROM pg_catalog.pg_attribute c CROSS JOIN LATERAL pg_catalog.aclexplode(c.attacl) a
      WHERE c.attrelid=r.oid AND a.grantee<>r.relowner))
), (
 EXISTS(SELECT FROM relation WHERE relkind='r') AND EXISTS(SELECT FROM receipt)
 AND coalesce((SELECT pg_catalog.has_schema_privilege(oid,'USAGE') FROM pg_catalog.pg_namespace WHERE nspname='core_channels'),false)
 AND NOT EXISTS(SELECT FROM expected e LEFT JOIN cols a ON a.attname=e.name WHERE
   NOT coalesce(pg_catalog.has_column_privilege(a.attrelid,a.attnum,'SELECT'),false)
   OR NOT coalesce(pg_catalog.has_column_privilege(a.attrelid,a.attnum,'INSERT'),false)
   OR (e.can_update AND NOT coalesce(pg_catalog.has_column_privilege(a.attrelid,a.attnum,'UPDATE'),false)))
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
#[cfg(test)]
#[path = "runtime_channel_observations_tests.rs"]
mod tests;

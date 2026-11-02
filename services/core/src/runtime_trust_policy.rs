//! Independent editor state and the proof verifier remain owner-only.
use super::RuntimeDatabaseError;
use sqlx::PgConnection;

pub(super) async fn check(
    c: &mut PgConnection,
    required: bool,
) -> Result<(), RuntimeDatabaseError> {
    let (unsafe_privileges, complete): (bool, bool) = sqlx::query_as(r#"
WITH expected(signature) AS (VALUES
 ('read_policy_edit_description(bigint)'),
 ('read_policy_editors(bigint)'),
 ('policy_editor_ready(bytea)'),
 ('policy_edit_binding(text,jsonb)'),
 ('authorized_policy_preview(bytea,bigint,bigint,bigint,jsonb,jsonb,jsonb,jsonb,bytea,bytea)'),
 ('authorized_policy_resolve(bytea,bigint,bigint,bytea)'),
 ('policy_editor_context(text,bigint)'),
 ('authorized_policy_challenge(jsonb,bytea)'),
 ('policy_challenge_context(bytea,bytea,bigint,bigint)'),
 ('authorized_policy_confirm(jsonb,bytea)')
), functions AS (
 SELECT p.oid, p.proname||'('||replace(pg_catalog.oidvectortypes(p.proargtypes),' ','')||')' AS signature
 FROM pg_catalog.pg_proc p JOIN pg_catalog.pg_namespace n ON n.oid=p.pronamespace
 WHERE n.nspname='core_trust_pricing'
), protected_relations AS (
 SELECT c.oid FROM pg_catalog.pg_class c
 JOIN pg_catalog.pg_namespace n ON n.oid=c.relnamespace
 WHERE (n.nspname='core_trust_pricing' AND c.relname LIKE 'policy_edit%')
    OR (n.nspname='core_meta' AND c.relname='trust_policy_upgrades')
)
SELECT (
 EXISTS(SELECT FROM functions f
   WHERE pg_catalog.has_function_privilege(f.oid,'EXECUTE WITH GRANT OPTION')
      OR (pg_catalog.has_function_privilege(f.oid,'EXECUTE')
          AND NOT EXISTS(SELECT FROM expected e WHERE e.signature=f.signature)))
 OR EXISTS(SELECT FROM protected_relations r
   WHERE pg_catalog.has_table_privilege(r.oid,'SELECT,INSERT,UPDATE,DELETE,TRUNCATE,TRIGGER,REFERENCES,MAINTAIN')
      OR pg_catalog.has_any_column_privilege(r.oid,'SELECT,INSERT,UPDATE,REFERENCES'))
), (
 COALESCE((SELECT pg_catalog.has_schema_privilege(n.oid,'USAGE')
           FROM pg_catalog.pg_namespace n WHERE n.nspname='core_trust_pricing'),false)
 AND NOT EXISTS(SELECT FROM expected e LEFT JOIN functions f ON f.signature=e.signature
   WHERE NOT COALESCE(pg_catalog.has_function_privilege(f.oid,'EXECUTE'),false))
 AND COALESCE((SELECT pg_catalog.has_table_privilege(c.oid,'SELECT')
               FROM pg_catalog.pg_class c JOIN pg_catalog.pg_namespace n ON n.oid=c.relnamespace
               WHERE n.nspname='core_trust_pricing' AND c.relname='policies'),false)
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

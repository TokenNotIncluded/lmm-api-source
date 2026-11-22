//! Retained gift authority stays owner-only; runtime appends a real claim and
//! updates only its bounded server-observed risk evidence.
use super::RuntimeDatabaseError;
use sqlx::PgConnection;

pub(super) async fn check(
    c: &mut PgConnection,
    required: bool,
) -> Result<(), RuntimeDatabaseError> {
    let unsafe_privileges: bool = sqlx::query_scalar(r#"
        SELECT EXISTS(SELECT FROM pg_class t JOIN pg_namespace n ON n.oid=t.relnamespace
          WHERE n.nspname='core_gift' AND t.relkind IN ('r','p') AND (
            (t.relname IN ('legacy_installations','legacy_bindings','legacy_decisions','legacy_risk_memories','legacy_imports')
             AND (has_table_privilege(t.oid,'INSERT,UPDATE,DELETE,TRUNCATE,REFERENCES,TRIGGER,MAINTAIN')
                  OR has_any_column_privilege(t.oid,'INSERT,UPDATE,REFERENCES')))
            OR (t.relname IN ('legacy_claims','legacy_fingerprints') AND
                (has_table_privilege(t.oid,'UPDATE') OR has_any_column_privilege(t.oid,'UPDATE')))
            OR (t.relname='legacy_profiles' AND EXISTS(SELECT FROM pg_attribute a
                WHERE a.attrelid=t.oid AND a.attnum>0 AND NOT a.attisdropped
                AND a.attname NOT IN ('identity_hash','network_hash','observed_at')
                AND has_column_privilege(t.oid,a.attname,'UPDATE')))))
    "#).fetch_one(&mut *c).await?;
    if unsafe_privileges {
        return Err(RuntimeDatabaseError::UnsafePrivileges);
    }
    if required {
        let complete: bool = sqlx::query_scalar(r#"
            SELECT NOT EXISTS(SELECT FROM unnest(ARRAY['legacy_installations','legacy_bindings','legacy_decisions',
                'legacy_risk_memories','legacy_imports','legacy_profiles','legacy_fingerprints','legacy_claims']) t(name)
                WHERE NOT has_table_privilege('core_gift.'||t.name,'SELECT'))
              AND NOT EXISTS(SELECT FROM unnest(ARRAY['legacy_profiles','legacy_fingerprints','legacy_claims']) t(name)
                WHERE NOT has_table_privilege('core_gift.'||t.name,'INSERT'))
              AND NOT EXISTS(SELECT FROM unnest(ARRAY['identity_hash','network_hash','observed_at']) a(name)
                WHERE NOT has_column_privilege('core_gift.legacy_profiles',a.name,'UPDATE'))
              AND has_table_privilege('core_gift.legacy_fingerprints','DELETE')
              AND has_sequence_privilege('core_gift.legacy_fingerprints_id_seq','USAGE')
              AND has_function_privilege('core_gift.legacy_oauth_subject(bigint,text)','EXECUTE')
        "#).fetch_one(c).await?;
        if !complete {
            return Err(RuntimeDatabaseError::MissingPrivileges);
        }
    }
    Ok(())
}

//! Weekly history has one owner import surface and bounded runtime capabilities.
use super::{RequiredCapabilities, RuntimeDatabaseError};
use sqlx::PgConnection;
pub(super) async fn check(
    c: &mut PgConnection,
    required: RequiredCapabilities,
) -> Result<(), RuntimeDatabaseError> {
    let (unsafe_access, reader, browser, agent): (bool, bool, bool, bool) = sqlx::query_as(
        r#"WITH expected(schema_name,table_name) AS (VALUES
              ('core_weekly','weekly_discounts'),('core_weekly','weekly_discount_policy'),
              ('core_weekly','legacy_weekly_discount_bases'),
              ('core_payments','weekly_discount_owners'),
              ('core_meta','legacy_weekly_discount_imports'),('core_meta','assistant_weekly_discount_upgrades')),
           relations AS (SELECT e.table_name,c.oid,n.oid AS schema_oid
              FROM expected e LEFT JOIN pg_catalog.pg_namespace n ON n.nspname=e.schema_name
              LEFT JOIN pg_catalog.pg_class c ON c.relnamespace=n.oid AND c.relname=e.table_name),
           functions AS (SELECT p.oid,p.proname FROM pg_catalog.pg_proc p
              JOIN pg_catalog.pg_namespace n ON n.oid=p.pronamespace
              WHERE n.nspname='core_weekly' AND
                ((p.proname='lock_weekly_discount_policy' AND p.pronargs=0)
                 OR (p.proname='claim_weekly_discount_code' AND p.pronargs=2
                     AND p.proargtypes[0]='text'::regtype AND p.proargtypes[1]='text'::regtype)
                 OR (p.proname='claimed_weekly_discount_code' AND p.pronargs=3
                     AND p.proargtypes[0]='text'::regtype AND p.proargtypes[1]='bigint'::regtype
                     AND p.proargtypes[2]='bigint'::regtype))),
           policy_read AS (SELECT EXISTS(SELECT FROM relations
              WHERE table_name='weekly_discount_policy' AND oid IS NOT NULL
                AND has_schema_privilege(schema_oid,'USAGE') AND has_table_privilege(oid,'SELECT'))
              AND EXISTS(SELECT FROM functions WHERE proname='lock_weekly_discount_policy'
                AND has_function_privilege(oid,'EXECUTE')) AS ok)
           SELECT
             EXISTS(SELECT FROM pg_catalog.pg_namespace WHERE nspname='core_weekly'
               AND has_schema_privilege(oid,'CREATE'))
             OR EXISTS(SELECT FROM relations r WHERE r.table_name IN
                ('legacy_weekly_discount_bases','legacy_weekly_discount_imports','assistant_weekly_discount_upgrades')
                AND (has_table_privilege(r.oid,'SELECT,INSERT,UPDATE,DELETE,TRUNCATE,TRIGGER,REFERENCES,MAINTAIN')
                  OR has_any_column_privilege(r.oid,'SELECT,INSERT,UPDATE,REFERENCES')))
             OR EXISTS(SELECT FROM relations r WHERE r.table_name IN ('weekly_discount_policy','weekly_discount_owners')
                AND (has_table_privilege(r.oid,'INSERT,UPDATE,DELETE,TRUNCATE,TRIGGER,REFERENCES,MAINTAIN')
                  OR has_any_column_privilege(r.oid,'INSERT,UPDATE,REFERENCES')))
             OR EXISTS(SELECT FROM relations r WHERE r.table_name='weekly_discounts'
                AND (has_table_privilege(r.oid,'INSERT,UPDATE,DELETE,TRUNCATE,TRIGGER,REFERENCES,MAINTAIN')
                  OR has_any_column_privilege(r.oid,'REFERENCES')
                  OR EXISTS(SELECT FROM pg_catalog.pg_attribute a WHERE a.attrelid=r.oid
                    AND a.attnum>0 AND NOT a.attisdropped AND
                      ((a.attname NOT IN ('id','user_id','account_id','week_start','status','discount_percent',
                        'reason','conversation_id','operation_id','credential_id','auth_version','web_session_sid','created_at')
                         AND has_column_privilege(a.attrelid,a.attnum,'INSERT'))
                       OR (a.attname NOT IN ('claim_nonce_sha256','claim_nonce_expires_at')
                         AND has_column_privilege(a.attrelid,a.attnum,'UPDATE'))))))
             OR EXISTS(SELECT FROM pg_catalog.pg_proc p JOIN pg_catalog.pg_namespace n ON n.oid=p.pronamespace
                WHERE (n.nspname='core_weekly' AND p.proname IN
                    ('weekly_discount_immutable','weekly_discount_binding','weekly_discount_tool_binding')
                  OR n.nspname='core_payments' AND p.proname='weekly_coupon_terms_immutable'
                  OR n.nspname='core_meta' AND p.proname IN ('guard_legacy_weekly_discount_import',
                    'legacy_weekly_discount_evidence_immutable','complete_legacy_weekly_discount_import'))
                  AND has_function_privilege(p.oid,'EXECUTE')),
             EXISTS(SELECT FROM relations r WHERE r.table_name='weekly_discount_owners'
                AND r.oid IS NOT NULL AND has_schema_privilege(r.schema_oid,'USAGE')
                AND has_table_privilege(r.oid,'SELECT'))
               AND EXISTS(SELECT FROM relations r WHERE r.table_name='weekly_discounts'
                AND r.oid IS NOT NULL AND has_schema_privilege(r.schema_oid,'USAGE')
                AND (SELECT count(*)=2 FROM pg_catalog.pg_attribute a WHERE a.attrelid=r.oid
                  AND a.attnum>0 AND NOT a.attisdropped AND a.attname IN ('code_id','user_id')
                  AND has_column_privilege(a.attrelid,a.attnum,'SELECT'))),
             (SELECT ok FROM policy_read)
               AND EXISTS(SELECT FROM relations r WHERE r.table_name='weekly_discounts'
                 AND r.oid IS NOT NULL AND has_schema_privilege(r.schema_oid,'USAGE')
                 AND has_table_privilege(r.oid,'SELECT')
                 AND (SELECT count(*)=2 FROM pg_catalog.pg_attribute a WHERE a.attrelid=r.oid
                   AND a.attnum>0 AND NOT a.attisdropped
                   AND a.attname IN ('claim_nonce_sha256','claim_nonce_expires_at')
                   AND has_column_privilege(a.attrelid,a.attnum,'UPDATE')))
               AND EXISTS(SELECT FROM functions WHERE proname='claimed_weekly_discount_code'
                 AND has_function_privilege(oid,'EXECUTE'))
               AND EXISTS(SELECT FROM functions WHERE proname='claim_weekly_discount_code'
                 AND has_function_privilege(oid,'EXECUTE')),
             (SELECT ok FROM policy_read)
               AND EXISTS(SELECT FROM relations r WHERE r.table_name='weekly_discounts'
                 AND r.oid IS NOT NULL AND has_schema_privilege(r.schema_oid,'USAGE')
                 AND has_table_privilege(r.oid,'SELECT')
                 AND (SELECT count(*)=13 FROM pg_catalog.pg_attribute a WHERE a.attrelid=r.oid
                   AND a.attnum>0 AND NOT a.attisdropped
                   AND a.attname IN ('id','user_id','account_id','week_start','status','discount_percent',
                     'reason','conversation_id','operation_id','credential_id','auth_version','web_session_sid','created_at')
                   AND has_column_privilege(a.attrelid,a.attnum,'INSERT')))"#,
    )
    .fetch_one(c).await?;
    if unsafe_access {
        return Err(RuntimeDatabaseError::UnsafePrivileges);
    }
    if (required.payments && !reader)
        || (required.assistant_tools && !browser)
        || (required.assistant_agent && !agent)
    {
        return Err(RuntimeDatabaseError::MissingPrivileges);
    }
    Ok(())
}

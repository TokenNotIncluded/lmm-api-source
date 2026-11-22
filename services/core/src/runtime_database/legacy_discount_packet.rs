//! Source coupon owners, used counts and payable holds remain offline inputs.
use super::RuntimeDatabaseError;
use sqlx::PgConnection;

pub(super) async fn check(
    c: &mut PgConnection,
    payments: bool,
    packets: bool,
) -> Result<(), RuntimeDatabaseError> {
    // Optional runtimes have no USAGE on core_payments. Resolve the fixed
    // relation OIDs through catalogs so the negative write check still runs
    // without requiring or granting access to that disabled module.
    let (unsafe_writes, readable): (bool, bool) = sqlx::query_as(
        r#"WITH expected(name) AS (VALUES
            ('legacy_discount_bases'),('legacy_discount_holds'),
            ('legacy_discount_owners'),('legacy_discount_hold_state'),
            ('discount_code_usage')),
        relations AS (SELECT c.oid AS id,n.oid AS schema_id FROM expected e
            LEFT JOIN pg_catalog.pg_namespace n ON n.nspname='core_payments'
            LEFT JOIN pg_catalog.pg_class c ON c.relnamespace=n.oid AND c.relname=e.name)
        SELECT EXISTS(SELECT FROM relations WHERE
            has_table_privilege(id,'INSERT,UPDATE,DELETE,TRUNCATE,TRIGGER,REFERENCES,MAINTAIN')
            OR has_any_column_privilege(id,'INSERT,UPDATE,REFERENCES')),
            NOT EXISTS(SELECT FROM relations WHERE id IS NULL
                OR NOT has_schema_privilege(schema_id,'USAGE')
                OR NOT has_table_privilege(id,'SELECT'))"#,
    )
    .fetch_one(&mut *c)
    .await?;
    if unsafe_writes {
        return Err(RuntimeDatabaseError::UnsafePrivileges);
    }
    if (payments || packets) && !readable {
        return Err(RuntimeDatabaseError::MissingPrivileges);
    }
    if packets {
        let complete: bool = sqlx::query_scalar(
            "SELECT has_function_privilege('core_payments.lock_legacy_discount_code(bigint)','EXECUTE')
             AND has_table_privilege('core_subscriptions.reset_codes','SELECT')
             AND has_table_privilege('core_subscriptions.reset_vouchers','SELECT')",
        )
        .fetch_one(&mut *c)
        .await?;
        if !complete {
            return Err(RuntimeDatabaseError::MissingPrivileges);
        }
    }
    Ok(())
}

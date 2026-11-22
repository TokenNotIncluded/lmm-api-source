//! Source coupon owners, used counts and payable holds remain offline inputs.
use super::RuntimeDatabaseError;
use sqlx::PgConnection;

pub(super) async fn check(
    c: &mut PgConnection,
    payments: bool,
    packets: bool,
) -> Result<(), RuntimeDatabaseError> {
    let (unsafe_writes, readable): (bool, bool) = sqlx::query_as(
        r#"WITH expected(name) AS (VALUES
            ('core_payments.legacy_discount_bases'),('core_payments.legacy_discount_holds'),
            ('core_payments.legacy_discount_owners'),('core_payments.legacy_discount_hold_state'),
            ('core_payments.discount_code_usage')),
        relations AS (SELECT to_regclass(name) AS id FROM expected)
        SELECT EXISTS(SELECT FROM relations WHERE
            has_table_privilege(id,'INSERT,UPDATE,DELETE,TRUNCATE,TRIGGER,REFERENCES,MAINTAIN')
            OR has_any_column_privilege(id,'INSERT,UPDATE,REFERENCES')),
            NOT EXISTS(SELECT FROM relations WHERE id IS NULL OR NOT has_table_privilege(id,'SELECT'))"#,
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

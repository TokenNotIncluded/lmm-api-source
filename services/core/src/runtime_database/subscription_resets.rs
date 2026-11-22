//! Reset generations are append-only runtime facts; import bindings and codes
//! remain owner-only inputs. This check does not repair or grant privileges.
use super::RuntimeDatabaseError;
use sqlx::PgConnection;
pub(super) async fn check(
    c: &mut PgConnection,
    required: bool,
) -> Result<(), RuntimeDatabaseError> {
    let unsafe_writes:bool=sqlx::query_scalar("SELECT has_table_privilege('core_charging.subscription_request_resets','UPDATE,DELETE,TRUNCATE,REFERENCES,TRIGGER,MAINTAIN') OR has_any_column_privilege('core_charging.subscription_request_resets','UPDATE,REFERENCES')")
        .fetch_one(&mut *c).await?;
    if unsafe_writes {
        return Err(RuntimeDatabaseError::UnsafePrivileges);
    }
    if required {
        let complete:bool=sqlx::query_scalar("SELECT has_schema_privilege('core_subscriptions','USAGE') AND has_table_privilege('core_subscriptions.reset_events','SELECT') AND has_table_privilege('core_charging.subscription_request_resets','SELECT') AND has_table_privilege('core_charging.subscription_request_resets','INSERT') AND has_function_privilege('core_charging.subscription_base_window(bigint,bigint)','EXECUTE') AND has_function_privilege('core_charging.subscription_consumption(bigint,bigint,text)','EXECUTE')")
            .fetch_one(c).await?;
        if !complete {
            return Err(RuntimeDatabaseError::MissingPrivileges);
        }
    }
    Ok(())
}

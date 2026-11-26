//! Effective model runtime authority over shared health and cached hints.
use super::RuntimeDatabaseError;
use sqlx::PgConnection;

pub(super) async fn check(
    connection: &mut PgConnection,
    required: bool,
) -> Result<(), RuntimeDatabaseError> {
    let (unsafe_privileges, complete): (bool, bool) =
        sqlx::query_as(include_str!("route_runtime.sql"))
            .fetch_one(connection)
            .await?;
    if unsafe_privileges {
        return Err(RuntimeDatabaseError::UnsafePrivileges);
    }
    if required && !complete {
        return Err(RuntimeDatabaseError::MissingPrivileges);
    }
    Ok(())
}

#[cfg(test)]
mod tests;

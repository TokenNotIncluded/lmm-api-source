//! Optional administrator display provenance, evaluated once per result page.
use super::{AdminUsersError, ReadResult, UserRow};
use sqlx::{PgConnection, Postgres, Transaction};
use std::collections::HashSet;

const PAGE_ORIGINS: &str = concat!(
    include_str!("../../history_read/history_only_identities.sql"),
    " SELECT DISTINCT id::text FROM history_only WHERE id=ANY($1::bigint[])",
);

pub(super) async fn annotate(
    tx: &mut Transaction<'_, Postgres>,
    rows: &mut [UserRow],
) -> ReadResult<()> {
    let candidates: Vec<i64> = rows
        .iter()
        .filter(|row| {
            !row.active
                && !row.account_active
                && row.platform_role == "user"
                && row.auth_version == "1"
        })
        .map(|row| row.id.parse().map_err(|_| AdminUsersError::Unavailable))
        .collect::<ReadResult<_>>()?;
    if candidates.is_empty() {
        return Ok(());
    }

    let connection: &mut PgConnection = tx.as_mut();
    sqlx::query("SAVEPOINT admin_identity_origin")
        .execute(&mut *connection)
        .await?;
    let result = sqlx::query_scalar::<_, String>(PAGE_ORIGINS)
        .bind(&candidates)
        .fetch_all(&mut *connection)
        .await;
    let matches = match result {
        Ok(ids) => ids,
        // A base admin installation need not have the optional history schemas or grants.
        Err(sqlx::Error::Database(error))
            if matches!(
                error.code().as_deref(),
                Some("42501" | "42P01" | "42703" | "3F000")
            ) =>
        {
            sqlx::query("ROLLBACK TO SAVEPOINT admin_identity_origin")
                .execute(&mut *connection)
                .await?;
            Vec::new()
        }
        Err(error) => return Err(error.into()),
    };
    sqlx::query("RELEASE SAVEPOINT admin_identity_origin")
        .execute(&mut *connection)
        .await?;
    let matches: HashSet<String> = matches.into_iter().collect();
    for row in rows {
        if matches.contains(&row.id) {
            row.identity_origin = Some("history_only");
        }
    }
    Ok(())
}

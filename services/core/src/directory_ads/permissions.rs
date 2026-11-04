use crate::runtime_database::RuntimeDatabaseError;
use sqlx::PgConnection;

pub(crate) async fn check_privileges(
    connection: &mut PgConnection,
    enabled: bool,
) -> Result<(), RuntimeDatabaseError> {
    let unsafe_grants: bool = sqlx::query_scalar(
        r#"SELECT EXISTS (
          SELECT FROM pg_class c JOIN pg_namespace n ON n.oid=c.relnamespace
          WHERE n.nspname='core_directory_ads' AND c.relkind='r' AND (
            has_table_privilege(c.oid,'DELETE,TRUNCATE,TRIGGER,REFERENCES,MAINTAIN')
            OR has_any_column_privilege(c.oid,'REFERENCES')
            OR EXISTS (SELECT FROM pg_attribute a WHERE a.attrelid=c.oid
              AND a.attnum>0 AND NOT a.attisdropped
              AND has_column_privilege(c.oid,a.attname,'UPDATE')
              AND NOT ((c.relname='quotes' AND a.attname IN
                ('confirmation_digest','confirmation_sid','confirmed_sid','confirmed_at'))
                OR (c.relname='refunds' AND a.attname IN ('receipt','journal_id'))))))"#,
    )
    .fetch_one(&mut *connection)
    .await?;
    if unsafe_grants {
        return Err(RuntimeDatabaseError::UnsafePrivileges);
    }
    if !enabled {
        return Ok(());
    }
    let complete: bool = sqlx::query_scalar(
        r#"SELECT COALESCE((SELECT has_schema_privilege(oid,'USAGE')
          FROM pg_namespace WHERE nspname='core_directory_ads'),false)
          AND (SELECT count(*)=3 AND COALESCE(bool_and(
            has_table_privilege(c.oid,'SELECT') AND has_table_privilege(c.oid,'INSERT')),false)
            FROM pg_class c JOIN pg_namespace n ON n.oid=c.relnamespace
            WHERE n.nspname='core_directory_ads' AND c.relkind='r'
              AND c.relname IN ('quotes','payments','refunds'))"#,
    )
    .fetch_one(&mut *connection)
    .await?;
    if !complete {
        return Err(RuntimeDatabaseError::MissingPrivileges);
    }
    for (table, columns) in [
        (
            "core_directory_ads.quotes",
            &[
                "confirmation_digest",
                "confirmation_sid",
                "confirmed_sid",
                "confirmed_at",
            ][..],
        ),
        ("core_directory_ads.refunds", &["receipt", "journal_id"][..]),
    ] {
        for column in columns {
            let granted: bool = sqlx::query_scalar("SELECT has_column_privilege($1,$2,'UPDATE')")
                .bind(table)
                .bind(column)
                .fetch_one(&mut *connection)
                .await?;
            if !granted {
                return Err(RuntimeDatabaseError::MissingPrivileges);
            }
        }
    }
    Ok(())
}

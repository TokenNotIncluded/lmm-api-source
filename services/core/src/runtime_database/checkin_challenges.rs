//! Short-lived proof state cannot grant mutable reward receipts or installer authority.
use super::RuntimeDatabaseError;
use sqlx::PgConnection;

pub(super) async fn check(
    c: &mut PgConnection,
    required: bool,
) -> Result<(), RuntimeDatabaseError> {
    let installed: bool =
        sqlx::query_scalar("SELECT to_regnamespace('core_checkin_challenges') IS NOT NULL")
            .fetch_one(&mut *c)
            .await?;
    if !installed {
        return if required {
            Err(RuntimeDatabaseError::MissingPrivileges)
        } else {
            Ok(())
        };
    }
    // Resolve through the readable catalogs. Text-to-regclass resolution itself
    // requires schema USAGE, which is precisely the missing grant being checked.
    let ids: Option<(i64,i64,i64,i64)> = sqlx::query_as(r#"
        SELECT n.oid::bigint,c.oid::bigint,r.oid::bigint,u.oid::bigint
        FROM pg_namespace n
        JOIN pg_class c ON c.relnamespace=n.oid AND c.relname='challenges' AND c.relkind='r'
        JOIN pg_class r ON r.relnamespace=n.oid AND r.relname='receipts' AND r.relkind='r'
        JOIN pg_namespace m ON m.nspname='core_meta'
        JOIN pg_class u ON u.relnamespace=m.oid AND u.relname='checkin_challenges_upgrades' AND u.relkind='r'
        WHERE n.nspname='core_checkin_challenges'
    "#).fetch_optional(&mut *c).await?;
    let Some((schema, challenges, receipts, upgrades)) = ids else {
        return Err(RuntimeDatabaseError::MissingPrivileges);
    };
    let unsafe_rights: bool = sqlx::query_scalar(r#"
        SELECT has_schema_privilege($1::oid,'CREATE')
        OR has_table_privilege($2::oid,'TRUNCATE,TRIGGER,REFERENCES,MAINTAIN')
        OR has_any_column_privilege($2::oid,'REFERENCES')
        OR EXISTS(SELECT FROM pg_attribute a WHERE a.attrelid=$2::oid
          AND a.attnum>0 AND NOT a.attisdropped AND a.attname NOT IN ('attempts','verified_at','consumed_at')
          AND has_column_privilege(a.attrelid,a.attname,'UPDATE'))
        OR has_table_privilege($3::oid,'UPDATE,DELETE,TRUNCATE,TRIGGER,REFERENCES,MAINTAIN')
        OR has_any_column_privilege($3::oid,'UPDATE,REFERENCES')
        OR has_table_privilege($4::oid,'SELECT,INSERT,UPDATE,DELETE,TRUNCATE,TRIGGER,REFERENCES,MAINTAIN')
        OR has_any_column_privilege($4::oid,'SELECT,INSERT,UPDATE,REFERENCES')
    "#).bind(schema).bind(challenges).bind(receipts).bind(upgrades).fetch_one(&mut *c).await?;
    if unsafe_rights {
        return Err(RuntimeDatabaseError::UnsafePrivileges);
    }
    if required {
        let complete: bool = sqlx::query_scalar(
            r#"
            SELECT has_schema_privilege($1::oid,'USAGE')
            AND has_table_privilege($2::oid,'SELECT')
            AND has_table_privilege($2::oid,'INSERT')
            AND has_table_privilege($2::oid,'DELETE')
            AND has_column_privilege($2::oid,'attempts','UPDATE')
            AND has_column_privilege($2::oid,'verified_at','UPDATE')
            AND has_column_privilege($2::oid,'consumed_at','UPDATE')
            AND has_table_privilege($3::oid,'SELECT')
            AND has_table_privilege($3::oid,'INSERT')
        "#,
        )
        .bind(schema)
        .bind(challenges)
        .bind(receipts)
        .fetch_one(&mut *c)
        .await?;
        if !complete {
            return Err(RuntimeDatabaseError::MissingPrivileges);
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests;

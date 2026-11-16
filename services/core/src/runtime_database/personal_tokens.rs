//! The personal-token table never grants database or native credential authority.
use super::RuntimeDatabaseError;
use sqlx::PgConnection;

pub(super) async fn check(
    c: &mut PgConnection,
    required: bool,
) -> Result<(), RuntimeDatabaseError> {
    let installed: bool =
        sqlx::query_scalar("SELECT to_regnamespace('core_personal_tokens') IS NOT NULL")
            .fetch_one(&mut *c)
            .await?;
    if !installed {
        return if required {
            Err(RuntimeDatabaseError::MissingPrivileges)
        } else {
            Ok(())
        };
    }
    // A disabled feature has no schema USAGE. Resolve the exact objects from
    // pg_catalog so inspection cannot fail during SQL name lookup or miss an
    // unsafe table grant merely because schema access is currently absent.
    let (tokens, receipt): (i64, i64) = sqlx::query_as(
        "SELECT t.oid::bigint,r.oid::bigint FROM pg_class t JOIN pg_namespace tn ON tn.oid=t.relnamespace CROSS JOIN pg_class r JOIN pg_namespace rn ON rn.oid=r.relnamespace WHERE tn.nspname='core_personal_tokens' AND t.relname='tokens' AND rn.nspname='core_meta' AND r.relname='personal_tokens_upgrades'",
    )
    .fetch_one(&mut *c)
    .await?;
    let unsafe_rights: bool = sqlx::query_scalar("SELECT has_schema_privilege('core_personal_tokens','CREATE') OR has_table_privilege($1::oid,'DELETE,TRUNCATE,TRIGGER,REFERENCES,MAINTAIN') OR has_any_column_privilege($1::oid,'REFERENCES') OR has_table_privilege($2::oid,'SELECT,INSERT,UPDATE,DELETE,TRUNCATE,TRIGGER,REFERENCES,MAINTAIN') OR has_any_column_privilege($2::oid,'SELECT,INSERT,UPDATE,REFERENCES')")
        .bind(tokens).bind(receipt).fetch_one(&mut *c).await?;
    if unsafe_rights {
        return Err(RuntimeDatabaseError::UnsafePrivileges);
    }
    if required {
        let complete: bool = sqlx::query_scalar("SELECT has_schema_privilege('core_personal_tokens','USAGE') AND has_table_privilege($1::oid,'SELECT') AND has_table_privilege($1::oid,'INSERT') AND has_table_privilege($1::oid,'UPDATE')")
            .bind(tokens).fetch_one(&mut *c).await?;
        if !complete {
            return Err(RuntimeDatabaseError::MissingPrivileges);
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use sqlx::PgPool;

    #[sqlx::test(migrations = false)]
    async fn disabled_schema_still_detects_unsafe_personal_token_privileges(pool: PgPool) {
        crate::core_schema::init(&pool).await.unwrap();
        let role = crate::test_runtime::RuntimeRole::create(&pool).await;
        sqlx::raw_sql(sqlx::AssertSqlSafe(format!(
            "REVOKE ALL ON SCHEMA core_personal_tokens FROM {0}; REVOKE ALL ON core_personal_tokens.tokens FROM {0}",
            role.name
        ))).execute(&pool).await.unwrap();
        let mut connection = role.pool.acquire().await.unwrap();
        check(&mut connection, false).await.unwrap();
        assert_eq!(
            check(&mut connection, true).await,
            Err(RuntimeDatabaseError::MissingPrivileges)
        );
        sqlx::raw_sql(sqlx::AssertSqlSafe(format!(
            "GRANT SELECT,INSERT,UPDATE ON core_personal_tokens.tokens TO {}",
            role.name
        )))
        .execute(&pool)
        .await
        .unwrap();
        assert_eq!(
            check(&mut connection, true).await,
            Err(RuntimeDatabaseError::MissingPrivileges)
        );
        sqlx::raw_sql(sqlx::AssertSqlSafe(format!(
            "GRANT USAGE ON SCHEMA core_personal_tokens TO {}",
            role.name
        )))
        .execute(&pool)
        .await
        .unwrap();
        check(&mut connection, true).await.unwrap();
        sqlx::raw_sql(sqlx::AssertSqlSafe(format!(
            "REVOKE USAGE ON SCHEMA core_personal_tokens FROM {}",
            role.name
        )))
        .execute(&pool)
        .await
        .unwrap();

        for (grant, revoke) in [
            (
                "GRANT DELETE ON core_personal_tokens.tokens TO",
                "REVOKE DELETE ON core_personal_tokens.tokens FROM",
            ),
            (
                "GRANT SELECT ON core_meta.personal_tokens_upgrades TO",
                "REVOKE SELECT ON core_meta.personal_tokens_upgrades FROM",
            ),
            (
                "GRANT CREATE ON SCHEMA core_personal_tokens TO",
                "REVOKE CREATE ON SCHEMA core_personal_tokens FROM",
            ),
        ] {
            sqlx::raw_sql(sqlx::AssertSqlSafe(format!("{grant} {}", role.name)))
                .execute(&pool)
                .await
                .unwrap();
            for required in [false, true] {
                assert_eq!(
                    check(&mut connection, required).await,
                    Err(RuntimeDatabaseError::UnsafePrivileges)
                );
            }
            sqlx::raw_sql(sqlx::AssertSqlSafe(format!("{revoke} {}", role.name)))
                .execute(&pool)
                .await
                .unwrap();
            check(&mut connection, false).await.unwrap();
        }
        drop(connection);
        role.cleanup().await;
    }
}

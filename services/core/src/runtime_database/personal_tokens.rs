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
    let unsafe_rights: bool = sqlx::query_scalar("SELECT has_schema_privilege('core_personal_tokens','CREATE') OR has_table_privilege('core_personal_tokens.tokens','DELETE,TRUNCATE,TRIGGER,REFERENCES,MAINTAIN') OR has_any_column_privilege('core_personal_tokens.tokens','REFERENCES') OR has_table_privilege('core_meta.personal_tokens_upgrades','SELECT,INSERT,UPDATE,DELETE,TRUNCATE,TRIGGER,REFERENCES,MAINTAIN') OR has_any_column_privilege('core_meta.personal_tokens_upgrades','SELECT,INSERT,UPDATE,REFERENCES')")
        .fetch_one(&mut *c).await?;
    if unsafe_rights {
        return Err(RuntimeDatabaseError::UnsafePrivileges);
    }
    if required {
        let complete: bool = sqlx::query_scalar("SELECT has_schema_privilege('core_personal_tokens','USAGE') AND has_table_privilege('core_personal_tokens.tokens','SELECT') AND has_table_privilege('core_personal_tokens.tokens','INSERT') AND has_table_privilege('core_personal_tokens.tokens','UPDATE')")
            .fetch_one(&mut *c).await?;
        if !complete {
            return Err(RuntimeDatabaseError::MissingPrivileges);
        }
    }
    Ok(())
}

//! Core checks current administrator/target authority; Go retains skill content.
//! This observation is not a reusable grant. Callers recheck it around every read.
use super::Error;
use crate::identity::{
    admin_users::AdminUsersError,
    permissions::{self, Action, Credential, PlatformRole, Scope},
};
use serde::Serialize;
use serde_json::json;
use sha2::{Digest, Sha256};
use sqlx::{PgConnection, PgPool, Row};

pub(crate) const FEATURE: &str = "assistant.admin_user_skills.v1";

#[derive(Clone, Debug, Serialize)]
pub(crate) struct Authorization {
    pub version: u32,
    pub actor_user_id: i64,
    pub target_user_id: i64,
    pub actor_auth_version: i64,
    pub target_auth_version: i64,
    pub actor_role: String,
    pub target_role: String,
    pub authorization_binding_sha256: String,
    pub checked_at_unix: i64,
}

fn permitted_admin_target(actor: i64, target: i64, actor_role: &str, target_role: &str) -> bool {
    if actor <= 0 || target <= 0 {
        return false;
    }
    if actor == target {
        return matches!(
            (actor_role, target_role),
            ("admin", "admin") | ("superadmin", "superadmin")
        );
    }
    matches!(
        (actor_role, target_role),
        ("admin", "user") | ("superadmin", "user" | "admin")
    )
}

fn role(value: &str) -> Result<PlatformRole, Error> {
    match value {
        "user" => Ok(PlatformRole::User),
        "admin" => Ok(PlatformRole::Admin),
        "superadmin" => Ok(PlatformRole::Root),
        _ => Err(Error::Unavailable),
    }
}

pub(crate) async fn check_ready(pool: &PgPool) -> Result<(), Error> {
    let mut tx = pool.begin().await?;
    sqlx::query("SET TRANSACTION READ ONLY")
        .execute(&mut *tx)
        .await?;
    for sql in [
        "SELECT id,personal_account_id,auth_version,platform_role,active FROM core_identity.users WHERE FALSE",
        "SELECT id,kind,active FROM core_identity.accounts WHERE FALSE",
        "SELECT user_id,revision,contract,sha256,canonical_payload,verified_complete FROM core_identity.permission_policies WHERE FALSE",
        "SELECT user_id,action,effect FROM core_identity.permission_policy_rows WHERE FALSE",
    ] {
        sqlx::query(sql).fetch_all(&mut *tx).await?;
    }
    tx.commit().await?;
    Ok(())
}

pub(crate) async fn read(
    pool: &PgPool,
    original_session: &str,
    target_user_id: i64,
) -> Result<Authorization, Error> {
    if target_user_id <= 0 {
        return Err(Error::Invalid);
    }
    let mut tx = pool.begin().await?;
    sqlx::query("SET TRANSACTION ISOLATION LEVEL REPEATABLE READ READ ONLY")
        .execute(&mut *tx)
        .await?;
    sqlx::query("SET LOCAL statement_timeout='5s'")
        .execute(&mut *tx)
        .await?;
    let actor = permissions::authorize_admin_read_snapshot(&mut tx, original_session)
        .await
        .map_err(admission_error)?;
    let result = observe(&mut tx, actor, target_user_id, AuthorityView::Snapshot).await?;
    tx.commit().await?;
    Ok(result)
}

/// Use only in the existing Core serializable operation transaction. This keeps
/// authority locks until the result is accepted or a model reservation commits;
/// it never opens a second pool connection or changes the transaction isolation.
pub(crate) async fn read_in_transaction(
    c: &mut PgConnection,
    original_session: &str,
    target_user_id: i64,
) -> Result<Authorization, Error> {
    if target_user_id <= 0 {
        return Err(Error::Invalid);
    }
    let actor = permissions::authorize_admin_read_locked(c, original_session)
        .await
        .map_err(admission_error)?;
    observe(c, actor, target_user_id, AuthorityView::Locked).await
}

fn admission_error(error: AdminUsersError) -> Error {
    match error {
        AdminUsersError::Unauthorized => Error::Unauthorized,
        AdminUsersError::Unavailable => Error::Unavailable,
        _ => Error::Forbidden,
    }
}

enum AuthorityView {
    Snapshot,
    Locked,
}

async fn observe(
    c: &mut PgConnection,
    actor: crate::identity::Principal,
    target_user_id: i64,
    view: AuthorityView,
) -> Result<Authorization, Error> {
    let actor_row = sqlx::query(
        "SELECT u.platform_role,u.auth_version,u.active,a.active AS account_active
         FROM core_identity.users u JOIN core_identity.accounts a ON a.id=u.personal_account_id
         WHERE u.id=$1 AND u.personal_account_id=$2 AND a.kind='personal'",
    )
    .bind(actor.user_id)
    .bind(actor.owner_account_id)
    .fetch_optional(&mut *c)
    .await?
    .ok_or(Error::Unauthorized)?;
    let actor_role: String = actor_row.try_get("platform_role")?;
    let actor_version: i64 = actor_row.try_get("auth_version")?;
    let actor_active: bool = actor_row.try_get("active")?;
    let actor_account_active: bool = actor_row.try_get("account_active")?;
    if !actor_active || !actor_account_active || actor_version != actor.credential_generation() {
        return Err(Error::Unauthorized);
    }
    let target_sql = match view {
        AuthorityView::Snapshot => {
            "SELECT u.id,u.personal_account_id,u.platform_role,u.auth_version,u.active,
                    a.active AS account_active
             FROM core_identity.users u JOIN core_identity.accounts a ON a.id=u.personal_account_id
             WHERE u.id=$1 AND a.kind='personal'"
        }
        AuthorityView::Locked => {
            "SELECT u.id,u.personal_account_id,u.platform_role,u.auth_version,u.active,
                    a.active AS account_active
             FROM core_identity.users u JOIN core_identity.accounts a ON a.id=u.personal_account_id
             WHERE u.id=$1 AND a.kind='personal' FOR SHARE OF u,a"
        }
    };
    let target = sqlx::query(target_sql)
        .bind(target_user_id)
        .fetch_optional(&mut *c)
        .await?
        .ok_or(Error::Forbidden)?;
    let target_role: String = target.try_get("platform_role")?;
    let target_account: i64 = target.try_get("personal_account_id")?;
    let target_version: i64 = target.try_get("auth_version")?;
    let target_active: bool = target.try_get("active")?;
    let target_account_active: bool = target.try_get("account_active")?;
    if !permitted_admin_target(actor.user_id, target_user_id, &actor_role, &target_role) {
        return Err(Error::Forbidden);
    }

    // The shared admission already rejects missing imported-admin policy.
    // Validate any stored native policy too; a malformed snapshot never becomes
    // a role-only fallback. UserRead is a static role action outside the nine
    // dynamic channel/acquisition permissions.
    let policy = match permissions::load_complete_policy(c, actor.user_id).await {
        Ok(policy) => Some(policy),
        Err(permissions::PolicyLoadError::Missing) => None,
        Err(permissions::PolicyLoadError::Storage) => return Err(Error::Unavailable),
        Err(_) => return Err(Error::Forbidden),
    };
    if let Some(policy) = &policy
        && !permissions::decide(
            &permissions::Actor {
                user_id: actor.user_id,
                personal_account_id: actor.owner_account_id,
                role: role(&actor_role)?,
                credential: Credential::Session,
                active: actor_active,
                account_active: actor_account_active,
                auth_version: actor_version,
                credential_auth_version: actor.credential_generation(),
            },
            &policy.as_policy(),
            if actor.user_id == target_user_id {
                Action::OwnProfileRead
            } else {
                Action::UserRead
            },
            Scope::User {
                user_id: target_user_id,
                account_id: target_account,
                role: role(&target_role)?,
            },
            None,
        )
        .allowed()
    {
        return Err(Error::Forbidden);
    }
    let policy_binding = policy
        .as_ref()
        .map(|policy| (policy.revision, crate::hex_codec::encode(policy.sha256)));
    let binding = json!({
        "contract": FEATURE,
        "actor_user_id": actor.user_id,
        "actor_account_id": actor.owner_account_id,
        "actor_credential_id": actor.credential_id,
        "actor_auth_version": actor_version,
        "actor_role": actor_role,
        "target_user_id": target_user_id,
        "target_account_id": target_account,
        "target_auth_version": target_version,
        "target_role": target_role,
        "target_active": target_active,
        "target_account_active": target_account_active,
        "complete_policy": policy_binding,
    });
    let digest = Sha256::digest(serde_json::to_vec(&binding).map_err(|_| Error::Unavailable)?);
    let checked_at_unix: i64 =
        sqlx::query_scalar("SELECT floor(extract(epoch FROM transaction_timestamp()))::bigint")
            .fetch_one(&mut *c)
            .await?;
    Ok(Authorization {
        version: 1,
        actor_user_id: actor.user_id,
        target_user_id,
        actor_auth_version: actor_version,
        target_auth_version: target_version,
        actor_role,
        target_role,
        authorization_binding_sha256: crate::hex_codec::encode(digest),
        checked_at_unix,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn private_skills_keep_admin_self_read_and_strict_cross_user_roles() {
        let cross_user = [
            ("admin", "user"),
            ("superadmin", "user"),
            ("superadmin", "admin"),
        ];
        let self_read = [("admin", "admin"), ("superadmin", "superadmin")];
        for actor in ["user", "admin", "superadmin", "unknown"] {
            for target in ["user", "admin", "superadmin", "unknown"] {
                assert_eq!(
                    permitted_admin_target(1, 2, actor, target),
                    cross_user.contains(&(actor, target))
                );
                assert_eq!(
                    permitted_admin_target(1, 1, actor, target),
                    self_read.contains(&(actor, target))
                );
                assert!(!permitted_admin_target(0, 2, actor, target));
                assert!(!permitted_admin_target(1, 0, actor, target));
            }
        }
    }
}

#[cfg(test)]
#[path = "admin_skills_pg_tests.rs"]
mod pg_tests;

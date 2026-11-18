//! Native routing-group administration. Hidden account policy is never edited.
use super::{AdminUsersStore, ProfileError, security};
use crate::identity::{self, CredentialKind};
use axum::{
    Json, Router,
    extract::{
        DefaultBodyLimit, Path, State,
        rejection::{JsonRejection, PathRejection},
    },
    http::HeaderMap,
    routing::get,
};
use serde::{Deserialize, Serialize};
use sqlx::PgConnection;
type Result<T> = std::result::Result<T, ProfileError>;
const CONTRACT: &str = "core.admin-user-access.v1";
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AccessValues {
    pub groups: Vec<String>,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AccessRequest {
    pub expected_auth_version: String,
    pub expected: AccessValues,
    pub groups: Vec<String>,
    pub reason: String,
}
#[derive(Serialize)]
pub struct AccessSnapshot {
    pub contract: &'static str,
    pub user_id: String,
    pub auth_version: String,
    pub active: bool,
    pub account_active: bool,
    pub groups: Vec<String>,
}
#[derive(Serialize)]
pub struct AccessUpdate {
    pub changed: bool,
    pub user: AccessSnapshot,
}
fn valid_groups(groups: &[String]) -> bool {
    groups.len() <= 100
        && groups.iter().all(|g| {
            !g.is_empty()
                && g != "auto"
                && g.len() <= 256
                && g.chars().count() <= 64
                && !g.chars().any(char::is_control)
        })
        && groups.windows(2).all(|p| p[0].as_bytes() < p[1].as_bytes())
}
impl AccessRequest {
    fn validate(&self) -> Result<i64> {
        let version = &self.expected_auth_version;
        if version.is_empty()
            || version.starts_with('0')
            || !version.bytes().all(|c| c.is_ascii_digit())
            || !valid_groups(&self.groups)
            || !valid_groups(&self.expected.groups)
            || self.reason.trim().is_empty()
            || self.reason.len() > 1000
            || self.reason.chars().any(char::is_control)
        {
            return Err(ProfileError::Invalid);
        }
        version
            .parse::<i64>()
            .ok()
            .filter(|v| *v > 0 && *v < i64::MAX)
            .ok_or(ProfileError::Invalid)
    }
}
async fn snapshot(c: &mut PgConnection, id: i64, lock: bool) -> Result<AccessSnapshot> {
    let user = security::target(c, id, lock).await?;
    let groups=sqlx::query_scalar("SELECT group_name FROM core_identity.user_routing_groups WHERE user_id=$1 AND enabled ORDER BY group_name COLLATE \"C\"")
        .bind(id).fetch_all(c).await?;
    Ok(AccessSnapshot {
        contract: CONTRACT,
        user_id: user.user_id,
        auth_version: user.auth_version,
        active: user.active,
        account_active: user.account_active,
        groups,
    })
}
impl AdminUsersStore {
    pub async fn user_access(&self, secret: &str, id: i64) -> Result<AccessSnapshot> {
        let mut tx = self.begin().await?;
        Self::authorize(&mut tx, secret).await?;
        let result = snapshot(&mut tx, id, false).await?;
        tx.commit().await?;
        Ok(result)
    }
    pub async fn update_user_access(
        &self,
        secret: &str,
        id: i64,
        input: &AccessRequest,
    ) -> Result<AccessUpdate> {
        let expected = input.validate()?;
        let mut tx = self.identity.pool.begin().await?;
        for sql in [
            "SET TRANSACTION ISOLATION LEVEL SERIALIZABLE",
            "SET LOCAL statement_timeout='5s'",
            "SET LOCAL lock_timeout='500ms'",
        ] {
            sqlx::query(sql).execute(&mut *tx).await?;
        }
        let actor = identity::authorize_in_transaction(&mut tx, secret).await?;
        if actor.credential_kind != CredentialKind::Session || actor.user_id == id {
            return Err(ProfileError::Forbidden);
        }
        Self::authorize(&mut tx, secret).await?;
        let before = snapshot(&mut tx, id, true).await?;
        if !before.account_active {
            return Err(ProfileError::Forbidden);
        }
        if before.auth_version != input.expected_auth_version
            || before.groups != input.expected.groups
        {
            return Err(ProfileError::Conflict);
        }
        let changed = before.groups != input.groups;
        if changed {
            sqlx::query("UPDATE core_identity.user_routing_groups SET enabled=FALSE WHERE user_id=$1 AND enabled AND NOT (group_name=ANY($2))")
                .bind(id).bind(&input.groups).execute(&mut *tx).await?;
            for group in &input.groups {
                sqlx::query("INSERT INTO core_identity.user_routing_groups AS g(user_id,group_name,enabled,source_reference) VALUES($1,$2,TRUE,$3) ON CONFLICT(user_id,group_name) DO UPDATE SET enabled=TRUE WHERE NOT g.enabled")
                    .bind(id).bind(group).bind(format!("admin-user-access:{}",actor.user_id)).execute(&mut *tx).await?;
            }
            sqlx::query(
                "UPDATE core_identity.users SET auth_version=GREATEST(auth_version,$2) WHERE id=$1",
            )
            .bind(id)
            .bind(expected + 1)
            .execute(&mut *tx)
            .await?;
            identity::audit(
                &mut tx,
                &actor,
                "admin.user.routing_groups.update",
                serde_json::json!({
                    "user_id":id,"previous_auth_version":before.auth_version,"before":before.groups,
                    "after":input.groups,"reason":input.reason.trim()
                }),
            )
            .await?;
        }
        let user = snapshot(&mut tx, id, false).await?;
        tx.commit().await?;
        Ok(AccessUpdate { changed, user })
    }
}
pub(super) fn routes() -> Router<AdminUsersStore> {
    Router::new()
        .route("/core/v1/admin/user-access/{id}", get(read).patch(update))
        .layer(DefaultBodyLimit::max(32 * 1024))
}
async fn read(
    State(store): State<AdminUsersStore>,
    headers: HeaderMap,
    path: std::result::Result<Path<i64>, PathRejection>,
) -> Result<Json<AccessSnapshot>> {
    let Path(id) = path.map_err(|_| ProfileError::Invalid)?;
    Ok(Json(
        store
            .user_access(super::http::bearer(&headers)?, id)
            .await?,
    ))
}
async fn update(
    State(store): State<AdminUsersStore>,
    headers: HeaderMap,
    path: std::result::Result<Path<i64>, PathRejection>,
    input: std::result::Result<Json<AccessRequest>, JsonRejection>,
) -> Result<Json<AccessUpdate>> {
    let Path(id) = path.map_err(|_| ProfileError::Invalid)?;
    let Json(input) = input.map_err(|_| ProfileError::Invalid)?;
    Ok(Json(
        store
            .update_user_access(super::http::bearer(&headers)?, id, &input)
            .await?,
    ))
}

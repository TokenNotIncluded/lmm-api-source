//! Review is conditional on both the request revision and suspension generation.
use super::{AdminUsersStore, ProfileError, security};
use crate::identity::{self, CredentialKind};
use axum::{
    Json, Router,
    extract::{
        DefaultBodyLimit, Path, Query, State,
        rejection::{JsonRejection, QueryRejection},
    },
    http::HeaderMap,
    routing::{get, post},
};
use serde::{Deserialize, Serialize};
use sqlx::{PgConnection, Row};

type Result<T> = std::result::Result<T, ProfileError>;
const COLUMNS: &str = "r.id,r.user_id,r.user_version,r.revision,r.reason,r.admin_note,COALESCE(r.reviewed_by,0) AS admin_user_id,floor(extract(epoch FROM r.created_at))::bigint AS created_at,COALESCE(floor(extract(epoch FROM r.reviewed_at))::bigint,0) AS reviewed_at,COALESCE(p.username,'') AS username,COALESCE(p.email,'') AS email,CASE WHEN r.status='pending' AND (r.user_version<>u.auth_version OR u.active OR NOT a.active OR u.platform_role<>'user' OR EXISTS(SELECT 1 FROM core_account_lifecycle.closures cl WHERE cl.user_id=u.id) OR EXISTS(SELECT 1 FROM core_identity.privileged_import_facts f WHERE f.user_id=u.id)) THEN 'superseded' ELSE r.status END AS status";
const FROM: &str = "FROM core_account_appeals.requests r JOIN core_identity.users u ON u.id=r.user_id JOIN core_identity.accounts a ON a.id=u.personal_account_id LEFT JOIN core_identity.user_profiles p ON p.user_id=u.id";

#[derive(Default, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct AccountAppealsQuery {
    pub status: Option<String>,
    pub page: Option<u32>,
    pub page_size: Option<u32>,
}
#[derive(Serialize)]
pub struct AccountAppealAdmin {
    pub id: i64,
    pub target_user_id: i64,
    pub requested_by_user_id: i64,
    pub kind: &'static str,
    pub status: String,
    pub reason: String,
    pub admin_user_id: i64,
    pub admin_note: String,
    pub created_at: i64,
    pub reviewed_at: i64,
    pub target_username: String,
    pub target_email: String,
    pub requested_by_username: String,
    pub requested_by_email: String,
    pub revision: String,
    pub auth_version: String,
}
impl AccountAppealAdmin {
    fn read(row: sqlx::postgres::PgRow) -> Result<Self> {
        let user = row.try_get("user_id")?;
        let username: String = row.try_get("username")?;
        let email: String = row.try_get("email")?;
        Ok(Self {
            id: row.try_get("id")?,
            target_user_id: user,
            requested_by_user_id: user,
            kind: "appeal",
            status: row.try_get("status")?,
            reason: row.try_get("reason")?,
            admin_user_id: row.try_get("admin_user_id")?,
            admin_note: row.try_get("admin_note")?,
            created_at: row.try_get("created_at")?,
            reviewed_at: row.try_get("reviewed_at")?,
            target_username: username.clone(),
            target_email: email.clone(),
            requested_by_username: username,
            requested_by_email: email,
            revision: row.try_get::<i64, _>("revision")?.to_string(),
            auth_version: row.try_get::<i64, _>("user_version")?.to_string(),
        })
    }
}
#[derive(Serialize)]
pub struct AccountAppealsPagination {
    pub page: u32,
    pub page_size: u32,
    pub total: i64,
}
#[derive(Serialize)]
pub struct AccountAppealsPage {
    pub success: bool,
    pub data: Vec<AccountAppealAdmin>,
    pub pagination: AccountAppealsPagination,
}
#[derive(Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AppealReviewAction {
    Approve,
    Reject,
}
impl AppealReviewAction {
    fn status(self) -> &'static str {
        match self {
            Self::Approve => "approved",
            Self::Reject => "rejected",
        }
    }
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AppealReviewRequest {
    pub expected_revision: String,
    pub expected_auth_version: String,
    pub note: String,
}
fn positive(value: &str) -> Result<i64> {
    let number = value.parse::<i64>().map_err(|_| ProfileError::Invalid)?;
    if number <= 0 || number == i64::MAX || number.to_string() != value {
        return Err(ProfileError::Invalid);
    }
    Ok(number)
}
async fn read(c: &mut PgConnection, id: i64) -> Result<AccountAppealAdmin> {
    let row = sqlx::query(&format!("SELECT {COLUMNS} {FROM} WHERE r.id=$1"))
        .bind(id)
        .fetch_optional(c)
        .await?
        .ok_or(ProfileError::NotFound)?;
    AccountAppealAdmin::read(row)
}
impl AdminUsersStore {
    pub async fn account_appeals(
        &self,
        secret: &str,
        input: &AccountAppealsQuery,
    ) -> Result<AccountAppealsPage> {
        let status = input.status.as_deref().unwrap_or("pending");
        if !matches!(
            status,
            "pending" | "approved" | "rejected" | "superseded" | "all"
        ) {
            return Err(ProfileError::Invalid);
        }
        let page = input.page.unwrap_or(1);
        let size = input.page_size.unwrap_or(20);
        if page == 0 || page > 100_000 || size == 0 || size > 100 {
            return Err(ProfileError::Invalid);
        }
        let mut tx = self.begin().await?;
        Self::authorize(&mut tx, secret).await?;
        let query = format!(
            "WITH requests AS (SELECT {COLUMNS} {FROM}) SELECT * FROM requests WHERE $1='all' OR status=$1 ORDER BY created_at,id LIMIT $2 OFFSET $3"
        );
        let rows = sqlx::query(&query)
            .bind(status)
            .bind(i64::from(size))
            .bind(i64::from(page - 1) * i64::from(size))
            .fetch_all(&mut *tx)
            .await?;
        let total: i64=sqlx::query_scalar(&format!("WITH requests AS (SELECT {COLUMNS} {FROM}) SELECT count(*) FROM requests WHERE $1='all' OR status=$1"))
            .bind(status).fetch_one(&mut *tx).await?;
        let data = rows
            .into_iter()
            .map(AccountAppealAdmin::read)
            .collect::<Result<Vec<_>>>()?;
        tx.commit().await?;
        Ok(AccountAppealsPage {
            success: true,
            data,
            pagination: AccountAppealsPagination {
                page,
                page_size: size,
                total,
            },
        })
    }
    pub async fn review_account_appeal(
        &self,
        secret: &str,
        id: i64,
        action: AppealReviewAction,
        input: &AppealReviewRequest,
    ) -> Result<AccountAppealAdmin> {
        let expected = positive(&input.expected_auth_version)?;
        let revision = positive(&input.expected_revision)?;
        let note = input.note.trim();
        if id <= 0
            || note.chars().count() > 2000
            || note.len() > 8192
            || note.contains('\0')
            || (action == AppealReviewAction::Reject && note.chars().count() < 2)
        {
            return Err(ProfileError::Invalid);
        }
        let mut tx = self.identity.pool.begin().await?;
        sqlx::query("SET LOCAL statement_timeout='5s'")
            .execute(&mut *tx)
            .await?;
        sqlx::query("SET LOCAL lock_timeout='500ms'")
            .execute(&mut *tx)
            .await?;
        let actor = identity::authorize_in_transaction(&mut tx, secret).await?;
        if actor.credential_kind != CredentialKind::Session {
            return Err(ProfileError::Forbidden);
        }
        Self::authorize(&mut tx, secret).await?;
        let user: i64 =
            sqlx::query_scalar("SELECT user_id FROM core_account_appeals.requests WHERE id=$1")
                .bind(id)
                .fetch_optional(&mut *tx)
                .await?
                .ok_or(ProfileError::NotFound)?;
        if user == actor.user_id {
            return Err(ProfileError::Forbidden);
        }
        let before = security::target(&mut tx, user, true).await?;
        let row=sqlx::query("SELECT user_version,revision,status,admin_note,reviewed_by FROM core_account_appeals.requests WHERE id=$1 FOR UPDATE")
            .bind(id).fetch_one(&mut *tx).await?;
        let previous_status: String = row.try_get("status")?;
        if row.try_get::<i64, _>("user_version")? != expected {
            return Err(ProfileError::Conflict);
        }
        if previous_status == action.status()
            && row.try_get::<i64, _>("revision")? == revision + 1
            && row.try_get::<Option<i64>, _>("reviewed_by")? == Some(actor.user_id)
            && row.try_get::<String, _>("admin_note")? == note
        {
            let view = read(&mut tx, id).await?;
            tx.commit().await?;
            return Ok(view);
        }
        if previous_status != "pending"
            || row.try_get::<i64, _>("revision")? != revision
            || before.auth_version != input.expected_auth_version
            || before.active
            || !before.account_active
        {
            return Err(ProfileError::Conflict);
        }
        let closed: bool = sqlx::query_scalar(
            "SELECT EXISTS(SELECT 1 FROM core_account_lifecycle.closures WHERE user_id=$1)",
        )
        .bind(user)
        .fetch_one(&mut *tx)
        .await?;
        if closed {
            return Err(ProfileError::Forbidden);
        }
        if action == AppealReviewAction::Approve {
            sqlx::query("UPDATE core_identity.users SET active=TRUE,auth_version=$2 WHERE id=$1")
                .bind(user)
                .bind(expected + 1)
                .execute(&mut *tx)
                .await?;
            sqlx::query("UPDATE core_identity.credentials SET revoked_at=clock_timestamp() WHERE user_id=$1 AND revoked_at IS NULL").bind(user).execute(&mut *tx).await?;
            sqlx::query("UPDATE core_identity.web_sessions SET revoked_at=clock_timestamp() WHERE user_id=$1 AND revoked_at IS NULL").bind(user).execute(&mut *tx).await?;
        }
        sqlx::query("UPDATE core_account_appeals.requests SET status=$2,admin_note=$3,reviewed_by=$4,reviewed_at=clock_timestamp(),revision=revision+1 WHERE id=$1")
            .bind(id).bind(action.status()).bind(note).bind(actor.user_id).execute(&mut *tx).await?;
        identity::audit(&mut tx,&actor,match action { AppealReviewAction::Approve=>"admin.user.appeal.approve",AppealReviewAction::Reject=>"admin.user.appeal.reject" },serde_json::json!({"request_id":id,"user_id":user,"previous_auth_version":input.expected_auth_version,"auth_version":(if action==AppealReviewAction::Approve {expected+1} else {expected}).to_string(),"reason":note})).await?;
        let view = read(&mut tx, id).await?;
        tx.commit().await?;
        Ok(view)
    }
}
pub(super) fn routes() -> Router<AdminUsersStore> {
    Router::new()
        .route("/api/account-action-requests", get(list))
        .route("/api/account-action-requests/{id}/{action}", post(review))
        .layer(DefaultBodyLimit::max(16384))
}
async fn list(
    State(store): State<AdminUsersStore>,
    headers: HeaderMap,
    input: std::result::Result<Query<AccountAppealsQuery>, QueryRejection>,
) -> Result<Json<AccountAppealsPage>> {
    let Query(input) = input.map_err(|_| ProfileError::Invalid)?;
    Ok(Json(
        store
            .account_appeals(security::bearer(&headers)?, &input)
            .await?,
    ))
}
async fn review(
    State(store): State<AdminUsersStore>,
    headers: HeaderMap,
    Path((id, action)): Path<(i64, AppealReviewAction)>,
    input: std::result::Result<Json<AppealReviewRequest>, JsonRejection>,
) -> Result<Json<serde_json::Value>> {
    let Json(input) = input.map_err(|_| ProfileError::Invalid)?;
    let view = store
        .review_account_appeal(security::bearer(&headers)?, id, action, &input)
        .await?;
    Ok(Json(serde_json::json!({"success":true,"data":view})))
}

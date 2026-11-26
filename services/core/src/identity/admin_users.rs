//! Core-owned administrator user reads and a separate conditional display-name update.
mod access_groups;
pub use access_groups::{AccessRequest, AccessSnapshot, AccessUpdate, AccessValues};
mod appeals;
mod create;
pub use create::{CreateUserRequest, CreatedUser};
mod balance_adjustments;
pub use appeals::{
    AccountAppealAdmin, AccountAppealsPage, AccountAppealsQuery, AppealReviewAction,
    AppealReviewRequest,
};
mod http;
mod identity_origin;
mod profile_mutation;
mod security;
pub use balance_adjustments::{AdjustmentReceipt, AdjustmentRequest};
pub use profile_mutation::{
    AdminProfile, PROFILE_CONTRACT, ProfileCapabilities, ProfileError, ProfileUpdate,
    ProfileUpdateRequest,
};
pub use security::{SecurityAction, SecurityRequest, SecuritySnapshot, SecurityUpdate};
mod statistics;
mod usage;
mod wallet;
use super::*;
pub use http::router;
use serde::{Deserialize, Serialize};
use sqlx::{Postgres, Row, Transaction};
pub use statistics::UserStatistics;
pub use usage::UsageProjection;
pub use wallet::WalletProjection;

pub const PROTOCOL: &str = "core.admin-users.v1";
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum AdminUsersError {
    Unauthorized,
    Forbidden,
    Invalid,
    NotFound,
    UnsupportedFilter,
    UnsupportedSort,
    Unavailable,
}
type ReadResult<T> = std::result::Result<T, AdminUsersError>;
impl From<sqlx::Error> for AdminUsersError {
    fn from(_: sqlx::Error) -> Self {
        Self::Unavailable
    }
}
impl From<IdentityError> for AdminUsersError {
    fn from(value: IdentityError) -> Self {
        match value {
            IdentityError::Unauthorized => Self::Unauthorized,
            IdentityError::Forbidden => Self::Forbidden,
            IdentityError::Invalid => Self::Invalid,
            IdentityError::Storage | IdentityError::Conflict => Self::Unavailable,
        }
    }
}
impl std::fmt::Display for AdminUsersError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{self:?}")
    }
}
impl std::error::Error for AdminUsersError {}

#[derive(Clone, Default, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct UsersQuery {
    pub keyword: String,
    pub group: Option<String>,
    pub role: Option<String>,
    pub status: Option<String>,
    pub team_id: Option<String>,
    pub page: Option<u32>,
    pub page_size: Option<u32>,
    pub sort_by: Option<String>,
    pub sort_order: Option<String>,
    pub risk_min: Option<String>,
    pub risk_max: Option<String>,
    pub transfers: Option<String>,
    pub usage: Option<String>,
    pub funding: Option<String>,
    pub checkin: Option<String>,
}
#[derive(Clone, Default, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct GroupsQuery {
    pub page: Option<u32>,
    pub page_size: Option<u32>,
}
struct ValidQuery {
    keyword: String,
    keyword_id: Option<i64>,
    group: Option<String>,
    role: Option<String>,
    active: Option<bool>,
    team_id: Option<i64>,
    page: u32,
    size: u32,
    offset: i64,
    order: String,
}
fn pagination(page: Option<u32>, size: Option<u32>) -> ReadResult<(u32, u32, i64)> {
    let page = page.unwrap_or(1);
    let size = size.unwrap_or(20);
    if page == 0 || page > 100_000 || size == 0 || size > 100 {
        return Err(AdminUsersError::Invalid);
    }
    Ok((page, size, i64::from(page - 1) * i64::from(size)))
}
impl UsersQuery {
    fn validate(&self) -> ReadResult<ValidQuery> {
        if self.keyword.len() > 256 || self.keyword.chars().any(char::is_control) {
            return Err(AdminUsersError::Invalid);
        }
        for value in [
            &self.risk_min,
            &self.risk_max,
            &self.transfers,
            &self.usage,
            &self.funding,
            &self.checkin,
        ] {
            if value.as_ref().is_some_and(|v| !v.is_empty()) {
                return Err(AdminUsersError::UnsupportedFilter);
            }
        }
        let group = self.group.as_ref().filter(|s| !s.is_empty()).cloned();
        if group.as_ref().is_some_and(|v| {
            v.chars().count() > 64 || v.len() > 256 || v.chars().any(char::is_control)
        }) {
            return Err(AdminUsersError::Invalid);
        }
        let role = self.role.as_ref().filter(|s| !s.is_empty()).cloned();
        if role
            .as_ref()
            .is_some_and(|v| !matches!(v.as_str(), "user" | "admin" | "superadmin"))
        {
            return Err(AdminUsersError::Invalid);
        }
        let active = match self.status.as_deref().filter(|s| !s.is_empty()) {
            None => None,
            Some("active") => Some(true),
            Some("disabled") => Some(false),
            Some("deleted" | "-1") => return Err(AdminUsersError::UnsupportedFilter),
            _ => return Err(AdminUsersError::Invalid),
        };
        let team_id = self
            .team_id
            .as_deref()
            .map(|value| {
                if value.is_empty() || !value.bytes().all(|c| c.is_ascii_digit()) {
                    return Err(AdminUsersError::Invalid);
                }
                value
                    .parse::<i64>()
                    .ok()
                    .filter(|id| *id > 0)
                    .ok_or(AdminUsersError::Invalid)
            })
            .transpose()?;
        let (page, size, offset) = pagination(self.page, self.page_size)?;
        let sort = match self.sort_by.as_deref().unwrap_or("id") {
            "id" => "id",
            "username" => "username COLLATE \"C\"",
            "display_name" => "display_name COLLATE \"C\"",
            "core_created_at" => "core_created_at",
            "registered_at" => "registered_at",
            "groups" => "groups COLLATE \"C\"",
            "role" => "role_number",
            "status" => "status_number",
            "quota"
            | "used_quota"
            | "request_count"
            | "last_login_at"
            | "group"
            | "created_at"
            | "topup_quota"
            | "topup_money"
            | "assistant_violations"
            | "risk_score"
            | "transferred_quota"
            | "received_quota"
            | "checkin_quota" => return Err(AdminUsersError::UnsupportedSort),
            _ => return Err(AdminUsersError::Invalid),
        };
        let direction = match self.sort_order.as_deref().unwrap_or("desc") {
            "asc" => "ASC",
            "desc" => "DESC",
            _ => return Err(AdminUsersError::Invalid),
        };
        // Only server-selected literals reach ORDER BY. Every filter stays bound.
        let order = format!("{sort} {direction} NULLS LAST,id {direction}");
        let keyword = self.keyword.trim().to_owned();
        let keyword_id = keyword.parse::<i64>().ok().filter(|id| *id > 0);
        Ok(ValidQuery {
            keyword,
            keyword_id,
            group,
            role,
            active,
            team_id,
            page,
            size,
            offset,
            order,
        })
    }
}
#[derive(Serialize)]
pub struct AdminUserTeam {
    pub id: String,
    pub account_id: String,
    pub name: String,
    pub role: String,
    pub can_spend: bool,
}
#[derive(Serialize)]
pub struct UserRow {
    pub id: String,
    pub personal_account_id: String,
    pub auth_version: String,
    pub profile_available: bool,
    pub username: Option<String>,
    pub display_name: Option<String>,
    pub email: Option<String>,
    pub platform_role: String,
    pub role: i16,
    pub active: bool,
    pub status: i16,
    pub account_active: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub identity_origin: Option<&'static str>,
    pub groups: Vec<String>,
    pub teams: Vec<AdminUserTeam>,
    pub core_account_created_at_unix_ms: i64,
    pub registered_at_unix_ms: Option<i64>,
    pub wallet: WalletProjection,
}
#[derive(Serialize)]
pub struct ReadCapabilities {
    pub filters: &'static [&'static str],
    pub sorts: &'static [&'static str],
    pub implemented_write_actions: &'static [&'static str],
    pub unavailable_projections: &'static [&'static str],
    pub unavailable_write_actions: &'static [&'static str],
}
const JS_SAFE_INTEGER: i64 = 9_007_199_254_740_991;
fn safe_total(value: i64) -> ReadResult<u64> {
    if !(0..=JS_SAFE_INTEGER).contains(&value) {
        return Err(AdminUsersError::Unavailable);
    }
    Ok(value as u64)
}
fn safe_time(value: i64) -> ReadResult<i64> {
    if !(-JS_SAFE_INTEGER..=JS_SAFE_INTEGER).contains(&value) {
        return Err(AdminUsersError::Unavailable);
    }
    Ok(value)
}
fn capabilities() -> ReadCapabilities {
    ReadCapabilities {
        filters: &["keyword", "group", "role", "status", "team_id"],
        sorts: &[
            "id",
            "username",
            "display_name",
            "core_created_at",
            "registered_at",
            "groups",
            "role",
            "status",
        ],
        implemented_write_actions: &["create"],
        unavailable_projections: &[
            "legacy_group",
            "used_quota",
            "normalized_used_quota",
            "request_count",
            "wallet_risk",
            "transfer_history",
            "checkin_history",
            "topup_summary",
            "referral_rewards",
            "referral_counts",
            "referral_history",
            "deleted_at",
            "last_login_at",
            "oauth_bindings",
            "assistant_profile",
            "assistant_conversations",
            "assistant_violations",
            "permission_matrix",
            "user_detail",
        ],
        unavailable_write_actions: &[
            "update",
            "delete",
            "enable",
            "promote",
            "demote",
            "adjust_money",
            "reset_onboarding",
            "normal_ban",
            "ban_abuse",
            "restore_referral",
            "inviter_penalty",
            "revoke_rewards",
            "unbind",
            "reset_passkey",
            "reset_2fa",
            "assistant_policy",
            "reset_assistant_violations",
            "subscription_mutations",
            "account_action_requests",
        ],
    }
}
#[derive(Serialize)]
pub struct UsersPage {
    pub version: u8,
    pub items: Vec<UserRow>,
    pub total: u64,
    pub page: u32,
    pub page_size: u32,
    pub capabilities: ReadCapabilities,
}
#[derive(Serialize)]
pub struct UserDetail {
    pub version: u8,
    pub user: UserRow,
    pub usage: UsageProjection,
    pub statistics: UserStatistics,
}
#[derive(Serialize)]
pub struct GroupsPage {
    pub version: u8,
    pub items: Vec<String>,
    pub total: u64,
    pub page: u32,
    pub page_size: u32,
    pub source: &'static str,
}
#[derive(Clone)]
pub struct AdminUsersStore {
    identity: IdentityStore,
}
impl AdminUsersStore {
    pub fn new(identity: IdentityStore) -> Self {
        Self { identity }
    }
    pub(super) async fn begin(&self) -> ReadResult<Transaction<'_, Postgres>> {
        let mut tx = self.identity.pool.begin().await?;
        sqlx::query("SET TRANSACTION ISOLATION LEVEL REPEATABLE READ READ ONLY")
            .execute(&mut *tx)
            .await?;
        sqlx::query("SET LOCAL statement_timeout='5s'")
            .execute(&mut *tx)
            .await?;
        sqlx::query("SET LOCAL lock_timeout='500ms'")
            .execute(&mut *tx)
            .await?;
        Ok(tx)
    }
    pub(super) async fn authorize(c: &mut PgConnection, secret: &str) -> ReadResult<Principal> {
        let actor = super::authenticate(c, secret, false).await?;
        Self::admit_current(c, actor).await
    }
    /// The same admission in a caller-owned write transaction, retaining native
    /// credential/user/account and MFA locks in the existing authentication order.
    pub(super) async fn authorize_locked(
        c: &mut PgConnection,
        secret: &str,
    ) -> ReadResult<Principal> {
        let actor = super::authorize_in_transaction(c, secret).await?;
        Self::admit_current(c, actor).await
    }
    async fn admit_current(c: &mut PgConnection, actor: Principal) -> ReadResult<Principal> {
        if actor.credential_kind != CredentialKind::Session
            || actor.owner.kind != AccountKind::Personal
            || actor.owner.id != actor.user_id
        {
            return Err(AdminUsersError::Forbidden);
        }
        let row = sqlx::query(
            "SELECT u.platform_role,u.personal_account_id,u.auth_version,u.active,a.active AS account_active
             FROM core_identity.users u
             JOIN core_identity.accounts a ON a.id=u.personal_account_id
             WHERE u.id=$1",
        )
        .bind(actor.user_id)
        .fetch_one(&mut *c)
        .await?;
        let role: String = row.try_get("platform_role")?;
        if !matches!(role.as_str(), "admin" | "superadmin") {
            return Err(AdminUsersError::Forbidden);
        }
        // Synthetic development bootstrap admins have no privileged_import_facts
        // and keep the role-only gate. Imported admins must load a complete
        // persisted policy; hash-only claims fail closed.
        let imported = super::permissions::has_privileged_import_facts(c, actor.user_id)
            .await
            .map_err(|_| AdminUsersError::Unavailable)?;
        if imported {
            let loaded = super::permissions::load_complete_policy(c, actor.user_id)
                .await
                .map_err(|error| match error {
                    super::permissions::PolicyLoadError::Storage => AdminUsersError::Unavailable,
                    super::permissions::PolicyLoadError::Missing
                    | super::permissions::PolicyLoadError::Incomplete
                    | super::permissions::PolicyLoadError::HashMismatch => {
                        AdminUsersError::Forbidden
                    }
                })?;
            let platform_role = match role.as_str() {
                "admin" => super::permissions::PlatformRole::Admin,
                "superadmin" => super::permissions::PlatformRole::Root,
                _ => return Err(AdminUsersError::Forbidden),
            };
            let personal_account_id: i64 = row.try_get("personal_account_id")?;
            let auth_version: i64 = row.try_get("auth_version")?;
            let active: bool = row.try_get("active")?;
            let account_active: bool = row.try_get("account_active")?;
            let decision = super::permissions::decide(
                &super::permissions::Actor {
                    user_id: actor.user_id,
                    personal_account_id,
                    role: platform_role,
                    credential: super::permissions::Credential::Session,
                    active,
                    account_active,
                    auth_version,
                    credential_auth_version: actor.user_version,
                },
                &loaded.as_policy(),
                super::permissions::Action::AdminUsersRead,
                super::permissions::Scope::Platform(
                    super::permissions::PlatformResource::Directory,
                ),
                None,
            );
            if !decision.allowed() {
                return Err(AdminUsersError::Forbidden);
            }
        }
        Ok(actor)
    }
    pub async fn users(&self, secret: &str, input: &UsersQuery) -> ReadResult<UsersPage> {
        self.read_users(secret, input, false).await
    }

    async fn read_users(
        &self,
        secret: &str,
        input: &UsersQuery,
        include_origin: bool,
    ) -> ReadResult<UsersPage> {
        let q = input.validate()?;
        let mut tx = self.begin().await?;
        Self::authorize(&mut tx, secret).await?;
        let total: i64 = sqlx::query_scalar(sqlx::AssertSqlSafe(format!(
            "{SELECTED} SELECT count(*) FROM selected"
        )))
        .bind(&q.keyword)
        .bind(q.keyword_id)
        .bind(&q.group)
        .bind(&q.role)
        .bind(q.active)
        .bind(q.team_id)
        .fetch_one(&mut *tx)
        .await?;
        let rows=sqlx::query(sqlx::AssertSqlSafe(format!("{SELECTED} SELECT *,floor(extract(epoch FROM core_created_at)*1000)::bigint AS core_created_ms,floor(extract(epoch FROM registered_at)*1000)::bigint AS registered_ms FROM selected ORDER BY {} LIMIT $7 OFFSET $8",q.order)))
            .bind(&q.keyword).bind(q.keyword_id).bind(&q.group).bind(&q.role).bind(q.active).bind(q.team_id).bind(i64::from(q.size)).bind(q.offset).fetch_all(&mut *tx).await?;
        let mut items = Vec::with_capacity(rows.len());
        for r in rows {
            items.push(project_user(&mut tx, r).await?);
        }
        if include_origin {
            identity_origin::annotate(&mut tx, &mut items).await?;
        }
        let result = UsersPage {
            version: 1,
            items,
            total: safe_total(total)?,
            page: q.page,
            page_size: q.size,
            capabilities: capabilities(),
        };
        tx.commit().await?;
        Ok(result)
    }

    pub async fn user(&self, secret: &str, user_id: i64) -> ReadResult<UserDetail> {
        self.read_user(secret, user_id, false).await
    }

    async fn read_user(
        &self,
        secret: &str,
        user_id: i64,
        include_origin: bool,
    ) -> ReadResult<UserDetail> {
        if user_id <= 0 {
            return Err(AdminUsersError::Invalid);
        }
        let mut tx = self.begin().await?;
        Self::authorize(&mut tx, secret).await?;
        // Use the list's projection, but select the exact ID independently of text search.
        let row = sqlx::query(sqlx::AssertSqlSafe(format!(
            "{SELECTED} SELECT *,floor(extract(epoch FROM core_created_at)*1000)::bigint AS core_created_ms,floor(extract(epoch FROM registered_at)*1000)::bigint AS registered_ms FROM selected WHERE id=$7"
        )))
        .bind("")
        .bind(None::<i64>)
        .bind(None::<String>)
        .bind(None::<String>)
        .bind(None::<bool>)
        .bind(None::<i64>)
        .bind(user_id)
        .fetch_optional(&mut *tx)
        .await?
        .ok_or(AdminUsersError::NotFound)?;
        let account: i64 = row.try_get("personal_account_id")?;
        let mut user = project_user(&mut tx, row).await?;
        if include_origin {
            identity_origin::annotate(&mut tx, std::slice::from_mut(&mut user)).await?;
        }
        let usage = usage::read(&mut tx, user_id, account).await?;
        let statistics = statistics::read(
            &mut tx,
            user_id,
            account,
            matches!(&user.wallet, WalletProjection::Available { .. }),
        )
        .await?;
        tx.commit().await?;
        Ok(UserDetail {
            version: 1,
            user,
            usage,
            statistics,
        })
    }
    pub async fn groups(&self, secret: &str, input: &GroupsQuery) -> ReadResult<GroupsPage> {
        let (page, size, offset) = pagination(input.page, input.page_size)?;
        let mut tx = self.begin().await?;
        Self::authorize(&mut tx, secret).await?;
        let total:i64=sqlx::query_scalar("SELECT count(DISTINCT group_name COLLATE \"C\") FROM core_identity.user_routing_groups WHERE enabled").fetch_one(&mut *tx).await?;
        let items=sqlx::query_scalar("SELECT DISTINCT group_name COLLATE \"C\" AS name FROM core_identity.user_routing_groups WHERE enabled ORDER BY name ASC LIMIT $1 OFFSET $2").bind(i64::from(size)).bind(offset).fetch_all(&mut *tx).await?;
        tx.commit().await?;
        Ok(GroupsPage {
            version: 1,
            items,
            total: safe_total(total)?,
            page,
            page_size: size,
            source: "enabled_explicit_user_routing_grants",
        })
    }
}
async fn project_user(c: &mut PgConnection, r: sqlx::postgres::PgRow) -> ReadResult<UserRow> {
    let id: i64 = r.try_get("id")?;
    let account: i64 = r.try_get("personal_account_id")?;
    let generation: i64 = r.try_get("auth_version")?;
    if id <= 0 || account <= 0 || generation <= 0 {
        return Err(AdminUsersError::Unavailable);
    }
    Ok(UserRow {
        id: id.to_string(),
        personal_account_id: account.to_string(),
        auth_version: generation.to_string(),
        profile_available: r.try_get("profile_available")?,
        username: r.try_get("username")?,
        display_name: r.try_get("display_name")?,
        email: r.try_get("email")?,
        platform_role: r.try_get("platform_role")?,
        role: r.try_get("role_number")?,
        active: r.try_get("active")?,
        status: r.try_get("status_number")?,
        account_active: r.try_get("account_active")?,
        identity_origin: None,
        groups: r.try_get("groups")?,
        teams: user_teams(c, id).await?,
        core_account_created_at_unix_ms: safe_time(r.try_get("core_created_ms")?)?,
        registered_at_unix_ms: r
            .try_get::<Option<i64>, _>("registered_ms")?
            .map(safe_time)
            .transpose()?,
        wallet: wallet::read(c, id, account).await?,
    })
}
async fn user_teams(c: &mut PgConnection, user_id: i64) -> ReadResult<Vec<AdminUserTeam>> {
    let rows = sqlx::query(
        "SELECT t.id,t.account_id,t.name,
          CASE WHEN t.owner_user_id=$1 THEN 'owner' ELSE m.role END AS role,
          (t.owner_user_id=$1 OR COALESCE(m.can_spend,FALSE)) AS can_spend
         FROM core_identity.teams t JOIN core_identity.accounts a ON a.id=t.account_id
         LEFT JOIN core_identity.memberships m ON m.team_id=t.id AND m.user_id=$1 AND m.active
         WHERE t.active AND a.active AND (t.owner_user_id=$1 OR m.user_id IS NOT NULL) ORDER BY t.id",
    ).bind(user_id).fetch_all(c).await?;
    rows.into_iter()
        .map(|row| {
            Ok(AdminUserTeam {
                id: row.try_get::<i64, _>("id")?.to_string(),
                account_id: row.try_get::<i64, _>("account_id")?.to_string(),
                name: row.try_get("name")?,
                role: row.try_get("role")?,
                can_spend: row.try_get("can_spend")?,
            })
        })
        .collect()
}
const SELECTED: &str = r#"
WITH selected AS (
 SELECT u.id,u.personal_account_id,u.auth_version,u.platform_role,u.active,
  a.active AS account_active,a.created_at AS core_created_at,
  p.user_id IS NOT NULL AS profile_available,p.username,p.display_name,p.email,
  o.registered_at,
  CASE u.platform_role WHEN 'user' THEN 1::smallint WHEN 'admin' THEN 10::smallint WHEN 'superadmin' THEN 100::smallint END AS role_number,
  CASE WHEN u.active THEN 1::smallint ELSE 2::smallint END AS status_number,
  ARRAY(SELECT g.group_name FROM core_identity.user_routing_groups g WHERE g.user_id=u.id AND g.enabled ORDER BY g.group_name COLLATE "C") AS groups
 FROM core_identity.users u JOIN core_identity.accounts a ON a.id=u.personal_account_id AND a.kind='personal'
 LEFT JOIN core_identity.user_profiles p ON p.user_id=u.id
 LEFT JOIN core_abuse.registration_observations o ON o.user_id=u.id
 WHERE ($1::text='' OR u.id=$2::bigint OR position(lower($1) in lower(COALESCE(p.username,'')))>0
   OR position(lower($1) in lower(COALESCE(p.display_name,'')))>0 OR position(lower($1) in lower(COALESCE(p.email,'')))>0)
  AND ($3::text IS NULL OR EXISTS(SELECT FROM core_identity.user_routing_groups g WHERE g.user_id=u.id AND g.enabled AND g.group_name COLLATE "C"=$3::text COLLATE "C"))
  AND ($4::text IS NULL OR u.platform_role=$4) AND ($5::boolean IS NULL OR u.active=$5)
  AND ($6::bigint IS NULL OR EXISTS(
    SELECT FROM core_identity.teams t JOIN core_identity.accounts ta ON ta.id=t.account_id
    LEFT JOIN core_identity.memberships m ON m.team_id=t.id AND m.user_id=u.id AND m.active
    WHERE t.id=$6 AND t.active AND ta.active AND (t.owner_user_id=u.id OR m.user_id IS NOT NULL)))
)
"#;
#[cfg(all(test, unix))]
mod tests;

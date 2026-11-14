//! Administrator-created ordinary users; no login session or authority grants.
use super::{AdminUsersStore, ProfileError};
use crate::identity::{self, CredentialKind, IdentityError};
use axum::{
    Json,
    extract::{State, rejection::JsonRejection},
    http::{HeaderMap, StatusCode},
};
use serde::{Deserialize, Serialize};

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CreateUserRequest {
    pub username: String,
    pub password: String,
    #[serde(default)]
    pub display_name: String,
}
#[derive(Serialize)]
pub struct CreatedUser {
    pub version: u8,
    pub user_id: String,
    pub platform_role: &'static str,
}
impl CreateUserRequest {
    fn validate(&self) -> Result<(), ProfileError> {
        if self.username.trim() != self.username
            || self.username.is_empty()
            || self.username.chars().count() > 20
            || self.username.len() > 128
            || self.username.chars().any(char::is_control)
            || !(8..=20).contains(&self.password.chars().count())
            || self.display_name.chars().count() > 20
            || self.display_name.chars().any(char::is_control)
        {
            return Err(ProfileError::Invalid);
        }
        Ok(())
    }
}
impl AdminUsersStore {
    pub async fn create_user(
        &self,
        secret: &str,
        input: &CreateUserRequest,
    ) -> Result<CreatedUser, ProfileError> {
        input.validate()?;
        let mut check = self.begin().await?;
        Self::authorize(&mut check, secret).await?;
        check.commit().await?;
        let hash = self.identity.hash_password(input.password.clone()).await?;
        let mut tx = self.identity.pool.begin().await?;
        sqlx::query("SET LOCAL lock_timeout='500ms'")
            .execute(&mut *tx)
            .await?;
        identity::auth::lock_registration(&mut tx).await?;
        let actor = identity::authorize_in_transaction(&mut tx, secret).await?;
        if actor.credential_kind != CredentialKind::Session {
            return Err(ProfileError::Forbidden);
        }
        Self::authorize(&mut tx, secret).await?;
        let conflict: bool = sqlx::query_scalar("SELECT EXISTS(SELECT FROM core_identity.user_profiles WHERE username=$1 OR (email_login_allowed AND email<>'' AND email=$1))")
            .bind(&input.username).fetch_one(&mut *tx).await?;
        if conflict {
            return Err(ProfileError::Conflict);
        }
        let user = identity::auth::create_user(&mut tx).await?;
        let login = format!("web_{}", &identity::new_secret("")?[..32]);
        sqlx::query("INSERT INTO core_identity.password_logins(login_name,user_id,password_hash) VALUES ($1,$2,$3)")
            .bind(login).bind(user).bind(hash).execute(&mut *tx).await.map_err(IdentityError::from)?;
        let name = if input.display_name.is_empty() {
            &input.username
        } else {
            &input.display_name
        };
        sqlx::query("INSERT INTO core_identity.user_profiles(user_id,username,display_name) VALUES ($1,$2,$3)")
            .bind(user).bind(&input.username).bind(name).execute(&mut *tx).await.map_err(IdentityError::from)?;
        identity::audit(
            &mut tx,
            &actor,
            "admin.user.create",
            serde_json::json!({"user_id":user,"platform_role":"user"}),
        )
        .await?;
        sqlx::query("INSERT INTO core_identity.audit(actor_user_id,action,target) VALUES ($1,'user.register',jsonb_build_object('user_id',$1::bigint,'created_by_admin',$2::bigint))")
            .bind(user).bind(actor.user_id).execute(&mut *tx).await?;
        tx.commit().await?;
        Ok(CreatedUser {
            version: 1,
            user_id: user.to_string(),
            platform_role: "user",
        })
    }
}
pub(super) async fn create(
    State(store): State<AdminUsersStore>,
    headers: HeaderMap,
    input: Result<Json<CreateUserRequest>, JsonRejection>,
) -> Result<(StatusCode, Json<CreatedUser>), ProfileError> {
    let secret = super::http::bearer(&headers).map_err(ProfileError::from)?;
    let Json(input) = input.map_err(|_| ProfileError::Invalid)?;
    Ok((
        StatusCode::CREATED,
        Json(store.create_user(secret, &input).await?),
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::identity::{BrowserAuth, BrowserSessionMetadata, IdentityStore};
    use axum::{
        body::{Body, to_bytes},
        http::Request,
    };
    use sqlx::PgPool;
    use tower::ServiceExt;

    #[sqlx::test]
    async fn admin_create_user_is_atomic_and_session_only(pool: PgPool) {
        crate::core_schema::init(&pool).await.unwrap();
        let identity = IdentityStore::from_pool(pool.clone());
        let admin = identity.bootstrap_user(720, 5).await.unwrap();
        let ordinary = identity.bootstrap_user(721, 0).await.unwrap();
        let role = crate::test_runtime::RuntimeRole::create(&pool).await;
        let runtime = IdentityStore::from_pool(role.pool.clone());
        let store = AdminUsersStore::new(runtime.clone());
        let input = CreateUserRequest {
            username: "admin-created".into(),
            password: "password12".into(),
            display_name: "Created".into(),
        };
        assert!(matches!(
            store.create_user(&ordinary.secret, &input).await,
            Err(ProfileError::Forbidden)
        ));
        let key = identity
            .issue_key(
                &admin.secret,
                crate::accounts::Account {
                    kind: crate::accounts::AccountKind::Personal,
                    id: 720,
                },
                crate::funding::FundingPolicy::new_personal_key(),
                3600,
            )
            .await
            .unwrap();
        assert!(matches!(
            store.create_user(&key.secret, &input).await,
            Err(ProfileError::Forbidden)
        ));
        let router = super::super::router(store.clone());
        let response = router.oneshot(Request::post("/core/v1/admin/users").header("authorization", format!("Bearer {}", admin.secret)).header("content-type", "application/json").body(Body::from(serde_json::json!({"username":input.username,"password":input.password,"display_name":input.display_name}).to_string())).unwrap()).await.unwrap();
        assert_eq!(response.status(), StatusCode::CREATED);
        let value: serde_json::Value =
            serde_json::from_slice(&to_bytes(response.into_body(), 4096).await.unwrap()).unwrap();
        assert_eq!(value["platform_role"], "user");
        assert!(value.get("secret").is_none());
        let user: i64 = value["user_id"].as_str().unwrap().parse().unwrap();
        let counts: (i64, i64) = sqlx::query_as("SELECT (SELECT count(*) FROM core_identity.credentials WHERE user_id=$1),(SELECT count(*) FROM core_identity.developer_access WHERE user_id=$1 AND granted)").bind(user).fetch_one(&pool).await.unwrap();
        assert_eq!(counts, (0, 0));
        let users_before: i64 = sqlx::query_scalar("SELECT count(*) FROM core_identity.users")
            .fetch_one(&pool)
            .await
            .unwrap();
        assert!(matches!(
            store.create_user(&admin.secret, &input).await,
            Err(ProfileError::Conflict)
        ));
        let users_after: i64 = sqlx::query_scalar("SELECT count(*) FROM core_identity.users")
            .fetch_one(&pool)
            .await
            .unwrap();
        assert_eq!(users_before, users_after);
        let browser = BrowserAuth::new(runtime, &[61; 32]).unwrap();
        assert!(
            browser
                .login(
                    &input.username,
                    &input.password,
                    &BrowserSessionMetadata {
                        ip: "127.0.0.1".into(),
                        user_agent: "admin-create-self-test".into()
                    }
                )
                .await
                .is_ok()
        );
        identity.revoke(&admin.secret, admin.id).await.unwrap();
        assert!(matches!(
            store.create_user(&admin.secret, &input).await,
            Err(ProfileError::Unauthorized)
        ));
        role.cleanup().await;
    }
    #[test]
    fn creation_has_no_role_or_grant_input_and_counts_password_characters() {
        assert!(
            serde_json::from_value::<CreateUserRequest>(
                serde_json::json!({"username":"alice","password":"password12","role":"admin"})
            )
            .is_err()
        );
        let mut input = CreateUserRequest {
            username: "alice".into(),
            password: "密".repeat(8),
            display_name: String::new(),
        };
        assert!(input.validate().is_ok());
        input.password = "密".repeat(21);
        assert_eq!(input.validate(), Err(ProfileError::Invalid));
    }
}

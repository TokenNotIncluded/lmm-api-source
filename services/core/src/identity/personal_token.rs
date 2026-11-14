//! Personal profile-read tokens are deliberately outside native credentials.
//! A token can never become a Session, API key, or administrator principal.
use super::*;
use axum::{
    Json, Router,
    extract::{State, rejection::JsonRejection},
    http::{HeaderMap, header},
    response::IntoResponse,
    routing::{get, post},
};
use serde::{Deserialize, Serialize};

/// Register as a fresh-install component and provide its reviewed predecessor
/// upgrade and runtime grants before mounting this module in production.
pub const SCHEMA: &str = include_str!("personal_token/schema.sql");
pub const SCOPE: &str = "own_profile_read";

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RotateRequest {
    pub expected_generation: String,
}
impl RotateRequest {
    fn generation(&self) -> Result<i64> {
        let value = self
            .expected_generation
            .parse::<i64>()
            .map_err(|_| IdentityError::Invalid)?;
        if value < 0 || value == i64::MAX || value.to_string() != self.expected_generation {
            return Err(IdentityError::Invalid);
        }
        Ok(value)
    }
}
#[derive(Serialize)]
pub struct RotatedToken {
    pub token: String,
    pub generation: String,
    pub expires_at: i64,
    pub scope: &'static str,
}
#[derive(Serialize)]
pub struct TokenStatus {
    pub generation: String,
    pub scope: &'static str,
}
#[derive(Serialize)]
pub struct PersonalProfile {
    pub user_id: String,
    pub username: String,
    pub display_name: String,
}
async fn personal_session(c: &mut PgConnection, secret: &str) -> Result<Principal> {
    let actor = session(c, secret).await?;
    if actor.owner.kind != AccountKind::Personal || actor.owner.id != actor.user_id {
        return Err(IdentityError::Forbidden);
    }
    Ok(actor)
}
impl IdentityStore {
    pub async fn personal_token_status(&self, secret: &str) -> Result<TokenStatus> {
        let mut tx = self.pool.begin().await?;
        let actor = personal_session(&mut tx, secret).await?;
        let generation: Option<i64> = sqlx::query_scalar(
            "SELECT generation FROM core_personal_tokens.tokens WHERE user_id=$1",
        )
        .bind(actor.user_id)
        .fetch_optional(&mut *tx)
        .await?;
        tx.commit().await?;
        Ok(TokenStatus {
            generation: generation.unwrap_or(0).to_string(),
            scope: SCOPE,
        })
    }
    pub async fn rotate_personal_token(
        &self,
        secret: &str,
        input: &RotateRequest,
    ) -> Result<RotatedToken> {
        let expected = input.generation()?;
        let mut tx = self.pool.begin().await?;
        sqlx::query("SET LOCAL lock_timeout='500ms'")
            .execute(&mut *tx)
            .await?;
        let actor = personal_session(&mut tx, secret).await?;
        let token = new_secret("lmmpt_")?;
        // Compare-and-swap serializes first issuance and concurrent rotations.
        let row = sqlx::query("INSERT INTO core_personal_tokens.tokens AS t(user_id,digest,user_version,generation,expires_at) SELECT $1,$2,$3,1,clock_timestamp()+interval '30 days' WHERE $4::bigint=0 ON CONFLICT(user_id) DO NOTHING RETURNING generation,extract(epoch FROM expires_at)::bigint AS expiry")
            .bind(actor.user_id).bind(digest(&token)?).bind(actor.user_version).bind(expected).fetch_optional(&mut *tx).await?;
        let row = match row {
            Some(row) => row,
            None if expected > 0 => sqlx::query("UPDATE core_personal_tokens.tokens SET digest=$2,user_version=$3,generation=generation+1,expires_at=clock_timestamp()+interval '30 days' WHERE user_id=$1 AND generation=$4 RETURNING generation,extract(epoch FROM expires_at)::bigint AS expiry")
                .bind(actor.user_id).bind(digest(&token)?).bind(actor.user_version).bind(expected).fetch_optional(&mut *tx).await?.ok_or(IdentityError::Conflict)?,
            None => return Err(IdentityError::Conflict),
        };
        let generation: i64 = row.try_get("generation")?;
        let expires_at = row.try_get("expiry")?;
        audit(
            &mut tx,
            &actor,
            "personal-token.rotate",
            json!({"generation":generation,"scope":SCOPE}),
        )
        .await?;
        tx.commit().await?;
        Ok(RotatedToken {
            token,
            generation: generation.to_string(),
            expires_at,
            scope: SCOPE,
        })
    }
    /// This result contains no credential principal or spending authority.
    pub async fn personal_token_profile(&self, token: &str) -> Result<PersonalProfile> {
        if !token.starts_with("lmmpt_") {
            return Err(IdentityError::Unauthorized);
        }
        let row = sqlx::query("SELECT u.id,p.username,p.display_name FROM core_personal_tokens.tokens t JOIN core_identity.users u ON u.id=t.user_id JOIN core_identity.accounts a ON a.id=u.personal_account_id JOIN core_identity.user_profiles p ON p.user_id=u.id WHERE t.digest=$1 AND t.user_version=u.auth_version AND t.expires_at>clock_timestamp() AND u.active AND a.active")
            .bind(digest(token)?).fetch_optional(&self.pool).await?.ok_or(IdentityError::Unauthorized)?;
        Ok(PersonalProfile {
            user_id: row.try_get::<i64, _>("id")?.to_string(),
            username: row.try_get("username")?,
            display_name: row.try_get("display_name")?,
        })
    }
}
fn bearer(headers: &HeaderMap) -> Result<&str> {
    let mut values = headers.get_all(header::AUTHORIZATION).iter();
    let raw = values
        .next()
        .ok_or(IdentityError::Unauthorized)?
        .to_str()
        .map_err(|_| IdentityError::Unauthorized)?;
    if values.next().is_some() {
        return Err(IdentityError::Unauthorized);
    }
    let (scheme, token) = raw.split_once(' ').ok_or(IdentityError::Unauthorized)?;
    if !scheme.eq_ignore_ascii_case("Bearer") {
        return Err(IdentityError::Unauthorized);
    }
    Ok(token)
}
// Reuse native HTTP error semantics without introducing a new global impl.
type HttpResult<T> = std::result::Result<T, super::admin_users::ProfileError>;
async fn status(
    State(store): State<IdentityStore>,
    headers: HeaderMap,
) -> HttpResult<Json<TokenStatus>> {
    Ok(Json(store.personal_token_status(bearer(&headers)?).await?))
}
async fn rotate(
    State(store): State<IdentityStore>,
    headers: HeaderMap,
    input: std::result::Result<Json<RotateRequest>, JsonRejection>,
) -> HttpResult<Json<RotatedToken>> {
    let secret = bearer(&headers)?;
    let Json(input) = input.map_err(|_| IdentityError::Invalid)?;
    Ok(Json(store.rotate_personal_token(secret, &input).await?))
}
async fn profile(
    State(store): State<IdentityStore>,
    headers: HeaderMap,
) -> HttpResult<Json<PersonalProfile>> {
    Ok(Json(store.personal_token_profile(bearer(&headers)?).await?))
}
pub fn router(store: IdentityStore) -> Router {
    Router::new()
        .route("/core/v1/me/personal-token", get(status))
        .route("/core/v1/me/personal-token/rotate", post(rotate))
        .route("/core/v1/me/personal-token/profile", get(profile))
        .layer(axum::middleware::from_fn(
            |request: axum::extract::Request, next: axum::middleware::Next| async move {
                let mut response = next.run(request).await.into_response();
                response.headers_mut().insert(
                    header::CACHE_CONTROL,
                    "private, no-store".parse().expect("static header"),
                );
                response.headers_mut().insert(
                    header::VARY,
                    "Authorization".parse().expect("static header"),
                );
                response
            },
        ))
        .with_state(store)
}

#[cfg(test)]
mod tests;

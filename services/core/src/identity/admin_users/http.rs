use super::*;
use axum::{
    Json, Router,
    extract::{
        Path, Query, State,
        rejection::{PathRejection, QueryRejection},
    },
    http::{HeaderMap, StatusCode, header},
    response::{IntoResponse, Response},
    routing::get,
};
fn include_identity_origin(headers: &HeaderMap) -> bool {
    headers.get_all("prefer").iter().any(|value| {
        value.to_str().is_ok_and(|value| {
            value
                .split(',')
                .any(|item| item.trim() == r#"include="identity-origin""#)
        })
    })
}

pub(super) fn bearer(h: &HeaderMap) -> ReadResult<&str> {
    let mut values = h.get_all(header::AUTHORIZATION).iter();
    let value = values
        .next()
        .ok_or(AdminUsersError::Unauthorized)?
        .to_str()
        .map_err(|_| AdminUsersError::Unauthorized)?;
    if values.next().is_some() {
        return Err(AdminUsersError::Unauthorized);
    }
    let (scheme, secret) = value.split_once(' ').ok_or(AdminUsersError::Unauthorized)?;
    if !scheme.eq_ignore_ascii_case("Bearer") {
        return Err(AdminUsersError::Unauthorized);
    }
    Ok(secret)
}
impl IntoResponse for AdminUsersError {
    fn into_response(self) -> Response {
        let (status, code) = match self {
            Self::Unauthorized => (StatusCode::UNAUTHORIZED, "ADMIN_USERS_UNAUTHORIZED"),
            Self::Forbidden => (StatusCode::FORBIDDEN, "ADMIN_USERS_FORBIDDEN"),
            Self::Invalid => (StatusCode::BAD_REQUEST, "ADMIN_USERS_INVALID"),
            Self::NotFound => (StatusCode::NOT_FOUND, "ADMIN_USERS_NOT_FOUND"),
            Self::UnsupportedFilter => (
                StatusCode::UNPROCESSABLE_ENTITY,
                "ADMIN_USERS_FILTER_UNAVAILABLE",
            ),
            Self::UnsupportedSort => (
                StatusCode::UNPROCESSABLE_ENTITY,
                "ADMIN_USERS_SORT_UNAVAILABLE",
            ),
            Self::Unavailable => (StatusCode::SERVICE_UNAVAILABLE, "ADMIN_USERS_UNAVAILABLE"),
        };
        (status, Json(json!({"error":{"code":code}}))).into_response()
    }
}
pub fn router(store: AdminUsersStore) -> Router {
    let personal_tokens = super::super::personal_token::router(store.identity.clone());
    Router::new()
        .merge(super::profile_mutation::routes())
        .merge(super::access_groups::routes())
        .merge(super::security::routes())
        .merge(super::appeals::routes())
        .merge(super::balance_adjustments::routes())
        .route(
            "/core/v1/admin/users",
            get(users).post(super::create::create),
        )
        .route("/core/v1/admin/users/groups", get(groups))
        .route("/core/v1/admin/users/{id}", get(user))
        .route("/core/v1/admin/system-resources", get(system_resources))
        .layer(axum::middleware::from_fn(
            |request: axum::extract::Request, next: axum::middleware::Next| async move {
                let vary = if include_identity_origin(request.headers()) {
                    "Authorization, Prefer"
                } else {
                    "Authorization"
                };
                let mut response = next.run(request).await;
                response.headers_mut().insert(
                    header::CACHE_CONTROL,
                    "private, no-store".parse().expect("static header"),
                );
                response
                    .headers_mut()
                    .insert(header::VARY, vary.parse().expect("static header"));
                response
            },
        ))
        .with_state(store)
        .merge(personal_tokens)
}
async fn users(
    State(store): State<AdminUsersStore>,
    headers: HeaderMap,
    input: std::result::Result<Query<UsersQuery>, QueryRejection>,
) -> ReadResult<Json<UsersPage>> {
    let secret = bearer(&headers)?;
    let Query(input) = input.map_err(|_| AdminUsersError::Invalid)?;
    Ok(Json(
        store
            .read_users(secret, &input, include_identity_origin(&headers))
            .await?,
    ))
}
async fn user(
    State(store): State<AdminUsersStore>,
    headers: HeaderMap,
    input: std::result::Result<Path<i64>, PathRejection>,
) -> ReadResult<Json<UserDetail>> {
    let secret = bearer(&headers)?;
    let Path(user_id) = input.map_err(|_| AdminUsersError::Invalid)?;
    Ok(Json(
        store
            .read_user(secret, user_id, include_identity_origin(&headers))
            .await?,
    ))
}
async fn groups(
    State(store): State<AdminUsersStore>,
    headers: HeaderMap,
    input: std::result::Result<Query<GroupsQuery>, QueryRejection>,
) -> ReadResult<Json<GroupsPage>> {
    let secret = bearer(&headers)?;
    let Query(input) = input.map_err(|_| AdminUsersError::Invalid)?;
    Ok(Json(store.groups(secret, &input).await?))
}

async fn system_resources(
    State(store): State<AdminUsersStore>,
    headers: HeaderMap,
) -> ReadResult<Json<crate::system_info::Snapshot>> {
    let secret = bearer(&headers)?;
    let mut tx = store.begin().await?;
    AdminUsersStore::authorize(&mut tx, secret).await?;
    tx.commit().await?;
    let snapshot = crate::system_info::snapshot()
        .await
        .map_err(|_| AdminUsersError::Unavailable)?;
    Ok(Json(snapshot))
}

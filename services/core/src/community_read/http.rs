//! Mount only on Core's protected private UDS listener with a community-specific
//! host token. Never mounted on the public HTTP listener.
use super::*;
use axum::{
    Json, Router,
    extract::{DefaultBodyLimit, State, rejection::JsonRejection},
    http::{HeaderMap, StatusCode},
    response::{IntoResponse, Response},
    routing::post,
};
use sha2::{Digest, Sha256};
use std::sync::Arc;
use subtle::ConstantTimeEq;
use tokio::sync::Semaphore;

#[derive(Clone)]
struct Service {
    read: CommunityRead,
    token: [u8; 32],
    slots: Arc<Semaphore>,
}

fn single<'a>(headers: &'a HeaderMap, name: &str) -> Result<&'a str> {
    let mut values = headers.get_all(name).iter();
    let v = values.next().ok_or(Error::Unauthorized)?;
    if values.next().is_some() {
        return Err(Error::Unauthorized);
    }
    v.to_str().map_err(|_| Error::Unauthorized)
}

impl Service {
    fn check(&self, headers: &HeaderMap) -> Result<tokio::sync::SemaphorePermit<'_>> {
        let bearer = single(headers, "authorization")?
            .strip_prefix("Bearer ")
            .ok_or(Error::Unauthorized)?;
        let digest: [u8; 32] = Sha256::digest(bearer.as_bytes()).into();
        if !(32..=256).contains(&bearer.len()) || !bool::from(digest.ct_eq(&self.token)) {
            return Err(Error::Unauthorized);
        }
        if single(headers, "x-lmm-protocol")? != "1" {
            return Err(Error::Invalid);
        }
        self.slots.try_acquire().map_err(|_| Error::Unavailable)
    }
}

fn reply<T: serde::Serialize>(result: Result<T>) -> Response {
    match result {
        Ok(v) => Json(v).into_response(),
        Err(e) => {
            let (status, code) = match e {
                Error::Invalid => (StatusCode::BAD_REQUEST, "invalid"),
                Error::Unauthorized => (StatusCode::UNAUTHORIZED, "unauthorized"),
                Error::TooDense => (StatusCode::UNPROCESSABLE_ENTITY, "too_dense"),
                Error::Unavailable => (StatusCode::SERVICE_UNAVAILABLE, "unavailable"),
            };
            (status, Json(serde_json::json!({"error":code}))).into_response()
        }
    }
}

macro_rules! endpoint {
    ($name:ident, $input:ty, $call:ident) => {
        async fn $name(
            State(s): State<Service>,
            headers: HeaderMap,
            body: std::result::Result<Json<$input>, JsonRejection>,
        ) -> Response {
            reply(
                async {
                    let _slot = s.check(&headers)?;
                    let Json(input) = body.map_err(|_| Error::Invalid)?;
                    s.read.$call(&input).await
                }
                .await,
            )
        }
    };
}
endpoint!(models, Window, model_buckets);
endpoint!(user_rankings, UsersRequest, user_rankings);
endpoint!(user_usage, UserUsageRequest, user_usage);
endpoint!(users, UsersInfoRequest, users);
endpoint!(key, KeyRequest, key);

async fn capabilities(State(s): State<Service>, headers: HeaderMap) -> Response {
    reply(s.check(&headers).map(|_| {
        serde_json::json!({"protocol_major":1,"features":["community.usage.v1","community.users.v1","community.keys.v1"],"history_source":s.read.history_source})
    }))
}

pub fn router(read: CommunityRead, token: &[u8]) -> Result<Router> {
    if !(32..=256).contains(&token.len()) || !token.iter().all(u8::is_ascii_graphic) {
        return Err(Error::Invalid);
    }
    Ok(Router::new()
        .route("/internal/community/v1/capabilities", post(capabilities))
        .route("/internal/community/v1/usage/models", post(models))
        .route("/internal/community/v1/usage/users", post(user_rankings))
        .route("/internal/community/v1/usage/user", post(user_usage))
        .route("/internal/community/v1/users", post(users))
        .route("/internal/community/v1/keys", post(key))
        .layer(DefaultBodyLimit::max(16 * 1024))
        .with_state(Service {
            read,
            token: Sha256::digest(token).into(),
            slots: Arc::new(Semaphore::new(2)),
        }))
}

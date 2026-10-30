//! Account closure has no Cookie write: revoked server authority is terminal.
//! Unknown transport outcomes are resolved by the original opaque receipt key.
use super::*;
use crate::identity::{AccountClosureOutcome, AccountClosureRequest};
use axum::extract::Path;

pub(super) fn routes() -> Router<AuthState> {
    Router::new()
        .route("/api/user/self/closure", get(preview))
        .route("/api/user/self/closure/receipts/{key}", get(receipt))
        .route("/api/user/self", axum::routing::delete(close))
}
async fn preview(State(state): State<AuthState>, headers: HeaderMap) -> Response {
    let result = async {
        state
            .browser
            .account_closure_preview(bearer(&headers)?.ok_or(IdentityError::Unauthorized)?)
            .await
    }
    .await;
    match result {
        Ok(view) => success(Some(json!(view))),
        Err(error) => auth_failure(error),
    }
}
async fn close(
    State(state): State<AuthState>,
    headers: HeaderMap,
    input: Result<Json<AccountClosureRequest>, JsonRejection>,
) -> Response {
    let result = async {
        let Json(input) = input.map_err(|_| IdentityError::Invalid)?;
        state
            .browser
            .close_account(
                bearer(&headers)?.ok_or(IdentityError::Unauthorized)?,
                &input,
            )
            .await
    }
    .await;
    match result {
        Ok(AccountClosureOutcome::Closed(receipt)) => success(Some(json!(receipt))),
        Ok(AccountClosureOutcome::Blocked(view)) => (
            StatusCode::CONFLICT,
            Json(json!({"success":false,"code":"ACCOUNT_CLOSURE_BLOCKED","data":view})),
        )
            .into_response(),
        Err(error) => auth_failure(error),
    }
}
async fn receipt(
    State(state): State<AuthState>,
    Path(key): Path<String>,
    peer: Result<ConnectInfo<SocketAddr>, axum::extract::rejection::ExtensionRejection>,
) -> Response {
    let peer = peer
        .map(|p| p.0.ip().to_string())
        .unwrap_or_else(|_| "unknown-peer".into());
    match state
        .store
        .auth_rate_limit("closure_receipt", &peer, 60)
        .await
    {
        Ok(true) => {}
        Ok(false) => {
            return failure(
                StatusCode::TOO_MANY_REQUESTS,
                "AUTH_RATE_LIMITED",
                "Please retry later",
            );
        }
        Err(error) => return auth_failure(error.into()),
    }
    match state.browser.account_closure_receipt(&key).await {
        Ok(Some(receipt)) => success(Some(json!(receipt))),
        Ok(None) => failure(
            StatusCode::NOT_FOUND,
            "ACCOUNT_CLOSURE_RECEIPT_UNKNOWN",
            "Account closure is not confirmed",
        ),
        Err(error) => auth_failure(error),
    }
}

//! Read-only policy preview using the same parser as settings and live access.
use super::*;

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Preview {
    rules: String,
    ip: String,
    country: Option<String>,
}

async fn authorize_readonly(state: &OptionsHttp, headers: &HeaderMap) -> Result<(), IdentityError> {
    let secret = bearer(headers)?;
    let mut tx = state.pool.begin().await?;
    sqlx::query("SET TRANSACTION READ ONLY")
        .execute(&mut *tx)
        .await?;
    let actor = crate::identity::authenticate_readonly_in_transaction(&mut tx, secret).await?;
    if actor.credential_kind != CredentialKind::Session || actor.platform_level < 6 {
        return Err(IdentityError::Forbidden);
    }
    tx.rollback().await?;
    Ok(())
}

pub(super) async fn preview(
    State(state): State<OptionsHttp>,
    headers: HeaderMap,
    body: Bytes,
) -> Response {
    if let Err(error) = authorize_readonly(&state, &headers).await {
        return auth_error(error);
    }
    let result = serde_json::from_slice::<Preview>(&body)
        .ok()
        .filter(|request| request.rules.len() <= 16_384)
        .and_then(|request| {
            crate::edge_access::preview_policy(
                &request.rules,
                &request.ip,
                request.country.as_deref(),
            )
            .ok()
        });
    match result {
        Some(data) => response(StatusCode::OK, json!({"success":true,"data":data})),
        None => response(
            StatusCode::BAD_REQUEST,
            json!({"success":false,"code":"SYSTEM_OPTIONS_INVALID",
                "message":"Invalid IP access rules, IP address or country"}),
        ),
    }
}

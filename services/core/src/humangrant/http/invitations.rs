//! Email-bearer links can refuse contact, but cannot approve any permission.
use super::*;
use crate::humangrant::mail::{MailInvitation, invitation_block_key};

pub(super) fn routes() -> Router<SharedEngine> {
    Router::new()
        .route("/invite/{token}", get(page))
        .route("/invite/{token}/reject", post(reject))
        .route("/invite/{token}/block", post(block))
}

fn invitation_id(token: &str) -> Result<String, ProtocolError> {
    if token.len() != 43
        || !token
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'_' || b == b'-')
    {
        return Err(failure(404, "not_found"));
    }
    Ok(fingerprint(token.as_bytes()))
}
async fn page(State(engine): State<SharedEngine>, Path(token): Path<String>) -> Response {
    let now = chrono::Utc::now().timestamp();
    let id = match invitation_id(&token) {
        Ok(id) => id,
        Err(error) => return error_response(error),
    };
    let result = async {
        let mut tx = engine.store.begin().await?;
        let invite = tx
            .get::<MailInvitation>("mail-invite", &id, now)
            .await?
            .ok_or_else(|| failure(404, "not_found"))?;
        let request = tx
            .get::<Request>("request", &invite.request_id, now)
            .await?
            .ok_or_else(|| failure(404, "not_found"))?;
        let (flow, key) = engine
            .new_flow(
                &mut tx,
                &format!("invite:{id}"),
                Human::default(),
                false,
                now,
            )
            .await?;
        tx.commit().await?;
        Ok((flow, key, request.link))
    }
    .await;
    match result {
        Ok((flow, key, link)) => {
            let html = render_page(
                "invite",
                &serde_json::json!({
                    "token": token, "csrf": flow.csrf, "review_url":format!("/human/{link}"),
                }),
            );
            html_response(
                html,
                Some(make_cookie_header(
                    engine.flow_cookie_name(),
                    &key,
                    600,
                    !engine.local || !engine.origin.starts_with("http:"),
                )),
            )
        }
        Err(error) => error_response(error),
    }
}

async fn refuse(
    engine: SharedEngine,
    token: String,
    headers: HeaderMap,
    blocked: bool,
) -> Response {
    let now = chrono::Utc::now().timestamp();
    let id = match invitation_id(&token) {
        Ok(id) => id,
        Err(error) => return error_response(error),
    };
    let result = async {
        let mut tx = engine.store.begin().await?;
        engine
            .check_flow(&mut tx, &headers, &format!("invite:{id}"), now)
            .await?;
        let mut invite = tx
            .get_for_update::<MailInvitation>("mail-invite", &id, now)
            .await?
            .ok_or_else(|| failure(404, "not_found"))?;
        if blocked {
            tx.put(
                "mail-block",
                &invitation_block_key(&invite.recipient_hash, &invite.actor),
                &invite.recipient_hash,
                0,
                &true,
            )
            .await?;
        }
        invite.rejected_at = now;
        tx.put(
            "mail-invite",
            &id,
            &invite.recipient_hash,
            invite.expires_at,
            &invite,
        )
        .await?;
        if let Some(mut request) = tx
            .get_for_update::<Request>("request", &invite.request_id, now)
            .await?
            && request.actor == invite.actor
            && matches!(
                request.phase.as_str(),
                "waiting_human" | "authenticating_human" | "awaiting_consent"
            )
        {
            request.phase = "rejected".into();
            request.updated = now;
            tx.put(
                "request",
                &request.id,
                &request.actor,
                request.created + RETENTION_SECONDS,
                &request,
            )
            .await?;
        }
        engine
            .audit(
                &mut tx,
                if blocked {
                    "invitation.block"
                } else {
                    "invitation.reject"
                },
                &invite.actor,
                "",
                &invite.request_id,
                now,
            )
            .await?;
        tx.commit().await?;
        Ok(())
    }
    .await;
    match result {
        Ok(()) => json_response(
            StatusCode::OK,
            &serde_json::json!({"status":if blocked {"blocked"} else {"rejected"}}),
        ),
        Err(error) => error_response(error),
    }
}
async fn reject(
    State(engine): State<SharedEngine>,
    Path(token): Path<String>,
    headers: HeaderMap,
) -> Response {
    refuse(engine, token, headers, false).await
}
async fn block(
    State(engine): State<SharedEngine>,
    Path(token): Path<String>,
    headers: HeaderMap,
) -> Response {
    refuse(engine, token, headers, true).await
}

//! Original-browser-only one-time key display. No model or extension receipt
//! has a route to this plaintext response.
use super::*;
use crate::identity::assistant_actions;

fn name(s: &BrowserConfirmation, id: &str) -> String {
    format!("{}-key", cookie_name(s, id))
}
pub(super) async fn inspect(
    State(s): State<BrowserConfirmation>,
    Path(id): Path<String>,
    h: HeaderMap,
) -> Response {
    if !valid_id(&id) {
        return response(Err(Error::Invalid));
    }
    let mut set_cookie = None;
    let result = async {
        let mut tx = crate::humangrant::HumanGrantStore::new(s.pool.clone())
            .begin()
            .await
            .map_err(|_| Error::Unavailable)?;
        let (row, context) = browser_row(&mut tx, &h, &id).await?;
        if row.try_get::<String, _>("tool_name")? != "request_create_key"
            || row.try_get::<String, _>("state")? != "approved"
        {
            return Err(Error::Forbidden);
        }
        let nonce = random();
        let card =
            assistant_actions::inspect_key_card(&mut tx.tx, &context, &id, &sha(nonce.as_bytes()))
                .await?;
        let mut result = json!({"ok":true,"key_card":card});
        if result["key_card"]["state"] == "ready" {
            result["csrf"] = json!(nonce);
            set_cookie = Some(format!(
                "{}={}; Path=/; HttpOnly; SameSite=Strict; Max-Age=600{}",
                name(&s, &id),
                nonce,
                if s.local && s.origin.starts_with("http:") {
                    ""
                } else {
                    "; Secure"
                },
            ));
        }
        tx.commit().await.map_err(|_| Error::Unavailable)?;
        Ok(result)
    }
    .await;
    let mut reply = response(result);
    if let Some(cookie) = set_cookie {
        reply
            .headers_mut()
            .append(header::SET_COOKIE, cookie.parse().unwrap());
    }
    reply
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct Reveal {
    reveal: bool,
}
pub(super) async fn consume(
    State(s): State<BrowserConfirmation>,
    Path(id): Path<String>,
    h: HeaderMap,
    Json(input): Json<Reveal>,
) -> Response {
    if !valid_id(&id) || !input.reveal || single_header(&h, "origin") != Some(s.origin.as_str()) {
        return response(Err(Error::Forbidden));
    }
    let result = async {
        let cookie_name = name(&s, &id);
        let nonce = cookie(&h, &cookie_name).ok_or(Error::Forbidden)?;
        let csrf = single_header(&h, "x-lmm-assistant-csrf").ok_or(Error::Forbidden)?;
        if nonce.len() != 64 || !bool::from(nonce.as_bytes().ct_eq(csrf.as_bytes())) {
            return Err(Error::Forbidden);
        }
        let mut tx = crate::humangrant::HumanGrantStore::new(s.pool.clone())
            .begin()
            .await
            .map_err(|_| Error::Unavailable)?;
        let (row, context) = browser_row(&mut tx, &h, &id).await?;
        if row.try_get::<String, _>("tool_name")? != "request_create_key"
            || row.try_get::<String, _>("state")? != "approved"
        {
            return Err(Error::Forbidden);
        }
        let store = s.identity.as_ref().ok_or(Error::Unavailable)?;
        let result = assistant_actions::consume_key_card(
            &mut tx.tx,
            store,
            &context,
            &id,
            &sha(nonce.as_bytes()),
        )
        .await?;
        // A lost commit acknowledgement is UNKNOWN to the browser; no automatic
        // retry reveals a second copy. GET can read the durable consumed state.
        tx.commit().await.map_err(|_| Error::Unavailable)?;
        Ok(result)
    }
    .await;
    let mut reply = response(result);
    if reply.status().is_success() {
        reply.headers_mut().append(
            header::SET_COOKIE,
            format!(
                "{}=; Path=/; HttpOnly; SameSite=Strict; Max-Age=0{}",
                name(&s, &id),
                if s.local && s.origin.starts_with("http:") {
                    ""
                } else {
                    "; Secure"
                },
            )
            .parse()
            .unwrap(),
        );
    }
    reply
}

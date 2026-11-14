use super::*;
use crate::html::escape;
use axum::{
    Json, Router,
    extract::{DefaultBodyLimit, Path, State},
    http::{HeaderMap, StatusCode, header},
    response::{Html, IntoResponse, Response},
    routing::get,
};
use serde::Deserialize;
#[derive(Clone)]
struct Browser {
    store: Store,
    origin: String,
}
impl IntoResponse for Error {
    fn into_response(self) -> Response {
        let status = match self {
            Self::Invalid => StatusCode::BAD_REQUEST,
            Self::Unauthorized => StatusCode::UNAUTHORIZED,
            Self::Forbidden | Self::ConfirmationRequired => StatusCode::FORBIDDEN,
            Self::Conflict | Self::Expired => StatusCode::CONFLICT,
            _ => StatusCode::SERVICE_UNAVAILABLE,
        };
        (status, Json(json!({"error":format!("{self:?}")}))).into_response()
    }
}
fn cookie(h: &HeaderMap) -> Result<&str> {
    let mut found = None;
    for value in h.get_all(header::COOKIE) {
        for part in value.to_str().map_err(|_| Error::Unauthorized)?.split(';') {
            if let Some((name, value)) = part.trim().split_once('=')
                && name == "new_api_refresh"
            {
                if found.is_some() || value.is_empty() {
                    return Err(Error::Unauthorized);
                }
                found = Some(value);
            }
        }
    }
    found.ok_or(Error::Unauthorized)
}
fn secure(mut r: Response) -> Response {
    for (k, v) in [
        ("cache-control", "no-store"),
        (
            "content-security-policy",
            "default-src 'none'; form-action 'self'; frame-ancestors 'none'; base-uri 'none'",
        ),
        ("x-content-type-options", "nosniff"),
        ("referrer-policy", "no-referrer"),
    ] {
        r.headers_mut().insert(
            axum::http::HeaderName::from_static(k),
            axum::http::HeaderValue::from_static(v),
        );
    }
    r
}
pub fn router(store: Store, origin: String, local_http: bool) -> Result<Router> {
    let u = reqwest::Url::parse(&origin).map_err(|_| Error::Invalid)?;
    let local = local_http
        && u.scheme() == "http"
        && u.host_str()
            .and_then(|h| h.trim_matches(['[', ']']).parse::<std::net::IpAddr>().ok())
            .is_some_and(|ip| ip.is_loopback());
    if (u.scheme() != "https" && !local)
        || u.origin().ascii_serialization() != origin
        || !u.username().is_empty()
        || u.password().is_some()
    {
        return Err(Error::Invalid);
    }
    Ok(Router::new()
        .route(
            "/core/v1/directory-ads/confirm/{id}",
            get(page).post(confirm),
        )
        .layer(DefaultBodyLimit::max(2048))
        .with_state(Browser { store, origin }))
}
async fn page(State(s): State<Browser>, Path(id): Path<String>, h: HeaderMap) -> Result<Response> {
    let mut t = s.store.begin().await?;
    let a = identity::BrowserAuth::oauth_identity_in_transaction(&mut t, cookie(&h)?).await?;
    let r=sqlx::query("SELECT command,quota,expires_at FROM core_directory_ads.quotes WHERE id=$1 AND user_id=$2 AND account_id=$3 FOR UPDATE").bind(&id).bind(a.user).bind(a.account).fetch_optional(&mut *t).await?.ok_or(Error::Forbidden)?;
    if r.try_get::<i64, _>("expires_at")? < now(&mut t).await? {
        return Err(Error::Expired);
    }
    let csrf = random_id();
    sqlx::query("UPDATE core_directory_ads.quotes SET confirmation_digest=$2,confirmation_sid=$3 WHERE id=$1").bind(&id).bind(Sha256::digest(csrf.as_bytes()).to_vec()).bind(a.sid).execute(&mut *t).await?;
    let content: Value = r.try_get("command")?;
    let quota: i64 = r.try_get("quota")?;
    t.commit().await?;
    let html = format!(
        "<!doctype html><meta charset=\"utf-8\"><title>Confirm advertisement</title><h1>Confirm advertisement purchase</h1><h2>{}</h2><p>{}</p><p>{}</p><p>{}</p><p>Charge: {quota} credits (500000 credits = 1 USD). Duration: 30 days.</p><form method=\"post\"><input type=\"hidden\" name=\"csrf\" value=\"{csrf}\"><button type=\"submit\">Approve this exact purchase</button></form><p>To cancel, close this window. Approval does not itself charge funds. Return to the original page to continue publishing.</p>",
        escape(content[1].as_str().ok_or(Error::Storage)?),
        escape(content[2].as_str().ok_or(Error::Storage)?),
        escape(content[3].as_str().ok_or(Error::Storage)?),
        escape(content[4].as_str().ok_or(Error::Storage)?)
    );
    Ok(secure(Html(html).into_response()))
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Confirm {
    csrf: String,
}
async fn confirm(
    State(s): State<Browser>,
    Path(id): Path<String>,
    h: HeaderMap,
    axum::Form(input): axum::Form<Confirm>,
) -> Result<Response> {
    let mut origins = h.get_all(header::ORIGIN).iter();
    if origins.next().and_then(|v| v.to_str().ok()) != Some(s.origin.as_str())
        || origins.next().is_some()
    {
        return Err(Error::Forbidden);
    }
    let mut t = s.store.begin().await?;
    let a = identity::BrowserAuth::oauth_identity_in_transaction(&mut t, cookie(&h)?).await?;
    let r=sqlx::query("SELECT * FROM core_directory_ads.quotes WHERE id=$1 AND user_id=$2 AND account_id=$3 FOR UPDATE").bind(&id).bind(a.user).bind(a.account).fetch_optional(&mut *t).await?.ok_or(Error::Forbidden)?;
    if r.try_get::<i64, _>("expires_at")? < now(&mut t).await? {
        return Err(Error::Expired);
    }
    if r.try_get::<Option<String>, _>("confirmation_sid")?
        .as_deref()
        != Some(a.sid.as_str())
        || r.try_get::<Option<Vec<u8>>, _>("confirmation_digest")?
            != Some(Sha256::digest(input.csrf.as_bytes()).to_vec())
    {
        return Err(Error::Forbidden);
    }
    identity::reject_marked_new_action(
        &mut t,
        a.user,
        identity::NewResourceAction::CommercePayment,
    )
    .await?;
    sqlx::query(
        "UPDATE core_directory_ads.quotes SET confirmed_sid=$2,confirmed_at=$3 WHERE id=$1",
    )
    .bind(id)
    .bind(a.sid)
    .bind(now(&mut t).await?)
    .execute(&mut *t)
    .await?;
    t.commit().await?;
    Ok(secure(Html("<!doctype html><meta charset=\"utf-8\"><title>Advertisement approved</title><h1>Purchase approved</h1><p>No payment has been made by this confirmation. Return to the original page and continue publishing. Only a successful payment receipt completes the purchase.</p>").into_response()))
}

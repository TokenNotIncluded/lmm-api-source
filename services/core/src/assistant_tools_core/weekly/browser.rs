//! Native browser-only claim boundary. Model/Go callers never receive the code.
use super::super::confirmation::{
    BrowserConfirmation, cookie, random, response, sha, single_header, valid_id,
};
use super::{Error, VerifiedToolContext, lock_owner, now, policy, projection, week_start};
use axum::{
    Json,
    body::Body,
    extract::{Path, Query, State},
    http::{HeaderMap, StatusCode, header},
    response::Response,
};
use serde::Deserialize;
use serde_json::{Value, json};
use sqlx::{PgConnection, Row, postgres::PgRow};
use subtle::ConstantTimeEq;

async fn authenticate(
    tx: &mut crate::humangrant::HumanGrantTx,
    h: &HeaderMap,
) -> Result<VerifiedToolContext, Error> {
    let (human, _) = tx
        .authenticate_human(cookie(h, "new_api_refresh"), chrono::Utc::now().timestamp())
        .await
        .map_err(|_| Error::Unavailable)?
        .ok_or(Error::Unauthorized)?;
    VerifiedToolContext::from_authenticated_browser(&mut tx.tx, &human).await
}
fn nonce_name(s: &BrowserConfirmation, id: &str) -> String {
    format!(
        "{}lmm-weekly-{}",
        if s.local && s.origin.starts_with("http:") {
            ""
        } else {
            "__Host-"
        },
        &id[..16]
    )
}
async fn owned(
    c: &mut PgConnection,
    context: &VerifiedToolContext,
    id: &str,
) -> Result<PgRow, Error> {
    context.revalidate(c).await?;
    sqlx::query("SELECT * FROM core_weekly.weekly_discounts WHERE id=$1 AND user_id=$2 AND account_id=$3 FOR UPDATE")
        .bind(id).bind(context.user_id()).bind(context.account_id()).fetch_optional(c).await?.ok_or(Error::NotFound)
}
fn original_browser(row: &PgRow, actor: &super::super::context::Facts) -> Result<bool, Error> {
    Ok(row.try_get::<String, _>("origin")? == "legacy"
        || (row.try_get::<Option<String>, _>("web_session_sid")? == actor.browser_session
            && row.try_get::<Option<i64>, _>("auth_version")? == Some(actor.auth_version)))
}
async fn browser_projection(c: &mut PgConnection, row: &PgRow) -> Result<Value, Error> {
    let mut value = projection(row)?;
    value["reason"] = json!(row.try_get::<String, _>("reason")?);
    if row.try_get::<Option<i64>, _>("code_id")?.is_some() {
        let code: Option<String> =
            sqlx::query_scalar("SELECT core_weekly.claimed_weekly_discount_code($1,$2,$3)")
                .bind(row.try_get::<String, _>("id")?)
                .bind(row.try_get::<i64, _>("user_id")?)
                .bind(row.try_get::<i64, _>("account_id")?)
                .fetch_one(c)
                .await?;
        value["code"] = json!(code.ok_or(Error::Unavailable)?);
    }
    Ok(value)
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(in crate::assistant_tools_core) struct HistoryArgs {
    before_week: Option<i64>,
}
pub(in crate::assistant_tools_core) async fn history(
    State(s): State<BrowserConfirmation>,
    Query(args): Query<HistoryArgs>,
    h: HeaderMap,
) -> Response {
    let result=async {
        if args.before_week.is_some_and(|v|v<0) { return Err(Error::Invalid); }
        let mut tx=crate::humangrant::HumanGrantStore::new(s.pool.clone()).begin().await.map_err(|_|Error::Unavailable)?;
        let context=authenticate(&mut tx,&h).await?;
        let rows=sqlx::query("SELECT * FROM core_weekly.weekly_discounts WHERE user_id=$1 AND account_id=$2 AND ($3::bigint IS NULL OR week_start<$3) ORDER BY week_start DESC LIMIT 100")
            .bind(context.user_id()).bind(context.account_id()).bind(args.before_week).fetch_all(&mut *tx.tx).await?;
        let decisions=rows.iter().map(projection).collect::<Result<Vec<_>,_>>()?;
        let current_utc_week=week_start(now(&mut tx.tx).await?);
        let next=if rows.len()==100 {Some(rows.last().ok_or(Error::Unavailable)?.try_get::<i64,_>("week_start")?)}else{None};
        tx.commit().await.map_err(|_|Error::Unavailable)?;
        Ok(json!({"ok":true,"current_utc_week":current_utc_week,"decisions":decisions,"next_before_week":next}))
    }.await;
    response(result)
}
// The immutable model result keeps the API URL. A real document navigation
// only opens the owner page; its subsequent JSON GET authenticates the session.
fn browser_navigation(h: &HeaderMap, id: &str) -> Option<Response> {
    if !valid_id(id)
        || single_header(h, "sec-fetch-mode") != Some("navigate")
        || single_header(h, "sec-fetch-dest") != Some("document")
        || !single_header(h, "accept")
            .is_some_and(|value| value.split(',').any(|part| part.trim() == "text/html"))
    {
        return None;
    }
    Some(
        Response::builder()
            .status(StatusCode::TEMPORARY_REDIRECT)
            .header(header::LOCATION, format!("/weekly-discounts?id={id}"))
            .header(header::CACHE_CONTROL, "no-store")
            .header(header::REFERRER_POLICY, "no-referrer")
            .header(header::VARY, "Accept, Sec-Fetch-Mode, Sec-Fetch-Dest")
            .header(header::X_CONTENT_TYPE_OPTIONS, "nosniff")
            .body(Body::empty())
            .expect("fixed weekly page navigation"),
    )
}

pub(in crate::assistant_tools_core) async fn inspect(
    State(s): State<BrowserConfirmation>,
    Path(id): Path<String>,
    h: HeaderMap,
) -> Response {
    if !valid_id(&id) {
        return response(Err(Error::Invalid));
    }
    if let Some(page) = browser_navigation(&h, &id) {
        return page;
    }
    let mut set_cookie = None;
    let result=async {
        let mut tx=crate::humangrant::HumanGrantStore::new(s.pool.clone()).begin().await.map_err(|_|Error::Unavailable)?;
        let context=authenticate(&mut tx,&h).await?;
        let row=owned(&mut tx.tx,&context,&id).await?;
        let actor=context.revalidate(&mut tx.tx).await?;
        let stamp=now(&mut tx.tx).await?;
        let (enabled,maximum)=policy(&mut tx.tx).await?;
        let status:String=row.try_get("status")?;
        let claimable=status=="offered" && row.try_get::<i64,_>("week_start")?==week_start(stamp)
            && actor.role=="user" && enabled && row.try_get::<i16,_>("discount_percent")?<=maximum
            && original_browser(&row,&actor)?;
        let mut value=json!({"ok":true,"decision":browser_projection(&mut tx.tx,&row).await?,"claimable":claimable});
        if claimable || status=="claimed" {
            let nonce=random();
            sqlx::query("UPDATE core_weekly.weekly_discounts SET claim_nonce_sha256=$2,claim_nonce_expires_at=$3 WHERE id=$1")
                .bind(&id).bind(sha(nonce.as_bytes())).bind(stamp+300).execute(&mut *tx.tx).await?;
            value["csrf"]=json!(nonce);
            set_cookie=Some(format!("{}={nonce}; Path=/; HttpOnly; SameSite=Strict; Max-Age=300{}",nonce_name(&s,&id),
                if s.local && s.origin.starts_with("http:"){""}else{"; Secure"}));
        }
        tx.commit().await.map_err(|_|Error::Unavailable)?;
        Ok(value)
    }.await;
    let mut result = response(result);
    if let Some(cookie) = set_cookie {
        result.headers_mut().append(
            header::SET_COOKIE,
            cookie.parse().expect("generated cookie"),
        );
    }
    result
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(in crate::assistant_tools_core) struct Claim {
    claim: bool,
}
pub(in crate::assistant_tools_core) async fn claim(
    State(s): State<BrowserConfirmation>,
    Path(id): Path<String>,
    h: HeaderMap,
    Json(input): Json<Claim>,
) -> Response {
    if !input.claim || !valid_id(&id) || single_header(&h, "origin") != Some(s.origin.as_str()) {
        return response(Err(Error::Forbidden));
    }
    let result=async {
        let name=nonce_name(&s,&id);
        let nonce=cookie(&h,&name).ok_or(Error::Forbidden)?;
        let csrf=single_header(&h,"x-lmm-assistant-csrf").ok_or(Error::Forbidden)?;
        if nonce.len()!=64 || !bool::from(nonce.as_bytes().ct_eq(csrf.as_bytes())) {return Err(Error::Forbidden);}
        let rate=crate::identity::assistant_workspace_rate::take(&s.pool,cookie(&h,"new_api_refresh").ok_or(Error::Unauthorized)?).await?;
        if !rate.allowed {return Ok(json!({"ok":false,"error":"confirmation_rate_limited","retry_after":rate.retry_after_seconds,"executed":false}));}
        let mut tx=crate::humangrant::HumanGrantStore::new(s.pool.clone()).begin().await.map_err(|_|Error::Unavailable)?;
        let context=authenticate(&mut tx,&h).await?;
        lock_owner(&mut tx.tx,context.user_id()).await?;
        let row=owned(&mut tx.tx,&context,&id).await?;
        let stamp=now(&mut tx.tx).await?;
        let expected:Option<Vec<u8>>=row.try_get("claim_nonce_sha256")?;
        if !expected.is_some_and(|e|bool::from(e.as_slice().ct_eq(&sha(nonce.as_bytes()))))
            || row.try_get::<Option<i64>,_>("claim_nonce_expires_at")?.is_none_or(|expiry|expiry<=stamp) {
            return Err(Error::Forbidden);
        }
        let actor=context.revalidate(&mut tx.tx).await?;
        if !original_browser(&row,&actor)? {return Err(Error::Forbidden);}
        if row.try_get::<String,_>("status")?!="claimed" {
            let (enabled,maximum)=policy(&mut tx.tx).await?;
            if row.try_get::<String,_>("status")?!="offered" || actor.role!="user" || !enabled
                || row.try_get::<i64,_>("week_start")?!=week_start(stamp)
                || row.try_get::<i16,_>("discount_percent")?>maximum {return Err(Error::Forbidden);}
            let code=format!("AIWEEK-{}",random()[..32].to_uppercase());
            sqlx::query("SELECT code_id FROM core_weekly.claim_weekly_discount_code($1,$2)")
                .bind(&id).bind(code).fetch_one(&mut *tx.tx).await?;
        }
        let final_row=owned(&mut tx.tx,&context,&id).await?;
        let decision=browser_projection(&mut tx.tx,&final_row).await?;
        tx.commit().await.map_err(|_|Error::Unavailable)?;
        Ok(json!({"ok":true,"decision":decision,"executed":true}))
    }.await;
    response(result)
}

#[cfg(test)]
mod navigation_tests {
    use super::*;

    fn document_headers() -> HeaderMap {
        let mut headers = HeaderMap::new();
        headers.insert(
            header::ACCEPT,
            "text/html,application/xhtml+xml,*/*;q=0.8".parse().unwrap(),
        );
        headers.insert("sec-fetch-mode", "navigate".parse().unwrap());
        headers.insert("sec-fetch-dest", "document".parse().unwrap());
        headers
    }

    #[test]
    fn exact_document_navigation_has_no_code_cookie_or_authority() {
        let id = "a".repeat(64);
        let response = browser_navigation(&document_headers(), &id).unwrap();
        assert_eq!(response.status(), StatusCode::TEMPORARY_REDIRECT);
        assert_eq!(
            response.headers()[header::LOCATION],
            format!("/weekly-discounts?id={id}")
        );
        assert_eq!(response.headers()[header::CACHE_CONTROL], "no-store");
        assert_eq!(response.headers()[header::REFERRER_POLICY], "no-referrer");
        assert!(response.headers().get(header::SET_COOKIE).is_none());
    }

    #[test]
    fn json_fetch_and_invalid_identifiers_never_become_document_redirects() {
        let id = "a".repeat(64);
        for invalid in [
            "../other".to_owned(),
            format!("{id}?claim=true"),
            format!("%61{}", &id[1..]),
            "A".repeat(64),
            "a".repeat(63),
        ] {
            assert!(browser_navigation(&document_headers(), &invalid).is_none());
        }
        for (name, value) in [
            ("accept", "application/json"),
            ("accept", "*/*"),
            ("accept", "text/html;q=0"),
            ("sec-fetch-mode", "cors"),
            ("sec-fetch-dest", "empty"),
        ] {
            let mut headers = document_headers();
            headers.insert(
                axum::http::HeaderName::from_bytes(name.as_bytes()).unwrap(),
                value.parse().unwrap(),
            );
            assert!(browser_navigation(&headers, &id).is_none());
        }
        let mut duplicate = document_headers();
        duplicate.append(header::ACCEPT, "text/html".parse().unwrap());
        assert!(browser_navigation(&duplicate, &id).is_none());
        assert!(browser_navigation(&HeaderMap::new(), &id).is_none());
    }
}

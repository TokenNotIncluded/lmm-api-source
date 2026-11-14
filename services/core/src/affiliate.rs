//! Browser-confirmed conversion of existing affiliate entitlement into wallet funds.
use crate::{
    billing::{
        Credits,
        ledger::{Action, Outcome, Request, Wallet},
    },
    identity::BrowserAuth,
    wallet::Error,
};
use axum::{
    Json, Router,
    extract::{DefaultBodyLimit, Path, Query, State},
    http::{HeaderMap, StatusCode, header},
    response::{Html, IntoResponse, Response},
    routing::{get, post},
};
use rand::TryRng as _;
use rand::rngs::SysRng;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use sqlx::{PgPool, Row, types::Json as SqlJson};
use subtle::ConstantTimeEq;

#[derive(Clone)]
pub struct Store {
    pool: PgPool,
}
#[derive(Serialize, Deserialize)]
pub struct Receipt {
    pub id: i64,
    pub amount_credits: i64,
    pub outcome: Outcome,
}
struct Preview {
    id: i64,
    amount: i64,
    csrf: String,
}
impl Store {
    pub fn new(pool: PgPool) -> Self {
        Self { pool }
    }
    pub async fn check_ready(&self) -> Result<(), Error> {
        let ready: bool = sqlx::query_scalar("SELECT has_column_privilege('core_console.options','key','SELECT') AND has_column_privilege('core_console.options','value','SELECT') AND has_table_privilege('core_wallet.affiliate_claims','SELECT') AND has_column_privilege('core_wallet.affiliate_claims','outcome','UPDATE') AND has_sequence_privilege('core_wallet.affiliate_claims_id_seq','USAGE') AND has_sequence_privilege('core_wallet.affiliate_claims_id_seq','SELECT') AND NOT has_table_privilege('core_wallet.affiliate_claims','UPDATE,DELETE,TRUNCATE') AND NOT EXISTS(SELECT FROM unnest(ARRAY['owner_user','account_id','session_id','amount','csrf_digest','expires_at','created_at','id']) c WHERE has_column_privilege('core_wallet.affiliate_claims',c,'UPDATE')) AND NOT EXISTS(SELECT FROM unnest(ARRAY['owner_user','account_id','session_id','amount','csrf_digest','expires_at']) c WHERE NOT has_column_privilege('core_wallet.affiliate_claims',c,'INSERT'))")
            .fetch_one(&self.pool).await?;
        if ready { Ok(()) } else { Err(Error::Forbidden) }
    }
    async fn preview(&self, cookie: &str, amount: Option<i64>) -> Result<Preview, Error> {
        let mut tx = self.pool.begin().await?;
        let actor = BrowserAuth::oauth_identity_in_transaction(&mut tx, cookie).await?;
        payment_policy(&mut tx).await?;
        let _: SqlJson<Wallet> = sqlx::query_scalar("SELECT core_billing.open_ledger_wallet($1)")
            .bind(actor.account)
            .fetch_one(&mut *tx)
            .await?;
        let available: i64 = sqlx::query_scalar("SELECT b.balance_units FROM core_billing.ledger_accounts a JOIN core_billing.balance_state b ON b.ledger_account_id=a.id WHERE a.owner_account_id=$1 AND a.bucket='affiliate'").bind(actor.account).fetch_one(&mut *tx).await?;
        let amount = amount.unwrap_or(available);
        if amount < 500_000 || amount > available || amount > 9_007_199_254_740_991 {
            return Err(Error::Invalid);
        }
        let mut random = [0u8; 32];
        SysRng
            .try_fill_bytes(&mut random)
            .map_err(|_| Error::Unavailable)?;
        let csrf: String = crate::hex_codec::encode(random);
        let id = sqlx::query_scalar("INSERT INTO core_wallet.affiliate_claims(owner_user,account_id,session_id,amount,csrf_digest,expires_at) VALUES($1,$2,$3,$4,$5,extract(epoch FROM clock_timestamp())::bigint+600) RETURNING id")
            .bind(actor.user).bind(actor.account).bind(actor.sid).bind(amount).bind(Sha256::digest(csrf.as_bytes()).to_vec()).fetch_one(&mut *tx).await?;
        tx.commit().await?;
        Ok(Preview { id, amount, csrf })
    }
    async fn confirm(&self, cookie: &str, id: i64, csrf: &str) -> Result<Receipt, Error> {
        if csrf.len() != 64 || !csrf.bytes().all(|b| b.is_ascii_hexdigit()) {
            return Err(Error::Invalid);
        }
        let mut tx = self.pool.begin().await?;
        let actor = BrowserAuth::oauth_identity_in_transaction(&mut tx, cookie).await?;
        let row = sqlx::query("SELECT *,expires_at>=extract(epoch FROM clock_timestamp())::bigint AS fresh FROM core_wallet.affiliate_claims WHERE id=$1 AND owner_user=$2 AND account_id=$3 FOR UPDATE")
            .bind(id).bind(actor.user).bind(actor.account).fetch_optional(&mut *tx).await?.ok_or(Error::NotFound)?;
        let digest: Vec<u8> = row.try_get("csrf_digest")?;
        if row.try_get::<String, _>("session_id")? != actor.sid
            || !bool::from(
                digest
                    .as_slice()
                    .ct_eq(Sha256::digest(csrf.as_bytes()).as_slice()),
            )
        {
            return Err(Error::Forbidden);
        }
        let amount: i64 = row.try_get("amount")?;
        if let Some(SqlJson(outcome)) = row.try_get::<Option<SqlJson<Outcome>>, _>("outcome")? {
            tx.commit().await?;
            return Ok(Receipt {
                id,
                amount_credits: amount,
                outcome,
            });
        }
        payment_policy(&mut tx).await?;
        if !row.try_get::<bool, _>("fresh")? {
            return Err(Error::Forbidden);
        }
        let request = Request {
            scope: "core-affiliate-claim".into(),
            operation_key: format!("claim-{id}"),
            actor_user_id: Some(actor.user),
            reason: "Browser-confirmed affiliate claim".into(),
            action: Action::ClaimAffiliate {
                account_id: actor.account,
                amount_units: Credits::try_from(amount).map_err(|_| Error::Invalid)?,
            },
        };
        request.validate().map_err(|_| Error::Invalid)?;
        let SqlJson(outcome): SqlJson<Outcome> =
            sqlx::query_scalar("SELECT core_billing.post_ledger($1)")
                .bind(SqlJson(request))
                .fetch_one(&mut *tx)
                .await?;
        sqlx::query("UPDATE core_wallet.affiliate_claims SET outcome=$2 WHERE id=$1")
            .bind(id)
            .bind(SqlJson(&outcome))
            .execute(&mut *tx)
            .await?;
        tx.commit().await?;
        Ok(Receipt {
            id,
            amount_credits: amount,
            outcome,
        })
    }
}
async fn payment_policy(c: &mut sqlx::PgConnection) -> Result<(), Error> {
    let confirmed: bool = sqlx::query_scalar("SELECT coalesce(bool_or(key='payment_setting.compliance_confirmed' AND value='true'),false) AND coalesce(bool_or(key='payment_setting.compliance_terms_version' AND value='v1'),false) FROM core_console.options WHERE key IN ('payment_setting.compliance_confirmed','payment_setting.compliance_terms_version')").fetch_one(c).await?;
    if confirmed {
        Ok(())
    } else {
        Err(Error::Unavailable)
    }
}

#[derive(Clone)]
struct Browser {
    store: Store,
    origin: String,
}
pub fn router(store: Store, origin: String, local_http: bool) -> Result<Router, Error> {
    let url = reqwest::Url::parse(&origin).map_err(|_| Error::Invalid)?;
    let local = local_http
        && url.scheme() == "http"
        && url
            .host_str()
            .and_then(|h| h.trim_matches(['[', ']']).parse::<std::net::IpAddr>().ok())
            .is_some_and(|ip| ip.is_loopback());
    if (url.scheme() != "https" && !local)
        || url.origin().ascii_serialization() != origin
        || !url.username().is_empty()
        || url.password().is_some()
    {
        return Err(Error::Invalid);
    }
    Ok(Router::new()
        .route("/core/v1/affiliate/claim", get(page))
        .route("/core/v1/affiliate/claim/{id}", post(confirm))
        .layer(DefaultBodyLimit::max(2048))
        .with_state(Browser { store, origin }))
}
fn cookie(headers: &HeaderMap) -> Result<&str, Error> {
    let mut value = None;
    for header in headers.get_all(header::COOKIE) {
        for item in header.to_str().map_err(|_| Error::Unauthorized)?.split(';') {
            if let Some((name, token)) = item.trim().split_once('=')
                && name == "new_api_refresh"
            {
                if value.is_some() || token.is_empty() {
                    return Err(Error::Unauthorized);
                }
                value = Some(token);
            }
        }
    }
    value.ok_or(Error::Unauthorized)
}
fn secure(mut response: Response) -> Response {
    for (name, value) in [
        (header::CACHE_CONTROL, "no-store"),
        (header::REFERRER_POLICY, "no-referrer"),
        (header::X_FRAME_OPTIONS, "DENY"),
        (
            header::CONTENT_SECURITY_POLICY,
            "default-src 'none'; script-src 'unsafe-inline'; connect-src 'self'; base-uri 'none'; frame-ancestors 'none'; form-action 'self'",
        ),
    ] {
        response.headers_mut().insert(name, value.parse().unwrap());
    }
    response
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Amount {
    amount_credits: Option<String>,
}
async fn page(
    State(s): State<Browser>,
    headers: HeaderMap,
    Query(query): Query<Amount>,
) -> Result<Response, Error> {
    let amount = query
        .amount_credits
        .map(|amount| {
            if amount.is_empty() || !amount.bytes().all(|c| c.is_ascii_digit()) {
                return Err(Error::Invalid);
            }
            amount.parse::<i64>().map_err(|_| Error::Invalid)
        })
        .transpose()?;
    let preview = match s.store.preview(cookie(&headers)?, amount).await {
        Ok(preview) => preview,
        Err(Error::Invalid) => return Ok(secure((StatusCode::BAD_REQUEST, Html("<!doctype html><meta charset=\"utf-8\"><title>Referral rewards</title><h1>Transfer unavailable</h1><p>Minimum transfer: 500000 CREDIT. Choose an amount within your available referral rewards.</p><a href=\"/wallet\">Return to wallet</a>")).into_response())),
        Err(Error::Unavailable) => return Ok(secure((StatusCode::SERVICE_UNAVAILABLE, Html("<!doctype html><meta charset=\"utf-8\"><title>Referral rewards</title><h1>Transfer unavailable</h1><p>Ask the administrator to check reward availability and the current payment confirmation.</p><a href=\"/wallet\">Return to wallet</a>")).into_response())),
        Err(error) => return Err(error),
    };
    let html = format!(
        r#"<!doctype html><meta charset="utf-8"><title>Move referral rewards</title><h1>Move referral rewards to wallet</h1><p>{} CREDIT (500000 CREDIT = 1 USD).</p><p>Minimum transfer: 500000 CREDIT. Any current wallet debt is repaid first. This does not grant a new referral reward.</p><button id="confirm">Confirm transfer</button><p id="status"></p><a href="/wallet">Return to wallet</a><script>document.getElementById('confirm').onclick=async()=>{{const button=document.getElementById('confirm');button.disabled=true;try{{const response=await fetch('/core/v1/affiliate/claim/{}',{{method:'POST',headers:{{'Content-Type':'application/json'}},body:JSON.stringify({{csrf:'{}'}})}});const data=await response.json();if(!response.ok||!['posted','rejected'].includes(data.outcome?.status))throw new Error('unknown result');document.getElementById('status').textContent=data.outcome.status==='posted'?'Transferred. Return to your wallet and refresh the balance.':'Not transferred. Check your balance and open a new preview.';}}catch(e){{button.disabled=false;document.getElementById('status').textContent='Result unknown. Retry this same confirmation to recover its receipt.';}}}};</script>"#,
        preview.amount, preview.id, preview.csrf
    );
    Ok(secure(Html(html).into_response()))
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Confirmation {
    csrf: String,
}
async fn confirm(
    State(s): State<Browser>,
    Path(id): Path<i64>,
    headers: HeaderMap,
    Json(input): Json<Confirmation>,
) -> Result<Response, Error> {
    let mut origins = headers.get_all(header::ORIGIN).iter();
    if origins.next().and_then(|v| v.to_str().ok()) != Some(s.origin.as_str())
        || origins.next().is_some()
    {
        return Err(Error::Forbidden);
    }
    let receipt = s.store.confirm(cookie(&headers)?, id, &input.csrf).await?;
    Ok(secure((StatusCode::OK, Json(receipt)).into_response()))
}
#[cfg(test)]
mod tests;

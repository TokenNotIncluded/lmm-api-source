//! Human proof is generated only by a browser carrying the native HttpOnly
//! refresh cookie. A model proposal, Go result or `confirmed=true` is not proof.
use super::{Error, context::VerifiedToolContext};
use crate::assistant_model::agent::FrozenProposal;
use axum::{
    Json, Router,
    extract::{Path, State},
    http::{HeaderMap, StatusCode, header},
    response::{IntoResponse, Redirect, Response},
    routing::get,
};
use rand::Rng;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use sqlx::{PgConnection, PgPool, Row, postgres::PgRow};
use subtle::ConstantTimeEq;
#[path = "confirmation_key_card.rs"]
mod key_card;
#[path = "confirmation_market.rs"]
pub(super) mod market;
pub const SCHEMA: &str = include_str!("../../schema/assistant_tools.sql");
const BROWSER_PREFIX: &str = "/api/user/auth/assistant/confirmations";
pub(super) fn random() -> String {
    let mut b = [0u8; 32];
    rand::rng().fill_bytes(&mut b);
    crate::hex_codec::encode(b)
}
pub(super) fn sha(bytes: &[u8]) -> Vec<u8> {
    Sha256::digest(bytes).to_vec()
}
fn allowed(name: &str, role: &str) -> bool {
    match name {
        "set_overview_greeting"
        | "create_site_issue"
        | "request_human_support"
        | "send_invitation"
        | "prepare_api_key_action"
        | "prepare_user_action"
        | "request_create_key"
        | "connect_market_tool" => true,
        "update_site_issue" => matches!(role, "admin" | "superadmin"),
        "prepare_admin_site_policy_change" => role == "superadmin",
        _ => false,
    }
}
fn canonical(value: &Value) -> Result<Vec<u8>, Error> {
    if !value.is_object() {
        return Err(Error::Invalid);
    }
    let b = serde_json::to_vec(value).map_err(|_| Error::Invalid)?;
    if b.len() > 16384 {
        return Err(Error::Invalid);
    }
    Ok(b)
}
#[derive(Serialize)]
pub struct Challenge {
    pub challenge_id: String,
    pub operation_id: String,
    pub tool_name: String,
    pub preview: Value,
    pub draft_sha256: String,
    pub arguments_sha256: String,
    pub tool_version: String,
    pub resource_kind: String,
    pub resource_id: String,
    pub resource_version: String,
    pub expires_at_unix: i64,
    pub state: String,
    pub browser_path: String,
    pub executed: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub execution_receipt: Option<Value>,
    #[serde(skip_serializing_if = "no_factor")]
    pub requires_two_factor: bool,
}
fn no_factor(required: &bool) -> bool {
    !required
}
use crate::hex_codec::encode as hex;
pub(super) fn projection(r: &PgRow) -> Result<Challenge, Error> {
    let id: String = r.try_get("id")?;
    Ok(Challenge {
        browser_path: format!("{BROWSER_PREFIX}/{id}"),
        challenge_id: id,
        operation_id: r.try_get("operation_id")?,
        tool_name: r.try_get("tool_name")?,
        preview: r.try_get("preview")?,
        draft_sha256: hex(&r.try_get::<Vec<u8>, _>("draft_sha256")?),
        arguments_sha256: hex(&r.try_get::<Vec<u8>, _>("arguments_sha256")?),
        tool_version: r.try_get("tool_version")?,
        resource_kind: r.try_get("resource_kind")?,
        resource_id: r.try_get("resource_id")?,
        resource_version: r.try_get("resource_version")?,
        expires_at_unix: r.try_get("expires_at")?,
        state: r.try_get("state")?,
        executed: r
            .try_get::<Option<Value>, _>("execution_receipt")?
            .is_some(),
        execution_receipt: r.try_get("execution_receipt")?,
        requires_two_factor: r
            .try_get::<Value, _>("preview")?
            .get("before")
            .and_then(|v| v.get("requires_two_factor"))
            .and_then(Value::as_bool)
            .unwrap_or(false),
    })
}
/// The private proof comes from the Core's actual persisted provider operation.
/// This never accepts a Go-supplied actor, tool name, grant or authority scope.
pub(crate) async fn prepare_in_transaction(
    c: &mut PgConnection,
    context: &VerifiedToolContext,
    proof: &FrozenProposal,
    draft: &Value,
) -> Result<Challenge, Error> {
    let facts = context.revalidate(c).await?;
    let family = facts.browser_session.as_deref().ok_or(Error::Forbidden)?;
    if proof.source_web_session_sid() != Some(family) || !allowed(proof.tool_name(), &facts.role) {
        return Err(Error::Forbidden);
    }
    let original: Value =
        serde_json::from_slice(proof.arguments_json()).map_err(|_| Error::Unavailable)?;
    let bytes = canonical(draft)?;
    if super::actions::is_tool(proof.tool_name()) {
        super::actions::validate_draft(c, context, proof, draft).await?;
    } else {
        super::normalization::validate(c, context, proof, &original, draft).await?;
    }
    let digest = sha(&bytes);
    let id = random();
    let now: i64 = sqlx::query_scalar("SELECT extract(epoch FROM clock_timestamp())::bigint")
        .fetch_one(&mut *c)
        .await?;
    let expiry = if super::actions::is_tool(proof.tool_name()) {
        now + 300
    } else {
        proof.expires_at_unix().min(now + 300)
    };
    if expiry <= chrono::Utc::now().timestamp() {
        return Err(Error::Forbidden);
    }
    sqlx::query("INSERT INTO core_assistant_tools.confirmations(id,operation_id,turn_id,actor_user_id,account_id,credential_id,auth_version,web_session_sid,tool_name,tool_version,arguments_sha256,draft_sha256,preview,resource_kind,resource_id,resource_version,expires_at) VALUES($1,$2,$3,$4,$5,$6,$7,$8,$9,$10,$11,$12,$13,$14,$15,$16,$17) ON CONFLICT(operation_id) DO NOTHING")
  .bind(id).bind(proof.operation_id()).bind(proof.turn_id()).bind(facts.user).bind(facts.account).bind(context.credential_id()).bind(facts.auth_version).bind(family).bind(proof.tool_name()).bind(proof.tool_version()).bind(proof.arguments_sha256().as_slice()).bind(&digest).bind(draft).bind(proof.resource_kind()).bind(proof.resource_id()).bind(proof.resource_version()).bind(expiry).execute(&mut *c).await?;
    let r = sqlx::query("SELECT * FROM core_assistant_tools.confirmations WHERE operation_id=$1")
        .bind(proof.operation_id())
        .fetch_one(c)
        .await?;
    if r.try_get::<i64, _>("credential_id")? != context.credential_id()
        || r.try_get::<Vec<u8>, _>("draft_sha256")? != digest
    {
        return Err(Error::Conflict);
    }
    projection(&r)
}
/// Resolve the operation from Core persistence; Go can provide only its opaque
/// operation ID and an exact draft, never authority fields or a FrozenProposal.
pub async fn prepare_from_operation_in_transaction(
    c: &mut PgConnection,
    context: &VerifiedToolContext,
    operation_id: &str,
    draft: &Value,
) -> Result<Challenge, Error> {
    let proof = crate::assistant_model::agent::verify_confirmation_proposal_in_transaction(
        c,
        operation_id,
        context,
    )
    .await
    .map_err(|error| match error {
        crate::assistant_model::Error::Invalid => Error::Invalid,
        crate::assistant_model::Error::Unauthorized => Error::Unauthorized,
        crate::assistant_model::Error::Forbidden => Error::Forbidden,
        crate::assistant_model::Error::Conflict => Error::Conflict,
        _ => Error::Unavailable,
    })?;
    prepare_in_transaction(c, context, &proof, draft).await
}
/// Idempotent readback for a domain adapter. It must compare its persisted exact
/// draft and apply its own business CAS/idempotency in the write transaction.
pub async fn approved_receipt_in_transaction(
    c: &mut PgConnection,
    context: &VerifiedToolContext,
    operation: &str,
    draft_sha256: &[u8],
) -> Result<Challenge, Error> {
    let facts = context.revalidate(c).await?;
    let r=sqlx::query("SELECT * FROM core_assistant_tools.confirmations WHERE operation_id=$1 AND actor_user_id=$2 AND account_id=$3 AND credential_id=$4 AND auth_version=$5 AND state='approved' AND expires_at>extract(epoch FROM clock_timestamp())::bigint")
  .bind(operation).bind(facts.user).bind(facts.account).bind(context.credential_id()).bind(facts.auth_version).fetch_optional(c).await?.ok_or(Error::Forbidden)?;
    if !bool::from(
        r.try_get::<Vec<u8>, _>("draft_sha256")?
            .as_slice()
            .ct_eq(draft_sha256),
    ) {
        return Err(Error::Conflict);
    }
    projection(&r)
}
#[derive(Clone)]
pub struct ConfirmationStore {
    pool: PgPool,
}
impl ConfirmationStore {
    pub fn new(pool: PgPool) -> Self {
        Self { pool }
    }
    pub async fn prepare(
        &self,
        original_session: &str,
        operation_id: &str,
        draft: &Value,
    ) -> Result<Challenge, Error> {
        let mut tx = self.pool.begin().await?;
        let context = super::context::from_session(&mut tx, original_session).await?;
        let challenge =
            prepare_from_operation_in_transaction(&mut tx, &context, operation_id, draft).await?;
        tx.commit().await?;
        Ok(challenge)
    }
    pub async fn receipt(
        &self,
        original_session: &str,
        operation_id: &str,
        draft_sha256: &[u8],
    ) -> Result<Challenge, Error> {
        let mut tx = self.pool.begin().await?;
        let context = super::context::from_session(&mut tx, original_session).await?;
        let receipt =
            approved_receipt_in_transaction(&mut tx, &context, operation_id, draft_sha256).await?;
        tx.commit().await?;
        Ok(receipt)
    }
}
/// This authority can only be created inside the validated browser coordinator.
/// It is not deserializable, and exposes no constructor to a domain or RPC caller.
pub(crate) struct BrowserActionProof {
    confirmation_id: String,
    operation_id: String,
    confirmed_at_unix: i64,
}
impl BrowserActionProof {
    pub(crate) fn confirmation_id(&self) -> &str {
        &self.confirmation_id
    }
    pub(crate) fn operation_id(&self) -> &str {
        &self.operation_id
    }
    pub(crate) fn confirmed_at_unix(&self) -> i64 {
        self.confirmed_at_unix
    }
}
#[derive(Clone)]
pub struct BrowserConfirmation {
    pub(super) pool: PgPool,
    pub(super) origin: String,
    pub(super) local: bool,
    identity: Option<crate::identity::IdentityStore>,
    market_runtime: Option<crate::tool_market::assistant_runtime::AssistantMarketRuntime>,
}
impl BrowserConfirmation {
    pub(crate) fn with_market_runtime(
        mut self,
        runtime: crate::tool_market::assistant_runtime::AssistantMarketRuntime,
    ) -> Self {
        self.market_runtime = Some(runtime);
        self
    }
    pub fn with_identity_store(mut self, store: crate::identity::IdentityStore) -> Self {
        self.identity = Some(store);
        self
    }
    pub fn new(pool: PgPool, origin: &str, local: bool) -> Result<Self, Error> {
        let origin =
            crate::humangrant::signature::origin(origin, local).map_err(|_| Error::Invalid)?;
        Ok(Self {
            pool,
            origin,
            local,
            identity: None,
            market_runtime: None,
        })
    }
}
pub fn router(state: BrowserConfirmation) -> Router {
    Router::new()
        .route(&format!("{BROWSER_PREFIX}/ui.js"), get(browser_script))
        .route(
            &format!("{BROWSER_PREFIX}/{{id}}"),
            get(inspect).post(decide),
        )
        .route(
            &format!("{BROWSER_PREFIX}/{{id}}/key"),
            get(key_card::inspect).post(key_card::consume),
        )
        // Existing immutable operation receipts may contain the earlier path.
        // Only navigation is retained there; every decision/reveal POST remains
        // under the native HttpOnly cookie scope and its original CSRF checks.
        .route(
            "/api/user/assistant/confirmations/ui.js",
            get(legacy_browser_script),
        )
        .route(
            "/api/user/assistant/confirmations/{id}",
            get(legacy_browser_page),
        )
        .route(
            "/api/user/assistant/confirmations/{id}/key",
            get(legacy_browser_key),
        )
        .route(
            "/api/user/auth/assistant/weekly-discounts",
            get(super::weekly::browser::history),
        )
        .route(
            "/api/user/auth/assistant/weekly-discounts/{id}",
            get(super::weekly::browser::inspect).post(super::weekly::browser::claim),
        )
        .with_state(state)
}
fn redirect_current(suffix: &str) -> Response {
    let mut response = Redirect::temporary(&format!("{BROWSER_PREFIX}/{suffix}")).into_response();
    for (name, value) in [
        (header::CACHE_CONTROL, "no-store"),
        (header::REFERRER_POLICY, "no-referrer"),
    ] {
        response
            .headers_mut()
            .insert(name, value.parse().expect("constant redirect header"));
    }
    response
}
async fn legacy_browser_script() -> Response {
    redirect_current("ui.js")
}
async fn legacy_browser_page(Path(id): Path<String>) -> Response {
    if !valid_id(&id) {
        return response(Err(Error::Invalid));
    }
    redirect_current(&id)
}
async fn legacy_browser_key(Path(id): Path<String>) -> Response {
    if !valid_id(&id) {
        return response(Err(Error::Invalid));
    }
    redirect_current(&format!("{id}/key"))
}
const HTML: &str = include_str!("confirmation.html");
const SCRIPT: &str = include_str!("confirmation.js");
fn document(content: &'static str, kind: &'static str) -> Response {
    let mut r = content.into_response();
    for (name, value) in [
        (header::CONTENT_TYPE, kind),
        (header::CACHE_CONTROL, "no-store"),
        (header::REFERRER_POLICY, "no-referrer"),
        (header::X_CONTENT_TYPE_OPTIONS, "nosniff"),
        (header::X_FRAME_OPTIONS, "DENY"),
        (
            header::CONTENT_SECURITY_POLICY,
            "default-src 'none'; script-src 'self'; connect-src 'self'; style-src 'unsafe-inline'; base-uri 'none'; frame-ancestors 'none'; form-action 'none'",
        ),
    ] {
        r.headers_mut().insert(
            name,
            value.parse().expect("constant browser response header"),
        );
    }
    r
}
async fn browser_script() -> Response {
    document(SCRIPT, "text/javascript; charset=utf-8")
}
fn accepts_html(h: &HeaderMap) -> bool {
    fn quality(raw: &str) -> Option<u16> {
        match raw {
            "1" => Some(1000),
            "0" => Some(0),
            _ => {
                let (integer, fraction) = raw.split_once('.')?;
                if fraction.is_empty()
                    || fraction.len() > 3
                    || !fraction.bytes().all(|c| c.is_ascii_digit())
                {
                    return None;
                }
                if integer == "1" && fraction.bytes().all(|c| c == b'0') {
                    Some(1000)
                } else if integer == "0" {
                    Some(fraction.parse::<u16>().ok()? * 10u16.pow((3 - fraction.len()) as u32))
                } else {
                    None
                }
            }
        }
    }
    let Some(accept) = single_header(h, "accept") else {
        return false;
    };
    if accept.len() > 2048 {
        return false;
    }
    let mut html = None;
    let mut json = None;
    for item in accept.split(',') {
        let mut parts = item.trim().split(';');
        let media = parts.next().unwrap_or("").trim();
        if !media.eq_ignore_ascii_case("text/html")
            && !media.eq_ignore_ascii_case("application/json")
        {
            continue;
        }
        let mut q = 1000;
        if let Some(parameter) = parts.next() {
            let Some((name, value)) = parameter.trim().split_once('=') else {
                return false;
            };
            if !name.eq_ignore_ascii_case("q") {
                return false;
            }
            let Some(parsed) = quality(value.trim()) else {
                return false;
            };
            q = parsed;
        }
        if parts.next().is_some() {
            return false;
        }
        let target = if media.eq_ignore_ascii_case("text/html") {
            &mut html
        } else {
            &mut json
        };
        if target.replace(q).is_some() {
            return false;
        }
    }
    html.is_some_and(|html| html > 0 && json.is_none_or(|json| html > json))
}
pub(super) fn cookie<'a>(h: &'a HeaderMap, name: &str) -> Option<&'a str> {
    let mut value = None;
    for line in h.get_all(header::COOKIE) {
        for part in line.to_str().ok()?.split(';') {
            if let Some((key, v)) = part.trim().split_once('=')
                && key == name
            {
                if value.is_some() {
                    return None;
                }
                value = Some(v)
            }
        }
    }
    value
}
fn cookie_name(s: &BrowserConfirmation, id: &str) -> String {
    format!(
        "{}lmm-ac-{}",
        if s.local && s.origin.starts_with("http:") {
            ""
        } else {
            "__Host-"
        },
        &id[..16]
    )
}
pub(super) fn response(result: Result<Value, Error>) -> Response {
    let mut r = match result {
        Ok(v) => (StatusCode::OK, Json(v)).into_response(),
        Err(e) => (
            match e {
                Error::Invalid => StatusCode::BAD_REQUEST,
                Error::Unauthorized => StatusCode::UNAUTHORIZED,
                Error::Forbidden => StatusCode::FORBIDDEN,
                Error::Conflict => StatusCode::CONFLICT,
                Error::NotFound => StatusCode::NOT_FOUND,
                Error::Unavailable => StatusCode::SERVICE_UNAVAILABLE,
            },
            Json(json!({"ok":false,"error":e})),
        )
            .into_response(),
    };
    r.headers_mut()
        .insert(header::CACHE_CONTROL, "no-store".parse().unwrap());
    r.headers_mut()
        .insert(header::REFERRER_POLICY, "no-referrer".parse().unwrap());
    r
}
pub(super) fn valid_id(id: &str) -> bool {
    id.len() == 64
        && id
            .bytes()
            .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
}
pub(super) fn single_header<'a>(headers: &'a HeaderMap, name: &str) -> Option<&'a str> {
    let mut values = headers.get_all(name).iter();
    let value = values.next()?.to_str().ok()?;
    if values.next().is_some() {
        return None;
    }
    Some(value)
}
async fn browser_row(
    tx: &mut crate::humangrant::HumanGrantTx,
    h: &HeaderMap,
    id: &str,
) -> Result<(PgRow, VerifiedToolContext), Error> {
    let (human, _) = tx
        .authenticate_human(cookie(h, "new_api_refresh"), chrono::Utc::now().timestamp())
        .await
        .map_err(|_| Error::Unavailable)?
        .ok_or(Error::Unauthorized)?;
    let context = VerifiedToolContext::from_authenticated_browser(&mut tx.tx, &human).await?;
    let row=sqlx::query("SELECT q.*,u.platform_role FROM core_assistant_tools.confirmations q JOIN core_identity.users u ON u.id=q.actor_user_id JOIN core_identity.web_sessions s ON s.sid=q.web_session_sid WHERE q.id=$1 AND q.actor_user_id=$2 AND q.web_session_sid=$3 AND q.auth_version=$4 AND q.credential_id=s.current_credential_id AND (q.expires_at>extract(epoch FROM clock_timestamp())::bigint OR (q.tool_name IN ('prepare_api_key_action','prepare_user_action','request_create_key','connect_market_tool') AND q.state='approved' AND q.execution_receipt IS NOT NULL)) FOR UPDATE OF q")
  .bind(id).bind(human.id.parse::<i64>().map_err(|_|Error::Unauthorized)?).bind(&human.session_id).bind(human.auth_version).fetch_optional(&mut *tx.tx).await?.ok_or(Error::Forbidden)?;
    let name: String = row.try_get("tool_name")?;
    if super::actions::is_tool(&name) {
        super::actions::verify_prepared(&mut tx.tx, &context, &row).await?;
    } else {
        // Recheck the real original provider proposal at the human decision, not
        // merely at prepare: a cancelled/expired/replaced operation cannot approve.
        let operation: String = row.try_get("operation_id")?;
        let proof = crate::assistant_model::agent::verify_confirmation_proposal_in_transaction(
            &mut tx.tx, &operation, &context,
        )
        .await
        .map_err(|error| match error {
            crate::assistant_model::Error::Invalid => Error::Invalid,
            crate::assistant_model::Error::Unauthorized => Error::Unauthorized,
            crate::assistant_model::Error::Forbidden => Error::Forbidden,
            crate::assistant_model::Error::Conflict => Error::Conflict,
            _ => Error::Unavailable,
        })?;
        if proof.tool_name() != row.try_get::<String, _>("tool_name")?
            || proof.arguments_sha256().as_slice()
                != row.try_get::<Vec<u8>, _>("arguments_sha256")?.as_slice()
            || proof.source_web_session_sid() != Some(human.session_id.as_str())
        {
            return Err(Error::Forbidden);
        }
    }
    if !allowed(
        &row.try_get::<String, _>("tool_name")?,
        &row.try_get::<String, _>("platform_role")?,
    ) {
        return Err(Error::Forbidden);
    }
    Ok((row, context))
}
async fn inspect(
    State(s): State<BrowserConfirmation>,
    Path(id): Path<String>,
    h: HeaderMap,
) -> Response {
    if !valid_id(&id) {
        return response(Err(Error::Invalid));
    }
    let mut set_cookie = None;
    let html = accepts_html(&h);
    let result = async {
        let mut tx = crate::humangrant::HumanGrantStore::new(s.pool.clone())
            .begin()
            .await
            .map_err(|_| Error::Unavailable)?;
        let (row, _) = browser_row(&mut tx, &h, &id).await?;
        if html {
            tx.commit().await.map_err(|_| Error::Unavailable)?;
            return Ok(Value::Null);
        }
        let projection = projection(&row)?;
        let mut v = json!({"ok":true,"confirmation":projection});
        if row.try_get::<String, _>("state")? == "pending" {
            let nonce = random();
            sqlx::query(
                "UPDATE core_assistant_tools.confirmations SET nonce_sha256=$2 WHERE id=$1",
            )
            .bind(&id)
            .bind(sha(nonce.as_bytes()))
            .execute(&mut *tx.tx)
            .await?;
            let name = cookie_name(&s, &id);
            set_cookie = Some(format!(
                "{name}={nonce}; Path=/; HttpOnly; SameSite=Strict; Max-Age=300{}",
                if s.local && s.origin.starts_with("http:") {
                    ""
                } else {
                    "; Secure"
                }
            ));
            v["csrf"] = json!(nonce);
        }
        tx.commit().await.map_err(|_| Error::Unavailable)?;
        Ok(v)
    }
    .await;
    let mut r = if html && result.is_ok() {
        document(HTML, "text/html; charset=utf-8")
    } else {
        response(result)
    };
    if let Some(c) = set_cookie {
        r.headers_mut()
            .append(header::SET_COOKIE, c.parse().unwrap());
    }
    r
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Decision {
    decision: Choice,
    two_factor_code: Option<String>,
    market_limits: Option<crate::tool_market::assistant_connect::BrowserLimits>,
}
#[derive(Deserialize)]
#[serde(rename_all = "snake_case")]
enum Choice {
    Approve,
    Reject,
}
async fn decide(
    State(s): State<BrowserConfirmation>,
    Path(id): Path<String>,
    h: HeaderMap,
    Json(mut input): Json<Decision>,
) -> Response {
    if !valid_id(&id) || single_header(&h, "origin") != Some(s.origin.as_str()) {
        return response(Err(Error::Forbidden));
    }
    let code = input.two_factor_code.take().map(zeroize::Zeroizing::new);
    let result = async {
        let name = cookie_name(&s, &id);
        let nonce = cookie(&h, &name).ok_or(Error::Forbidden)?;
        let csrf = single_header(&h, "x-lmm-assistant-csrf").ok_or(Error::Forbidden)?;
        if nonce.len() != 64 || !bool::from(nonce.as_bytes().ct_eq(csrf.as_bytes())) {
            return Err(Error::Forbidden);
        }
        // Fixed shared workspace budget. This separately committed transaction
        // counts later rejected actions and cannot approve or execute anything.
        let rate = crate::identity::assistant_workspace_rate::take(
            &s.pool,
            cookie(&h, "new_api_refresh").ok_or(Error::Unauthorized)?,
        )
        .await?;
        if !rate.allowed {
            return Ok(json!({
                "ok": false,
                "error": "confirmation_rate_limited",
                "retry_after": rate.retry_after_seconds,
                "executed": false
            }));
        }
        decide_transaction(
            &s,
            &id,
            &h,
            &input,
            code.as_ref().map(|value| value.as_str()),
            nonce,
        )
        .await
    }
    .await;
    let factor_status = result
        .as_ref()
        .ok()
        .and_then(|value| value.get("error"))
        .and_then(Value::as_str)
        .map(|error| match error {
            "two_factor_code_invalid" => StatusCode::UNPROCESSABLE_ENTITY,
            "two_factor_locked" | "confirmation_rate_limited" => StatusCode::TOO_MANY_REQUESTS,
            _ => StatusCode::OK,
        });
    let rate_retry = result
        .as_ref()
        .ok()
        .filter(|value| {
            value.get("error").and_then(Value::as_str) == Some("confirmation_rate_limited")
        })
        .and_then(|value| value.get("retry_after"))
        .and_then(Value::as_i64);
    let mut reply = response(result);
    if let Some(retry) = rate_retry {
        reply
            .headers_mut()
            .insert(header::RETRY_AFTER, retry.to_string().parse().unwrap());
    }
    if let Some(status) = factor_status {
        *reply.status_mut() = status;
    }
    // MFA outcomes are fixed codes only; neither submitted code nor factor
    // material is included in the response, stored preview, or model result.
    if reply.status().is_success() {
        // The JSON ok flag remains authoritative for retryable factor failures.
        reply
            .headers_mut()
            .insert(header::CACHE_CONTROL, "no-store".parse().unwrap());
    }
    reply
}
// Gate retries finish before any identity, MFA or action work begins.
// Business errors and ambiguous COMMIT acknowledgements never replay the body.
async fn decide_transaction(
    s: &BrowserConfirmation,
    id: &str,
    h: &HeaderMap,
    input: &Decision,
    code: Option<&str>,
    nonce: &str,
) -> Result<Value, Error> {
    let mut tx = crate::humangrant::HumanGrantStore::new(s.pool.clone())
        .begin_serializable()
        .await
        .map_err(|_| Error::Unavailable)?;
    let value = decide_in_transaction(&mut tx, s, id, h, input, code, nonce).await?;
    tx.commit().await.map_err(|_| Error::Unavailable)?;
    Ok(value)
}
async fn decide_in_transaction(
    tx: &mut crate::humangrant::HumanGrantTx,
    s: &BrowserConfirmation,
    id: &str,
    h: &HeaderMap,
    input: &Decision,
    code: Option<&str>,
    nonce: &str,
) -> Result<Value, Error> {
    let (r, context) = browser_row(tx, h, id).await?;
    let expected: Option<Vec<u8>> = r.try_get("nonce_sha256")?;
    if !expected
        .is_some_and(|expected| bool::from(expected.as_slice().ct_eq(&sha(nonce.as_bytes()))))
    {
        return Err(Error::Forbidden);
    }
    let state = match &input.decision {
        Choice::Approve => "approved",
        Choice::Reject => "rejected",
    };
    let old: String = r.try_get("state")?;
    if old != "pending" && old != state {
        return Err(Error::Conflict);
    }
    let tool: String = r.try_get("tool_name")?;
    if tool != crate::tool_market::assistant_connect::TOOL && input.market_limits.is_some() {
        return Err(Error::Invalid);
    }
    if old == "approved" && tool == crate::tool_market::assistant_connect::TOOL {
        if code.is_some() {
            return Err(Error::Invalid);
        }
        let limits = input.market_limits.as_ref().ok_or(Error::Invalid)?;
        let original: Value = r.try_get("execution_receipt")?;
        if serde_json::to_value(limits).map_err(|_| Error::Unavailable)?
            != original["after"]["browser_limits"]
        {
            return Err(Error::Conflict);
        }
    }
    if old == "pending"
        && state == "approved"
        && tool == crate::tool_market::assistant_connect::TOOL
    {
        if code.is_some() {
            return Err(Error::Invalid);
        }
        let limits = input.market_limits.as_ref().ok_or(Error::Invalid)?;
        let runtime = s.market_runtime.as_ref().ok_or(Error::Unavailable)?;
        let draft = market::draft(&r.try_get("preview")?)?;
        // The original HttpOnly browser, CSRF nonce, immutable completed
        // preparation, Session/actor, expiry and revocation checks above
        // are the only construction path for this proof.
        let stamp: i64 = sqlx::query_scalar("SELECT extract(epoch FROM clock_timestamp())::bigint")
            .fetch_one(&mut *tx.tx)
            .await?;
        let authority = BrowserActionProof {
            confirmation_id: id.to_owned(),
            operation_id: r.try_get("operation_id")?,
            confirmed_at_unix: stamp,
        };
        let execution =
            market::apply(&mut tx.tx, &context, &authority, &draft, limits, runtime).await?;
        sqlx::query(
            "UPDATE core_assistant_tools.confirmations SET state='approved',
                  confirmed_at=$3,executed_at=$3,execution_receipt=$2,
                  execution_sha256=sha256(convert_to(($2::jsonb)::text,'UTF8'))
                 WHERE id=$1 AND state='pending'",
        )
        .bind(id)
        .bind(execution)
        .bind(stamp)
        .execute(&mut *tx.tx)
        .await?;
    } else if old == "pending" && state == "approved" && super::actions::is_tool(&tool) {
        let store = s.identity.as_ref().ok_or(Error::Unavailable)?;
        let draft: crate::identity::assistant_actions::Draft =
            serde_json::from_value(r.try_get("preview")?).map_err(|_| Error::Unavailable)?;
        let operation: String = r.try_get("operation_id")?;
        let outcome = crate::identity::assistant_actions::apply(
            &mut tx.tx, store, &context, &draft, id, &operation, code,
        )
        .await;
        let execution = match outcome {
            Ok(receipt) => receipt,
            Err(crate::identity::MfaError::CodeInvalid) => {
                // The outer transaction commits only the original factor
                // attempt counter. No action, approval, or receipt is written.
                return Ok(json!({"ok":false,"error":"two_factor_code_invalid","executed":false}));
            }
            Err(crate::identity::MfaError::Locked(retry)) => {
                return Ok(
                    json!({"ok":false,"error":"two_factor_locked","retry_after":retry,"executed":false}),
                );
            }
            Err(crate::identity::MfaError::Identity(error)) => {
                return Err(Error::from(error));
            }
            Err(_) => return Err(Error::Unavailable),
        };
        sqlx::query(
            "WITH stamp AS MATERIALIZED (SELECT extract(epoch FROM clock_timestamp())::bigint AS now)
                 UPDATE core_assistant_tools.confirmations SET state='approved',
                  confirmed_at=stamp.now,executed_at=stamp.now,execution_receipt=$2,
                  execution_sha256=sha256(convert_to(($2::jsonb)::text,'UTF8'))
                 FROM stamp WHERE id=$1 AND state='pending'"
        ).bind(id).bind(execution).execute(&mut *tx.tx).await?;
    } else if old == "pending" {
        if code.is_some() || input.market_limits.is_some() {
            return Err(Error::Invalid);
        }
        sqlx::query(
            "UPDATE core_assistant_tools.confirmations SET state=$2,confirmed_at=extract(epoch FROM clock_timestamp())::bigint WHERE id=$1 AND state='pending'"
        ).bind(id).bind(state).execute(&mut *tx.tx).await?;
    }
    let row = sqlx::query("SELECT * FROM core_assistant_tools.confirmations WHERE id=$1")
        .bind(id)
        .fetch_one(&mut *tx.tx)
        .await?;
    let receipt = projection(&row)?;
    let executed = receipt.executed;
    Ok(json!({"ok":true,"receipt":receipt,"executed":executed}))
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn html_negotiation_is_explicit_and_json_remains_the_default() {
        for (accept, expected) in [
            ("text/html", true),
            (
                "text/html,application/xhtml+xml,application/xml;q=0.9,*/*;q=0.8",
                true,
            ),
            ("application/json", false),
            ("*/*", false),
            ("text/*", false),
            ("text/html;q=0", false),
            ("text/html,application/json", false),
            ("text/html;q=0.9,application/json;q=1", false),
            ("text/html;q=1,application/json;q=0.9", true),
            ("text/html;q=2", false),
            ("text/html;q=0.1234", false),
            ("text/html;q=NaN", false),
            ("text/html;q=1;q=1", false),
            ("text/html,text/html", false),
            ("text/html;charset=utf-8", false),
        ] {
            let mut h = HeaderMap::new();
            h.insert(header::ACCEPT, accept.parse().unwrap());
            assert_eq!(accepts_html(&h), expected, "{accept}");
        }
        let mut h = HeaderMap::new();
        assert!(!accepts_html(&h));
        h.append(header::ACCEPT, "text/html".parse().unwrap());
        h.append(header::ACCEPT, "application/json".parse().unwrap());
        assert!(!accepts_html(&h));
    }
    #[test]
    fn browser_page_has_no_dynamic_html_or_unsafe_submission_fallback() {
        assert!(HTML.contains("<noscript>"));
        assert!(HTML.contains("id=\"approve\" type=\"button\" disabled"));
        assert!(!HTML.contains("<form"));
        assert!(!HTML.contains("onclick="));
        assert!(!SCRIPT.contains("innerHTML"));
        assert!(!SCRIPT.contains("document.write"));
        let page = document(HTML, "text/html; charset=utf-8");
        assert_eq!(page.headers()[header::CACHE_CONTROL], "no-store");
        assert_eq!(page.headers()[header::X_FRAME_OPTIONS], "DENY");
        assert!(
            page.headers()[header::CONTENT_SECURITY_POLICY]
                .to_str()
                .unwrap()
                .contains("form-action 'none'")
        );
    }
    #[tokio::test]
    async fn historical_browser_paths_only_redirect_gets_into_the_cookie_scope() {
        use axum::{body::Body, http::Request};
        use tower::ServiceExt;

        let pool = sqlx::postgres::PgPoolOptions::new()
            .connect_lazy("postgresql://fixture@localhost:1/unused")
            .unwrap();
        let app = router(BrowserConfirmation::new(pool, "http://127.0.0.1", true).unwrap());
        let id = "a".repeat(64);
        for suffix in [id.clone(), format!("{id}/key"), "ui.js".into()] {
            let old = format!("/api/user/assistant/confirmations/{suffix}");
            let response = app
                .clone()
                .oneshot(Request::get(&old).body(Body::empty()).unwrap())
                .await
                .unwrap();
            assert_eq!(response.status(), StatusCode::TEMPORARY_REDIRECT);
            assert_eq!(
                response.headers()[header::LOCATION],
                format!("{BROWSER_PREFIX}/{suffix}")
            );
            assert_eq!(response.headers()[header::CACHE_CONTROL], "no-store");
            assert_eq!(response.headers()[header::REFERRER_POLICY], "no-referrer");
            let response = app
                .clone()
                .oneshot(Request::post(&old).body(Body::empty()).unwrap())
                .await
                .unwrap();
            assert_eq!(response.status(), StatusCode::METHOD_NOT_ALLOWED);
            assert!(!response.headers().contains_key(header::LOCATION));
        }
        let response = app
            .oneshot(
                Request::get("/api/user/assistant/confirmations/not-a-proof")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::BAD_REQUEST);
    }
    #[test]
    fn browser_inputs_do_not_accept_ambiguous_credentials_or_origins() {
        let mut headers = HeaderMap::new();
        headers.append(header::COOKIE, "new_api_refresh=one; x=1".parse().unwrap());
        assert_eq!(cookie(&headers, "new_api_refresh"), Some("one"));
        headers.append(header::COOKIE, "new_api_refresh=two".parse().unwrap());
        assert_eq!(cookie(&headers, "new_api_refresh"), None);
        headers.append(header::ORIGIN, "https://example.test".parse().unwrap());
        assert_eq!(
            single_header(&headers, "origin"),
            Some("https://example.test")
        );
        headers.append(header::ORIGIN, "https://example.test".parse().unwrap());
        assert_eq!(single_header(&headers, "origin"), None);
        assert!(
            serde_json::from_value::<Decision>(json!({"decision":"approve","confirmed":true}))
                .is_err()
        );
        assert!(serde_json::from_value::<Decision>(json!({"decision":"execute"})).is_err());
        assert!(!valid_id("../../secret"));
        assert!(!valid_id(&"A".repeat(64)));
        assert!(valid_id(&"a".repeat(64)));
    }
}

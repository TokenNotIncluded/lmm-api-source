//! Market approval belongs to the original browser, never the model payload.
use super::*;
use crate::tool_market::{
    assistant_connect::{self, BrowserLimits, MarketConsentDraft},
    assistant_runtime::AssistantMarketRuntime,
};

#[derive(Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub(crate) enum Preview {
    MarketTool { market: MarketConsentDraft },
}
pub(crate) fn draft(value: &Value) -> Result<MarketConsentDraft, Error> {
    let Preview::MarketTool { market } =
        serde_json::from_value(value.clone()).map_err(|_| Error::Unavailable)?;
    Ok(market)
}
pub(crate) fn map_error(error: crate::tool_market::Error) -> Error {
    match error {
        crate::tool_market::Error::Invalid => Error::Invalid,
        crate::tool_market::Error::Unauthorized => Error::Unauthorized,
        crate::tool_market::Error::Forbidden => Error::Forbidden,
        crate::tool_market::Error::Conflict => Error::Conflict,
        crate::tool_market::Error::Missing => Error::NotFound,
        _ => Error::Unavailable,
    }
}
async fn digest(c: &mut PgConnection, value: &Value) -> Result<String, Error> {
    Ok(
        sqlx::query_scalar("SELECT encode(sha256(convert_to(($1::jsonb)::text,'UTF8')),'hex')")
            .bind(value)
            .fetch_one(c)
            .await?,
    )
}
pub(super) async fn apply(
    c: &mut PgConnection,
    context: &VerifiedToolContext,
    authority: &BrowserActionProof,
    draft: &MarketConsentDraft,
    limits: &BrowserLimits,
    runtime: &AssistantMarketRuntime,
) -> Result<Value, Error> {
    let facts = context.revalidate(c).await?;
    // Domain helper checks pending confirmation, live policy, exact current
    // quote/resource revision and browser limits, then creates one real consent.
    let consent = assistant_connect::apply(c, context, authority, draft, limits, runtime)
        .await
        .map_err(map_error)?;
    let request_bytes = serde_json::to_vec(&consent.request).map_err(|_| Error::Unavailable)?;
    let request_sha256 = hex(Sha256::digest(&request_bytes));
    let resource_id = consent.id.clone();
    let before = serde_json::to_value(draft).map_err(|_| Error::Unavailable)?;
    let after = json!({
        "consent": consent, "browser_limits": limits, "request_sha256": request_sha256
    });
    let before_sha = digest(c, &before).await?;
    let after_sha = digest(c, &after).await?;
    let target = json!({
        "version": 1, "operation_id": authority.operation_id(),
        "confirmation_id": authority.confirmation_id(),
        "action": "connect", "resource_kind": "market_consent",
        "resource_id": resource_id, "account_id": facts.account,
        "before_sha256": before_sha, "after_sha256": after_sha, "after": after
    });
    let audit_id: i64 = sqlx::query_scalar(
        "INSERT INTO core_identity.audit(actor_user_id,credential_id,action,target)
         VALUES($1,$2,'assistant.confirmed_action',$3) RETURNING id",
    )
    .bind(facts.user)
    .bind(context.credential_id())
    .bind(&target)
    .fetch_one(c)
    .await?;
    let mut receipt = target;
    receipt["executed"] = json!(true);
    receipt["actor_user_id"] = json!(facts.user);
    receipt["audit_id"] = json!(audit_id);
    Ok(receipt)
}

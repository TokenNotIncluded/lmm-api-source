//! Validated immutable native receipts shared by usage and dashboard readers.
//! Loading limits, authorization and HTTP errors belong to the caller.
use crate::billing::{
    CREDITS_PER_USD,
    charging::{Change, LedgerCommand, Outcome, Price, Source, Usage},
};
use serde::Deserialize;
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use sqlx::{Row, postgres::PgRow};
use std::collections::HashMap;

#[derive(Debug)]
pub(crate) struct InvalidReceipt;
impl From<sqlx::Error> for InvalidReceipt {
    fn from(_: sqlx::Error) -> Self {
        Self
    }
}

pub(crate) const JS_SAFE: i64 = 9_007_199_254_740_991;

pub(crate) fn safe(value: i64) -> Result<i64, InvalidReceipt> {
    if !(0..=JS_SAFE).contains(&value) {
        return Err(InvalidReceipt);
    }
    Ok(value)
}
pub(crate) fn add(total: &mut i64, value: i64) -> Result<(), InvalidReceipt> {
    *total = safe(total.checked_add(value).ok_or(InvalidReceipt)?)?;
    Ok(())
}
pub(crate) fn usd(credits: i64) -> String {
    let millionths = i128::from(credits) * 1_000_000 / i128::from(CREDITS_PER_USD);
    format!("{}.{:06}", millionths / 1_000_000, millionths % 1_000_000)
}
#[derive(Default, Clone, Copy)]
pub(crate) struct Amounts {
    pub(crate) gross: i64,
    pub(crate) refunded: i64,
}
impl Amounts {
    pub(crate) fn accumulate(&mut self, other: Self) -> Result<(), InvalidReceipt> {
        add(&mut self.gross, other.gross)?;
        add(&mut self.refunded, other.refunded)
    }
    pub(crate) fn net(self) -> i64 {
        self.gross - self.refunded
    }
    pub(crate) fn project(self, value: &mut Value) {
        value["gross_settled_credits"] = json!(self.gross);
        value["refunded_credits"] = json!(self.refunded);
        value["net_consumption_credits"] = json!(self.net());
        value["billing_basis"] = json!("net_consumption");
        value["billing_currency"] = json!("USD");
        // This API projects consumption, not ledger entries or wallet balances.
        value["wallet_debit_supported"] = json!(false);
        value["quota"] = json!(self.net());
        value["quota_usd"] = json!(usd(self.net()));
        value["quota_money_available"] = json!(true);
        value["quota_money_basis"] = json!("captured_credits_per_usd");
    }
}
// Matches the immutable Charging::command receipt identity, never a new command
// submission. Keeping the original request ID and tag prevents refund aliasing.
fn operation_id(request_id: &str, tag: &str) -> Result<String, InvalidReceipt> {
    let encoded =
        serde_json::to_vec(&("core-charging-v1", request_id, tag)).map_err(|_| InvalidReceipt)?;
    Ok(format!("{:x}", Sha256::digest(encoded)))
}
#[derive(Deserialize)]
pub(crate) struct StoredCommand {
    pub(crate) operation_id: String,
    pub(crate) command: Value,
}
#[derive(Deserialize)]
pub(crate) struct Refund {
    pub(crate) refund_id: String,
    pub(crate) amount: i64,
}
#[derive(Deserialize)]
struct FinalEvent {
    kind: String,
    usage: Usage,
    outcome: Outcome,
}
fn decode<T: serde::de::DeserializeOwned>(value: Value) -> Result<T, InvalidReceipt> {
    serde_json::from_value(value).map_err(|_| InvalidReceipt)
}
pub(crate) struct ValidatedRequest {
    pub(crate) id: String,
    pub(crate) user: i64,
    pub(crate) key: Option<i64>,
    pub(crate) oauth_source: Option<crate::identity::grants::OAuthSource>,
    pub(crate) credential_source: &'static str,
    pub(crate) payer_is_owner: bool,
    pub(crate) sponsored: bool,
    pub(crate) at: i64,
    pub(crate) price: Price,
    pub(crate) token_name: String,
    pub(crate) state: String,
    pub(crate) outcome: Option<String>,
    pub(crate) payer_kind: String,
    pub(crate) source: Source,
    pub(crate) reserved: i64,
    pub(crate) usage: Usage,
    pub(crate) amounts: Amounts,
}
// One receipt validator for both per-request-bounded external reads and internal streams.
pub(crate) struct ReceiptValidation {
    request: ValidatedRequest,
    owner: i64,
    payer: i64,
    final_event: Option<Value>,
    receipts: HashMap<String, Change>,
    refund_total: i64,
    refund_count: usize,
}
impl ReceiptValidation {
    pub(crate) fn new(row: &PgRow) -> Result<Self, InvalidReceipt> {
        let id: String = row.try_get("id")?;
        let price: Price = decode(row.try_get("price")?)?;
        price.validate().map_err(|_| InvalidReceipt)?;
        let user = safe(row.try_get("actor_user_id")?)?;
        let key = row
            .try_get::<Option<i64>, _>("key_id")?
            .map(safe)
            .transpose()?;
        let oauth_source: Option<crate::identity::grants::OAuthSource> = row
            .try_get::<Option<Value>, _>("oauth_source")?
            .map(decode)
            .transpose()?;
        let owner: i64 = row.try_get("owner_account_id")?;
        let payer: i64 = row.try_get("payer_account_id")?;
        let image_operation: Option<String> = row.try_get("image_operation_id")?;
        let assistant_grant: Option<String> = row.try_get("assistant_grant_id")?;
        let assistant_sponsor: Option<i64> = row.try_get("assistant_sponsor_account_id")?;
        if price.image.is_some() != image_operation.is_some()
            || image_operation.as_deref().is_some_and(|op| op != id)
        {
            return Err(InvalidReceipt);
        }
        let credential_source = match (&key, &oauth_source) {
            (Some(key), None)
                if *key > 0 && assistant_grant.is_none() && assistant_sponsor.is_none() =>
            {
                "api_key"
            }
            (None, Some(source))
                if image_operation.is_none()
                    && assistant_grant.is_none()
                    && assistant_sponsor.is_none() =>
            {
                source.validate_stored().map_err(|_| InvalidReceipt)?;
                "oauth_grant"
            }
            (None, None)
                if image_operation.is_some()
                    && assistant_grant.is_none()
                    && assistant_sponsor.is_none()
                    && owner == payer =>
            {
                "drawing_session"
            }
            (None, None)
                if image_operation.is_none()
                    && assistant_grant.as_ref().is_some_and(|g| !g.is_empty())
                    && (assistant_sponsor == Some(payer)
                        || (assistant_sponsor.is_none() && owner == payer)) =>
            {
                "assistant_session"
            }
            _ => return Err(InvalidReceipt),
        };
        if let Some(source) = &oauth_source
            && (source.user_id != user
                || source.personal_account_id != owner
                || source.personal_account_id != payer)
        {
            return Err(InvalidReceipt);
        }
        let source = match row.try_get::<Option<i64>, _>("source_subscription_id")? {
            None => Source::Wallet,
            Some(id) => Source::Subscription {
                id,
                start: row.try_get("source_start")?,
                end: row.try_get("source_end")?,
            },
        };
        let state: String = row.try_get("state")?;
        let outcome: Option<String> = row.try_get("outcome")?;
        let reserved = safe(row.try_get("reserved")?)?;
        let amounts = Amounts {
            gross: safe(row.try_get("settled")?)?,
            refunded: safe(row.try_get("refunded")?)?,
        };
        let usage = Usage {
            image: row
                .try_get::<Option<Value>, _>("image_usage")?
                .map(decode)
                .transpose()?,
            input: safe(row.try_get("input_tokens")?)?,
            cached: safe(row.try_get("cached_tokens")?)?,
            output: safe(row.try_get("output_tokens")?)?,
            ..Default::default()
        };
        safe(
            usage
                .input
                .checked_add(usage.output)
                .ok_or(InvalidReceipt)?,
        )?;
        if !usage.valid()
            || (usage.image.is_some() && price.image.is_none())
            || (state == "settled" && price.image.is_some() && usage.image.is_none())
            || amounts.refunded > amounts.gross
            || amounts.gross > reserved
        {
            return Err(InvalidReceipt);
        }
        let final_event = row.try_get("final_event")?;
        Ok(Self {
            request: ValidatedRequest {
                id,
                user,
                key,
                oauth_source,
                credential_source,
                payer_is_owner: payer == owner,
                sponsored: assistant_sponsor.is_some(),
                at: safe(row.try_get("authorized_at")?)?,
                price,
                token_name: row.try_get("token_name")?,
                state,
                outcome,
                payer_kind: row.try_get("payer_kind")?,
                source,
                reserved,
                usage,
                amounts,
            },
            owner,
            payer,
            final_event,
            receipts: HashMap::new(),
            refund_total: 0,
            refund_count: 0,
        })
    }

    pub(crate) fn command(&mut self, stored: StoredCommand) -> Result<(), InvalidReceipt> {
        let r = &self.request;
        let command: LedgerCommand = decode(stored.command.clone())?;
        // Preserve the original canonical, complete receipt identity checks.
        if serde_json::to_value(&command).map_err(|_| InvalidReceipt)? != stored.command
            || stored.operation_id != command.operation_id
            || command.request_id != r.id
            || command.actor_user_id != r.user
            || command.key_id != r.key
            || command.oauth_source != r.oauth_source
            || command.owner_account_id != self.owner
            || command.payer_account_id != self.payer
            || command.price_version != r.price.version
            || command.source != r.source
            || self
                .receipts
                .insert(stored.operation_id, command.change)
                .is_some()
        {
            return Err(InvalidReceipt);
        }
        Ok(())
    }
    pub(crate) fn refund(&mut self, refund: Refund) -> Result<(), InvalidReceipt> {
        self.refund_count += 1;
        if !self.request.settled()
            || refund.amount <= 0
            || self.receipts.remove(&operation_id(
                &self.request.id,
                &format!("refund:{}", refund.refund_id),
            )?) != Some(Change::Refund {
                amount: refund.amount,
            })
        {
            return Err(InvalidReceipt);
        }
        add(&mut self.refund_total, refund.amount)
    }
    pub(crate) fn finish(self) -> Result<ValidatedRequest, InvalidReceipt> {
        let Self {
            mut request,
            mut receipts,
            refund_total,
            refund_count,
            final_event,
            ..
        } = self;
        let id = request.id.as_str();
        let state = request.state.as_str();
        let outcome = &request.outcome;
        let price = &request.price;
        let amounts = request.amounts;
        let mut usage = request.usage;
        if state == "settled" {
            let event: FinalEvent = decode(final_event.ok_or(InvalidReceipt)?)?;
            let final_outcome = match event.outcome {
                Outcome::Completed => "completed",
                Outcome::Cancelled => "cancelled",
                Outcome::Failed => "failed",
            };
            if event.kind != "final"
                || Some(final_outcome) != outcome.as_deref()
                || event.usage.input != usage.input
                || event.usage.cached != usage.cached
                || event.usage.output != usage.output
                || event.usage.image != usage.image
                || (price.tool.is_some() != event.usage.tool.is_some())
                || (price.text_request.is_some() != event.usage.text_request.is_some())
                || event
                    .usage
                    .text_request
                    .is_some_and(|receipt| receipt.outcome() != event.outcome)
                || (price.grounding.is_some() != event.usage.grounding.is_some())
                || price.cost(event.usage).map_err(|_| InvalidReceipt)? != amounts.gross
            {
                return Err(InvalidReceipt);
            }
            usage = event.usage;
            if receipts.remove(&operation_id(id, "settle")?)
                != Some(Change::Settle {
                    amount: amounts.gross,
                })
            {
                return Err(InvalidReceipt);
            }
            if refund_total != amounts.refunded {
                return Err(InvalidReceipt);
            }
        } else if amounts.gross != 0
            || amounts.refunded != 0
            || refund_count != 0
            || outcome.is_some()
            || !matches!(state, "reserved" | "streaming" | "reconcile" | "released")
        {
            return Err(InvalidReceipt);
        }
        // A refund/settle command with no matching row/terminal record is corrupt,
        // even if it uses another operation ID or claims a different amount.
        if receipts
            .values()
            .any(|c| matches!(c, Change::Refund { .. } | Change::Settle { .. }))
        {
            return Err(InvalidReceipt);
        }
        request.usage = usage;
        Ok(request)
    }
}

impl ValidatedRequest {
    pub(crate) fn settled(&self) -> bool {
        self.state == "settled"
    }
    pub(crate) fn held(&self) -> bool {
        matches!(self.state.as_str(), "reserved" | "streaming" | "reconcile")
    }
    pub(crate) fn subscription(&self) -> bool {
        matches!(self.source, Source::Subscription { .. })
    }
}

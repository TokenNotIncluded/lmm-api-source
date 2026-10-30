use super::{Error, Result};
use serde::{Deserialize, Serialize};

pub const CONTRACT: &str = "core.subscriptions.v1";
const MAX_UNITS: i64 = 1_000_000_000_000;

pub(crate) fn units(value: &str) -> Result<i64> {
    let n = value.parse::<i64>().map_err(|_| Error::Invalid)?;
    if !(1..=MAX_UNITS).contains(&n) || n.to_string() != value {
        return Err(Error::Invalid);
    }
    Ok(n)
}
pub(crate) fn operation_key(value: &str) -> Result<()> {
    if !(16..=80).contains(&value.len())
        || !value
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b"_-".contains(&b))
    {
        return Err(Error::Invalid);
    }
    Ok(())
}
pub(crate) fn token(value: &str) -> Result<()> {
    if value.len() != 64
        || !value
            .bytes()
            .all(|b| b.is_ascii_hexdigit() && !b.is_ascii_uppercase())
    {
        return Err(Error::Invalid);
    }
    Ok(())
}

/// Monetary quantities are exact integer Core credits, encoded as strings.
/// Model/group filters restrict eligibility; they never grant routing access.
#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct PlanTerms {
    pub title: String,
    pub price_units: String,
    pub duration_seconds: i64,
    /// grant, day, week, month, anniversary_month, or custom; all use UTC.
    pub reset_period: String,
    pub reset_seconds: i64,
    pub limit_units: String,
    pub models: Vec<String>,
    pub groups: Vec<String>,
    pub max_purchases_per_user: i64,
    pub enabled: bool,
}
impl PlanTerms {
    pub(crate) fn validate(&self) -> Result<()> {
        units(&self.price_units)?;
        units(&self.limit_units)?;
        if self.title.trim().is_empty()
            || self.title.len() > 160
            || self.title.chars().any(char::is_control)
            || !(60..=315_576_000).contains(&self.duration_seconds)
            || !(0..=10_000).contains(&self.max_purchases_per_user)
            || !matches!(
                self.reset_period.as_str(),
                "grant" | "day" | "week" | "month" | "anniversary_month" | "custom"
            )
            || (self.reset_period == "custom" && !(1..=315_576_000).contains(&self.reset_seconds))
            || (self.reset_period != "custom" && self.reset_seconds != 0)
        {
            return Err(Error::Invalid);
        }
        for items in [&self.models, &self.groups] {
            let unique: std::collections::BTreeSet<_> = items.iter().collect();
            if items.is_empty()
                || items.len() > 256
                || unique.len() != items.len()
                || items
                    .iter()
                    .any(|v| v.is_empty() || v.len() > 128 || v.chars().any(char::is_control))
            {
                return Err(Error::Invalid);
            }
        }
        Ok(())
    }
}
#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
pub struct Plan {
    pub id: i64,
    pub revision: i64,
    pub archived: bool,
    pub terms: PlanTerms,
}
#[derive(Clone, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct SavePlan {
    pub operation_key: String,
    pub expected_revision: Option<i64>,
    pub terms: PlanTerms,
}
#[derive(Clone, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ArchivePlan {
    pub operation_key: String,
    pub expected_revision: i64,
}
#[derive(Clone, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct QuoteRequest {
    pub operation_key: String,
    pub plan_id: i64,
    pub renew_from: Option<i64>,
}
#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
pub struct Quote {
    pub id: String,
    pub plan: Plan,
    pub starts_at: i64,
    pub ends_at: i64,
    pub expires_at: i64,
    pub renew_from: Option<i64>,
}
#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
pub struct Receipt {
    pub quote_id: String,
    pub status: String,
    pub purchase_id: Option<i64>,
    pub subscription_id: Option<i64>,
    pub journal_id: Option<i64>,
    pub amount_units: String,
    pub rejection: Option<String>,
}
#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
pub struct Purchase {
    pub id: i64,
    pub subscription_id: i64,
    pub plan: Plan,
    pub starts_at: i64,
    pub ends_at: i64,
    pub used_units: String,
    pub reserved_units: String,
    pub entitlement_active: bool,
    pub renewal_canceled: bool,
    pub canceled_at: Option<i64>,
    pub refund_journal_id: Option<i64>,
}
#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
pub struct RefundQuote {
    pub id: String,
    pub purchase_id: i64,
    pub amount_units: String,
    pub expires_at: i64,
}

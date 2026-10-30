use super::{Error, Result};
use serde::{Deserialize, Serialize};

pub const CONTRACT: &str = "core.promotions.v1";
pub(crate) fn units(value: &str) -> Result<i64> {
    let n = value.parse::<i64>().map_err(|_| Error::Invalid)?;
    if !(1..=1_000_000_000_000).contains(&n) || n.to_string() != value {
        return Err(Error::Invalid);
    }
    Ok(n)
}
pub(crate) fn key(value: &str) -> Result<()> {
    if !(16..=80).contains(&value.len())
        || !value
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b"_-".contains(&b))
    {
        return Err(Error::Invalid);
    }
    Ok(())
}
pub(crate) fn id(value: &str) -> Result<()> {
    if value.len() != 64
        || !value
            .bytes()
            .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
    {
        return Err(Error::Invalid);
    }
    Ok(())
}
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct CampaignSpec {
    pub kind: String,
    pub title: String,
    pub award_units: String,
    /// Authorized promotional issuance cap, not an external cash balance.
    pub budget_units: String,
    pub max_claims: i64,
    pub max_claims_per_user: i64,
    pub starts_at: i64,
    pub ends_at: i64,
    pub enabled: bool,
}
impl CampaignSpec {
    pub(crate) fn validate(&self) -> Result<()> {
        let amount = units(&self.award_units)?;
        let budget = units(&self.budget_units)?;
        if amount > budget
            || !matches!(self.kind.as_str(), "redemption" | "checkin" | "envelope")
            || self.title.trim().is_empty()
            || self.title.len() > 160
            || self.title.chars().any(char::is_control)
            || !(1..=1_000_000).contains(&self.max_claims)
            || !(1..=10_000).contains(&self.max_claims_per_user)
            || self.starts_at < 0
            || self.ends_at <= self.starts_at
            || self.ends_at > 253_402_300_799
            || (self.kind == "envelope" && self.max_claims_per_user != 1)
        {
            return Err(Error::Invalid);
        }
        Ok(())
    }
}
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct Campaign {
    pub id: String,
    pub revision: i64,
    pub spec: CampaignSpec,
    pub spent_units: String,
    pub reserved_units: String,
    pub claims: i64,
    pub reserved_claims: i64,
}
#[derive(Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SaveCampaign {
    pub operation_key: String,
    pub expected_revision: Option<i64>,
    pub spec: CampaignSpec,
}
#[derive(Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct IssueRequest {
    pub operation_key: String,
    pub quantity: i32,
}
#[derive(Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ClaimRequest {
    pub operation_key: String,
}
#[derive(Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RedeemRequest {
    pub operation_key: String,
    pub code: String,
}
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct IssuedCode {
    pub id: String,
    pub code: String,
}
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct Issue {
    pub id: String,
    pub campaign_id: String,
    pub codes: Vec<IssuedCode>,
}
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct Receipt {
    pub id: String,
    pub campaign_id: String,
    pub amount_units: String,
    pub journal_id: i64,
    pub claimed_at: i64,
}
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct CodeStatus {
    pub id: String,
    pub revoked: bool,
    pub claimed: bool,
}

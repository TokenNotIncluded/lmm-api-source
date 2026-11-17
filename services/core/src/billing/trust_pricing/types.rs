use super::{Error, Result};
use serde::{Deserialize, Serialize};
const MAX_AT: i64 = 253_402_300_799;
const MAX_DENOMINATOR: u64 = 1_000_000_000_000_000_000;

/// Exact bounded ratio. Construction rejects zero, increases and rounding.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(try_from = "RawRatio", into = "RawRatio")]
pub struct DiscountRatio {
    numerator: u64,
    denominator: u64,
}
#[derive(Clone, Copy, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct RawRatio {
    numerator: u64,
    denominator: u64,
}
impl TryFrom<RawRatio> for DiscountRatio {
    type Error = &'static str;
    fn try_from(r: RawRatio) -> std::result::Result<Self, Self::Error> {
        Self::new(r.numerator, r.denominator).map_err(|_| "invalid trust ratio")
    }
}
impl From<DiscountRatio> for RawRatio {
    fn from(r: DiscountRatio) -> Self {
        Self {
            numerator: r.numerator,
            denominator: r.denominator,
        }
    }
}
impl DiscountRatio {
    pub fn new(numerator: u64, denominator: u64) -> Result<Self> {
        if numerator == 0
            || denominator == 0
            || numerator > denominator
            || denominator > MAX_DENOMINATOR
        {
            return Err(Error::Invalid);
        }
        let (mut a, mut b) = (numerator, denominator);
        while b != 0 {
            (a, b) = (b, a % b);
        }
        Ok(Self {
            numerator: numerator / a,
            denominator: denominator / a,
        })
    }
    pub fn numerator(self) -> u64 {
        self.numerator
    }
    pub fn denominator(self) -> u64 {
        self.denominator
    }
}
/// Legacy discount evidence for an already admitted server-owned request.
/// Deserialization supports restart and settlement; fresh requests cannot mint
/// or accept this evidence as price authority.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct FrozenTrustPrice {
    user_id: i64,
    policy_revision: i64,
    basis_revision: i64,
    effective_level: u8,
    level_source: String,
    discount: DiscountRatio,
    evaluated_at: i64,
    next_decay_at: Option<i64>,
    evidence_digest: [u8; 32],
}
impl FrozenTrustPrice {
    pub fn validate(&self) -> Result<()> {
        if self.user_id <= 0
            || self.policy_revision <= 0
            || self.basis_revision < 0
            || self.effective_level > 6
            || !valid_at(self.evaluated_at)
            || self
                .next_decay_at
                .is_some_and(|v| !valid_at(v) || v <= self.evaluated_at)
            || self.evidence_digest == [0; 32]
            || !matches!(
                self.level_source.as_str(),
                "role" | "override" | "automatic"
            )
            || (self.effective_level >= 5) != (self.level_source == "role")
            || (self.level_source != "automatic" && self.next_decay_at.is_some())
        {
            return Err(Error::Invalid);
        }
        Ok(())
    }
    pub fn discount(&self) -> DiscountRatio {
        self.discount
    }
    pub fn user_id(&self) -> i64 {
        self.user_id
    }
    pub fn level(&self) -> u8 {
        self.effective_level
    }
    pub fn next_decay_at(&self) -> Option<i64> {
        self.next_decay_at
    }
}
fn valid_at(at: i64) -> bool {
    (0..=MAX_AT).contains(&at)
}

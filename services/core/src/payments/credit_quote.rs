use super::{Error, Result, cumulative_half_up_credit_delta};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

const MAX_RATIO_PART: i64 = 1_000_000_000;
const CREDITS_PER_USD_MINOR: i128 = 5_000;

#[derive(Clone, Debug, Deserialize, Serialize, Eq, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct CreditQuoteRatio {
    pub numerator: i64,
    pub denominator: i64,
}

impl CreditQuoteRatio {
    fn valid(&self) -> bool {
        (1..=MAX_RATIO_PART).contains(&self.numerator)
            && (1..=MAX_RATIO_PART).contains(&self.denominator)
            && self.numerator <= self.denominator
    }
}

#[derive(Clone, Debug, Deserialize, Serialize, Eq, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct CreditQuotePolicy {
    pub settlement_per_usd_numerator: i64,
    pub settlement_per_usd_denominator: i64,
    #[serde(default, deserialize_with = "deserialize_discounts")]
    pub discounts: BTreeMap<i64, CreditQuoteRatio>,
}

fn deserialize_discounts<'de, D>(
    deserializer: D,
) -> std::result::Result<BTreeMap<i64, CreditQuoteRatio>, D::Error>
where
    D: serde::Deserializer<'de>,
{
    // Untagged configurations buffer TOML map keys as strings.
    BTreeMap::<String, CreditQuoteRatio>::deserialize(deserializer)?
        .into_iter()
        .map(|(key, ratio)| {
            let credits = key.parse::<i64>().map_err(serde::de::Error::custom)?;
            if key != credits.to_string() {
                return Err(serde::de::Error::custom(
                    "discount quantity must be a canonical integer",
                ));
            }
            Ok((credits, ratio))
        })
        .collect()
}

impl CreditQuotePolicy {
    pub(crate) fn valid(&self) -> bool {
        (1..=MAX_RATIO_PART).contains(&self.settlement_per_usd_numerator)
            && (1..=MAX_RATIO_PART).contains(&self.settlement_per_usd_denominator)
            && i128::from(self.settlement_per_usd_numerator)
                <= CREDITS_PER_USD_MINOR * i128::from(self.settlement_per_usd_denominator)
            && self
                .discounts
                .iter()
                .all(|(credits, ratio)| *credits > 0 && ratio.valid())
    }

    pub(super) fn freeze(&self, credits: i64) -> Result<CreditQuote> {
        if !self.valid() || credits <= 0 {
            return Err(Error::Invalid);
        }
        let discount = self
            .discounts
            .get(&credits)
            .cloned()
            .unwrap_or(CreditQuoteRatio {
                numerator: 1,
                denominator: 1,
            });
        Ok(CreditQuote {
            kind: CreditQuoteKind::CreditQuantityV1,
            settlement_per_usd_numerator: self.settlement_per_usd_numerator,
            settlement_per_usd_denominator: self.settlement_per_usd_denominator,
            discount_numerator: discount.numerator,
            discount_denominator: discount.denominator,
        })
    }
}

#[derive(Clone, Copy, Debug, Deserialize, Serialize, Eq, PartialEq)]
pub(super) enum CreditQuoteKind {
    #[serde(rename = "credit_quantity_v1")]
    CreditQuantityV1,
}

#[derive(Clone, Debug, Deserialize, Serialize, Eq, PartialEq)]
#[serde(deny_unknown_fields)]
pub(super) struct CreditQuote {
    pub kind: CreditQuoteKind,
    pub settlement_per_usd_numerator: i64,
    pub settlement_per_usd_denominator: i64,
    pub discount_numerator: i64,
    pub discount_denominator: i64,
}

impl CreditQuote {
    pub(super) fn amount_minor(&self, credits: i64) -> Result<i64> {
        if credits <= 0
            || !(1..=MAX_RATIO_PART).contains(&self.settlement_per_usd_numerator)
            || !(1..=MAX_RATIO_PART).contains(&self.settlement_per_usd_denominator)
            || !(CreditQuoteRatio {
                numerator: self.discount_numerator,
                denominator: self.discount_denominator,
            })
            .valid()
        {
            return Err(Error::Invalid);
        }
        let numerator = i128::from(credits)
            .checked_mul(i128::from(self.settlement_per_usd_numerator))
            .and_then(|v| v.checked_mul(i128::from(self.discount_numerator)))
            .ok_or(Error::Invalid)?;
        let denominator = CREDITS_PER_USD_MINOR
            .checked_mul(i128::from(self.settlement_per_usd_denominator))
            .and_then(|v| v.checked_mul(i128::from(self.discount_denominator)))
            .ok_or(Error::Invalid)?;
        let rounded = numerator
            .checked_add(denominator / 2)
            .ok_or(Error::Invalid)?
            / denominator;
        let amount = i64::try_from(rounded).map_err(|_| Error::Invalid)?;
        if amount <= 0 {
            return Err(Error::Invalid);
        }
        Ok(amount)
    }
}

pub(super) fn refund_credit_delta(
    credits: i64,
    total_minor: i64,
    refunded_minor: i64,
    amount_minor: i64,
) -> Result<i64> {
    if credits <= 0
        || total_minor <= 0
        || refunded_minor < 0
        || amount_minor <= 0
        || refunded_minor
            .checked_add(amount_minor)
            .is_none_or(|v| v > total_minor)
    {
        return Err(Error::Invalid);
    }
    cumulative_half_up_credit_delta(
        i128::from(refunded_minor),
        i128::from(amount_minor),
        i128::from(credits),
        i128::from(total_minor),
    )
    .map_err(|_| Error::Invalid)
}

pub(super) fn refund_hold_credits(
    credits: i64,
    total_minor: i64,
    refunded_minor: i64,
    amount_minor: i64,
) -> Result<i64> {
    if credits <= 0 || total_minor <= 0 || amount_minor <= 0 || amount_minor > total_minor {
        return Err(Error::Invalid);
    }
    let numerator = i128::from(credits)
        .checked_mul(i128::from(amount_minor))
        .ok_or(Error::Invalid)?;
    let rounded = numerator
        .checked_add(i128::from(total_minor) - 1)
        .ok_or(Error::Invalid)?
        / i128::from(total_minor);
    let refunded = if refunded_minor == 0 {
        0
    } else {
        refund_credit_delta(credits, total_minor, 0, refunded_minor)?
    };
    let remaining = credits.checked_sub(refunded).ok_or(Error::Invalid)?;
    Ok(i64::try_from(rounded)
        .map_err(|_| Error::Invalid)?
        .min(remaining))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn policy() -> CreditQuotePolicy {
        CreditQuotePolicy {
            settlement_per_usd_numerator: 418769,
            settlement_per_usd_denominator: 62500,
            discounts: BTreeMap::from([(
                1_000_000,
                CreditQuoteRatio {
                    numerator: 99,
                    denominator: 100,
                },
            )]),
        }
    }

    #[test]
    fn credit_quantity_and_discount_are_frozen_before_fiat_rounding() {
        let policy = policy();
        let full = policy.freeze(500_000).unwrap();
        assert_eq!(full.amount_minor(500_000).unwrap(), 670);
        let discounted = policy.freeze(1_000_000).unwrap();
        assert_eq!(discounted.amount_minor(1_000_000).unwrap(), 1327);
        let mut changed = policy.clone();
        changed.settlement_per_usd_numerator = 7;
        changed.settlement_per_usd_denominator = 1;
        assert_eq!(full.amount_minor(500_000).unwrap(), 670);
        assert_eq!(
            changed
                .freeze(500_000)
                .unwrap()
                .amount_minor(500_000)
                .unwrap(),
            700
        );
    }

    #[test]
    fn cumulative_refunds_conserve_every_credit() {
        assert_eq!(refund_credit_delta(500_000, 670, 0, 1).unwrap(), 746);
        assert_eq!(refund_credit_delta(500_000, 670, 1, 1).unwrap(), 747);
        assert_eq!(refund_hold_credits(500_000, 670, 0, 1).unwrap(), 747);
        assert_eq!(refund_hold_credits(500_000, 670, 2, 668).unwrap(), 498_507);
        for split in 1..670 {
            let first = refund_credit_delta(500_000, 670, 0, split).unwrap();
            let last = refund_credit_delta(500_000, 670, split, 670 - split).unwrap();
            assert_eq!(first + last, 500_000);
        }
        assert_eq!(refund_credit_delta(100, 1000, 0, 1).unwrap(), 0);
        assert!(refund_credit_delta(500_000, 670, 670, 1).is_err());
    }
}

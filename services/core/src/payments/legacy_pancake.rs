//! Verify a legacy checkout without interpreting its micros as native cents.
//! This admits no proof, posts no funds, and cannot activate an archived quote.
use serde_json::Value;
use sha2::{Digest, Sha256};

use super::{Channel, Error, Result, providers, valid_id};

/// Trusted owner-reviewed settlement input, never callback or client input.
/// Currency must be explicitly bound even when the old source column was empty.
/// The archived oracle receipt alone does not authorize constructing this quote.
pub struct FrozenPancakeQuote<'a> {
    pub order_reference: &'a str,
    pub currency: &'a str,
    pub amount_micros: i64,
    pub credit_units: i64,
    pub source_store_id: Option<&'a str>,
    pub source_product_id: Option<&'a str>,
}

/// A verified wire observation, distinct from native cent-based payment proofs.
/// Old Go stored `orderId` as its transaction; native Core uses `paymentId`.
/// Preserve both: admission must reconcile these aliases before crediting.
#[derive(Debug, Eq, PartialEq)]
pub struct VerifiedLegacyPancake {
    pub event_id: String,
    pub order_id: String,
    pub payment_id: Option<String>,
    pub amount_micros: i64,
    pub credit_units: i64,
    pub payload_sha256: [u8; 32],
}

fn text<'a>(value: &'a Value, key: &str) -> Result<&'a str> {
    value.get(key).and_then(Value::as_str).ok_or(Error::Invalid)
}

fn identifier(value: &Value, key: &str) -> Result<String> {
    let value = text(value, key)?;
    if !valid_id(value) {
        return Err(Error::Invalid);
    }
    Ok(value.to_owned())
}

fn optional_status(data: &Value, key: &str, expected: &str) -> Result<()> {
    if let Some(value) = data.get(key).filter(|v| !v.is_null()) {
        let status = value.as_str().ok_or(Error::Invalid)?.trim();
        if !status.is_empty() && !status.eq_ignore_ascii_case(expected) {
            return Err(Error::Conflict);
        }
    }
    Ok(())
}

pub(super) fn decimal_micros(value: &str) -> Result<i64> {
    let value = value.trim();
    if value.is_empty() || value.len() > 32 * 1024 {
        return Err(Error::Invalid);
    }
    // Match the pinned Go decimal parser's positive decimal/scientific syntax
    // without a float or an allocation proportional to the exponent.
    let (mantissa, exponent) = match value.find(['e', 'E']) {
        Some(index) => (
            &value[..index],
            value[index + 1..]
                .parse::<i32>()
                .map_err(|_| Error::Invalid)?,
        ),
        None => (value, 0),
    };
    let mantissa = mantissa.strip_prefix('+').unwrap_or(mantissa);
    let (whole, fraction) = mantissa.split_once('.').unwrap_or((mantissa, ""));
    if (whole.is_empty() && fraction.is_empty())
        || !whole.bytes().all(|b| b.is_ascii_digit())
        || !fraction.bytes().all(|b| b.is_ascii_digit())
    {
        return Err(Error::Invalid);
    }
    let decimal_exponent = i64::from(exponent) - fraction.len() as i64;
    if i32::try_from(decimal_exponent).is_err() {
        return Err(Error::Invalid);
    }
    let digits = format!("{whole}{fraction}");
    let mut significant = digits.trim_start_matches('0');
    if significant.is_empty() {
        return Err(Error::Invalid);
    }
    let shift = decimal_exponent + 6;
    let scale = if shift < 0 {
        let remove = usize::try_from(-shift).map_err(|_| Error::Invalid)?;
        let keep = significant
            .len()
            .checked_sub(remove)
            .ok_or(Error::Invalid)?;
        if !significant[keep..].bytes().all(|b| b == b'0') {
            return Err(Error::Invalid);
        }
        significant = &significant[..keep];
        0
    } else {
        if shift > 18 {
            return Err(Error::Invalid);
        }
        shift as u32
    };
    let coefficient: i64 = significant.parse().map_err(|_| Error::Invalid)?;
    coefficient
        .checked_mul(10_i64.pow(scale))
        .filter(|&n| n > 0)
        .ok_or(Error::Invalid)
}

/// Verify original signed bytes and exact frozen terms. Current channel prices,
/// ratios and checkout amount caps do not reprice an already-created order.
pub fn verify_legacy_pancake(
    payload: &[u8],
    signature: &[u8],
    channel: &Channel,
    quote: &FrozenPancakeQuote<'_>,
    now: i64,
) -> Result<VerifiedLegacyPancake> {
    if channel.provider != "waffo_pancake" || !channel.validate() {
        return Err(Error::Unavailable);
    }
    if !valid_id(quote.order_reference) || quote.amount_micros <= 0 || quote.credit_units <= 0 {
        return Err(Error::Invalid);
    }
    if quote.currency != channel.currency
        || quote
            .source_store_id
            .is_some_and(|store| store != channel.verification.store_id)
    {
        return Err(Error::Conflict);
    }
    let event = providers::pancake_envelope(payload, signature, channel, now)?;
    let data = event
        .get("data")
        .filter(|v| v.is_object())
        .ok_or(Error::Invalid)?;
    let mode = if channel.environment == "live" {
        "prod"
    } else {
        "test"
    };
    if text(&event, "eventType")? != "order.completed"
        || text(&event, "mode")? != mode
        || text(&event, "storeId")? != channel.verification.store_id
        || text(data, "orderMerchantExternalId")? != quote.order_reference
        || !text(data, "currency")?
            .trim()
            .eq_ignore_ascii_case(quote.currency)
    {
        return Err(Error::Conflict);
    }
    // As in the pinned old SDK, chargedAmount supersedes catalogue amount.
    // A modern price snapshot without its charged amount is not paid evidence.
    let charged = match data.get("chargedAmount").filter(|v| !v.is_null()) {
        Some(value) => value.as_str().ok_or(Error::Invalid)?,
        None if data.get("listPrice").is_some_and(|v| !v.is_null()) => {
            return Err(Error::Invalid);
        }
        None => text(data, "amount")?,
    };
    if decimal_micros(charged)? != quote.amount_micros {
        return Err(Error::Conflict);
    }
    optional_status(data, "orderStatus", "completed")?;
    optional_status(data, "paymentStatus", "succeeded")?;
    if let Some(metadata) = data.get("orderMetadata").filter(|v| !v.is_null()) {
        let metadata = metadata.as_object().ok_or(Error::Invalid)?;
        for (key, value) in metadata {
            let value = if value.is_null() {
                ""
            } else {
                value.as_str().ok_or(Error::Invalid)?.trim()
            };
            let expected = match key.as_str() {
                "lmm_product_id" => quote.source_product_id,

                _ => None,
            };
            if expected.is_some_and(|expected| expected != value) {
                return Err(Error::Conflict);
            }
        }
    }
    let event_id = if event
        .get("id")
        .and_then(Value::as_str)
        .is_some_and(|id| !id.is_empty())
    {
        identifier(&event, "id")?
    } else {
        identifier(&event, "eventId")?
    };
    let payment_id = match data.get("paymentId").filter(|v| !v.is_null()) {
        None => None,
        Some(value) if value.as_str() == Some("") => None,
        Some(_) => Some(identifier(data, "paymentId")?),
    };
    Ok(VerifiedLegacyPancake {
        event_id,
        order_id: identifier(data, "orderId")?,
        payment_id,
        amount_micros: quote.amount_micros,
        credit_units: quote.credit_units,
        payload_sha256: Sha256::digest(payload).into(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn micros_are_exact_and_never_rounded() {
        for (text, expected) in [
            ("0.000001", 1),
            (" 1.234567000 ", 1_234_567),
            ("1.10", 1_100_000),
            ("2", 2_000_000),
            ("1.", 1_000_000),
            (".5", 500_000),
            ("+1", 1_000_000),
            ("1e3", 1_000_000_000),
            ("12.34567E-1", 1_234_567),
            ("10000000000000000000000000000000000000e-37", 1_000_000),
            ("9223372036854.775807", i64::MAX),
        ] {
            assert_eq!(decimal_micros(text), Ok(expected), "{text}");
        }
        for text in [
            "0",
            "-1",
            "NaN",
            "1e-7",
            "1e2147483647",
            "1e-2147483648",
            "1e2147483648",
            "1e",
            "+",
            ".",
            "1.2345671",
            "9223372036854.775808",
        ] {
            assert_eq!(decimal_micros(text), Err(Error::Invalid), "{text}");
        }
    }
}

//! The reviewed Fable ordinary-ratio compilation, never general tiered admission.
use super::{Error, typed_digest};
use crate::{
    billing::charging::Price,
    routing::{CacheCreationMode, DecimalRate, InputSemantics},
};
use num_bigint::BigUint;
use num_rational::Ratio;
use serde_json::Value;

type Fraction = Ratio<BigUint>;

fn fraction(value: &Value) -> Result<(u64, u64), Error> {
    let text = value.as_str().ok_or(Error::Invalid)?;
    let (numerator, denominator) = text.split_once('/').unwrap_or((text, "1"));
    let numerator: u64 = numerator.parse().map_err(|_| Error::Invalid)?;
    let denominator: u64 = denominator.parse().map_err(|_| Error::Invalid)?;
    if denominator == 0 {
        return Err(Error::Invalid);
    }
    Ok((numerator, denominator))
}
fn exact(value: &Value) -> Result<Fraction, Error> {
    let (n, d) = fraction(value)?;
    Ok(Ratio::new(BigUint::from(n), BigUint::from(d)))
}
fn rate(value: DecimalRate) -> Result<Fraction, Error> {
    if value.denominator == 0 {
        return Err(Error::Invalid);
    }
    Ok(Ratio::new(
        BigUint::from(value.numerator),
        BigUint::from(value.denominator),
    ))
}
/// Importer F.digest uses type-tagged canonical JSON, unlike receipt hashing.
pub(crate) fn source_digest(value: &Value) -> Result<String, Error> {
    fn tagged(value: &Value) -> Result<Value, Error> {
        Ok(match value {
            Value::Null => serde_json::json!(["null"]),
            Value::Bool(value) => serde_json::json!(["boolean", value]),
            Value::String(value) => serde_json::json!(["string", value]),
            Value::Number(value) if value.is_i64() || value.is_u64() => {
                serde_json::json!(["integer", value.to_string()])
            }
            Value::Array(values) => serde_json::json!([
                "array",
                values.iter().map(tagged).collect::<Result<Vec<_>, _>>()?
            ]),
            Value::Object(values) => {
                let mut pairs = values
                    .iter()
                    .map(|(key, value)| {
                        Ok((
                            serde_json::to_string(&serde_json::json!(["string", key]))
                                .map_err(|_| Error::Invalid)?,
                            serde_json::json!([["string", key], tagged(value)?]),
                        ))
                    })
                    .collect::<Result<Vec<_>, Error>>()?;
                pairs.sort_by(|left, right| left.0.cmp(&right.0));
                serde_json::json!([
                    "object",
                    pairs.into_iter().map(|(_, pair)| pair).collect::<Vec<_>>()
                ])
            }
            _ => return Err(Error::Invalid),
        })
    }
    typed_digest(&tagged(value)?)
}

pub(super) fn ratios(origin: &Value) -> Result<(f64, f64), Error> {
    let (mn, md) = fraction(&origin["ratios"]["model"])?;
    let (gn, gd) = fraction(&origin["ratios"]["group"])?;
    Ok((mn as f64 / md as f64, gn as f64 / gd as f64))
}

pub(super) fn verify_price(origin: &Value, price: &Price, group_ppm: u32) -> Result<(), Error> {
    price.validate()?;
    let stored = price.tiered.as_ref().ok_or(Error::Invalid)?;
    let tariff = &stored.tariff;
    if origin["format_version"] != 1
        || origin["kind"] != "legacy_ratio_decimal"
        || origin["source_mode"] != "ratio"
        || origin["source_revision"] != super::super::legacy_precharge::SOURCE_REVISION
        || origin["source_usage_contract"] != "cc52-openrouter-chat-fable-usage-v1"
        || origin["payload_policy"] != "legacy_openrouter_chat_usage_v1"
        || origin["input_basis"] != "projected-disjoint-sum"
        || origin["cost_inference"] != "cc52_custom_ModelRatio_not_in_default_map"
        || origin["counter_projection"] != "cc52-DTO-alias-TTL-semantic-priority-v1"
        || origin["model"].as_str() != Some(price.model.as_str())
        || origin["group"].as_str() != Some(price.group.as_str())
        || source_digest(origin)? != tariff.source_digest
        || tariff.revision != 3
        || tariff.tiers.len() != 1
        || tariff.tiers[0].up_to_input_tokens.is_some()
        || tariff.calendar.is_some()
        || stored.calendar.is_some()
        || !tariff.uses_cache_read
        || !tariff.uses_cache_write
        || !tariff.uses_cache_write_1h
        || tariff.uses_cache_write_12h
        || stored.input_semantics != InputSemantics::AnthropicText
        || stored.cache_creation_mode != Some(CacheCreationMode::GenericRemainderV1)
        || stored.group_multiplier_ppm != group_ppm
        || stored.speed_multiplier_ppm != 1_000_000
        || stored.credits_per_usd != 500_000
        || price.rounding.is_some()
        || price.audio.is_some()
        || price.text_request.is_some()
        || price.tool.is_some()
        || price.grounding.is_some()
        || price.image.is_some()
        || price.upstream_cost_group_ppm.is_some()
    {
        return Err(Error::Conflict);
    }
    let model = exact(&origin["ratios"]["model"])?;
    let completion = exact(&origin["ratios"]["completion"])?;
    let cache = exact(&origin["ratios"]["cache"])?;
    let group = exact(&origin["ratios"]["group"])?;
    let creation = exact(&origin["creation_ratio"])?;
    let creation_1h = exact(&origin["creation_ratio_1h"])?;
    let zero = Ratio::from_integer(BigUint::from(0_u8));
    let integer = |n: u64| Ratio::from_integer(BigUint::from(n));
    if model <= zero
        || completion <= zero
        || cache <= zero
        || group != Ratio::new(BigUint::from(group_ppm), BigUint::from(1_000_000_u64))
        || creation != Ratio::new(BigUint::from(5_u8), BigUint::from(4_u8))
        || creation_1h != integer(2)
    {
        return Err(Error::Conflict);
    }
    let rates = tariff.tiers[0].rates;
    let input = model * integer(2);
    if rate(rates.input)? != input
        || rate(rates.output)? != &input * completion
        || rate(rates.cache_read)? != &input * cache
        || rate(rates.cache_write)? != &input * creation
        || rate(rates.cache_write_1h.ok_or(Error::Invalid)?)? != input * creation_1h
        || rates.cache_write_12h.is_some()
    {
        return Err(Error::Conflict);
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    #[test]
    fn importer_source_digest_uses_type_tagged_canonical_json() {
        let origin = serde_json::json!({"kind":"legacy_ratio_decimal", "ratios":{"model":"3/2","group":"1/25"}, "format_version":1,"enabled":true,"missing":null});
        // scripts/migrate-legacy-financial.py F.digest, independently evaluated.
        assert_eq!(
            super::source_digest(&origin).unwrap(),
            "3ee0dddff3ddb87aff2d942d4c6e1318c96ee7421c9e4aa4cb375a2c5a545b43"
        );
    }
}

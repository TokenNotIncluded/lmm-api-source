//! Sparse base-price quotes from an explicitly selected OpenRouter-format channel.
use serde::Serialize;
use serde_json::{Map, Value};
use std::collections::{BTreeMap, BTreeSet};

/// Prices use decimal USD per million tokens or per named operation (request,
/// image, web_search). An absent key has no quote; an explicit "0" is free.
pub type Rates = BTreeMap<String, String>;
type Parse<T> = Result<T, &'static str>;

#[derive(Debug, Serialize, PartialEq, Eq)]
pub struct ChannelQuoteSource {
    pub kind: &'static str,
    pub channel_id: u64,
    pub upstream_model: String,
}

#[derive(Debug, Serialize, PartialEq, Eq)]
pub struct ContextTier {
    /// This tier applies strictly above the threshold and through the next bound.
    pub above_input_tokens: u64,
    /// Overrides relative to the base rates. Absent fields inherit the base.
    pub rates: Rates,
}

#[derive(Debug, Serialize, PartialEq, Eq)]
pub struct SourceQuote {
    pub model: String,
    pub source: ChannelQuoteSource,
    pub currency: &'static str,
    pub source_token_unit: &'static str,
    pub token_unit: &'static str,
    pub rates: Rates,
    /// None means the source supplied no context schedule. Keep existing tiers.
    pub context_tiers: Option<Vec<ContextTier>>,
}

// Canonical meter and whether its upstream price is denominated per token.
fn meter(name: &str) -> Option<(&'static str, bool)> {
    Some(match name {
        "prompt" => ("input", true),
        "completion" => ("output", true),
        "input_cache_read" => ("cache_read", true),
        "input_cache_write" => ("cache_write", true),
        "input_cache_write_1h" => ("cache_write_1h", true),
        "internal_reasoning" => ("reasoning", true),
        "audio" => ("input_audio", true),
        "audio_output" => ("output_audio", true),
        "input_audio_cache" => ("cache_read_audio", true),
        "image_token" => ("image_token", true),
        "request" => ("request", false),
        "web_search" => ("web_search", false),
        "image" => ("image", false),
        "image_output" => ("image_output", false),
        _ => return None,
    })
}

fn decimal(text: &str, shift: i32) -> Parse<String> {
    const BAD: &str = "unrepresentable_source_price";
    if text.is_empty() || text.len() > 128 {
        return Err(BAD);
    }
    let (mantissa, exp) = match text.split_once(['e', 'E']) {
        Some((m, e)) => (m, e.parse::<i32>().map_err(|_| BAD)?),
        None => (text, 0),
    };
    let (whole, fraction) = match mantissa.split_once('.') {
        Some((_, "")) => return Err(BAD),
        Some(pair) => pair,
        None => (mantissa, ""),
    };
    if whole.is_empty()
        || !whole
            .bytes()
            .chain(fraction.bytes())
            .all(|c| c.is_ascii_digit())
    {
        return Err(BAD);
    }
    let digits = format!("{whole}{fraction}");
    if digits.bytes().all(|c| c == b'0') {
        return Ok("0".into());
    }
    // Bound expansion of untrusted exponent notation before allocating.
    let point = whole.len() as i64 + i64::from(exp) + i64::from(shift);
    if !(-128..=128).contains(&point) {
        return Err(BAD);
    }
    let rendered = if point <= 0 {
        format!("0.{}{digits}", "0".repeat((-point) as usize))
    } else if point as usize >= digits.len() {
        format!("{digits}{}", "0".repeat(point as usize - digits.len()))
    } else {
        let (int, frac) = digits.split_at(point as usize);
        format!("{int}.{frac}")
    };
    let (int, frac) = rendered.split_once('.').unwrap_or((&rendered, ""));
    let int = int.trim_start_matches('0');
    let int = if int.is_empty() { "0" } else { int };
    let frac = frac.trim_end_matches('0');
    Ok(if frac.is_empty() {
        int.to_owned()
    } else {
        format!("{int}.{frac}")
    })
}

fn rates(row: &Map<String, Value>, metadata: &str) -> Parse<Rates> {
    let mut prices = Rates::new();
    for (field, value) in row {
        if field == metadata || value.is_null() {
            continue;
        }
        let (name, per_token) = meter(field).ok_or("unsupported_source_pricing")?;
        let price = value.as_str().ok_or("invalid_source_price")?;
        prices.insert(name.into(), decimal(price, if per_token { 6 } else { 0 })?);
    }
    Ok(prices)
}

/// Use only with the selected channel's explicitly known USD/OpenRouter price schema.
pub fn channel_models(
    catalog: &Value,
    channel_id: u64,
    model: &str,
    upstream_model: &str,
) -> Parse<Option<SourceQuote>> {
    let entries = catalog
        .get("data")
        .and_then(Value::as_array)
        .ok_or("invalid_models_response")?;
    let mut matches = entries
        .iter()
        .filter(|m| m.get("id").and_then(Value::as_str) == Some(upstream_model));
    let Some(entry) = matches.next() else {
        return Ok(None);
    };
    if matches.next().is_some() {
        return Err("duplicate_source_model");
    }
    let Some(cost) = entry.get("pricing").filter(|v| !v.is_null()) else {
        return Ok(None);
    };
    let prices = rates(cost.as_object().ok_or("invalid_source_price")?, "overrides")?;
    let mut rules = Vec::new();
    let tiers = match cost.get("overrides") {
        None | Some(Value::Null) => None,
        Some(value) => Some(value.as_array().ok_or("invalid_context_tiers")?),
    };
    if let Some(tiers) = tiers {
        for tier in tiers {
            let bound = tier
                .get("min_prompt_tokens")
                .and_then(Value::as_u64)
                .filter(|n| *n > 0)
                .ok_or("unsupported_context_condition")?;
            let overrides = rates(
                tier.as_object().ok_or("invalid_context_tier")?,
                "min_prompt_tokens",
            )?;
            if overrides.is_empty() {
                return Err("invalid_context_tier");
            }
            rules.push((bound, overrides));
        }
    }
    let bounds: BTreeSet<_> = rules.iter().map(|(n, _)| *n).collect();
    // Three thresholds plus the base are the four tiers accepted by Core.
    if bounds.len() > 3 {
        return Err("unsupported_context_tier_count");
    }
    let context = tiers.map(|_| {
        bounds
            .into_iter()
            .map(|above_input_tokens| {
                let mut rates = Rates::new();
                // OpenRouter resolves matching overrides per key, last listed wins.
                for (_, fields) in rules
                    .iter()
                    .filter(|(bound, _)| *bound <= above_input_tokens)
                {
                    rates.extend(fields.clone());
                }
                ContextTier {
                    above_input_tokens,
                    rates,
                }
            })
            .collect()
    });
    if prices.is_empty() && context.as_ref().is_none_or(Vec::is_empty) {
        return Ok(None);
    }
    Ok(Some(SourceQuote {
        model: model.into(),
        source: ChannelQuoteSource {
            kind: "channel",
            channel_id,
            upstream_model: upstream_model.into(),
        },
        currency: "USD",
        source_token_unit: "per_token",
        token_unit: "per_million_tokens",
        rates: prices,
        context_tiers: context,
    }))
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn channel_converts_token_rates_only_and_observes_override_precedence() {
        let catalog = json!({"data":[{"id":"m","pricing":{"prompt":"6.5e-8","request":"0.015","input_cache_write_1h":"0.000005","overrides":[
            {"min_prompt_tokens":300,"completion":"1e-6"}, {"min_prompt_tokens":200,"completion":"2e-6"},
            {"min_prompt_tokens":300,"prompt":"0.001"}
        ]}}]});
        let quote = channel_models(&catalog, 31, "m", "m").unwrap().unwrap();
        assert_eq!(quote.rates["input"], "0.065");
        assert_eq!(quote.rates["request"], "0.015");
        assert_eq!(quote.rates["cache_write_1h"], "5");
        let tiers = quote.context_tiers.unwrap();
        assert_eq!(tiers[0].above_input_tokens, 200);
        assert_eq!(tiers[1].above_input_tokens, 300);
        assert_eq!(tiers[1].rates["output"], "2");
        assert_eq!(tiers[1].rates["input"], "1000");
    }

    #[test]
    fn absent_pricing_never_generates_zero_or_substitutes_another_model() {
        let catalog = json!({"data":[{"id":"a","pricing":{"prompt":"0"}},{"id":"b"}]});
        assert!(channel_models(&catalog, 31, "b", "b").unwrap().is_none());
        assert!(channel_models(&catalog, 31, "c", "c").unwrap().is_none());
        let a = channel_models(&catalog, 31, "a", "a").unwrap().unwrap();
        assert_eq!(
            serde_json::to_value(&a).unwrap(),
            json!({
                "model":"a", "source":{"kind":"channel","channel_id":31,"upstream_model":"a"},
                "currency":"USD","source_token_unit":"per_token","token_unit":"per_million_tokens",
                "rates":{"input":"0"}, "context_tiers":null
            })
        );
        assert_eq!(a.rates.len(), 1);
        assert_eq!(a.rates["input"], "0");
        assert!(a.context_tiers.is_none());
        assert!(channel_models(&json!({"data":[{"id":"a"},{"id":"a"}]}), 31, "a", "a").is_err());
    }

    #[test]
    fn unrepresentable_conditions_and_invalid_decimals_preserve_the_old_quote() {
        for value in ["-1", "nan", "1.", "1e100000"] {
            assert!(decimal(value, 6).is_err());
        }
        let catalog = json!({"data":[{"id":"m","pricing":{"prompt":"1","overrides":[{"min_prompt_tokens":200,"utc_start":1200,"prompt":"2"}]}}]});
        assert!(channel_models(&catalog, 31, "m", "m").is_err());
    }
}

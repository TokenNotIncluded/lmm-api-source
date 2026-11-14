//! Owner-selected fixed cc52 OpenRouter DTO projection. Not a native parser.
use super::{CacheCreationDetails, RelayError, Usage};
use crate::routing::CacheCreationMode;
use serde_json::Value;

fn signed(v: &Value, key: &str) -> Result<i64, RelayError> {
    match v.get(key) {
        None | Some(Value::Null) => Ok(0),
        Some(value) => value
            .as_i64()
            .ok_or(RelayError::Protocol("invalid legacy usage counter")),
    }
}
fn object(v: &Value, key: &str) -> Result<(), RelayError> {
    if v.get(key).is_some_and(|v| !v.is_null() && !v.is_object()) {
        return Err(RelayError::Protocol("invalid legacy usage details"));
    }
    Ok(())
}
fn text<'a>(v: &'a Value, key: &str) -> Result<&'a str, RelayError> {
    match v.get(key) {
        None | Some(Value::Null) => Ok(""),
        Some(value) => value
            .as_str()
            .ok_or(RelayError::Protocol("invalid legacy usage semantic")),
    }
}
fn add(a: u64, b: u64) -> Result<u64, RelayError> {
    a.checked_add(b)
        .filter(|v| *v <= i64::MAX as u64)
        .ok_or(RelayError::Protocol("legacy usage overflow"))
}
fn positive(v: i64) -> u64 {
    v.max(0) as u64
}

pub(crate) fn project(original: &Value) -> Result<Usage, RelayError> {
    project_with_source(original).map(|(usage, _)| usage)
}

pub(crate) fn tiered_source(
    original: &Value,
) -> Result<crate::billing::legacy_tiered::RawUsage, RelayError> {
    project_with_source(original)?
        .1
        .ok_or(RelayError::Protocol("missing authoritative legacy usage"))
}

fn project_with_source(
    original: &Value,
) -> Result<(Usage, Option<crate::billing::legacy_tiered::RawUsage>), RelayError> {
    if original.is_null() {
        return Ok((Usage::default(), None));
    }
    if !original.is_object() {
        return Err(RelayError::Protocol("invalid legacy usage object"));
    }
    object(original, "billing_usage")?;
    let billing = &original["billing_usage"];
    for key in ["openai_usage", "claude_usage", "gemini_usage_metadata"] {
        object(billing, key)?;
    }
    let source = text(billing, "source")?.trim();
    let semantic = text(billing, "semantic")?.trim();
    let openai = billing["openai_usage"].is_object()
        && (["oai_chat", "oai_responses"]
            .iter()
            .any(|s| source.eq_ignore_ascii_case(s))
            || semantic.eq_ignore_ascii_case("openai"));
    let anthropic = !openai
        && billing["claude_usage"].is_object()
        && (source.eq_ignore_ascii_case("claude_messages")
            || semantic.eq_ignore_ascii_case("anthropic"));
    // This is the reviewed Fable Chat source, not Gemini envelopes or audio.
    if !openai
        && !anthropic
        && billing["gemini_usage_metadata"].is_object()
        && (source.eq_ignore_ascii_case("gemini_chat") || semantic.eq_ignore_ascii_case("gemini"))
    {
        return Err(RelayError::Unsupported(
            "legacy Chat Gemini billing envelope",
        ));
    }
    let v = if openai {
        &billing["openai_usage"]
    } else if anthropic {
        &billing["claude_usage"]
    } else {
        original
    };
    for key in [
        "prompt_tokens_details",
        "input_tokens_details",
        "completion_tokens_details",
        "output_tokens_details",
    ] {
        object(v, key)?;
    }
    if anthropic {
        object(v, "cache_creation")?;
    }
    // Never convert an absent primary count into an authoritative zero. The
    // fixed old text-estimation fallback is outside this measured-usage slice.
    let primary = if anthropic {
        "input_tokens"
    } else {
        "prompt_tokens"
    };
    let output_key = if anthropic {
        "output_tokens"
    } else {
        "completion_tokens"
    };
    if (v.get(primary).is_none_or(Value::is_null)
        && (!openai || v.get("input_tokens").is_none_or(Value::is_null)))
        || (v.get(output_key).is_none_or(Value::is_null)
            && (!openai || v.get("output_tokens").is_none_or(Value::is_null)))
    {
        return Err(RelayError::Protocol("missing authoritative legacy usage"));
    }
    let d = &v["prompt_tokens_details"];
    let fallback = &v["input_tokens_details"];
    let mut prompt = signed(
        v,
        if anthropic {
            "input_tokens"
        } else {
            "prompt_tokens"
        },
    )?;
    let mut output = signed(
        v,
        if anthropic {
            "output_tokens"
        } else {
            "completion_tokens"
        },
    )?;
    let mut cached = signed(
        if anthropic { v } else { d },
        if anthropic {
            "cache_read_input_tokens"
        } else {
            "cached_tokens"
        },
    )?;
    let mut a = signed(
        if anthropic { v } else { d },
        if anthropic {
            "cache_creation_input_tokens"
        } else {
            "cached_creation_tokens"
        },
    )?;
    let mut b = if anthropic {
        0
    } else {
        signed(d, "cache_write_tokens")?
    };
    if openai {
        // Preserve zero-only fallback (negative is not zero) and positive aliases.
        if prompt == 0 && signed(v, "input_tokens")? > 0 {
            prompt = signed(v, "input_tokens")?;
        }
        if output == 0 && signed(v, "output_tokens")? > 0 {
            output = signed(v, "output_tokens")?;
        }
        if cached == 0 && signed(fallback, "cached_tokens")? > 0 {
            cached = signed(fallback, "cached_tokens")?;
        }
        if a == 0 && signed(fallback, "cached_creation_tokens")? > 0 {
            a = signed(fallback, "cached_creation_tokens")?;
        }
        if b == 0 && signed(fallback, "cache_write_tokens")? > 0 {
            b = signed(fallback, "cache_write_tokens")?;
        }
        if cached == 0 && signed(v, "prompt_cache_hit_tokens")? > 0 {
            cached = signed(v, "prompt_cache_hit_tokens")?;
        }
    }
    let mut five = signed(v, "claude_cache_creation_5_m_tokens")?;
    let mut hour = signed(v, "claude_cache_creation_1_h_tokens")?;
    if anthropic {
        let f = signed(&v["cache_creation"], "ephemeral_5m_input_tokens")?;
        let h = signed(&v["cache_creation"], "ephemeral_1h_input_tokens")?;
        if f != 0 {
            five = f;
        }
        if h != 0 {
            hour = h;
        }
    }
    use crate::billing::legacy_tiered::{CreationSemantics, PromptSemantics, RawUsage};
    let source_counts = RawUsage {
        prompt,
        completion: output,
        cached,
        creation_legacy: a,
        creation_openai: b,
        creation_5m: five,
        creation_1h: hour,
        ..RawUsage::default()
    };
    let (prompt, output, cached, five, hour) = (
        positive(prompt),
        positive(output),
        positive(cached),
        positive(five),
        positive(hour),
    );
    let used_semantic = if openai {
        "openai"
    } else if anthropic {
        "anthropic"
    } else {
        text(v, "usage_semantic")?
    };
    let used_source = if openai {
        text(billing, "source")?
    } else if anthropic {
        "claude_messages"
    } else {
        text(v, "usage_source")?
    };
    let legacy_derived =
        used_semantic.is_empty() && used_source.is_empty() && (five > 0 || hour > 0);
    let created = positive(a).max(positive(b)).max(add(five, hour)?);
    let uncached = if legacy_derived {
        prompt
    } else {
        prompt.saturating_sub(cached).saturating_sub(created)
    };
    let split = used_semantic == "anthropic" || legacy_derived;
    // Ordinary OpenAI prices all creation at the generic rate even with TTLs.
    let (priced_five, priced_hour) = if split { (five, hour) } else { (0, 0) };
    for details in [
        d,
        fallback,
        &v["completion_tokens_details"],
        &v["output_tokens_details"],
    ] {
        if signed(details, "audio_tokens")? != 0 {
            return Err(RelayError::Unsupported("legacy audio usage"));
        }
    }
    let usage = Usage {
        legacy_openrouter_raw: Some(Box::new(super::PrivateUsageEvidence(original.clone()))),
        input_tokens: Some(add(add(uncached, cached)?, created)?),
        uncached_input_tokens: Some(uncached),
        output_tokens: Some(output),
        cached_input_tokens: Some(cached),
        cache_creation_input_tokens: Some(created),
        cache_creation_details: Some(CacheCreationDetails {
            ephemeral_5m_input_tokens: Some(priced_five),
            ephemeral_1h_input_tokens: Some(priced_hour),
        }),
        cache_creation_mode: Some(CacheCreationMode::GenericRemainderV1),
        ..Usage::default()
    };
    usage.validate()?;
    // Expression billing uses IsClaudeUsageSemantic, independently of the
    // ratio-only legacy-derived heuristic used by the disjoint projection.
    let source = RawUsage {
        prompt_semantics: if used_semantic == "anthropic" {
            PromptSemantics::ClaudeText
        } else {
            PromptSemantics::Inclusive
        },
        creation_semantics: if used_semantic == "anthropic" {
            CreationSemantics::Anthropic
        } else {
            CreationSemantics::Aggregate
        },
        ..source_counts
    };
    Ok((usage, Some(source)))
}

#[cfg(test)]
#[path = "legacy_openrouter_usage_tests.rs"]
mod tests;

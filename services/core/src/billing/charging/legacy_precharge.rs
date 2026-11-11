//! Fixed cc52 precharge arithmetic. These estimates are never usage bounds.
//!
//! No wallet, clock, input tokenizer, admission or settlement authority lives
//! here. The host must separately bind the effective source options, source
//! prompt estimator and selected group to its immutable prepared contract.

pub(crate) mod openai_tokens;
pub(crate) mod responses_text;

pub(crate) const SOURCE_REVISION: &str = "cc52aa195c6f2a01d703a1a407be73c31b89867d";
pub(crate) const TIERED_DEFAULT_OUTPUT: u32 = 8192;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Error {
    Invalid,
    QuotaRange,
}

/// Effective options, not inferred from missing database rows or environment.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct SourceOptions {
    pub preconsumed_tokens: u32,
    pub count_token: bool,
    pub get_media_token: bool,
    pub get_media_token_not_stream: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct Estimate {
    pub prompt_tokens: u32,
    pub completion_tokens: u32,
    /// Preserve the actual intermediate f64 for the independent Go oracle.
    pub before_round_bits: u64,
    pub hold: i32,
}

fn nonnegative(bits: u64) -> Result<f64, Error> {
    let value = f64::from_bits(bits);
    if !value.is_finite() || value < 0.0 {
        return Err(Error::Invalid);
    }
    Ok(value)
}

// common.QuotaFromFloatStrict checks the unrounded value then truncates.
// Settlement half-away/min1 and Native ceil are separate contracts.
fn truncate_strict(value: f64) -> Result<i32, Error> {
    if !value.is_finite() || value < 0.0 || value > i32::MAX as f64 {
        return Err(Error::QuotaRange);
    }
    Ok(value as i32)
}

/// Ordinary text ratio only: no completion-ratio multiplier in old precharge.
/// `source_free_model` is the reviewed source FreeModel result, not guessed
/// from a zero ratio or an omitted usage counter. No other-ratio/fee shortcut.
pub(crate) fn ordinary_text(
    options: SourceOptions,
    source_prompt_tokens: u32,
    explicit_max_tokens: Option<u32>,
    model_ratio_bits: u64,
    selected_group_ratio_bits: u64,
    source_free_model: bool,
) -> Result<Estimate, Error> {
    let model = nonnegative(model_ratio_bits)?;
    let group = nonnegative(selected_group_ratio_bits)?;
    let prompt = if options.count_token {
        source_prompt_tokens
    } else {
        0
    };
    let completion = explicit_max_tokens.unwrap_or(0);
    let tokens = u64::from(prompt.max(options.preconsumed_tokens))
        .checked_add(u64::from(completion))
        .ok_or(Error::QuotaRange)?;
    // price.go: ratio := modelRatio * groupRatio; float64(tokens) * ratio.
    let ratio = model * group;
    let value = tokens as f64 * ratio;
    let hold = if source_free_model {
        0
    } else {
        truncate_strict(value)?.max(1)
    };
    Ok(Estimate {
        prompt_tokens: prompt,
        completion_tokens: completion,
        before_round_bits: value.to_bits(),
        hold,
    })
}

/// Ordinary text tiered mode's completion estimate. A source zero/absent max
/// selects 8192 only for a paid selected group. This is not a provider limit.
pub(crate) fn tiered_text_completion(
    explicit_max_tokens: Option<u32>,
    selected_group_ratio_bits: u64,
) -> Result<u32, Error> {
    let group = nonnegative(selected_group_ratio_bits)?;
    Ok(match explicit_max_tokens {
        Some(value) if value != 0 => value,
        _ if group != 0.0 => TIERED_DEFAULT_OUTPUT,
        _ => 0,
    })
}

/// refreshTieredBillingGroup multiplies the ORIGINAL pre-group estimate by
/// the newly selected group, then rounds. Never multiply the previous hold.
pub(crate) fn refresh_tiered_group(
    estimated_before_group_bits: u64,
    selected_group_ratio_bits: u64,
) -> Result<(u64, i32), Error> {
    let base = nonnegative(estimated_before_group_bits)?;
    let group = nonnegative(selected_group_ratio_bits)?;
    let value = base * group;
    let rounded = value.round();
    let quota = truncate_strict(rounded)?;
    // Group refresh stores zero literally; PrepareTieredBillingForSelectedGroup
    // skips funding for a zero group. Only PreConsumeBilling applies min1.
    Ok((value.to_bits(), quota))
}

/// Exact common.GetEnvOrDefaultBool + strconv.ParseBool semantics. Does not
/// read the local process environment or infer the source process's setting.
pub(crate) fn source_bool(value: Option<&str>, default: bool) -> bool {
    match value {
        None | Some("") => default,
        Some("1" | "t" | "T" | "TRUE" | "true" | "True") => true,
        Some("0" | "f" | "F" | "FALSE" | "false" | "False") => false,
        Some(_) => default,
    }
}

#[cfg(test)]
#[path = "legacy_precharge_tests.rs"]
mod tests;

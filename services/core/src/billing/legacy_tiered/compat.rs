// SPDX-License-Identifier: AGPL-3.0-or-later
use super::{
    CacheDetails, Error, Evaluation, Financial, Program, RawUsage, SettlementClock, TokenParams,
    Variable, project_source, quota_round_checked,
};

/// Ordinary text only. Completion is the already selected cc52 estimate
/// (including the paid-group fallback, if applicable). A calendar expression
/// uses the caller's admission clock here, not the terminal clock.
pub(crate) fn estimate_text(
    program: &Program,
    prompt: u32,
    completion: u32,
    financial: Financial,
    clock: Option<SettlementClock>,
) -> Result<Evaluation, Error> {
    // cc52 modelPriceHelperTieredForKey builds estimates directly; there
    // is no measured-usage projection and its classified cache is zero.
    let mut params = TokenParams {
        classified: Some(CacheDetails::default()),
        ..TokenParams::default()
    };
    params.values[Variable::P as usize] = f64::from(prompt);
    params.values[Variable::Len as usize] = f64::from(prompt);
    params.values[Variable::C as usize] = f64::from(completion);
    let mut result = program.evaluate_f64(&params, financial, clock)?;
    // Estimation uses QuotaRoundStrict. Any later preconsume minimum is
    // separate from the amount frozen in the estimate.
    let (quota, clamp) = quota_round_checked(f64::from_bits(result.after_group_bits));
    if clamp.is_some() {
        return Err(Error::Financial);
    }
    result.quota = quota;
    Ok(result)
}

/// Price a measured terminal usage with the original durable clock, if the
/// expression needs one. Absence of usage must be handled by the caller.
pub(crate) fn settle(
    program: &Program,
    usage: &RawUsage,
    financial: Financial,
    clock: Option<SettlementClock>,
) -> Result<Evaluation, Error> {
    let params = project_source(Some(usage), program.used_vars())?;
    program.evaluate_f64(&params, financial, clock)
}

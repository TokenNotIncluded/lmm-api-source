//! Prepare the fixed source expression estimator without changing usage bounds.
use super::legacy_precharge::SourceContract;
use crate::{
    billing::{
        charging::contract::{self, LegacyPrechargeAuthorization, LegacyTieredEstimate},
        trust_pricing::FrozenTrustPrice,
    },
    relay::{RelayError, Request},
};

pub(super) fn authorization(
    source: &SourceContract,
    request: &Request,
    prompt_tokens: u32,
    group_ppm: u32,
    trust: Option<FrozenTrustPrice>,
) -> Result<LegacyPrechargeAuthorization, RelayError> {
    let estimate = LegacyTieredEstimate {
        source_expression: source
            .source_expression
            .as_ref()
            .ok_or(RelayError::Conflict)?
            .clone(),
        options: source.options(),
        prompt_tokens,
        explicit_max_tokens: request.explicit_max_output_tokens,
        group_ppm,
        group_ratio_bits: (f64::from(group_ppm) / 1_000_000.0).to_bits(),
        source_free_model: source.source_free_model,
        trust,
    };
    let estimated_hold = estimate.hold().map_err(super::billing::error)?;
    if estimated_hold > source.operator_max_billable {
        return Err(RelayError::Limit(
            "legacy precharge exceeds operator maximum",
        ));
    }
    Ok(LegacyPrechargeAuthorization::Cc52TieredWalletV1 {
        source_contract_sha256: contract::typed_digest(source).map_err(super::billing::error)?,
        estimate_sha256: contract::typed_digest(&estimate).map_err(super::billing::error)?,
        estimated_hold,
        operator_max_billable: source.operator_max_billable,
        estimate,
    })
}

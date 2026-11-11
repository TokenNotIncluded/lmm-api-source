//! Operator-selected cc52 ordinary text precharge contracts.
use crate::{billing::charging, relay::RelayError};
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct SourceContract {
    pub source_revision: String,
    pub upstream_id: u64,
    pub upstream_model: String,
    pub model: String,
    pub preconsumed_tokens: u32,
    pub count_token: bool,
    pub get_media_token: bool,
    pub get_media_token_not_stream: bool,
    pub source_free_model: bool,
    pub operator_max_billable: i64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source_tariff: Option<serde_json::Value>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source_expression: Option<String>,
}

impl SourceContract {
    pub(super) fn validate(&self) -> Result<(), RelayError> {
        if self.source_revision != charging::legacy_precharge::SOURCE_REVISION
            || self.upstream_id == 0
            || self.upstream_model.is_empty()
            || self.model.is_empty()
            || (self.source_tariff.is_some() && self.source_expression.is_some())
            || self.operator_max_billable <= 0
            || self.operator_max_billable > 9_007_199_254_740_991
        {
            return Err(RelayError::Invalid("legacy precharge source contract"));
        }
        if let Some(expression) = &self.source_expression {
            let program = crate::billing::legacy_tiered::Program::compile(expression)
                .map_err(|_| RelayError::Invalid("legacy precharge source expression"))?;
            if program.requires_clock() {
                return Err(RelayError::Unsupported(
                    "legacy precharge calendar expression",
                ));
            }
        }
        if let Some(origin) = &self.source_tariff {
            if origin["kind"] != "legacy_ratio_decimal"
                || origin["source_mode"] != "ratio"
                || origin["source_revision"].as_str() != Some(self.source_revision.as_str())
                || origin["source_usage_contract"] != "cc52-openrouter-chat-fable-usage-v1"
                || origin["model"].as_str() != Some(self.model.as_str())
                || origin["group"].as_str().is_none_or(str::is_empty)
            {
                return Err(RelayError::Invalid(
                    "legacy ordinary decimal source contract",
                ));
            }
            source_ratio(&origin["ratios"]["model"])?;
            source_ratio(&origin["ratios"]["group"])?;
        }
        Ok(())
    }

    pub(super) fn options(&self) -> charging::legacy_precharge::SourceOptions {
        charging::legacy_precharge::SourceOptions {
            preconsumed_tokens: self.preconsumed_tokens,
            count_token: self.count_token,
            get_media_token: self.get_media_token,
            get_media_token_not_stream: self.get_media_token_not_stream,
        }
    }
}

impl super::Bridge {
    pub(super) async fn legacy_authorization(
        &self,
        selected: &super::routing::Selected,
        request: &crate::relay::Request,
        body: &[u8],
        secret: &str,
        peer: Option<std::net::IpAddr>,
        issuer: Option<&crate::identity::grants::LmmIssuer>,
    ) -> Result<Option<charging::contract::LegacyPrechargeAuthorization>, RelayError> {
        let Some(source) = self
            .legacy_precharge
            .get()
            .into_iter()
            .flatten()
            .find(|source| {
                source.model == request.model
                    && source.upstream_id == selected.attempt.upstream_id()
                    && source.upstream_model == selected.attempt.upstream_model()
                    && source.source_tariff.as_ref().is_none_or(|origin| {
                        origin["group"].as_str() == Some(selected.price.group.as_str())
                    })
            })
        else {
            return Ok(None);
        };
        let quote = selected.attempt.quote();
        let rates = quote.rates();
        let (group, speed) = quote.multipliers_ppm();
        if !matches!(
            request.client,
            crate::relay::ClientProtocol::Responses | crate::relay::ClientProtocol::Chat
        ) || selected.price.upstream_cost_group_ppm.is_some()
            || selected.price.grounding.is_some()
            || selected.price.text_request.is_some()
            || selected.price.tool.is_some()
            || selected.price.audio.is_some()
            || selected.price.image.is_some()
            || rates.input > (1_u64 << 53)
            || rates.request != 0
            || speed != 1_000_000
            || request
                .messages
                .iter()
                .flat_map(|message| &message.content)
                .any(|block| !matches!(block, crate::relay::InputBlock::Text(_)))
        {
            return Ok(None);
        }
        if let Some(expression) = &source.source_expression {
            if !matches_source_expression(expression, &selected.price) {
                return Ok(None);
            }
        } else if let Some(origin) = &source.source_tariff {
            let Some(tariff) = &selected.price.tiered else {
                return Ok(None);
            };
            if selected.attempt.payload_policy()
                != crate::routing::PayloadPolicy::LegacyOpenRouterChatUsageV1
                || selected.attempt.protocol() != crate::routing::Protocol::OpenAiChat
                || !matches_source_tariff(source, origin, &selected.price)?
                || tariff.group_multiplier_ppm != group
            {
                return Ok(None);
            }
        } else if selected.price.rounding != Some(crate::routing::TokenRounding::LegacyGoRatioV1)
            || selected.price.tiered.is_some()
        {
            return Ok(None);
        }
        let trust = self
            .charging
            .legacy_precharge_trust(secret, peer, issuer, &selected.price)
            .await
            .map_err(super::billing::error)?;
        match authorization(source, request, body, rates.input, group, trust) {
            Ok(authorization) => Ok(Some(authorization)),
            Err(RelayError::Unsupported(_)) => Ok(None),
            Err(error) => Err(error),
        }
    }
}

fn authorization(
    source: &SourceContract,
    request: &crate::relay::Request,
    body: &[u8],
    input_rate: u64,
    group: u32,
    trust: Option<crate::billing::trust_pricing::FrozenTrustPrice>,
) -> Result<charging::contract::LegacyPrechargeAuthorization, RelayError> {
    use charging::contract;
    if source.source_expression.is_some() {
        let estimated = estimate_text(source, request, body, 1.0, 1.0)?;
        return super::legacy_tiered_precharge::authorization(
            source,
            request,
            estimated.prompt_tokens,
            group,
            trust,
        );
    }
    let (model_ratio, group_ratio) = if let Some(origin) = &source.source_tariff {
        (
            source_ratio(&origin["ratios"]["model"])?,
            source_ratio(&origin["ratios"]["group"])?,
        )
    } else {
        (
            input_rate as f64 / 2_000_000.0,
            f64::from(group) / 1_000_000.0,
        )
    };
    let discount = trust.as_ref().map(|trust| trust.discount());
    let effective_group = group_ratio
        * discount.map_or(1.0, |discount| {
            discount.numerator() as f64 / discount.denominator() as f64
        });
    let estimated = estimate_text(source, request, body, model_ratio, effective_group)?;
    let estimate = contract::LegacyOrdinaryEstimate {
        options: source.options(),
        input_rate,
        group_ppm: group,
        prompt_tokens: estimated.prompt_tokens,
        explicit_max_tokens: request.explicit_max_output_tokens,
        model_ratio_bits: model_ratio.to_bits(),
        group_ratio_bits: group_ratio.to_bits(),
        source_free_model: source.source_free_model,
        trust,
        source_tariff: source.source_tariff.clone(),
    };
    Ok(
        contract::LegacyPrechargeAuthorization::Cc52OrdinaryWalletV1 {
            source_contract_sha256: contract::typed_digest(source)
                .map_err(super::billing::error)?,
            estimate_sha256: contract::typed_digest(&estimate).map_err(super::billing::error)?,
            estimated_hold: i64::from(estimated.hold),
            operator_max_billable: source.operator_max_billable,
            estimate,
        },
    )
}

fn estimate_text(
    source: &SourceContract,
    request: &crate::relay::Request,
    body: &[u8],
    model_ratio: f64,
    effective_group: f64,
) -> Result<charging::legacy_precharge::Estimate, RelayError> {
    use charging::legacy_precharge;
    let estimator = match request.client {
        crate::relay::ClientProtocol::Responses => {
            legacy_precharge::responses_text::estimate_ordinary
        }
        crate::relay::ClientProtocol::Chat => legacy_precharge::chat_text::estimate_ordinary,
        _ => return Err(RelayError::Unsupported("legacy precharge client protocol")),
    };
    estimator(
        source.options(),
        request,
        body,
        model_ratio.to_bits(),
        effective_group.to_bits(),
        source.source_free_model,
    )
    .map_err(|_| RelayError::Unsupported("legacy precharge text estimator"))
}

fn matches_source_expression(expression: &str, price: &charging::Price) -> bool {
    let Some(tariff) = &price.tiered else {
        return false;
    };
    tariff.tariff.source_digest == charging::contract::digest(expression.as_bytes())
        && tariff.calendar.is_none()
        && tariff.tariff.calendar.is_none()
        && tariff.speed_multiplier_ppm == 1_000_000
}

fn source_ratio(value: &serde_json::Value) -> Result<f64, RelayError> {
    let invalid = || RelayError::Invalid("legacy source ratio");
    let raw = value.as_str().ok_or_else(invalid)?;
    let (numerator, denominator) = raw.split_once('/').unwrap_or((raw, "1"));
    let numerator = numerator.parse::<u64>().map_err(|_| invalid())?;
    let denominator = denominator.parse::<u64>().map_err(|_| invalid())?;
    let value = numerator as f64 / denominator as f64;
    if denominator == 0 || !value.is_finite() || value < 0.0 {
        return Err(invalid());
    }
    Ok(value)
}

fn matches_source_tariff(
    source: &SourceContract,
    origin: &serde_json::Value,
    price: &charging::Price,
) -> Result<bool, RelayError> {
    let Some(tariff) = &price.tiered else {
        return Ok(false);
    };
    Ok(origin["format_version"] == 1
        && origin["kind"] == "legacy_ratio_decimal"
        && origin["source_mode"] == "ratio"
        && origin["source_revision"].as_str() == Some(source.source_revision.as_str())
        && origin["source_usage_contract"] == "cc52-openrouter-chat-fable-usage-v1"
        && origin["model"].as_str() == Some(source.model.as_str())
        && origin["group"].as_str() == Some(price.group.as_str())
        && origin["payload_policy"] == "legacy_openrouter_chat_usage_v1"
        && tariff.tariff.source_digest
            == charging::contract::legacy_decimal::source_digest(origin)
                .map_err(super::billing::error)?
        && tariff.tariff.revision == 3
        && tariff.tariff.tiers.len() == 1
        && tariff.tariff.tiers[0].up_to_input_tokens.is_none()
        && tariff.tariff.calendar.is_none()
        && tariff.calendar.is_none()
        && tariff.speed_multiplier_ppm == 1_000_000)
}

#[cfg(test)]
#[path = "legacy_precharge_tests.rs"]
mod tests;

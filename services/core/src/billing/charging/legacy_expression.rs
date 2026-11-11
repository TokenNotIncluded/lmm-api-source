//! Frozen source expression inputs for the legacy wallet precharge.
use crate::billing::charging::{Error, Price, legacy_precharge::SourceOptions};
use crate::billing::{legacy_tiered, trust_pricing::FrozenTrustPrice};
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct LegacyTieredEstimate {
    pub source_expression: String,
    pub options: SourceOptions,
    pub prompt_tokens: u32,
    pub explicit_max_tokens: Option<u32>,
    pub group_ppm: u32,
    pub group_ratio_bits: u64,
    pub source_free_model: bool,
    pub trust: Option<FrozenTrustPrice>,
}

impl LegacyTieredEstimate {
    pub(crate) fn hold(&self) -> Result<i64, Error> {
        let program =
            legacy_tiered::Program::compile(&self.source_expression).map_err(|_| Error::Invalid)?;
        if program.requires_clock()
            || self.group_ratio_bits != (f64::from(self.group_ppm) / 1_000_000.0).to_bits()
            || (!self.options.count_token && self.prompt_tokens != 0)
        {
            return Err(Error::Invalid);
        }
        let mut group = f64::from_bits(self.group_ratio_bits);
        if let Some(trust) = &self.trust {
            trust.validate().map_err(|_| Error::Invalid)?;
            let discount = trust.discount();
            group *= discount.numerator() as f64 / discount.denominator() as f64;
        }
        // In cc52 only a zero effective group can bypass tiered preconsume.
        if self.source_free_model && group != 0.0 {
            return Err(Error::Invalid);
        }
        let completion = crate::billing::charging::legacy_precharge::tiered_text_completion(
            self.explicit_max_tokens,
            group.to_bits(),
        )
        .map_err(|_| Error::Invalid)?;
        let financial =
            legacy_tiered::Financial::from_bits(500_000.0_f64.to_bits(), group.to_bits())
                .map_err(|_| Error::Invalid)?;
        let evaluated = legacy_tiered::compat::estimate_text(
            &program,
            self.prompt_tokens,
            completion,
            financial,
            None,
        )
        .map_err(|_| Error::Invalid)?;
        Ok(if self.source_free_model {
            0
        } else {
            i64::from(evaluated.quota).max(1)
        })
    }

    /// Price measured supplier facts with the original binary64 operation
    /// order and source rounding. The caller selects this only for the frozen
    /// typed legacy expression wallet authorization.
    pub(crate) fn terminal_cost(
        &self,
        price: &Price,
        usage: crate::billing::charging::Usage,
        observed: &crate::relay::Usage,
    ) -> Result<i64, Error> {
        use crate::billing::legacy_tiered::{CreationSemantics, PromptSemantics, RawUsage};
        self.verify_price(price)?;
        super::verify_legacy_trust(self.trust.as_ref(), price.user_trust.as_ref())?;
        observed.validate().map_err(|_| Error::Invalid)?;
        let tariff = price.tiered.as_ref().ok_or(Error::Invalid)?;
        let integer = |value: u64| i64::try_from(value).map_err(|_| Error::Invalid);
        let cached = integer(observed.cached_input_tokens.unwrap_or(0))?;
        let output = integer(observed.output_tokens.ok_or(Error::Invalid)?)?;
        let reported_creation = observed.cache_creation_input_tokens;
        let five = observed
            .cache_creation_details
            .and_then(|v| v.ephemeral_5m_input_tokens);
        let hour = observed
            .cache_creation_details
            .and_then(|v| v.ephemeral_1h_input_tokens);
        let created = match usage.cache_creation {
            Some(created) => {
                let mut expected_five = five.map(integer).transpose()?;
                let mut expected_hour = hour.map(integer).transpose()?;
                let expected_reported = if tariff.cache_creation_mode.is_some() {
                    reported_creation.map(integer).transpose()?
                } else {
                    let total = match reported_creation {
                        Some(total) => integer(total)?,
                        None if tariff.missing_cache_creation_is_zero
                            && five.is_none()
                            && hour.is_none() =>
                        {
                            0
                        }
                        None => return Err(Error::Conflict),
                    };
                    if total != created.total {
                        return Err(Error::Conflict);
                    }
                    if total == 0 {
                        expected_five = Some(0);
                        expected_hour = Some(0);
                    } else if let Some(five) = expected_five
                        && expected_hour.is_none()
                    {
                        expected_hour = Some(total.checked_sub(five).ok_or(Error::Conflict)?);
                    } else if let Some(hour) = expected_hour
                        && expected_five.is_none()
                    {
                        expected_five = Some(total.checked_sub(hour).ok_or(Error::Conflict)?);
                    }
                    None
                };
                if created.cache_creation_mode != tariff.cache_creation_mode
                    || created.reported_total != expected_reported
                    || created.five_minute != expected_five
                    || created.one_hour != expected_hour
                    || created.twelve_hour.is_some()
                {
                    return Err(Error::Conflict);
                }
                created.total
            }
            None if reported_creation.unwrap_or(0) == 0
                && five.unwrap_or(0) == 0
                && hour.unwrap_or(0) == 0 =>
            {
                0
            }
            None => return Err(Error::Conflict),
        };
        let input = match tariff.input_semantics {
            crate::routing::InputSemantics::Inclusive => {
                if observed.uncached_input_tokens.is_some() {
                    return Err(Error::Conflict);
                }
                integer(observed.input_tokens.ok_or(Error::Invalid)?)?
            }
            crate::routing::InputSemantics::AnthropicText
                if tariff.cache_creation_mode.is_some() =>
            {
                let uncached = integer(observed.uncached_input_tokens.ok_or(Error::Invalid)?)?;
                let total = uncached
                    .checked_add(cached)
                    .and_then(|v| v.checked_add(created))
                    .ok_or(Error::Invalid)?;
                if observed
                    .input_tokens
                    .is_some_and(|v| i64::try_from(v).ok() != Some(total))
                {
                    return Err(Error::Conflict);
                }
                total
            }
            crate::routing::InputSemantics::AnthropicText => {
                if observed.uncached_input_tokens.is_some() {
                    return Err(Error::Conflict);
                }
                integer(observed.input_tokens.ok_or(Error::Invalid)?)?
            }
        };
        if input != usage.input
            || cached != usage.cached
            || output != usage.output
            || !usage.valid()
        {
            return Err(Error::Conflict);
        }
        let raw = if let Some(original) = &observed.legacy_openrouter_raw {
            if crate::relay::legacy_openrouter_usage::project(original)
                .map_err(|_| Error::Invalid)?
                != *observed
            {
                return Err(Error::Conflict);
            }
            crate::relay::legacy_openrouter_usage::tiered_source(original)
                .map_err(|_| Error::Invalid)?
        } else {
            let anthropic = observed.uncached_input_tokens.is_some();
            RawUsage {
                prompt: if anthropic {
                    integer(observed.uncached_input_tokens.ok_or(Error::Invalid)?)?
                } else {
                    input
                },
                completion: output,
                cached,
                creation_legacy: created,
                creation_openai: created,
                creation_5m: integer(five.unwrap_or(0))?,
                creation_1h: integer(hour.unwrap_or(0))?,
                prompt_semantics: if anthropic {
                    PromptSemantics::ClaudeText
                } else {
                    PromptSemantics::Inclusive
                },
                creation_semantics: if anthropic {
                    CreationSemantics::Anthropic
                } else {
                    CreationSemantics::Aggregate
                },
                ..RawUsage::default()
            }
        };
        let program =
            legacy_tiered::Program::compile(&self.source_expression).map_err(|_| Error::Invalid)?;
        let mut group = f64::from_bits(self.group_ratio_bits);
        if let Some(trust) = &price.user_trust {
            let discount = trust.discount();
            group *= discount.numerator() as f64 / discount.denominator() as f64;
        }
        let financial =
            legacy_tiered::Financial::from_bits(500_000.0_f64.to_bits(), group.to_bits())
                .map_err(|_| Error::Invalid)?;
        let evaluated = legacy_tiered::compat::settle(&program, &raw, financial, None)
            .map_err(|_| Error::Invalid)?;
        if evaluated.quota < 0 {
            return Err(Error::Invalid);
        }
        Ok(i64::from(evaluated.quota))
    }

    pub(crate) fn verify_price(&self, price: &Price) -> Result<(), Error> {
        let frozen = price.tiered.as_ref().ok_or(Error::Conflict)?;
        if price.rounding.is_some()
            || price.audio.is_some()
            || price.text_request.is_some()
            || price.tool.is_some()
            || price.grounding.is_some()
            || price.image.is_some()
            || price.upstream_cost_group_ppm.is_some()
            || price.input_per_million != 0
            || price.cached_per_million != 0
            || price.output_per_million != 0
            || frozen.tariff.source_digest
                != crate::billing::charging::contract::digest(self.source_expression.as_bytes())
            || frozen.calendar.is_some()
            || frozen.tariff.calendar.is_some()
            || frozen.group_multiplier_ppm != self.group_ppm
            || frozen.speed_multiplier_ppm != 1_000_000
            || frozen.credits_per_usd != 500_000
        {
            return Err(Error::Conflict);
        }
        self.hold()?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn estimate() -> LegacyTieredEstimate {
        LegacyTieredEstimate {
            source_expression: "tier(\"standard\", p * 2 + c * 4)".into(),
            options: SourceOptions {
                preconsumed_tokens: 5000,
                count_token: true,
                get_media_token: true,
                get_media_token_not_stream: false,
            },
            prompt_tokens: 10,
            explicit_max_tokens: None,
            group_ppm: 40_000,
            group_ratio_bits: 0.04_f64.to_bits(),
            source_free_model: false,
            trust: None,
        }
    }

    #[test]
    fn expression_precharge_uses_source_completion_default_and_rounding() {
        let mut value = estimate();
        assert_eq!(value.hold(), Ok(656));
        value.explicit_max_tokens = Some(3);
        assert_eq!(value.hold(), Ok(1));
        value.options.count_token = false;
        assert_eq!(value.hold(), Err(Error::Invalid));
        value.prompt_tokens = 0;
        assert_eq!(value.hold(), Ok(1));
    }

    #[test]
    fn free_group_and_unproved_free_model_are_distinct() {
        let mut value = estimate();
        value.source_free_model = true;
        assert_eq!(value.hold(), Err(Error::Invalid));
        value.group_ppm = 0;
        value.group_ratio_bits = 0.0_f64.to_bits();
        assert_eq!(value.hold(), Ok(0));
        value.source_free_model = false;
        assert_eq!(value.hold(), Ok(1));
    }
}

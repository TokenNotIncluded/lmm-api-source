// SPDX-License-Identifier: AGPL-3.0-or-later
//! Compatibility arithmetic for the bounded cc52 billing expressions.
//!
//! No configuration, clock acquisition, database, admission or price authority.
//! The owner must bind the original UTF-8 expression/hash and money bits before
//! using this leaf. Native decimal pricing does not call this module.
pub(crate) mod compat;
#[cfg(test)]
mod compat_tests;
mod eval;
mod parser;
#[cfg(test)]
mod tests;

pub use eval::{Evaluation, Financial, SettlementClock, quota_round_checked};
use parser::Node;

pub const SOURCE_REVISION: &str = "cc52aa195c6f2a01d703a1a407be73c31b89867d";
pub const VM_VERSION: &str = "expr-lang/expr@v1.17.8";

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Error {
    Syntax,
    Limit,
    Unsupported,
    Measurement,
    MissingClock,
    ClockRange,
    Financial,
}
impl std::fmt::Display for Error {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "legacy tiered {:?}", self)
    }
}
impl std::error::Error for Error {}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(u8)]
pub enum Variable {
    P,
    C,
    Len,
    Cr,
    Cc,
    Cc1h,
    Img,
    ImgO,
    Ai,
    CrText,
    CrImg,
}
impl Variable {
    fn parse(name: &str) -> Option<Self> {
        Some(match name {
            "p" => Self::P,
            "c" => Self::C,
            "len" => Self::Len,
            "cr" => Self::Cr,
            "cc" => Self::Cc,
            "cc1h" => Self::Cc1h,
            "img" => Self::Img,
            "img_o" => Self::ImgO,
            "ai" => Self::Ai,
            "cr_text" => Self::CrText,
            "cr_img" => Self::CrImg,
            _ => return None,
        })
    }
}
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct UsedVars(u16);
impl UsedVars {
    pub fn contains(self, var: Variable) -> bool {
        self.0 & (1 << var as u8) != 0
    }
    fn insert(&mut self, var: Variable) {
        self.0 |= 1 << var as u8;
    }
}

#[derive(Clone, Debug)]
pub struct Program {
    source: String,
    root: Node,
    used: UsedVars,
    calendar: bool,
}
impl Program {
    /// Parses only the observed arithmetic/length/calendar family. This is not
    /// an arbitrary Expr language interpreter or an admission approval.
    pub fn compile(source: &str) -> Result<Self, Error> {
        parser::compile(source)
    }
    pub fn source(&self) -> &str {
        &self.source
    }
    pub fn used_vars(&self) -> UsedVars {
        self.used
    }
    pub fn requires_clock(&self) -> bool {
        self.calendar
    }
    pub fn evaluate_f64(
        &self,
        params: &TokenParams,
        financial: Financial,
        clock: Option<SettlementClock>,
    ) -> Result<Evaluation, Error> {
        eval::evaluate(self, params, financial, clock)
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum PromptSemantics {
    #[default]
    Inclusive,
    ClaudeText,
}
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum CreationSemantics {
    #[default]
    Aggregate,
    Anthropic,
}
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct CacheDetails {
    pub text: i64,
    pub image: i64,
    pub audio: i64,
}

/// Original DTO counts, before Core's disjoint projection. The two semantics
/// are independent in cc52: the function argument and usage.UsageSemantic.
#[derive(Clone, Copy, Debug, Default)]
pub struct RawUsage {
    pub prompt: i64,
    pub completion: i64,
    pub cached: i64,
    pub creation_legacy: i64,
    pub creation_openai: i64,
    pub creation_5m: i64,
    pub creation_1h: i64,
    pub input_text: i64,
    pub input_image: i64,
    pub input_audio: i64,
    pub output_image: i64,
    pub cached_details: Option<CacheDetails>,
    pub prompt_semantics: PromptSemantics,
    pub creation_semantics: CreationSemantics,
}
#[derive(Clone, Copy, Debug, Default)]
pub struct TokenParams {
    values: [f64; 11],
    classified: Option<CacheDetails>,
}
impl TokenParams {
    pub fn value(&self, var: Variable) -> Option<f64> {
        match var {
            Variable::CrText => self.classified.map(|v| v.text as f64),
            Variable::CrImg => self.classified.map(|v| v.image as f64),
            _ => Some(self.values[var as usize]),
        }
    }
}

fn validated_cache(u: &RawUsage, limit: i64) -> Option<CacheDetails> {
    let d = u.cached_details?;
    if limit < 0
        || u.cached < 0
        || u.cached > limit
        || u.input_text < 0
        || u.input_image < 0
        || u.input_audio < 0
    {
        return None;
    }
    let mut left = limit;
    for n in [u.input_text, u.input_image, u.input_audio] {
        if n > left {
            return None;
        }
        left -= n;
    }
    left = u.cached;
    for n in [d.text, d.image, d.audio] {
        if n < 0 || n > left {
            return None;
        }
        left -= n;
    }
    if left != 0 || d.image > u.input_image || d.audio > u.input_audio {
        return None;
    }
    let text_limit = if u.input_text == 0 {
        limit
            .saturating_sub(u.input_image)
            .max(0)
            .saturating_sub(u.input_audio)
            .max(0)
    } else {
        u.input_text
    };
    (d.text <= text_limit).then_some(d)
}

/// cc52 service/BuildTieredTokenParams and DTO cache validation. Missing raw
/// usage is distinct from a measured zero (especially classified cache).
pub fn project_source(raw: Option<&RawUsage>, used: UsedVars) -> Result<TokenParams, Error> {
    let Some(u) = raw else {
        return Ok(TokenParams::default());
    };
    let cc = if u.creation_semantics == CreationSemantics::Anthropic {
        u.creation_5m
    } else {
        u.creation_legacy.max(u.creation_openai).max(0)
    };
    let hour = if u.creation_semantics == CreationSemantics::Anthropic {
        u.creation_1h
    } else {
        0
    };
    let mut p = u.prompt as f64;
    let mut c = u.completion as f64;
    let cr = u.cached as f64;
    let cc_f = cc as f64;
    let hour_f = hour as f64;
    let input_len = if u.prompt_semantics == PromptSemantics::ClaudeText {
        p + cr + cc_f + hour_f
    } else {
        p
    };
    let mut img = u.input_image as f64;
    let mut ai = u.input_audio as f64;
    if u.prompt_semantics == PromptSemantics::Inclusive {
        for (var, count) in [
            (Variable::Cr, cr),
            (Variable::Cc, cc_f),
            (Variable::Cc1h, hour_f),
            (Variable::Img, img),
            (Variable::Ai, ai),
        ] {
            if used.contains(var) {
                p -= count;
            }
        }
        if used.contains(Variable::ImgO) {
            c -= u.output_image as f64;
        }
    }
    if p < 0.0 {
        p = 0.0;
    }
    if c < 0.0 {
        c = 0.0;
    }
    let mut limit = u.prompt;
    if u.prompt_semantics == PromptSemantics::ClaudeText {
        for n in [u.cached, cc, hour] {
            limit = if n < 0 || limit < 0 {
                -1
            } else {
                limit.checked_add(n).unwrap_or(-1)
            };
        }
    }
    let classified = validated_cache(u, limit).or_else(|| {
        (u.cached_details.is_none() && u.cached == 0 && limit >= 0)
            .then_some(CacheDetails::default())
    });
    if used.contains(Variable::CrText) || used.contains(Variable::CrImg) {
        let d = classified.ok_or(Error::Measurement)?;
        let excluded = if used.contains(Variable::Cr) {
            cr
        } else {
            let mut n = 0.0;
            if used.contains(Variable::CrText) {
                n += d.text as f64;
            }
            if used.contains(Variable::CrImg) {
                n += d.image as f64;
            }
            n
        };
        if used.contains(Variable::Cr) || used.contains(Variable::CrImg) {
            img -= d.image as f64;
        }
        if used.contains(Variable::Cr) {
            ai -= d.audio as f64;
        }
        if u.prompt_semantics == PromptSemantics::Inclusive {
            p = u.prompt as f64 - excluded;
            for (var, count) in [
                (Variable::Cc, cc_f),
                (Variable::Cc1h, hour_f),
                (Variable::Img, img),
                (Variable::Ai, ai),
            ] {
                if used.contains(var) {
                    p -= count;
                }
            }
        }
        if p < 0.0 || img < 0.0 || ai < 0.0 {
            return Err(Error::Measurement);
        }
    }
    let mut params = TokenParams {
        values: [0.0; 11],
        classified,
    };
    for (var, value) in [
        (Variable::P, p),
        (Variable::C, c),
        (Variable::Len, input_len),
        (Variable::Cr, cr),
        (Variable::Cc, cc_f),
        (Variable::Cc1h, hour_f),
        (Variable::Img, img),
        (Variable::ImgO, u.output_image as f64),
        (Variable::Ai, ai),
    ] {
        params.values[var as usize] = value;
    }
    Ok(params)
}

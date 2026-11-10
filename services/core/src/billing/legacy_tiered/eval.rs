// SPDX-License-Identifier: AGPL-3.0-or-later
use super::parser::{ClockPart, Node, Op};
use super::{Error, Program, TokenParams};

#[derive(Clone, Copy, Debug)]
pub struct Financial {
    unit: f64,
    group: f64,
}
impl Financial {
    /// Bits freeze the actual Go snapshot values; do not reconstruct group
    /// ratios by reversing rounded PPM rates or apply a second native factor.
    pub fn from_bits(quota_per_unit: u64, group_ratio: u64) -> Result<Self, Error> {
        let unit = f64::from_bits(quota_per_unit);
        let group = f64::from_bits(group_ratio);
        if unit != 500_000.0 || !group.is_finite() || group < 0.0 {
            return Err(Error::Financial);
        }
        Ok(Self { unit, group })
    }
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct SettlementClock {
    unix_seconds: i64,
}
impl SettlementClock {
    /// Time is supplied by a trusted durable checkpoint, never sampled here.
    /// The observed Shanghai calendar uses its post-1991 UTC+8 domain.
    pub fn from_unix_seconds(value: i64) -> Result<Self, Error> {
        if !(946_684_800..4_102_444_800).contains(&value) {
            return Err(Error::ClockRange);
        }
        Ok(Self {
            unix_seconds: value,
        })
    }
    pub fn unix_seconds(self) -> i64 {
        self.unix_seconds
    }
    fn part(self, part: ClockPart) -> f64 {
        match part {
            ClockPart::Hour => ((self.unix_seconds % 86_400) / 3600) as f64,
            ClockPart::Weekday => ((self.unix_seconds / 86_400 + 4) % 7) as f64,
            ClockPart::ShanghaiDate => {
                let mut days = (self.unix_seconds + 28_800) / 86_400;
                let mut year = 1970;
                let leap = |y: i64| y % 4 == 0 && (y % 100 != 0 || y % 400 == 0);
                loop {
                    let n = if leap(year) { 366 } else { 365 };
                    if days < n {
                        break;
                    }
                    days -= n;
                    year += 1;
                }
                let months = [
                    31,
                    if leap(year) { 29 } else { 28 },
                    31,
                    30,
                    31,
                    30,
                    31,
                    31,
                    30,
                    31,
                    30,
                    31,
                ];
                let mut month = 1;
                for n in months {
                    if days < n {
                        break;
                    }
                    days -= n;
                    month += 1;
                }
                (year * 10000 + month * 100 + days + 1) as f64
            }
        }
    }
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum QuotaClamp {
    NaN,
    Overflow,
    Underflow,
}

/// Exactly the cc52 common.QuotaRoundChecked conversion, including saturation.
pub fn quota_round_checked(value: f64) -> (i32, Option<QuotaClamp>) {
    let rounded = value.round();
    if rounded.is_nan() {
        (0, Some(QuotaClamp::NaN))
    } else if rounded > i32::MAX as f64 {
        (i32::MAX, Some(QuotaClamp::Overflow))
    } else if rounded < i32::MIN as f64 {
        (i32::MIN, Some(QuotaClamp::Underflow))
    } else {
        (rounded as i32, None)
    }
}
#[derive(Clone, Debug)]
pub struct Evaluation {
    pub raw_expression_bits: u64,
    pub before_group_bits: u64,
    pub after_group_bits: u64,
    pub quota: i32,
    pub clamp: Option<QuotaClamp>,
    pub matched_tier: String,
    pub tier_cost_bits: u64,
}
enum Value {
    Number(f64),
    Bool(bool),
}
impl Value {
    fn number(self) -> Result<f64, Error> {
        match self {
            Self::Number(v) => Ok(v),
            _ => Err(Error::Syntax),
        }
    }
    fn boolean(self) -> Result<bool, Error> {
        match self {
            Self::Bool(v) => Ok(v),
            _ => Err(Error::Syntax),
        }
    }
}
struct Eval<'a> {
    params: &'a TokenParams,
    clock: Option<SettlementClock>,
    tier: String,
    tier_cost: f64,
}
impl Eval<'_> {
    fn node(&mut self, node: &Node) -> Result<Value, Error> {
        Ok(match node {
            Node::Number(n) => Value::Number(*n),
            Node::Var(v) => Value::Number(self.params.value(*v).ok_or(Error::Measurement)?),
            Node::Clock(p) => Value::Number(self.clock.ok_or(Error::MissingClock)?.part(*p)),
            Node::Not(n) => Value::Bool(!self.node(n)?.boolean()?),
            Node::Conditional(p, a, b) => {
                if self.node(p)?.boolean()? {
                    self.node(a)?
                } else {
                    self.node(b)?
                }
            }
            Node::Tier(label, body) => {
                let value = self.node(body)?.number()?;
                self.tier = label.clone();
                self.tier_cost = value;
                Value::Number(value)
            }
            Node::Binary(Op::And, a, b) => {
                Value::Bool(self.node(a)?.boolean()? && self.node(b)?.boolean()?)
            }
            Node::Binary(Op::Or, a, b) => {
                Value::Bool(self.node(a)?.boolean()? || self.node(b)?.boolean()?)
            }
            Node::Binary(op, a, b) => {
                let a = self.node(a)?.number()?;
                let b = self.node(b)?.number()?;
                match op {
                    Op::Add => Value::Number(a + b),
                    Op::Mul => Value::Number(a * b),
                    Op::Lt => Value::Bool(a < b),
                    Op::Le => Value::Bool(a <= b),
                    Op::Gt => Value::Bool(a > b),
                    Op::Ge => Value::Bool(a >= b),
                    _ => return Err(Error::Syntax),
                }
            }
        })
    }
}
pub(super) fn evaluate(
    program: &Program,
    params: &TokenParams,
    money: Financial,
    clock: Option<SettlementClock>,
) -> Result<Evaluation, Error> {
    if program.calendar && clock.is_none() {
        return Err(Error::MissingClock);
    }
    // Original VM checks every used measured dimension before branch selection.
    for var in [super::Variable::CrText, super::Variable::CrImg] {
        if program.used.contains(var) && params.value(var).is_none() {
            return Err(Error::Measurement);
        }
    }
    let mut state = Eval {
        params,
        clock,
        tier: String::new(),
        tier_cost: 0.0,
    };
    let raw = state.node(&program.root)?.number()?;
    // Keep these distinct binary64 operations in the original order; no FMA,
    // coefficient collection, rational conversion or algebraic reassociation.
    let before = raw / 1_000_000.0 * money.unit;
    let after = before * money.group;
    let (mut quota, clamp) = quota_round_checked(after);
    if quota == 0 && before > 0.0 && money.group > 0.0 {
        quota = 1;
    }
    Ok(Evaluation {
        raw_expression_bits: raw.to_bits(),
        before_group_bits: before.to_bits(),
        after_group_bits: after.to_bits(),
        quota,
        clamp,
        matched_tier: state.tier,
        tier_cost_bits: state.tier_cost.to_bits(),
    })
}

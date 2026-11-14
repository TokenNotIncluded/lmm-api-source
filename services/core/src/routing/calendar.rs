//! The reviewed DeepSeek calendar: UTC weekdays/hours and explicit Shanghai
//! exclusion dates. Selection is made once, before reservation, never at settle.
use super::{CompiledTariff, RoutingError, TieredTariff, TokenTier};
use chrono::{Datelike, NaiveDate, Timelike, Utc};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct HourRange {
    pub start: u32,
    pub end: u32,
}
#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct DateRange {
    pub start: u32,
    pub end: u32,
}
#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct CalendarSchedule {
    pub revision: u32,
    pub source_expression: String,
    pub weekday_timezone: String,
    pub weekdays: Vec<u32>,
    pub hour_timezone: String,
    pub hour_ranges: Vec<HourRange>,
    pub excluded_date_timezone: String,
    pub excluded_dates: Vec<DateRange>,
    pub match_tiers: Vec<TokenTier>,
}
#[derive(Clone, Copy, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum CalendarBranch {
    Peak,
    OffPeak,
}
#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct FrozenCalendar {
    pub quote_at_unix_seconds: i64,
    pub branch: CalendarBranch,
    pub schedule: CalendarSchedule,
    pub off_peak_tiers: Vec<TokenTier>,
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) struct CompiledCalendar {
    schedule: CalendarSchedule,
    off_peak: CompiledTariff,
    peak: CompiledTariff,
    excluded_utc: Vec<(i64, i64)>,
}
fn invalid() -> RoutingError {
    RoutingError::InvalidConfig("calendar tariff")
}
fn date(value: u32) -> Result<NaiveDate, RoutingError> {
    let d = NaiveDate::from_ymd_opt((value / 10000) as i32, value / 100 % 100, value % 100)
        .ok_or_else(invalid)?;
    // The actual source exclusions are in 2026, when Asia/Shanghai is UTC+08.
    // Do not claim historical DST or future calendar data from this narrow wire.
    if d.year() != 2026 {
        return Err(invalid());
    }
    Ok(d)
}
impl CalendarSchedule {
    pub(super) fn compile(&self, base: &TieredTariff) -> Result<CompiledCalendar, RoutingError> {
        if self.revision != 1
            || self.source_expression.is_empty()
            || self.source_expression.len() > 16384
            || crate::hex_codec::encode(Sha256::digest(self.source_expression.as_bytes()))
                != base.source_digest
            || self.weekday_timezone != "UTC"
            || self.hour_timezone != "UTC"
            || self.excluded_date_timezone != "Asia/Shanghai"
            || self.weekdays.is_empty()
            || self.weekdays.len() > 7
            || self.weekdays.iter().any(|v| !(1..=7).contains(v))
            || self.weekdays.windows(2).any(|w| w[0] >= w[1])
            || self.hour_ranges.is_empty()
            || self.hour_ranges.len() > 4
            || self
                .hour_ranges
                .iter()
                .any(|r| r.start >= r.end || r.end > 24)
            || self.hour_ranges.windows(2).any(|w| w[0].end > w[1].start)
            || self.excluded_dates.len() > 16
        {
            return Err(invalid());
        }
        let mut excluded_utc = Vec::new();
        for r in &self.excluded_dates {
            let start = date(r.start)?;
            let end = date(r.end)?;
            if start > end {
                return Err(invalid());
            }
            let start = start
                .and_hms_opt(0, 0, 0)
                .ok_or_else(invalid)?
                .and_utc()
                .timestamp()
                - 8 * 3600;
            let end = end
                .succ_opt()
                .ok_or_else(invalid)?
                .and_hms_opt(0, 0, 0)
                .ok_or_else(invalid)?
                .and_utc()
                .timestamp()
                - 8 * 3600;
            if excluded_utc
                .last()
                .is_some_and(|&(_, previous)| previous > start)
            {
                return Err(invalid());
            }
            excluded_utc.push((start, end));
        }
        let mut off_peak = base.clone();
        off_peak.calendar = None;
        let mut peak = off_peak.clone();
        peak.tiers = self.match_tiers.clone();
        Ok(CompiledCalendar {
            schedule: self.clone(),
            off_peak: off_peak.compile()?,
            peak: peak.compile()?,
            excluded_utc,
        })
    }
}
impl CompiledCalendar {
    pub(super) fn select(
        &self,
        timestamp: i64,
    ) -> Result<(CompiledTariff, FrozenCalendar), RoutingError> {
        let now = chrono::DateTime::<Utc>::from_timestamp(timestamp, 0).ok_or_else(invalid)?;
        let peak = self
            .schedule
            .weekdays
            .contains(&now.weekday().number_from_monday())
            && self
                .schedule
                .hour_ranges
                .iter()
                .any(|r| r.start <= now.hour() && now.hour() < r.end)
            && !self
                .excluded_utc
                .iter()
                .any(|&(start, end)| start <= timestamp && timestamp < end);
        Ok((
            if peak { self.peak } else { self.off_peak },
            FrozenCalendar {
                quote_at_unix_seconds: timestamp,
                branch: if peak {
                    CalendarBranch::Peak
                } else {
                    CalendarBranch::OffPeak
                },
                schedule: self.schedule.clone(),
                off_peak_tiers: self.off_peak.tariff().tiers,
            },
        ))
    }
}
impl FrozenCalendar {
    pub fn validate(&self, selected: &TieredTariff) -> Result<(), RoutingError> {
        let mut base = selected.clone();
        base.tiers = self.off_peak_tiers.clone();
        let (compiled, frozen) = self
            .schedule
            .compile(&base)?
            .select(self.quote_at_unix_seconds)?;
        if frozen != *self || compiled.tariff() != *selected {
            return Err(invalid());
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::routing::{
        InputSemantics, PersistedTieredTariff, PriceConfig, PriceSnapshot, Speed, TieredUsage,
    };
    fn time(s: &str) -> i64 {
        chrono::DateTime::parse_from_rfc3339(s).unwrap().timestamp()
    }
    fn fixture() -> TieredTariff {
        let source = "synthetic-calendar-fixture";
        serde_json::from_value(serde_json::json!({
            "revision":1,"source_digest":crate::hex_codec::encode(Sha256::digest(source.as_bytes())),
            "uses_cache_read":false,"uses_cache_write":false,
            "tiers":[{"rates":{"input":{"numerator":1,"denominator":1},"output":{"numerator":0,"denominator":1},
                "cache_read":{"numerator":0,"denominator":1},"cache_write":{"numerator":0,"denominator":1}}}],
            "calendar":{"revision":1,"source_expression":source,"weekday_timezone":"UTC","weekdays":[1,2,3,4,5],
                "hour_timezone":"UTC","hour_ranges":[{"start":1,"end":4},{"start":6,"end":10}],
                "excluded_date_timezone":"Asia/Shanghai","excluded_dates":[{"start":20260101,"end":20260103}],
                "match_tiers":[{"rates":{"input":{"numerator":2,"denominator":1},"output":{"numerator":0,"denominator":1},
                    "cache_read":{"numerator":0,"denominator":1},"cache_write":{"numerator":0,"denominator":1}}}]}
        })).unwrap()
    }
    #[test]
    fn calendar_utc_hours_weekdays_and_inclusive_shanghai_dates() {
        let tariff = fixture();
        let schedule = tariff.calendar.as_ref().unwrap().compile(&tariff).unwrap();
        for (stamp, branch) in [
            ("2026-11-10T00:59:59Z", CalendarBranch::OffPeak),
            ("2026-11-10T01:00:00Z", CalendarBranch::Peak),
            ("2026-11-10T03:59:59Z", CalendarBranch::Peak),
            ("2026-11-10T04:00:00Z", CalendarBranch::OffPeak),
            ("2026-11-10T06:00:00Z", CalendarBranch::Peak),
            ("2026-11-10T09:59:59Z", CalendarBranch::Peak),
            ("2026-11-10T10:00:00Z", CalendarBranch::OffPeak),
            ("2026-11-14T01:00:00Z", CalendarBranch::OffPeak),
            ("2026-01-01T01:00:00Z", CalendarBranch::OffPeak),
            ("2026-01-02T06:00:00Z", CalendarBranch::OffPeak),
            ("2026-01-05T01:00:00Z", CalendarBranch::Peak),
        ] {
            assert_eq!(
                schedule.select(time(stamp)).unwrap().1.branch,
                branch,
                "{stamp}"
            );
        }
        assert_eq!(
            schedule.excluded_utc,
            vec![(time("2025-12-31T16:00:00Z"), time("2026-01-03T16:00:00Z"))]
        );
    }
    #[test]
    fn calendar_persisted_choice_survives_boundaries_and_restores_exact_price() {
        let tariff = fixture();
        let schedule = tariff.calendar.as_ref().unwrap().compile(&tariff).unwrap();
        let (selected, calendar) = schedule.select(time("2026-11-10T03:59:59Z")).unwrap();
        let frozen = PersistedTieredTariff {
            tariff: selected.tariff(),
            calendar: Some(calendar),
            input_semantics: InputSemantics::Inclusive,
            group_multiplier_ppm: 1_000_000,
            speed_multiplier_ppm: 1_000_000,
            credits_per_usd: 500_000,
            missing_cache_creation_is_zero: false,
            cache_creation_mode: None,
        };
        let restored: PersistedTieredTariff =
            serde_json::from_slice(&serde_json::to_vec(&frozen).unwrap()).unwrap();
        assert_eq!(
            restored
                .cost(TieredUsage {
                    input: 1_000_000,
                    ..Default::default()
                })
                .unwrap(),
            1_000_000
        );
        assert_eq!(
            schedule
                .select(time("2026-11-10T04:00:00Z"))
                .unwrap()
                .1
                .branch,
            CalendarBranch::OffPeak
        );
        assert_eq!(restored, frozen); // recovery never consults the later clock/config
        let mut wrong = restored.clone();
        wrong.calendar.as_mut().unwrap().branch = CalendarBranch::OffPeak;
        assert!(wrong.validate().is_err());
        let mut wrong = restored;
        wrong.tariff.tiers[0].rates.input.numerator = 1;
        assert!(wrong.validate().is_err());
    }
    #[test]
    fn calendar_snapshot_requires_explicit_admission_time_and_checks_provenance() {
        let tariff = fixture();
        assert!(tariff.compile().is_err());
        let config:PriceConfig=serde_json::from_value(serde_json::json!({"version":1,"rules":[{
            "id":1,"model":"synthetic","group":"default","speed":"standard","billing_mode":"tiered_decimal","tiered":tariff,
            "rates":{"input":0,"output":0,"cache_read":0,"cache_write":0,"request":0},
            "group_multiplier_ppm":1_000_000,"speed_multiplier_ppm":1_000_000}]})).unwrap();
        let prices = PriceSnapshot::compile(config.clone()).unwrap();
        assert!(
            prices
                .quote("synthetic", "default", Speed::Standard)
                .is_none()
        );
        let (_, frozen) = prices
            .quote_at(
                "synthetic",
                "default",
                Speed::Standard,
                time("2026-11-10T01:00:00Z"),
            )
            .unwrap()
            .unwrap();
        assert_eq!(frozen.unwrap().branch, CalendarBranch::Peak);
        let mut changed = config.clone();
        changed.rules[0]
            .tiered
            .as_mut()
            .unwrap()
            .calendar
            .as_mut()
            .unwrap()
            .match_tiers[0]
            .rates
            .input
            .numerator = 3;
        assert!(!prices.same_content(&PriceSnapshot::compile(changed).unwrap()));
        let mut bad = config.clone();
        bad.rules[0].tiered.as_mut().unwrap().source_digest = "ab".repeat(32);
        assert!(PriceSnapshot::compile(bad).is_err());
        let mut bad = config;
        bad.rules[0]
            .tiered
            .as_mut()
            .unwrap()
            .calendar
            .as_mut()
            .unwrap()
            .excluded_dates[0]
            .end = 20270101;
        assert!(PriceSnapshot::compile(bad).is_err());
    }
    #[test]
    fn calendar_router_retry_retains_original_admission_receipt() {
        use crate::routing::{
            DeliveryState, Failure, FailureKind, Protocol, ReplayPermission, RouteRequest, Router,
            RoutingConfig,
        };
        use std::sync::Arc;
        let config:PriceConfig=serde_json::from_value(serde_json::json!({"version":1,"rules":[{
            "id":1,"model":"synthetic","group":"default","speed":"standard","billing_mode":"tiered_decimal","tiered":fixture(),
            "rates":{"input":0,"output":0,"cache_read":0,"cache_write":0,"request":0},
            "group_multiplier_ppm":1_000_000,"speed_multiplier_ppm":1_000_000}]})).unwrap();
        let routing:RoutingConfig=serde_json::from_value(serde_json::json!({
            "version":1,"groups":[{"name":"default","enabled":true}],"aliases":[],
            "upstreams":[
                {"id":1,"endpoint":"https://one.example/v1","protocol":"openai_chat","credential":{"id":1,"revision":1},"enabled":true},
                {"id":2,"endpoint":"https://two.example/v1","protocol":"openai_chat","credential":{"id":2,"revision":1},"enabled":true}],
            "routes":[{"id":1,"model":"synthetic","group":"default","enabled":true,"speeds":["standard"],
                "targets":[
                    {"id":1,"upstream_id":1,"upstream_model":"synthetic","enabled":true,"priority":1,"weight":1,"speeds":["standard"],"payload":"adapt","timeout_ms":1000},
                    {"id":2,"upstream_id":2,"upstream_model":"synthetic","enabled":true,"priority":2,"weight":1,"speeds":["standard"],"payload":"adapt","timeout_ms":1000}],
                "failover":{"max_attempts":2,"retry_on":["connect"],"allow_replay_after_send":false}}]})).unwrap();
        let router =
            Router::new(routing, Arc::new(PriceSnapshot::compile(config).unwrap())).unwrap();
        let mut plan = router
            .route(RouteRequest {
                model: "synthetic",
                group: "default",
                speed: None,
                protocol: Protocol::OpenAiChat,
                replay: ReplayPermission::Denied,
            })
            .unwrap();
        let first = plan.start().unwrap();
        let second = plan
            .retry(Failure {
                kind: FailureKind::Connect,
                delivery: DeliveryState::NotSent,
            })
            .unwrap();
        assert_eq!(first.quote(), second.quote());
        assert!(std::ptr::eq(
            first.calendar().unwrap(),
            second.calendar().unwrap()
        ));
    }
}

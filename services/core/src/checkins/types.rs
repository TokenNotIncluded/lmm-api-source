use serde::{Deserialize, Serialize};

pub const SOURCE_REVISION: &str = "cc52aa195c6f2a01d703a1a407be73c31b89867d";
pub const CALENDAR_OFFSET_SECONDS: i32 = 28_800;

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct ClaimRequest {
    pub operation_key: String,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq)]
pub struct DailyRecord {
    pub checkin_date: String,
    pub quota_awarded: String,
    pub created_at: i64,
    pub journal_id: Option<i64>,
    pub origin: String,
    pub policy_revision: Option<i64>,
    pub trust_level: Option<u8>,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq)]
pub struct ClaimResult {
    pub already_claimed: bool,
    pub record: DailyRecord,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq)]
pub struct RewardRange {
    pub trust_level: u8,
    pub reward_multiplier: f64,
    pub base_min_quota: String,
    pub base_max_quota: String,
    pub min_quota: String,
    pub max_quota: String,
}

#[derive(Clone, Debug, Serialize)]
pub struct Status {
    pub enabled: bool,
    pub today: String,
    pub month: String,
    pub calendar_offset_seconds: i32,
    pub policy_revision: Option<i64>,
    pub checked_in_today: bool,
    pub reward_range: Option<RewardRange>,
    pub today_record: Option<DailyRecord>,
    pub records: Vec<DailyRecord>,
    pub total_checkins: i64,
    pub total_quota_awarded: String,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct Policy {
    pub enabled: bool,
    pub min_quota: String,
    pub max_quota: String,
    pub level_multipliers: Vec<f64>,
    pub calendar_offset_seconds: i32,
    pub credits_per_usd: i64,
    pub source_revision: String,
    pub source_options_sha256: String,
    pub history_seed_id: String,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct PublishPolicy {
    pub operation_key: String,
    pub expected_revision: Option<i64>,
    pub policy: Policy,
    /// Explicit subsequent owner configuration. Initial publication uses the
    /// immutable source settings retained by the seed receipt.
    #[serde(default)]
    pub source_options: Option<std::collections::BTreeMap<String, String>>,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq)]
pub struct PublishedPolicy {
    pub revision: i64,
    pub policy: Policy,
}

/// The exact formal JSONL is authenticated against its committed identity receipt.
/// Check-in rows come from the separately committed full history receipt.
#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct SeedHistory {
    pub operation_key: String,
    pub history_migration_id: String,
    pub identity_migration_id: String,
    pub source_archive_sha256: String,
    pub mapping_sha256: String,
    pub source_cutoff: i64,
    pub identity_map_jsonl: String,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq)]
pub struct SeedReceipt {
    pub seed_id: String,
    pub history_migration_id: String,
    pub identity_migration_id: String,
    pub source_archive_sha256: String,
    pub mapping_sha256: String,
    pub source_cutoff: i64,
    pub source_options_sha256: String,
    pub source_rows: i64,
    pub mapped_rows: i64,
    pub orphan_rows: i64,
    pub balance_effect: String,
}

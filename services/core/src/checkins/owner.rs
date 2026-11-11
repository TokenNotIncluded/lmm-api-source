use super::*;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::collections::{BTreeMap, BTreeSet};

const DEFAULT_MULTIPLIERS: [f64; 5] = [0.5, 0.65, 0.8, 0.9, 1.0];

impl Policy {
    pub fn validate(&self) -> Result<()> {
        if self.source_revision != SOURCE_REVISION
            || self.calendar_offset_seconds != CALENDAR_OFFSET_SECONDS
            || self.credits_per_usd != 500_000
            || !hash_valid(&self.source_options_sha256)
            || !hash_valid(&self.history_seed_id)
            || self.level_multipliers.iter().any(|v| !v.is_finite())
        {
            return Err(Error::Invalid);
        }
        for level in 0..5 {
            self.reward_range(level)?;
        }
        Ok(())
    }

    pub fn reward_range(&self, level: u8) -> Result<RewardRange> {
        let index = usize::from(level.min(4));
        let multiplier = self
            .level_multipliers
            .get(index)
            .copied()
            .filter(|v| v.is_finite() && *v >= 0.0)
            .unwrap_or(DEFAULT_MULTIPLIERS[index]);
        let base_min = integer(&self.min_quota)?.max(0);
        let base_max = integer(&self.max_quota)?.max(base_min);
        let scale = |value: i64| -> Result<i64> {
            // cc52 uses Go math.Round(float64(base)*multiplier), half away
            // from zero. Bounds here are nonnegative, matching f64::round.
            let scaled = (value as f64 * multiplier).round();
            if !scaled.is_finite() || scaled < 0.0 || scaled >= i64::MAX as f64 {
                return Err(Error::Invalid);
            }
            Ok(scaled as i64)
        };
        let min = scale(base_min)?;
        let max = scale(base_max)?.max(min);
        Ok(RewardRange {
            trust_level: level,
            reward_multiplier: multiplier,
            base_min_quota: self.min_quota.clone(),
            base_max_quota: self.max_quota.clone(),
            min_quota: min.to_string(),
            max_quota: max.to_string(),
        })
    }
}

fn hash_valid(value: &str) -> bool {
    value.len() == 64
        && value
            .bytes()
            .all(|v| v.is_ascii_digit() || (b'a'..=b'f').contains(&v))
}
fn sha(bytes: &[u8]) -> String {
    Sha256::digest(bytes)
        .iter()
        .map(|v| format!("{v:02x}"))
        .collect()
}
fn value_hash(value: &Value) -> Result<String> {
    // The history options/rows contain flat objects. BTreeMap makes ordering
    // independent of serde_json's optional preserve_order feature.
    let object = value.as_object().ok_or(Error::Invalid)?;
    let ordered: BTreeMap<_, _> = object.iter().collect();
    Ok(sha(
        &serde_json::to_vec(&ordered).map_err(|_| Error::Invalid)?
    ))
}
async fn owner(c: &mut PgConnection) -> Result<()> {
    let allowed: bool = sqlx::query_scalar("SELECT core_identity.is_identity_relation_owner() AND (current_user=session_user OR EXISTS(SELECT FROM pg_roles r WHERE r.rolname=current_user AND NOT r.rolcanlogin AND pg_has_role(session_user,r.rolname,'MEMBER'))) AND NOT EXISTS(SELECT FROM pg_class c JOIN pg_namespace n ON n.oid=c.relnamespace WHERE n.nspname='core_checkins' AND c.relkind IN ('r','p') AND pg_get_userbyid(c.relowner)<>current_user)")
        .fetch_one(&mut *c).await?;
    if !allowed {
        return Err(Error::Forbidden);
    }
    sqlx::query("SELECT pg_advisory_xact_lock(hashtextextended('core-checkins-owner',0))")
        .execute(c)
        .await?;
    Ok(())
}
async fn prior(c: &mut PgConnection, key: &str, request: &Value) -> Result<Option<Value>> {
    let row = sqlx::query(
        "SELECT request,result FROM core_checkins.owner_commands WHERE operation_key=$1",
    )
    .bind(key)
    .fetch_optional(c)
    .await?;
    match row {
        None => Ok(None),
        Some(row) => {
            if row.try_get::<Value, _>("request")? != *request {
                return Err(Error::Conflict);
            }
            Ok(Some(row.try_get("result")?))
        }
    }
}
async fn remember<T: Serialize>(
    c: &mut PgConnection,
    key: &str,
    request: &Value,
    result: &T,
) -> Result<()> {
    sqlx::query(
        "INSERT INTO core_checkins.owner_commands(operation_key,request,result) VALUES($1,$2,$3)",
    )
    .bind(key)
    .bind(Json(request))
    .bind(Json(result))
    .execute(c)
    .await?;
    Ok(())
}

impl CheckinStore {
    pub async fn publish_policy(&self, input: PublishPolicy) -> Result<PublishedPolicy> {
        valid_key(&input.operation_key)?;
        input.policy.validate()?;
        let request =
            serde_json::to_value((&"publish_policy", &input)).map_err(|_| Error::Invalid)?;
        let mut tx = self.begin().await?;
        owner(&mut tx).await?;
        if let Some(result) = prior(&mut tx, &input.operation_key, &request).await? {
            let result = serde_json::from_value(result).map_err(|_| Error::Unavailable)?;
            tx.commit().await?;
            return Ok(result);
        }
        let current = policy(&mut tx).await?;
        if current.as_ref().map(|p| p.revision) != input.expected_revision {
            return Err(Error::Conflict);
        }
        if current
            .as_ref()
            .is_some_and(|p| p.policy.history_seed_id != input.policy.history_seed_id)
        {
            return Err(Error::Conflict);
        }
        let settings: Option<Value> =
            sqlx::query_scalar("SELECT settings FROM core_checkins.seed_receipts WHERE seed_id=$1")
                .bind(&input.policy.history_seed_id)
                .fetch_optional(&mut *tx)
                .await?;
        let original_settings = settings.ok_or(Error::NotFound)?;
        let settings = match &input.source_options {
            None => match &current {
                None => original_settings.clone(),
                Some(current) => sqlx::query_scalar(
                    "SELECT source_options FROM core_checkins.policy_revisions WHERE revision=$1",
                )
                .bind(current.revision)
                .fetch_one(&mut *tx)
                .await?,
            },
            Some(options) => serde_json::to_value(options).map_err(|_| Error::Invalid)?,
        };
        if (current.is_none() && settings != original_settings)
            || value_hash(&settings)? != input.policy.source_options_sha256
        {
            return Err(Error::Conflict);
        }
        policy_matches_source(&input.policy, &settings)?;
        let revision =
            current.map_or(Ok(1), |p| p.revision.checked_add(1).ok_or(Error::Invalid))?;
        let result = PublishedPolicy {
            revision,
            policy: input.policy,
        };
        sqlx::query("INSERT INTO core_checkins.policy_revisions(revision,policy,seed_id,created_at,source_options) VALUES($1,$2,$3,$4,$5)")
            .bind(revision).bind(Json(&result.policy)).bind(&result.policy.history_seed_id).bind(now(&mut tx).await?).bind(Json(&settings))
            .execute(&mut *tx).await?;
        sqlx::query("INSERT INTO core_checkins.active_policy(singleton,revision) VALUES(true,$1) ON CONFLICT(singleton) DO UPDATE SET revision=excluded.revision")
            .bind(revision).execute(&mut *tx).await?;
        remember(&mut tx, &input.operation_key, &request, &result).await?;
        tx.commit().await?;
        Ok(result)
    }

    pub async fn seed_history(&self, input: SeedHistory) -> Result<SeedReceipt> {
        valid_key(&input.operation_key)?;
        if !hash_valid(&input.source_archive_sha256)
            || !hash_valid(&input.mapping_sha256)
            || input.source_cutoff <= 0
        {
            return Err(Error::Invalid);
        }
        let request = json!(["seed_history", input]);
        let mut tx = self.begin().await?;
        owner(&mut tx).await?;
        if let Some(result) = prior(&mut tx, &input.operation_key, &request).await? {
            let result = serde_json::from_value(result).map_err(|_| Error::Unavailable)?;
            tx.commit().await?;
            return Ok(result);
        }
        let history = manifest(&mut tx, &input.history_migration_id).await?;
        let identity = manifest(&mut tx, &input.identity_migration_id).await?;
        let map_hash = sha(input.identity_map_jsonl.as_bytes());
        if history["scope"] != "checkin-history.v1"
            || history["source_namespace"] != "lmm_api.lmm_prod_20260802.checkins"
            || history["history_complete"] != true
            || history["balance_effect"] != "none"
            || history["award_unit"] != "legacy_raw_no_conversion"
            || history["source_archive_sha256"] != input.source_archive_sha256
            || history["observed_at"].as_i64() != Some(input.source_cutoff)
            || identity["scope"] != "identity-only"
            || identity["source_archive_sha256"] != input.source_archive_sha256
            || identity["mapping_sha256"] != input.mapping_sha256
            || identity["source_target_id_map_sha256"] != map_hash
        {
            return Err(Error::Conflict);
        }
        let opening: bool=sqlx::query_scalar("SELECT EXISTS(SELECT FROM core_billing.opening_import_runs WHERE source_archive_sha256=$1 AND observed_at=$2)")
            .bind(&input.source_archive_sha256).bind(input.source_cutoff).fetch_one(&mut *tx).await?;
        if !opening {
            return Err(Error::Conflict);
        }
        let mapping = validated_mapping(&mut tx, &input).await?;
        let source_users = history["source_user_ids"]
            .as_array()
            .ok_or(Error::Invalid)?
            .iter()
            .map(|v| v.as_i64().filter(|v| *v > 0).ok_or(Error::Invalid))
            .collect::<Result<BTreeSet<_>>>()?;
        let rows = history["rows"].as_array().ok_or(Error::Invalid)?;
        if history["source_row_count"].as_u64() != Some(rows.len() as u64) {
            return Err(Error::Conflict);
        }
        let mut ids = BTreeSet::new();
        let mut source_days = BTreeSet::new();
        let mut mapped = Vec::new();
        for entry in rows {
            let row: SourceRow =
                serde_json::from_value(entry["record"].clone()).map_err(|_| Error::Invalid)?;
            if row.id <= 0
                || row.user_id <= 0
                || row.quota_awarded < 0
                || row.created_at < 0
                || row.created_at > input.source_cutoff
                || calendar_day(row.created_at)? != row.checkin_date
                || !ids.insert(row.id)
                || !source_days.insert((row.user_id, row.checkin_date.clone()))
                || entry["source_row_sha256"] != value_hash(&entry["record"])?
                || entry["source_user_present"].as_bool()
                    != Some(source_users.contains(&row.user_id))
            {
                return Err(Error::Conflict);
            }
            match mapping.get(&row.user_id) {
                Some(target) if source_users.contains(&row.user_id) => mapped.push((row, *target)),
                Some(_) => return Err(Error::Conflict),
                None if source_users.contains(&row.user_id) => return Err(Error::Conflict),
                None => {} // Full original row remains in the immutable history receipt.
            }
        }
        let settings = history["settings"].clone();
        let source_options_sha256 = value_hash(&settings)?;
        let seed_id = sha(format!(
            "checkin-seed:{}:{}",
            input.history_migration_id, input.mapping_sha256
        )
        .as_bytes());
        let result = SeedReceipt {
            seed_id,
            history_migration_id: input.history_migration_id.clone(),
            identity_migration_id: input.identity_migration_id.clone(),
            source_archive_sha256: input.source_archive_sha256.clone(),
            mapping_sha256: input.mapping_sha256.clone(),
            source_cutoff: input.source_cutoff,
            source_options_sha256,
            source_rows: rows.len() as i64,
            mapped_rows: mapped.len() as i64,
            orphan_rows: (rows.len() - mapped.len()) as i64,
            balance_effect: "none".into(),
        };
        let existing: Option<Json<SeedReceipt>> = sqlx::query_scalar(
            "SELECT result FROM core_checkins.seed_receipts WHERE history_migration_id=$1",
        )
        .bind(&input.history_migration_id)
        .fetch_optional(&mut *tx)
        .await?;
        if let Some(existing) = existing {
            if existing.0 != result {
                return Err(Error::Conflict);
            }
            remember(&mut tx, &input.operation_key, &request, &result).await?;
            tx.commit().await?;
            return Ok(result);
        }
        sqlx::query("INSERT INTO core_checkins.seed_receipts(seed_id,history_migration_id,identity_migration_id,source_archive_sha256,mapping_sha256,source_cutoff,source_options_sha256,settings,result) VALUES($1,$2,$3,$4,$5,$6,$7,$8,$9)")
            .bind(&result.seed_id).bind(&input.history_migration_id).bind(&input.identity_migration_id)
            .bind(&input.source_archive_sha256).bind(&input.mapping_sha256).bind(input.source_cutoff)
            .bind(&result.source_options_sha256).bind(Json(&settings)).bind(Json(&result)).execute(&mut *tx).await?;
        for (row, (user, account)) in mapped {
            if daily(&mut tx, account, &row.checkin_date).await?.is_some() {
                return Err(Error::Conflict);
            }
            let record = DailyRecord {
                checkin_date: row.checkin_date,
                quota_awarded: row.quota_awarded.to_string(),
                created_at: row.created_at,
                journal_id: None,
                origin: "history".into(),
                policy_revision: None,
                trust_level: None,
            };
            sqlx::query("INSERT INTO core_checkins.days(account_id,user_id,checkin_date,amount_units,origin,seed_id,created_at,result) VALUES($1,$2,$3::text::date,$4,'history',$5,$6,$7)")
                .bind(account).bind(user).bind(&record.checkin_date).bind(row.quota_awarded)
                .bind(&result.seed_id).bind(row.created_at).bind(Json(&record)).execute(&mut *tx).await?;
        }
        remember(&mut tx, &input.operation_key, &request, &result).await?;
        tx.commit().await?;
        Ok(result)
    }
}

fn policy_matches_source(policy: &Policy, settings: &Value) -> Result<()> {
    let prefix = "checkin_setting.";
    let text = |key: &str| {
        settings
            .get(format!("{prefix}{key}"))
            .and_then(Value::as_str)
            .ok_or(Error::Invalid)
    };
    if settings.as_object().map(|m| m.len()) != Some(4) {
        return Err(Error::Invalid);
    }
    let source_enabled: bool =
        serde_json::from_str(text("enabled")?).map_err(|_| Error::Invalid)?;
    let min = integer(text("min_quota")?)?;
    let max = integer(text("max_quota")?)?;
    let multipliers: Vec<f64> =
        serde_json::from_str(text("level_multipliers")?).map_err(|_| Error::Invalid)?;
    if (policy.enabled && !source_enabled)
        || integer(&policy.min_quota)? != min
        || integer(&policy.max_quota)? != max
        || policy.level_multipliers != multipliers
    {
        return Err(Error::Conflict);
    }
    Ok(())
}
async fn manifest(c: &mut PgConnection, id: &str) -> Result<Value> {
    sqlx::query_scalar(
        "SELECT manifest FROM core_migration.import_runs WHERE migration_id=$1 FOR SHARE",
    )
    .bind(id)
    .fetch_optional(c)
    .await?
    .ok_or(Error::NotFound)
}
#[derive(Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct SourceRow {
    id: i64,
    user_id: i64,
    checkin_date: String,
    quota_awarded: i64,
    created_at: i64,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct IdentityMap {
    source_user_id: i64,
    target_user_id: i64,
    target_personal_account_id: i64,
}
async fn validated_mapping(
    c: &mut PgConnection,
    input: &SeedHistory,
) -> Result<BTreeMap<i64, (i64, i64)>> {
    let mut result = BTreeMap::new();
    let mut users = BTreeSet::new();
    let mut accounts = BTreeSet::new();
    for line in input.identity_map_jsonl.lines().filter(|v| !v.is_empty()) {
        let row: IdentityMap = serde_json::from_str(line).map_err(|_| Error::Invalid)?;
        if row.source_user_id <= 0
            || row.target_user_id <= 0
            || row.target_personal_account_id <= 0
            || result
                .insert(
                    row.source_user_id,
                    (row.target_user_id, row.target_personal_account_id),
                )
                .is_some()
            || !users.insert(row.target_user_id)
            || !accounts.insert(row.target_personal_account_id)
        {
            return Err(Error::Conflict);
        }
    }
    let rows=sqlx::query("SELECT u.id,u.personal_account_id FROM core_identity.users u JOIN core_identity.accounts a ON a.id=u.personal_account_id WHERE u.id=ANY($1) AND a.kind='personal' FOR SHARE OF u,a")
        .bind(users.iter().copied().collect::<Vec<_>>()).fetch_all(&mut *c).await?;
    let actual = rows
        .into_iter()
        .map(|r| {
            Ok((
                r.try_get::<i64, _>("id")?,
                r.try_get::<i64, _>("personal_account_id")?,
            ))
        })
        .collect::<std::result::Result<BTreeMap<_, _>, sqlx::Error>>()?;
    if result.values().any(|(u, a)| actual.get(u) != Some(a)) {
        return Err(Error::Conflict);
    }
    // Identity and financial mapping digests are different domains. Bind each
    // formal source/user/account triple to its actual committed opening row.
    for (source, (_, account)) in &result {
        let covered: bool = sqlx::query_scalar(
            r#"
            SELECT EXISTS (
                SELECT FROM core_billing.opening_imports o
                JOIN core_billing.opening_import_runs r
                  ON r.migration_id = o.migration_id
                WHERE r.source_archive_sha256 = $1
                  AND r.observed_at = $2
                  AND o.account_id = $3
                  AND o.source_entity_key IN (
                      'legacy.users/' || $4::text,
                      'user:' || $4::text
                  )
                  AND (o.request->>'account_id')::bigint = $3
                  AND o.request->>'source_entity_key' = o.source_entity_key
                  AND o.request->>'source_archive_sha256' = r.source_archive_sha256
                  AND o.request->>'mapping_sha256' = r.mapping_sha256
                  AND o.request->>'migration_id' = r.migration_id
                  AND (o.request->>'observed_at')::bigint = r.observed_at
            )
            "#,
        )
        .bind(&input.source_archive_sha256)
        .bind(input.source_cutoff)
        .bind(account)
        .bind(source)
        .fetch_one(&mut *c)
        .await?;
        if !covered {
            return Err(Error::Conflict);
        }
    }
    Ok(result)
}

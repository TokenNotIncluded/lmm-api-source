//! Synthetic local PostgreSQL only. Historical seed never posts another reward.
use lmm_core::{
    billing::ledger::Ledger,
    checkins::{
        self, CheckinStore, ClaimRequest, Error, Policy, PublishPolicy, SeedHistory, SeedReceipt,
    },
    identity::IdentityStore,
};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use sqlx::{PgPool, types::Json};
use std::{collections::BTreeMap, sync::Arc};
use tokio::sync::Barrier;
#[path = "common/runtime.rs"]
mod runtime;

fn sha(bytes: &[u8]) -> String {
    Sha256::digest(bytes)
        .iter()
        .map(|v| format!("{v:02x}"))
        .collect()
}
fn source_policy(min: i64, max: i64, seed: &SeedReceipt) -> Policy {
    Policy {
        enabled: true,
        min_quota: min.to_string(),
        max_quota: max.to_string(),
        level_multipliers: vec![0.0, 0.2, 0.7, 1.0, 1.5],
        calendar_offset_seconds: 28_800,
        credits_per_usd: 500_000,
        source_revision: checkins::SOURCE_REVISION.into(),
        source_options_sha256: seed.source_options_sha256.clone(),
        history_seed_id: seed.seed_id.clone(),
    }
}
fn claim(key: &str) -> ClaimRequest {
    ClaimRequest {
        operation_key: key.into(),
    }
}
struct Fixture {
    store: CheckinStore,
    user: String,
    admin: String,
    account: i64,
    seed: SeedReceipt,
    input: SeedHistory,
    at: i64,
}
async fn fixture(pool: &PgPool, min: i64, max: i64, historical_today: bool) -> Fixture {
    fixture_with_opening(pool, min, max, historical_today, 0).await
}
async fn fixture_with_opening(
    pool: &PgPool,
    min: i64,
    max: i64,
    historical_today: bool,
    available: i64,
) -> Fixture {
    lmm_core::core_schema::init(pool).await.unwrap();
    let installed: bool = sqlx::query_scalar("SELECT to_regnamespace('core_checkins') IS NOT NULL")
        .fetch_one(pool)
        .await
        .unwrap();
    if !installed {
        sqlx::raw_sql(checkins::SCHEMA).execute(pool).await.unwrap();
    }
    let identity = IdentityStore::from_pool(pool.clone());
    let admin = identity.bootstrap_user(61000, 6).await.unwrap().secret;
    let user = identity.bootstrap_user(61001, 0).await.unwrap().secret;
    let account = identity.authorize(&user).await.unwrap().owner_account_id;
    assert_ne!(
        account, 61001,
        "physical ledger account is independently allocated"
    );
    Ledger::from_pool(pool.clone())
        .open_wallet(account)
        .await
        .unwrap();
    sqlx::query("INSERT INTO core_trust_pricing.user_bases(user_id,original_created_at,console_activated_at,last_api_activity_at,override_level,projection_known,order_count,revision,source_digest) VALUES(61001,floor(extract(epoch FROM clock_timestamp()))::bigint,0,0,0,true,0,1,decode(repeat('11',32),'hex'))")
        .execute(pool).await.unwrap();
    let at: i64 = sqlx::query_scalar("SELECT floor(extract(epoch FROM clock_timestamp()))::bigint")
        .fetch_one(pool)
        .await
        .unwrap();
    let map = format!(
        "{{\"source_user_id\":101,\"target_user_id\":61001,\"target_personal_account_id\":{account}}}\n"
    );
    let archive = "a".repeat(64);
    let mapping = "b".repeat(64);
    let identity_manifest = json!({"scope":"identity-only","source_archive_sha256":archive,"mapping_sha256":mapping,"source_target_id_map_sha256":sha(map.as_bytes())});
    sqlx::query("INSERT INTO core_migration.import_runs(migration_id,source_archive_sha256,mapping_sha256,manifest) VALUES('synthetic-checkin-identity',$1,$2,$3)")
        .bind(&archive).bind(&mapping).bind(Json(identity_manifest)).execute(pool).await.unwrap();
    let row = json!({"id":1,"user_id":101,"checkin_date":checkins::calendar_day(at).unwrap(),"quota_awarded":17,"created_at":at});
    let ordered: BTreeMap<_, _> = row.as_object().unwrap().iter().collect();
    let rows = if historical_today {
        vec![
            json!({"record":row,"source_row_sha256":sha(&serde_json::to_vec(&ordered).unwrap()),"source_user_present":true}),
        ]
    } else {
        vec![]
    };
    let history = json!({"scope":"checkin-history.v1","migration_id":"synthetic-checkin-history","source_namespace":"lmm_api.lmm_prod_20260802.checkins",
        "source_archive_sha256":archive,"source_export_sha256":"c".repeat(64),"observed_at":at,"source_row_count":rows.len(),
        "source_user_ids":[101],"rows":rows,"balance_effect":"none","award_unit":"legacy_raw_no_conversion","history_complete":true,
        "native_checkin_enabled_by_import":false,"legacy_calendar_timezone":null,
        "settings":{"checkin_setting.enabled":"true","checkin_setting.min_quota":min.to_string(),
          "checkin_setting.max_quota":max.to_string(),"checkin_setting.level_multipliers":"[0,0.2,0.7,1,1.5]"}});
    sqlx::query("INSERT INTO core_migration.import_runs(migration_id,source_archive_sha256,mapping_sha256,manifest) VALUES('synthetic-checkin-history',$1,$2,$3)")
        .bind(&archive).bind(&mapping).bind(Json(history)).execute(pool).await.unwrap();
    let opening = json!({"migration_id":"synthetic-checkin-opening","source_archive_sha256":archive,"mapping_sha256":"d".repeat(64),
        "source_entity_key":"legacy.users/101","source_row_sha256":"e".repeat(64),"account_id":account,
        "available_units":available,"reserved_units":0,"affiliate_units":0,"observed_at":at});
    let mut opening_tx = pool.begin().await.unwrap();
    sqlx::raw_sql("SET TRANSACTION ISOLATION LEVEL SERIALIZABLE; SET LOCAL synchronous_commit=on")
        .execute(&mut *opening_tx)
        .await
        .unwrap();
    sqlx::query("SELECT core_billing.import_opening_balance($1)")
        .bind(Json(opening))
        .execute(&mut *opening_tx)
        .await
        .unwrap();
    opening_tx.commit().await.unwrap();
    let input = SeedHistory {
        operation_key: "seed_checkin_history_0001".into(),
        history_migration_id: "synthetic-checkin-history".into(),
        identity_migration_id: "synthetic-checkin-identity".into(),
        source_archive_sha256: archive,
        mapping_sha256: mapping,
        source_cutoff: at,
        identity_map_jsonl: map,
    };
    let store = CheckinStore::new(pool.clone());
    let seed = store.seed_history(input.clone()).await.unwrap();
    store
        .publish_policy(PublishPolicy {
            operation_key: "publish_checkin_policy_0001".into(),
            expected_revision: None,
            policy: source_policy(min, max, &seed),
            source_options: None,
        })
        .await
        .unwrap();
    Fixture {
        store,
        user,
        admin,
        account,
        seed,
        input,
        at,
    }
}
async fn journals(pool: &PgPool) -> i64 {
    sqlx::query_scalar("SELECT count(*) FROM core_billing.ledger_journals")
        .fetch_one(pool)
        .await
        .unwrap()
}
async fn override_level(pool: &PgPool, level: i16) {
    sqlx::query("UPDATE core_trust_pricing.user_bases SET override_level=$1,revision=revision+1 WHERE user_id=61001").bind(level).execute(pool).await.unwrap();
}

#[test]
fn exact_cc52_ranges_round_clamp_and_calendar() {
    let seed:SeedReceipt=serde_json::from_value(json!({"seed_id":"d".repeat(64),"history_migration_id":"h","identity_migration_id":"i",
        "source_archive_sha256":"a".repeat(64),"mapping_sha256":"b".repeat(64),"source_cutoff":1,"source_options_sha256":"c".repeat(64),
        "source_rows":0,"mapped_rows":0,"orphan_rows":0,"balance_effect":"none"})).unwrap();
    let policy = source_policy(1, 500000, &seed);
    for (level, (min, max)) in [(0, 0), (0, 100000), (1, 350000), (1, 500000), (2, 750000)]
        .into_iter()
        .enumerate()
    {
        let range = policy.reward_range(level as u8).unwrap();
        assert_eq!(
            (range.min_quota, range.max_quota),
            (min.to_string(), max.to_string())
        );
    }
    assert_eq!(
        policy.reward_range(6).unwrap().max_quota,
        policy.reward_range(4).unwrap().max_quota
    );
    let boundary = chrono::DateTime::parse_from_rfc3339("2026-11-11T16:00:00Z")
        .unwrap()
        .timestamp();
    assert_eq!(checkins::calendar_day(boundary - 1).unwrap(), "2026-11-11");
    assert_eq!(checkins::calendar_day(boundary).unwrap(), "2026-11-12");
    let mut fallback = policy.clone();
    fallback.level_multipliers.clear();
    assert_eq!(fallback.reward_range(0).unwrap().min_quota, "1");
    assert_eq!(fallback.reward_range(0).unwrap().max_quota, "250000");
    fallback.level_multipliers = vec![-1.0];
    assert_eq!(fallback.reward_range(0).unwrap().max_quota, "250000");
    fallback.level_multipliers = vec![0.0];
    assert_eq!(fallback.reward_range(0).unwrap().max_quota, "0");
    let mut zero = policy.clone();
    zero.min_quota = "-5".into();
    zero.max_quota = "-10".into();
    assert_eq!(zero.reward_range(4).unwrap().max_quota, "0");
}

#[sqlx::test]
async fn zero_dynamic_trust_revision_and_durable_operation_replay(pool: PgPool) {
    let f = fixture(&pool, 1, 500000, false).await;
    let before = journals(&pool).await;
    let status = f.store.status(&f.user, None).await.unwrap();
    assert!(status.enabled);
    assert_eq!(status.reward_range.unwrap().max_quota, "0");
    let receipt = f
        .store
        .claim(&f.user, claim("zero_checkin_first_key_0001"))
        .await
        .unwrap();
    assert!(!receipt.already_claimed);
    assert_eq!(receipt.record.quota_awarded, "0");
    assert_eq!(receipt.record.journal_id, None);
    assert_eq!(journals(&pool).await, before);
    override_level(&pool, 3).await;
    let status = f.store.status(&f.user, None).await.unwrap();
    assert!(status.checked_in_today);
    assert_eq!(status.reward_range.unwrap().max_quota, "500000");
    assert_eq!(
        f.store
            .claim(&f.user, claim("zero_checkin_first_key_0001"))
            .await
            .unwrap(),
        receipt
    );
    f.store
        .publish_policy(PublishPolicy {
            operation_key: "publish_checkin_policy_0002".into(),
            expected_revision: Some(1),
            policy: source_policy(1, 500000, &f.seed),
            source_options: None,
        })
        .await
        .unwrap();
    let duplicate = f
        .store
        .claim(&f.user, claim("zero_checkin_second_key_0002"))
        .await
        .unwrap();
    assert!(duplicate.already_claimed);
    assert_eq!(duplicate.record, receipt.record);
    assert_eq!(journals(&pool).await, before);
    let admin = f.store.status(&f.admin, None).await.unwrap();
    assert_eq!(admin.reward_range.unwrap().trust_level, 6);
    // A durable earlier-day command is returned before today's new eligibility.
    let mut earlier = receipt.clone();
    earlier.record.checkin_date = "2026-01-01".into();
    sqlx::query("INSERT INTO core_checkins.claim_commands(user_id,operation_key,result) VALUES(61001,'prior_day_lost_ack_0001',$1)")
        .bind(Json(&earlier)).execute(&pool).await.unwrap();
    let reopened = CheckinStore::new(pool.clone());
    assert_eq!(
        reopened
            .claim(&f.user, claim("prior_day_lost_ack_0001"))
            .await
            .unwrap(),
        earlier
    );
}

#[sqlx::test]
async fn complete_history_seed_formal_mapping_and_no_recredit(pool: PgPool) {
    let f = fixture(&pool, 1, 500000, true).await;
    let before = journals(&pool).await;
    assert_eq!(
        (f.seed.source_rows, f.seed.mapped_rows, f.seed.orphan_rows),
        (1, 1, 0)
    );
    let status = f.store.status(&f.user, None).await.unwrap();
    assert!(status.checked_in_today);
    assert_eq!(status.today_record.unwrap().quota_awarded, "17");
    let receipt = f
        .store
        .claim(&f.user, claim("seeded_today_cannot_claim_0001"))
        .await
        .unwrap();
    assert!(receipt.already_claimed);
    assert_eq!(receipt.record.origin, "history");
    assert_eq!(receipt.record.journal_id, None);
    let mut replay = f.input.clone();
    replay.operation_key = "seed_new_operation_same_history_0002".into();
    assert_eq!(f.store.seed_history(replay).await.unwrap(), f.seed);
    let mut forged = f.input.clone();
    forged.operation_key = "seed_corrupted_map_0003".into();
    forged.identity_map_jsonl = forged
        .identity_map_jsonl
        .replace(&f.account.to_string(), "99999999");
    assert_eq!(f.store.seed_history(forged).await, Err(Error::Conflict));
    let mut wrong_cutoff = f.input;
    wrong_cutoff.operation_key = "seed_wrong_cutoff_0004".into();
    wrong_cutoff.source_cutoff = f.at + 1;
    assert_eq!(
        f.store.seed_history(wrong_cutoff).await,
        Err(Error::Conflict)
    );
    assert_eq!(journals(&pool).await, before);
}

#[sqlx::test]
async fn gift_repays_debt_and_failure_rolls_back_every_reward_record(pool: PgPool) {
    let f = fixture_with_opening(&pool, 100, 100, false, -70).await;
    override_level(&pool, 3).await;
    let ledger = Ledger::from_pool(pool.clone());
    let before = ledger.balance(f.account).await.unwrap();
    assert_eq!(before.available_units, -70);
    sqlx::raw_sql("CREATE FUNCTION core_checkins.fixture_reject_day() RETURNS trigger LANGUAGE plpgsql AS $$BEGIN RAISE EXCEPTION 'synthetic persistence fault'; END$$; CREATE TRIGGER fixture_reject_day BEFORE INSERT ON core_checkins.days FOR EACH ROW EXECUTE FUNCTION core_checkins.fixture_reject_day();")
        .execute(&pool).await.unwrap();
    let count = journals(&pool).await;
    assert_eq!(
        f.store
            .claim(&f.user, claim("fault_then_retry_same_key_0001"))
            .await,
        Err(Error::Unavailable)
    );
    assert_eq!(ledger.balance(f.account).await.unwrap(), before);
    assert_eq!(journals(&pool).await, count);
    let rows:i64=sqlx::query_scalar("SELECT (SELECT count(*) FROM core_checkins.days)+(SELECT count(*) FROM core_checkins.claim_commands)").fetch_one(&pool).await.unwrap();
    assert_eq!(rows, 0);
    sqlx::raw_sql("DROP TRIGGER fixture_reject_day ON core_checkins.days; DROP FUNCTION core_checkins.fixture_reject_day();").execute(&pool).await.unwrap();
    let receipt = f
        .store
        .claim(&f.user, claim("fault_then_retry_same_key_0001"))
        .await
        .unwrap();
    assert_eq!(receipt.record.quota_awarded, "100");
    assert!(receipt.record.journal_id.is_some());
    let after = ledger.balance(f.account).await.unwrap();
    assert_eq!(after.available_units, 30);
    assert_eq!(after.spendable_units, 30);
    assert_eq!(after.debt_units, 0);
}

#[sqlx::test]
async fn concurrent_distinct_keys_create_one_gift_and_runtime_cannot_publish(pool: PgPool) {
    let f = fixture(&pool, 5, 5, false).await;
    override_level(&pool, 3).await;
    let barrier = Arc::new(Barrier::new(6));
    let mut tasks = Vec::new();
    for i in 0..6 {
        let store = f.store.clone();
        let secret = f.user.clone();
        let gate = barrier.clone();
        tasks.push(tokio::spawn(async move {
            gate.wait().await;
            for _ in 0..20 {
                match store
                    .claim(&secret, claim(&format!("concurrent_checkin_key_{i:04}")))
                    .await
                {
                    Err(Error::Retry) => tokio::task::yield_now().await,
                    result => return result.unwrap(),
                }
            }
            panic!("serializable claim did not converge");
        }));
    }
    let mut first = 0;
    for task in tasks {
        let result = task.await.unwrap();
        first += usize::from(!result.already_claimed);
        assert_eq!(result.record.quota_awarded, "5");
    }
    assert_eq!(first, 1);
    assert_eq!(journals(&pool).await, 1);
    let runtime = runtime::RuntimeRole::create(&pool).await;
    sqlx::raw_sql(&format!("GRANT USAGE ON SCHEMA core_checkins TO {}; GRANT SELECT ON ALL TABLES IN SCHEMA core_checkins TO {};",runtime.name,runtime.name)).execute(&pool).await.unwrap();
    let unprivileged = CheckinStore::new(runtime.pool.clone());
    assert_eq!(
        unprivileged
            .publish_policy(PublishPolicy {
                operation_key: "runtime_cannot_publish_0001".into(),
                expected_revision: Some(1),
                policy: source_policy(5, 5, &f.seed),
                source_options: None
            })
            .await,
        Err(Error::Forbidden)
    );
    runtime.cleanup().await;
}

#[path = "checkins/runtime_wiring.rs"]
mod runtime_wiring;

#[sqlx::test]
async fn actual_csprng_claim_preserves_inclusive_range_and_zero_journal_semantics(pool: PgPool) {
    let f = fixture(&pool, 0, 2, false).await;
    override_level(&pool, 3).await;
    let result = f
        .store
        .claim(&f.user, claim("actual_csprng_bounded_claim_0001"))
        .await
        .unwrap();
    let amount = result.record.quota_awarded.parse::<i64>().unwrap();
    assert!((0..=2).contains(&amount));
    assert_eq!(result.record.journal_id.is_none(), amount == 0);
    assert_eq!(
        Ledger::from_pool(pool.clone())
            .balance(f.account)
            .await
            .unwrap()
            .available_units,
        amount
    );
    assert_eq!(
        f.store
            .claim(&f.user, claim("actual_csprng_bounded_claim_0001"))
            .await
            .unwrap(),
        result
    );
}

#[sqlx::test]
async fn seed_rejects_formal_personal_account_without_matching_source_opening(pool: PgPool) {
    let f = fixture(&pool, 1, 500000, false).await;
    let mut input = f.input.clone();
    input.operation_key = "seed_wrong_source_opening_0001".into();
    input.history_migration_id = "synthetic-wrong-source-history".into();
    input.identity_migration_id = "synthetic-wrong-source-identity".into();
    input.mapping_sha256 = "f".repeat(64);
    input.identity_map_jsonl = input
        .identity_map_jsonl
        .replace("\"source_user_id\":101", "\"source_user_id\":201");
    let identity = json!({"scope":"identity-only","source_archive_sha256":input.source_archive_sha256,
        "mapping_sha256":input.mapping_sha256,"source_target_id_map_sha256":sha(input.identity_map_jsonl.as_bytes())});
    let mut history: Value =
        sqlx::query_scalar("SELECT manifest FROM core_migration.import_runs WHERE migration_id=$1")
            .bind(&f.input.history_migration_id)
            .fetch_one(&pool)
            .await
            .unwrap();
    history["migration_id"] = json!(input.history_migration_id);
    history["source_user_ids"] = json!([201]);
    for (id, manifest) in [
        (&input.identity_migration_id, identity),
        (&input.history_migration_id, history),
    ] {
        sqlx::query("INSERT INTO core_migration.import_runs(migration_id,source_archive_sha256,mapping_sha256,manifest) VALUES($1,$2,$3,$4)")
            .bind(id).bind(&input.source_archive_sha256).bind(&input.mapping_sha256).bind(Json(manifest)).execute(&pool).await.unwrap();
    }
    let before = journals(&pool).await;
    assert_eq!(f.store.seed_history(input).await, Err(Error::Conflict));
    assert_eq!(journals(&pool).await, before);
    let count: i64 = sqlx::query_scalar("SELECT count(*) FROM core_checkins.seed_receipts")
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(count, 1);
}

#[sqlx::test]
async fn day_receipt_missing_text_and_nonobject_are_rejected(pool: PgPool) {
    let f = fixture(&pool, 1, 500000, false).await;
    let valid = json!({"checkin_date":checkins::calendar_day(f.at).unwrap(),"quota_awarded":"0",
        "created_at":f.at,"journal_id":null,"origin":"native","policy_revision":1,"trust_level":0});
    let mut cases = vec![json!([])];
    for field in ["checkin_date", "quota_awarded", "origin"] {
        let mut malformed = valid.clone();
        malformed.as_object_mut().unwrap().remove(field);
        cases.push(malformed);
    }
    for malformed in cases {
        let error=sqlx::query("INSERT INTO core_checkins.days(account_id,user_id,checkin_date,amount_units,origin,policy_revision,created_at,result) VALUES($1,61001,$2::text::date,0,'native',1,$3,$4)")
            .bind(f.account).bind(checkins::calendar_day(f.at).unwrap()).bind(f.at).bind(Json(malformed)).execute(&pool).await.unwrap_err();
        assert_eq!(
            error.as_database_error().unwrap().code().as_deref(),
            Some("23514")
        );
    }
    assert_eq!(journals(&pool).await, 0);
    let rows: i64 = sqlx::query_scalar("SELECT count(*) FROM core_checkins.days")
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(rows, 0);
}

#[sqlx::test]
async fn exact_runtime_acl_rejects_owner_table_and_column_privileges(pool: PgPool) {
    let f = fixture(&pool, 1, 500000, false).await;
    let runtime = runtime::RuntimeRole::create(&pool).await;
    let grants = include_str!("../../../deployment/postgres/grant-core-runtime.sql")
        .split_once("-- BEGIN checkins runtime grants\n")
        .unwrap()
        .1
        .split_once("-- END checkins runtime grants")
        .unwrap()
        .0
        .replace(":\"runtime_role\"", &runtime.name);
    sqlx::raw_sql(&grants).execute(&pool).await.unwrap();
    let required = lmm_core::runtime_database::RequiredCapabilities {
        checkins: true,
        ..Default::default()
    };
    lmm_core::runtime_database::check(&runtime.pool, required)
        .await
        .unwrap();
    override_level(&pool, 3).await;
    let live = CheckinStore::new(runtime.pool.clone());
    let receipt = live
        .claim(&f.user, claim("runtime_actual_checkin_gift_0001"))
        .await
        .unwrap();
    let amount = receipt.record.quota_awarded.parse::<i64>().unwrap();
    assert!((1..=500000).contains(&amount));
    assert!(receipt.record.journal_id.is_some());
    assert_eq!(
        Ledger::from_pool(pool.clone())
            .balance(f.account)
            .await
            .unwrap()
            .available_units,
        amount
    );
    sqlx::raw_sql(&format!(
        "REVOKE SELECT ON core_checkins.active_policy FROM {}",
        runtime.name
    ))
    .execute(&pool)
    .await
    .unwrap();
    assert_eq!(
        lmm_core::runtime_database::check(&runtime.pool, required).await,
        Err(lmm_core::runtime_database::RuntimeDatabaseError::MissingPrivileges)
    );
    sqlx::raw_sql(&grants).execute(&pool).await.unwrap();
    for (grant, revoke) in [
        (
            "GRANT INSERT ON core_checkins.policy_revisions TO",
            "REVOKE INSERT ON core_checkins.policy_revisions FROM",
        ),
        (
            "GRANT UPDATE(revision) ON core_checkins.active_policy TO",
            "REVOKE UPDATE(revision) ON core_checkins.active_policy FROM",
        ),
        (
            "GRANT SELECT ON core_checkins.seed_receipts TO",
            "REVOKE SELECT ON core_checkins.seed_receipts FROM",
        ),
        (
            "GRANT SELECT(operation_key) ON core_checkins.owner_commands TO",
            "REVOKE SELECT(operation_key) ON core_checkins.owner_commands FROM",
        ),
    ] {
        sqlx::raw_sql(&format!("{grant} {}", runtime.name))
            .execute(&pool)
            .await
            .unwrap();
        assert_eq!(
            lmm_core::runtime_database::check(&runtime.pool, required).await,
            Err(lmm_core::runtime_database::RuntimeDatabaseError::UnsafePrivileges)
        );
        sqlx::raw_sql(&format!("{revoke} {}", runtime.name))
            .execute(&pool)
            .await
            .unwrap();
    }
    lmm_core::runtime_database::check(&runtime.pool, required)
        .await
        .unwrap();
    runtime.cleanup().await;
}

#[sqlx::test]
async fn owner_range_update_is_audited_cas_and_preserves_old_day(pool: PgPool) {
    let f = fixture(&pool, 5, 5, false).await;
    override_level(&pool, 3).await;
    let original = f
        .store
        .claim(&f.user, claim("before_owner_range_update_0001"))
        .await
        .unwrap();
    let old: Value =
        sqlx::query_scalar("SELECT policy FROM core_checkins.policy_revisions WHERE revision=1")
            .fetch_one(&pool)
            .await
            .unwrap();
    let options: BTreeMap<String, String> = [
        ("checkin_setting.enabled", "true"),
        ("checkin_setting.min_quota", "5"),
        ("checkin_setting.max_quota", "10"),
        ("checkin_setting.level_multipliers", "[0,0.2,0.7,1,1.5]"),
    ]
    .into_iter()
    .map(|(a, b)| (a.into(), b.into()))
    .collect();
    let mut policy = source_policy(5, 10, &f.seed);
    policy.source_options_sha256 = sha(&serde_json::to_vec(&options).unwrap());
    let input = PublishPolicy {
        operation_key: "owner_range_update_0001".into(),
        expected_revision: Some(1),
        policy,
        source_options: Some(options),
    };
    let updated = f.store.publish_policy(input.clone()).await.unwrap();
    assert_eq!(updated.revision, 2);
    assert_eq!(
        f.store.publish_policy(input.clone()).await.unwrap(),
        updated
    );
    let mut stale = input;
    stale.operation_key = "stale_owner_range_update_0002".into();
    assert_eq!(f.store.publish_policy(stale).await, Err(Error::Conflict));
    assert_eq!(
        f.store
            .status(&f.user, None)
            .await
            .unwrap()
            .reward_range
            .unwrap()
            .max_quota,
        "10"
    );
    assert_eq!(
        f.store
            .claim(&f.user, claim("after_owner_range_update_0002"))
            .await
            .unwrap()
            .record,
        original.record
    );
    let unchanged: Value =
        sqlx::query_scalar("SELECT policy FROM core_checkins.policy_revisions WHERE revision=1")
            .fetch_one(&pool)
            .await
            .unwrap();
    assert_eq!(unchanged, old);
    let original_seed_settings: Value =
        sqlx::query_scalar("SELECT settings FROM core_checkins.seed_receipts WHERE seed_id=$1")
            .bind(&f.seed.seed_id)
            .fetch_one(&pool)
            .await
            .unwrap();
    assert_eq!(original_seed_settings["checkin_setting.max_quota"], "5");
}

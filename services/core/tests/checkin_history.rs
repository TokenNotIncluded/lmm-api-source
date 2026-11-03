//! Synthetic real-PostgreSQL/HTTP regressions, not independent migration acceptance.
#[path = "common/database_snapshot.rs"]
mod database_snapshot;
#[path = "common/runtime.rs"]
mod runtime;
use axum::{
    Router,
    body::{Body, to_bytes},
    http::Request,
};
use lmm_core::{
    accounts::{Account, AccountKind},
    core_schema,
    funding::FundingPolicy,
    http::{Capabilities, configured_router},
    identity::IdentityStore,
};
use serde_json::{Value, json};
use sqlx::PgPool;
use tower::ServiceExt;

async fn read(app: &Router, secret: &str, query: &str) -> (u16, Value) {
    let mut request = Request::builder().uri(format!("/core/v1/checkins/history?{query}"));
    if !secret.is_empty() {
        request = request.header("Authorization", format!("Bearer {secret}"));
    }
    let response = app
        .clone()
        .oneshot(request.body(Body::empty()).unwrap())
        .await
        .unwrap();
    let status = response.status().as_u16();
    assert_eq!(response.headers()["cache-control"], "no-store");
    let body = to_bytes(response.into_body(), 1 << 20).await.unwrap();
    (status, serde_json::from_slice(&body).unwrap())
}

#[sqlx::test(migrations = false)]
async fn checkin_history_is_source_bound_session_scoped_and_never_replays_rewards(pool: PgPool) {
    core_schema::init(&pool).await.unwrap();
    let identity = IdentityStore::from_pool(pool.clone());
    let mut sessions = vec![];
    for id in [7, 8, 9, 10] {
        sessions.push(identity.bootstrap_user(id, 1).await.unwrap());
    }
    let key = identity
        .issue_key(
            &sessions[0].secret,
            Account {
                kind: AccountKind::Personal,
                id: 7,
            },
            FundingPolicy::inherit_account_settings(),
            3600,
        )
        .await
        .unwrap();
    let role = runtime::RuntimeRole::create(&pool).await;
    role.optional("web_identity").await;
    let broad_receipt_access: bool = sqlx::query_scalar("SELECT has_column_privilege('core_migration.import_runs','source_archive_sha256','SELECT')")
        .fetch_one(&role.pool).await.unwrap();
    assert!(
        !broad_receipt_access,
        "test must use the two-column receipt grant"
    );
    let app = configured_router(
        role.pool.clone(),
        Capabilities {
            identity: true,
            ..Default::default()
        },
    );
    assert_eq!(
        read(&app, &sessions[0].secret, "month=2026-11").await.0,
        503
    );
    let rows = vec![
        (7, "2026-11-01", 9007199254740991_i64, true),
        (7, "2026-11-02", 0, true),
        (7, "2026-10-31", 17, true),
        (8, "2026-11-01", 50, true),
        (9, "2026-11-01", 88, false),
    ];
    let rows:Vec<Value>=rows.into_iter().enumerate().map(|(i,(user,day,award,present))|json!({"record":{"id":i+1,"user_id":user,"checkin_date":day,"quota_awarded":award,"created_at":1793500000},"source_user_present":present})).collect();
    let manifest = json!({"scope":"checkin-history.v1","source_namespace":"lmm_api.lmm_prod_20260802.checkins","source_archive_sha256":"a".repeat(64),"observed_at":1793674367,"history_complete":true,"source_row_count":5,"source_user_ids":[7,8,10],"balance_effect":"none","award_unit":"legacy_raw_no_conversion","rows":rows});
    sqlx::query("INSERT INTO core_migration.import_runs(migration_id,source_archive_sha256,mapping_sha256,manifest) VALUES('synthetic-checkin',repeat('a',64),repeat('b',64),$1)").bind(&manifest).execute(&pool).await.unwrap();
    assert_eq!(
        read(&app, &sessions[0].secret, "month=2026-11").await.0,
        503,
        "unbound source cannot masquerade as opened history"
    );
    sqlx::query("INSERT INTO core_billing.opening_import_runs VALUES('synthetic-opening',repeat('a',64),repeat('b',64),1793674367)").execute(&pool).await.unwrap();
    let before = database_snapshot::snapshot(&pool).await;
    let (status, response) = read(&app, &sessions[0].secret, "month=2026-11").await;
    assert_eq!(status, 200);
    let data = &response["data"];
    assert_eq!(data["total_checkins"], 3);
    assert_eq!(data["total_raw_quota_awarded"], "9007199254741008");
    assert_eq!(data["records"].as_array().unwrap().len(), 2);
    assert_eq!(data["records"][0]["checkin_date"], "2026-11-02");
    assert_eq!(data["records"][0]["raw_quota_awarded"], 0);
    assert_eq!(data["native_claim_available"], false);
    assert_eq!(data["calendar_timezone"], Value::Null);
    assert_eq!(
        read(&app, &sessions[1].secret, "month=2026-11").await.1["data"]["total_raw_quota_awarded"],
        "50"
    );
    assert_eq!(
        read(&app, &sessions[2].secret, "month=2026-11").await.0,
        503,
        "reused orphan ID gains no archive rights"
    );
    let zero = read(&app, &sessions[3].secret, "month=2026-11").await;
    assert_eq!(zero.0, 200);
    assert_eq!(zero.1["data"]["total_raw_quota_awarded"], "0");
    assert_eq!(zero.1["data"]["records"], json!([]));
    assert_eq!(read(&app, "", "month=2026-11").await.0, 401);
    assert_eq!(read(&app, &key.secret, "month=2026-11").await.0, 403);
    for q in [
        "month=2026-13",
        "month=2026-1",
        "month=0000-01",
        "month=2026-11&user_id=8",
    ] {
        assert_eq!(read(&app, &sessions[0].secret, q).await.0, 400);
    }
    assert_eq!(
        before,
        database_snapshot::snapshot(&pool).await,
        "all history reads must preserve tables and sequences"
    );
    sqlx::query("INSERT INTO core_migration.import_runs(migration_id,source_archive_sha256,mapping_sha256,manifest) VALUES('ambiguous-checkin',repeat('a',64),repeat('b',64),$1)").bind(&manifest).execute(&pool).await.unwrap();
    assert_eq!(
        read(&app, &sessions[0].secret, "month=2026-11").await.0,
        503
    );
    role.cleanup().await;
}

#[sqlx::test(migrations = false)]
#[ignore = "requires the explicit private restored-source check-in manifest"]
async fn restored_history_reads_exactly_for_every_mapped_account(pool: PgPool) {
    use std::{
        collections::{BTreeMap, BTreeSet},
        path::PathBuf,
    };
    let path = PathBuf::from(
        std::env::var_os("LMM_CHECKIN_HISTORY_MANIFEST")
            .expect("explicit private manifest required"),
    )
    .canonicalize()
    .unwrap();
    let backups = PathBuf::from(std::env::var_os("HOME").unwrap())
        .join("Backups")
        .canonicalize()
        .unwrap();
    assert!(
        path.starts_with(backups),
        "only local Backups input allowed"
    );
    let manifest: Value = serde_json::from_slice(&std::fs::read(path).unwrap()).unwrap();
    assert_eq!(manifest["scope"], "checkin-history.v1");
    assert_eq!(
        manifest["source_archive_sha256"],
        "3f02b054dd05db6a1ca880b33e64b10c2bf4f2a22e2bc836fda9b725f512a90f"
    );
    core_schema::init(&pool).await.unwrap();
    let identity = IdentityStore::from_pool(pool.clone());
    let mut sessions = vec![];
    // Synthetic sessions on fresh isolated identities; no original login or live
    // account is touched. Only the archived history and mapping are real facts.
    for value in manifest["source_user_ids"].as_array().unwrap() {
        let id = value.as_i64().unwrap();
        sessions.push((id, identity.bootstrap_user(id, 1).await.unwrap().secret));
    }
    sqlx::query("INSERT INTO core_billing.opening_import_runs VALUES('history-read-fixture',$1,repeat('b',64),$2)")
        .bind(manifest["source_archive_sha256"].as_str().unwrap()).bind(manifest["observed_at"].as_i64().unwrap()).execute(&pool).await.unwrap();
    sqlx::query("INSERT INTO core_migration.import_runs(migration_id,source_archive_sha256,mapping_sha256,manifest) VALUES($1,$2,$3,$4)")
        .bind(manifest["migration_id"].as_str().unwrap()).bind(manifest["source_archive_sha256"].as_str().unwrap()).bind(manifest["mapping_sha256"].as_str().unwrap()).bind(&manifest).execute(&pool).await.unwrap();
    let role = runtime::RuntimeRole::create(&pool).await;
    role.optional("web_identity").await;
    let broad_receipt_access: bool = sqlx::query_scalar("SELECT has_column_privilege('core_migration.import_runs','source_archive_sha256','SELECT')")
        .fetch_one(&role.pool).await.unwrap();
    assert!(
        !broad_receipt_access,
        "test must use the two-column receipt grant"
    );
    let app = configured_router(
        role.pool.clone(),
        Capabilities {
            identity: true,
            ..Default::default()
        },
    );
    let before = database_snapshot::snapshot(&pool).await;
    let mut by_user: BTreeMap<i64, Vec<&Value>> = BTreeMap::new();
    let mut orphans = 0;
    for row in manifest["rows"].as_array().unwrap() {
        if row["source_user_present"] == true {
            by_user
                .entry(row["record"]["user_id"].as_i64().unwrap())
                .or_default()
                .push(&row["record"]);
        } else {
            orphans += 1;
        }
    }
    let mut returned = 0;
    for (id, secret) in &sessions {
        let expected = by_user.get(id).cloned().unwrap_or_default();
        let total: i64 = expected
            .iter()
            .map(|r| r["quota_awarded"].as_i64().unwrap())
            .sum();
        let mut months: BTreeSet<String> = expected
            .iter()
            .map(|r| r["checkin_date"].as_str().unwrap()[..7].to_owned())
            .collect();
        if months.is_empty() {
            months.insert("2026-11".to_owned());
        }
        for month in months {
            let (status, response) = read(&app, secret, &format!("month={month}")).await;
            assert_eq!(status, 200, "mapped account history unavailable");
            let data = &response["data"];
            assert!(
                data["total_checkins"].as_u64() == Some(expected.len() as u64),
                "source count differs"
            );
            assert!(
                data["total_raw_quota_awarded"].as_str() == Some(total.to_string().as_str()),
                "source raw total differs"
            );
            let mut monthly:Vec<_>=expected.iter().filter(|r|r["checkin_date"].as_str().unwrap().starts_with(&month)).map(|r|json!({"checkin_date":r["checkin_date"],"raw_quota_awarded":r["quota_awarded"],"source_created_at":r["created_at"]})).collect();
            monthly.sort_by(|a, b| b["checkin_date"].as_str().cmp(&a["checkin_date"].as_str()));
            assert!(
                data["records"] == json!(monthly),
                "original monthly record fields differ"
            );
            returned += monthly.len();
        }
    }
    assert_eq!(
        returned + orphans,
        manifest["source_row_count"].as_u64().unwrap() as usize
    );
    assert!(
        before == database_snapshot::snapshot(&pool).await,
        "history reads mutated the isolated database"
    );
    println!(
        "restored facts: {} synthetic sessions; {} mapped records read; {} orphan records withheld",
        sessions.len(),
        returned,
        orphans
    );
    role.cleanup().await;
}

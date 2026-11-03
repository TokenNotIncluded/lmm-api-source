//! Source-bound real-PG/HTTP self-tests. No supplier calls or production accounts.
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
    let mut req = Request::builder().uri(format!("/core/v1/hero-sms/history?{query}"));
    if !secret.is_empty() {
        req = req.header("Authorization", format!("Bearer {secret}"));
    }
    let response = app
        .clone()
        .oneshot(req.body(Body::empty()).unwrap())
        .await
        .unwrap();
    let status = response.status().as_u16();
    assert_eq!(response.headers()["cache-control"], "no-store");
    let bytes = to_bytes(response.into_body(), 1 << 20).await.unwrap();
    (status, serde_json::from_slice(&bytes).unwrap())
}
async fn install(pool: &PgPool, m: &Value, opening_observed_at: i64) {
    sqlx::query("INSERT INTO core_migration.import_runs(migration_id,source_archive_sha256,mapping_sha256,manifest) VALUES('sms-fixture',$1,repeat('b',64),$2)").bind(m["source_archive_sha256"].as_str().unwrap()).bind(m).execute(pool).await.unwrap();
    sqlx::query(
        "INSERT INTO core_billing.opening_import_runs VALUES('sms-opening',$1,repeat('b',64),$2)",
    )
    .bind(m["source_archive_sha256"].as_str().unwrap())
    .bind(opening_observed_at)
    .execute(pool)
    .await
    .unwrap();
}
async fn app(pool: &PgPool) -> (Router, runtime::RuntimeRole) {
    let role = runtime::RuntimeRole::create(pool).await;
    role.optional("web_identity").await;
    let access:bool=sqlx::query_scalar("SELECT has_column_privilege('core_migration.import_runs','source_archive_sha256','SELECT')").fetch_one(&role.pool).await.unwrap();
    assert!(!access);
    (
        configured_router(
            role.pool.clone(),
            Capabilities {
                identity: true,
                ..Default::default()
            },
        ),
        role,
    )
}
#[sqlx::test(migrations = false)]
async fn sms_history_keeps_hidden_orders_private_and_never_exposes_payloads_or_replays_money(
    pool: PgPool,
) {
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
    let (router, role) = app(&pool).await;
    assert_eq!(read(&router, &sessions[0].secret, "").await.0, 503);
    let mut orders = vec![];
    let mut ledgers = vec![];
    for (i, (id, user, hidden, refund)) in [
        ("visible", 7, 0, 40),
        ("hidden", 7, 1, 100),
        ("other", 8, 0, 100),
        ("orphan", 9, 0, 100),
    ]
    .into_iter()
    .enumerate()
    {
        orders.push(json!({"id":id,"user_id":user,"history_hidden_at":hidden,"service":"synthetic-service","country_id":1,"operator":"synthetic-operator","status":"completed","customer_price_usd":"1.000000000000000001","reserved_quota":100,"refunded_quota":refund,"created_at":1000,"completed_at":1001,"phone_ciphertext":"DO-NOT-EXPOSE-PAYLOAD","code_ciphertext":"DO-NOT-EXPOSE-PAYLOAD","message_ciphertext":"DO-NOT-EXPOSE-PAYLOAD","provider_snapshot_ciphertext":"DO-NOT-EXPOSE-PAYLOAD","provider_price_cny":"DO-NOT-EXPOSE-UPSTREAM-COST"}));
        ledgers.push(json!({"id":i*2+1,"user_id":user,"order_id":id,"entry_type":"reserve","amount_quota":-100,"created_at":1000}));
        ledgers.push(json!({"id":i*2+2,"user_id":user,"order_id":id,"entry_type":"refund","amount_quota":refund,"created_at":1001}));
    }
    let manifest = json!({"scope":"hero-sms-history.v1","source_namespace":"lmm_api.lmm_prod_20260802.hero_sms_sms","source_archive_sha256":"a".repeat(64),"observed_at":1793674367,"history_complete":true,"source_order_count":4,"source_ledger_count":8,"source_user_ids":[7,8,10],"balance_effect":"none","quota_unit":"legacy_raw_no_conversion","tables":{"hero_sms_sms_orders":orders,"hero_sms_sms_quota_ledgers":ledgers}});
    install(&pool, &manifest, manifest["observed_at"].as_i64().unwrap()).await;
    let before = database_snapshot::snapshot(&pool).await;
    let (status, response) = read(&router, &sessions[0].secret, "p=1&page_size=1").await;
    assert_eq!(status, 200);
    let d = &response["data"];
    assert_eq!(d["total"], 1);
    let o = &d["items"][0];
    assert_eq!(o["id"], "visible");
    assert_eq!(o["quoted_price_usd"], "1.000000000000000001");
    assert_eq!(o["raw_net_debit_units"], "60");
    assert_eq!(o["ledger"][0]["raw_amount_units"], "-100");
    assert_eq!(o["ledger"][1]["raw_amount_units"], "40");
    assert_eq!(d["payload_available"], false);
    assert_eq!(d["supplier_operations_available"], false);
    let body = response.to_string();
    assert!(!body.contains("DO-NOT-EXPOSE"));
    assert!(!body.contains("ciphertext"));
    assert!(!body.contains("hidden"));
    let page2 = read(&router, &sessions[0].secret, "p=2&page_size=1").await;
    assert_eq!(page2.1["data"]["items"], json!([]));
    assert_eq!(page2.1["data"]["total"], 1);
    assert_eq!(
        read(&router, &sessions[1].secret, "").await.1["data"]["items"][0]["id"],
        "other"
    );
    assert_eq!(read(&router, &sessions[2].secret, "").await.0, 503);
    assert_eq!(
        read(&router, &sessions[3].secret, "").await.1["data"]["total"],
        0
    );
    assert_eq!(read(&router, "", "").await.0, 401);
    assert_eq!(read(&router, &key.secret, "").await.0, 403);
    for q in [
        "p=0",
        "p=100001",
        "page_size=101",
        "user_id=8",
        "include_hidden=true",
    ] {
        assert_eq!(read(&router, &sessions[0].secret, q).await.0, 400);
    }
    assert!(
        before == database_snapshot::snapshot(&pool).await,
        "history read mutated database state"
    );
    sqlx::query("INSERT INTO core_migration.import_runs(migration_id,source_archive_sha256,mapping_sha256,manifest) SELECT 'sms-duplicate',source_archive_sha256,mapping_sha256,manifest FROM core_migration.import_runs WHERE migration_id='sms-fixture'")
        .execute(&pool).await.unwrap();
    assert_eq!(read(&router, &sessions[0].secret, "").await.0, 503);
    role.cleanup().await;
}

#[sqlx::test(migrations = false)]
#[ignore = "requires the explicit private restored Hero SMS manifest"]
async fn restored_visible_orders_and_ledgers_match_for_every_mapped_account(pool: PgPool) {
    use std::{collections::BTreeMap, path::PathBuf};
    let path = PathBuf::from(
        std::env::var_os("LMM_SMS_HISTORY_MANIFEST").expect("explicit local manifest required"),
    )
    .canonicalize()
    .unwrap();
    let backups = PathBuf::from(std::env::var_os("HOME").unwrap())
        .join("Backups")
        .canonicalize()
        .unwrap();
    assert!(path.starts_with(backups));
    let m: Value = serde_json::from_slice(&std::fs::read(path).unwrap()).unwrap();
    assert_eq!(m["scope"], "hero-sms-history.v1");
    assert_eq!(
        m["source_archive_sha256"],
        "3f02b054dd05db6a1ca880b33e64b10c2bf4f2a22e2bc836fda9b725f512a90f"
    );
    core_schema::init(&pool).await.unwrap();
    let identity = IdentityStore::from_pool(pool.clone());
    let mut sessions = vec![];
    for id in m["source_user_ids"].as_array().unwrap() {
        let id = id.as_i64().unwrap();
        sessions.push((id, identity.bootstrap_user(id, 1).await.unwrap().secret));
    }
    install(&pool, &m, m["observed_at"].as_i64().unwrap()).await;
    let (router, role) = app(&pool).await;
    let before = database_snapshot::snapshot(&pool).await;
    let source_orders = m["tables"]["hero_sms_sms_orders"].as_array().unwrap();
    let source_ledgers = m["tables"]["hero_sms_sms_quota_ledgers"]
        .as_array()
        .unwrap();
    let mut by_user: BTreeMap<i64, Vec<&Value>> = BTreeMap::new();
    let mut hidden = 0;
    for order in source_orders {
        if order["history_hidden_at"] == 0 {
            by_user
                .entry(order["user_id"].as_i64().unwrap())
                .or_default()
                .push(order);
        } else {
            hidden += 1;
        }
    }
    let (mut order_count, mut ledger_count) = (0, 0);
    for (user, secret) in &sessions {
        let mut expected = by_user.get(user).cloned().unwrap_or_default();
        expected.sort_by(|a, b| {
            b["created_at"]
                .as_i64()
                .cmp(&a["created_at"].as_i64())
                .then(b["id"].as_str().cmp(&a["id"].as_str()))
        });
        let (status, r) = read(&router, secret, "page_size=100").await;
        assert_eq!(status, 200);
        let data = &r["data"];
        assert!(data["total"].as_u64() == Some(expected.len() as u64));
        let items = data["items"].as_array().unwrap();
        assert_eq!(items.len(), expected.len());
        for (item, original) in items.iter().zip(expected) {
            let fields = [
                ("id", "id"),
                ("service", "service"),
                ("country_id", "country_id"),
                ("operator", "operator"),
                ("status", "status"),
                ("quoted_price_usd", "customer_price_usd"),
                ("created_at", "created_at"),
                ("completed_at", "completed_at"),
            ];
            assert!(
                fields.iter().all(|(a, b)| item[a] == original[b]),
                "original order projection differs"
            );
            let reserved = original["reserved_quota"].as_i64().unwrap();
            let refunded = original["refunded_quota"].as_i64().unwrap();
            assert!(
                item["raw_reserved_units"] == reserved.to_string()
                    && item["raw_refunded_units"] == refunded.to_string()
                    && item["raw_net_debit_units"] == (reserved - refunded).to_string(),
                "original raw quota differs"
            );
            let mut expected_ledger: Vec<_> = source_ledgers
                .iter()
                .filter(|l| l["user_id"] == *user && l["order_id"] == original["id"])
                .collect();
            expected_ledger.sort_by_key(|l| (l["created_at"].as_i64(), l["id"].as_i64()));
            let expected_ledger:Vec<_>=expected_ledger.into_iter().map(|l|json!({"entry_type":l["entry_type"],"raw_amount_units":l["amount_quota"].as_i64().unwrap().to_string(),"created_at":l["created_at"]})).collect();
            assert!(
                item["ledger"] == json!(expected_ledger),
                "original ledger projection differs"
            );
            order_count += 1;
            ledger_count += expected_ledger.len();
        }
    }
    assert_eq!(order_count + hidden, source_orders.len());
    assert!(before == database_snapshot::snapshot(&pool).await);
    role.cleanup().await;
    println!(
        "restored facts: {} synthetic sessions; {} visible orders and {} ledger entries exact; {} hidden orders withheld",
        sessions.len(),
        order_count,
        ledger_count,
        hidden
    );
}

#[sqlx::test(migrations = false)]
async fn sms_history_requires_the_same_frozen_opening_boundary(pool: PgPool) {
    core_schema::init(&pool).await.unwrap();
    let session = IdentityStore::from_pool(pool.clone())
        .bootstrap_user(7, 1)
        .await
        .unwrap();
    let m = json!({"scope":"hero-sms-history.v1","source_namespace":"lmm_api.lmm_prod_20260802.hero_sms_sms","source_archive_sha256":"a".repeat(64),"observed_at":1793674367,"history_complete":true,"source_order_count":0,"source_ledger_count":0,"source_user_ids":[7],"balance_effect":"none","quota_unit":"legacy_raw_no_conversion","tables":{"hero_sms_sms_orders":[],"hero_sms_sms_quota_ledgers":[]}});
    // Both receipts stay immutable: first install a different opening boundary.
    install(&pool, &m, 1793674366).await;
    let (router, role) = app(&pool).await;
    assert_eq!(read(&router, &session.secret, "").await.0, 503);
    sqlx::query("INSERT INTO core_billing.opening_import_runs VALUES('sms-opening-matching',repeat('a',64),repeat('b',64),1793674367)").execute(&pool).await.unwrap();
    let (status, response) = read(&router, &session.secret, "").await;
    assert_eq!(status, 200);
    assert_eq!(response["data"]["total"], 0);
    role.cleanup().await;
}

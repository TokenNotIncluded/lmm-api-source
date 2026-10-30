use axum::http::{StatusCode, header};
use lmm_core::{
    core_schema,
    identity::IdentityStore,
    operations::{Heartbeat, OperationsError, OperationsStore, router},
};
use serde_json::Value;
use sqlx::PgPool;
#[path = "common/runtime.rs"]
mod runtime;
use axum::{
    body::{Body, to_bytes},
    http::Request,
};
use runtime::RuntimeRole;
use tower::ServiceExt;

async fn source(pool: &PgPool, archive: &str) -> i64 {
    sqlx::query_scalar("INSERT INTO core_history.sources(contract_version,source_archive_sha256,mapping_sha256,source_namespace,observed_at,complete) VALUES('core.history-source.v1',$1,$1,'synthetic',1,true) RETURNING source_id")
        .bind(archive.repeat(64)).fetch_one(pool).await.unwrap()
}
async fn logs(pool: &PgPool, source: i64, user: i64, owner: i64, count: i64, at: i64) {
    sqlx::query("INSERT INTO core_history.log_records(source_id,source_log_id,canonical_event_key,source_row_sha256,user_id,owner_account_id,created_at,log_type) SELECT $1,g,$1::text||':'||g::text,repeat('a',64),$2,$3,$4,2 FROM generate_series(1,$5::bigint) g")
        .bind(source).bind(user).bind(owner).bind(at).bind(count).execute(pool).await.unwrap();
}

#[sqlx::test(migrations = false)]
async fn log_cleanup_resumes_committed_batches_and_preserves_native_evidence(pool: PgPool) {
    core_schema::init(&pool).await.unwrap();
    let identity = IdentityStore::from_pool(pool.clone());
    let admin = identity.bootstrap_user(1101, 6).await.unwrap();
    let user = identity.bootstrap_user(1102, 0).await.unwrap();
    let owner = identity
        .authorize(&user.secret)
        .await
        .unwrap()
        .owner_account_id;
    let ledger = lmm_core::billing::ledger::Ledger::from_pool(pool.clone());
    ledger.open_wallet(owner).await.unwrap();
    let receipt = ledger
        .execute(&lmm_core::billing::ledger::Request {
            scope: "operations-test".into(),
            operation_key: "synthetic-credit".into(),
            actor_user_id: Some(1102),
            reason: "Synthetic cleanup isolation".into(),
            action: lmm_core::billing::ledger::Action::Credit {
                account_id: owner,
                amount_units: lmm_core::billing::Credits::try_from(1000).unwrap(),
                payment_reference: "operations-synthetic-credit".into(),
            },
        })
        .await
        .unwrap();
    assert!(matches!(
        receipt,
        lmm_core::billing::ledger::Outcome::Posted { .. }
    ));
    let funds_before:String=sqlx::query_scalar("SELECT md5(jsonb_agg(to_jsonb(j) ORDER BY to_jsonb(j)::text)::text) FROM core_billing.ledger_journals j").fetch_one(&pool).await.unwrap();
    let first = source(&pool, "a").await;
    logs(&pool, first, 1102, owner, 1201, 10).await;
    let newer = source(&pool, "b").await;
    logs(&pool, newer, 1102, owner, 1, 50).await;
    let role = RuntimeRole::create(&pool).await;
    let store = OperationsStore::new(role.pool.clone());
    assert!(matches!(
        store.start_cleanup(&user.secret, 20, "job-one").await,
        Err(OperationsError::Forbidden)
    ));
    let start = store
        .start_cleanup(&admin.secret, 20, "job-one")
        .await
        .unwrap();
    assert_eq!(start["state"]["total"], 1201);
    let app = router(store.clone());
    let wire = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/api/system-task/log-cleanup?target_timestamp=20")
                .header("authorization", format!("Bearer {}", admin.secret))
                .header("idempotency-key", "job-one")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(wire.status(), StatusCode::OK);
    assert!(
        wire.headers()[header::CACHE_CONTROL]
            .to_str()
            .unwrap()
            .contains("no-store")
    );
    let wire_value: Value =
        serde_json::from_slice(&to_bytes(wire.into_body(), 1 << 20).await.unwrap()).unwrap();
    assert_eq!(wire_value["data"], start);
    let missing_key = app
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/api/system-task/log-cleanup?target_timestamp=20")
                .header("authorization", format!("Bearer {}", admin.secret))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(missing_key.status(), StatusCode::BAD_REQUEST);
    assert_eq!(
        start,
        store
            .start_cleanup(&admin.secret, 20, "job-one")
            .await
            .unwrap()
    );
    assert!(matches!(
        store.start_cleanup(&admin.secret, 21, "job-one").await,
        Err(OperationsError::Conflict)
    ));
    assert!(matches!(
        store
            .start_cleanup(&admin.secret, 20, "different-job")
            .await,
        Err(OperationsError::Conflict)
    ));
    let later_import = source(&pool, "c").await;
    logs(&pool, later_import, 1102, owner, 1, 10).await;
    assert!(
        sqlx::query("DELETE FROM core_history.log_records")
            .execute(&role.pool)
            .await
            .is_err()
    );
    assert!(
        sqlx::query("SELECT * FROM core_operations.upgrade_receipts")
            .execute(&role.pool)
            .await
            .is_err()
    );
    // Simulate process failure before its batch commits: both deletion and progress roll back.
    let mut interrupted = role.pool.begin().await.unwrap();
    sqlx::query("SELECT core_operations.run_log_cleanup_batch()")
        .execute(&mut *interrupted)
        .await
        .unwrap();
    interrupted.rollback().await.unwrap();
    assert_eq!(
        store.tasks(&admin.secret, None, 1).await.unwrap()[0]["state"]["processed"],
        0
    );
    store.cleanup_batch().await.unwrap();
    let after_first = store
        .tasks(&admin.secret, None, 1)
        .await
        .unwrap()
        .pop()
        .unwrap();
    assert_eq!(after_first["state"]["processed"], 500);
    let restarted = OperationsStore::new(role.pool.clone());
    let (one, two) = tokio::join!(restarted.cleanup_batch(), store.cleanup_batch());
    one.unwrap();
    two.unwrap();
    while restarted.cleanup_batch().await.unwrap().is_some() {}
    let done = restarted
        .tasks(&admin.secret, None, 1)
        .await
        .unwrap()
        .pop()
        .unwrap();
    assert_eq!(done["status"], "succeeded");
    assert_eq!(done["result"]["deleted_count"], 1201);
    assert_eq!(
        store
            .start_cleanup(&admin.secret, 20, "job-one")
            .await
            .unwrap(),
        done
    );
    let remaining: Vec<(i64, i64)> = sqlx::query_as(
        "SELECT source_id,created_at FROM core_history.log_records ORDER BY source_id",
    )
    .fetch_all(&pool)
    .await
    .unwrap();
    assert_eq!(remaining, vec![(newer, 50), (later_import, 10)]);
    let intact: bool =
        sqlx::query_scalar("SELECT count(*)=3 AND bool_and(complete) FROM core_history.sources")
            .fetch_one(&pool)
            .await
            .unwrap();
    assert!(intact);
    let funds_after:String=sqlx::query_scalar("SELECT md5(jsonb_agg(to_jsonb(j) ORDER BY to_jsonb(j)::text)::text) FROM core_billing.ledger_journals j").fetch_one(&pool).await.unwrap();
    assert_eq!(funds_before, funds_after);
    lmm_core::runtime_database::check(&role.pool, Default::default())
        .await
        .unwrap();
    role.cleanup().await;
}

#[sqlx::test(migrations = false)]
async fn instance_heartbeats_share_database_and_expire_without_clock_trust(pool: PgPool) {
    core_schema::init(&pool).await.unwrap();
    let identity = IdentityStore::from_pool(pool.clone());
    let admin = identity.bootstrap_user(1201, 5).await.unwrap();
    let user = identity.bootstrap_user(1202, 0).await.unwrap();
    let role = RuntimeRole::create(&pool).await;
    let one = Heartbeat::new(role.pool.clone(), Some("same-label".into())).unwrap();
    let two = Heartbeat::new(role.pool.clone(), Some("same-label".into())).unwrap();
    one.pulse("ready", 2, true, false).await.unwrap();
    two.pulse("ready", 0, true, false).await.unwrap();
    let store = OperationsStore::new(role.pool.clone());
    let list = store.instances(&admin.secret).await.unwrap();
    assert_eq!(list["items"].as_array().unwrap().len(), 2);
    assert_ne!(list["items"][0]["boot_id"], list["items"][1]["boot_id"]);
    assert!(
        list["items"]
            .as_array()
            .unwrap()
            .iter()
            .all(|r| r["online"] == true)
    );
    let one_boot = list["items"]
        .as_array()
        .unwrap()
        .iter()
        .find(|item| item["in_flight"] == 2)
        .unwrap()["boot_id"]
        .as_str()
        .unwrap();
    sqlx::query("UPDATE core_operations.instances SET heartbeat_at=clock_timestamp()-interval '46 seconds' WHERE boot_id=$1")
        .bind(one_boot).execute(&pool).await.unwrap();
    let stale = store.instances(&admin.secret).await.unwrap();
    assert_eq!(stale["items"][1]["online"], false);
    assert_eq!(stale["items"][0]["online"], true);
    two.pulse("stopped", 0, false, false).await.unwrap();
    assert!(
        store.instances(&admin.secret).await.unwrap()["items"]
            .as_array()
            .unwrap()
            .iter()
            .all(|r| r["online"] == false)
    );
    assert!(matches!(
        store.instances(&user.secret).await,
        Err(OperationsError::Forbidden)
    ));
    let app = router(store);
    let response = app
        .oneshot(
            Request::builder()
                .uri("/core/v1/admin/instances")
                .header("authorization", format!("Bearer {}", admin.secret))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    assert!(
        response.headers()[header::CACHE_CONTROL]
            .to_str()
            .unwrap()
            .contains("no-store")
    );
    let value: Value =
        serde_json::from_slice(&to_bytes(response.into_body(), 1 << 20).await.unwrap()).unwrap();
    assert_eq!(value["stale_after_seconds"], 45);
    assert!(!value.to_string().contains(&admin.secret));
    role.cleanup().await;
}

//! Transactional publication tests. `database` and `fixture` are the existing
//! routing test helpers; each SQLx test gets its own disposable database.

use super::{PostgresRoutingStore, ReleaseVersion, StoreError, database, fixture};
use sqlx::PgPool;
use std::sync::Arc;
use tokio::sync::Barrier;

fn release(routing: u64, pricing: u64) -> Option<ReleaseVersion> {
    Some(ReleaseVersion { routing, pricing })
}

#[sqlx::test]
async fn postgres_outer_failure_rolls_back_publication_and_caller_writes(pool: PgPool) {
    let store = database(&pool).await;
    let (first, first_prices) = fixture(1);
    store.stage_prices(first_prices).await.unwrap();
    let baseline = store.publish(first, 1, None).await.unwrap();
    sqlx::query("CREATE TABLE publish_test_audit (operation text NOT NULL)")
        .execute(&pool)
        .await
        .unwrap();

    let (second, second_prices) = fixture(2);
    let mut transaction = pool.begin().await.unwrap();
    sqlx::query("SELECT routing_stage_prices($1)")
        .bind(serde_json::to_value(&second_prices).unwrap())
        .execute(&mut *transaction)
        .await
        .unwrap();
    let pending = PostgresRoutingStore::publish_in_transaction(
        &mut transaction,
        second.clone(),
        2,
        release(1, 1),
    )
    .await
    .unwrap();
    assert_eq!(pending.routing.version, 2);
    let inside: i64 =
        sqlx::query_scalar("SELECT config_version FROM routing_active_release WHERE singleton")
            .fetch_one(&mut *transaction)
            .await
            .unwrap();
    assert_eq!(inside, 2);
    assert_eq!(store.load_active().await.unwrap(), baseline);
    sqlx::query("INSERT INTO publish_test_audit VALUES ('synthetic operation')")
        .execute(&mut *transaction)
        .await
        .unwrap();
    let failed = sqlx::query("SELECT 1 / 0")
        .execute(&mut *transaction)
        .await
        .unwrap_err();
    assert_eq!(
        failed
            .as_database_error()
            .and_then(|error| error.code())
            .as_deref(),
        Some("22012")
    );
    transaction.rollback().await.unwrap();

    assert_eq!(store.load_active().await.unwrap(), baseline);
    for table in [
        "routing_price_versions",
        "routing_price_rules",
        "routing_config_versions",
        "routing_groups",
        "routing_upstreams",
        "routing_routes",
        "routing_targets",
        "routing_aliases",
    ] {
        let count: i64 =
            sqlx::query_scalar(&format!("SELECT count(*) FROM {table} WHERE version=2"))
                .fetch_one(&pool)
                .await
                .unwrap();
        assert_eq!(count, 0, "rollback left rows in {table}");
    }
    let audit_count: i64 = sqlx::query_scalar("SELECT count(*) FROM publish_test_audit")
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(audit_count, 0);

    // The same CAS baseline remains usable after rollback. Caller writes commit
    // with the release, but this fixture table is not a product audit feature.
    store.stage_prices(second_prices).await.unwrap();
    let mut transaction = pool.begin().await.unwrap();
    let published =
        PostgresRoutingStore::publish_in_transaction(&mut transaction, second, 2, release(1, 1))
            .await
            .unwrap();
    sqlx::query("INSERT INTO publish_test_audit VALUES ('synthetic operation')")
        .execute(&mut *transaction)
        .await
        .unwrap();
    transaction.commit().await.unwrap();
    assert_eq!(store.load_active().await.unwrap(), published);
    let audit_count: i64 = sqlx::query_scalar("SELECT count(*) FROM publish_test_audit")
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(audit_count, 1);
}

#[sqlx::test]
async fn postgres_concurrent_outer_transactions_and_legacy_publish(pool: PgPool) {
    let store = database(&pool).await;
    let (first, prices) = fixture(1);
    store.stage_prices(prices).await.unwrap();
    let baseline = store.publish(first, 1, None).await.unwrap();
    assert_eq!(store.load_active().await.unwrap(), baseline);

    let barrier = Arc::new(Barrier::new(2));
    let contender = |version: u64, barrier: Arc<Barrier>| {
        let pool = pool.clone();
        async move {
            let (config, _) = fixture(version);
            let mut transaction = pool.begin().await.unwrap();
            barrier.wait().await;
            let result = PostgresRoutingStore::publish_in_transaction(
                &mut transaction,
                config,
                1,
                release(1, 1),
            )
            .await;
            match result {
                Ok(document) => {
                    transaction.commit().await.unwrap();
                    Ok(document)
                }
                Err(error) => {
                    transaction.rollback().await.unwrap();
                    Err(error)
                }
            }
        }
    };
    let (a, b) = tokio::join!(contender(2, barrier.clone()), contender(3, barrier));
    assert_eq!(usize::from(a.is_ok()) + usize::from(b.is_ok()), 1);
    assert!(
        matches!(&a, Err(StoreError::VersionConflict))
            || matches!(&b, Err(StoreError::VersionConflict))
    );
    let winner = a.or(b).unwrap();
    assert_eq!(store.load_active().await.unwrap(), winner);
    let versions: Vec<i64> = sqlx::query_scalar(
        "SELECT version FROM routing_config_versions WHERE version IN (2, 3) ORDER BY version",
    )
    .fetch_all(&pool)
    .await
    .unwrap();
    assert_eq!(versions, vec![winner.routing.version as i64]);

    let (later, _) = fixture(4);
    assert_eq!(
        store
            .publish(later.clone(), 1, release(1, 1))
            .await
            .unwrap_err(),
        StoreError::VersionConflict
    );
    assert_eq!(store.load_active().await.unwrap(), winner);
    let published = store
        .publish(later, 1, release(winner.routing.version, 1))
        .await
        .unwrap();
    assert_eq!(published.routing.version, 4);
    assert_eq!(store.load_active().await.unwrap(), published);

    let (mut invalid, _) = fixture(5);
    invalid.routes[0].targets[0].upstream_id = 999;
    let mut transaction = pool.begin().await.unwrap();
    let error =
        PostgresRoutingStore::publish_in_transaction(&mut transaction, invalid, 1, release(4, 1))
            .await
            .unwrap_err();
    assert!(matches!(error, StoreError::Routing(_)));
    transaction.rollback().await.unwrap();
    assert_eq!(store.load_active().await.unwrap(), published);
    let (next, _) = fixture(5);
    assert_eq!(
        store.publish(next, 0, release(4, 1)).await.unwrap_err(),
        StoreError::InvalidDocument
    );
}

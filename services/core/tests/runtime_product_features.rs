//! Isolated PostgreSQL role checks using the production grants, not owner access.
use lmm_core::runtime_database::RuntimeDatabaseError;
use sqlx::PgPool;

#[path = "../src/runtime_database/product_features.rs"]
mod product_features;
#[path = "common/runtime.rs"]
mod runtime;

#[sqlx::test]
async fn product_features_require_exact_grants_and_reject_excess_even_when_disabled(pool: PgPool) {
    lmm_core::core_schema::init(&pool).await.unwrap();
    let role = runtime::RuntimeRole::create(&pool).await;
    let mut connection = role.pool.acquire().await.unwrap();
    product_features::check(&mut connection, false)
        .await
        .unwrap();
    assert_eq!(
        product_features::check(&mut connection, true).await,
        Err(RuntimeDatabaseError::MissingPrivileges)
    );
    role.optional("core_console").await;
    // Component 74 grants these exact rights even when Creem channels are off.
    // Exercise the production upgrade fragment, not a hand-written fixture ACL.
    let creem_grants = include_str!("../schema/subscription_creem_grants.sql")
        .replace(":\"runtime_role\"", &role.name);
    sqlx::raw_sql(sqlx::AssertSqlSafe(creem_grants))
        .execute(&pool)
        .await
        .unwrap();
    for required in [false, true] {
        product_features::check(&mut connection, required)
            .await
            .unwrap();
    }
    // Charging also supports model writers. Their legitimate larger authority
    // must not be rejected by the three product schemas' stricter checks.
    role.optional("core_charging").await;
    product_features::check(&mut connection, true)
        .await
        .unwrap();

    let alterations = [
        (
            "GRANT UPDATE(order_id) ON core_subscriptions.creem_state TO",
            "REVOKE UPDATE(order_id) ON core_subscriptions.creem_state FROM",
        ),
        (
            "GRANT UPDATE(result) ON core_subscriptions.creem_events TO",
            "REVOKE UPDATE(result) ON core_subscriptions.creem_events FROM",
        ),
        (
            "GRANT DELETE,TRUNCATE,MAINTAIN ON core_subscriptions.creem_cycles TO",
            "REVOKE DELETE,TRUNCATE,MAINTAIN ON core_subscriptions.creem_cycles FROM",
        ),
        (
            "GRANT EXECUTE ON FUNCTION core_subscriptions.check_creem_cycle() TO",
            "REVOKE EXECUTE ON FUNCTION core_subscriptions.check_creem_cycle() FROM",
        ),
        (
            "GRANT UPDATE(terms) ON core_subscriptions.plan_revisions TO",
            "REVOKE UPDATE(terms) ON core_subscriptions.plan_revisions FROM",
        ),
        (
            "GRANT UPDATE(operation_key) ON core_subscriptions.quotes TO",
            "REVOKE UPDATE(operation_key) ON core_subscriptions.quotes FROM",
        ),
        (
            "GRANT INSERT ON core_promotions.secrets TO",
            "REVOKE INSERT ON core_promotions.secrets FROM",
        ),
        (
            "GRANT INSERT ON core_notifications.price_events TO",
            "REVOKE INSERT ON core_notifications.price_events FROM",
        ),
        (
            "GRANT DELETE,TRUNCATE,MAINTAIN ON core_promotions.claims TO",
            "REVOKE DELETE,TRUNCATE,MAINTAIN ON core_promotions.claims FROM",
        ),
        (
            "GRANT REFERENCES(user_id) ON core_notifications.inbox TO",
            "REVOKE REFERENCES(user_id) ON core_notifications.inbox FROM",
        ),
        (
            "GRANT USAGE ON SEQUENCE core_notifications.price_events_id_seq TO",
            "REVOKE USAGE ON SEQUENCE core_notifications.price_events_id_seq FROM",
        ),
        (
            "GRANT EXECUTE ON FUNCTION core_promotions.guard_campaign() TO",
            "REVOKE EXECUTE ON FUNCTION core_promotions.guard_campaign() FROM",
        ),
    ];
    for (grant, revoke) in alterations {
        sqlx::raw_sql(sqlx::AssertSqlSafe(format!("{grant} {}", role.name)))
            .execute(&pool)
            .await
            .unwrap();
        for required in [false, true] {
            assert_eq!(
                product_features::check(&mut connection, required).await,
                Err(RuntimeDatabaseError::UnsafePrivileges),
                "{grant} should remain unsafe with required={required}",
            );
        }
        sqlx::raw_sql(sqlx::AssertSqlSafe(format!("{revoke} {}", role.name)))
            .execute(&pool)
            .await
            .unwrap();
    }
    sqlx::raw_sql(sqlx::AssertSqlSafe(format!(
        "GRANT SELECT ON core_notifications.inbox TO {} WITH GRANT OPTION",
        role.name
    )))
    .execute(&pool)
    .await
    .unwrap();
    assert_eq!(
        product_features::check(&mut connection, false).await,
        Err(RuntimeDatabaseError::UnsafePrivileges)
    );
    sqlx::raw_sql(sqlx::AssertSqlSafe(format!(
        "REVOKE GRANT OPTION FOR SELECT ON core_notifications.inbox FROM {}",
        role.name
    )))
    .execute(&pool)
    .await
    .unwrap();
    sqlx::raw_sql(sqlx::AssertSqlSafe(format!(
        "REVOKE UPDATE(enabled) ON core_notifications.preferences FROM {}",
        role.name
    )))
    .execute(&pool)
    .await
    .unwrap();
    assert_eq!(
        product_features::check(&mut connection, true).await,
        Err(RuntimeDatabaseError::MissingPrivileges)
    );
    product_features::check(&mut connection, false)
        .await
        .unwrap();
    drop(connection);
    role.cleanup().await;
}

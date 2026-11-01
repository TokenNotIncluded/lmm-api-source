//! The offline definer fences every publication even without an active slot row.
use lmm_core::core_schema;
use sqlx::PgPool;
#[path = "common/runtime.rs"]
mod runtime;

#[sqlx::test(migrations = false)]
async fn balance_lock_fences_all_publication_entries_with_and_without_singleton(pool: PgPool) {
    core_schema::init(&pool).await.unwrap();
    let role = runtime::RuntimeRole::create(&pool).await;
    // The direct config publisher requires an existing sealed pricing version.
    sqlx::raw_sql("INSERT INTO public.routing_price_versions(version,document,sealed) VALUES(1,'{\"version\":1,\"rules\":[]}',true)")
        .execute(&pool).await.unwrap();
    // A publisher may already hold the singleton row FOR UPDATE. The helper
    // must take only table locks, so this ordering does not wait on that row.
    let mut early_publisher = pool.begin().await.unwrap();
    sqlx::query("SELECT singleton FROM public.routing_active_release FOR UPDATE")
        .execute(&mut *early_publisher)
        .await
        .unwrap();
    let mut observation = role.pool.begin().await.unwrap();
    sqlx::query("SET LOCAL lock_timeout='500ms'")
        .execute(&mut *observation)
        .await
        .unwrap();
    sqlx::query("SELECT public.routing_control_balance_lock()")
        .execute(&mut *observation)
        .await
        .unwrap();
    observation.rollback().await.unwrap();
    early_publisher.rollback().await.unwrap();
    // An in-flight version write instead makes the caller's 500ms bound apply.
    let mut publisher = pool.begin().await.unwrap();
    sqlx::raw_sql(
        "INSERT INTO public.routing_price_versions(version,document) VALUES(2,'{\"version\":2}')",
    )
    .execute(&mut *publisher)
    .await
    .unwrap();
    let mut observation = role.pool.begin().await.unwrap();
    sqlx::query("SET LOCAL lock_timeout='500ms'")
        .execute(&mut *observation)
        .await
        .unwrap();
    let error = sqlx::query("SELECT public.routing_control_balance_lock()")
        .execute(&mut *observation)
        .await
        .unwrap_err();
    assert_eq!(
        error.as_database_error().unwrap().code().as_deref(),
        Some("55P03")
    );
    observation.rollback().await.unwrap();
    publisher.rollback().await.unwrap();
    for missing_slot in [false, true] {
        if missing_slot {
            // Owner-only synthetic fixture; restore the unchanged guard immediately.
            sqlx::raw_sql("ALTER TABLE public.routing_active_release DISABLE TRIGGER routing_active_guard; DELETE FROM public.routing_active_release; ALTER TABLE public.routing_active_release ENABLE TRIGGER routing_active_guard")
                .execute(&pool).await.unwrap();
        }
        let mut observation = role.pool.begin().await.unwrap();
        sqlx::query("SET LOCAL lock_timeout='500ms'")
            .execute(&mut *observation)
            .await
            .unwrap();
        sqlx::query("SELECT public.routing_control_balance_lock()")
            .execute(&mut *observation)
            .await
            .unwrap();
        for statement in [
            "SELECT public.routing_control_publish('{\"version\":1,\"upstreams\":[],\"groups\":[],\"routes\":[],\"aliases\":[]}', '{\"version\":1,\"rules\":[]}', NULL, NULL)",
            "SELECT public.routing_control_publish('{\"version\":1,\"upstreams\":[],\"groups\":[],\"routes\":[],\"aliases\":[]}', '{\"version\":1,\"rules\":[]}', '{}', NULL, NULL)",
            "SELECT public.routing_publish_config('{\"version\":1,\"upstreams\":[],\"groups\":[],\"routes\":[],\"aliases\":[]}',1,NULL,NULL)",
            "SELECT public.routing_stage_prices('{\"version\":1,\"rules\":[]}')",
            "UPDATE public.routing_active_release SET config_version=config_version",
            "INSERT INTO public.routing_active_release(singleton) VALUES(true) ON CONFLICT(singleton) DO NOTHING",
        ] {
            let mut publication = pool.begin().await.unwrap();
            sqlx::query("SET LOCAL lock_timeout='150ms'")
                .execute(&mut *publication)
                .await
                .unwrap();
            let error = sqlx::raw_sql(statement)
                .execute(&mut *publication)
                .await
                .unwrap_err();
            assert_eq!(
                error.as_database_error().unwrap().code().as_deref(),
                Some("55P03"),
                "missing_slot={missing_slot}: {statement}: {error}"
            );
            publication.rollback().await.unwrap();
        }
        observation.rollback().await.unwrap();
        // Commit/rollback of the observing transaction releases its fence.
        let mut publication = pool.begin().await.unwrap();
        sqlx::query("SET LOCAL lock_timeout='150ms'")
            .execute(&mut *publication)
            .await
            .unwrap();
        sqlx::raw_sql("SELECT public.routing_control_publish('{\"version\":1,\"upstreams\":[],\"groups\":[],\"routes\":[],\"aliases\":[]}', '{\"version\":1,\"rules\":[]}', NULL, NULL)")
            .execute(&mut *publication).await.unwrap();
        publication.rollback().await.unwrap();
    }
    // EXECUTE grants convey no direct row lock/write authority over publications.
    let error = sqlx::query("SELECT singleton FROM public.routing_active_release FOR SHARE")
        .execute(&role.pool)
        .await
        .unwrap_err();
    assert_eq!(
        error.as_database_error().unwrap().code().as_deref(),
        Some("42501")
    );
    core_schema::check(&pool).await.unwrap();
    role.cleanup().await;
}

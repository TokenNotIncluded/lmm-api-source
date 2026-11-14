//! Real PostgreSQL coverage of the additive attribution schema and runtime ACL.
use super::*;
use sqlx::PgPool;

#[sqlx::test(migrations = false)]
async fn affiliate71_suffix_preserves_prior_rows_and_limits_runtime(pool: PgPool) {
    use crate::core_schema as root;
    authorize_target().unwrap();
    let mut tx = pool.begin().await.unwrap();
    for (_, ddl) in &COMPONENTS[..70] {
        sqlx::raw_sql(*ddl).execute(&mut *tx).await.unwrap();
    }
    for ddl in [
        root::CONTRACT,
        root::UPGRADE_METADATA,
        root::ADDITIVE_UPGRADE_METADATA,
        root::LEDGER_REVISION_METADATA,
        root::CONSOLE_REVISION_METADATA,
        root::OFFLINE_REVISION_METADATA,
    ] {
        sqlx::raw_sql(ddl).execute(&mut *tx).await.unwrap();
    }
    assert_eq!(
        hex(fingerprint(&definitions(&mut tx).await.unwrap())),
        ENGINE.predecessor_catalog
    );
    IdentityStore::record_schema_contract(&mut tx)
        .await
        .unwrap();
    crate::identity::grants::record_schema_contract(&mut tx)
        .await
        .unwrap();
    tx.commit().await.unwrap();
    let identity = IdentityStore::from_pool(pool.clone());
    identity.bootstrap_user(71001, 0).await.unwrap();
    let before: serde_json::Value =
        sqlx::query_scalar("SELECT to_jsonb(u) FROM core_identity.users u WHERE id=71001")
            .fetch_one(&pool)
            .await
            .unwrap();
    let role = crate::test_runtime::RuntimeRole::create(&pool).await;
    let mut tx = pool.begin().await.unwrap();
    sqlx::raw_sql(COMPONENTS[70].1)
        .execute(&mut *tx)
        .await
        .unwrap();
    assert_eq!(
        hex(fingerprint(&definitions(&mut tx).await.unwrap())),
        serde_json::from_str::<serde_json::Value>(TARGET).unwrap()["catalog_sha256"]
            .as_str()
            .unwrap()
    );
    grant_runtime(&mut tx, &role.name).await.unwrap();
    tx.commit().await.unwrap();
    let after: serde_json::Value =
        sqlx::query_scalar("SELECT to_jsonb(u) FROM core_identity.users u WHERE id=71001")
            .fetch_one(&pool)
            .await
            .unwrap();
    assert_eq!(before, after);
    let rights: (bool, bool, bool) = sqlx::query_as("SELECT has_table_privilege('core_identity.affiliate_codes','SELECT,INSERT'), has_table_privilege('core_identity.affiliate_codes','UPDATE,DELETE,TRUNCATE'), has_table_privilege('core_meta.affiliate_registration_upgrades','SELECT,INSERT,UPDATE,DELETE,TRUNCATE')")
        .fetch_one(&role.pool).await.unwrap();
    assert_eq!(rights, (true, false, false));
    role.cleanup().await;
}

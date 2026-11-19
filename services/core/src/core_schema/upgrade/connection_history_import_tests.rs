//! Real isolated PostgreSQL upgrade, exact replay, and preservation.
use super::super::payment_discounts::tests::{EngineBaseline, engine_offline_boundary};
use super::*;
use crate::core_schema::{UpgradeDatabase, UpgradeRoles, upgrade_database};
use sqlx::{PgPool, postgres::PgPoolOptions};
async fn prefix76(pool: &PgPool) {
    use crate::core_schema as root;
    let mut tx = pool.begin().await.unwrap();
    for (_, ddl) in &COMPONENTS[..76] {
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
    IdentityStore::record_schema_contract(&mut tx)
        .await
        .unwrap();
    crate::identity::grants::record_schema_contract(&mut tx)
        .await
        .unwrap();
    for (name, ddl) in &COMPONENTS[..76] {
        sqlx::query("INSERT INTO core_meta.component_contract(name,fingerprint) VALUES($1,$2)")
            .bind(name)
            .bind(Sha256::digest(ddl.as_bytes()).to_vec())
            .execute(&mut *tx)
            .await
            .unwrap();
    }
    let catalog = fingerprint(&definitions(&mut tx).await.unwrap());
    assert_eq!(hex(&catalog), ENGINE.predecessor_catalog);
    sqlx::query("INSERT INTO core_meta.runtime_contract(catalog_fingerprint) VALUES($1)")
        .bind(catalog)
        .execute(&mut *tx)
        .await
        .unwrap();
    tx.commit().await.unwrap();
}

#[sqlx::test(migrations = false)]
async fn connection_history77_exact_upgrade_preserves_data_acl_and_replays(pool: PgPool) {
    authorize_target().unwrap();
    let facts=sqlx::query("SELECT current_database()::text AS name,d.oid::bigint AS oid,current_setting('server_version_num')::bigint AS version,(SELECT system_identifier::text FROM pg_control_system()) AS system,session_user::text AS original FROM pg_database d WHERE datname=current_database()")
        .fetch_one(&pool).await.unwrap();
    let suffix = format!("{:016x}", rand::random::<u64>());
    let request = UpgradeRequest {
        version: 1,
        procedure: PROCEDURE.into(),
        upgrade_id: format!("connection_history77-owner-{suffix}"),
        database: UpgradeDatabase {
            name: facts.get("name"),
            oid: u32::try_from(facts.get::<i64, _>("oid")).unwrap(),
            system_identifier: facts.get("system"),
            server_version_num: u32::try_from(facts.get::<i64, _>("version")).unwrap(),
        },
        roles: UpgradeRoles {
            installer: format!("connection_history77_i_{suffix}"),
            owner: format!("connection_history77_o_{suffix}"),
            runtime: format!("connection_history77_r_{suffix}"),
            extensions: format!("connection_history77_e_{suffix}"),
        },
        predecessor_manifest_sha256: ENGINE.predecessor_manifest.into(),
        predecessor_catalog_sha256: ENGINE.predecessor_catalog.into(),
        successor_manifest_sha256: ENGINE.target_manifest().unwrap(),
    };
    let original: String = facts.get("original");
    let options = (*pool.connect_options()).clone();
    let password = format!("synthetic-{:032x}", rand::random::<u128>());
    sqlx::raw_sql(sqlx::AssertSqlSafe(format!("CREATE ROLE {owner} NOLOGIN; CREATE ROLE {installer} LOGIN NOINHERIT PASSWORD '{password}'; CREATE ROLE {runtime} NOLOGIN; CREATE ROLE {extensions} NOLOGIN; GRANT {owner} TO {installer} WITH INHERIT FALSE, SET TRUE; ALTER DATABASE {database} OWNER TO {owner}; REVOKE CONNECT,CREATE,TEMP ON DATABASE {database} FROM PUBLIC; GRANT CONNECT ON DATABASE {database} TO {installer}; REVOKE EXECUTE ON FUNCTION pg_catalog.pg_control_system() FROM PUBLIC; GRANT EXECUTE ON FUNCTION pg_catalog.pg_control_system() TO {owner};",
        owner=quote(&request.roles.owner),installer=quote(&request.roles.installer),runtime=quote(&request.roles.runtime),extensions=quote(&request.roles.extensions),database=quote(&request.database.name))))
        .execute(&pool).await.unwrap();
    let installer = PgPoolOptions::new()
        .max_connections(1)
        .connect_with(
            options
                .clone()
                .username(&request.roles.installer)
                .password(&password)
                .options([("role", request.roles.owner.as_str())]),
        )
        .await
        .unwrap();
    prefix76(&installer).await;

    IdentityStore::from_pool(installer.clone())
        .bootstrap_user(77001, 0)
        .await
        .unwrap();
    sqlx::query("INSERT INTO core_console.options(key,value,revision) VALUES ('history_import_marker','preserve-unchanged',12)")
        .execute(&installer).await.unwrap();
    sqlx::raw_sql(sqlx::AssertSqlSafe(format!(
        "GRANT USAGE ON SCHEMA core_history TO {runtime}; GRANT SELECT(document,mapping,native_credential_floor,snapshot_observed_at,target_database,target_database_oid,target_nonce) ON core_history.connection_snapshot TO {runtime}",
        runtime=quote(&request.roles.runtime)
    ))).execute(&installer).await.unwrap();
    let mut c = installer.acquire().await.unwrap();
    let baseline = EngineBaseline::capture(&mut c).await;
    drop(c);
    let mut url = reqwest::Url::parse(&std::env::var("DATABASE_URL").unwrap()).unwrap();
    url.set_path(&format!("/{}", request.database.name));
    url.set_username(&request.roles.installer).unwrap();
    url.set_password(Some(&password)).unwrap();
    installer.close().await;
    pool.close().await;
    engine_offline_boundary(&request).await;
    let mut wrong = request.clone();
    wrong.predecessor_catalog_sha256 = "00".repeat(32);
    assert!(upgrade_database(url.as_str(), wrong).await.is_err());
    engine_offline_boundary(&request).await;
    let report = upgrade_database(url.as_str(), request.clone())
        .await
        .unwrap();
    assert_eq!(report.status, "committed");
    assert_eq!(report.components, 77);
    engine_offline_boundary(&request).await;
    let report = upgrade_database(url.as_str(), request.clone())
        .await
        .unwrap();
    assert_eq!(report.status, "already_committed");
    let mut changed = request.clone();
    changed.upgrade_id.push_str("-different");
    engine_offline_boundary(&request).await;
    assert!(matches!(
        upgrade_database(url.as_str(), changed).await,
        Err(UpgradeError::Rejected(
            "CONNECTION_HISTORY_IMPORT_ORIGINAL_RECEIPT_ABSENT"
        ))
    ));
    let mut inspect = PgConnection::connect_with(&options).await.unwrap();
    baseline.assert_preserved(&mut inspect).await;
    let runtime_complete: bool = sqlx::query_scalar("SELECT has_column_privilege($1,'core_history.connection_snapshot','document','SELECT') AND NOT has_table_privilege($1,'core_history.connection_snapshot','INSERT,UPDATE,DELETE,TRUNCATE') AND NOT has_column_privilege($1,'core_history.connection_snapshot','document_utf8','SELECT') AND NOT has_table_privilege($1,'core_meta.connection_history_import_upgrades','SELECT,INSERT,UPDATE,DELETE')").bind(&request.roles.runtime).fetch_one(&mut inspect).await.unwrap();
    assert!(runtime_complete);
    let old_policy: (String, i64) = sqlx::query_as(
        "SELECT value,revision FROM core_console.options WHERE key='history_import_marker'",
    )
    .fetch_one(&mut inspect)
    .await
    .unwrap();
    assert_eq!(old_policy, ("preserve-unchanged".into(), 12));
    let receipts: i64 =
        sqlx::query_scalar("SELECT count(*) FROM core_meta.connection_history_import_upgrades")
            .fetch_one(&mut inspect)
            .await
            .unwrap();
    assert_eq!(receipts, 1);
    assert!(
        sqlx::query("UPDATE core_meta.connection_history_import_upgrades SET upgrade_id='changed'")
            .execute(&mut inspect)
            .await
            .is_err()
    );
    let guard: String = sqlx::query_scalar(
        "SELECT pg_get_functiondef('core_history.guard_connection_snapshot()'::regprocedure)",
    )
    .fetch_one(&mut inspect)
    .await
    .unwrap();
    assert!(guard.contains("core_meta.component_contract)<>77"));
    sqlx::raw_sql(sqlx::AssertSqlSafe(format!("REASSIGN OWNED BY {} TO {}; DROP OWNED BY {},{},{},{} CASCADE; ALTER DATABASE {} OWNER TO {}; DROP ROLE {},{},{},{};", quote(&request.roles.owner),quote(&original),quote(&request.roles.installer),quote(&request.roles.owner),quote(&request.roles.runtime),quote(&request.roles.extensions),quote(&request.database.name),quote(&original),quote(&request.roles.installer),quote(&request.roles.owner),quote(&request.roles.runtime),quote(&request.roles.extensions)))).execute(&mut inspect).await.unwrap();
    inspect.close().await.unwrap();
}

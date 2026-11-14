//! Isolated PG18.6 capture, exact offline upgrade/replay, data and ACL evidence.
use super::super::payment_discounts::tests::{EngineBaseline, engine_offline_boundary};
use super::*;
use sqlx::PgPool;
async fn prefix68(pool: &PgPool) {
    use crate::core_schema as root;
    let mut tx = pool.begin().await.unwrap();
    for (_, ddl) in &COMPONENTS[..68] {
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
    for (name, ddl) in &COMPONENTS[..68] {
        sqlx::query("INSERT INTO core_meta.component_contract(name,fingerprint) VALUES($1,$2)")
            .bind(name)
            .bind(Sha256::digest(ddl.as_bytes()).to_vec())
            .execute(&mut *tx)
            .await
            .unwrap();
    }
    let fingerprint = crate::core_schema::catalog(&mut tx).await.unwrap();
    sqlx::query("INSERT INTO core_meta.runtime_contract(catalog_fingerprint) VALUES($1)")
        .bind(fingerprint)
        .execute(&mut *tx)
        .await
        .unwrap();
    tx.commit().await.unwrap();
    IdentityStore::from_pool(pool.clone())
        .bootstrap_user(68001, 0)
        .await
        .unwrap();
}
fn manifest(count: usize) -> String {
    let mut m = schema_manifest();
    m.components.truncate(count);
    let mut bytes = serde_json::to_vec(&m).unwrap();
    bytes.push(b'\n');
    hex(Sha256::digest(bytes))
}

async fn measure(pool: &PgPool) -> serde_json::Value {
    prefix68(pool).await;
    assert_eq!(COMPONENTS[68].0, "responses-continuity");
    let mut c = pool.acquire().await.unwrap();
    let version: i32 = sqlx::query_scalar("SELECT current_setting('server_version_num')::int")
        .fetch_one(&mut *c)
        .await
        .unwrap();
    assert_eq!(version, 180006);
    let before = hex(fingerprint(&definitions(&mut c).await.unwrap()));
    let baseline = EngineBaseline::capture(&mut c).await;
    let mut tx = c.begin().await.unwrap();
    sqlx::raw_sql(sqlx::AssertSqlSafe(COMPONENTS[68].1))
        .execute(&mut *tx)
        .await
        .unwrap();
    let after = hex(fingerprint(&definitions(&mut tx).await.unwrap()));
    tx.commit().await.unwrap();
    baseline.assert_preserved(&mut c).await;
    serde_json::json!({"predecessor":{"manifest_sha256":manifest(68),"catalog_sha256":before},"successor":{"manifest_sha256":manifest(69),"catalog_sha256":after}})
}
#[sqlx::test(migrations = false)]
#[ignore = "explicit isolated owner and private capture output"]
async fn app07_capture_68_to69(pool: PgPool) {
    use std::{fs::OpenOptions, io::Write, os::unix::fs::OpenOptionsExt};
    let mut f = OpenOptions::new()
        .write(true)
        .create_new(true)
        .mode(0o600)
        .open(std::env::var("LMM_APP07_CATALOG_CAPTURE").unwrap())
        .unwrap();
    writeln!(
        f,
        "{}",
        serde_json::to_string_pretty(&measure(&pool).await).unwrap()
    )
    .unwrap();
}
#[sqlx::test(migrations = false)]
async fn app07_68_to69_preserves_data_and_acl(pool: PgPool) {
    let measured = measure(&pool).await;
    assert_eq!(
        measured["predecessor"]["manifest_sha256"],
        ENGINE.predecessor_manifest
    );
    assert_eq!(
        measured["predecessor"]["catalog_sha256"],
        ENGINE.predecessor_catalog
    );
    let pin: serde_json::Value = serde_json::from_str(TARGET).unwrap();
    assert_eq!(measured["successor"], pin);
    authorize_target().unwrap();
}
#[sqlx::test(migrations = false)]
async fn app07_upgrade_engine_68_to69_exact_replay_and_runtime_denial(pool: PgPool) {
    use crate::core_schema::{UpgradeDatabase, UpgradeRoles, upgrade_database};
    use sqlx::postgres::PgPoolOptions;
    // A missing/mismatched reviewed pin is a failure, not a skipped engine test.
    authorize_target()
        .expect("capture and fix reviewed 68/69 pins before running engine acceptance");
    let facts = sqlx::query("SELECT current_database()::text AS name,d.oid::bigint AS oid,current_setting('server_version_num')::bigint AS version,(SELECT system_identifier::text FROM pg_control_system()) AS system,session_user::text AS original FROM pg_database d WHERE datname=current_database()")
        .fetch_one(&pool).await.unwrap();
    let suffix = format!("{:016x}", rand::random::<u64>());
    let r = UpgradeRequest {
        version: 1,
        procedure: PROCEDURE.into(),
        upgrade_id: format!("app07-engine-{suffix}"),
        database: UpgradeDatabase {
            name: facts.get("name"),
            oid: u32::try_from(facts.get::<i64, _>("oid")).unwrap(),
            system_identifier: facts.get("system"),
            server_version_num: u32::try_from(facts.get::<i64, _>("version")).unwrap(),
        },
        roles: UpgradeRoles {
            installer: format!("app07_i_{suffix}"),
            owner: format!("app07_o_{suffix}"),
            runtime: format!("app07_r_{suffix}"),
            extensions: format!("app07_e_{suffix}"),
        },
        predecessor_manifest_sha256: ENGINE.predecessor_manifest.into(),
        predecessor_catalog_sha256: ENGINE.predecessor_catalog.into(),
        successor_manifest_sha256: ENGINE.target_manifest().unwrap(),
    };
    assert_eq!(r.database.server_version_num, 180006);
    assert_eq!(manifest(68), r.predecessor_manifest_sha256);
    let original: String = facts.get("original");
    let admin_options = (*pool.connect_options()).clone();
    let password = format!("synthetic-{:032x}", rand::random::<u128>());
    sqlx::raw_sql(sqlx::AssertSqlSafe(format!("CREATE ROLE {owner} NOLOGIN; CREATE ROLE {installer} LOGIN NOINHERIT PASSWORD '{password}'; CREATE ROLE {runtime} NOLOGIN; CREATE ROLE {extensions} NOLOGIN; GRANT {owner} TO {installer} WITH INHERIT FALSE, SET TRUE; ALTER DATABASE {database} OWNER TO {owner}; REVOKE CONNECT,CREATE,TEMP ON DATABASE {database} FROM PUBLIC; GRANT CONNECT ON DATABASE {database} TO {installer}; REVOKE EXECUTE ON FUNCTION pg_catalog.pg_control_system() FROM PUBLIC; GRANT EXECUTE ON FUNCTION pg_catalog.pg_control_system() TO {owner};",
        owner=quote(&r.roles.owner), installer=quote(&r.roles.installer), runtime=quote(&r.roles.runtime), extensions=quote(&r.roles.extensions), database=quote(&r.database.name))))
        .execute(&pool).await.unwrap();
    let installer = PgPoolOptions::new()
        .max_connections(1)
        .connect_with(
            admin_options
                .clone()
                .username(&r.roles.installer)
                .password(&password)
                .options([("role", r.roles.owner.as_str())]),
        )
        .await
        .unwrap();
    prefix68(&installer).await;
    let identity = IdentityStore::from_pool(installer.clone());
    let root = identity.bootstrap_user(68002, 6).await.unwrap();
    crate::identity::admin_users::AdminUsersStore::new(identity)
        .adjust_user_balance(
            &root.secret,
            68001,
            &crate::identity::admin_users::AdjustmentRequest {
                delta_units: "1234568".into(),
                operation_key: "app07-real-upgrade-ledger-fixture".into(),
                reason: "isolated upgrade preservation fixture".into(),
            },
        )
        .await
        .unwrap();
    // Preserve pre-existing coupon rows, bound intent/reservation and frozen wire.
    // Explicit fixture insertion represents already-bound component-68 state.
    sqlx::query("INSERT INTO core_payments.discount_codes(code,name,discount_percent,min_credit_units,max_uses,enabled,starts_at,ends_at,created_by) VALUES('PRIOR68','Predecessor fixture',20,0,1,TRUE,0,0,68002)")
        .execute(&installer).await.unwrap();
    let payments = crate::payments::PaymentsStore::new(
        installer.clone(),
        vec![crate::payments::Channel::stripe_test(
            1,
            "acct_fixture".into(),
            "whsec_fixture_only".into(),
            100_000,
        )],
    )
    .unwrap();
    let account = IdentityStore::from_pool(installer.clone())
        .authorize(&root.secret)
        .await
        .unwrap()
        .owner_account_id;
    let intent = payments
        .prepare(
            &root.secret,
            crate::internal_rpc::pb::PrepareTopupRequest {
                mutation: Some(crate::internal_rpc::pb::MutationContext {
                    idempotency_key: "app07-prior-intent-preservation".into(),
                    expected_version: None,
                }),
                account_id: account,
                payment_channel_id: 1,
                currency: "USD".into(),
                amount_minor: 100,
                requested_credit_units: 0,
            },
        )
        .await
        .unwrap();
    sqlx::query("INSERT INTO core_payments.discount_reservations(intent_id,code_id) SELECT $1,id FROM core_payments.discount_codes WHERE code='PRIOR68'").bind(intent.intent_id).execute(&installer).await.unwrap();
    drop(payments);
    // Exercise non-default pre-existing schema/table/column/sequence/function ACLs.
    sqlx::raw_sql(sqlx::AssertSqlSafe(format!("GRANT USAGE ON SCHEMA core_identity,core_payments,core_meta,core_model TO {runtime}; GRANT SELECT ON core_identity.users TO {runtime}; GRANT UPDATE(revoked_at) ON core_identity.credentials TO {runtime}; GRANT USAGE ON SEQUENCE core_identity.credentials_id_seq TO {runtime}; GRANT EXECUTE ON FUNCTION public.routing_stage_prices(jsonb) TO {runtime};",
        runtime=quote(&r.roles.runtime)))).execute(&installer).await.unwrap();
    let mut connection = installer.acquire().await.unwrap();
    assert_eq!(
        hex(fingerprint(&definitions(&mut connection).await.unwrap())),
        r.predecessor_catalog_sha256
    );
    let journals: i64 = sqlx::query_scalar("SELECT count(*) FROM core_billing.ledger_journals")
        .fetch_one(&mut *connection)
        .await
        .unwrap();
    assert!(
        journals > 0,
        "preservation must include real financial postings"
    );
    sqlx::query("SELECT setval('core_identity.credentials_id_seq',9000,false)")
        .execute(&mut *connection)
        .await
        .unwrap();
    let baseline = EngineBaseline::capture(&mut connection).await;
    drop(connection);
    let mut url = reqwest::Url::parse(&std::env::var("DATABASE_URL").unwrap()).unwrap();
    url.set_path(&format!("/{}", r.database.name));
    url.set_username(&r.roles.installer).unwrap();
    url.set_password(Some(&password)).unwrap();
    installer.close().await;
    pool.close().await;

    // Real incarnation checks execute against the live target before any DDL.
    let mut wrong_oid = r.clone();
    wrong_oid.database.oid = r.database.oid.checked_add(1).unwrap();
    engine_offline_boundary(&r).await;
    assert!(matches!(
        upgrade_database(url.as_str(), wrong_oid).await,
        Err(UpgradeError::Rejected(
            "UPGRADE_DB_INCARNATION_OR_ROLE_MISMATCH"
        ))
    ));
    let mut wrong_system = r.clone();
    wrong_system.database.system_identifier = if r.database.system_identifier == "1" {
        "2".into()
    } else {
        "1".into()
    };
    engine_offline_boundary(&r).await;
    assert!(matches!(
        upgrade_database(url.as_str(), wrong_system).await,
        Err(UpgradeError::Rejected(
            "UPGRADE_DB_SYSTEM_IDENTIFIER_MISMATCH"
        ))
    ));

    engine_offline_boundary(&r).await;
    let failed = FAIL_AFTER_DDL
        .scope(true, upgrade_database(url.as_str(), r.clone()))
        .await;
    assert!(matches!(
        failed,
        Err(UpgradeError::Rejected(
            "RESPONSES_CONTINUITY_INJECTED_AFTER_DDL"
        ))
    ));
    let mut inspect = PgConnection::connect_with(&admin_options).await.unwrap();
    baseline.assert_preserved(&mut inspect).await;
    assert_eq!(
        hex(fingerprint(&definitions(&mut inspect).await.unwrap())),
        r.predecessor_catalog_sha256
    );
    let absent: bool =
        sqlx::query_scalar("SELECT to_regclass('core_model.responses_continuations') IS NULL")
            .fetch_one(&mut inspect)
            .await
            .unwrap();
    assert!(absent);
    inspect.close().await.unwrap();
    engine_offline_boundary(&r).await;
    let proxy = crate::test_pg_commit_fault::PgCommitFault::start(
        url.as_str(),
        "INSERT INTO core_meta.responses_continuity_upgrades",
    )
    .await;
    let lost = upgrade_database(&proxy.url, r.clone()).await;
    assert!(
        matches!(lost, Err(UpgradeError::CommitUnknown { .. })),
        "real wire ACK loss must remain UNKNOWN"
    );
    proxy.assert_commit_ack_dropped().await;
    drop(proxy);
    engine_offline_boundary(&r).await;
    let committed = upgrade_database(url.as_str(), r.clone()).await.unwrap();
    assert_eq!(committed.status, "already_committed");
    assert_eq!(committed.components, 69);
    assert_eq!(committed.request_sha256, hex(r.digest().unwrap()));
    let pin: serde_json::Value = serde_json::from_str(TARGET).unwrap();
    assert_eq!(committed.successor_catalog_sha256, pin["catalog_sha256"]);
    engine_offline_boundary(&r).await;
    let replay = upgrade_database(url.as_str(), r.clone()).await.unwrap();
    assert_eq!(replay.status, "already_committed");
    assert_eq!(replay.request_sha256, committed.request_sha256);
    assert_eq!(
        replay.successor_catalog_sha256,
        committed.successor_catalog_sha256
    );
    let mut another = r.clone();
    another.upgrade_id.push('2');
    engine_offline_boundary(&r).await;
    assert!(matches!(
        upgrade_database(url.as_str(), another).await,
        Err(UpgradeError::Rejected(
            "RESPONSES_CONTINUITY_ORIGINAL_RECEIPT_ABSENT"
        ))
    ));
    let mut changed = r.clone();
    std::mem::swap(&mut changed.roles.runtime, &mut changed.roles.extensions);
    engine_offline_boundary(&r).await;
    assert!(matches!(
        upgrade_database(url.as_str(), changed).await,
        Err(UpgradeError::Rejected(
            "RESPONSES_CONTINUITY_RECEIPT_OR_CATALOG_CONFLICT"
        ))
    ));

    let mut inspect = PgConnection::connect_with(&admin_options).await.unwrap();
    baseline.assert_preserved(&mut inspect).await;
    let receipt: (i64, Vec<u8>, serde_json::Value, Vec<u8>, Vec<u8>) = sqlx::query_as("SELECT count(*) OVER(),request_sha256,request,preserved_data_sha256,preserved_permissions_sha256 FROM core_meta.responses_continuity_upgrades WHERE upgrade_id=$1")
        .bind(&r.upgrade_id).fetch_one(&mut inspect).await.unwrap();
    assert_eq!(receipt.0, 1);
    let receipt_count: i64 =
        sqlx::query_scalar("SELECT count(*) FROM core_meta.responses_continuity_upgrades")
            .fetch_one(&mut inspect)
            .await
            .unwrap();
    assert_eq!(
        receipt_count, 1,
        "replay and rejected requests cannot create another receipt"
    );
    assert_eq!(receipt.1, r.digest().unwrap());
    assert_eq!(receipt.2, serde_json::to_value(&r).unwrap());
    assert_eq!(receipt.3, super::super::finance::data_hash(&baseline.data));
    assert_eq!(
        receipt.4,
        Sha256::digest(serde_json::to_vec(&baseline.permissions).unwrap()).to_vec()
    );
    let contracts: i64 = sqlx::query_scalar("SELECT count(*) FROM core_meta.component_contract")
        .fetch_one(&mut inspect)
        .await
        .unwrap();
    assert_eq!(contracts, 69);
    let current_catalog = hex(fingerprint(&definitions(&mut inspect).await.unwrap()));
    assert_eq!(current_catalog, committed.successor_catalog_sha256);
    let recorded: String = sqlx::query_scalar(
        "SELECT encode(catalog_fingerprint,'hex') FROM core_meta.runtime_contract WHERE singleton",
    )
    .fetch_one(&mut inspect)
    .await
    .unwrap();
    assert_eq!(recorded, current_catalog);
    inspect.close().await.unwrap();

    // NOLOGIN runtime is the production offline contract. Enter that exact
    // PostgreSQL session identity using the isolated test superuser, not an
    // installer connection or an altered production role contract.
    engine_offline_boundary(&r).await;
    let mut runtime = PgConnection::connect_with(&admin_options).await.unwrap();
    sqlx::query(sqlx::AssertSqlSafe(format!(
        "SET SESSION AUTHORIZATION {}",
        quote(&r.roles.runtime)
    )))
    .execute(&mut runtime)
    .await
    .unwrap();
    let identities: (String, String) =
        sqlx::query_as("SELECT current_user::text,session_user::text")
            .fetch_one(&mut runtime)
            .await
            .unwrap();
    assert_eq!(
        identities,
        (r.roles.runtime.clone(), r.roles.runtime.clone())
    );
    for statement in [
        "SELECT * FROM core_model.responses_continuations",
        "DELETE FROM core_model.responses_continuations",
        "UPDATE core_model.responses_continuations SET active=active",
        "TRUNCATE core_model.responses_continuations",
    ] {
        let error = sqlx::query(statement)
            .execute(&mut runtime)
            .await
            .unwrap_err();
        assert_eq!(
            error.as_database_error().and_then(|e| e.code()).as_deref(),
            Some("42501")
        );
    }
    for statement in [
        "SELECT * FROM core_meta.responses_continuity_upgrades",
        "UPDATE core_meta.runtime_contract SET catalog_fingerprint=catalog_fingerprint",
    ] {
        let error = sqlx::query(statement)
            .execute(&mut runtime)
            .await
            .err()
            .unwrap();
        assert_eq!(
            error.as_database_error().and_then(|e| e.code()).as_deref(),
            Some("42501")
        );
    }
    assert!(matches!(
        connected(&mut runtime, &r).await,
        Err(UpgradeError::Rejected("UPGRADE_DB_INSTALLER_MISMATCH"))
    ));
    runtime.close().await.unwrap();
    engine_offline_boundary(&r).await;
    let final_replay = upgrade_database(url.as_str(), r.clone()).await.unwrap();
    assert_eq!(
        final_replay.status, "already_committed",
        "runtime denials must not damage the upgrade receipt"
    );

    // Only this SQLx-owned database and its random synthetic principals.
    let mut admin = PgConnection::connect_with(&admin_options).await.unwrap();
    sqlx::raw_sql(sqlx::AssertSqlSafe(format!("DROP OWNED BY {installer},{runtime},{extensions},{owner} CASCADE; ALTER DATABASE {database} OWNER TO {original}; DROP ROLE {installer},{runtime},{extensions},{owner};",
        installer=quote(&r.roles.installer),runtime=quote(&r.roles.runtime),extensions=quote(&r.roles.extensions),owner=quote(&r.roles.owner),database=quote(&r.database.name),original=quote(&original))))
        .execute(&mut admin).await.unwrap();
    admin.close().await.unwrap();
}

#[sqlx::test(migrations = false)]
async fn app07_fresh69_matches_captured_target_and_denies_runtime_tables(pool: PgPool) {
    assert_eq!(crate::core_schema::component_count(), 69);
    crate::core_schema::init(&pool).await.unwrap();
    crate::core_schema::check(&pool).await.unwrap();
    let actual: String = sqlx::query_scalar(
        "SELECT encode(catalog_fingerprint,'hex') FROM core_meta.runtime_contract WHERE singleton",
    )
    .fetch_one(&pool)
    .await
    .unwrap();
    let pin: serde_json::Value = serde_json::from_str(TARGET).unwrap();
    assert_eq!(actual, pin["catalog_sha256"].as_str().unwrap());
    assert_eq!(manifest(69), pin["manifest_sha256"].as_str().unwrap());
    let runtime = crate::test_runtime::RuntimeRole::create(&pool).await;
    runtime.optional("core_model").await;
    assert!(
        sqlx::query("SELECT * FROM core_model.responses_continuations")
            .fetch_all(&runtime.pool)
            .await
            .is_err()
    );
    assert!(
        sqlx::query("DELETE FROM core_model.responses_continuations")
            .execute(&runtime.pool)
            .await
            .is_err()
    );
    runtime.cleanup().await;
}

#[sqlx::test(migrations = false)]
async fn app07_independent_http_predecessor_upgrade_adapter(pool: PgPool) {
    use crate::core_schema::{UpgradeDatabase, UpgradeRoles};
    use sqlx::postgres::PgPoolOptions;
    // A missing/mismatched reviewed pin is a failure, not a skipped engine test.
    authorize_target()
        .expect("capture and fix reviewed 68/69 pins before running engine acceptance");
    let facts = sqlx::query("SELECT current_database()::text AS name,d.oid::bigint AS oid,current_setting('server_version_num')::bigint AS version,(SELECT system_identifier::text FROM pg_control_system()) AS system,session_user::text AS original FROM pg_database d WHERE datname=current_database()")
        .fetch_one(&pool).await.unwrap();
    let suffix = format!("{:016x}", rand::random::<u64>());
    let r = UpgradeRequest {
        version: 1,
        procedure: PROCEDURE.into(),
        upgrade_id: format!("app07-independent-{suffix}"),
        database: UpgradeDatabase {
            name: facts.get("name"),
            oid: u32::try_from(facts.get::<i64, _>("oid")).unwrap(),
            system_identifier: facts.get("system"),
            server_version_num: u32::try_from(facts.get::<i64, _>("version")).unwrap(),
        },
        roles: UpgradeRoles {
            installer: format!("app07_i_{suffix}"),
            owner: format!("app07_o_{suffix}"),
            runtime: format!("app07_r_{suffix}"),
            extensions: format!("app07_e_{suffix}"),
        },
        predecessor_manifest_sha256: ENGINE.predecessor_manifest.into(),
        predecessor_catalog_sha256: ENGINE.predecessor_catalog.into(),
        successor_manifest_sha256: ENGINE.target_manifest().unwrap(),
    };
    assert_eq!(r.database.server_version_num, 180006);
    assert_eq!(manifest(68), r.predecessor_manifest_sha256);
    let original: String = facts.get("original");
    let admin_options = (*pool.connect_options()).clone();
    let password = format!("synthetic-{:032x}", rand::random::<u128>());
    sqlx::raw_sql(sqlx::AssertSqlSafe(format!("CREATE ROLE {owner} NOLOGIN; CREATE ROLE {installer} LOGIN NOINHERIT PASSWORD '{password}'; CREATE ROLE {runtime} NOLOGIN; CREATE ROLE {extensions} NOLOGIN; GRANT {owner} TO {installer} WITH INHERIT FALSE, SET TRUE; ALTER DATABASE {database} OWNER TO {owner}; REVOKE CONNECT,CREATE,TEMP ON DATABASE {database} FROM PUBLIC; GRANT CONNECT ON DATABASE {database} TO {installer}; REVOKE EXECUTE ON FUNCTION pg_catalog.pg_control_system() FROM PUBLIC; GRANT EXECUTE ON FUNCTION pg_catalog.pg_control_system() TO {owner};",
        owner=quote(&r.roles.owner), installer=quote(&r.roles.installer), runtime=quote(&r.roles.runtime), extensions=quote(&r.roles.extensions), database=quote(&r.database.name))))
        .execute(&pool).await.unwrap();
    let installer = PgPoolOptions::new()
        .max_connections(1)
        .connect_with(
            admin_options
                .clone()
                .username(&r.roles.installer)
                .password(&password)
                .options([("role", r.roles.owner.as_str())]),
        )
        .await
        .unwrap();
    prefix68(&installer).await;
    sqlx::raw_sql(sqlx::AssertSqlSafe(format!(
        "GRANT USAGE ON SCHEMA core_model TO {}",
        quote(&r.roles.runtime)
    )))
    .execute(&installer)
    .await
    .unwrap();
    crate::model_http::seed_continuity_upgrade_charges(&installer, &pool).await;
    let mut url = reqwest::Url::parse(&std::env::var("DATABASE_URL").unwrap()).unwrap();
    url.set_username(&r.roles.installer).unwrap();
    url.set_password(Some(&password)).unwrap();
    url.set_path(&format!("/{}", r.database.name));
    pool.close().await;
    crate::model_http::verify_continuity_upgrade_fixture(installer, url.as_str(), r.clone()).await;
    let mut admin = PgConnection::connect_with(&admin_options).await.unwrap();
    sqlx::raw_sql(sqlx::AssertSqlSafe(format!("DROP OWNED BY {installer},{runtime},{extensions},{owner} CASCADE; ALTER DATABASE {database} OWNER TO {original}; DROP ROLE {installer},{runtime},{extensions},{owner};",
        installer=quote(&r.roles.installer),runtime=quote(&r.roles.runtime),extensions=quote(&r.roles.extensions),owner=quote(&r.roles.owner),database=quote(&r.database.name),original=quote(&original))))
        .execute(&mut admin).await.unwrap();
    admin.close().await.unwrap();
}

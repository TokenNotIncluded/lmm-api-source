//! Owner-only isolated PG capture and additive preservation evidence. No runtime
//! path bypasses reviewed pins. This fixture builds only the 66/67 prefixes even
//! when another agent appends component 68.
use super::*;
use sqlx::PgPool;

async fn prefix66(pool: &PgPool) {
    use crate::core_schema as root;
    let mut tx = pool.begin().await.unwrap();
    for (_, ddl) in &COMPONENTS[..66] {
        sqlx::raw_sql(ddl).execute(&mut *tx).await.unwrap();
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
    for (name, ddl) in &COMPONENTS[..66] {
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
        .bootstrap_user(67001, 0)
        .await
        .unwrap();
}
fn manifest(count: usize) -> String {
    let mut m = schema_manifest();
    m.components.truncate(count);
    let mut bytes = serde_json::to_vec(&m).unwrap();
    bytes.push(b'\n');
    hex(&Sha256::digest(bytes))
}
fn key(fact: &str) -> Option<String> {
    let v: serde_json::Value = serde_json::from_str(fact).unwrap();
    match v[0].as_str() {
        Some("schema") => Some(format!("schema:{}", v[1])),
        Some("function") => Some(format!("function:{}:{}", v[1], v[2])),
        _ => None,
    }
}
async fn measure(pool: &PgPool) -> serde_json::Value {
    prefix66(pool).await;
    assert_eq!(COMPONENTS[66].0, "payment-discounts");
    let mut tx = pool.begin().await.unwrap();
    let version: i32 = sqlx::query_scalar("SELECT current_setting('server_version_num')::int")
        .fetch_one(&mut *tx)
        .await
        .unwrap();
    assert_eq!(version, 180006, "capture is pinned to PostgreSQL 18.6");
    let before = hex(&fingerprint(&definitions(&mut tx).await.unwrap()));
    let tables: Vec<String> = sqlx::query_scalar("SELECT format('%I.%I',n.nspname,c.relname) FROM pg_class c JOIN pg_namespace n ON n.oid=c.relnamespace WHERE n.nspname NOT IN ('pg_catalog','information_schema') AND left(n.nspname,3)<>'pg_' AND c.relkind IN ('r','p') ORDER BY n.nspname,c.relname").fetch_all(&mut *tx).await.unwrap();
    let sequences: Vec<String> = sqlx::query_scalar("SELECT format('%I.%I',n.nspname,c.relname) FROM pg_class c JOIN pg_namespace n ON n.oid=c.relnamespace WHERE n.nspname NOT IN ('pg_catalog','information_schema') AND left(n.nspname,3)<>'pg_' AND c.relkind='S' ORDER BY n.nspname,c.relname").fetch_all(&mut *tx).await.unwrap();
    let old_data =
        super::super::request_operations::preservation::data(&mut tx, &tables, &sequences)
            .await
            .unwrap();
    let rels = super::super::finance::object_ids(&mut tx, "relation")
        .await
        .unwrap();
    let types = super::super::finance::object_ids(&mut tx, "type")
        .await
        .unwrap();
    // Roles exist only in this rolled-back transaction, so repeated runs leave
    // no cluster-wide principals or grants behind.
    let oid: i64 =
        sqlx::query_scalar("SELECT oid::bigint FROM pg_database WHERE datname=current_database()")
            .fetch_one(&mut *tx)
            .await
            .unwrap();
    let runtime = format!("app05_runtime_{oid}");
    let extension = format!("app05_extension_{oid}");
    sqlx::raw_sql(&format!(
        "CREATE ROLE {} NOLOGIN; CREATE ROLE {} NOLOGIN",
        quote(&runtime),
        quote(&extension)
    ))
    .execute(&mut *tx)
    .await
    .unwrap();
    let old_permissions = super::super::finance::permissions(&mut tx, &rels, &types)
        .await
        .unwrap();
    let keys: std::collections::BTreeSet<String> =
        old_permissions.iter().filter_map(|v| key(v)).collect();
    sqlx::raw_sql(COMPONENTS[66].1)
        .execute(&mut *tx)
        .await
        .unwrap();
    grant_runtime(&mut tx, &runtime).await.unwrap();
    let after = hex(&fingerprint(&definitions(&mut tx).await.unwrap()));
    let new_permissions: Vec<String> = super::super::finance::permissions(&mut tx, &rels, &types)
        .await
        .unwrap()
        .into_iter()
        .filter(|v| key(v).is_none_or(|k| keys.contains(&k)))
        .collect();
    assert_eq!(
        old_permissions, new_permissions,
        "predecessor ACLs/owners must survive"
    );
    assert_eq!(
        old_data,
        super::super::request_operations::preservation::data(&mut tx, &tables, &sequences)
            .await
            .unwrap(),
        "predecessor rows and sequences must survive"
    );
    for table in [
        "core_payments.discount_codes",
        "core_payments.discount_reservations",
    ] {
        let permissions: (bool, bool, bool) = sqlx::query_as("SELECT has_table_privilege($1,$3,'SELECT,INSERT,UPDATE'),has_table_privilege($2,$3,'SELECT,INSERT,UPDATE,DELETE,TRUNCATE'),has_table_privilege($1,$3,'DELETE,TRUNCATE')")
            .bind(&runtime).bind(&extension).bind(table).fetch_one(&mut *tx).await.unwrap();
        assert_eq!(permissions, (true, false, false));
    }
    let receipt_private: bool = sqlx::query_scalar("SELECT NOT has_table_privilege($1,'core_meta.payment_discount_upgrades','SELECT,INSERT,UPDATE,DELETE,TRUNCATE') AND NOT has_table_privilege($2,'core_meta.payment_discount_upgrades','SELECT,INSERT,UPDATE,DELETE,TRUNCATE')")
        .bind(&runtime).bind(&extension).fetch_one(&mut *tx).await.unwrap();
    assert!(receipt_private);
    tx.rollback().await.unwrap();
    serde_json::json!({"predecessor":{"component_count":66,"manifest_sha256":manifest(66),"catalog_sha256":before},
        "successor":{"component_count":67,"manifest_sha256":manifest(67),"catalog_sha256":after},
        "delta_raw_schema_sha256":hex(&Sha256::digest(COMPONENTS[66].1.as_bytes())),
        "runtime_grants_sha256":hex(&Sha256::digest(include_bytes!("../../../schema/payment_discounts_grants.sql"))),
        "predecessor_data_preserved":true,"predecessor_acl_preserved":true})
}

#[sqlx::test(migrations = false)]
#[ignore = "explicit isolated PG18.6 owner with CREATEROLE and private capture output required"]
async fn app05_capture_66_to67(pool: PgPool) {
    use std::{fs::OpenOptions, io::Write, os::unix::fs::OpenOptionsExt};
    let output = std::env::var("LMM_APP05_CATALOG_CAPTURE").unwrap();
    let mut file = OpenOptions::new()
        .write(true)
        .create_new(true)
        .mode(0o600)
        .open(output)
        .unwrap();
    let measured = measure(&pool).await;
    writeln!(file, "{}", serde_json::to_string_pretty(&measured).unwrap()).unwrap();
    file.sync_all().unwrap();
}

#[sqlx::test(migrations = false)]
async fn app05_66_to67_preserves_data_and_acl(pool: PgPool) {
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
    assert_eq!(
        measured["successor"]["manifest_sha256"], pin["manifest_sha256"],
        "real capture must replace temporary null pin"
    );
    assert_eq!(
        measured["successor"]["catalog_sha256"],
        pin["catalog_sha256"]
    );
    authorize_target().unwrap();
}

// Uses the same four-role, real-connection boundary as completion_upgrade.rs.
// The observer connects to DATABASE_URL's control database, never the target.
pub(crate) async fn engine_offline_boundary(request: &UpgradeRequest) {
    let mut control = PgConnection::connect(&std::env::var("DATABASE_URL").unwrap())
        .await
        .unwrap();
    let database: String = sqlx::query_scalar("SELECT current_database()::text")
        .fetch_one(&mut control)
        .await
        .unwrap();
    assert_ne!(database, request.database.name);
    tokio::time::timeout(std::time::Duration::from_secs(5), async {
        let mut tick = tokio::time::interval(std::time::Duration::from_millis(10));
        loop {
            let remaining: i64 =
                sqlx::query_scalar("SELECT count(*) FROM pg_stat_activity WHERE datid::bigint=$1")
                    .bind(i64::from(request.database.oid))
                    .fetch_one(&mut control)
                    .await
                    .unwrap();
            if remaining == 0 {
                break;
            }
            tick.tick().await;
        }
    })
    .await
    .expect("SQLx-owned target must have no remaining backend before upgrade");
    control.close().await.unwrap();
}

pub(crate) struct EngineBaseline {
    tables: Vec<String>,
    sequences: Vec<String>,
    relations: Vec<i64>,
    types: Vec<i64>,
    keys: std::collections::BTreeSet<String>,
    pub(crate) data: BTreeMap<String, Vec<u8>>,
    pub(crate) permissions: Vec<String>,
}
impl EngineBaseline {
    pub(crate) async fn capture(c: &mut PgConnection) -> Self {
        let mut tx = c.begin().await.unwrap();
        sqlx::query("SET LOCAL search_path=pg_catalog")
            .execute(&mut *tx)
            .await
            .unwrap();
        let tables = sqlx::query_scalar("SELECT format('%I.%I',n.nspname,c.relname) FROM pg_class c JOIN pg_namespace n ON n.oid=c.relnamespace WHERE n.nspname NOT IN ('pg_catalog','information_schema') AND left(n.nspname,3)<>'pg_' AND c.relkind IN ('r','p') ORDER BY n.nspname,c.relname")
            .fetch_all(&mut *tx).await.unwrap();
        let sequences = sqlx::query_scalar("SELECT format('%I.%I',n.nspname,c.relname) FROM pg_class c JOIN pg_namespace n ON n.oid=c.relnamespace WHERE n.nspname NOT IN ('pg_catalog','information_schema') AND left(n.nspname,3)<>'pg_' AND c.relkind='S' ORDER BY n.nspname,c.relname")
            .fetch_all(&mut *tx).await.unwrap();
        let relations = super::super::finance::object_ids(&mut tx, "relation")
            .await
            .unwrap();
        let types = super::super::finance::object_ids(&mut tx, "type")
            .await
            .unwrap();
        let permissions = super::super::finance::permissions(&mut tx, &relations, &types)
            .await
            .unwrap();
        let keys = permissions.iter().filter_map(|v| key(v)).collect();
        let data =
            super::super::request_operations::preservation::data(&mut tx, &tables, &sequences)
                .await
                .unwrap();
        tx.commit().await.unwrap();
        Self {
            tables,
            sequences,
            relations,
            types,
            keys,
            data,
            permissions,
        }
    }
    pub(crate) async fn assert_preserved(&self, c: &mut PgConnection) {
        let mut tx = c.begin().await.unwrap();
        sqlx::query("SET LOCAL search_path=pg_catalog")
            .execute(&mut *tx)
            .await
            .unwrap();
        assert_eq!(
            self.data,
            super::super::request_operations::preservation::data(
                &mut tx,
                &self.tables,
                &self.sequences
            )
            .await
            .unwrap(),
            "all old rows and exact sequence states must survive engine execution/replay"
        );
        let permissions: Vec<String> =
            super::super::finance::permissions(&mut tx, &self.relations, &self.types)
                .await
                .unwrap()
                .into_iter()
                .filter(|v| key(v).is_none_or(|k| self.keys.contains(&k)))
                .collect();
        assert_eq!(
            self.permissions, permissions,
            "all old role, relation, column, function and schema ACL facts must survive"
        );
        tx.commit().await.unwrap();
    }
}

#[sqlx::test(migrations = false)]
async fn app05_upgrade_engine_66_to67_exact_replay_and_runtime_denial(pool: PgPool) {
    use crate::core_schema::{UpgradeDatabase, UpgradeRoles, upgrade_database};
    use sqlx::postgres::PgPoolOptions;
    // A missing/mismatched reviewed pin is a failure, not a skipped engine test.
    authorize_target()
        .expect("capture and fix reviewed 66/67 pins before running engine acceptance");
    let facts = sqlx::query("SELECT current_database()::text AS name,d.oid::bigint AS oid,current_setting('server_version_num')::bigint AS version,(SELECT system_identifier::text FROM pg_control_system()) AS system,session_user::text AS original FROM pg_database d WHERE datname=current_database()")
        .fetch_one(&pool).await.unwrap();
    let suffix = format!("{:016x}", rand::random::<u64>());
    let r = UpgradeRequest {
        version: 1,
        procedure: PROCEDURE.into(),
        upgrade_id: format!("app05-engine-{suffix}"),
        database: UpgradeDatabase {
            name: facts.get("name"),
            oid: u32::try_from(facts.get::<i64, _>("oid")).unwrap(),
            system_identifier: facts.get("system"),
            server_version_num: u32::try_from(facts.get::<i64, _>("version")).unwrap(),
        },
        roles: UpgradeRoles {
            installer: format!("app05_i_{suffix}"),
            owner: format!("app05_o_{suffix}"),
            runtime: format!("app05_r_{suffix}"),
            extensions: format!("app05_e_{suffix}"),
        },
        predecessor_manifest_sha256: ENGINE.predecessor_manifest.into(),
        predecessor_catalog_sha256: ENGINE.predecessor_catalog.into(),
        successor_manifest_sha256: ENGINE.target_manifest().unwrap(),
    };
    assert_eq!(r.database.server_version_num, 180006);
    assert_eq!(manifest(66), r.predecessor_manifest_sha256);
    let original: String = facts.get("original");
    let admin_options = (*pool.connect_options()).clone();
    let password = format!("synthetic-{:032x}", rand::random::<u128>());
    sqlx::raw_sql(&format!("CREATE ROLE {owner} NOLOGIN; CREATE ROLE {installer} LOGIN NOINHERIT PASSWORD '{password}'; CREATE ROLE {runtime} NOLOGIN; CREATE ROLE {extensions} NOLOGIN; GRANT {owner} TO {installer} WITH INHERIT FALSE, SET TRUE; ALTER DATABASE {database} OWNER TO {owner}; REVOKE CONNECT,CREATE,TEMP ON DATABASE {database} FROM PUBLIC; GRANT CONNECT ON DATABASE {database} TO {installer}; REVOKE EXECUTE ON FUNCTION pg_catalog.pg_control_system() FROM PUBLIC; GRANT EXECUTE ON FUNCTION pg_catalog.pg_control_system() TO {owner};",
        owner=quote(&r.roles.owner), installer=quote(&r.roles.installer), runtime=quote(&r.roles.runtime), extensions=quote(&r.roles.extensions), database=quote(&r.database.name)))
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
    prefix66(&installer).await;
    let identity = IdentityStore::from_pool(installer.clone());
    let root = identity.bootstrap_user(67002, 6).await.unwrap();
    crate::identity::admin_users::AdminUsersStore::new(identity)
        .adjust_user_balance(
            &root.secret,
            67001,
            &crate::identity::admin_users::AdjustmentRequest {
                delta_units: "1234567".into(),
                operation_key: "app05-real-upgrade-ledger-fixture".into(),
                reason: "isolated upgrade preservation fixture".into(),
            },
        )
        .await
        .unwrap();
    // Exercise non-default pre-existing schema/table/column/sequence/function ACLs.
    sqlx::raw_sql(&format!("GRANT USAGE ON SCHEMA core_identity,core_payments,core_meta TO {runtime}; GRANT SELECT ON core_identity.users TO {runtime}; GRANT UPDATE(revoked_at) ON core_identity.credentials TO {runtime}; GRANT USAGE ON SEQUENCE core_identity.credentials_id_seq TO {runtime}; GRANT EXECUTE ON FUNCTION public.routing_stage_prices(jsonb) TO {runtime};",
        runtime=quote(&r.roles.runtime))).execute(&installer).await.unwrap();
    let mut connection = installer.acquire().await.unwrap();
    assert_eq!(
        hex(&fingerprint(&definitions(&mut connection).await.unwrap())),
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
    let committed = upgrade_database(url.as_str(), r.clone()).await.unwrap();
    assert_eq!(committed.status, "committed");
    assert_eq!(committed.components, 67);
    assert_eq!(committed.request_sha256, hex(&r.digest().unwrap()));
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
            "PAYMENT_DISCOUNTS_ORIGINAL_RECEIPT_ABSENT"
        ))
    ));
    let mut changed = r.clone();
    std::mem::swap(&mut changed.roles.runtime, &mut changed.roles.extensions);
    engine_offline_boundary(&r).await;
    assert!(matches!(
        upgrade_database(url.as_str(), changed).await,
        Err(UpgradeError::Rejected(
            "PAYMENT_DISCOUNTS_RECEIPT_OR_CATALOG_CONFLICT"
        ))
    ));

    let mut inspect = PgConnection::connect_with(&admin_options).await.unwrap();
    baseline.assert_preserved(&mut inspect).await;
    let receipt: (i64, Vec<u8>, serde_json::Value, Vec<u8>, Vec<u8>) = sqlx::query_as("SELECT count(*) OVER(),request_sha256,request,preserved_data_sha256,preserved_permissions_sha256 FROM core_meta.payment_discount_upgrades WHERE upgrade_id=$1")
        .bind(&r.upgrade_id).fetch_one(&mut inspect).await.unwrap();
    assert_eq!(receipt.0, 1);
    let receipt_count: i64 =
        sqlx::query_scalar("SELECT count(*) FROM core_meta.payment_discount_upgrades")
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
    assert_eq!(contracts, 67);
    let current_catalog = hex(&fingerprint(&definitions(&mut inspect).await.unwrap()));
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
    sqlx::query(&format!(
        "SET SESSION AUTHORIZATION {}",
        quote(&r.roles.runtime)
    ))
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
    for table in [
        "core_payments.discount_codes",
        "core_payments.discount_reservations",
    ] {
        let count: i64 = sqlx::query_scalar(&format!("SELECT count(*) FROM {table}"))
            .fetch_one(&mut runtime)
            .await
            .unwrap();
        assert_eq!(count, 0, "new runtime SELECT permission is usable");
        let error = sqlx::query(&format!("DELETE FROM {table}"))
            .execute(&mut runtime)
            .await
            .err()
            .unwrap();
        assert_eq!(
            error.as_database_error().and_then(|e| e.code()).as_deref(),
            Some("42501")
        );
    }
    for statement in [
        "SELECT * FROM core_meta.payment_discount_upgrades",
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
    sqlx::raw_sql(&format!("DROP OWNED BY {installer},{runtime},{extensions},{owner} CASCADE; ALTER DATABASE {database} OWNER TO {original}; DROP ROLE {installer},{runtime},{extensions},{owner};",
        installer=quote(&r.roles.installer),runtime=quote(&r.roles.runtime),extensions=quote(&r.roles.extensions),owner=quote(&r.roles.owner),database=quote(&r.database.name),original=quote(&original)))
        .execute(&mut admin).await.unwrap();
    admin.close().await.unwrap();
}

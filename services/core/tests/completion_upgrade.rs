//! Exact 32-component predecessor upgraded through the public offline Core API.
//! Uses real separate installer/owner/runtime identities and preserves financial facts.
use lmm_core::core_schema::{
    UpgradeDatabase, UpgradeError, UpgradeRequest, UpgradeRoles, schema_manifest, upgrade_database,
};
use sha2::{Digest, Sha256};
use sqlx::{Connection, PgConnection, Row, postgres::PgPoolOptions};

const PROCEDURE: &str = "core-functional-completion-v1";
const PRE_MANIFEST: &str = "9c9726a7fa204a150d9fe18139d4580d23feb4cec40145fcb829338331a4cd15";
const PRE_CATALOG: &str = "abe3979b376da869d4a05b32dda8c3ad33197497db7af16970d8c9b1153bc6ce";
#[path = "common/completion_predecessor.rs"]
mod predecessor;

fn quote(value: &str) -> String {
    format!("\"{}\"", value.replace('"', "\"\""))
}
fn manifest_sha256() -> String {
    let mut bytes = serde_json::to_vec(&schema_manifest()).unwrap();
    bytes.push(b'\n');
    format!("{:x}", Sha256::digest(bytes))
}
async fn definitions(connection: &mut PgConnection) -> Result<Vec<String>, sqlx::Error> {
    sqlx::query_scalar(schema_manifest().catalog.query_utf8)
        .fetch_all(connection)
        .await
}

async fn sequence_states(
    c: &mut PgConnection,
    names: &[String],
) -> Vec<(String, serde_json::Value)> {
    let mut states = Vec::new();
    for name in names {
        // Identifiers originate from pg_catalog's format('%I.%I', ...).
        let value = sqlx::query_scalar(&format!(
            "SELECT jsonb_build_array(last_value,log_cnt,is_called) FROM {name}"
        ))
        .fetch_one(&mut *c)
        .await
        .unwrap();
        states.push((name.clone(), value));
    }
    states
}

async fn exercised_authority(c: &mut PgConnection) -> serde_json::Value {
    sqlx::query_scalar(r#"
SELECT jsonb_build_object(
 'schema',(SELECT jsonb_build_array(nspowner,nspacl) FROM pg_namespace WHERE nspname='core_identity'),
 'relations',(SELECT jsonb_agg(jsonb_build_array(relname,relowner,relacl) ORDER BY relname)
   FROM pg_class WHERE oid IN ('core_identity.users'::regclass,'core_identity.credentials'::regclass,'core_identity.credentials_id_seq'::regclass)),
 'column',(SELECT attacl::text FROM pg_attribute WHERE attrelid='core_identity.credentials'::regclass AND attname='revoked_at'),
 'replaced_function',(SELECT jsonb_build_array(proowner,proacl) FROM pg_proc WHERE oid='public.routing_stage_prices(jsonb)'::regprocedure))
"#)
        .fetch_one(c)
        .await
        .unwrap()
}

fn request() -> UpgradeRequest {
    UpgradeRequest {
        version: 1,
        procedure: PROCEDURE.into(),
        upgrade_id: "synthetic-completion-upgrade".into(),
        database: UpgradeDatabase {
            name: "synthetic_completion".into(),
            oid: 1,
            system_identifier: "1".into(),
            server_version_num: 180006,
        },
        roles: UpgradeRoles {
            installer: "synthetic_installer".into(),
            owner: "synthetic_owner".into(),
            runtime: "synthetic_runtime".into(),
            extensions: "synthetic_extensions".into(),
        },
        predecessor_manifest_sha256: PRE_MANIFEST.into(),
        predecessor_catalog_sha256: PRE_CATALOG.into(),
        successor_manifest_sha256: manifest_sha256(),
    }
}
#[sqlx::test(migrations = false)]
async fn completion_upgrade_preserves_real_predecessor_and_resolves_exact_replay(
    pool: sqlx::PgPool,
) {
    let suffix = format!("{:012x}", rand::random::<u64>());
    let mut r = request();
    r.roles = UpgradeRoles {
        installer: format!("ops_i_{suffix}"),
        owner: format!("ops_o_{suffix}"),
        runtime: format!("ops_r_{suffix}"),
        extensions: format!("ops_e_{suffix}"),
    };
    r.database.name = pool.connect_options().get_database().unwrap().to_string();
    let facts=sqlx::query("SELECT d.oid::bigint AS oid,current_setting('server_version_num')::bigint AS version,(SELECT system_identifier::text FROM pg_control_system()) AS system,session_user::text AS original FROM pg_database d WHERE datname=current_database()")
        .fetch_one(&pool).await.unwrap();
    r.database.oid = facts.get::<i64, _>("oid") as u32;
    r.database.server_version_num = facts.get::<i64, _>("version") as u32;
    r.database.system_identifier = facts.get("system");
    let original: String = facts.get("original");
    let password = format!("synthetic-{:032x}", rand::random::<u128>());
    sqlx::raw_sql(&format!("CREATE ROLE {owner} NOLOGIN; CREATE ROLE {installer} LOGIN NOINHERIT PASSWORD '{password}'; CREATE ROLE {runtime} NOLOGIN; CREATE ROLE {extensions} NOLOGIN; GRANT {owner} TO {installer} WITH INHERIT FALSE, SET TRUE; ALTER DATABASE {database} OWNER TO {owner}; REVOKE CONNECT,CREATE,TEMP ON DATABASE {database} FROM PUBLIC; GRANT CONNECT ON DATABASE {database} TO {installer}; REVOKE EXECUTE ON FUNCTION pg_catalog.pg_control_system() FROM PUBLIC; GRANT EXECUTE ON FUNCTION pg_catalog.pg_control_system() TO {owner};",
        owner=quote(&r.roles.owner),installer=quote(&r.roles.installer),runtime=quote(&r.roles.runtime),extensions=quote(&r.roles.extensions),database=quote(&r.database.name)))
        .execute(&pool).await.unwrap();
    let options = (*pool.connect_options())
        .clone()
        .username(&r.roles.installer)
        .password(&password)
        .options([("role", r.roles.owner.as_str())]);
    let installer = PgPoolOptions::new()
        .max_connections(1)
        .connect_with(options)
        .await
        .unwrap();
    predecessor::install(&installer).await;
    // Exercise real pre-existing schema/table/column/sequence/function grants.
    // The function is replaced by channel-control, so its ACL must survive too.
    sqlx::raw_sql(&format!(
        "GRANT USAGE ON SCHEMA core_identity TO {runtime}; \
         GRANT SELECT ON core_identity.users TO {runtime}; \
         GRANT UPDATE(revoked_at) ON core_identity.credentials TO {runtime}; \
         GRANT USAGE ON SEQUENCE core_identity.credentials_id_seq TO {runtime}; \
         GRANT EXECUTE ON FUNCTION public.routing_stage_prices(jsonb) TO {runtime};",
        runtime = quote(&r.roles.runtime)
    ))
    .execute(&installer)
    .await
    .unwrap();
    let mut conn = installer.acquire().await.unwrap();
    let before = definitions(&mut conn).await.unwrap();
    let mut hash = Sha256::new();
    for definition in before {
        hash.update((definition.len() as u64).to_be_bytes());
        hash.update(definition.as_bytes());
    }
    assert_eq!(format!("{:x}", hash.finalize()), PRE_CATALOG);
    drop(conn);
    let identity = lmm_core::identity::IdentityStore::from_pool(installer.clone());
    let root = identity.bootstrap_user(7001, 6).await.unwrap();
    identity.bootstrap_user(7002, 0).await.unwrap();
    lmm_core::identity::admin_users::AdminUsersStore::new(identity.clone())
        .adjust_user_balance(
            &root.secret,
            7002,
            &lmm_core::identity::admin_users::AdjustmentRequest {
                delta_units: "1234567".into(),
                operation_key: "synthetic-completion-upgrade-balance-fixture".into(),
                reason: "isolated upgrade preservation fixture".into(),
            },
        )
        .await
        .unwrap();
    let before_users: Vec<String> =
        sqlx::query_scalar("SELECT to_jsonb(u)::text FROM core_identity.users u ORDER BY id")
            .fetch_all(&installer)
            .await
            .unwrap();
    let before_finance: Vec<String> = sqlx::query_scalar(
        "SELECT to_jsonb(j)::text FROM core_billing.ledger_journals j ORDER BY id",
    )
    .fetch_all(&installer)
    .await
    .unwrap();
    assert!(!before_finance.is_empty());
    let mut conn = installer.acquire().await.unwrap();
    sqlx::query("SELECT setval('core_identity.credentials_id_seq',9000,false)")
        .execute(&mut *conn)
        .await
        .unwrap();
    let sequences: Vec<String> = sqlx::query_scalar(
        "SELECT format('%I.%I',n.nspname,c.relname) FROM pg_class c JOIN pg_namespace n ON n.oid=c.relnamespace \
         WHERE c.relkind='S' AND n.nspname NOT IN ('pg_catalog','information_schema') \
         AND left(n.nspname,3)<>'pg_' ORDER BY n.nspname,c.relname",
    )
    .fetch_all(&mut *conn)
    .await
    .unwrap();
    assert!(!sequences.is_empty());
    let before_sequences = sequence_states(&mut conn, &sequences).await;
    let before_authority = exercised_authority(&mut conn).await;
    let actual_grants: (bool, bool, bool, bool) = sqlx::query_as(
        "SELECT has_table_privilege($1,'core_identity.users','SELECT'), \
         has_column_privilege($1,'core_identity.credentials','revoked_at','UPDATE'), \
         has_sequence_privilege($1,'core_identity.credentials_id_seq','USAGE'), \
         has_function_privilege($1,'public.routing_stage_prices(jsonb)','EXECUTE')",
    )
    .bind(&r.roles.runtime)
    .fetch_one(&mut *conn)
    .await
    .unwrap();
    assert_eq!(actual_grants, (true, true, true, true));
    drop(conn);
    let mut url = reqwest::Url::parse(&std::env::var("DATABASE_URL").unwrap()).unwrap();
    url.set_path(&format!("/{}", r.database.name));
    url.set_username(&r.roles.installer).unwrap();
    url.set_password(Some(&password)).unwrap();
    installer.close().await;
    pool.close().await;
    let result = upgrade_database(url.as_str(), r.clone()).await.unwrap();
    assert_eq!(result.status, "committed");
    assert_eq!(result.components, lmm_core::core_schema::component_count());
    let replay = upgrade_database(url.as_str(), r.clone()).await.unwrap();
    assert_eq!(replay.status, "already_committed");
    assert_eq!(
        replay.successor_catalog_sha256,
        result.successor_catalog_sha256
    );
    let mut another = r.clone();
    another.upgrade_id.push('2');
    assert!(matches!(
        upgrade_database(url.as_str(), another).await,
        Err(UpgradeError::Rejected("COMPLETION_ORIGINAL_RECEIPT_ABSENT"))
    ));
    let mut changed_request = r.clone();
    std::mem::swap(
        &mut changed_request.roles.runtime,
        &mut changed_request.roles.extensions,
    );
    assert!(matches!(
        upgrade_database(url.as_str(), changed_request).await,
        Err(UpgradeError::Rejected(
            "COMPLETION_RECEIPT_OR_CATALOG_CONFLICT"
        ))
    ));
    let mut inspect = PgConnection::connect(url.as_str()).await.unwrap();
    sqlx::query(&format!("SET ROLE {}", quote(&r.roles.owner)))
        .execute(&mut inspect)
        .await
        .unwrap();
    assert_eq!(
        sqlx::query_scalar::<_, String>(
            "SELECT to_jsonb(u)::text FROM core_identity.users u ORDER BY id"
        )
        .fetch_all(&mut inspect)
        .await
        .unwrap(),
        before_users
    );
    assert_eq!(
        sequence_states(&mut inspect, &sequences).await,
        before_sequences
    );
    assert_eq!(exercised_authority(&mut inspect).await, before_authority);
    assert_eq!(
        sqlx::query_scalar::<_, String>(
            "SELECT to_jsonb(j)::text FROM core_billing.ledger_journals j ORDER BY id"
        )
        .fetch_all(&mut inspect)
        .await
        .unwrap(),
        before_finance
    );
    assert_eq!(
        sqlx::query_scalar::<_, i64>("SELECT count(*) FROM core_meta.component_contract")
            .fetch_one(&mut inspect)
            .await
            .unwrap(),
        lmm_core::core_schema::component_count() as i64
    );
    // The successful public replay above already verified the receipt privileges.
    sqlx::query(&format!(
        "GRANT SELECT ON core_meta.completion_upgrades TO {}",
        quote(&r.roles.installer)
    ))
    .execute(&mut inspect)
    .await
    .unwrap();
    // The offline API requires an exclusive connection. Validate the same
    // original receipt through that API, not by exporting private privilege helpers.
    inspect.close().await.unwrap();
    assert!(matches!(
        upgrade_database(url.as_str(), r.clone()).await,
        Err(UpgradeError::Rejected(
            "COMPLETION_RECEIPT_PRIVILEGES_INVALID"
        ))
    ));
    let mut inspect = PgConnection::connect(url.as_str()).await.unwrap();
    sqlx::query(&format!("SET ROLE {}", quote(&r.roles.owner)))
        .execute(&mut inspect)
        .await
        .unwrap();
    sqlx::query(&format!(
        "REVOKE SELECT ON core_meta.completion_upgrades FROM {}",
        quote(&r.roles.installer)
    ))
    .execute(&mut inspect)
    .await
    .unwrap();
    inspect.close().await.unwrap();
    let restored = upgrade_database(url.as_str(), r.clone()).await.unwrap();
    assert_eq!(restored.status, "already_committed");
    assert_eq!(
        restored.successor_catalog_sha256,
        result.successor_catalog_sha256
    );
    // Only these random, synthetic roles and this SQLx-owned database are touched.
    let mut admin_url = reqwest::Url::parse(&std::env::var("DATABASE_URL").unwrap()).unwrap();
    admin_url.set_path(&format!("/{}", r.database.name));
    let mut admin = PgConnection::connect(admin_url.as_str()).await.unwrap();
    sqlx::raw_sql(&format!("DROP OWNED BY {installer},{runtime},{extensions},{owner} CASCADE; ALTER DATABASE {database} OWNER TO {original}; DROP ROLE {installer},{runtime},{extensions},{owner};",
        installer=quote(&r.roles.installer),runtime=quote(&r.roles.runtime),extensions=quote(&r.roles.extensions),owner=quote(&r.roles.owner),database=quote(&r.database.name),original=quote(&original)))
        .execute(&mut admin).await.unwrap();
    admin.close().await.unwrap();
}

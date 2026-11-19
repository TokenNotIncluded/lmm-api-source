//! The current installer must reject the historical 29 -> 31 procedure unchanged.
//! Success for that procedure requires its separately preserved 31-component binary.
use lmm_core::core_schema::{
    UpgradeDatabase, UpgradeError, UpgradeRequest, UpgradeRoles, schema_manifest, upgrade_database,
};
use sha2::{Digest, Sha256};
use sqlx::{Connection, PgConnection, Row, postgres::PgPoolOptions};

const PROCEDURE: &str = "core-request-operations-v1";
const PRE_MANIFEST: &str = "48578c69632d80d7d4e41a5fb36c67f365875e8bb67e0695f62e1d688177f530";
const PRE_CATALOG: &str = "cb9349ca60bafbd49204dc237a88eec5ca10ac4654e654e7c816ea59673bc21e";
const PREDECESSOR: &str = include_str!("../src/core_schema/upgrade/payment_gateways_target.json");

fn quote(value: &str) -> String {
    format!("\"{}\"", value.replace('"', "\"\""))
}
fn manifest_sha256() -> String {
    let mut bytes = serde_json::to_vec(&schema_manifest()).unwrap();
    bytes.push(b'\n');
    hex_codec::encode(Sha256::digest(bytes))
}
async fn definitions(connection: &mut PgConnection) -> Result<Vec<String>, sqlx::Error> {
    sqlx::query_scalar(sqlx::AssertSqlSafe(schema_manifest().catalog.query_utf8))
        .fetch_all(connection)
        .await
}
fn predecessor_definitions() -> Result<Vec<String>, serde_json::Error> {
    let frozen: serde_json::Value = serde_json::from_str(PREDECESSOR)?;
    let definitions: Vec<String> = serde_json::from_value(frozen["definitions"].clone())?;
    assert_eq!(frozen["manifest_sha256"], PRE_MANIFEST);
    assert_eq!(frozen["catalog_sha256"], PRE_CATALOG);
    let mut hash = Sha256::new();
    for definition in &definitions {
        hash.update((definition.len() as u64).to_be_bytes());
        hash.update(definition.as_bytes());
    }
    assert_eq!(hex_codec::encode(hash.finalize()), PRE_CATALOG);
    Ok(definitions)
}

async fn protected_state(connection: &mut PgConnection) -> Vec<(String, String)> {
    let tables: Vec<(String, String)> = sqlx::query_as(
        "SELECT n.nspname,c.relname FROM pg_class c JOIN pg_namespace n ON n.oid=c.relnamespace WHERE c.relkind='r' AND n.nspname LIKE 'core_%' ORDER BY 1,2",
    )
    .fetch_all(&mut *connection)
    .await
    .unwrap();
    let mut state = Vec::new();
    for (schema, table) in tables {
        let digest: String = sqlx::query_scalar(sqlx::AssertSqlSafe(format!(
            "SELECT md5(coalesce(string_agg(row_to_json(t)::text,E'\\n' ORDER BY row_to_json(t)::text),'')) FROM {}.{} t",
            quote(&schema),
            quote(&table),
        )))
        .fetch_one(&mut *connection)
        .await
        .unwrap();
        state.push((format!("{schema}.{table}"), digest));
    }
    state
}

fn request() -> UpgradeRequest {
    UpgradeRequest {
        version: 1,
        procedure: PROCEDURE.into(),
        upgrade_id: "synthetic-operations-upgrade".into(),
        database: UpgradeDatabase {
            name: "synthetic_operations".into(),
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
async fn operations_upgrade_rejects_current_installer_without_mutation(pool: sqlx::PgPool) {
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
    sqlx::raw_sql(sqlx::AssertSqlSafe(format!("CREATE ROLE {owner} NOLOGIN; CREATE ROLE {installer} LOGIN NOINHERIT PASSWORD '{password}'; CREATE ROLE {runtime} NOLOGIN; CREATE ROLE {extensions} NOLOGIN; GRANT {owner} TO {installer} WITH INHERIT FALSE, SET TRUE; ALTER DATABASE {database} OWNER TO {owner}; REVOKE CONNECT,CREATE,TEMP ON DATABASE {database} FROM PUBLIC; GRANT CONNECT ON DATABASE {database} TO {installer}; REVOKE EXECUTE ON FUNCTION pg_catalog.pg_control_system() FROM PUBLIC; GRANT EXECUTE ON FUNCTION pg_catalog.pg_control_system() TO {owner};",
        owner=quote(&r.roles.owner),installer=quote(&r.roles.installer),runtime=quote(&r.roles.runtime),extensions=quote(&r.roles.extensions),database=quote(&r.database.name))))
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
    lmm_core::core_schema::init(&installer).await.unwrap();
    let identity = lmm_core::identity::IdentityStore::from_pool(installer.clone());
    identity.bootstrap_user(7001, 6).await.unwrap();
    let before_users: i64 = sqlx::query_scalar("SELECT count(*) FROM core_identity.users")
        .fetch_one(&installer)
        .await
        .unwrap();
    let mut conn = installer.acquire().await.unwrap();
    let before_catalog = definitions(&mut conn).await.unwrap();
    let before_data = protected_state(&mut conn).await;
    assert_ne!(before_catalog, predecessor_definitions().unwrap());
    let current_components = schema_manifest().components.len() as i64;
    assert_ne!(current_components, 31);
    drop(conn);
    let mut url = reqwest::Url::parse(&std::env::var("DATABASE_URL").unwrap()).unwrap();
    url.set_path(&format!("/{}", r.database.name));
    url.set_username(&r.roles.installer).unwrap();
    url.set_password(Some(&password)).unwrap();
    installer.close().await;
    pool.close().await;

    // The current installer is not the frozen 31-component executor. Deleting
    // two schemas cannot reconstruct its 29-component predecessor or authorize
    // a historical upgrade. Fresh requests and exact retries must fail closed.
    for upgrade_id in [
        r.upgrade_id.clone(),
        r.upgrade_id.clone(),
        format!("{}2", r.upgrade_id),
    ] {
        let mut attempt = r.clone();
        attempt.upgrade_id = upgrade_id;
        assert!(matches!(
            upgrade_database(url.as_str(), attempt).await,
            Err(UpgradeError::Rejected(
                "REQUEST_OPERATIONS_COMPILED_COMPONENTS_CHANGED"
            ))
        ));
    }

    let mut inspect = PgConnection::connect(url.as_str()).await.unwrap();
    sqlx::query(sqlx::AssertSqlSafe(format!(
        "SET ROLE {}",
        quote(&r.roles.owner)
    )))
    .execute(&mut inspect)
    .await
    .unwrap();
    assert_eq!(definitions(&mut inspect).await.unwrap(), before_catalog);
    assert_eq!(protected_state(&mut inspect).await, before_data);
    assert_eq!(
        sqlx::query_scalar::<_, i64>("SELECT count(*) FROM core_identity.users")
            .fetch_one(&mut inspect)
            .await
            .unwrap(),
        before_users
    );
    assert_eq!(
        sqlx::query_scalar::<_, i64>("SELECT count(*) FROM core_meta.component_contract")
            .fetch_one(&mut inspect)
            .await
            .unwrap(),
        current_components
    );
    assert_eq!(
        sqlx::query_scalar::<_, i64>("SELECT count(*) FROM core_operations.upgrade_receipts")
            .fetch_one(&mut inspect)
            .await
            .unwrap(),
        0
    );
    inspect.close().await.unwrap();
    // Only these random, synthetic roles and this SQLx-owned database are touched.
    let mut admin_url = reqwest::Url::parse(&std::env::var("DATABASE_URL").unwrap()).unwrap();
    admin_url.set_path(&format!("/{}", r.database.name));
    let mut admin = PgConnection::connect(admin_url.as_str()).await.unwrap();
    sqlx::raw_sql(sqlx::AssertSqlSafe(format!("DROP OWNED BY {installer},{runtime},{extensions},{owner} CASCADE; ALTER DATABASE {database} OWNER TO {original}; DROP ROLE {installer},{runtime},{extensions},{owner};",
        installer=quote(&r.roles.installer),runtime=quote(&r.roles.runtime),extensions=quote(&r.roles.extensions),owner=quote(&r.roles.owner),database=quote(&r.database.name),original=quote(&original))))
        .execute(&mut admin).await.unwrap();
    admin.close().await.unwrap();
}

#[path = "../src/hex_codec.rs"]
mod hex_codec;

//! Real offline 29 -> 31 upgrade through the public Core API. Kept in its own
//! integration target so this PG fixture does not require the full lib test binary.
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
    format!("{:x}", Sha256::digest(bytes))
}
async fn definitions(connection: &mut PgConnection) -> Result<Vec<String>, sqlx::Error> {
    sqlx::query_scalar(schema_manifest().catalog.query_utf8)
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
    assert_eq!(format!("{:x}", hash.finalize()), PRE_CATALOG);
    Ok(definitions)
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
async fn operations_upgrade_preserves_real_predecessor_and_resolves_exact_replay(
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
    lmm_core::core_schema::init(&installer).await.unwrap();
    // Reconstruct the exact frozen predecessor, then verify every catalog byte.
    sqlx::raw_sql("DROP SCHEMA core_operations CASCADE; DROP SCHEMA core_request_limits CASCADE; DELETE FROM core_meta.component_contract WHERE name IN ('operations','request-limits')")
        .execute(&installer).await.unwrap();
    sqlx::query("UPDATE core_meta.runtime_contract SET catalog_fingerprint=decode($1,'hex')")
        .bind(PRE_CATALOG)
        .execute(&installer)
        .await
        .unwrap();
    let mut conn = installer.acquire().await.unwrap();
    assert_eq!(
        definitions(&mut conn).await.unwrap(),
        predecessor_definitions().unwrap()
    );
    drop(conn);
    let identity = lmm_core::identity::IdentityStore::from_pool(installer.clone());
    identity.bootstrap_user(7001, 6).await.unwrap();
    let before_users: i64 = sqlx::query_scalar("SELECT count(*) FROM core_identity.users")
        .fetch_one(&installer)
        .await
        .unwrap();
    let mut url = reqwest::Url::parse(&std::env::var("DATABASE_URL").unwrap()).unwrap();
    url.set_path(&format!("/{}", r.database.name));
    url.set_username(&r.roles.installer).unwrap();
    url.set_password(Some(&password)).unwrap();
    installer.close().await;
    pool.close().await;
    let result = upgrade_database(url.as_str(), r.clone()).await.unwrap();
    assert_eq!(result.status, "committed");
    assert_eq!(result.components, 31);
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
        Err(UpgradeError::Rejected(
            "REQUEST_OPERATIONS_ORIGINAL_RECEIPT_ABSENT"
        ))
    ));
    let mut inspect = PgConnection::connect(url.as_str()).await.unwrap();
    sqlx::query(&format!("SET ROLE {}", quote(&r.roles.owner)))
        .execute(&mut inspect)
        .await
        .unwrap();
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
        31
    );
    // The successful public replay above already verified the receipt privileges.
    sqlx::query(&format!(
        "GRANT SELECT ON core_operations.upgrade_receipts TO {}",
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
            "REQUEST_OPERATIONS_PRIVILEGES_INVALID"
        ))
    ));
    let mut inspect = PgConnection::connect(url.as_str()).await.unwrap();
    sqlx::query(&format!("SET ROLE {}", quote(&r.roles.owner)))
        .execute(&mut inspect)
        .await
        .unwrap();
    sqlx::query(&format!(
        "REVOKE SELECT ON core_operations.upgrade_receipts FROM {}",
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

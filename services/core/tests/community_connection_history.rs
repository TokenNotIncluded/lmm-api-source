//! Synthetic retained-snapshot contract tests. No real source artifact is used.
use lmm_core::{
    community_read::{CommunityRead, UsersInfoRequest},
    core_schema,
    identity::IdentityStore,
    runtime_database::{self, RequiredCapabilities},
};
use serde_json::json;
use sqlx::{PgPool, postgres::PgPoolOptions};
use std::{
    path::PathBuf,
    process::Command,
    time::{SystemTime, UNIX_EPOCH},
};

fn evidence() -> PathBuf {
    let path = PathBuf::from(
        std::env::var("CONNECTION55_EVIDENCE").expect("private evidence directory required"),
    );
    assert!(
        path.starts_with(
            PathBuf::from(std::env::var("HOME").unwrap())
                .join(".local/state/lmm-api/connection-filter")
        )
    );
    path
}

#[sqlx::test(migrations = false)]
#[ignore = "explicit private new-PG catalog capture only"]
async fn capture_connection55(pool: PgPool) {
    use sha2::{Digest, Sha256};
    use std::io::Write;
    core_schema::init(&pool).await.unwrap();
    let catalog: String = sqlx::query_scalar(
        "SELECT encode(catalog_fingerprint,'hex') FROM core_meta.runtime_contract WHERE singleton",
    )
    .fetch_one(&pool)
    .await
    .unwrap();
    let mut raw = serde_json::to_vec(&core_schema::schema_manifest()).unwrap();
    raw.push(b'\n');
    let record =
        json!({"manifest_sha256":format!("{:x}",Sha256::digest(raw)),"catalog_sha256":catalog});
    let mut file = std::fs::OpenOptions::new()
        .create_new(true)
        .write(true)
        .open(evidence().join("fresh-target-capture.json"))
        .unwrap();
    writeln!(file, "{}", serde_json::to_string_pretty(&record).unwrap()).unwrap();
    file.sync_all().unwrap();
}

async fn protected_state(pool: &PgPool) -> Vec<(String, String)> {
    let tables:Vec<(String,String)>=sqlx::query_as("SELECT n.nspname,c.relname FROM pg_class c JOIN pg_namespace n ON n.oid=c.relnamespace WHERE c.relkind='r' AND n.nspname LIKE 'core_%' AND NOT(n.nspname='core_history' AND c.relname='connection_snapshot') ORDER BY 1,2").fetch_all(pool).await.unwrap();
    let mut state = vec![];
    for (schema, table) in tables {
        let digest:String=sqlx::query_scalar(&format!("SELECT md5(coalesce(string_agg(row_to_json(t)::text,E'\\n' ORDER BY row_to_json(t)::text),'')) FROM {schema}.{table} t")).fetch_one(pool).await.unwrap();
        state.push((format!("{schema}.{table}"), digest));
    }
    state
}

#[sqlx::test(migrations = false)]
async fn retained_snapshot_import_and_runtime_funnel(pool: PgPool) {
    let out = evidence();
    let db = pool.connect_options().get_database().unwrap().to_owned();
    assert!(db.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'_'));
    sqlx::raw_sql(&format!("ALTER DATABASE \"{db}\" OWNER TO lmm_core_owner; ALTER SCHEMA public OWNER TO lmm_core_owner; ALTER DATABASE \"{db}\" SET lmm.migration_instance_nonce='{}'","d".repeat(64))).execute(&pool).await.unwrap();
    let owner = PgPoolOptions::new()
        .max_connections(3)
        .after_connect(|c, _| {
            Box::pin(async move {
                sqlx::query("SET ROLE lmm_core_owner").execute(c).await?;
                Ok(())
            })
        })
        .connect_with(pool.connect_options().as_ref().clone())
        .await
        .unwrap();
    core_schema::init(&owner).await.unwrap();
    let identity = IdentityStore::from_pool(owner.clone());
    let registered = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_secs() as i64
        - 2 * 86400;
    let mut times = serde_json::Map::new();
    for uid in 11..=16 {
        let session = identity.bootstrap_user(uid, 0).await.unwrap();
        let account = identity
            .authorize(&session.secret)
            .await
            .unwrap()
            .owner_account_id;
        times.insert(account.to_string(), json!(registered));
        sqlx::query("UPDATE core_identity.accounts SET created_at=to_timestamp($1) WHERE id=$2")
            .bind(registered)
            .bind(account)
            .execute(&owner)
            .await
            .unwrap();
        let request = json!({"migration_id":"synthetic-connection-opening","source_entity_key":format!("legacy.users/{}",uid+90),
    "source_archive_sha256":"a".repeat(64),"mapping_sha256":"b".repeat(64),"source_row_sha256":"c".repeat(64),
    "observed_at":registered+3600,"account_id":account,"available_units":0,"reserved_units":0,"affiliate_units":0});
        let mut tx = owner.begin().await.unwrap();
        sqlx::query("SET TRANSACTION ISOLATION LEVEL SERIALIZABLE")
            .execute(&mut *tx)
            .await
            .unwrap();
        sqlx::query("SET LOCAL synchronous_commit=on")
            .execute(&mut *tx)
            .await
            .unwrap();
        sqlx::query("SELECT core_billing.import_opening_balance($1)")
            .bind(request)
            .execute(&mut *tx)
            .await
            .unwrap();
        tx.commit().await.unwrap();
    }
    sqlx::query("INSERT INTO core_migration.import_runs(migration_id,source_archive_sha256,mapping_sha256,manifest) VALUES('synthetic-connection-identity',repeat('a',64),repeat('b',64),$1)").bind(json!({"scope":"legacy-cutover-identity.v1","source_archive_sha256":"a".repeat(64),"registration_times":times})).execute(&owner).await.unwrap();
    for (id, uid, offset) in [
        (1001_i64, 11_i64, 100_i64),
        (1002, 12, 200),
        (1003, 14, 100),
        (1004, 14, 50),
        (1005, 15, 0),
        (1006, 15, 300),
    ] {
        let created = if id == 1005 { 0 } else { registered + offset };
        sqlx::query("INSERT INTO core_identity.credentials(id,digest,kind,user_id,user_version,owner_account_id,expires_at,revoked_at,created_at,session_started_at) OVERRIDING SYSTEM VALUE SELECT $1,decode(lpad(to_hex($1::bigint),64,'0'),'hex'),'api_key',id,auth_version,personal_account_id,now()-interval '1 day',now()-interval '1 day',to_timestamp($2),to_timestamp($2) FROM core_identity.users WHERE id=$3").bind(id).bind(created).bind(uid).execute(&owner).await.unwrap();
    }
    sqlx::query("SELECT setval(pg_get_serial_sequence('core_identity.credentials','id'),1006)")
        .execute(&owner)
        .await
        .unwrap();
    let before = protected_state(&owner).await;
    let source = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..");
    let script = source.join("services/core/tests/community_connection_history_fixture.py");
    let child_db = db.clone();
    let child_out = out.clone();
    let output = tokio::task::spawn_blocking(move || {
        Command::new("python3")
            .arg(script)
            .arg(child_db)
            .arg(registered.to_string())
            .env("CONNECTION55_EVIDENCE", child_out)
            .output()
            .unwrap()
    })
    .await
    .unwrap();
    std::fs::write(out.join("import-fixture.stdout.log"), &output.stdout).unwrap();
    std::fs::write(out.join("import-fixture.stderr.log"), &output.stderr).unwrap();
    assert!(
        output.status.success(),
        "synthetic importer: {}{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    assert_eq!(
        before,
        protected_state(&owner).await,
        "historical import mutated another Core table"
    );
    sqlx::query("CREATE ROLE connection_history_runtime LOGIN PASSWORD 'synthetic-history-runtime-only' NOSUPERUSER NOCREATEDB NOCREATEROLE NOREPLICATION NOBYPASSRLS").execute(&pool).await.unwrap();
    let grant = Command::new("psql")
        .args([
            "-X",
            "-q",
            "-v",
            "ON_ERROR_STOP=1",
            "-v",
            "runtime_role=connection_history_runtime",
            "-v",
            "enable_community=true",
            "-f",
        ])
        .arg(source.join("deployment/postgres/grant-core-runtime.sql"))
        .env("PGDATABASE", &db)
        .output()
        .unwrap();
    std::fs::write(out.join("runtime-grant.stderr.log"), &grant.stderr).unwrap();
    assert!(
        grant.status.success(),
        "runtime grant: {}",
        String::from_utf8_lossy(&grant.stderr)
    );
    let opts = pool
        .connect_options()
        .as_ref()
        .clone()
        .username("connection_history_runtime")
        .password("synthetic-history-runtime-only");
    let runtime = PgPoolOptions::new()
        .max_connections(3)
        .connect_with(opts)
        .await
        .unwrap();
    let role: String = sqlx::query_scalar("SELECT current_user")
        .fetch_one(&runtime)
        .await
        .unwrap();
    assert_eq!(role, "connection_history_runtime");
    core_schema::check(&runtime).await.unwrap();
    runtime_database::check(
        &runtime,
        RequiredCapabilities {
            community: true,
            ..Default::default()
        },
    )
    .await
    .unwrap();
    assert!(
        sqlx::query("DELETE FROM core_history.connection_snapshot")
            .execute(&runtime)
            .await
            .is_err()
    );
    for operation in [
        "INSERT INTO core_history.connection_snapshot DEFAULT VALUES",
        "UPDATE core_history.connection_snapshot SET target_nonce=repeat('e',64)",
        "DELETE FROM core_history.connection_snapshot",
        "TRUNCATE core_history.connection_snapshot",
        "SELECT document_utf8 FROM core_history.connection_snapshot",
    ] {
        let error = sqlx::query(operation).execute(&runtime).await.unwrap_err();
        assert_eq!(
            error.as_database_error().and_then(|e| e.code()).as_deref(),
            Some("42501")
        );
    }
    for column in [
        "document",
        "mapping",
        "native_credential_floor",
        "snapshot_observed_at",
        "target_database",
        "target_database_oid",
        "target_nonce",
    ] {
        sqlx::query(&format!("REVOKE SELECT({column}) ON core_history.connection_snapshot FROM connection_history_runtime")).execute(&owner).await.unwrap();
        assert_eq!(
            runtime_database::check(
                &runtime,
                RequiredCapabilities {
                    community: true,
                    ..Default::default()
                }
            )
            .await,
            Err(runtime_database::RuntimeDatabaseError::MissingPrivileges),
            "missing SELECT({column}) must block community startup"
        );
        let missing = CommunityRead::new(runtime.clone()).await.unwrap();
        assert!(
            missing
                .connection_milestones(&UsersInfoRequest { user_ids: vec![11] })
                .await
                .is_err()
        );
        sqlx::query(&format!("GRANT SELECT({column}) ON core_history.connection_snapshot TO connection_history_runtime")).execute(&owner).await.unwrap();
    }
    // Reset optional grants with the shipped script, preserving baseline history.
    let revoke = Command::new("psql")
        .args([
            "-X",
            "-q",
            "-v",
            "ON_ERROR_STOP=1",
            "-v",
            "runtime_role=connection_history_runtime",
            "-v",
            "confirm_revoke=true",
            "-f",
        ])
        .arg(source.join("deployment/postgres/revoke-core-runtime.sql"))
        .env("PGDATABASE", &db)
        .output()
        .unwrap();
    std::fs::write(out.join("runtime-revoke.stderr.log"), &revoke.stderr).unwrap();
    assert!(
        revoke.status.success(),
        "runtime revoke: {}",
        String::from_utf8_lossy(&revoke.stderr)
    );
    let snapshot_readable: bool = sqlx::query_scalar(
        "SELECT has_any_column_privilege('core_history.connection_snapshot','SELECT')",
    )
    .fetch_one(&runtime)
    .await
    .unwrap();
    assert!(
        !snapshot_readable,
        "optional reset left snapshot column grants"
    );
    let baseline_readable: bool = sqlx::query_scalar(
        "SELECT has_schema_privilege('core_history','USAGE') AND has_table_privilege('core_history.sources','SELECT')",
    ).fetch_one(&runtime).await.unwrap();
    assert!(
        baseline_readable,
        "optional reset removed baseline history reads"
    );
    assert_eq!(
        runtime_database::check(
            &runtime,
            RequiredCapabilities {
                community: true,
                ..Default::default()
            }
        )
        .await,
        Err(runtime_database::RuntimeDatabaseError::MissingPrivileges)
    );
    let regrant = Command::new("psql")
        .args([
            "-X",
            "-q",
            "-v",
            "ON_ERROR_STOP=1",
            "-v",
            "runtime_role=connection_history_runtime",
            "-v",
            "enable_community=true",
            "-f",
        ])
        .arg(source.join("deployment/postgres/grant-core-runtime.sql"))
        .env("PGDATABASE", &db)
        .output()
        .unwrap();
    std::fs::write(out.join("runtime-regrant.stderr.log"), &regrant.stderr).unwrap();
    assert!(
        regrant.status.success(),
        "runtime regrant: {}",
        String::from_utf8_lossy(&regrant.stderr)
    );
    runtime_database::check(
        &runtime,
        RequiredCapabilities {
            community: true,
            ..Default::default()
        },
    )
    .await
    .unwrap();
    let read = CommunityRead::new(runtime.clone()).await.unwrap();
    let facts = read
        .connection_milestones(&UsersInfoRequest {
            user_ids: (11..=16).collect(),
        })
        .await
        .unwrap();
    std::fs::write(
        out.join("runtime-facts.json"),
        serde_json::to_vec_pretty(&facts).unwrap(),
    )
    .unwrap();
    assert_eq!(facts.users.len(), 6);
    assert_eq!(facts.users[0].api_key_created_at, Some(registered + 100));
    assert_eq!(facts.users[1].oauth_authorized_at, Some(registered + 150));
    assert_eq!(
        facts.users[1].api_key_created_at, None,
        "managed imported token classified as ordinary API key"
    );
    assert_eq!(facts.users[3].oauth_authorized_at, Some(registered + 50));
    assert!(
        facts.users[..4]
            .iter()
            .all(|f| f.history_complete && f.history_source == "retained_snapshot_and_native")
    );
    assert!(!facts.users[4].history_complete && !facts.users[5].history_complete);
    assert_eq!(
        facts.users[4].history_source,
        "retained_snapshot_and_native"
    );
    assert_eq!(facts.users[5].history_source, "incomplete_legacy");
    assert_eq!(
        facts.users[0].application_submitted_at,
        Some(registered + 10)
    );
    assert_eq!(facts.users[0].access_approved_at, Some(registered + 20));
    assert!(
        facts.users[1].approval_history_complete && facts.users[1].access_approved_at.is_none()
    );
    assert!(
        !facts.users[4].application_history_complete && !facts.users[4].approval_history_complete
    );
    assert!(
        read.connection_milestones(&UsersInfoRequest {
            user_ids: vec![11; 101]
        })
        .await
        .is_err()
    );
    let isolated = read
        .connection_milestones(&UsersInfoRequest {
            user_ids: vec![12, 999],
        })
        .await
        .unwrap();
    assert_eq!(isolated.users.len(), 1);
    assert_eq!(isolated.users[0].user_id, 12);
    let socket = out.join("core.sock");
    let listener = tokio::net::UnixListener::bind(&socket).unwrap();
    let router = lmm_core::community_read::http::router(
        read.clone(),
        b"synthetic-community-purpose-token-20261104",
    )
    .unwrap();
    let server = tokio::spawn(async move {
        axum::serve(listener, router).await.unwrap();
    });
    let child_source = source.clone();
    let output = tokio::task::spawn_blocking(move || {
        Command::new("go")
            .args([
                "test",
                "./internal/modules/acquisition",
                "-run",
                "^TestConnectionHistoryFunnelLive$",
                "-v",
                "-count=1",
            ])
            .current_dir(child_source.join("services/extensions"))
            .env("LMM_TEST_CONNECTION_HISTORY_SOCKET", socket)
            .output()
            .unwrap()
    })
    .await
    .unwrap();
    server.abort();
    let _ = server.await;
    std::fs::write(out.join("go-history.stdout.log"), &output.stdout).unwrap();
    std::fs::write(out.join("go-history.stderr.log"), &output.stderr).unwrap();
    assert!(
        output.status.success(),
        "Go historical funnel: {}{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(
        String::from_utf8_lossy(&output.stdout)
            .contains("--- PASS: TestConnectionHistoryFunnelLive")
    );
    assert_eq!(
        before,
        protected_state(&owner).await,
        "historical read mutated another Core table"
    );
    // A genuine later native key is retained in addition to the sealed source.
    sqlx::query("INSERT INTO core_identity.credentials(digest,kind,user_id,user_version,owner_account_id,expires_at) SELECT decode(repeat('ef',32),'hex'),'api_key',id,auth_version,personal_account_id,now()+interval '1 day' FROM core_identity.users WHERE id=13").execute(&owner).await.unwrap();
    let later = read
        .connection_milestones(&UsersInfoRequest { user_ids: vec![13] })
        .await
        .unwrap();
    assert!(
        later.users[0]
            .api_key_created_at
            .is_some_and(|at| at > registered + 3600)
    );
    assert!(later.users[0].history_complete);
    let immutable = sqlx::query(
        "UPDATE core_history.connection_snapshot SET source_archive_sha256=repeat('f',64)",
    )
    .execute(&owner)
    .await;
    assert!(immutable.is_err());

    runtime.close().await;
    owner.close().await;
}

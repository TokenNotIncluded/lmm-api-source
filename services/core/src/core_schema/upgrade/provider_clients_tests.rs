//! Actual PG fresh/runtime and pinned owner transition; no model traffic here.
use super::*;
use sqlx::PgPool;

// Preservation uses a server cursor; standalone inspections need a transaction.
async fn snapshot_data(
    c: &mut PgConnection,
    tables: &[String],
    sequences: &[String],
) -> Result<std::collections::BTreeMap<String, Vec<u8>>, UpgradeError> {
    let mut tx = c.begin().await?;
    let data =
        super::super::request_operations::preservation::data(&mut tx, tables, sequences).await?;
    tx.rollback().await?;
    Ok(data)
}

async fn prefix77(pool: &PgPool) {
    use crate::core_schema as root;
    let mut tx = pool.begin().await.unwrap();
    for (_, ddl) in &COMPONENTS[..77] {
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
    for (name, ddl) in &COMPONENTS[..77] {
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
async fn provider78_singleton_exact_one_field_and_no_repair(pool: PgPool) {
    prefix77(&pool).await;
    let mut c = pool.acquire().await.unwrap();
    let tables = vec!["core_oauth.contract".to_owned()];
    let mut expected = snapshot_data(&mut c, &tables, &[]).await.unwrap();
    let before: serde_json::Value =
        sqlx::query_scalar("SELECT to_jsonb(c) FROM core_oauth.contract c")
            .fetch_one(&mut *c)
            .await
            .unwrap();
    let mut tx = c.begin().await.unwrap();
    sqlx::raw_sql(COMPONENTS[77].1)
        .execute(&mut *tx)
        .await
        .unwrap();
    advance_oauth_contract(&mut tx, &mut expected)
        .await
        .unwrap();
    let actual = super::super::request_operations::preservation::data(&mut tx, &tables, &[])
        .await
        .unwrap();
    assert_eq!(
        actual, expected,
        "the singleton remains included in preservation"
    );
    let after: serde_json::Value =
        sqlx::query_scalar("SELECT to_jsonb(c) FROM core_oauth.contract c")
            .fetch_one(&mut *tx)
            .await
            .unwrap();
    let mut expected_row = before.clone();
    expected_row["catalog_fingerprint"] =
        serde_json::json!(format!("\\x{}", captured().unwrap().oauth_catalog_sha256));
    assert_eq!(after, expected_row);
    tx.rollback().await.unwrap();
    let mut tx = c.begin().await.unwrap();
    sqlx::raw_sql(COMPONENTS[77].1)
        .execute(&mut *tx)
        .await
        .unwrap();
    sqlx::query("UPDATE core_oauth.contract SET ddl_fingerprint=decode(repeat('00',32),'hex')")
        .execute(&mut *tx)
        .await
        .unwrap();
    let broken: serde_json::Value =
        sqlx::query_scalar("SELECT to_jsonb(c) FROM core_oauth.contract c")
            .fetch_one(&mut *tx)
            .await
            .unwrap();
    assert!(
        advance_oauth_contract(&mut tx, &mut expected)
            .await
            .is_err()
    );
    let retained: serde_json::Value =
        sqlx::query_scalar("SELECT to_jsonb(c) FROM core_oauth.contract c")
            .fetch_one(&mut *tx)
            .await
            .unwrap();
    assert_eq!(
        broken, retained,
        "rejecting an unknown receipt must not repair it"
    );
    tx.rollback().await.unwrap();
}

fn permission_key(fact: &str) -> Option<String> {
    let value: serde_json::Value = serde_json::from_str(fact).unwrap();
    match value[0].as_str() {
        Some("schema") => Some(format!("schema:{}", value[1])),
        Some("function") => Some(format!("function:{}:{}", value[1], value[2])),
        _ => None,
    }
}

#[sqlx::test(migrations = false)]
async fn provider78_owner_upgrade_preserves_all_other_data_acl_and_commit_recovery(pool: PgPool) {
    use super::super::payment_discounts::tests::engine_offline_boundary;
    use crate::core_schema::{UpgradeDatabase, UpgradeRoles, upgrade_database};
    use sqlx::postgres::PgPoolOptions;
    authorize_target().unwrap();
    let facts=sqlx::query("SELECT current_database()::text AS name,d.oid::bigint AS oid,current_setting('server_version_num')::bigint AS version,(SELECT system_identifier::text FROM pg_control_system()) AS system,session_user::text AS original FROM pg_database d WHERE datname=current_database()")
        .fetch_one(&pool).await.unwrap();
    let suffix = format!("{:016x}", rand::random::<u64>());
    let request = UpgradeRequest {
        version: 1,
        procedure: PROCEDURE.into(),
        upgrade_id: format!("provider78-owner-{suffix}"),
        database: UpgradeDatabase {
            name: facts.get("name"),
            oid: u32::try_from(facts.get::<i64, _>("oid")).unwrap(),
            system_identifier: facts.get("system"),
            server_version_num: u32::try_from(facts.get::<i64, _>("version")).unwrap(),
        },
        roles: UpgradeRoles {
            installer: format!("provider78_i_{suffix}"),
            owner: format!("provider78_o_{suffix}"),
            runtime: format!("provider78_r_{suffix}"),
            extensions: format!("provider78_e_{suffix}"),
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
    prefix77(&installer).await;
    let identity = IdentityStore::from_pool(installer.clone());
    identity.bootstrap_user(78001, 0).await.unwrap();
    let root = identity.bootstrap_user(78002, 6).await.unwrap();
    crate::identity::admin_users::AdminUsersStore::new(identity)
        .adjust_user_balance(
            &root.secret,
            78001,
            &crate::identity::admin_users::AdjustmentRequest {
                delta_units: "1234568".into(),
                operation_key: "provider78-real-prior-ledger-operation".into(),
                reason: "isolated predecessor preservation".into(),
            },
        )
        .await
        .unwrap();
    // Real prior grant/token rows: component78 must not rewrite existing clients.
    sqlx::query("INSERT INTO core_oauth.families(id,user_id,owner_account_id,user_version,issuer,resource,client_id,scopes,expires_at) SELECT $1,id,personal_account_id,auth_version,'https://api.example.test','https://api.example.test/api/oauth2','lmm-pi',ARRAY['catalog:read'],clock_timestamp()+interval '1 day' FROM core_identity.users WHERE id=78001")
        .bind(format!("lmm_gr_{}","a".repeat(64))).execute(&installer).await.unwrap();
    sqlx::query("INSERT INTO core_oauth.refresh_tokens(digest,family_id,consent_version,scopes,expires_at) SELECT decode(repeat('01',32),'hex'),id,consent_version,scopes,expires_at FROM core_oauth.families")
        .execute(&installer).await.unwrap();
    sqlx::raw_sql(sqlx::AssertSqlSafe(format!("GRANT USAGE ON SCHEMA core_identity,core_meta,core_oauth TO {runtime}; GRANT SELECT ON core_identity.users TO {runtime}; GRANT UPDATE(revoked_at) ON core_identity.credentials TO {runtime}; GRANT USAGE ON SEQUENCE core_identity.credentials_id_seq TO {runtime}; GRANT EXECUTE ON FUNCTION core_oauth.client_scope_set(text,text[],boolean) TO {runtime};",runtime=quote(&request.roles.runtime))))
        .execute(&installer).await.unwrap();
    let mut c = installer.acquire().await.unwrap();
    sqlx::query("SELECT setval('core_identity.credentials_id_seq',9000,false)")
        .execute(&mut *c)
        .await
        .unwrap();
    let tables:Vec<String>=sqlx::query_scalar("SELECT format('%I.%I',n.nspname,c.relname) FROM pg_class c JOIN pg_namespace n ON n.oid=c.relnamespace WHERE n.nspname NOT IN ('pg_catalog','information_schema') AND left(n.nspname,3)<>'pg_' AND c.relkind IN ('r','p') ORDER BY n.nspname,c.relname").fetch_all(&mut *c).await.unwrap();
    let sequences:Vec<String>=sqlx::query_scalar("SELECT format('%I.%I',n.nspname,c.relname) FROM pg_class c JOIN pg_namespace n ON n.oid=c.relnamespace WHERE n.nspname NOT IN ('pg_catalog','information_schema') AND left(n.nspname,3)<>'pg_' AND c.relkind='S' ORDER BY n.nspname,c.relname").fetch_all(&mut *c).await.unwrap();
    let before_import_guard: String = sqlx::query_scalar(
        "SELECT pg_get_functiondef('core_history.guard_connection_snapshot()'::regprocedure)",
    )
    .fetch_one(&mut *c)
    .await
    .unwrap();
    let before_data = snapshot_data(&mut c, &tables, &sequences).await.unwrap();
    let mut expected_data = before_data.clone();
    let new = crate::hex_codec::decode(captured().unwrap().oauth_catalog_sha256).unwrap();
    *expected_data.get_mut("core_oauth.contract").unwrap() =
        singleton_hash(&mut c, Some(new)).await.unwrap();
    let before_row: serde_json::Value =
        sqlx::query_scalar("SELECT to_jsonb(c) FROM core_oauth.contract c")
            .fetch_one(&mut *c)
            .await
            .unwrap();
    let rels = super::super::finance::object_ids(&mut c, "relation")
        .await
        .unwrap();
    let types = super::super::finance::object_ids(&mut c, "type")
        .await
        .unwrap();
    let before_permissions = super::super::finance::permissions(&mut c, &rels, &types)
        .await
        .unwrap();
    let keys: std::collections::BTreeSet<String> = before_permissions
        .iter()
        .filter_map(|v| permission_key(v))
        .collect();
    drop(c);
    let mut url = reqwest::Url::parse(&std::env::var("DATABASE_URL").unwrap()).unwrap();
    url.set_path(&format!("/{}", request.database.name));
    url.set_username(&request.roles.installer).unwrap();
    url.set_password(Some(&password)).unwrap();
    installer.close().await;
    pool.close().await;
    let mut wrong = request.clone();
    wrong.predecessor_catalog_sha256 = "00".repeat(32);
    engine_offline_boundary(&request).await;
    assert!(matches!(
        upgrade_database(url.as_str(), wrong).await,
        Err(UpgradeError::Rejected(
            "PROVIDER_CLIENTS_UPGRADE_CONTRACT_INVALID"
        ))
    ));
    engine_offline_boundary(&request).await;
    let rollback = FAIL_AFTER_OAUTH_CAS
        .scope(true, upgrade_database(url.as_str(), request.clone()))
        .await;
    assert!(matches!(
        rollback,
        Err(UpgradeError::Rejected(
            "PROVIDER_CLIENTS_INJECTED_AFTER_OAUTH_CAS"
        ))
    ));
    let mut inspect = PgConnection::connect_with(&options).await.unwrap();
    assert_eq!(
        snapshot_data(&mut inspect, &tables, &sequences)
            .await
            .unwrap(),
        before_data
    );
    assert_eq!(
        hex(fingerprint(&definitions(&mut inspect).await.unwrap())),
        request.predecessor_catalog_sha256
    );
    inspect.close().await.unwrap();
    engine_offline_boundary(&request).await;
    let proxy = crate::test_pg_commit_fault::PgCommitFault::start(
        url.as_str(),
        "INSERT INTO core_meta.provider_clients_upgrades",
    )
    .await;
    let lost = upgrade_database(&proxy.url, request.clone()).await;
    assert!(
        matches!(lost, Err(UpgradeError::CommitUnknown { .. })),
        "real lost COMMIT acknowledgement stays unknown"
    );
    proxy.assert_commit_ack_dropped().await;
    drop(proxy);
    engine_offline_boundary(&request).await;
    let recovered = upgrade_database(url.as_str(), request.clone())
        .await
        .unwrap();
    assert_eq!(recovered.status, "already_committed");
    assert_eq!(recovered.components, 78);
    let mut inspect = PgConnection::connect_with(&options).await.unwrap();
    assert_eq!(
        snapshot_data(&mut inspect, &tables, &sequences)
            .await
            .unwrap(),
        expected_data
    );
    let after_permissions: Vec<String> =
        super::super::finance::permissions(&mut inspect, &rels, &types)
            .await
            .unwrap()
            .into_iter()
            .filter(|v| permission_key(v).is_none_or(|k| keys.contains(&k)))
            .collect();
    assert_eq!(before_permissions, after_permissions);
    let mut row = before_row;
    row["catalog_fingerprint"] =
        serde_json::json!(format!("\\x{}", captured().unwrap().oauth_catalog_sha256));
    let actual: serde_json::Value =
        sqlx::query_scalar("SELECT to_jsonb(c) FROM core_oauth.contract c")
            .fetch_one(&mut inspect)
            .await
            .unwrap();
    assert_eq!(actual, row);
    check_oauth_contract(&mut inspect).await.unwrap();
    let after_import_guard: String = sqlx::query_scalar(
        "SELECT pg_get_functiondef('core_history.guard_connection_snapshot()'::regprocedure)",
    )
    .fetch_one(&mut inspect)
    .await
    .unwrap();
    assert!(before_import_guard.contains("component_contract)<>77"));
    assert_eq!(
        after_import_guard,
        before_import_guard.replace("component_contract)<>77", "component_contract)<>78")
    );
    let count: i64 = sqlx::query_scalar("SELECT count(*) FROM core_meta.provider_clients_upgrades")
        .fetch_one(&mut inspect)
        .await
        .unwrap();
    assert_eq!(count, 1);
    let stored: Vec<u8> =
        sqlx::query_scalar("SELECT preserved_data_sha256 FROM core_meta.provider_clients_upgrades")
            .fetch_one(&mut inspect)
            .await
            .unwrap();
    assert_eq!(stored, super::super::finance::data_hash(&before_data));
    sqlx::raw_sql(sqlx::AssertSqlSafe(format!("REASSIGN OWNED BY {} TO {}; DROP OWNED BY {},{},{},{} CASCADE; ALTER DATABASE {} OWNER TO {}; DROP ROLE {},{},{},{};",
        quote(&request.roles.owner),quote(&original),quote(&request.roles.installer),quote(&request.roles.owner),quote(&request.roles.runtime),quote(&request.roles.extensions),quote(&request.database.name),quote(&original),quote(&request.roles.installer),quote(&request.roles.owner),quote(&request.roles.runtime),quote(&request.roles.extensions))))
        .execute(&mut inspect).await.unwrap();
    inspect.close().await.unwrap();
}

#[sqlx::test(migrations = false)]
async fn provider78_fresh_catalog_and_scope_boundaries(pool: PgPool) {
    crate::core_schema::init(&pool).await.unwrap();
    crate::core_schema::managed_applications_ready(&pool)
        .await
        .unwrap();
    let mut c = pool.acquire().await.unwrap();
    check_oauth_contract(&mut c).await.unwrap();
    for client in [
        "lmm-dsh",
        "lmm-opencode",
        "lmm-codewhale",
        "lmm-vscode",
        "lmm-zed",
    ] {
        for scopes in [
            vec![
                "catalog:read",
                "balance:read",
                "usage:read",
                "models:invoke",
            ],
            vec![
                "catalog:read",
                "balance:read",
                "usage:read",
                "models:invoke",
                "mcp:bounties",
                "mcp:drawing",
            ],
        ] {
            let allowed: bool =
                sqlx::query_scalar("SELECT core_oauth.client_scope_set($1,$2,true)")
                    .bind(client)
                    .bind(scopes)
                    .fetch_one(&mut *c)
                    .await
                    .unwrap();
            assert!(allowed, "{client} initial scopes");
        }
        for scopes in [
            vec![
                "catalog:read",
                "balance:read",
                "usage:read",
                "models:invoke",
                "remote:control",
            ],
            vec!["catalog:read", "balance:read", "models:invoke"],
            vec![
                "catalog:read",
                "balance:read",
                "usage:read",
                "models:invoke",
                "catalog:read",
            ],
        ] {
            let allowed: bool =
                sqlx::query_scalar("SELECT core_oauth.client_scope_set($1,$2,true)")
                    .bind(client)
                    .bind(scopes)
                    .fetch_one(&mut *c)
                    .await
                    .unwrap();
            assert!(!allowed, "{client} invalid initial scopes");
        }
        let allowed: bool = sqlx::query_scalar("SELECT core_oauth.client_scope_set($1,ARRAY['catalog:read','balance:read','usage:read','models:invoke','group:YWxsb3dlZA'],false)")
            .bind(client).fetch_one(&mut *c).await.unwrap();
        assert!(allowed, "{client} final scopes");
        let optional_granted: bool = sqlx::query_scalar("SELECT core_oauth.client_scope_set($1,ARRAY['catalog:read','balance:read','usage:read','models:invoke','mcp:drawing'],false)")
            .bind(client).fetch_one(&mut *c).await.unwrap();
        assert!(!optional_granted);
    }
    let unknown: bool = sqlx::query_scalar("SELECT core_oauth.client_scope_set('lmm-unknown',ARRAY['catalog:read','balance:read','usage:read','models:invoke'],true)")
        .fetch_one(&mut *c).await.unwrap();
    assert!(!unknown);
}

/// Owner-only measurement of a new frozen target in an isolated empty database.
#[sqlx::test(migrations = false)]
#[ignore = "explicit candidate capture requires LMM_PROVIDER_CAPTURE_FILE"]
async fn capture_provider_candidate(pool: PgPool) {
    use crate::core_schema as root;
    let mut tx = pool.begin().await.unwrap();
    for (_, ddl) in &COMPONENTS[..77] {
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
    let predecessor = hex(fingerprint(&definitions(&mut tx).await.unwrap()));
    assert_eq!(predecessor, ENGINE.predecessor_catalog);
    let oauth_predecessor = hex(crate::identity::grants::catalog(&mut tx).await.unwrap());
    sqlx::raw_sql(COMPONENTS[77].1)
        .execute(&mut *tx)
        .await
        .unwrap();
    let catalog = hex(fingerprint(&definitions(&mut tx).await.unwrap()));
    let oauth = hex(crate::identity::grants::catalog(&mut tx).await.unwrap());
    let mut manifest = schema_manifest();
    manifest.components.truncate(78);
    let mut bytes = serde_json::to_vec(&manifest).unwrap();
    bytes.push(b'\n');
    let manifest_sha = hex(Sha256::digest(bytes));
    let result = serde_json::json!({
        "provider": {
            "manifest_sha256": manifest_sha, "catalog_sha256": catalog,
            "oauth_predecessor_catalog_sha256": oauth_predecessor,
            "oauth_catalog_sha256": oauth,
            "raw_schema_sha256": hex(Sha256::digest(COMPONENTS[77].1))
        },
        "fresh": {
            "component_count": COMPONENTS.len(),
            "manifest_sha256": manifest_sha256().unwrap(),
            "catalog_sha256": catalog
        }
    });
    std::fs::write(
        std::env::var_os("LMM_PROVIDER_CAPTURE_FILE").unwrap(),
        serde_json::to_vec_pretty(&result).unwrap(),
    )
    .unwrap();
    tx.rollback().await.unwrap();
}

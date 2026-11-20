//! Real PostgreSQL persistent-client admission and exact owner77->78 preservation.
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
    identity.bootstrap_user(70001, 0).await.unwrap();
    let root = identity.bootstrap_user(70002, 6).await.unwrap();
    crate::identity::admin_users::AdminUsersStore::new(identity)
        .adjust_user_balance(
            &root.secret,
            70001,
            &crate::identity::admin_users::AdjustmentRequest {
                delta_units: "1234568".into(),
                operation_key: "provider78-real-prior-ledger-operation".into(),
                reason: "isolated predecessor preservation".into(),
            },
        )
        .await
        .unwrap();
    // Real prior grant/token rows: component78 must not rewrite existing clients.
    sqlx::query("INSERT INTO core_oauth.families(id,user_id,owner_account_id,user_version,issuer,resource,client_id,scopes,expires_at) SELECT $1,id,personal_account_id,auth_version,'https://api.example.test','https://api.example.test/api/oauth2','lmm-pi',ARRAY['catalog:read'],clock_timestamp()+interval '1 day' FROM core_identity.users WHERE id=70001")
        .bind(format!("lmm_gr_{}","a".repeat(64))).execute(&installer).await.unwrap();
    sqlx::query("INSERT INTO core_oauth.refresh_tokens(digest,family_id,consent_version,scopes,expires_at) SELECT decode(repeat('01',32),'hex'),id,consent_version,scopes,expires_at FROM core_oauth.families")
        .execute(&installer).await.unwrap();
    for (index, client) in ["lmm", "lmm-cc-switch", "lmm-magpie"]
        .into_iter()
        .enumerate()
    {
        let (purpose, budget, scopes) = if client == "lmm" {
            (
                "read_only",
                None,
                vec!["catalog:read", "balance:read", "group:ZGVmYXVsdA"],
            )
        } else {
            (
                "normal_model",
                Some(23_i64),
                vec![
                    "catalog:read",
                    "balance:read",
                    "usage:read",
                    "models:invoke",
                    "group:ZGVmYXVsdA",
                ],
            )
        };
        sqlx::query("INSERT INTO core_oauth.families(id,user_id,owner_account_id,user_version,issuer,resource,client_id,issuance_kind,purpose,budget_credits,scopes,expires_at) SELECT $1,id,personal_account_id,auth_version,'https://api.example.test','https://api.example.test/api/oauth2',$2,'browser',$3,$4,$5,clock_timestamp()+interval '1 day' FROM core_identity.users WHERE id=70001")
            .bind(format!("lmm_gr_{:064x}",index+11)).bind(client).bind(purpose).bind(budget).bind(scopes).execute(&installer).await.unwrap();
    }
    sqlx::raw_sql(sqlx::AssertSqlSafe(format!("GRANT USAGE ON SCHEMA core_identity,core_meta,core_oauth TO {runtime}; GRANT SELECT ON core_identity.users TO {runtime}; GRANT UPDATE(revoked_at) ON core_identity.credentials TO {runtime}; GRANT USAGE ON SEQUENCE core_identity.credentials_id_seq TO {runtime}; GRANT EXECUTE ON FUNCTION core_oauth.client_scope_set(text,text[],boolean) TO {runtime};",runtime=quote(&request.roles.runtime))))
        .execute(&installer).await.unwrap();
    let mut c = installer.acquire().await.unwrap();
    sqlx::query("SELECT setval('core_identity.credentials_id_seq',9000,false)")
        .execute(&mut *c)
        .await
        .unwrap();
    let tables:Vec<String>=sqlx::query_scalar("SELECT format('%I.%I',n.nspname,c.relname) FROM pg_class c JOIN pg_namespace n ON n.oid=c.relnamespace WHERE n.nspname NOT IN ('pg_catalog','information_schema') AND left(n.nspname,3)<>'pg_' AND c.relkind IN ('r','p') ORDER BY n.nspname,c.relname").fetch_all(&mut *c).await.unwrap();
    let sequences:Vec<String>=sqlx::query_scalar("SELECT format('%I.%I',n.nspname,c.relname) FROM pg_class c JOIN pg_namespace n ON n.oid=c.relnamespace WHERE n.nspname NOT IN ('pg_catalog','information_schema') AND left(n.nspname,3)<>'pg_' AND c.relkind='S' ORDER BY n.nspname,c.relname").fetch_all(&mut *c).await.unwrap();
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
            "PROVIDER_OAUTH_CLIENTS_UPGRADE_CONTRACT_INVALID"
        ))
    ));
    let mut wrong_roles = request.clone();
    std::mem::swap(&mut wrong_roles.roles.owner, &mut wrong_roles.roles.runtime);
    engine_offline_boundary(&request).await;
    assert!(upgrade_database(url.as_str(), wrong_roles).await.is_err());
    engine_offline_boundary(&request).await;
    let rollback = FAIL_AFTER_OAUTH_CAS
        .scope(true, upgrade_database(url.as_str(), request.clone()))
        .await
        .map(|report| report.status);
    assert!(
        matches!(
            &rollback,
            Err(UpgradeError::Rejected(
                "PROVIDER_OAUTH_CLIENTS_INJECTED_AFTER_OAUTH_CAS"
            ))
        ),
        "provider78 injected rollback result: {rollback:?}"
    );
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
        "INSERT INTO core_meta.provider_oauth_clients_upgrades",
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
    let count: i64 =
        sqlx::query_scalar("SELECT count(*) FROM core_meta.provider_oauth_clients_upgrades")
            .fetch_one(&mut inspect)
            .await
            .unwrap();
    assert_eq!(count, 1);
    let stored: Vec<u8> = sqlx::query_scalar(
        "SELECT preserved_data_sha256 FROM core_meta.provider_oauth_clients_upgrades",
    )
    .fetch_one(&mut inspect)
    .await
    .unwrap();
    assert_eq!(stored, super::super::finance::data_hash(&before_data));
    let runtime_receipt_access: bool = sqlx::query_scalar("SELECT has_table_privilege($1,'core_meta.provider_oauth_clients_upgrades','SELECT,INSERT,UPDATE,DELETE,TRUNCATE')")
        .bind(&request.roles.runtime).fetch_one(&mut inspect).await.unwrap();
    assert!(!runtime_receipt_access);
    let guard: String = sqlx::query_scalar(
        "SELECT pg_get_functiondef('core_history.guard_connection_snapshot()'::regprocedure)",
    )
    .fetch_one(&mut inspect)
    .await
    .unwrap();
    assert!(guard.contains("core_meta.component_contract)<>78"));
    assert!(!guard.contains("core_meta.component_contract)<>77"));
    let immutable = sqlx::query(
        "UPDATE core_meta.provider_oauth_clients_upgrades SET upgrade_id='not-an-update'",
    )
    .execute(&mut inspect)
    .await
    .unwrap_err();
    assert_eq!(
        immutable
            .as_database_error()
            .and_then(|e| e.code())
            .as_deref(),
        Some("55000")
    );
    sqlx::raw_sql(sqlx::AssertSqlSafe(format!("REASSIGN OWNED BY {} TO {}; DROP OWNED BY {},{},{},{} CASCADE; ALTER DATABASE {} OWNER TO {}; DROP ROLE {},{},{},{};",
        quote(&request.roles.owner),quote(&original),quote(&request.roles.installer),quote(&request.roles.owner),quote(&request.roles.runtime),quote(&request.roles.extensions),quote(&request.database.name),quote(&original),quote(&request.roles.installer),quote(&request.roles.owner),quote(&request.roles.runtime),quote(&request.roles.extensions))))
        .execute(&mut inspect).await.unwrap();
    inspect.close().await.unwrap();
}

const PUBLIC_PROVIDERS: [&str; 5] = [
    "lmm-dsh",
    "lmm-codewhale",
    "lmm-opencode",
    "lmm-vscode",
    "lmm-zed",
];

async fn persist_family(
    pool: &PgPool,
    id: &str,
    client: &str,
    purpose: &str,
    budget: Option<i64>,
) -> Result<(), sqlx::Error> {
    sqlx::query("INSERT INTO core_oauth.families(id,user_id,owner_account_id,user_version,issuer,resource,client_id,issuance_kind,purpose,budget_credits,scopes,expires_at) SELECT $1,id,personal_account_id,auth_version,'https://api.example.test','https://api.example.test/api/oauth2',$2,'browser',$3,$4,ARRAY['catalog:read','balance:read','usage:read','models:invoke','group:ZGVmYXVsdA'],clock_timestamp()+interval '1 day' FROM core_identity.users WHERE id=78001")
        .bind(id).bind(client).bind(purpose).bind(budget).execute(pool).await?;
    Ok(())
}

async fn persist_request(
    pool: &PgPool,
    seed: u8,
    client: &str,
    scopes: Vec<&str>,
) -> Result<(), sqlx::Error> {
    sqlx::query("INSERT INTO core_oauth.authorization_requests(binding_digest,csrf_digest,client_id,issuer,resource,redirect_uri,state,pkce_challenge,requested_scopes,granted_scopes) VALUES($1,$2,$3,'https://api.example.test','https://api.example.test/api/oauth2','http://127.0.0.1:38123/oauth/lmm/callback','isolated-provider-state',$4,$5,ARRAY[]::text[])")
        .bind(vec![seed;32]).bind(vec![seed.wrapping_add(100);32]).bind(client).bind("A".repeat(43)).bind(scopes).execute(pool).await?;
    Ok(())
}

fn assert_check_violation(error: sqlx::Error) {
    assert_eq!(
        error.as_database_error().and_then(|e| e.code()).as_deref(),
        Some("23514")
    );
}

#[sqlx::test(migrations = false)]
async fn provider78_fresh_persistent_client_boundaries(pool: PgPool) {
    authorize_target().unwrap();
    crate::core_schema::init(&pool).await.unwrap();
    crate::core_schema::managed_applications_ready(&pool)
        .await
        .unwrap();
    IdentityStore::from_pool(pool.clone())
        .bootstrap_user(78001, 0)
        .await
        .unwrap();
    let initial = vec![
        "catalog:read",
        "balance:read",
        "usage:read",
        "models:invoke",
    ];
    for (index, client) in PUBLIC_PROVIDERS.into_iter().enumerate() {
        let family = format!("lmm_gr_{:064x}", index + 1);
        persist_request(
            &pool,
            u8::try_from(index + 1).unwrap(),
            client,
            initial.clone(),
        )
        .await
        .unwrap();
        persist_family(&pool, &family, client, "normal_model", Some(17))
            .await
            .unwrap();
        sqlx::query("INSERT INTO core_oauth.authorization_codes(digest,family_id,user_id,owner_account_id,user_version,consent_version,issuer,resource,client_id,redirect_uri,state,pkce_challenge,scopes,expires_at) SELECT $1,id,user_id,owner_account_id,user_version,consent_version,issuer,resource,client_id,'http://127.0.0.1:38123/oauth/lmm/callback','isolated-provider-state',$2,scopes,statement_timestamp()+interval '60 seconds' FROM core_oauth.families WHERE id=$3")
            .bind(vec![u8::try_from(index+1).unwrap();32]).bind("A".repeat(43)).bind(&family).execute(&pool).await.unwrap();
        sqlx::query("INSERT INTO core_oauth.access_tokens(id,digest,family_id,consent_version,scopes,expires_at) SELECT $1,$2,id,consent_version,scopes,statement_timestamp()+interval '900 seconds' FROM core_oauth.families WHERE id=$3")
            .bind(format!("lmm_ac_{:064x}",index+1)).bind(vec![u8::try_from(index+1).unwrap();32]).bind(&family).execute(&pool).await.unwrap();
        sqlx::query("INSERT INTO core_oauth.refresh_tokens(digest,family_id,consent_version,scopes,expires_at) SELECT $1,id,consent_version,scopes,expires_at FROM core_oauth.families WHERE id=$2")
            .bind(vec![u8::try_from(index+1).unwrap();32]).bind(&family).execute(&pool).await.unwrap();
        let observed: (String,String,String,i64) = sqlx::query_as("SELECT f.client_id,c.client_id,r.client_id,f.budget_credits FROM core_oauth.families f JOIN core_oauth.authorization_codes c ON c.family_id=f.id JOIN core_oauth.authorization_requests r ON r.client_id=f.client_id WHERE f.id=$1")
            .bind(&family).fetch_one(&pool).await.unwrap();
        assert_eq!(observed, (client.into(), client.into(), client.into(), 17));
        for (purpose, budget) in [
            ("foundation", None),
            ("honeypot_only", None),
            ("normal_model", None),
            ("normal_model", Some(0)),
            ("normal_model", Some(9_007_199_254_740_992)),
        ] {
            assert_check_violation(
                persist_family(
                    &pool,
                    &format!("lmm_gr_{}", "f".repeat(64)),
                    client,
                    purpose,
                    budget,
                )
                .await
                .unwrap_err(),
            );
        }
        let mut remote = initial.clone();
        remote.push("remote:control");
        assert_check_violation(
            persist_request(&pool, 90, client, remote)
                .await
                .unwrap_err(),
        );
        let mut duplicate = initial.clone();
        duplicate.push("models:invoke");
        assert_check_violation(
            persist_request(&pool, 91, client, duplicate)
                .await
                .unwrap_err(),
        );
    }
    for client in ["unknown-provider", "lmm-dsh ", "LMM-DSH"] {
        assert_check_violation(
            persist_request(&pool, 92, client, initial.clone())
                .await
                .unwrap_err(),
        );
        assert_check_violation(
            persist_family(
                &pool,
                &format!("lmm_gr_{}", "e".repeat(64)),
                client,
                "normal_model",
                Some(17),
            )
            .await
            .unwrap_err(),
        );
    }
    // New clients never acquire managed installation or recovery authority.
    let managed_bindings: i64 =
        sqlx::query_scalar("SELECT count(*) FROM core_oauth.managed_family_bindings")
            .fetch_one(&pool)
            .await
            .unwrap();
    assert_eq!(managed_bindings, 0);
    sqlx::query("INSERT INTO core_oauth.families(id,user_id,owner_account_id,user_version,issuer,resource,client_id,scopes,expires_at) SELECT $1,id,personal_account_id,auth_version,'https://api.example.test','https://api.example.test/api/oauth2','lmm-pi',ARRAY['catalog:read'],clock_timestamp()+interval '1 day' FROM core_identity.users WHERE id=78001")
        .bind(format!("lmm_gr_{}","a".repeat(64))).execute(&pool).await.unwrap();
    sqlx::query("UPDATE core_oauth.contract SET catalog_fingerprint=decode(repeat('00',32),'hex')")
        .execute(&pool)
        .await
        .unwrap();
    assert!(
        crate::core_schema::managed_applications_ready(&pool)
            .await
            .is_err()
    );
    let retained: Vec<u8> =
        sqlx::query_scalar("SELECT catalog_fingerprint FROM core_oauth.contract WHERE singleton")
            .fetch_one(&pool)
            .await
            .unwrap();
    assert_eq!(
        retained,
        vec![0; 32],
        "startup must not repair a damaged OAuth contract"
    );
}

fn manifest_for(count: usize) -> (String, String) {
    let mut manifest = schema_manifest();
    manifest.components.truncate(count);
    let mut bytes = serde_json::to_vec(&manifest).unwrap();
    bytes.push(b'\n');
    (
        hex(Sha256::digest(&bytes)),
        String::from_utf8(bytes).unwrap(),
    )
}

async fn capture_schema(c: &mut PgConnection, count: usize) {
    use crate::core_schema as root;
    for (_, ddl) in &COMPONENTS[..count] {
        sqlx::raw_sql(*ddl).execute(&mut *c).await.unwrap();
    }
    for ddl in [
        root::CONTRACT,
        root::UPGRADE_METADATA,
        root::ADDITIVE_UPGRADE_METADATA,
        root::LEDGER_REVISION_METADATA,
        root::CONSOLE_REVISION_METADATA,
        root::OFFLINE_REVISION_METADATA,
    ] {
        sqlx::raw_sql(ddl).execute(&mut *c).await.unwrap();
    }
}

/// Capture is an explicit owner measurement on one disposable SQLx database.
/// It executes the real DDL, rolls both transactions back, and writes proposals;
/// it neither bypasses production target checks nor modifies a source target.
#[sqlx::test(migrations = false)]
#[ignore = "explicit PG18.6 capture requires LMM_PROVIDER_OAUTH_CAPTURE_FILE and source SHA"]
async fn capture_provider_oauth_clients78_targets(pool: PgPool) {
    use std::{fs::OpenOptions, io::Write, os::unix::fs::OpenOptionsExt, path::PathBuf};
    assert_eq!(COMPONENTS.len(), 78);
    assert_eq!(COMPONENTS[77].0, "provider-oauth-clients");
    let version: i64 = sqlx::query_scalar("SELECT current_setting('server_version_num')::bigint")
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(
        version, 180_006,
        "target capture is bound to PostgreSQL18.6"
    );
    let source = std::env::var("LMM_PROVIDER_OAUTH_CAPTURE_SOURCE_SHA").unwrap();
    assert!(
        source.len() == 40
            && source
                .bytes()
                .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
    );
    let empty: bool = sqlx::query_scalar("SELECT NOT EXISTS(SELECT FROM pg_class c JOIN pg_namespace n ON n.oid=c.relnamespace WHERE n.nspname NOT IN ('pg_catalog','information_schema') AND left(n.nspname,3)<>'pg_' AND c.relkind IN ('r','p','S'))")
        .fetch_one(&pool).await.unwrap();
    assert!(empty, "capture requires an empty isolated database");
    let output = PathBuf::from(std::env::var_os("LMM_PROVIDER_OAUTH_CAPTURE_FILE").unwrap());
    let parent = output.parent().unwrap().canonicalize().unwrap();
    let private = PathBuf::from(std::env::var_os("HOME").unwrap())
        .join(".local/state/lmm-api")
        .canonicalize()
        .unwrap();
    assert!(parent.starts_with(private));
    let (manifest77, _) = manifest_for(77);
    assert_eq!(manifest77, ENGINE.predecessor_manifest);
    let mut tx = pool.begin().await.unwrap();
    capture_schema(&mut tx, 77).await;
    let predecessor_catalog = hex(fingerprint(&definitions(&mut tx).await.unwrap()));
    assert_eq!(predecessor_catalog, ENGINE.predecessor_catalog);
    let oauth_predecessor = hex(crate::identity::grants::catalog(&mut tx).await.unwrap());
    sqlx::raw_sql(COMPONENTS[77].1)
        .execute(&mut *tx)
        .await
        .unwrap();
    let catalog78 = hex(fingerprint(&definitions(&mut tx).await.unwrap()));
    let oauth78 = hex(crate::identity::grants::catalog(&mut tx).await.unwrap());
    tx.rollback().await.unwrap();
    let mut tx = pool.begin().await.unwrap();
    capture_schema(&mut tx, 78).await;
    let fresh_catalog = hex(fingerprint(&definitions(&mut tx).await.unwrap()));
    let fresh_oauth = hex(crate::identity::grants::catalog(&mut tx).await.unwrap());
    assert_eq!(
        fresh_catalog, catalog78,
        "fresh and incremental assembly must have the same catalog"
    );
    assert_eq!(fresh_oauth, oauth78);
    tx.rollback().await.unwrap();
    let (manifest78, manifest_utf8) = manifest_for(78);
    let historical: serde_json::Map<String, serde_json::Value> = [
        "oauth-grants",
        "managed-applications",
        "charging",
        "connection-history-import",
    ]
    .into_iter()
    .map(|name| {
        let (_, ddl) = COMPONENTS.iter().find(|(n, _)| *n == name).unwrap();
        (
            name.into(),
            serde_json::json!(hex(Sha256::digest(ddl.as_bytes()))),
        )
    })
    .collect();
    let result = serde_json::json!({
        "schema":"core-provider-oauth-clients-capture.v1",
        "source_sha":source,"server_version_num":version,
        "predecessor":{"component_count":77,"manifest_sha256":manifest77,"catalog_sha256":predecessor_catalog},
        "provider_target":{
            "manifest_sha256":manifest78,"catalog_sha256":catalog78,
            "oauth_predecessor_catalog_sha256":oauth_predecessor,
            "oauth_catalog_sha256":oauth78,"raw_schema_sha256":hex(Sha256::digest(COMPONENTS[77].1))
        },
        "fresh_target":{"component_count":78,"manifest_sha256":manifest78,"catalog_sha256":fresh_catalog},
        "fresh_and_incremental_catalog_equal":true,
        "historical_schema_sha256":historical,
        "compiled_manifest_utf8":manifest_utf8,
        "database_transactions":"both_rolled_back",
        "qualification":"catalog_measurement_only; ordinary fresh and owner77->78 acceptance still required"
    });
    let mut file = OpenOptions::new()
        .write(true)
        .create_new(true)
        .mode(0o600)
        .open(output)
        .unwrap();
    writeln!(file, "{}", serde_json::to_string_pretty(&result).unwrap()).unwrap();
    file.sync_all().unwrap();
}

//! Real isolated PostgreSQL upgrade, exact replay, and preservation.
use super::super::payment_discounts::tests::{EngineBaseline, engine_offline_boundary};
use super::*;
use crate::core_schema::{UpgradeDatabase, UpgradeRoles, upgrade_database};
use sqlx::{PgPool, postgres::PgPoolOptions};
async fn prefix82(pool: &PgPool) {
    use crate::core_schema as root;
    let mut tx = pool.begin().await.unwrap();
    for (_, ddl) in &COMPONENTS[..82] {
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
    for (name, ddl) in &COMPONENTS[..82] {
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
async fn agent_gift83_exact_upgrade_preserves_data_acl_and_replays(pool: PgPool) {
    authorize_target().unwrap();
    let facts=sqlx::query("SELECT current_database()::text AS name,d.oid::bigint AS oid,current_setting('server_version_num')::bigint AS version,(SELECT system_identifier::text FROM pg_control_system()) AS system,session_user::text AS original FROM pg_database d WHERE datname=current_database()")
        .fetch_one(&pool).await.unwrap();
    let suffix = format!("{:016x}", rand::random::<u64>());
    let request = UpgradeRequest {
        version: 1,
        procedure: PROCEDURE.into(),
        upgrade_id: format!("agent_gift83-owner-{suffix}"),
        database: UpgradeDatabase {
            name: facts.get("name"),
            oid: u32::try_from(facts.get::<i64, _>("oid")).unwrap(),
            system_identifier: facts.get("system"),
            server_version_num: u32::try_from(facts.get::<i64, _>("version")).unwrap(),
        },
        roles: UpgradeRoles {
            installer: format!("agent_gift83_i_{suffix}"),
            owner: format!("agent_gift83_o_{suffix}"),
            runtime: format!("agent_gift83_r_{suffix}"),
            extensions: format!("agent_gift83_e_{suffix}"),
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
    prefix82(&installer).await;

    let identity = IdentityStore::from_pool(installer.clone());
    let original_user = identity.bootstrap_user(79001, 0).await.unwrap();
    let root = identity.bootstrap_user(79002, 6).await.unwrap();
    crate::identity::admin_users::AdminUsersStore::new(identity)
        .adjust_user_balance(
            &root.secret,
            79001,
            &crate::identity::admin_users::AdjustmentRequest {
                delta_units: "1234568".into(),
                operation_key: "agent_gift83-prior-opening-operation-0001".into(),
                reason: "isolated predecessor preservation".into(),
            },
        )
        .await
        .unwrap();
    let mut seed = installer.begin().await.unwrap();
    sqlx::query("SET TRANSACTION ISOLATION LEVEL SERIALIZABLE")
        .execute(&mut *seed)
        .await
        .unwrap();
    let grant:i64=sqlx::query_scalar("INSERT INTO core_charging.subscriptions(account_id,grant_reference,period,anchor,seconds,ends_at,limit_credits,models,groups) SELECT personal_account_id,'agent_gift83-old-grant','grant',1,0,253402300799,1000,ARRAY['*'],ARRAY['*'] FROM core_identity.users WHERE id=79001 RETURNING id")
        .fetch_one(&mut *seed).await.unwrap();
    sqlx::query("INSERT INTO core_charging.subscription_opening_consumption(subscription_id,start_at,end_at,consumed_credits,import_reference) VALUES($1,1,253402300799,123,'agent_gift83-old-opening')")
        .bind(grant).execute(&mut *seed).await.unwrap();
    seed.commit().await.unwrap();
    sqlx::query("INSERT INTO core_console.options(key,value,revision) VALUES ('IPAccessRoutingRules','fallback: reject',12)")
        .execute(&installer).await.unwrap();
    sqlx::raw_sql(sqlx::AssertSqlSafe(format!(
        "GRANT USAGE ON SCHEMA core_console TO {runtime}; GRANT SELECT ON core_console.options TO {runtime}; GRANT EXECUTE ON FUNCTION core_console.lock_public_options(),core_console.write_public_option(text,text),core_console.write_display_exchange_rate(text) TO {runtime}",
        runtime=quote(&request.roles.runtime)
    ))).execute(&installer).await.unwrap();
    // An existing non-sponsored grant remains byte-for-byte unchanged.
    let owner: i64 =
        sqlx::query_scalar("SELECT personal_account_id FROM core_identity.users WHERE id=79001")
            .fetch_one(&installer)
            .await
            .unwrap();
    let conversation = format!("lmm_cv_{}", "1".repeat(64));
    let grant_id = format!("lmm_ad_{}", "2".repeat(64));
    sqlx::query("INSERT INTO core_assistant.conversations(id,owner_user_id,owner_account_id,creation_key,created_at) VALUES($1,79001,$2,'prior-service-funding-conversation',1)")
        .bind(&conversation).bind(owner).execute(&installer).await.unwrap();
    sqlx::query("INSERT INTO core_assistant.grants(id,conversation_id,turn_id,session_id,auth_version,policy,price,issued_at,expires_at) VALUES($1,$2,'old-turn',$3,1,$4,'{}',1,301)")
        .bind(grant_id).bind(conversation).bind(original_user.id)
        .bind(serde_json::json!({"model":"prior-model","group":"prior-group","budget_credits":10000,"max_output_tokens":32,"ttl_seconds":300,"gift_tools":false,"sponsor":null}))
        .execute(&installer).await.unwrap();
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
    assert_eq!(report.components, 83);
    engine_offline_boundary(&request).await;
    let report = upgrade_database(url.as_str(), request.clone())
        .await
        .unwrap();
    assert_eq!(report.status, "already_committed");
    let mut inspect = PgConnection::connect_with(&options).await.unwrap();
    baseline.assert_preserved(&mut inspect).await;
    let owner_only:bool=sqlx::query_scalar("SELECT NOT has_table_privilege($1,'core_meta.assistant_agent_gift_upgrades','SELECT,INSERT,UPDATE,DELETE,TRUNCATE') AND has_table_privilege($1,'core_console.options','SELECT')")
        .bind(&request.roles.runtime).fetch_one(&mut inspect).await.unwrap();
    assert!(owner_only);
    let consumption:(i64,i64,Option<i64>)=sqlx::query_as("SELECT used_credits::bigint,reserved_credits::bigint,reset_event_id FROM core_charging.subscription_consumption($1,1,NULL)")
        .bind(grant).fetch_one(&mut inspect).await.unwrap();
    assert_eq!(
        consumption,
        (123, 0, None),
        "generation zero must retain old imported consumption"
    );
    let reset_rows:i64=sqlx::query_scalar("SELECT (SELECT count(*) FROM core_subscriptions.reset_events)+(SELECT count(*) FROM core_charging.subscription_request_resets)").fetch_one(&mut inspect).await.unwrap();
    assert_eq!(reset_rows, 0, "an upgrade must not reset any grant");
    let history: String = sqlx::query_scalar(
        "SELECT pg_get_functiondef('core_history.guard_connection_snapshot()'::regprocedure)",
    )
    .fetch_one(&mut inspect)
    .await
    .unwrap();
    assert!(history.contains("component_contract)<>82"));
    let old_policy: (String, i64) = sqlx::query_as(
        "SELECT value,revision FROM core_console.options WHERE key='IPAccessRoutingRules'",
    )
    .fetch_one(&mut inspect)
    .await
    .unwrap();
    assert_eq!(old_policy, ("fallback: reject".into(), 12));
    let bindings: i64 = sqlx::query_scalar("SELECT count(*) FROM pg_trigger WHERE tgname IN ('agent_gift_tool_binding','agent_gift_decision_binding') AND tgdeferrable AND tginitdeferred AND tgenabled='O'")
        .fetch_one(&mut inspect).await.unwrap();
    assert_eq!(bindings, 2);
    let receipts: i64 =
        sqlx::query_scalar("SELECT count(*) FROM core_meta.assistant_agent_gift_upgrades")
            .fetch_one(&mut inspect)
            .await
            .unwrap();
    assert_eq!(receipts, 1);
    sqlx::raw_sql(sqlx::AssertSqlSafe(format!("REASSIGN OWNED BY {} TO {}; DROP OWNED BY {},{},{},{} CASCADE; ALTER DATABASE {} OWNER TO {}; DROP ROLE {},{},{},{};", quote(&request.roles.owner),quote(&original),quote(&request.roles.installer),quote(&request.roles.owner),quote(&request.roles.runtime),quote(&request.roles.extensions),quote(&request.database.name),quote(&original),quote(&request.roles.installer),quote(&request.roles.owner),quote(&request.roles.runtime),quote(&request.roles.extensions)))).execute(&mut inspect).await.unwrap();
    inspect.close().await.unwrap();
}

#[test]
fn agent_gift83_suffix_keeps_exact_predecessor_and_append_position() {
    assert_eq!(ENGINE.predecessor_count, 82);
    assert_eq!(ENGINE.target_count, 83);
    assert_eq!(COMPONENTS[82].0, "assistant-agent-gift");
    let old: serde_json::Value =
        serde_json::from_str(include_str!("assistant_service_funding_target.json")).unwrap();
    assert_eq!(ENGINE.predecessor_manifest, old["manifest_sha256"]);
    assert_eq!(ENGINE.predecessor_catalog, old["catalog_sha256"]);
}

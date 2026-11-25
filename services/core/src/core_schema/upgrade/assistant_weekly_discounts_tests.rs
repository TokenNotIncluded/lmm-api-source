//! Real isolated PostgreSQL upgrade, exact replay, and preservation.
use super::super::payment_discounts::tests::{EngineBaseline, engine_offline_boundary};
use super::*;
use crate::core_schema::{UpgradeDatabase, UpgradeRoles, upgrade_database};
use sqlx::{PgPool, postgres::PgPoolOptions};
const READER: u8 = 1;
const BROWSER: u8 = 2;
const AGENT: u8 = 4;
async fn prefix87(pool: &PgPool) {
    use crate::core_schema as root;
    let mut tx = pool.begin().await.unwrap();
    for (_, ddl) in &COMPONENTS[..87] {
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
    for (name, ddl) in &COMPONENTS[..87] {
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

async fn exact_upgrade(pool: PgPool, capabilities: u8) {
    authorize_target().unwrap();
    let facts=sqlx::query("SELECT current_database()::text AS name,d.oid::bigint AS oid,current_setting('server_version_num')::bigint AS version,(SELECT system_identifier::text FROM pg_control_system()) AS system,session_user::text AS original FROM pg_database d WHERE datname=current_database()")
        .fetch_one(&pool).await.unwrap();
    let suffix = format!("{:016x}", rand::random::<u64>());
    let request = UpgradeRequest {
        version: 1,
        procedure: PROCEDURE.into(),
        upgrade_id: format!("weekly88-owner-{suffix}"),
        database: UpgradeDatabase {
            name: facts.get("name"),
            oid: u32::try_from(facts.get::<i64, _>("oid")).unwrap(),
            system_identifier: facts.get("system"),
            server_version_num: u32::try_from(facts.get::<i64, _>("version")).unwrap(),
        },
        roles: UpgradeRoles {
            installer: format!("weekly88_i_{suffix}"),
            owner: format!("weekly88_o_{suffix}"),
            runtime: format!("weekly88_r_{suffix}"),
            extensions: format!("weekly88_e_{suffix}"),
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
    prefix87(&installer).await;

    let identity = IdentityStore::from_pool(installer.clone());
    identity.bootstrap_user(79001, 0).await.unwrap();
    let root = identity.bootstrap_user(79002, 6).await.unwrap();
    crate::identity::admin_users::AdminUsersStore::new(identity)
        .adjust_user_balance(
            &root.secret,
            79001,
            &crate::identity::admin_users::AdjustmentRequest {
                delta_units: "1234568".into(),
                operation_key: "weekly88-prior-opening-operation-0001".into(),
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
    let grant:i64=sqlx::query_scalar("INSERT INTO core_charging.subscriptions(account_id,grant_reference,period,anchor,seconds,ends_at,limit_credits,models,groups) SELECT personal_account_id,'weekly88-old-grant','grant',1,0,253402300799,1000,ARRAY['*'],ARRAY['*'] FROM core_identity.users WHERE id=79001 RETURNING id")
        .fetch_one(&mut *seed).await.unwrap();
    sqlx::query("INSERT INTO core_charging.subscription_opening_consumption(subscription_id,start_at,end_at,consumed_credits,import_reference) VALUES($1,1,253402300799,123,'weekly88-old-opening')")
        .bind(grant).execute(&mut *seed).await.unwrap();
    seed.commit().await.unwrap();
    sqlx::query("INSERT INTO core_console.options(key,value,revision) VALUES ('IPAccessRoutingRules','fallback: reject',12)")
        .execute(&installer).await.unwrap();
    sqlx::raw_sql(sqlx::AssertSqlSafe(format!(
        "GRANT USAGE ON SCHEMA core_console TO {runtime}; GRANT SELECT ON core_console.options TO {runtime}; GRANT EXECUTE ON FUNCTION core_console.lock_public_options(),core_console.write_public_option(text,text),core_console.write_display_exchange_rate(text) TO {runtime}",
        runtime=quote(&request.roles.runtime)
    ))).execute(&installer).await.unwrap();
    sqlx::query(
        "INSERT INTO core_identity.affiliate_codes(user_id,code) VALUES(79001,repeat('a',32))",
    )
    .execute(&installer)
    .await
    .unwrap();
    sqlx::query(
        "INSERT INTO core_identity.affiliate_registrations(user_id,inviter_id) VALUES(79002,79001)",
    )
    .execute(&installer)
    .await
    .unwrap();
    if capabilities & BROWSER != 0 {
        sqlx::raw_sql(sqlx::AssertSqlSafe(format!(
            "GRANT USAGE ON SCHEMA core_assistant_tools,core_payments TO {role}; GRANT SELECT,INSERT ON core_assistant_tools.confirmations TO {role}; GRANT UPDATE(nonce_sha256,state,confirmed_at,execution_receipt,execution_sha256,executed_at) ON core_assistant_tools.confirmations TO {role}",
            role=quote(&request.roles.runtime)
        ))).execute(&installer).await.unwrap();
    }
    if capabilities & READER != 0 {
        sqlx::raw_sql(sqlx::AssertSqlSafe(format!(
            "GRANT USAGE ON SCHEMA core_payments TO {role}; GRANT SELECT ON core_payments.discount_codes TO {role}",
            role=quote(&request.roles.runtime)
        ))).execute(&installer).await.unwrap();
    }
    if capabilities & AGENT != 0 {
        sqlx::raw_sql(sqlx::AssertSqlSafe(format!(
            "GRANT USAGE ON SCHEMA core_assistant TO {role}; GRANT SELECT,INSERT ON core_assistant.agent_tool_operations TO {role}",
            role=quote(&request.roles.runtime)
        ))).execute(&installer).await.unwrap();
    }
    sqlx::query("INSERT INTO core_payments.discount_codes(code,name,discount_percent,min_credit_units,max_uses,enabled,starts_at,ends_at,created_by) VALUES('PRIOR-WEEKLY-COUPON','Existing ordinary coupon',5,0,3,TRUE,1,253402300799,79002)")
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
    assert_eq!(report.components, 88);
    engine_offline_boundary(&request).await;
    let report = upgrade_database(url.as_str(), request.clone())
        .await
        .unwrap();
    assert_eq!(report.status, "already_committed");
    let mut inspect = PgConnection::connect_with(&options).await.unwrap();
    baseline.assert_preserved(&mut inspect).await;
    let owner_only:bool=sqlx::query_scalar("SELECT NOT has_table_privilege($1,'core_meta.assistant_weekly_discount_upgrades','SELECT,INSERT,UPDATE,DELETE,TRUNCATE') AND has_table_privilege($1,'core_console.options','SELECT')")
        .bind(&request.roles.runtime).fetch_one(&mut inspect).await.unwrap();
    assert!(owner_only);
    let retained:bool=sqlx::query_scalar("SELECT has_table_privilege($1,'core_assistant_tools.confirmations','SELECT') AND has_table_privilege($1,'core_assistant_tools.confirmations','INSERT') AND has_column_privilege($1,'core_assistant_tools.confirmations','state','UPDATE')")
        .bind(&request.roles.runtime).fetch_one(&mut inspect).await.unwrap();
    assert_eq!(
        retained,
        capabilities & BROWSER != 0,
        "upgrade preserves whether generic confirmations are enabled"
    );
    let receipt_hidden:bool=sqlx::query_scalar("SELECT NOT has_table_privilege($1,'core_meta.assistant_weekly_discount_upgrades','SELECT,INSERT,UPDATE,DELETE,TRUNCATE,REFERENCES,TRIGGER,MAINTAIN') AND NOT has_any_column_privilege($1,'core_meta.assistant_weekly_discount_upgrades','SELECT,INSERT,UPDATE,REFERENCES') AND NOT has_table_privilege($2,'core_assistant_tools.confirmations','SELECT,INSERT,UPDATE,DELETE')")
        .bind(&request.roles.runtime).bind(&request.roles.extensions).fetch_one(&mut inspect).await.unwrap();
    assert!(receipt_hidden, "no upgrade receipt or new Go authority");
    let rights=sqlx::query(
        "SELECT
          has_schema_privilege($1,'core_weekly','USAGE')
            AND has_table_privilege($1,'core_payments.weekly_discount_owners','SELECT') AS coupon_reader,
          has_function_privilege($1,'core_weekly.claim_weekly_discount_code(text,text)','EXECUTE') AS browser_claim,
          has_function_privilege($1,'core_weekly.claimed_weekly_discount_code(text,bigint,bigint)','EXECUTE') AS private_code_read,
          has_column_privilege($1,'core_weekly.weekly_discounts','claim_nonce_sha256','UPDATE')
            AND has_column_privilege($1,'core_weekly.weekly_discounts','claim_nonce_expires_at','UPDATE') AS nonce_writer,
          (SELECT count(*)=13 FROM pg_attribute a WHERE a.attrelid='core_weekly.weekly_discounts'::regclass
            AND a.attnum>0 AND NOT a.attisdropped AND a.attname IN
              ('id','user_id','account_id','week_start','status','discount_percent','reason','conversation_id',
               'operation_id','credential_id','auth_version','web_session_sid','created_at')
            AND has_column_privilege($1,a.attrelid,a.attnum,'INSERT')) AS native_writer,
          has_table_privilege($1,'core_payments.discount_codes','SELECT') AS old_coupon_select,
          NOT has_schema_privilege($1,'core_weekly','CREATE')
            AND NOT has_table_privilege($1,'core_weekly.weekly_discount_policy','INSERT,UPDATE,DELETE,TRUNCATE')
            AND NOT has_any_column_privilege($1,'core_weekly.weekly_discount_policy','INSERT,UPDATE,REFERENCES')
            AND NOT has_table_privilege($1,'core_weekly.legacy_weekly_discount_bases','SELECT,INSERT,UPDATE,DELETE')
            AND NOT has_any_column_privilege($1,'core_weekly.legacy_weekly_discount_bases','SELECT,INSERT,UPDATE,REFERENCES')
            AND NOT has_table_privilege($1,'core_meta.legacy_weekly_discount_imports','SELECT,INSERT,UPDATE,DELETE')
            AND NOT has_any_column_privilege($1,'core_meta.legacy_weekly_discount_imports','SELECT,INSERT,UPDATE,REFERENCES')
            AND NOT has_table_privilege($1,'core_weekly.weekly_discounts','INSERT,UPDATE')
            AND NOT has_column_privilege($1,'core_weekly.weekly_discounts','origin','INSERT')
            AND NOT has_column_privilege($1,'core_weekly.weekly_discounts','code_id','UPDATE')
            AND NOT has_schema_privilege($2,'core_weekly','USAGE') AS owner_boundary")
        .bind(&request.roles.runtime).bind(&request.roles.extensions).fetch_one(&mut inspect).await.unwrap();
    assert!(rights.get::<bool, _>("owner_boundary"));
    assert_eq!(
        rights.get::<bool, _>("coupon_reader"),
        capabilities & (READER | BROWSER) != 0
    );
    assert_eq!(
        rights.get::<bool, _>("browser_claim"),
        capabilities & BROWSER != 0
    );
    assert_eq!(
        rights.get::<bool, _>("private_code_read"),
        capabilities & BROWSER != 0
    );
    assert_eq!(
        rights.get::<bool, _>("nonce_writer"),
        capabilities & BROWSER != 0
    );
    assert_eq!(
        rights.get::<bool, _>("native_writer"),
        capabilities & AGENT != 0
    );
    assert_eq!(
        rights.get::<bool, _>("old_coupon_select"),
        capabilities & READER != 0
    );
    let policy: (bool, i16) = sqlx::query_as(
        "SELECT enabled,maximum_percent FROM core_weekly.weekly_discount_policy WHERE singleton",
    )
    .fetch_one(&mut inspect)
    .await
    .unwrap();
    assert_eq!(
        policy,
        (false, 10),
        "upgrade does not activate new economic decisions"
    );
    let decisions: i64 = sqlx::query_scalar("SELECT count(*) FROM core_weekly.weekly_discounts")
        .fetch_one(&mut inspect)
        .await
        .unwrap();
    assert_eq!(
        decisions, 0,
        "history import and native decisions require their own authority"
    );
    let constraint:String=sqlx::query_scalar("SELECT pg_get_constraintdef(oid) FROM pg_constraint WHERE conrelid='core_assistant_tools.confirmations'::regclass AND conname='confirmations_tool_name_check'")
        .fetch_one(&mut inspect).await.unwrap();
    assert!(
        constraint.contains("'send_invitation'")
            && constraint.contains("'request_create_key'")
            && constraint.contains("'connect_market_tool'")
    );
    let alias_rows: i64 =
        sqlx::query_scalar("SELECT count(*) FROM core_identity.legacy_affiliate_aliases")
            .fetch_one(&mut inspect)
            .await
            .unwrap();
    assert_eq!(
        alias_rows, 0,
        "upgrade does not import links or replay referral rewards"
    );
    assert_eq!(
        crate::affiliate::existing_invitation_code(&mut inspect, 79001)
            .await
            .unwrap(),
        Some("a".repeat(32)),
        "native invitation codes remain available after the weekly upgrade",
    );

    let consumption:(i64,i64,Option<i64>)=sqlx::query_as("SELECT used_credits::bigint,reserved_credits::bigint,reset_event_id FROM core_charging.subscription_consumption($1,1,NULL)")
        .bind(grant).fetch_one(&mut inspect).await.unwrap();
    assert_eq!(
        consumption,
        (123, 0, None),
        "generation zero must retain old imported consumption"
    );
    let reset_rows:i64=sqlx::query_scalar("SELECT (SELECT count(*) FROM core_subscriptions.reset_events)+(SELECT count(*) FROM core_charging.subscription_request_resets)").fetch_one(&mut inspect).await.unwrap();
    assert_eq!(reset_rows, 0, "an upgrade must not reset any grant");
    let old_policy: (String, i64) = sqlx::query_as(
        "SELECT value,revision FROM core_console.options WHERE key='IPAccessRoutingRules'",
    )
    .fetch_one(&mut inspect)
    .await
    .unwrap();
    assert_eq!(old_policy, ("fallback: reject".into(), 12));
    let receipts: i64 =
        sqlx::query_scalar("SELECT count(*) FROM core_meta.assistant_weekly_discount_upgrades")
            .fetch_one(&mut inspect)
            .await
            .unwrap();
    assert_eq!(receipts, 1);
    sqlx::raw_sql(sqlx::AssertSqlSafe(format!("REASSIGN OWNED BY {} TO {}; DROP OWNED BY {},{},{},{} CASCADE; ALTER DATABASE {} OWNER TO {}; DROP ROLE {},{},{},{};", quote(&request.roles.owner),quote(&original),quote(&request.roles.installer),quote(&request.roles.owner),quote(&request.roles.runtime),quote(&request.roles.extensions),quote(&request.database.name),quote(&original),quote(&request.roles.installer),quote(&request.roles.owner),quote(&request.roles.runtime),quote(&request.roles.extensions)))).execute(&mut inspect).await.unwrap();
    inspect.close().await.unwrap();
}

#[test]
fn weekly88_suffix_keeps_exact_predecessor_and_append_position() {
    assert_eq!(ENGINE.predecessor_count, 87);
    assert_eq!(ENGINE.target_count, 88);
    assert_eq!(COMPONENTS[87].0, "assistant-weekly-discounts");
    let old: serde_json::Value = serde_json::from_str(include_str!(
        "assistant_invitation_confirmation_target.json"
    ))
    .unwrap();
    assert_eq!(ENGINE.predecessor_manifest, old["manifest_sha256"]);
    assert_eq!(ENGINE.predecessor_catalog, old["catalog_sha256"]);
}

#[sqlx::test(migrations = false)]
async fn weekly88_upgrade_keeps_disabled_capabilities_and_replays(pool: PgPool) {
    exact_upgrade(pool, 0).await;
}
#[sqlx::test(migrations = false)]
async fn weekly88_upgrade_keeps_payment_reader_without_decision_or_coupon_write(pool: PgPool) {
    exact_upgrade(pool, READER).await;
}
#[sqlx::test(migrations = false)]
async fn weekly88_upgrade_keeps_browser_without_general_coupon_authority(pool: PgPool) {
    exact_upgrade(pool, BROWSER).await;
}
#[sqlx::test(migrations = false)]
async fn weekly88_upgrade_keeps_worker_without_browser_claim_authority(pool: PgPool) {
    exact_upgrade(pool, AGENT).await;
}
#[sqlx::test(migrations = false)]
async fn weekly88_upgrade_preserves_all_enabled_capabilities_and_replays(pool: PgPool) {
    exact_upgrade(pool, READER | BROWSER | AGENT).await;
}

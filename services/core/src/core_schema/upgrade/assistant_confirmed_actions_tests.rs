//! Real isolated PostgreSQL upgrade, exact replay, and preservation.
use super::super::payment_discounts::tests::engine_offline_boundary;
use super::super::{finance, request_operations::preservation};
use super::*;
use crate::core_schema::{UpgradeDatabase, UpgradeRoles, upgrade_database};
use sqlx::{Connection, PgPool, postgres::PgPoolOptions};
use std::collections::{BTreeMap, BTreeSet};

const CONFIRMATIONS: &str = "core_assistant_tools.confirmations";
const OLD_CONFIRMATION_ROWS: &str = "SELECT COALESCE(jsonb_agg(to_jsonb(q) ORDER BY q.id),'[]'::jsonb) FROM (SELECT id,operation_id,turn_id,actor_user_id,account_id,credential_id,auth_version,web_session_sid,tool_name,tool_version,arguments_sha256,draft_sha256,preview,resource_kind,resource_id,resource_version,expires_at,nonce_sha256,state,confirmed_at FROM core_assistant_tools.confirmations) q";

fn permission_object_key(fact: &str) -> Option<String> {
    let value: serde_json::Value = serde_json::from_str(fact).unwrap();
    let width = match value[0].as_str() {
        Some("schema") => 2,
        Some("function") => 3,
        Some("column") => 4,
        _ => return None,
    };
    Some(serde_json::to_string(&value.as_array().unwrap()[..width]).unwrap())
}

struct ConfirmationBaseline {
    tables: Vec<String>,
    sequences: Vec<String>,
    relations: Vec<i64>,
    types: Vec<i64>,
    permission_keys: BTreeSet<String>,
    permissions: Vec<String>,
    data: BTreeMap<String, Vec<u8>>,
    original_rows: serde_json::Value,
}
impl ConfirmationBaseline {
    async fn capture(c: &mut PgConnection) -> Self {
        let mut tx = c.begin().await.unwrap();
        sqlx::query("SET LOCAL search_path=pg_catalog")
            .execute(&mut *tx)
            .await
            .unwrap();
        let tables: Vec<String> = sqlx::query_scalar(
            "SELECT format('%I.%I',n.nspname,c.relname) FROM pg_class c JOIN pg_namespace n ON n.oid=c.relnamespace WHERE n.nspname NOT IN ('pg_catalog','information_schema') AND left(n.nspname,3)<>'pg_' AND c.relkind IN ('r','p') ORDER BY n.nspname,c.relname",
        ).fetch_all(&mut *tx).await.unwrap();
        let sequences: Vec<String> = sqlx::query_scalar(
            "SELECT format('%I.%I',n.nspname,c.relname) FROM pg_class c JOIN pg_namespace n ON n.oid=c.relnamespace WHERE n.nspname NOT IN ('pg_catalog','information_schema') AND left(n.nspname,3)<>'pg_' AND c.relkind='S' ORDER BY n.nspname,c.relname",
        ).fetch_all(&mut *tx).await.unwrap();
        let relations = finance::object_ids(&mut tx, "relation").await.unwrap();
        let types = finance::object_ids(&mut tx, "type").await.unwrap();
        let permissions = finance::permissions(&mut tx, &relations, &types)
            .await
            .unwrap();
        let permission_keys = permissions
            .iter()
            .filter_map(|f| permission_object_key(f))
            .collect();
        let data = preservation::data_confirmed_actions(&mut tx, &tables, &sequences)
            .await
            .unwrap();
        let original_rows: serde_json::Value = sqlx::query_scalar(OLD_CONFIRMATION_ROWS)
            .fetch_one(&mut *tx)
            .await
            .unwrap();
        assert_eq!(original_rows.as_array().unwrap().len(), 2);
        assert_eq!(original_rows[0]["state"], "pending");
        assert_eq!(original_rows[1]["state"], "approved");
        for row in original_rows.as_array().unwrap() {
            assert_eq!(row.as_object().unwrap().len(), 20);
        }
        tx.commit().await.unwrap();
        Self {
            tables,
            sequences,
            relations,
            types,
            permission_keys,
            permissions,
            data,
            original_rows,
        }
    }

    async fn assert_preserved(&self, c: &mut PgConnection) {
        let mut tx = c.begin().await.unwrap();
        sqlx::query("SET LOCAL search_path=pg_catalog")
            .execute(&mut *tx)
            .await
            .unwrap();
        assert_eq!(
            self.data,
            preservation::data_confirmed_actions(&mut tx, &self.tables, &self.sequences)
                .await
                .unwrap(),
            "only the three nullable execution columns may be projected out; all old table and exact sequence states must survive",
        );
        let actual_rows: serde_json::Value = sqlx::query_scalar(OLD_CONFIRMATION_ROWS)
            .fetch_one(&mut *tx)
            .await
            .unwrap();
        assert_eq!(
            actual_rows, self.original_rows,
            "every original confirmation field must survive"
        );
        let permissions: Vec<String> = finance::permissions(&mut tx, &self.relations, &self.types)
            .await
            .unwrap()
            .into_iter()
            .filter(|f| {
                permission_object_key(f).is_none_or(|key| self.permission_keys.contains(&key))
            })
            .collect();
        assert_eq!(
            self.permissions, permissions,
            "all predecessor column/relation/schema/function/type/role ACL facts must survive"
        );
        tx.commit().await.unwrap();
    }
}

async fn prefix84(pool: &PgPool) {
    use crate::core_schema as root;
    let mut tx = pool.begin().await.unwrap();
    for (_, ddl) in &COMPONENTS[..84] {
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
    for (name, ddl) in &COMPONENTS[..84] {
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

async fn exact_upgrade(pool: PgPool, enable_confirmations: bool) {
    authorize_target().unwrap();
    let facts=sqlx::query("SELECT current_database()::text AS name,d.oid::bigint AS oid,current_setting('server_version_num')::bigint AS version,(SELECT system_identifier::text FROM pg_control_system()) AS system,session_user::text AS original FROM pg_database d WHERE datname=current_database()")
        .fetch_one(&pool).await.unwrap();
    let suffix = format!("{:016x}", rand::random::<u64>());
    let request = UpgradeRequest {
        version: 1,
        procedure: PROCEDURE.into(),
        upgrade_id: format!("confirmation85-owner-{suffix}"),
        database: UpgradeDatabase {
            name: facts.get("name"),
            oid: u32::try_from(facts.get::<i64, _>("oid")).unwrap(),
            system_identifier: facts.get("system"),
            server_version_num: u32::try_from(facts.get::<i64, _>("version")).unwrap(),
        },
        roles: UpgradeRoles {
            installer: format!("confirmation85_i_{suffix}"),
            owner: format!("confirmation85_o_{suffix}"),
            runtime: format!("confirmation85_r_{suffix}"),
            extensions: format!("confirmation85_e_{suffix}"),
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
    prefix84(&installer).await;

    let identity = IdentityStore::from_pool(installer.clone());
    let original_user = identity.bootstrap_user(79001, 0).await.unwrap();
    let root = identity.bootstrap_user(79002, 6).await.unwrap();
    crate::identity::admin_users::AdminUsersStore::new(identity)
        .adjust_user_balance(
            &root.secret,
            79001,
            &crate::identity::admin_users::AdjustmentRequest {
                delta_units: "1234568".into(),
                operation_key: "confirmation85-prior-opening-operation-0001".into(),
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
    let grant:i64=sqlx::query_scalar("INSERT INTO core_charging.subscriptions(account_id,grant_reference,period,anchor,seconds,ends_at,limit_credits,models,groups) SELECT personal_account_id,'confirmation85-old-grant','grant',1,0,253402300799,1000,ARRAY['*'],ARRAY['*'] FROM core_identity.users WHERE id=79001 RETURNING id")
        .fetch_one(&mut *seed).await.unwrap();
    sqlx::query("INSERT INTO core_charging.subscription_opening_consumption(subscription_id,start_at,end_at,consumed_credits,import_reference) VALUES($1,1,253402300799,123,'confirmation85-old-opening')")
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

    // These are real predecessor rows for existing Go tools. They have a real
    // native Session/browser-family FK and never claim an executed Core action.
    let sid = "8".repeat(64);
    let generation: i64 =
        sqlx::query_scalar("SELECT auth_version FROM core_identity.users WHERE id=79001")
            .fetch_one(&installer)
            .await
            .unwrap();
    sqlx::query("INSERT INTO core_identity.web_sessions(sid,user_id,user_version,current_credential_id,refresh_digest,ip,user_agent) VALUES($1,79001,$2,$3,$4,'127.0.0.1','synthetic predecessor browser')")
        .bind(&sid).bind(generation).bind(original_user.id).bind(vec![8_u8;32])
        .execute(&installer).await.unwrap();
    for (index, tool, state, confirmed_at) in [
        (0_usize, "create_site_issue", "pending", None),
        (1_usize, "set_overview_greeting", "approved", Some(2_i64)),
    ] {
        let id = if index == 0 {
            "a".repeat(64)
        } else {
            "b".repeat(64)
        };
        let operation = if index == 0 {
            "c".repeat(64)
        } else {
            "d".repeat(64)
        };
        let nonce = confirmed_at.map(|_| vec![9_u8; 32]);
        sqlx::query("INSERT INTO core_assistant_tools.confirmations(id,operation_id,turn_id,actor_user_id,account_id,credential_id,auth_version,web_session_sid,tool_name,tool_version,arguments_sha256,draft_sha256,preview,resource_kind,resource_id,resource_version,expires_at,nonce_sha256,state,confirmed_at) VALUES($1,$2,$3,79001,$4,$5,$6,$7,$8,'synthetic-predecessor-go-tool-v1',$9,$10,$11,'synthetic-go-resource',$12,'7',253402300799,$13,$14,$15)")
            .bind(&id).bind(operation).bind("e".repeat(64)).bind(owner)
            .bind(original_user.id).bind(generation).bind(&sid).bind(tool)
            .bind(vec![1_u8;32]).bind(vec![2_u8;32])
            .bind(serde_json::json!({"kind":"synthetic_predecessor_go","ordinal":index}))
            .bind(format!("old-resource-{index}")).bind(nonce).bind(state).bind(confirmed_at)
            .execute(&installer).await.unwrap();
    }
    if enable_confirmations {
        sqlx::raw_sql(sqlx::AssertSqlSafe(format!(
            "GRANT USAGE ON SCHEMA core_assistant_tools TO {role}; GRANT SELECT,INSERT ON core_assistant_tools.confirmations TO {role}; GRANT UPDATE(nonce_sha256,state,confirmed_at) ON core_assistant_tools.confirmations TO {role}",
            role=quote(&request.roles.runtime)
        ))).execute(&installer).await.unwrap();
    }
    let mut c = installer.acquire().await.unwrap();
    let baseline = ConfirmationBaseline::capture(&mut c).await;
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
    let mut rejected = PgConnection::connect_with(&options).await.unwrap();
    baseline.assert_preserved(&mut rejected).await;
    let untouched: (i64, bool) = sqlx::query_as(
        "SELECT (SELECT count(*) FROM core_meta.component_contract),to_regclass('core_meta.assistant_confirmed_actions_upgrades') IS NULL",
    ).fetch_one(&mut rejected).await.unwrap();
    assert_eq!(
        untouched,
        (84, true),
        "wrong pin must not append any schema or receipt"
    );
    rejected.close().await.unwrap();
    engine_offline_boundary(&request).await;
    let report = upgrade_database(url.as_str(), request.clone())
        .await
        .unwrap();
    assert_eq!(report.status, "committed");
    assert_eq!(report.components, 85);
    assert_eq!(report.request_sha256, hex(request.digest().unwrap()));
    let target: serde_json::Value = serde_json::from_str(TARGET).unwrap();
    assert_eq!(report.successor_catalog_sha256, target["catalog_sha256"]);
    engine_offline_boundary(&request).await;
    let report = upgrade_database(url.as_str(), request.clone())
        .await
        .unwrap();
    assert_eq!(report.status, "already_committed");
    let mut inspect = PgConnection::connect_with(&options).await.unwrap();
    baseline.assert_preserved(&mut inspect).await;

    let owner_only: bool = sqlx::query_scalar(
        "SELECT NOT has_table_privilege($1,'core_meta.assistant_confirmed_actions_upgrades','SELECT,INSERT,UPDATE,DELETE,TRUNCATE,REFERENCES,TRIGGER,MAINTAIN') AND NOT has_any_column_privilege($1,'core_meta.assistant_confirmed_actions_upgrades','SELECT,INSERT,UPDATE,REFERENCES') AND has_table_privilege($1,'core_console.options','SELECT')",
    ).bind(&request.roles.runtime).fetch_one(&mut inspect).await.unwrap();
    assert!(owner_only);
    let all_unexecuted: bool = sqlx::query_scalar(
        "SELECT count(*)=2 AND bool_and(execution_receipt IS NULL AND execution_sha256 IS NULL AND executed_at IS NULL) FROM core_assistant_tools.confirmations",
    ).fetch_one(&mut inspect).await.unwrap();
    assert!(
        all_unexecuted,
        "neither pending nor approved old Go consent is a Core execution"
    );
    let nullable: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM pg_attribute WHERE attrelid='core_assistant_tools.confirmations'::regclass AND attname IN ('execution_receipt','execution_sha256','executed_at') AND NOT attnotnull AND NOT attisdropped",
    ).fetch_one(&mut inspect).await.unwrap();
    assert_eq!(nullable, 3);
    for column in ["execution_receipt", "execution_sha256", "executed_at"] {
        let allowed: bool = sqlx::query_scalar(
            "SELECT has_column_privilege($1,'core_assistant_tools.confirmations',$2,'UPDATE')",
        )
        .bind(&request.roles.runtime)
        .bind(column)
        .fetch_one(&mut inspect)
        .await
        .unwrap();
        assert_eq!(allowed, enable_confirmations, "runtime UPDATE({column})");
        let extensions_allowed: bool = sqlx::query_scalar(
            "SELECT has_column_privilege($1,'core_assistant_tools.confirmations',$2,'UPDATE')",
        )
        .bind(&request.roles.extensions)
        .bind(column)
        .fetch_one(&mut inspect)
        .await
        .unwrap();
        assert!(
            !extensions_allowed,
            "the Go role receives no Core execution authority"
        );
    }
    let card_authority: (bool, bool, i64, bool, bool, bool) = sqlx::query_as(
        "SELECT has_table_privilege($1,'core_assistant_tools.key_creation_cards','SELECT'),
          has_table_privilege($1,'core_assistant_tools.key_creation_cards','INSERT'),
          (SELECT count(*) FROM unnest(ARRAY['nonce_sha256','ciphertext','consumed_at']) c(name)
            WHERE has_column_privilege($1,'core_assistant_tools.key_creation_cards',c.name,'UPDATE')),
          has_function_privilege($1,'core_assistant_tools.market_confirmation_bound(text)','EXECUTE'),
          has_table_privilege($1,'core_assistant_tools.key_creation_cards','UPDATE,DELETE,TRUNCATE,REFERENCES,TRIGGER,MAINTAIN'),
          has_table_privilege($2,'core_assistant_tools.key_creation_cards','SELECT,INSERT,UPDATE,DELETE,TRUNCATE,REFERENCES,TRIGGER,MAINTAIN')",
    ).bind(&request.roles.runtime).bind(&request.roles.extensions)
        .fetch_one(&mut inspect).await.unwrap();
    assert_eq!(
        card_authority,
        (
            enable_confirmations,
            enable_confirmations,
            if enable_confirmations { 3 } else { 0 },
            enable_confirmations,
            false,
            false,
        ),
        "only an already enabled coordinator receives the exact card and market proof authority"
    );
    let card_count: i64 =
        sqlx::query_scalar("SELECT count(*) FROM core_assistant_tools.key_creation_cards")
            .fetch_one(&mut inspect)
            .await
            .unwrap();
    assert_eq!(card_count, 0, "upgrade creates no API keys or reveal cards");

    let table_update: bool = sqlx::query_scalar(
        "SELECT has_table_privilege($1,'core_assistant_tools.confirmations','UPDATE,DELETE,TRUNCATE,REFERENCES,TRIGGER,MAINTAIN')",
    ).bind(&request.roles.runtime).fetch_one(&mut inspect).await.unwrap();
    assert!(!table_update, "new execution grants remain column-scoped");
    let unexpected_acl: bool = sqlx::query_scalar(
        "SELECT EXISTS(SELECT FROM pg_attribute a JOIN pg_class c ON c.oid=a.attrelid CROSS JOIN LATERAL aclexplode(a.attacl) x WHERE a.attrelid='core_assistant_tools.confirmations'::regclass AND a.attname IN ('execution_receipt','execution_sha256','executed_at') AND (x.grantee<>(SELECT oid FROM pg_roles WHERE rolname=$1) OR x.privilege_type<>'UPDATE' OR x.is_grantable OR x.grantor<>c.relowner))",
    ).bind(&request.roles.runtime).fetch_one(&mut inspect).await.unwrap();
    assert!(!unexpected_acl);

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
    let current_history: String = sqlx::query_scalar(
        "SELECT pg_get_functiondef('core_history.guard_current_connection_snapshot()'::regprocedure)",
    ).fetch_one(&mut inspect).await.unwrap();
    assert!(current_history.contains("component_contract)<>84"));

    let old_policy: (String, i64) = sqlx::query_as(
        "SELECT value,revision FROM core_console.options WHERE key='IPAccessRoutingRules'",
    )
    .fetch_one(&mut inspect)
    .await
    .unwrap();
    assert_eq!(old_policy, ("fallback: reject".into(), 12));
    let receipts: i64 =
        sqlx::query_scalar("SELECT count(*) FROM core_meta.assistant_confirmed_actions_upgrades")
            .fetch_one(&mut inspect)
            .await
            .unwrap();

    assert_eq!(receipts, 1);
    let receipt: (Vec<u8>, serde_json::Value, Vec<u8>, Vec<u8>) = sqlx::query_as(
        "SELECT request_sha256,request,preserved_data_sha256,successor_catalog_sha256 FROM core_meta.assistant_confirmed_actions_upgrades WHERE upgrade_id=$1",
    ).bind(&request.upgrade_id).fetch_one(&mut inspect).await.unwrap();
    assert_eq!(receipt.0, request.digest().unwrap());
    assert_eq!(receipt.1, serde_json::to_value(&request).unwrap());
    assert_eq!(receipt.2, finance::data_hash(&baseline.data));
    assert_eq!(hex(&receipt.3), target["catalog_sha256"]);
    let observed_catalog = hex(fingerprint(&definitions(&mut inspect).await.unwrap()));
    assert_eq!(observed_catalog, target["catalog_sha256"]);
    let recorded: String = sqlx::query_scalar(
        "SELECT encode(catalog_fingerprint,'hex') FROM core_meta.runtime_contract WHERE singleton",
    )
    .fetch_one(&mut inspect)
    .await
    .unwrap();
    assert_eq!(recorded, observed_catalog);

    // A legitimate old-field update must change preservation evidence. Do not
    // disable a trigger or manufacture a new execution receipt to test this.
    let mut changed = inspect.begin().await.unwrap();
    sqlx::query("SET LOCAL search_path=pg_catalog")
        .execute(&mut *changed)
        .await
        .unwrap();
    let count = sqlx::query(
        "UPDATE core_assistant_tools.confirmations SET nonce_sha256=$1 WHERE id=$2 AND state='pending'",
    ).bind(vec![6_u8;32]).bind("a".repeat(64)).execute(&mut *changed).await.unwrap().rows_affected();
    assert_eq!(count, 1);
    let changed_data =
        preservation::data_confirmed_actions(&mut changed, &baseline.tables, &baseline.sequences)
            .await
            .unwrap();
    assert_ne!(
        changed_data[CONFIRMATIONS], baseline.data[CONFIRMATIONS],
        "the explicit nullable-column projection must not hide mutations of any predecessor field",
    );
    changed.rollback().await.unwrap();
    baseline.assert_preserved(&mut inspect).await;
    inspect.close().await.unwrap();

    let mut runtime = PgConnection::connect_with(&options).await.unwrap();
    sqlx::query(sqlx::AssertSqlSafe(format!(
        "SET SESSION AUTHORIZATION {}",
        quote(&request.roles.runtime),
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
        (request.roles.runtime.clone(), request.roles.runtime.clone())
    );
    let new_columns = sqlx::query(
        "UPDATE core_assistant_tools.confirmations SET execution_receipt=execution_receipt,execution_sha256=execution_sha256,executed_at=executed_at WHERE false",
    ).execute(&mut runtime).await;
    if enable_confirmations {
        assert_eq!(
            new_columns.unwrap().rows_affected(),
            0,
            "new column grants are usable by the actual runtime role"
        );
    } else {
        assert_eq!(
            new_columns
                .unwrap_err()
                .as_database_error()
                .and_then(|e| e.code())
                .as_deref(),
            Some("42501"),
        );
    }
    for denied in [
        "UPDATE core_assistant_tools.confirmations SET preview=preview WHERE false",
        "SELECT * FROM core_meta.assistant_confirmed_actions_upgrades",
    ] {
        let error = sqlx::query(denied).execute(&mut runtime).await.unwrap_err();
        assert_eq!(
            error.as_database_error().and_then(|e| e.code()).as_deref(),
            Some("42501")
        );
    }
    runtime.close().await.unwrap();
    engine_offline_boundary(&request).await;
    let final_replay = upgrade_database(url.as_str(), request.clone())
        .await
        .unwrap();
    assert_eq!(final_replay.status, "already_committed");
    let mut inspect = PgConnection::connect_with(&options).await.unwrap();
    baseline.assert_preserved(&mut inspect).await;
    sqlx::raw_sql(sqlx::AssertSqlSafe(format!("REASSIGN OWNED BY {} TO {}; DROP OWNED BY {},{},{},{} CASCADE; ALTER DATABASE {} OWNER TO {}; DROP ROLE {},{},{},{};", quote(&request.roles.owner),quote(&original),quote(&request.roles.installer),quote(&request.roles.owner),quote(&request.roles.runtime),quote(&request.roles.extensions),quote(&request.database.name),quote(&original),quote(&request.roles.installer),quote(&request.roles.owner),quote(&request.roles.runtime),quote(&request.roles.extensions)))).execute(&mut inspect).await.unwrap();
    inspect.close().await.unwrap();
}

#[test]
fn confirmation85_suffix_keeps_exact_predecessor_and_append_position() {
    assert_eq!(ENGINE.predecessor_count, 84);
    assert_eq!(ENGINE.target_count, 85);
    assert_eq!(COMPONENTS[84].0, "assistant-confirmed-actions");
    let old: serde_json::Value =
        serde_json::from_str(include_str!("current_connection_history_target.json")).unwrap();
    assert_eq!(ENGINE.predecessor_manifest, old["manifest_sha256"]);
    assert_eq!(ENGINE.predecessor_catalog, old["catalog_sha256"]);
    assert_eq!(
        ENGINE.target_manifest().unwrap(),
        "2c4563c35a29fe5c0d579c5d0ea41e9300a3027343fbae641ad098d5524e8d80"
    );
    let target: serde_json::Value = serde_json::from_str(TARGET).unwrap();
    assert_eq!(
        target["catalog_sha256"],
        "1890e9f888a13b535dd653bffe5771745c6e4e44421211ae6723fcffedd64462"
    );
}

#[sqlx::test(migrations = false)]
async fn confirmation85_upgrade_preserves_pending_and_approved_with_enabled_runtime(pool: PgPool) {
    exact_upgrade(pool, true).await;
}

#[sqlx::test(migrations = false)]
async fn confirmation85_upgrade_preserves_disabled_runtime_without_execution_rights(pool: PgPool) {
    exact_upgrade(pool, false).await;
}

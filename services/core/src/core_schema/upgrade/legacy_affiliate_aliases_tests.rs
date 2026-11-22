//! Real isolated PostgreSQL upgrade, exact replay, and preservation.
use super::super::payment_discounts::tests::{EngineBaseline, engine_offline_boundary};
use super::*;
use crate::core_schema::{UpgradeDatabase, UpgradeRoles, upgrade_database};
use sqlx::{PgPool, postgres::PgPoolOptions};
async fn prefix85(pool: &PgPool) {
    use crate::core_schema as root;
    let mut tx = pool.begin().await.unwrap();
    for (_, ddl) in &COMPONENTS[..85] {
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
    for (name, ddl) in &COMPONENTS[..85] {
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

async fn exact_upgrade(pool: PgPool, enable_aliases: bool) {
    authorize_target().unwrap();
    let facts=sqlx::query("SELECT current_database()::text AS name,d.oid::bigint AS oid,current_setting('server_version_num')::bigint AS version,(SELECT system_identifier::text FROM pg_control_system()) AS system,session_user::text AS original FROM pg_database d WHERE datname=current_database()")
        .fetch_one(&pool).await.unwrap();
    let suffix = format!("{:016x}", rand::random::<u64>());
    let request = UpgradeRequest {
        version: 1,
        procedure: PROCEDURE.into(),
        upgrade_id: format!("aliases86-owner-{suffix}"),
        database: UpgradeDatabase {
            name: facts.get("name"),
            oid: u32::try_from(facts.get::<i64, _>("oid")).unwrap(),
            system_identifier: facts.get("system"),
            server_version_num: u32::try_from(facts.get::<i64, _>("version")).unwrap(),
        },
        roles: UpgradeRoles {
            installer: format!("aliases86_i_{suffix}"),
            owner: format!("aliases86_o_{suffix}"),
            runtime: format!("aliases86_r_{suffix}"),
            extensions: format!("aliases86_e_{suffix}"),
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
    prefix85(&installer).await;

    let identity = IdentityStore::from_pool(installer.clone());
    identity.bootstrap_user(79001, 0).await.unwrap();
    let root = identity.bootstrap_user(79002, 6).await.unwrap();
    crate::identity::admin_users::AdminUsersStore::new(identity)
        .adjust_user_balance(
            &root.secret,
            79001,
            &crate::identity::admin_users::AdjustmentRequest {
                delta_units: "1234568".into(),
                operation_key: "aliases86-prior-opening-operation-0001".into(),
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
    let grant:i64=sqlx::query_scalar("INSERT INTO core_charging.subscriptions(account_id,grant_reference,period,anchor,seconds,ends_at,limit_credits,models,groups) SELECT personal_account_id,'aliases86-old-grant','grant',1,0,253402300799,1000,ARRAY['*'],ARRAY['*'] FROM core_identity.users WHERE id=79001 RETURNING id")
        .fetch_one(&mut *seed).await.unwrap();
    sqlx::query("INSERT INTO core_charging.subscription_opening_consumption(subscription_id,start_at,end_at,consumed_credits,import_reference) VALUES($1,1,253402300799,123,'aliases86-old-opening')")
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
    if enable_aliases {
        sqlx::raw_sql(sqlx::AssertSqlSafe(format!(
            "GRANT USAGE ON SCHEMA core_identity TO {role}; GRANT SELECT,INSERT ON core_identity.affiliate_codes,core_identity.affiliate_registrations TO {role}",
            role=quote(&request.roles.runtime)
        ))).execute(&installer).await.unwrap();
    }
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
    assert_eq!(report.components, 86);
    engine_offline_boundary(&request).await;
    let report = upgrade_database(url.as_str(), request.clone())
        .await
        .unwrap();
    assert_eq!(report.status, "already_committed");
    let mut inspect = PgConnection::connect_with(&options).await.unwrap();
    baseline.assert_preserved(&mut inspect).await;
    let owner_only:bool=sqlx::query_scalar("SELECT NOT has_table_privilege($1,'core_meta.legacy_affiliate_alias_upgrades','SELECT,INSERT,UPDATE,DELETE,TRUNCATE') AND has_table_privilege($1,'core_console.options','SELECT')")
        .bind(&request.roles.runtime).fetch_one(&mut inspect).await.unwrap();
    assert!(owner_only);
    for column in ["code", "user_id"] {
        let allowed: bool = sqlx::query_scalar(
            "SELECT has_column_privilege($1,'core_identity.legacy_affiliate_aliases',$2,'SELECT')",
        )
        .bind(&request.roles.runtime)
        .bind(column)
        .fetch_one(&mut inspect)
        .await
        .unwrap();
        assert_eq!(allowed, enable_aliases);
    }
    let unsafe_authority: bool = sqlx::query_scalar(
        "SELECT has_table_privilege($1,'core_identity.legacy_affiliate_aliases','INSERT,UPDATE,DELETE,TRUNCATE,REFERENCES,TRIGGER,MAINTAIN')
          OR has_any_column_privilege($1,'core_identity.legacy_affiliate_aliases','INSERT,UPDATE,REFERENCES')
          OR EXISTS(SELECT FROM unnest(ARRAY['source_user_id','source_row_sha256','import_id']) c(name)
             WHERE has_column_privilege($1,'core_identity.legacy_affiliate_aliases',c.name,'SELECT'))
          OR has_any_column_privilege($1,'core_meta.legacy_affiliate_alias_imports','SELECT,INSERT,UPDATE,REFERENCES')
          OR has_table_privilege($2,'core_identity.legacy_affiliate_aliases','SELECT,INSERT,UPDATE,DELETE,TRUNCATE,REFERENCES,TRIGGER,MAINTAIN')
          OR has_any_column_privilege($2,'core_identity.legacy_affiliate_aliases','SELECT,INSERT,UPDATE,REFERENCES')",
    ).bind(&request.roles.runtime).bind(&request.roles.extensions)
        .fetch_one(&mut inspect).await.unwrap();
    assert!(
        !unsafe_authority,
        "runtime and Go receive no import or source-record authority"
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
        "native invitation codes remain available after appending legacy aliases",
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
        sqlx::query_scalar("SELECT count(*) FROM core_meta.legacy_affiliate_alias_upgrades")
            .fetch_one(&mut inspect)
            .await
            .unwrap();
    assert_eq!(receipts, 1);
    sqlx::raw_sql(sqlx::AssertSqlSafe(format!("REASSIGN OWNED BY {} TO {}; DROP OWNED BY {},{},{},{} CASCADE; ALTER DATABASE {} OWNER TO {}; DROP ROLE {},{},{},{};", quote(&request.roles.owner),quote(&original),quote(&request.roles.installer),quote(&request.roles.owner),quote(&request.roles.runtime),quote(&request.roles.extensions),quote(&request.database.name),quote(&original),quote(&request.roles.installer),quote(&request.roles.owner),quote(&request.roles.runtime),quote(&request.roles.extensions)))).execute(&mut inspect).await.unwrap();
    inspect.close().await.unwrap();
}

#[test]
fn aliases86_suffix_keeps_exact_predecessor_and_append_position() {
    assert_eq!(ENGINE.predecessor_count, 85);
    assert_eq!(ENGINE.target_count, 86);
    assert_eq!(COMPONENTS[85].0, "legacy-affiliate-aliases");
    let old: serde_json::Value =
        serde_json::from_str(include_str!("assistant_confirmed_actions_target.json")).unwrap();
    assert_eq!(ENGINE.predecessor_manifest, old["manifest_sha256"]);
    assert_eq!(ENGINE.predecessor_catalog, old["catalog_sha256"]);
}

#[sqlx::test(migrations = false)]
async fn aliases86_upgrade_preserves_enabled_reader_and_replays(pool: PgPool) {
    exact_upgrade(pool, true).await;
}

#[sqlx::test(migrations = false)]
async fn aliases86_upgrade_keeps_disabled_reader_without_new_privileges(pool: PgPool) {
    exact_upgrade(pool, false).await;
}

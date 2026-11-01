//! Frozen 41-component setup for a synthetic, empty SQLx database only.
use sha2::{Digest, Sha256};
use sqlx::PgPool;
const COMPONENTS: &[(&str, &str)] = &[
    ("humangrant", include_str!("../../schema/humangrant.sql")),
    ("identity", include_str!("../../schema/identity.sql")),
    ("ledger", include_str!("../../schema/ledger.sql")),
    ("payments", include_str!("../../schema/payments.sql")),
    ("charging", include_str!("../../schema/charging.sql")),
    ("routing", include_str!("../../schema/routing.sql")),
    ("events", include_str!("../../schema/events.sql")),
    (
        "charging-ledger",
        include_str!("../../schema/charging_ledger.sql"),
    ),
    ("migration", include_str!("../../schema/migration.sql")),
    ("console", include_str!("../../schema/console.sql")),
    (
        "model-gateway",
        include_str!("../../schema/model_gateway.sql"),
    ),
    ("commerce", include_str!("../../schema/commerce.sql")),
    ("tool-market", include_str!("../../schema/tool_market.sql")),
    ("abuse", include_str!("../../schema/abuse.sql")),
    (
        "oauth-grants",
        include_str!("../../schema/oauth_grants.sql"),
    ),
    ("history", include_str!("../../schema/history.sql")),
    (
        "assistant-model",
        include_str!("../../schema/assistant_model.sql"),
    ),
    ("gift", include_str!("../../schema/gift.sql")),
    ("image", include_str!("../../schema/image.sql")),
    (
        "market-oauth",
        include_str!("../../schema/market_oauth.sql"),
    ),
    (
        "tool-market-credit",
        include_str!("../../schema/tool_market_credit.sql"),
    ),
    ("wallet", include_str!("../../schema/wallet.sql")),
    ("bounties", include_str!("../../schema/bounties.sql")),
    (
        "assistant-tools",
        include_str!("../../schema/assistant_tools.sql"),
    ),
    (
        "trust-pricing",
        concat!(
            include_str!("../../schema/trust_pricing.sql"),
            "\n",
            include_str!("../../schema/image_trust_extension.sql")
        ),
    ),
    (
        "telegram-auth",
        include_str!("../../schema/telegram_auth.sql"),
    ),
    (
        "assistant-agent",
        include_str!("../../schema/assistant_agent.sql"),
    ),
    (
        "site-finance",
        include_str!("../../schema/site_statistics.sql"),
    ),
    (
        "payment-gateways",
        include_str!("../../schema/payment_gateways.sql"),
    ),
    (
        "request-limits",
        include_str!("../../schema/request_limits.sql"),
    ),
    ("operations", include_str!("../../schema/operations.sql")),
    (
        "coinbase-payments",
        include_str!("../../schema/coinbase_payments.sql"),
    ),
    (
        "channel-control",
        include_str!("../../schema/channel_control.sql"),
    ),
    (
        "channel-vault",
        include_str!("../../schema/channel_vault.sql"),
    ),
    ("passkeys", include_str!("../../schema/passkeys.sql")),
    (
        "account-lifecycle",
        include_str!("../../schema/account_lifecycle.sql"),
    ),
    (
        "subscriptions",
        include_str!("../../schema/subscriptions.sql"),
    ),
    ("promotions", include_str!("../../schema/promotions.sql")),
    (
        "notifications",
        include_str!("../../schema/notifications.sql"),
    ),
    (
        "account-links",
        include_str!("../../schema/account_links.sql"),
    ),
    (
        "completion-upgrade",
        include_str!("../../schema/completion_upgrade.sql"),
    ),
];
const OAUTH_CATALOG: &str = r#"
WITH catalog_path AS MATERIALIZED (
 SELECT pg_catalog.set_config('search_path','pg_catalog',true)
), oauth_namespace AS MATERIALIZED (
 SELECT n.oid FROM pg_namespace n CROSS JOIN catalog_path WHERE n.nspname='core_oauth'
), definitions AS (
 SELECT json_build_array('relation',c.relname,c.relkind)::text AS d
 FROM pg_class c JOIN pg_namespace n ON n.oid=c.relnamespace WHERE n.oid IN (SELECT oid FROM oauth_namespace)
 UNION ALL
 SELECT json_build_array('column',c.relname,a.attnum,a.attname,format_type(a.atttypid,a.atttypmod),a.attnotnull,a.attidentity,a.attgenerated,pg_get_expr(ad.adbin,ad.adrelid))::text
 FROM pg_attribute a JOIN pg_class c ON c.oid=a.attrelid JOIN pg_namespace n ON n.oid=c.relnamespace LEFT JOIN pg_attrdef ad ON ad.adrelid=a.attrelid AND ad.adnum=a.attnum WHERE n.oid IN (SELECT oid FROM oauth_namespace) AND a.attnum>0 AND NOT a.attisdropped
 UNION ALL
 SELECT json_build_array('constraint',k.conname,pg_get_constraintdef(k.oid,true))::text FROM pg_constraint k JOIN pg_namespace n ON n.oid=k.connamespace WHERE n.oid IN (SELECT oid FROM oauth_namespace)
 UNION ALL
 SELECT json_build_array('index',c.relname,pg_get_indexdef(i.indexrelid))::text FROM pg_index i JOIN pg_class c ON c.oid=i.indexrelid JOIN pg_namespace n ON n.oid=c.relnamespace WHERE n.oid IN (SELECT oid FROM oauth_namespace)
 UNION ALL
 SELECT json_build_array('function',p.proname,pg_get_functiondef(p.oid))::text FROM pg_proc p JOIN pg_namespace n ON n.oid=p.pronamespace WHERE n.oid IN (SELECT oid FROM oauth_namespace) AND p.prokind IN ('f','p')
 UNION ALL
 SELECT json_build_array('trigger',c.relname,t.tgname,t.tgenabled,pg_get_triggerdef(t.oid,true))::text FROM pg_trigger t JOIN pg_class c ON c.oid=t.tgrelid JOIN pg_namespace n ON n.oid=c.relnamespace WHERE n.oid IN (SELECT oid FROM oauth_namespace) AND NOT t.tgisinternal
 UNION ALL
 SELECT json_build_array('internal-trigger',c.relname,k.conname,pn.nspname,p.proname,t.tgtype,t.tgenabled,t.tgdeferrable,t.tginitdeferred,t.tgattr::text,encode(t.tgargs,'hex'),pg_get_expr(t.tgqual,t.tgrelid))::text FROM pg_trigger t JOIN pg_class c ON c.oid=t.tgrelid JOIN pg_namespace n ON n.oid=c.relnamespace JOIN pg_proc p ON p.oid=t.tgfoid JOIN pg_namespace pn ON pn.oid=p.pronamespace LEFT JOIN pg_constraint k ON k.oid=t.tgconstraint WHERE n.oid IN (SELECT oid FROM oauth_namespace) AND t.tgisinternal
) SELECT d FROM definitions ORDER BY d COLLATE "C"
"#;
fn digest(definitions: Vec<String>) -> Vec<u8> {
    let mut hash = Sha256::new();
    for definition in definitions {
        hash.update((definition.len() as u64).to_be_bytes());
        hash.update(definition.as_bytes());
    }
    hash.finalize().to_vec()
}
pub async fn install(pool: &PgPool) {
    let mut tx = pool.begin().await.unwrap();
    for (_, ddl) in COMPONENTS {
        sqlx::raw_sql(ddl).execute(&mut *tx).await.unwrap();
    }
    sqlx::raw_sql("CREATE TABLE core_meta.component_contract (name TEXT PRIMARY KEY, fingerprint BYTEA NOT NULL CHECK (octet_length(fingerprint)=32)); CREATE TABLE core_meta.runtime_contract (singleton BOOLEAN PRIMARY KEY DEFAULT TRUE CHECK (singleton), catalog_fingerprint BYTEA NOT NULL CHECK (octet_length(catalog_fingerprint)=32));").execute(&mut *tx).await.unwrap();
    for ddl in [
        include_str!("../../schema/core_upgrade.sql"),
        include_str!("../../schema/component_upgrade.sql"),
        include_str!("../../schema/ledger_revision.sql"),
        include_str!("../../src/core_schema/upgrade/console_receipt.sql"),
        include_str!("../../schema/offline_upgrade.sql"),
    ] {
        sqlx::raw_sql(ddl).execute(&mut *tx).await.unwrap();
    }
    let manifest = lmm_core::core_schema::schema_manifest();
    sqlx::query(
        "INSERT INTO core_meta.schema_contract(version,fingerprint) VALUES($1,decode($2,'hex'))",
    )
    .bind(manifest.identity.version)
    .bind(&manifest.identity.fingerprint_sha256)
    .execute(&mut *tx)
    .await
    .unwrap();
    let oauth = digest(
        sqlx::query_scalar(OAUTH_CATALOG)
            .fetch_all(&mut *tx)
            .await
            .unwrap(),
    );
    sqlx::query("INSERT INTO core_oauth.contract(version,ddl_fingerprint,catalog_fingerprint) VALUES(1,$1,$2)").bind(Sha256::digest(include_str!("../../schema/oauth_grants.sql").as_bytes()).to_vec()).bind(oauth).execute(&mut *tx).await.unwrap();
    for (name, ddl) in COMPONENTS {
        sqlx::query("INSERT INTO core_meta.component_contract(name,fingerprint) VALUES($1,$2)")
            .bind(name)
            .bind(Sha256::digest(ddl.as_bytes()).to_vec())
            .execute(&mut *tx)
            .await
            .unwrap();
    }
    let catalog = digest(
        sqlx::query_scalar(manifest.catalog.query_utf8)
            .fetch_all(&mut *tx)
            .await
            .unwrap(),
    );
    sqlx::query("INSERT INTO core_meta.runtime_contract(catalog_fingerprint) VALUES($1)")
        .bind(catalog)
        .execute(&mut *tx)
        .await
        .unwrap();
    tx.commit().await.unwrap();
}

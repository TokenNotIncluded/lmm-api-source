//! Standalone development harness: real frozen Core library and private synthetic PG.
//! This is separate from the cfg(test) leaf in the Core library.
use axum::{
    Router,
    body::{Body, to_bytes},
    http::{Request, StatusCode, header},
    response::Response,
};
use lmm_core::{
    accounts::{Account, AccountKind},
    funding::FundingPolicy,
    identity::{self as permissions, IdentityStore},
    routing::{
        CredentialRef, FailoverPolicy, ModelGroup, ModelRoute, PayloadPolicy, PriceConfig,
        PriceRule, PriceSnapshot, Protocol, RouteTarget, RoutingConfig, Speed, TokenRates,
        UpstreamConfig,
    },
};
use serde_json::Value;
use sha2::{Digest, Sha256};
use sqlx::{
    PgPool,
    postgres::{PgConnectOptions, PgPoolOptions},
};
use std::{str::FromStr, sync::Arc};
use tower::ServiceExt;

async fn request(app: Router, path: &str, secret: Option<&str>, method: &str) -> Response {
    let mut req = Request::builder().uri(path).method(method);
    if let Some(secret) = secret {
        req = req.header(header::AUTHORIZATION, format!("Bearer {secret}"));
    }
    app.oneshot(req.body(Body::empty()).unwrap()).await.unwrap()
}
fn fixture() -> (RoutingConfig, PriceConfig) {
    let config = RoutingConfig {
        version: 1,
        groups: vec![ModelGroup {
            name: "default".into(),
            enabled: true,
        }],
        upstreams: (1..=21)
            .map(|id| UpstreamConfig {
                id,
                endpoint: "https://private-upstream.invalid/v1"
                    .to_owned()
                    .try_into()
                    .unwrap(),
                protocol: Protocol::OpenAiChat,
                credential: CredentialRef {
                    id: 999_000 + id,
                    revision: 53,
                },
                enabled: true,
            })
            .collect(),
        routes: vec![ModelRoute {
            id: 1,
            model: "model-a".into(),
            group: "default".into(),
            enabled: true,
            speeds: vec![Speed::Standard],
            targets: vec![RouteTarget {
                id: 1,
                upstream_id: 1,
                upstream_model: "private-model-name".into(),
                enabled: true,
                priority: 3,
                weight: 1,
                speeds: vec![Speed::Standard],
                payload: PayloadPolicy::Adapt,
                timeout_ms: 30000,
            }],
            failover: FailoverPolicy::default(),
        }],
        aliases: vec![],
    };
    let prices = PriceConfig {
        version: 1,
        rules: vec![PriceRule {
            billing_mode: lmm_core::routing::BillingMode::Tokens,
            tiered: None,
            id: 1,
            model: "model-a".into(),
            group: "default".into(),
            speed: Speed::Standard,
            rates: TokenRates {
                input: 1,
                output: 2,
                cache_read: 0,
                cache_write: 0,
                request: 0,
            },
            group_multiplier_ppm: 1_000_000,
            speed_multiplier_ppm: 1_000_000,
        }],
    };
    (config, prices)
}

fn serving(config: RoutingConfig, prices: PriceConfig) -> Arc<lmm_core::routing::Router> {
    Arc::new(
        lmm_core::routing::Router::new(config, Arc::new(PriceSnapshot::compile(prices).unwrap()))
            .unwrap(),
    )
}

const READS: [&str; 5] = [
    "/api/channel",
    "/api/channel/",
    "/api/channel/search?keyword=21",
    "/api/channel/ops",
    "/api/channel/1",
];

async fn policy(pool: &PgPool, user: i64, read: &str, complete: bool, valid_hash: bool) {
    let entries: Vec<_> = permissions::Action::CATALOG
        .iter()
        .map(|action| {
            (
                action.catalog_name().unwrap().to_owned(),
                if *action == permissions::Action::ChannelRead {
                    read
                } else {
                    "allow"
                }
                .to_owned(),
            )
        })
        .collect();
    let snapshot = permissions::complete_policy_snapshot(user, 1, &entries).unwrap();
    let payload = permissions::canonical_policy_payload(&snapshot).unwrap();
    let mut hash = permissions::policy_sha256(&snapshot).unwrap();
    if !valid_hash {
        hash[0] ^= 1;
    }
    let mut tx = pool.begin().await.unwrap();
    sqlx::query("INSERT INTO core_identity.permission_policies(user_id,revision,contract,sha256,canonical_payload,verified_complete) VALUES ($1,1,$2,$3,$4,TRUE)")
        .bind(user).bind(permissions::PERMISSIONS_CONTRACT).bind(hash.as_slice()).bind(payload)
        .execute(&mut *tx).await.unwrap();
    for (index, (action, effect)) in entries.iter().enumerate() {
        if !complete && index == 8 {
            continue;
        }
        sqlx::query("INSERT INTO core_identity.permission_policy_rows(user_id,action,effect) VALUES ($1,$2,$3)")
            .bind(user).bind(action).bind(effect).execute(&mut *tx).await.unwrap();
    }
    tx.commit().await.unwrap();
}

async fn imported(pool: &PgPool, user: i64, role: &str) {
    let hash = [7_u8; 32];
    sqlx::query("INSERT INTO core_identity.privileged_import_facts(user_id,archive_sha256,a2_semantic_sha256,a2_file_sha256,a2_sql_sha256,migration_id,system_id,database_oid,instance_nonce,import_run,source_enabled,source_deleted,password_verifier_recognized,password_login_enabled,password_hash_sha256,login_name_sha256,source_role,source_second_factor_required,source_auth_version) VALUES ($1,$2,$2,$2,$2,$3,$2,1,$2,$2,TRUE,FALSE,TRUE,TRUE,$2,$2,$4,FALSE,1)")
        .bind(user).bind(hash.as_slice()).bind(format!("channel-fixture-{user}"))
        .bind(role).execute(pool).await.unwrap();
}

async fn state(pool: &PgPool) -> Vec<(String, String)> {
    let tables: Vec<(String, String)> = sqlx::query_as("SELECT n.nspname,c.relname FROM pg_class c JOIN pg_namespace n ON n.oid=c.relnamespace WHERE c.relkind='r' AND (n.nspname LIKE 'core_%' OR n.nspname='public') ORDER BY n.nspname,c.relname")
        .fetch_all(pool).await.unwrap();
    let mut result = Vec::new();
    for (schema, table) in tables {
        assert!(
            schema
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || b == b'_')
        );
        assert!(
            table
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || b == b'_')
        );
        let sql = format!(
            "SELECT encode(sha256(convert_to(COALESCE(jsonb_agg(to_jsonb(t) ORDER BY to_jsonb(t)::text),'[]'::jsonb)::text,'UTF8')),'hex') FROM \"{schema}\".\"{table}\" t"
        );
        let hash: String = sqlx::query_scalar(&sql).fetch_one(pool).await.unwrap();
        result.push((format!("{schema}.{table}"), hash));
    }
    let sequences: Vec<(String, String)> = sqlx::query_as("SELECT schemaname||'.'||sequencename,COALESCE(last_value::text,'null') FROM pg_sequences WHERE schemaname LIKE 'core_%' OR schemaname='public' ORDER BY schemaname,sequencename")
        .fetch_all(pool).await.unwrap();
    result.extend(sequences);
    result
}

async fn check_reads(app: &Router, secret: &str, expected: StatusCode) {
    for path in READS {
        let response = request(app.clone(), path, Some(secret), "GET").await;
        assert_eq!(response.status(), expected, "GET {path}");
        assert_eq!(response.headers()[header::CACHE_CONTROL], "no-store");
        let body = to_bytes(response.into_body(), 65536).await.unwrap();
        let value: Value = serde_json::from_slice(&body).unwrap();
        if expected == StatusCode::OK {
            assert_eq!(value["success"], true, "GET {path}");
            let serialized = String::from_utf8(body.to_vec()).unwrap();
            for forbidden in [
                "private-upstream",
                "credential",
                "999001",
                "base_url",
                "private-model-name",
            ] {
                assert!(
                    !serialized.contains(forbidden),
                    "GET {path} leaks {forbidden}"
                );
            }
        } else {
            assert_eq!(value["success"], false, "GET {path}");
        }
    }
}

#[test]
fn real_frozen_library_channel_router_postgres() {
    let rt = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap();
    rt.block_on(async {
        let path = std::env::var_os("LMM_CHANNEL_OWNER_URL_FILE").expect("fixture URL file required");
        let socket = std::env::var_os("LMM_CHANNEL_PRIVATE_SOCKET").expect("fixture socket required");
        let url = std::fs::read_to_string(path).expect("cannot read private fixture URL");
        let options = PgConnectOptions::from_str(url.trim()).expect("invalid private fixture URL");
        assert_eq!(options.get_database(), Some("lmm_channel_fixture"));
        assert_eq!(options.get_username(), "postgres");
        assert_eq!(options.get_socket().map(|p| p.as_path()), Some(std::path::Path::new(&socket)));
        let pool = PgPoolOptions::new().max_connections(2).connect_with(options.clone()).await.unwrap();
        let isolated: (String, bool, bool) = sqlx::query_as(
            "SELECT current_database(), inet_server_addr() IS NULL, to_regnamespace('core_identity') IS NULL",
        ).fetch_one(&pool).await.unwrap();
        assert_eq!(isolated, ("lmm_channel_fixture".to_owned(), true, true), "fixture must be a fresh isolated Unix-socket database");
        lmm_core::core_schema::init(&pool).await.unwrap();
    let identity = IdentityStore::from_pool(pool.clone());
    let allowed = identity.bootstrap_user(101, 5).await.unwrap();
    let denied = identity.bootstrap_user(102, 5).await.unwrap();
    let root = identity.bootstrap_user(103, 6).await.unwrap();
    let user = identity.bootstrap_user(104, 1).await.unwrap();
    let revoked = identity.bootstrap_user(105, 5).await.unwrap();
    let changed = identity.bootstrap_user(106, 5).await.unwrap();
    let missing = identity.bootstrap_user(107, 5).await.unwrap();
    let incomplete = identity.bootstrap_user(108, 5).await.unwrap();
    let bad_hash = identity.bootstrap_user(109, 5).await.unwrap();
    let inactive = identity.bootstrap_user(110, 5).await.unwrap();
    let stale = identity.bootstrap_user(111, 5).await.unwrap();
    let root_missing = identity.bootstrap_user(112, 6).await.unwrap();
    let root_bad_hash = identity.bootstrap_user(113, 6).await.unwrap();
    identity.bootstrap_user(114, 5).await.unwrap();
    let bare = "lmms_synthetic_session_without_assurance_114";
    let bare_digest = Sha256::digest(bare.as_bytes()).to_vec();
    sqlx::query("INSERT INTO core_identity.credentials(digest,kind,user_id,user_version,owner_account_id,expires_at) SELECT $1,'session',id,auth_version,personal_account_id,clock_timestamp()+interval '1 hour' FROM core_identity.users WHERE id=114")
        .bind(bare_digest).execute(&pool).await.unwrap();
    let key = identity
        .issue_key(
            &allowed.secret,
            Account {
                kind: AccountKind::Personal,
                id: 101,
            },
            FundingPolicy::new_personal_key(),
            3600,
        )
        .await
        .unwrap();

    for id in [101, 102, 107, 108, 109] {
        imported(&pool, id, "admin").await;
    }
    imported(&pool, 112, "superadmin").await;
    imported(&pool, 113, "superadmin").await;
    policy(&pool, 101, "allow", true, true).await;
    policy(&pool, 102, "deny", true, true).await;
    policy(&pool, 103, "deny", true, true).await;
    policy(&pool, 108, "allow", false, true).await;
    policy(&pool, 109, "allow", true, false).await;
    policy(&pool, 113, "allow", true, false).await;
    sqlx::query("UPDATE core_identity.credentials SET revoked_at=clock_timestamp() WHERE id=$1")
        .bind(revoked.id)
        .execute(&pool)
        .await
        .unwrap();
    sqlx::query("UPDATE core_identity.accounts SET active=FALSE WHERE id=(SELECT personal_account_id FROM core_identity.users WHERE id=110)").execute(&pool).await.unwrap();
    sqlx::query("UPDATE core_identity.users SET auth_version=auth_version+1 WHERE id=111")
        .execute(&pool)
        .await
        .unwrap();

    let runtime_name = "lmm_channel_fixture_runtime";
        sqlx::raw_sql("CREATE ROLE lmm_channel_fixture_runtime LOGIN NOSUPERUSER NOCREATEDB NOCREATEROLE NOREPLICATION NOBYPASSRLS; GRANT CONNECT ON DATABASE lmm_channel_fixture TO lmm_channel_fixture_runtime; GRANT USAGE ON SCHEMA core_identity TO lmm_channel_fixture_runtime; GRANT SELECT ON core_identity.credentials,core_identity.users,core_identity.accounts,core_identity.teams,core_identity.memberships,core_identity.api_key_settings,core_identity.web_sessions,core_identity.user_profiles,core_identity.mfa_factors,core_identity.session_authentication,core_identity.key_funding_rules,core_identity.credential_grants,core_identity.privileged_import_facts,core_identity.permission_policies,core_identity.permission_policy_rows,core_identity.user_routing_groups TO lmm_channel_fixture_runtime")
        .execute(&pool).await.unwrap();
    let runtime_options = options.clone().username(runtime_name);
    let runtime_pool = sqlx::postgres::PgPoolOptions::new().max_connections(2).connect_with(runtime_options).await.unwrap();
    let (session, current, superuser): (String, String, bool) = sqlx::query_as(
        "SELECT session_user::text,current_user::text,rolsuper FROM pg_roles WHERE rolname=current_user",
    ).fetch_one(&runtime_pool).await.unwrap();
    assert_eq!(session, runtime_name);
    assert_eq!(current, runtime_name);
    assert!(!superuser);
        let cannot_write: bool = sqlx::query_scalar("SELECT NOT EXISTS (SELECT FROM pg_class c JOIN pg_namespace n ON n.oid=c.relnamespace WHERE n.nspname='core_identity' AND c.relkind IN ('r','p') AND (has_table_privilege(c.oid,'INSERT,UPDATE,DELETE,TRUNCATE,TRIGGER,REFERENCES,MAINTAIN') OR has_any_column_privilege(c.oid,'INSERT,UPDATE,REFERENCES')))")
            .fetch_one(&runtime_pool).await.unwrap();
        assert!(cannot_write, "fixture runtime must have no identity table or column writes");
    let (config, prices) = fixture();
    let app = lmm_core::channel_http::router(runtime_pool.clone(), serving(config, prices));
    let before = state(&pool).await;
    check_reads(&app, &allowed.secret, StatusCode::OK).await;
    check_reads(&app, &denied.secret, StatusCode::FORBIDDEN).await;
    check_reads(&app, &root.secret, StatusCode::OK).await;
    check_reads(&app, &user.secret, StatusCode::FORBIDDEN).await;
    check_reads(&app, &key.secret, StatusCode::FORBIDDEN).await;
    check_reads(&app, &revoked.secret, StatusCode::UNAUTHORIZED).await;
    check_reads(&app, &inactive.secret, StatusCode::UNAUTHORIZED).await;
    check_reads(&app, &stale.secret, StatusCode::UNAUTHORIZED).await;
    check_reads(&app, bare, StatusCode::UNAUTHORIZED).await;
    check_reads(&app, &missing.secret, StatusCode::FORBIDDEN).await;
    check_reads(&app, &incomplete.secret, StatusCode::FORBIDDEN).await;
    check_reads(&app, &bad_hash.secret, StatusCode::FORBIDDEN).await;
    check_reads(&app, &root_missing.secret, StatusCode::FORBIDDEN).await;
    check_reads(&app, &root_bad_hash.secret, StatusCode::FORBIDDEN).await;
    check_reads(&app, &changed.secret, StatusCode::OK).await;
    // AdminUsersRead remains allowed while ChannelRead is explicitly denied.
    let directory = lmm_core::identity::admin_users::AdminUsersStore::new(
        IdentityStore::from_pool(runtime_pool.clone()),
    );
    assert!(directory.groups(&denied.secret, &lmm_core::identity::admin_users::GroupsQuery::default()).await.is_ok());
    assert_eq!(
        before,
        state(&pool).await,
        "channel reads changed database rows or sequences"
    );

    // A native admin begins with the existing default; the first installed
    // immutable policy denies it on the next request, without changing Session.
    policy(&pool, 106, "deny", true, true).await;
    policy(&pool, 107, "allow", true, true).await;
    let before = state(&pool).await;
    check_reads(&app, &changed.secret, StatusCode::FORBIDDEN).await;
    check_reads(&app, &missing.secret, StatusCode::OK).await;
    for (path, method) in [
        ("/api/channel", "POST"),
        ("/api/channel/", "PUT"),
        ("/api/channel/1", "DELETE"),
    ] {
        let response = request(app.clone(), path, Some(&allowed.secret), method).await;
        assert_eq!(
            response.status(),
            StatusCode::NOT_IMPLEMENTED,
            "{method} {path}"
        );
    }
    assert_eq!(
        before,
        state(&pool).await,
        "channel operations changed database rows or sequences"
    );

        drop(app);
        runtime_pool.close().await;
        pool.close().await;
    });
}

use super::permissions;
use axum::{
    Router,
    body::{Body, to_bytes},
    http::{Request, StatusCode, header},
};
use serde_json::Value;
use sqlx::PgPool;
use tower::ServiceExt;

const READS: [&str; 5] = [
    "/api/channel",
    "/api/channel/",
    "/api/channel/search?keyword=21",
    "/api/channel/ops",
    "/api/channel/1",
];

pub(super) async fn policy(
    pool: &PgPool,
    user: i64,
    read: &str,
    complete: bool,
    valid_hash: bool,
) {
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
        .bind(user).bind(super::CONTRACT).bind(hash.as_slice()).bind(payload)
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

pub(super) async fn imported(pool: &PgPool, user: i64, role: &str) {
    let hash = [7_u8; 32];
    sqlx::query("INSERT INTO core_identity.privileged_import_facts(user_id,archive_sha256,a2_semantic_sha256,a2_file_sha256,a2_sql_sha256,migration_id,system_id,database_oid,instance_nonce,import_run,source_enabled,source_deleted,password_verifier_recognized,password_login_enabled,password_hash_sha256,login_name_sha256,source_role,source_second_factor_required,source_auth_version) VALUES ($1,$2,$2,$2,$2,$3,$2,1,$2,$2,TRUE,FALSE,TRUE,TRUE,$2,$2,$4,FALSE,1)")
        .bind(user).bind(hash.as_slice()).bind(format!("channel-fixture-{user}"))
        .bind(role).execute(pool).await.unwrap();
}

pub(super) async fn state(pool: &PgPool) -> Vec<(String, String)> {
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
        let hash: String = sqlx::query_scalar(sqlx::AssertSqlSafe::<&str>(&sql))
            .fetch_one(pool)
            .await
            .unwrap();
        result.push((format!("{schema}.{table}"), hash));
    }
    let sequences: Vec<(String, String)> = sqlx::query_as("SELECT schemaname||'.'||sequencename,COALESCE(last_value::text,'null') FROM pg_sequences WHERE schemaname LIKE 'core_%' OR schemaname='public' ORDER BY schemaname,sequencename")
        .fetch_all(pool).await.unwrap();
    result.extend(sequences);
    result
}

pub(super) async fn check_reads(app: &Router, secret: &str, expected: StatusCode) {
    for path in READS {
        let req = Request::builder()
            .uri(path)
            .method("GET")
            .header(header::AUTHORIZATION, format!("Bearer {secret}"))
            .body(Body::empty())
            .unwrap();
        let response = app.clone().oneshot(req).await.unwrap();
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

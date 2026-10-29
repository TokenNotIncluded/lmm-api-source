//! Synthetic PostgreSQL/Router regression; execution requires an isolated SQLx database.
use super::{router, tests, *};
use crate::{
    accounts::{Account, AccountKind},
    funding::FundingPolicy,
    identity::{self, IdentityStore, permissions},
    test_runtime::RuntimeRole,
};
use axum::body::to_bytes;
use sha2::{Digest, Sha256};

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
        .bind(user).bind(permissions::CONTRACT_VERSION).bind(hash.as_slice()).bind(payload)
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
        let response = tests::request(app.clone(), path, Some(secret), "GET").await;
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

#[sqlx::test(migrations = false)]
async fn channel_read_uses_current_policy_and_preserves_read_only_state(pool: PgPool) {
    crate::core_schema::init(&pool).await.unwrap();
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

    let role = RuntimeRole::create(&pool).await;
    let (session, current, superuser): (String, String, bool) = sqlx::query_as(
        "SELECT session_user::text,current_user::text,rolsuper FROM pg_roles WHERE rolname=current_user",
    )
    .fetch_one(&role.pool)
    .await
    .unwrap();
    assert_eq!(session, role.name);
    assert_eq!(current, role.name);
    assert!(!superuser);
    let (config, prices) = tests::fixture();
    let app = router(role.pool.clone(), tests::serving(config, prices));
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
    let mut tx = role.pool.begin().await.unwrap();
    sqlx::query("SET TRANSACTION ISOLATION LEVEL REPEATABLE READ READ ONLY")
        .execute(&mut *tx)
        .await
        .unwrap();
    assert!(
        identity::authorize_admin_read_snapshot(&mut tx, &denied.secret)
            .await
            .is_ok()
    );
    tx.commit().await.unwrap();
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
        let response = tests::request(app.clone(), path, Some(&allowed.secret), method).await;
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
    role.cleanup().await;
}

//! Account appeal persistence and the actual minimum runtime writer authority.
use lmm_core::{core_schema, identity::IdentityStore};
use sqlx::PgPool;
#[path = "common/runtime.rs"]
mod runtime;

#[sqlx::test(migrations = false)]
async fn appeals_constraints_and_runtime_column_authority(pool: PgPool) {
    core_schema::init(&pool).await.unwrap();
    let identity = IdentityStore::from_pool(pool.clone());
    identity.bootstrap_user(8101, 0).await.unwrap();
    identity.bootstrap_user(8102, 6).await.unwrap();
    let role = runtime::RuntimeRole::create(&pool).await;
    role.optional("web_identity").await;
    let id: i64 = sqlx::query_scalar("INSERT INTO core_account_appeals.requests(user_id,user_version,reason) VALUES(8101,1,'Please review my account') RETURNING id")
        .fetch_one(&role.pool).await.unwrap();
    assert!((1..=9_007_199_254_740_991).contains(&id));
    let initial: (String, String, i64, bool) = sqlx::query_as("SELECT status,admin_note,revision,reviewed_by IS NULL AND reviewed_at IS NULL FROM core_account_appeals.requests WHERE id=$1")
        .bind(id).fetch_one(&role.pool).await.unwrap();
    assert_eq!(initial, ("pending".into(), "".into(), 1, true));
    for statement in [
        "INSERT INTO core_account_appeals.requests(user_id,user_version,reason) VALUES(8101,1,'duplicate pending')",
        "INSERT INTO core_account_appeals.requests(user_id,user_version,reason) VALUES(8101,2,'tiny')",
        "INSERT INTO core_account_appeals.requests(user_id,user_version,reason) VALUES(8101,2,repeat('字',2001))",
        "INSERT INTO core_account_appeals.requests(user_id,user_version,reason) VALUES(8101,0,'valid reason')",
        "INSERT INTO core_account_appeals.requests(user_id,user_version,reason,revision) VALUES(8101,2,'valid reason',0)",
        "INSERT INTO core_account_appeals.requests(user_id,user_version,reason,status) VALUES(8101,2,'valid reason','other')",
        "INSERT INTO core_account_appeals.requests(user_id,user_version,reason,status) VALUES(8101,2,'valid reason','approved')",
        "INSERT INTO core_account_appeals.requests(user_id,user_version,reason,reviewed_by) VALUES(8101,2,'valid reason',8102)",
        "INSERT INTO core_account_appeals.requests(user_id,user_version,reason,admin_note) VALUES(8101,2,'valid reason',repeat('a',2001))",
        "INSERT INTO core_account_appeals.requests(user_id,user_version,reason) VALUES(-1,2,'valid reason')",
        "INSERT INTO core_account_appeals.requests(id,user_id,user_version,reason) OVERRIDING SYSTEM VALUE VALUES(9007199254740992,8101,2,'valid reason')",
        "INSERT INTO core_account_appeals.requests(id,user_id,user_version,reason) OVERRIDING SYSTEM VALUE VALUES(0,8101,2,'valid reason')",
        "INSERT INTO core_account_appeals.flows(digest,user_id,user_version,proof_kind,expires_at) VALUES(decode('01','hex'),8101,1,'password',clock_timestamp())",
        "INSERT INTO core_account_appeals.flows(digest,user_id,user_version,proof_kind,expires_at) VALUES(decode(repeat('01',32),'hex'),8101,1,'',clock_timestamp())",
        "INSERT INTO core_account_appeals.flows(digest,user_id,user_version,proof_kind,expires_at) VALUES(decode(repeat('01',32),'hex'),8101,1,repeat('a',65),clock_timestamp())",
        "INSERT INTO core_account_appeals.flows(digest,user_id,user_version,proof_kind,expires_at) VALUES(decode(repeat('01',32),'hex'),8101,0,'password',clock_timestamp())",
        "INSERT INTO core_account_appeals.flows(digest,user_id,user_version,proof_kind,expires_at,request_id) VALUES(decode(repeat('01',32),'hex'),8101,1,'password',clock_timestamp(),-1)",
    ] {
        let error = sqlx::raw_sql(statement).execute(&pool).await.unwrap_err();
        assert!(
            matches!(
                error.as_database_error().unwrap().code().as_deref(),
                Some("23505" | "23514" | "23503")
            ),
            "{statement}: {error}"
        );
    }
    sqlx::query("INSERT INTO core_account_appeals.flows(digest,user_id,user_version,proof_kind,expires_at) VALUES(decode(repeat('02',32),'hex'),8101,1,'password',clock_timestamp()+interval '5 minutes')")
        .execute(&role.pool).await.unwrap();
    sqlx::query("UPDATE core_account_appeals.flows SET request_id=$1")
        .bind(id)
        .execute(&role.pool)
        .await
        .unwrap();
    for statement in [
        "UPDATE core_account_appeals.requests SET reason='rewritten reason'",
        "UPDATE core_account_appeals.requests SET user_version=2",
        "UPDATE core_account_appeals.requests SET created_at=clock_timestamp()",
        "DELETE FROM core_account_appeals.requests",
        "UPDATE core_account_appeals.flows SET expires_at=clock_timestamp()",
        "UPDATE core_account_appeals.flows SET user_id=8102",
        "SELECT * FROM core_meta.account_appeal_upgrades",
    ] {
        let error = sqlx::raw_sql(statement)
            .execute(&role.pool)
            .await
            .unwrap_err();
        assert_eq!(
            error.as_database_error().unwrap().code().as_deref(),
            Some("42501"),
            "{statement}: {error}"
        );
    }
    for status in ["approved", "rejected"] {
        sqlx::query("UPDATE core_account_appeals.requests SET status=$1,admin_note='reviewed',reviewed_by=8102,reviewed_at=clock_timestamp(),revision=revision+1 WHERE id=$2")
            .bind(status).bind(id).execute(&role.pool).await.unwrap();
    }
    // A resolved request releases only its own pending (user,version) slot.
    sqlx::query("INSERT INTO core_account_appeals.requests(user_id,user_version,reason) VALUES(8101,1,'another valid request'),(8101,2,'different auth version')")
        .execute(&role.pool).await.unwrap();
    sqlx::query("DELETE FROM core_account_appeals.flows")
        .execute(&role.pool)
        .await
        .unwrap();
    core_schema::check(&pool).await.unwrap();
    role.cleanup().await;
}

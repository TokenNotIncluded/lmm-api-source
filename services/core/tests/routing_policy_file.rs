//! Run the local TOML operation against a real Core database and synthetic sessions.
#![cfg(unix)]

use lmm_core::{
    accounts::{Account, AccountKind},
    core_schema,
    funding::FundingPolicy,
    identity::IdentityStore,
    request_limits::RoutingPolicyFile,
};
use serde_json::json;
use sqlx::PgPool;
use std::{
    fs::OpenOptions,
    io::{self, Write},
    os::unix::fs::OpenOptionsExt,
    path::PathBuf,
};

struct SessionFile(PathBuf);
impl SessionFile {
    fn new(secret: &str) -> Self {
        let suffix: u64 = rand::random();
        let path = std::env::temp_dir().join(format!("lmm-routing-session-{suffix:016x}"));
        let mut out = OpenOptions::new()
            .write(true)
            .create_new(true)
            .mode(0o600)
            .open(&path)
            .unwrap();
        writeln!(out, "{secret}").unwrap();
        Self(path)
    }

    fn operation(&self, limits_rev: i64, groups_rev: i64) -> RoutingPolicyFile {
        let input = format!(
            r#"session_file = "{}"
[request_limits]
expected_revision = {limits_rev}
[request_limits.policy]
enabled = true
window_seconds = 120
user_limit = 8
group_limits = {{ default = 2 }}
[auto_groups]
expected_revision = {groups_rev}
groups = ["私有", "default"]
"#,
            self.0.display()
        );
        RoutingPolicyFile::read(input.as_bytes()).unwrap()
    }
}
impl Drop for SessionFile {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.0);
    }
}

#[test]
fn requires_explicit_revision_and_valid_policy() {
    let valid = "session_file='/private/admin'\n[auto_groups]\nexpected_revision=1\ngroups=[]";
    assert!(RoutingPolicyFile::read(valid.as_bytes()).is_ok());
    for invalid in [
        valid.replace("/private/admin", "relative"),
        valid.replace("expected_revision=1", "expected_revision=0"),
        valid.replace("groups=[]", "groups=['default','default']"),
        valid.replace("groups=[]", "groups=['']"),
        valid.replace("expected_revision=1", "expected_revision=1\nunknown=3"),
        valid.replace("session_file='/private/admin'", "session='inline'"),
        "session_file='/private/admin'".to_owned(),
        "session_file='/private/admin'\n[request_limits]\nexpected_revision=1\n[request_limits.policy]\nenabled=true\nwindow_seconds=60\ngroup_limits={}".to_owned(),
    ] {
        assert!(RoutingPolicyFile::read(invalid.as_bytes()).is_err());
    }
}

#[sqlx::test(migrations = false)]
async fn local_policy_session_revisions_audit_and_atomic_rollback(pool: PgPool) {
    core_schema::init(&pool).await.unwrap();
    let identity = IdentityStore::from_pool(pool.clone());
    let root = identity.bootstrap_user(1, 6).await.unwrap();
    let ordinary = identity.bootstrap_user(2, 5).await.unwrap();
    let key = identity
        .issue_key(
            &root.secret,
            Account {
                kind: AccountKind::Personal,
                id: 1,
            },
            FundingPolicy::new_personal_key(),
            3600,
        )
        .await
        .unwrap();
    for secret in [&ordinary.secret, &key.secret] {
        let file = SessionFile::new(secret);
        let err = file.operation(1, 1).apply(&pool).await.unwrap_err();
        assert_eq!(err.kind(), io::ErrorKind::PermissionDenied);
    }
    let file = SessionFile::new(&root.secret);
    assert_eq!(
        file.operation(1, 1).apply(&pool).await.unwrap(),
        json!({"request_limits_revision": 2, "auto_groups_revision": 2})
    );
    // The first CAS succeeds in the transaction, the second one conflicts.
    // The partial limits update and its policy event must both roll back.
    assert!(file.operation(2, 1).apply(&pool).await.is_err());
    let row: (i64, i32, Option<i32>, i64, Vec<String>, i64) = sqlx::query_as(
        r#"SELECT p.revision,p.window_seconds,p.user_limit,a.revision,a.groups,
 (SELECT count(*) FROM core_request_limits.policy_events WHERE actor_user_id=1)
FROM core_request_limits.policy p CROSS JOIN core_request_limits.auto_policy a"#,
    )
    .fetch_one(&pool)
    .await
    .unwrap();
    assert_eq!(
        row,
        (2, 120, Some(8), 2, vec!["私有".to_owned(), "default".to_owned()], 2)
    );
}

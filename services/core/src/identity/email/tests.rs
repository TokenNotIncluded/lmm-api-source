use super::*;
use sha2::{Digest, Sha256};
use sqlx::{PgPool, Row, postgres::PgPoolOptions};

const BASE: &str = include_str!("../../../schema/identity.sql");

#[test]
fn address_policy_preserves_tags_and_rejects_header_syntax() {
    assert_eq!(
        EmailAddress::parse(" A.B+Tag@Example.TEST ")
            .unwrap()
            .as_str(),
        "a.b+tag@example.test"
    );
    for bad in [
        "a@b\r\n",
        "a@b\n",
        "Name <a@b>",
        "a..b@c",
        "a@@b",
        "a@-b.c",
        "a@b..c",
        "a b@c",
        "\"a\"@b",
        "中@b",
    ] {
        assert_eq!(
            EmailAddress::parse(bad),
            Err(EmailError::InvalidAddress),
            "{bad:?}"
        );
    }
}

fn address(input: &str) -> EmailAddress {
    EmailAddress::parse(input).unwrap()
}

fn store(pool: PgPool) -> EmailChallenges {
    EmailChallenges::new(pool, "test-v1", [17; 32], [39; 32]).unwrap()
}

async fn pool(variable: &str) -> PgPool {
    let url = std::env::var(variable).expect("run the isolated email test runner");
    assert!(
        url.contains("/email_slice?host=/tmp/lmm-email-"),
        "refuse non-fixture database"
    );
    PgPoolOptions::new()
        .max_connections(4)
        .connect(&url)
        .await
        .unwrap()
}

async fn fresh() -> (PgPool, PgPool, EmailChallenges) {
    let owner = pool("EMAIL_TEST_OWNER_URL").await;
    sqlx::raw_sql("DROP SCHEMA IF EXISTS core_identity CASCADE; DROP SCHEMA IF EXISTS core_billing CASCADE; DROP SCHEMA IF EXISTS core_meta CASCADE;")
        .execute(&owner).await.unwrap();
    sqlx::raw_sql(BASE).execute(&owner).await.unwrap();
    sqlx::raw_sql(include_str!("schema.sql"))
        .execute(&owner)
        .await
        .unwrap();
    sqlx::raw_sql(include_str!("test_grants.sql"))
        .execute(&owner)
        .await
        .unwrap();
    let runtime = pool("EMAIL_TEST_RUNTIME_URL").await;
    let roles: (bool, bool, bool) = sqlx::query_as(
        "SELECT rolsuper,rolcreatedb,rolcreaterole FROM pg_roles WHERE rolname=current_user",
    )
    .fetch_one(&runtime)
    .await
    .unwrap();
    assert_eq!(roles, (false, false, false));
    (owner, runtime.clone(), store(runtime))
}

async fn user(owner: &PgPool, id: i64, email: &str) {
    let account: i64 = sqlx::query_scalar(
        "INSERT INTO core_identity.accounts(kind) VALUES ('personal') RETURNING id",
    )
    .fetch_one(owner)
    .await
    .unwrap();
    sqlx::query("INSERT INTO core_identity.users(id,personal_account_id) VALUES ($1,$2)")
        .bind(id)
        .bind(account)
        .execute(owner)
        .await
        .unwrap();
    sqlx::query(
        "INSERT INTO core_identity.user_profiles(user_id,username,email) VALUES ($1,$2,$3)",
    )
    .bind(id)
    .bind(format!("test{id}"))
    .bind(email)
    .execute(owner)
    .await
    .unwrap();
    sqlx::query("INSERT INTO core_identity.password_logins(login_name,user_id,password_hash) VALUES ($1,$2,$3)")
        .bind(format!("test{id}")).bind(id).bind("synthetic-verifier-placeholder-not-a-password".repeat(2)).execute(owner).await.unwrap();
}

async fn browser(owner: &PgPool, user: i64, sid: &str, access: &str) {
    let id: i64 = sqlx::query_scalar(
        "INSERT INTO core_identity.credentials(digest,kind,user_id,user_version,owner_account_id,expires_at)
         SELECT $1,'session',id,auth_version,personal_account_id,clock_timestamp()+interval '15 minutes'
         FROM core_identity.users WHERE id=$2 RETURNING id",
    ).bind(Sha256::digest(access.as_bytes()).to_vec()).bind(user).fetch_one(owner).await.unwrap();
    sqlx::query(
        "INSERT INTO core_identity.web_sessions(sid,user_id,user_version,current_credential_id,refresh_digest,ip,user_agent)
         SELECT $1,id,auth_version,$2,$3,'127.0.0.1','email-selftest' FROM core_identity.users WHERE id=$4",
    ).bind(sid).bind(id).bind(vec![11_u8;32]).bind(user).execute(owner).await.unwrap();
    sqlx::query(
        "INSERT INTO core_identity.session_authentication(credential_id,user_id,user_version,factor_revision,
             factor_level,method,authenticated_at)
         SELECT $1,id,auth_version,0,1,'primary',clock_timestamp() FROM core_identity.users WHERE id=$2",
    ).bind(id).bind(user).execute(owner).await.unwrap();
}

async fn issued(
    store: &EmailChallenges,
    request: Request<'_>,
    email: &EmailAddress,
) -> IssuedEmailChallenge {
    match store.issue(request, email).await.unwrap() {
        IssueOutcome::Issued(proof) => proof,
        _ => panic!("expected eligible issue"),
    }
}

async fn age_issue_events(owner: &PgPool) {
    sqlx::query("UPDATE core_identity.email_rate_events SET occurred_at=occurred_at-interval '61 seconds' WHERE kind='issue'")
        .execute(owner).await.unwrap();
}

#[tokio::test]
#[ignore = "requires the isolated email test runner"]
async fn persistence_purpose_attempts_and_target_budget_survive_reopen() {
    let (owner, runtime, s) = fresh().await;
    let email = address("Proof@Example.test");
    let proof = issued(&s, Request::Register, &email).await;
    assert_eq!(proof.code.len(), 6);
    assert!(
        s.verify(Request::PasswordReset, &email, &proof.id, &proof.code)
            .await
            .is_err()
    );
    let wrong = if *proof.code == "ffffff" {
        "000000"
    } else {
        "ffffff"
    };
    for _ in 0..5 {
        assert!(matches!(
            store(runtime.clone())
                .verify(Request::Register, &email, &proof.id, wrong)
                .await,
            Err(EmailError::InvalidProof)
        ));
    }
    assert!(
        store(runtime.clone())
            .verify(Request::Register, &email, &proof.id, &proof.code)
            .await
            .is_err()
    );
    let row = sqlx::query("SELECT attempts,state,row_to_json(c)::text AS stored FROM core_identity.email_challenges c WHERE id=$1")
        .bind(&proof.id).fetch_one(&owner).await.unwrap();
    assert_eq!(row.get::<i16, _>("attempts"), 5);
    assert_eq!(row.get::<String, _>("state"), "locked");
    assert!(!row.get::<String, _>("stored").contains(proof.code.as_str()));
    age_issue_events(&owner).await;
    let next = issued(&s, Request::Register, &email).await;
    // Purpose mismatch plus five wrong codes plus the locked replay used 7/10.
    for _ in 0..3 {
        assert!(
            s.verify(Request::Register, &email, &next.id, wrong)
                .await
                .is_err()
        );
    }
    let attempts: i16 =
        sqlx::query_scalar("SELECT attempts FROM core_identity.email_challenges WHERE id=$1")
            .bind(&next.id)
            .fetch_one(&owner)
            .await
            .unwrap();
    assert_eq!(attempts, 3);
    assert!(
        s.verify(Request::Register, &email, &next.id, &next.code)
            .await
            .is_err()
    );
    let attempts_after: i16 =
        sqlx::query_scalar("SELECT attempts FROM core_identity.email_challenges WHERE id=$1")
            .bind(&next.id)
            .fetch_one(&owner)
            .await
            .unwrap();
    assert_eq!(
        attempts_after, 3,
        "target budget must persist across challenges"
    );
}

#[tokio::test]
#[ignore = "requires the isolated email test runner"]
async fn transaction_drop_and_business_failure_preserve_proof_and_final_consumption_is_unique() {
    let (owner, _, s) = fresh().await;
    user(&owner, 1, "").await;
    let email = address("atomic@example.test");
    let proof = issued(&s, Request::Register, &email).await;
    let mut mutation = s
        .verify(Request::Register, &email, &proof.id, &proof.code)
        .await
        .unwrap();
    sqlx::query(
        "UPDATE core_identity.user_profiles SET display_name='rolled back' WHERE user_id=1",
    )
    .execute(mutation.connection())
    .await
    .unwrap();
    drop(mutation);
    let mut mutation = s
        .verify(Request::Register, &email, &proof.id, &proof.code)
        .await
        .unwrap();
    assert!(
        sqlx::query(
            "UPDATE core_identity.user_profiles SET display_name=repeat('x',257) WHERE user_id=1"
        )
        .execute(mutation.connection())
        .await
        .is_err()
    );
    drop(mutation);
    let mut mutation = s
        .verify(Request::Register, &email, &proof.id, &proof.code)
        .await
        .unwrap();
    sqlx::query("UPDATE core_identity.user_profiles SET display_name='committed' WHERE user_id=1")
        .execute(mutation.connection())
        .await
        .unwrap();
    mutation.finish(1).await.unwrap();
    assert!(
        s.verify(Request::Register, &email, &proof.id, &proof.code)
            .await
            .is_err()
    );
    let name: String =
        sqlx::query_scalar("SELECT display_name FROM core_identity.user_profiles WHERE user_id=1")
            .fetch_one(&owner)
            .await
            .unwrap();
    assert_eq!(name, "committed");
    let audits: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM core_identity.audit WHERE action='identity.email_proof_consumed'",
    )
    .fetch_one(&owner)
    .await
    .unwrap();
    assert_eq!(audits, 1);
    let receipt_count: i64 = sqlx::query_scalar("SELECT count(*) FROM core_meta.schema_contract")
        .fetch_one(&owner)
        .await
        .unwrap();
    assert_eq!(
        receipt_count, 0,
        "leaf fixture must never forge an installed contract"
    );
}

#[tokio::test]
#[ignore = "requires the isolated email test runner"]
async fn concurrent_stores_only_commit_one_mutation() {
    let (owner, runtime, s) = fresh().await;
    user(&owner, 1, "").await;
    let email = address("race@example.test");
    let proof = issued(&s, Request::Register, &email).await;
    let mut tasks = Vec::new();
    for _ in 0..8 {
        let other = store(runtime.clone());
        let email = email.clone();
        let id = proof.id.clone();
        let code = proof.code.to_string();
        tasks.push(tokio::spawn(async move {
            let mut mutation=other.verify(Request::Register,&email,&id,&code).await?;
            sqlx::query("UPDATE core_identity.user_profiles SET display_name=display_name||'x' WHERE user_id=1")
                .execute(mutation.connection()).await?;
            mutation.finish(1).await
        }));
    }
    let mut successes = 0;
    for task in tasks {
        successes += usize::from(task.await.unwrap().is_ok());
    }
    assert_eq!(successes, 1);
    let name: String =
        sqlx::query_scalar("SELECT display_name FROM core_identity.user_profiles WHERE user_id=1")
            .fetch_one(&owner)
            .await
            .unwrap();
    assert_eq!(name, "x");
}

#[tokio::test]
#[ignore = "requires the isolated email test runner"]
async fn resends_keep_deadline_attempts_and_cross_purpose_cooldown() {
    let (owner, _, s) = fresh().await;
    user(&owner, 1, "").await;
    let sid = "a".repeat(64);
    let access = "lmms_synthetic_browser_access_00000000000000000000000000000000";
    browser(&owner, 1, &sid, access).await;
    let email = address("resend@example.test");
    let proof = issued(&s, Request::Register, &email).await;
    assert!(
        s.verify(Request::Register, &email, &proof.id, "invalid")
            .await
            .is_err()
    );
    assert!(matches!(
        s.issue(Request::Bind { access, sid: &sid }, &email)
            .await
            .unwrap(),
        IssueOutcome::Suppressed
    ));
    age_issue_events(&owner).await;
    assert!(matches!(
        s.issue(Request::Register, &email).await.unwrap(),
        IssueOutcome::Pending
    ));
    let row=sqlx::query("SELECT attempts,floor(extract(epoch FROM expires_at))::bigint AS expiry FROM core_identity.email_challenges WHERE id=$1")
        .bind(&proof.id).fetch_one(&owner).await.unwrap();
    assert_eq!(row.get::<i16, _>("attempts"), 1);
    assert_eq!(row.get::<i64, _>("expiry"), proof.expires_at);
    // Every purpose shares the per-target hour budget, even without a new code.
    for _ in 0..3 {
        age_issue_events(&owner).await;
        assert!(matches!(
            s.issue(Request::Register, &email).await.unwrap(),
            IssueOutcome::Pending
        ));
    }
    age_issue_events(&owner).await;
    assert!(matches!(
        s.issue(Request::Register, &email).await.unwrap(),
        IssueOutcome::Suppressed
    ));
}

#[tokio::test]
#[ignore = "requires the isolated email test runner"]
async fn legacy_email_never_grants_recovery_and_current_generations_are_required() {
    let (owner, _, s) = fresh().await;
    user(&owner, 1, "Old@Example.test").await;
    let email = address("old@example.test");
    assert!(matches!(
        s.issue(Request::PasswordReset, &email).await.unwrap(),
        IssueOutcome::Suppressed
    ));
    age_issue_events(&owner).await;
    assert!(matches!(
        s.issue(Request::Register, &email).await.unwrap(),
        IssueOutcome::Suppressed
    ));
    sqlx::query("INSERT INTO core_identity.email_addresses(email_key,user_id,state,email_version) VALUES ($1,1,'legacy_unverified',0)")
        .bind(email.as_str()).execute(&owner).await.unwrap();
    age_issue_events(&owner).await;
    assert!(matches!(
        s.issue(Request::PasswordReset, &email).await.unwrap(),
        IssueOutcome::Suppressed
    ));
    sqlx::query("UPDATE core_identity.email_addresses SET state='verified',verified_at=clock_timestamp(),email_version=1 WHERE user_id=1")
        .execute(&owner).await.unwrap();
    sqlx::query("UPDATE core_identity.user_profiles SET email_version=1 WHERE user_id=1")
        .execute(&owner)
        .await
        .unwrap();
    age_issue_events(&owner).await;
    let proof = issued(&s, Request::PasswordReset, &email).await;
    assert_eq!(proof.code.len(), 32);
    sqlx::query("UPDATE core_identity.users SET auth_version=auth_version+1 WHERE id=1")
        .execute(&owner)
        .await
        .unwrap();
    assert!(
        s.verify(Request::PasswordReset, &email, &proof.id, &proof.code)
            .await
            .is_err()
    );
    sqlx::query("UPDATE core_identity.user_profiles SET email_version=2 WHERE user_id=1")
        .execute(&owner)
        .await
        .unwrap();
    assert!(
        s.verify(Request::PasswordReset, &email, &proof.id, &proof.code)
            .await
            .is_err()
    );
    // Removing a password method must not let recovery recreate it.
    sqlx::query("DELETE FROM core_identity.password_logins WHERE user_id=1")
        .execute(&owner)
        .await
        .unwrap();
    age_issue_events(&owner).await;
    assert!(matches!(
        s.issue(Request::PasswordReset, &email).await.unwrap(),
        IssueOutcome::Suppressed
    ));
}

#[tokio::test]
#[ignore = "requires the isolated email test runner"]
async fn binding_requires_current_browser_family_and_recent_assurance() {
    let (owner, _, s) = fresh().await;
    user(&owner, 1, "").await;
    let sid = "a".repeat(64);
    let second_sid = "b".repeat(64);
    let access = "lmms_synthetic_browser_access_00000000000000000000000000000001";
    let second_access = "lmms_synthetic_browser_access_00000000000000000000000000000002";
    browser(&owner, 1, &sid, access).await;
    browser(&owner, 1, &second_sid, second_access).await;
    let email = address("binding@example.test");
    let proof = issued(&s, Request::Bind { access, sid: &sid }, &email).await;
    assert!(
        s.verify(
            Request::Bind {
                access: second_access,
                sid: &second_sid
            },
            &email,
            &proof.id,
            &proof.code
        )
        .await
        .is_err()
    );
    assert!(
        s.verify(
            Request::Bind {
                access: "fake",
                sid: &sid
            },
            &email,
            &proof.id,
            &proof.code
        )
        .await
        .is_err()
    );
    s.verify(
        Request::Bind { access, sid: &sid },
        &email,
        &proof.id,
        &proof.code,
    )
    .await
    .unwrap()
    .finish(1)
    .await
    .unwrap();
    // A stale primary assertion cannot request a new binding.
    sqlx::query("ALTER TABLE core_identity.session_authentication DISABLE TRIGGER session_authentication_immutable").execute(&owner).await.unwrap();
    sqlx::query("UPDATE core_identity.session_authentication SET authenticated_at=clock_timestamp()-interval '6 minutes'").execute(&owner).await.unwrap();
    assert!(matches!(
        s.issue(
            Request::Bind { access, sid: &sid },
            &address("stale@example.test")
        )
        .await
        .unwrap(),
        IssueOutcome::Suppressed
    ));
    // Unknown/mismatched MFA policy remains closed.
    sqlx::query(
        "UPDATE core_identity.user_profiles SET second_factor_required=TRUE WHERE user_id=1",
    )
    .execute(&owner)
    .await
    .unwrap();
    assert!(matches!(
        s.issue(
            Request::Bind { access, sid: &sid },
            &address("mfa@example.test")
        )
        .await
        .unwrap(),
        IssueOutcome::Suppressed
    ));
}

#[tokio::test]
#[ignore = "requires the isolated email test runner"]
async fn expiry_after_lock_wait_and_at_finish_roll_back_business_writes() {
    let (owner, _, s) = fresh().await;
    user(&owner, 1, "").await;
    let email = address("expiry@example.test");
    let proof = issued(&s, Request::Register, &email).await;
    // Owner-only fixture manipulation; runtime cannot extend/change deadlines.
    sqlx::query("ALTER TABLE core_identity.email_challenges DISABLE TRIGGER email_challenge_guard")
        .execute(&owner)
        .await
        .unwrap();
    sqlx::query("UPDATE core_identity.email_challenges SET issued_at=clock_timestamp()-interval '599 seconds',expires_at=clock_timestamp()+interval '0.12 seconds' WHERE id=$1")
        .bind(&proof.id).execute(&owner).await.unwrap();
    let mut lock = owner.begin().await.unwrap();
    sqlx::query("SELECT id FROM core_identity.email_challenges WHERE id=$1 FOR UPDATE")
        .bind(&proof.id)
        .fetch_one(&mut *lock)
        .await
        .unwrap();
    let other = s.clone();
    let e = email.clone();
    let id = proof.id.clone();
    let code = proof.code.to_string();
    let waiting =
        tokio::spawn(async move { other.verify(Request::Register, &e, &id, &code).await });
    tokio::time::sleep(std::time::Duration::from_millis(180)).await;
    lock.commit().await.unwrap();
    assert!(matches!(
        waiting.await.unwrap(),
        Err(EmailError::InvalidProof)
    ));
    let another = address("finish@example.test");
    let proof = issued(&s, Request::Register, &another).await;
    sqlx::query("UPDATE core_identity.email_challenges SET issued_at=clock_timestamp()-interval '599 seconds',expires_at=clock_timestamp()+interval '0.12 seconds' WHERE id=$1")
        .bind(&proof.id).execute(&owner).await.unwrap();
    let mut mutation = s
        .verify(Request::Register, &another, &proof.id, &proof.code)
        .await
        .unwrap();
    sqlx::query(
        "UPDATE core_identity.user_profiles SET display_name='must roll back' WHERE user_id=1",
    )
    .execute(mutation.connection())
    .await
    .unwrap();
    tokio::time::sleep(std::time::Duration::from_millis(180)).await;
    assert_eq!(mutation.finish(1).await, Err(EmailError::InvalidProof));
    let name: String =
        sqlx::query_scalar("SELECT display_name FROM core_identity.user_profiles WHERE user_id=1")
            .fetch_one(&owner)
            .await
            .unwrap();
    assert_eq!(name, "");
}

#[tokio::test]
#[ignore = "requires the isolated email test runner"]
async fn restricted_role_cannot_rewrite_proof_or_create_schema_and_audit_failure_rolls_back() {
    let (owner, runtime, s) = fresh().await;
    user(&owner, 1, "").await;
    let email = address("permissions@example.test");
    let proof = issued(&s, Request::Register, &email).await;
    assert!(
        sqlx::query("UPDATE core_identity.email_challenges SET token_mac=$1 WHERE id=$2")
            .bind(vec![0_u8; 32])
            .bind(&proof.id)
            .execute(&runtime)
            .await
            .is_err()
    );
    assert!(
        sqlx::query("CREATE TABLE core_identity.forbidden(id bigint)")
            .execute(&runtime)
            .await
            .is_err()
    );
    sqlx::query("REVOKE INSERT ON core_identity.audit FROM email_runtime")
        .execute(&owner)
        .await
        .unwrap();
    let mut mutation = s
        .verify(Request::Register, &email, &proof.id, &proof.code)
        .await
        .unwrap();
    sqlx::query("UPDATE core_identity.user_profiles SET display_name='no audit' WHERE user_id=1")
        .execute(mutation.connection())
        .await
        .unwrap();
    assert_eq!(mutation.finish(1).await, Err(EmailError::Unavailable));
    let row=sqlx::query("SELECT c.state,p.display_name FROM core_identity.email_challenges c,core_identity.user_profiles p WHERE c.id=$1 AND p.user_id=1")
        .bind(&proof.id).fetch_one(&owner).await.unwrap();
    assert_eq!(row.get::<String, _>("state"), "active");
    assert_eq!(row.get::<String, _>("display_name"), "");
}

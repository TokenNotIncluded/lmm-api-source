//! Developer self-tests, not independent acceptance. The runner owns the PG
//! cluster. Its notification marker is not a production encrypted mail outbox.
use lmm_core::{
    accounts::{Account, AccountKind},
    core_schema,
    funding::FundingPolicy,
    identity::{
        BrowserAuth, BrowserBundle, BrowserLoginOutcome, BrowserRegistration,
        BrowserSessionMetadata, IdentityError, IdentityStore, LoginRequest, MfaCompletion,
        email::{
            EmailAddress, EmailChallenges, EmailError, IssueOutcome, IssuedEmailChallenge, Request,
        },
        email_business::{
            EmailBusinessError, EmailConfirmation, EmailRegistration, EmailSecurityChange,
            PendingEmailBusiness,
        },
    },
};
use sqlx::{PgPool, Row, postgres::PgPoolOptions};
use std::{path::Path, process::Command};
use totp_rs::{Algorithm, Builder, Secret};

const PASSWORD: &str = "test passphrase";
const NEW_PASSWORD: &str = "new test passphrase";
const BINDING: &str = "1111111111111111111111111111111111111111111111111111111111111111";

struct Fixture {
    owner: PgPool,
    runtime: PgPool,
    store: IdentityStore,
    challenges: EmailChallenges,
    browser: BrowserAuth,
}

async fn fixture() -> Fixture {
    let root = std::env::var("EMAIL_BUSINESS_FIXTURE_ROOT").expect("use run_email_business.py");
    let url = std::env::var("EMAIL_BUSINESS_OWNER_URL").expect("private runner URL");
    assert!(
        root.starts_with("/tmp/lmm-email-business-")
            && Path::new(&root).join("data/PG_VERSION").is_file()
            && url.contains(&format!("host={root}/s")),
        "refuse a database outside the runner's disposable cluster"
    );
    let admin = PgPoolOptions::new()
        .max_connections(1)
        .connect(&url)
        .await
        .unwrap();
    let database = format!("email_business_{:016x}", rand::random::<u64>());
    sqlx::query(&format!("CREATE DATABASE {database}"))
        .execute(&admin)
        .await
        .unwrap();
    let options = admin.connect_options().as_ref().clone().database(&database);
    admin.close().await;
    let owner = PgPoolOptions::new()
        .max_connections(4)
        .connect_with(options.clone())
        .await
        .unwrap();
    core_schema::init(&owner).await.unwrap();
    let grants = Command::new("psql")
        .args([
            "-X",
            "-v",
            "ON_ERROR_STOP=1",
            "-v",
            "runtime_role=email_runtime",
            "-v",
            "enable_web_identity=true",
            "-f",
        ])
        .arg(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../../deployment/postgres/grant-core-runtime.sql"
        ))
        .env("PGDATABASE", &database)
        .output()
        .unwrap();
    assert!(grants.status.success(), "canonical runtime grants failed");
    // Precisely the new business writer's additional authority. Test-only until
    // the official identity revision and runtime contract are reviewed.
    sqlx::raw_sql(
        "GRANT INSERT ON core_identity.email_addresses TO email_runtime;
         GRANT UPDATE(email_key,state,email_version,verified_at)
             ON core_identity.email_addresses TO email_runtime;
         CREATE SCHEMA email_business_test;
         CREATE TABLE email_business_test.notices (
             challenge_id TEXT PRIMARY KEY, user_id BIGINT NOT NULL, change TEXT NOT NULL);
         GRANT USAGE ON SCHEMA email_business_test TO email_runtime;
         GRANT INSERT,SELECT ON email_business_test.notices TO email_runtime;",
    )
    .execute(&owner)
    .await
    .unwrap();
    let runtime = PgPoolOptions::new()
        .max_connections(6)
        .connect_with(options.username("email_runtime"))
        .await
        .unwrap();
    let safe: bool = sqlx::query_scalar(
        "SELECT NOT (rolsuper OR rolcreatedb OR rolcreaterole OR rolreplication OR rolbypassrls)
         AND NOT pg_has_role(current_user,'lmm_core_owner','MEMBER')
         FROM pg_roles WHERE rolname=current_user",
    )
    .fetch_one(&runtime)
    .await
    .unwrap();
    assert!(safe);
    let store = IdentityStore::from_pool(runtime.clone())
        .with_registration_enabled(true)
        .with_mfa_keys("test".into(), vec![("test".into(), vec![47; 32])])
        .unwrap();
    let challenges = EmailChallenges::new(runtime.clone(), "test", [17; 32], [39; 32]).unwrap();
    let browser = BrowserAuth::new(store.clone(), &[43; 32]).unwrap();
    Fixture {
        owner,
        runtime,
        store,
        challenges,
        browser,
    }
}

fn address(value: &str) -> EmailAddress {
    EmailAddress::parse(value).unwrap()
}

fn confirmation<'a>(
    email: &'a EmailAddress,
    proof: &'a IssuedEmailChallenge,
) -> EmailConfirmation<'a> {
    EmailConfirmation {
        email,
        code: &proof.code,
    }
}

async fn issued(f: &Fixture, request: Request<'_>, email: &EmailAddress) -> IssuedEmailChallenge {
    // Business tests advance only synthetic issuance-window fixtures to avoid
    // real minute-long sleeps. Challenge TTL/attempts/verification stay intact.
    sqlx::query(
        "UPDATE core_identity.email_rate_events SET occurred_at=occurred_at-interval '61 seconds'
         WHERE kind='issue'",
    )
    .execute(&f.owner)
    .await
    .unwrap();
    match f.challenges.issue(request, email).await.unwrap() {
        IssueOutcome::Issued(proof) => proof,
        _ => panic!("expected a synthetic challenge"),
    }
}

async fn finish(pending: PendingEmailBusiness) -> i64 {
    let (mut mutation, notice) = pending.into_parts();
    // A real SQL insert/failure on the proof connection demonstrates the
    // notification integration boundary. This marker sends no mail.
    sqlx::query(
        "INSERT INTO email_business_test.notices(challenge_id,user_id,change) VALUES ($1,$2,$3)",
    )
    .bind(&notice.challenge_id)
    .bind(notice.user_id)
    .bind(format!("{:?}", notice.change))
    .execute(mutation.connection())
    .await
    .unwrap();
    mutation.finish(notice.user_id).await.unwrap();
    notice.user_id
}

async fn register(f: &Fixture, username: &str, email: &EmailAddress) -> i64 {
    let proof = issued(f, Request::Register, email).await;
    finish(
        f.store
            .register_with_email(
                &f.challenges,
                &EmailRegistration {
                    username,
                    password: PASSWORD,
                    confirmation: confirmation(email, &proof),
                },
            )
            .await
            .unwrap(),
    )
    .await
}

fn access(bundle: &BrowserBundle) -> &str {
    bundle.data["access_token"].as_str().unwrap()
}

fn sid(bundle: &BrowserBundle) -> &str {
    bundle.data["session"]["sid"].as_str().unwrap()
}

async fn login(f: &Fixture, username: &str, password: &str) -> BrowserBundle {
    f.browser
        .login(username, password, &BrowserSessionMetadata::default())
        .await
        .unwrap()
}

async fn state(f: &Fixture, user: i64) -> (String, i64, bool, String, i64, bool) {
    sqlx::query_as(
        "SELECT p.password_hash,u.auth_version,pr.legacy_bcrypt_allowed,
             pr.email,pr.email_version,pr.second_factor_required
         FROM core_identity.users u JOIN core_identity.user_profiles pr ON pr.user_id=u.id
         JOIN core_identity.password_logins p ON p.user_id=u.id WHERE u.id=$1",
    )
    .bind(user)
    .fetch_one(&f.owner)
    .await
    .unwrap()
}

async fn challenge_state(f: &Fixture, proof: &IssuedEmailChallenge) -> (String, i16) {
    sqlx::query_as("SELECT state,attempts FROM core_identity.email_challenges WHERE id=$1")
        .bind(&proof.id)
        .fetch_one(&f.owner)
        .await
        .unwrap()
}

#[tokio::test]
#[ignore = "requires the disposable PostgreSQL runner"]
async fn registration_is_atomic_unique_and_policy_bound() {
    let f = fixture().await;
    f.browser
        .register(&BrowserRegistration {
            username: "occupied",
            password: PASSWORD,
            email: "",
        })
        .await
        .unwrap();
    let email = address(" Registered+Tag@Example.TEST ");
    let proof = issued(&f, Request::Register, &email).await;
    let counts: (i64, i64, i64) = sqlx::query_as(
        "SELECT (SELECT count(*) FROM core_identity.accounts),
                (SELECT count(*) FROM core_identity.users),
                (SELECT count(*) FROM core_billing.account_policies)",
    )
    .fetch_one(&f.owner)
    .await
    .unwrap();
    let input = EmailRegistration {
        username: "occupied",
        password: PASSWORD,
        confirmation: confirmation(&email, &proof),
    };
    assert!(matches!(
        f.store.register_with_email(&f.challenges, &input).await,
        Err(EmailBusinessError::Identity(IdentityError::Conflict))
    ));
    let after: (i64, i64, i64) = sqlx::query_as(
        "SELECT (SELECT count(*) FROM core_identity.accounts),
                (SELECT count(*) FROM core_identity.users),
                (SELECT count(*) FROM core_billing.account_policies)",
    )
    .fetch_one(&f.owner)
    .await
    .unwrap();
    assert_eq!(after, counts, "failed registration left orphan records");
    assert_eq!(challenge_state(&f, &proof).await, ("active".into(), 0));
    let gated = f.store.clone().with_registration_enabled(false);
    assert!(matches!(
        gated.register_with_email(&f.challenges, &input).await,
        Err(EmailBusinessError::Identity(IdentityError::Forbidden))
    ));
    let domain_gated = f
        .store
        .clone()
        .with_registration_email_policy(true, &["other.test".into()])
        .unwrap();
    assert!(matches!(
        domain_gated
            .register_with_email(&f.challenges, &input)
            .await,
        Err(EmailBusinessError::Identity(IdentityError::Forbidden))
    ));
    let required = f.store.clone().with_registration_email_verification(true);
    let user = finish(
        required
            .register_with_email(
                &f.challenges,
                &EmailRegistration {
                    username: "registered",
                    ..input
                },
            )
            .await
            .unwrap(),
    )
    .await;
    let row = sqlx::query(
        "SELECT u.platform_role,u.auth_version,pr.email,pr.email_version,pr.email_login_allowed,
             e.state,p.password_hash FROM core_identity.users u
         JOIN core_identity.user_profiles pr ON pr.user_id=u.id
         JOIN core_identity.email_addresses e ON e.user_id=u.id
         JOIN core_identity.password_logins p ON p.user_id=u.id WHERE u.id=$1",
    )
    .bind(user)
    .fetch_one(&f.owner)
    .await
    .unwrap();
    assert_eq!(row.get::<String, _>("platform_role"), "user");
    assert_eq!(row.get::<i64, _>("auth_version"), 1);
    assert_eq!(row.get::<i64, _>("email_version"), 1);
    assert_eq!(row.get::<String, _>("email"), email.as_str());
    assert_eq!(row.get::<String, _>("state"), "verified");
    assert!(row.get::<bool, _>("email_login_allowed"));
    assert!(
        row.get::<String, _>("password_hash")
            .starts_with("$argon2id$v=19$m=19456,t=2,p=1$")
    );
    login(&f, "registered", PASSWORD).await;
    login(&f, email.as_str(), PASSWORD).await;
    assert_eq!(challenge_state(&f, &proof).await.0, "consumed");
    assert!(matches!(
        required
            .register_with_email(
                &f.challenges,
                &EmailRegistration {
                    username: "replay",
                    password: PASSWORD,
                    confirmation: confirmation(&email, &proof),
                },
            )
            .await,
        Err(EmailBusinessError::Proof(EmailError::InvalidProof))
    ));
    let email_username = address("name@example.test");
    register(&f, email_username.as_str(), &email_username).await;
    login(&f, email_username.as_str(), PASSWORD).await;
}

#[tokio::test]
#[ignore = "requires the disposable PostgreSQL runner"]
async fn reset_invalidates_real_browser_native_key_and_refresh() {
    let f = fixture().await;
    let email = address("reset@example.test");
    let user = register(&f, "reset", &email).await;
    let web = login(&f, "reset", PASSWORD).await;
    let canonical: String =
        sqlx::query_scalar("SELECT login_name FROM core_identity.password_logins WHERE user_id=$1")
            .bind(user)
            .fetch_one(&f.owner)
            .await
            .unwrap();
    let native = f
        .store
        .login(&LoginRequest {
            login_name: canonical,
            password: PASSWORD.into(),
        })
        .await
        .unwrap();
    // Key issuance requires a real developer grant; the runtime role cannot
    // supply it. Only this disposable fixture's installer creates the grant.
    sqlx::query(
        "INSERT INTO core_identity.developer_access(user_id,granted,source_reference)
         VALUES ($1,TRUE,'synthetic-email-reset-test')",
    )
    .bind(user)
    .execute(&f.owner)
    .await
    .unwrap();
    let key = f
        .store
        .issue_key(
            &native.secret,
            Account {
                kind: AccountKind::Personal,
                id: user,
            },
            FundingPolicy::new_personal_key(),
            3600,
        )
        .await
        .unwrap();
    for token in [access(&web), &native.secret, &key.secret] {
        assert!(f.store.authorize(token).await.is_ok());
    }
    sqlx::query(
        "UPDATE core_identity.user_profiles SET legacy_bcrypt_allowed=TRUE WHERE user_id=$1",
    )
    .bind(user)
    .execute(&f.owner)
    .await
    .unwrap();
    let before = state(&f, user).await;
    let proof = issued(&f, Request::PasswordReset, &email).await;
    finish(
        f.store
            .reset_password_with_email(&f.challenges, &confirmation(&email, &proof), NEW_PASSWORD)
            .await
            .unwrap(),
    )
    .await;
    let after = state(&f, user).await;
    assert_ne!(after.0, before.0);
    assert_eq!(after.1, before.1 + 1);
    assert!(!after.2);
    assert_eq!(
        (&after.3, after.4, after.5),
        (&before.3, before.4, before.5)
    );
    for token in [access(&web), &native.secret, &key.secret] {
        assert!(matches!(
            f.store.authorize(token).await,
            Err(IdentityError::Unauthorized)
        ));
    }
    assert!(
        f.browser
            .refresh(&web.refresh_token, Some(sid(&web)))
            .await
            .is_err()
    );
    assert!(
        f.browser
            .login("reset", PASSWORD, &BrowserSessionMetadata::default())
            .await
            .is_err()
    );
    login(&f, "reset", NEW_PASSWORD).await;
    assert_eq!(challenge_state(&f, &proof).await, ("consumed".into(), 0));
}

#[tokio::test]
#[ignore = "requires the disposable PostgreSQL runner"]
async fn reset_audit_notice_and_finish_failures_roll_back_every_write() {
    let f = fixture().await;
    let email = address("rollback@example.test");
    let user = register(&f, "rollback", &email).await;
    let proof = issued(&f, Request::PasswordReset, &email).await;
    let before = state(&f, user).await;
    sqlx::raw_sql(
        "CREATE FUNCTION email_business_test.reject_reset() RETURNS TRIGGER LANGUAGE plpgsql AS $$
         BEGIN IF NEW.action='identity.email_password_reset' THEN
             RAISE EXCEPTION 'synthetic audit rejection'; END IF; RETURN NEW; END $$;
         CREATE TRIGGER reject_reset BEFORE INSERT ON core_identity.audit
             FOR EACH ROW EXECUTE FUNCTION email_business_test.reject_reset();",
    )
    .execute(&f.owner)
    .await
    .unwrap();
    assert!(
        f.store
            .reset_password_with_email(&f.challenges, &confirmation(&email, &proof), NEW_PASSWORD)
            .await
            .is_err()
    );
    assert_eq!(state(&f, user).await, before);
    assert_eq!(challenge_state(&f, &proof).await, ("active".into(), 0));
    sqlx::query("DROP TRIGGER reject_reset ON core_identity.audit")
        .execute(&f.owner)
        .await
        .unwrap();
    let pending = f
        .store
        .reset_password_with_email(&f.challenges, &confirmation(&email, &proof), NEW_PASSWORD)
        .await
        .unwrap();
    let (mut mutation, _) = pending.into_parts();
    // The failed required enqueue aborts the very same transaction.
    assert!(
        sqlx::query("INSERT INTO email_business_test.notices VALUES ($1,NULL,'reset')")
            .bind(&proof.id)
            .execute(mutation.connection())
            .await
            .is_err()
    );
    assert!(mutation.finish(user).await.is_err());
    assert_eq!(state(&f, user).await, before);
    assert_eq!(challenge_state(&f, &proof).await, ("active".into(), 0));
    let pending = f
        .store
        .reset_password_with_email(&f.challenges, &confirmation(&email, &proof), NEW_PASSWORD)
        .await
        .unwrap();
    drop(pending);
    assert_eq!(state(&f, user).await, before);
    assert_eq!(challenge_state(&f, &proof).await, ("active".into(), 0));
    finish(
        f.store
            .reset_password_with_email(&f.challenges, &confirmation(&email, &proof), NEW_PASSWORD)
            .await
            .unwrap(),
    )
    .await;
}

#[tokio::test]
#[ignore = "requires the disposable PostgreSQL runner"]
async fn reset_requires_current_verified_authority_and_enabled_existing_password() {
    let f = fixture().await;
    f.browser
        .register(&BrowserRegistration {
            username: "legacy",
            password: PASSWORD,
            email: "",
        })
        .await
        .unwrap();
    sqlx::query(
        "UPDATE core_identity.user_profiles SET email='legacy@example.test',email_login_allowed=TRUE
         WHERE username='legacy'",
    )
    .execute(&f.owner)
    .await
    .unwrap();
    assert!(matches!(
        f.challenges
            .issue(Request::PasswordReset, &address("legacy@example.test"))
            .await
            .unwrap(),
        IssueOutcome::Suppressed
    ));
    let email = address("current@example.test");
    let user = register(&f, "current", &email).await;
    let proof = issued(&f, Request::PasswordReset, &email).await;
    let before = state(&f, user).await;
    sqlx::query(
        "UPDATE core_identity.email_addresses SET email_version=email_version+1 WHERE user_id=$1",
    )
    .bind(user)
    .execute(&f.owner)
    .await
    .unwrap();
    assert!(
        f.store
            .reset_password_with_email(&f.challenges, &confirmation(&email, &proof), NEW_PASSWORD)
            .await
            .is_err()
    );
    let after = state(&f, user).await;
    assert_eq!(after.0, before.0);
    assert_eq!(challenge_state(&f, &proof).await, ("active".into(), 0));
    for (name, sql) in [
        (
            "password_disabled",
            "UPDATE core_identity.user_profiles SET password_login_allowed=FALSE WHERE user_id=$1",
        ),
        (
            "user_inactive",
            "UPDATE core_identity.users SET active=FALSE WHERE id=$1",
        ),
        (
            "account_inactive",
            "UPDATE core_identity.accounts SET active=FALSE
             WHERE id=(SELECT personal_account_id FROM core_identity.users WHERE id=$1)",
        ),
    ] {
        let email = address(&format!("{name}@example.test"));
        let subject = register(&f, name, &email).await;
        let token = issued(&f, Request::PasswordReset, &email).await;
        let hash_before = state(&f, subject).await.0;
        sqlx::query(sql)
            .bind(subject)
            .execute(&f.owner)
            .await
            .unwrap();
        assert!(
            f.store
                .reset_password_with_email(
                    &f.challenges,
                    &confirmation(&email, &token),
                    NEW_PASSWORD,
                )
                .await
                .is_err()
        );
        assert_eq!(state(&f, subject).await.0, hash_before);
        assert_eq!(challenge_state(&f, &token).await, ("active".into(), 0));
    }
    let provider_email = address("provider@example.test");
    let provider = register(&f, "provider", &provider_email).await;
    sqlx::query("DELETE FROM core_identity.password_logins WHERE user_id=$1")
        .bind(provider)
        .execute(&f.owner)
        .await
        .unwrap();
    sqlx::query(
        "UPDATE core_identity.email_rate_events SET occurred_at=occurred_at-interval '61 seconds'",
    )
    .execute(&f.owner)
    .await
    .unwrap();
    assert!(matches!(
        f.challenges
            .issue(Request::PasswordReset, &provider_email)
            .await
            .unwrap(),
        IssueOutcome::Suppressed
    ));
}

#[tokio::test]
#[ignore = "requires the disposable PostgreSQL runner"]
async fn concurrent_reset_consumes_once_and_advances_one_generation() {
    let f = fixture().await;
    let email = address("race@example.test");
    let user = register(&f, "race", &email).await;
    let proof = issued(&f, Request::PasswordReset, &email).await;
    let before = state(&f, user).await;
    let confirm = confirmation(&email, &proof);
    let store = &f.store;
    let challenges = &f.challenges;
    let confirm = &confirm;
    let attempt = |password| async move {
        match store
            .reset_password_with_email(challenges, confirm, password)
            .await
        {
            Ok(pending) => {
                finish(pending).await;
                true
            }
            Err(EmailBusinessError::Proof(EmailError::InvalidProof | EmailError::Conflict))
            | Err(EmailBusinessError::Identity(IdentityError::Conflict)) => false,
            Err(error) => panic!("unexpected synthetic race result: {error}"),
        }
    };
    let (first, second) = tokio::join!(attempt("new pass alpha"), attempt("new pass beta"));
    assert_ne!(first, second);
    let after = state(&f, user).await;
    assert_eq!(after.1, before.1 + 1);
    let notices: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM email_business_test.notices WHERE challenge_id=$1",
    )
    .bind(&proof.id)
    .fetch_one(&f.owner)
    .await
    .unwrap();
    assert_eq!(notices, 1);
    login(
        &f,
        "race",
        if first {
            "new pass alpha"
        } else {
            "new pass beta"
        },
    )
    .await;
}

#[tokio::test]
#[ignore = "requires the disposable PostgreSQL runner"]
async fn binding_is_transactional_and_retires_old_email_proofs() {
    let f = fixture().await;
    let old = address("old@example.test");
    let new = address("new@example.test");
    let user = register(&f, "binding", &old).await;
    let web = login(&f, "binding", PASSWORD).await;
    let old_reset = issued(&f, Request::PasswordReset, &old).await;
    let proof = issued(
        &f,
        Request::Bind {
            access: access(&web),
            sid: sid(&web),
        },
        &new,
    )
    .await;
    let before = state(&f, user).await;
    let pending = f
        .store
        .bind_verified_email(
            &f.challenges,
            access(&web),
            sid(&web),
            &confirmation(&new, &proof),
        )
        .await
        .unwrap();
    let (mutation, notice) = pending.into_parts();
    assert_eq!(notice.change, EmailSecurityChange::Bound);
    assert_eq!(notice.previous_verified_email.as_ref(), Some(&old));
    drop(mutation);
    assert_eq!(state(&f, user).await, before);
    assert_eq!(challenge_state(&f, &proof).await, ("active".into(), 0));
    finish(
        f.store
            .bind_verified_email(
                &f.challenges,
                access(&web),
                sid(&web),
                &confirmation(&new, &proof),
            )
            .await
            .unwrap(),
    )
    .await;
    let after = state(&f, user).await;
    assert_eq!(
        (&after.0, after.1, after.2, after.5),
        (&before.0, before.1, before.2, before.5)
    );
    assert_eq!(after.3, new.as_str());
    assert_eq!(after.4, before.4 + 1);
    assert!(f.store.authorize(access(&web)).await.is_ok());
    assert!(
        f.store
            .reset_password_with_email(
                &f.challenges,
                &confirmation(&old, &old_reset),
                NEW_PASSWORD,
            )
            .await
            .is_err()
    );
    assert!(
        f.store
            .bind_verified_email(
                &f.challenges,
                access(&web),
                sid(&web),
                &confirmation(&new, &proof)
            )
            .await
            .is_err()
    );
}

#[tokio::test]
#[ignore = "requires the disposable PostgreSQL runner"]
async fn reset_keeps_real_mfa_role_and_rejects_pre_reset_mfa_challenge() {
    let f = fixture().await;
    let email = address("mfa@example.test");
    let user = register(&f, "mfa", &email).await;
    sqlx::query("UPDATE core_identity.users SET platform_role='admin' WHERE id=$1")
        .bind(user)
        .execute(&f.owner)
        .await
        .unwrap();
    let web = login(&f, "mfa", PASSWORD).await;
    let setup = f.browser.mfa_setup(access(&web)).await.unwrap();
    let epoch: i64 =
        sqlx::query_scalar("SELECT floor(extract(epoch FROM clock_timestamp()))::bigint")
            .fetch_one(&f.owner)
            .await
            .unwrap();
    let secret = Secret::try_from_base32(&setup.secret).unwrap();
    let code = Builder::new()
        .with_algorithm(Algorithm::SHA1)
        .with_digits(6)
        .with_step_duration(30)
        .with_skew(1)
        .with_secret(secret.as_bytes())
        .build()
        .unwrap()
        .generate(u64::try_from(epoch).unwrap())
        .to_string();
    let assured = f.browser.mfa_enable(access(&web), &code).await.unwrap();
    let stale = match f
        .browser
        .login_with_mfa("mfa", PASSWORD, &BrowserSessionMetadata::default(), BINDING)
        .await
        .unwrap()
    {
        BrowserLoginOutcome::Challenge(challenge) => challenge,
        _ => panic!("MFA login must not return a session"),
    };
    let factors: String = sqlx::query_scalar(
        "SELECT row_to_json(m)::text FROM core_identity.mfa_factors m WHERE user_id=$1",
    )
    .bind(user)
    .fetch_one(&f.owner)
    .await
    .unwrap();
    let proof = issued(&f, Request::PasswordReset, &email).await;
    finish(
        f.store
            .reset_password_with_email(&f.challenges, &confirmation(&email, &proof), NEW_PASSWORD)
            .await
            .unwrap(),
    )
    .await;
    let persisted: (String, bool) = sqlx::query_as(
        "SELECT u.platform_role,pr.second_factor_required FROM core_identity.users u
         JOIN core_identity.user_profiles pr ON pr.user_id=u.id WHERE u.id=$1",
    )
    .bind(user)
    .fetch_one(&f.owner)
    .await
    .unwrap();
    assert_eq!(persisted, ("admin".into(), true));
    let after_factors: String = sqlx::query_scalar(
        "SELECT row_to_json(m)::text FROM core_identity.mfa_factors m WHERE user_id=$1",
    )
    .bind(user)
    .fetch_one(&f.owner)
    .await
    .unwrap();
    assert_eq!(after_factors, factors);
    assert!(f.store.authorize(access(&assured)).await.is_err());
    assert!(
        f.browser
            .complete_mfa_login(
                &MfaCompletion {
                    flow_token: stale.flow_token,
                    code: setup.backup_codes[0].clone(),
                },
                BINDING,
            )
            .await
            .is_err()
    );
    let fresh = match f
        .browser
        .login_with_mfa(
            "mfa",
            NEW_PASSWORD,
            &BrowserSessionMetadata::default(),
            BINDING,
        )
        .await
        .unwrap()
    {
        BrowserLoginOutcome::Challenge(challenge) => challenge,
        _ => panic!("password reset bypassed MFA"),
    };
    let complete = f
        .browser
        .complete_mfa_login(
            &MfaCompletion {
                flow_token: fresh.flow_token,
                code: setup.backup_codes[0].clone(),
            },
            BINDING,
        )
        .await
        .unwrap();
    assert!(f.store.authorize(access(&complete)).await.is_ok());
}

#[tokio::test]
#[ignore = "requires the disposable PostgreSQL runner"]
async fn invalid_password_and_wrong_code_do_not_mutate_identity() {
    let f = fixture().await;
    let email = address("wrong@example.test");
    let user = register(&f, "wrong", &email).await;
    let proof = issued(&f, Request::PasswordReset, &email).await;
    let before = state(&f, user).await;
    assert!(matches!(
        f.store
            .reset_password_with_email(&f.challenges, &confirmation(&email, &proof), "short")
            .await,
        Err(EmailBusinessError::Identity(IdentityError::Invalid))
    ));
    let wrong = EmailConfirmation {
        email: &email,
        code: "invalid",
    };
    assert!(matches!(
        f.store
            .reset_password_with_email(&f.challenges, &wrong, NEW_PASSWORD)
            .await,
        Err(EmailBusinessError::Proof(EmailError::InvalidProof))
    ));
    assert_eq!(state(&f, user).await, before);
    assert_eq!(challenge_state(&f, &proof).await, ("active".into(), 1));
    let cannot_ddl = sqlx::query("CREATE TABLE public.email_business_forbidden(id int)")
        .execute(&f.runtime)
        .await;
    assert!(cannot_ddl.is_err());
}

#[path = "../email_http/cases.rs"]
mod http_integration;

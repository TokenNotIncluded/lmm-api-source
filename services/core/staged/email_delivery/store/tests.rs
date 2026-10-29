use super::super::{MailKeys, ResetSite, SmtpConfig, SmtpMailer, SmtpOutcome, SmtpTls};
use super::*;
use crate::identity::email::{EmailAddress, EmailChallenges, IssueOutcome, Request};
use sha2::{Digest, Sha256};
use sqlx::{PgPool, Row, postgres::PgPoolOptions};
use std::time::Duration;
use tokio::{
    io::{AsyncBufReadExt, AsyncWriteExt, BufReader},
    net::TcpListener,
};

const BASE: &str = include_str!("../../../schema/identity.sql");

fn address(raw: &str) -> EmailAddress {
    EmailAddress::parse(raw).expect("valid synthetic mailbox")
}

fn store(pool: PgPool) -> EmailChallenges {
    EmailChallenges::new(pool, "proof-test", [17; 32], [39; 32]).expect("synthetic keys")
}

fn outbox(pool: PgPool) -> Outbox {
    Outbox::new(
        pool,
        MailKeys::new("mail-test", vec![("mail-test".into(), vec![71; 32])])
            .expect("independent synthetic mail key"),
    )
}

async fn challenge_id(pool: &PgPool, email: &EmailAddress) -> String {
    sqlx::query_scalar(
        "SELECT id FROM core_identity.email_challenges WHERE email_key=$1
         ORDER BY issued_at DESC LIMIT 1",
    )
    .bind(email.as_str())
    .fetch_one(pool)
    .await
    .unwrap()
}

async fn attempt(pool: &PgPool, email: &EmailAddress) -> Option<String> {
    sqlx::query_scalar(
        "SELECT id FROM core_identity.email_delivery_outbox
         WHERE challenge_id=(SELECT id FROM core_identity.email_challenges WHERE email_key=$1
             ORDER BY issued_at DESC LIMIT 1)
             AND kind='challenge' ORDER BY created_at DESC,id DESC LIMIT 1",
    )
    .bind(email.as_str())
    .fetch_optional(pool)
    .await
    .unwrap()
}

async fn pool(variable: &str) -> PgPool {
    let url = std::env::var(variable).expect("run the isolated email delivery fixture");
    assert!(
        url.contains("/email_slice?host=/tmp/lmm-email-pg-"),
        "refuse a database outside this disposable fixture"
    );
    PgPoolOptions::new()
        .max_connections(8)
        .connect(&url)
        .await
        .expect("fixture database")
}

async fn fresh() -> (PgPool, PgPool, EmailChallenges, Outbox) {
    let owner = pool("EMAIL_TEST_OWNER_URL").await;
    sqlx::raw_sql(
        "DROP SCHEMA IF EXISTS core_identity CASCADE;
         DROP SCHEMA IF EXISTS core_billing CASCADE;
         DROP SCHEMA IF EXISTS core_meta CASCADE;",
    )
    .execute(&owner)
    .await
    .expect("reset disposable fixture");
    sqlx::raw_sql(BASE)
        .execute(&owner)
        .await
        .expect("base identity schema");
    sqlx::raw_sql(include_str!("../../../src/identity/email/test_grants.sql"))
        .execute(&owner)
        .await
        .expect("challenge test grants");
    sqlx::raw_sql(include_str!("../test_grants.sql"))
        .execute(&owner)
        .await
        .expect("outbox test grants");
    let runtime = pool("EMAIL_TEST_RUNTIME_URL").await;
    let roles: (bool, bool, bool) = sqlx::query_as(
        "SELECT rolsuper,rolcreatedb,rolcreaterole FROM pg_roles WHERE rolname=current_user",
    )
    .fetch_one(&runtime)
    .await
    .expect("runtime role flags");
    assert_eq!(roles, (false, false, false));
    let challenge = store(runtime.clone());
    let delivery = outbox(runtime.clone());
    (owner, runtime, challenge, delivery)
}

#[derive(Clone, Copy)]
enum SinkReply {
    Accepted,
    TemporaryReject,
    PermanentReject,
    LostDataReply,
}

async fn smtp_sink(mode: SinkReply) -> (u16, tokio::task::JoinHandle<String>) {
    let listener = TcpListener::bind("127.0.0.1:0").await.expect("local sink");
    let port = listener.local_addr().expect("sink address").port();
    let task = tokio::spawn(async move {
        let (stream, _) = listener.accept().await.expect("SMTP connection");
        let mut stream = BufReader::new(stream);
        stream
            .get_mut()
            .write_all(b"220 local.test ESMTP\r\n")
            .await
            .expect("greeting");
        let mut message = String::new();
        loop {
            let mut line = String::new();
            if stream.read_line(&mut line).await.expect("SMTP command") == 0 {
                break;
            }
            let response = if line.starts_with("EHLO") {
                "250-local.test\r\n250 PIPELINING\r\n"
            } else if line.starts_with("MAIL FROM:") {
                "250 sender accepted\r\n"
            } else if line.starts_with("RCPT TO:") {
                match mode {
                    SinkReply::TemporaryReject => "451 recipient deferred\r\n",
                    SinkReply::PermanentReject => "550 recipient rejected\r\n",
                    _ => "250 recipient accepted\r\n",
                }
            } else if line.starts_with("DATA") {
                stream
                    .get_mut()
                    .write_all(b"354 send data\r\n")
                    .await
                    .expect("DATA prompt");
                loop {
                    let mut body = String::new();
                    if stream.read_line(&mut body).await.expect("message body") == 0 {
                        return message;
                    }
                    if body == ".\r\n" {
                        break;
                    }
                    message.push_str(&body);
                }
                match mode {
                    SinkReply::Accepted => "250 queued\r\n",
                    SinkReply::TemporaryReject => unreachable!(),
                    SinkReply::PermanentReject => unreachable!(),
                    SinkReply::LostDataReply => return message,
                }
            } else if line.starts_with("QUIT") {
                stream
                    .get_mut()
                    .write_all(b"221 bye\r\n")
                    .await
                    .expect("QUIT response");
                break;
            } else {
                panic!("unexpected SMTP command class");
            };
            stream
                .get_mut()
                .write_all(response.as_bytes())
                .await
                .expect("SMTP response");
        }
        message
    });
    (port, task)
}

fn local_mailer(port: u16) -> SmtpMailer {
    SmtpMailer::open(
        &SmtpConfig {
            host: "127.0.0.1".into(),
            port,
            from: "sender@example.test".into(),
            tls: SmtpTls::PlaintextLoopbackTest,
            username: None,
            password_file: None,
        },
        ResetSite::https("https://mail-web.example.test").expect("synthetic Web origin"),
    )
    .expect("loopback-only SMTP configuration")
}

#[tokio::test]
#[ignore = "requires the isolated email delivery fixture"]
async fn real_smtp_sink_distinguishes_acceptance_rejection_and_lost_ack() {
    for (index, mode, expected) in [
        (1, SinkReply::Accepted, SmtpOutcome::Accepted),
        (
            2,
            SinkReply::TemporaryReject,
            SmtpOutcome::RejectedTemporary,
        ),
        (
            3,
            SinkReply::PermanentReject,
            SmtpOutcome::RejectedPermanent,
        ),
        (4, SinkReply::LostDataReply, SmtpOutcome::Unknown),
    ] {
        let (_owner, _runtime, challenge, delivery) = fresh().await;
        let email = address(&format!("recipient{index}@example.test"));
        let (port, capture) = smtp_sink(mode).await;
        let mailer = local_mailer(port);
        delivery
            .issue(&challenge, Request::Register, &email)
            .await
            .unwrap();
        let attempt_id = attempt(&_runtime, &email).await.unwrap();
        let lease = delivery.claim().await.unwrap().unwrap();
        assert_eq!(lease.attempt_id(), attempt_id);
        assert_eq!(
            delivery.dispatch(lease, &mailer).await.unwrap(),
            Dispatch::Finished(expected)
        );
        let message = tokio::time::timeout(Duration::from_secs(3), capture)
            .await
            .unwrap()
            .unwrap();
        match mode {
            SinkReply::Accepted | SinkReply::LostDataReply => {
                assert!(message.contains("To:"));
                assert!(message.contains("verification code"));
            }
            SinkReply::TemporaryReject | SinkReply::PermanentReject => {
                assert!(message.is_empty(), "RCPT rejection must precede DATA");
            }
        }
        assert!(!message.contains("password="));
        let state: String = sqlx::query_scalar(
            "SELECT status FROM core_identity.email_delivery_outbox WHERE id=$1",
        )
        .bind(&attempt_id)
        .fetch_one(&_runtime)
        .await
        .unwrap();
        let wanted = match expected {
            SmtpOutcome::Accepted => "accepted_by_smtp",
            SmtpOutcome::RejectedTemporary | SmtpOutcome::RejectedPermanent => "rejected",
            SmtpOutcome::Unknown => "unknown",
        };
        assert_eq!(state, wanted);
        assert!(delivery.claim().await.unwrap().is_none());
    }
}

#[tokio::test]
#[ignore = "requires the isolated email delivery fixture"]
async fn bind_delivery_rechecks_new_address_occupancy_before_smtp() {
    let (owner, runtime, challenges, delivery) = fresh().await;
    for id in [1_i64, 2] {
        let account: i64 = sqlx::query_scalar(
            "INSERT INTO core_identity.accounts(kind) VALUES ('personal') RETURNING id",
        )
        .fetch_one(&owner)
        .await
        .unwrap();
        sqlx::query("INSERT INTO core_identity.users(id,personal_account_id) VALUES ($1,$2)")
            .bind(id)
            .bind(account)
            .execute(&owner)
            .await
            .unwrap();
        sqlx::query("INSERT INTO core_identity.user_profiles(user_id,username) VALUES ($1,$2)")
            .bind(id)
            .bind(format!("bindtest{id}"))
            .execute(&owner)
            .await
            .unwrap();
    }
    let sid = "b".repeat(64);
    let access = "synthetic-bind-session";
    let credential: i64 = sqlx::query_scalar(
        "INSERT INTO core_identity.credentials(digest,kind,user_id,user_version,owner_account_id,expires_at)
         SELECT $1,'session',id,auth_version,personal_account_id,clock_timestamp()+interval '15 minutes'
         FROM core_identity.users WHERE id=1 RETURNING id",
    )
    .bind(Sha256::digest(access.as_bytes()).to_vec())
    .fetch_one(&owner)
    .await
    .unwrap();
    sqlx::query(
        "INSERT INTO core_identity.web_sessions(sid,user_id,user_version,current_credential_id,refresh_digest,ip,user_agent)
         SELECT $1,id,auth_version,$2,$3,'127.0.0.1','outbox-selftest' FROM core_identity.users WHERE id=1",
    )
    .bind(&sid).bind(credential).bind(vec![21_u8; 32])
    .execute(&owner).await.unwrap();
    sqlx::query(
        "INSERT INTO core_identity.session_authentication(credential_id,user_id,user_version,factor_revision,
             factor_level,method,authenticated_at)
         SELECT $1,id,auth_version,0,1,'primary',clock_timestamp() FROM core_identity.users WHERE id=1",
    )
    .bind(credential).execute(&owner).await.unwrap();
    let email = address("occupied-after-issue@example.test");
    delivery
        .issue(&challenges, Request::Bind { access, sid: &sid }, &email)
        .await
        .unwrap();
    let id = attempt(&runtime, &email)
        .await
        .expect("eligible bind queued");
    sqlx::query("UPDATE core_identity.user_profiles SET email=$1 WHERE user_id=2")
        .bind(email.as_str())
        .execute(&owner)
        .await
        .unwrap();
    let lease = delivery.claim().await.unwrap().unwrap();
    let (port, capture) = smtp_sink(SinkReply::Accepted).await;
    assert_eq!(
        delivery.dispatch(lease, &local_mailer(port)).await.unwrap(),
        Dispatch::LostLease
    );
    let terminal: (String, bool) = sqlx::query_as(
        "SELECT status,payload IS NULL FROM core_identity.email_delivery_outbox WHERE id=$1",
    )
    .bind(id)
    .fetch_one(&runtime)
    .await
    .unwrap();
    assert_eq!(terminal, ("canceled".into(), true));
    assert!(
        tokio::time::timeout(Duration::from_millis(200), capture)
            .await
            .is_err(),
        "ineligible mail must not open an SMTP connection"
    );
}

#[tokio::test]
#[ignore = "requires the isolated email delivery fixture"]
async fn challenge_budget_and_ciphertext_rollback_with_failed_enqueue() {
    let (owner, runtime, challenge, delivery) = fresh().await;
    sqlx::raw_sql(
        "CREATE FUNCTION core_identity.reject_outbox() RETURNS trigger LANGUAGE plpgsql AS $$
         BEGIN RAISE EXCEPTION 'synthetic enqueue failure'; END $$;
         CREATE TRIGGER reject_outbox BEFORE INSERT ON core_identity.email_delivery_outbox
         FOR EACH ROW EXECUTE FUNCTION core_identity.reject_outbox();",
    )
    .execute(&owner)
    .await
    .unwrap();
    let email = address("atomic@example.test");
    assert!(
        delivery
            .issue(&challenge, Request::Register, &email)
            .await
            .is_err()
    );
    let counts: (i64, i64) = sqlx::query_as(
        "SELECT (SELECT count(*) FROM core_identity.email_challenges),
                (SELECT count(*) FROM core_identity.email_rate_events WHERE kind='issue')",
    )
    .fetch_one(&owner)
    .await
    .unwrap();
    assert_eq!(counts, (0, 0));
    sqlx::raw_sql(
        "DROP TRIGGER reject_outbox ON core_identity.email_delivery_outbox;
         DROP FUNCTION core_identity.reject_outbox();",
    )
    .execute(&owner)
    .await
    .unwrap();
    delivery
        .issue(&challenge, Request::Register, &email)
        .await
        .unwrap();
    let stored: (Vec<u8>, String) = sqlx::query_as(
        "SELECT o.payload,c.email_key FROM core_identity.email_delivery_outbox o
         JOIN core_identity.email_challenges c ON c.id=o.challenge_id",
    )
    .fetch_one(&runtime)
    .await
    .unwrap();
    assert!(
        !stored
            .0
            .windows(email.as_str().len())
            .any(|part| part == email.as_str().as_bytes())
    );
    let row = sqlx::query(
        "SELECT * FROM core_identity.email_delivery_outbox
         WHERE challenge_id=(SELECT id FROM core_identity.email_challenges WHERE email_key=$1)",
    )
    .bind(email.as_str())
    .fetch_one(&runtime)
    .await
    .unwrap();
    let code = delivery.open_payload(&row).unwrap().code.to_string();
    assert!(
        !stored
            .0
            .windows(code.len())
            .any(|part| part == code.as_bytes())
    );
    assert_eq!(stored.1, email.as_str());
}

#[tokio::test]
#[ignore = "requires the isolated email delivery fixture"]
async fn expired_lease_is_terminal_and_old_owner_cannot_finish_or_send() {
    let (owner, _runtime, challenge, delivery) = fresh().await;
    let email = address("fence@example.test");
    delivery
        .issue(&challenge, Request::Register, &email)
        .await
        .unwrap();
    let attempt_id = attempt(&owner, &email).await.unwrap();
    let old = delivery.claim().await.unwrap().unwrap();
    sqlx::query(
        "UPDATE core_identity.email_delivery_outbox SET lease_until=clock_timestamp()-interval '1 second'
         WHERE id=$1",
    ).bind(&attempt_id).execute(&owner).await.unwrap();
    assert!(!delivery.finish(&old, SmtpOutcome::Accepted).await.unwrap());
    assert_eq!(delivery.maintain().await.unwrap(), 1);
    assert!(!delivery.finish(&old, SmtpOutcome::Accepted).await.unwrap());
    let state: String =
        sqlx::query_scalar("SELECT status FROM core_identity.email_delivery_outbox WHERE id=$1")
            .bind(attempt_id)
            .fetch_one(&owner)
            .await
            .unwrap();
    assert_eq!(state, "unknown");
    let (port, capture) = smtp_sink(SinkReply::Accepted).await;
    let result = delivery.dispatch(old, &local_mailer(port)).await.unwrap();
    assert_eq!(result, Dispatch::LostLease);
    assert!(
        tokio::time::timeout(Duration::from_millis(200), capture)
            .await
            .is_err()
    );
}

#[tokio::test]
#[ignore = "requires the isolated email delivery fixture"]
async fn pending_without_cipher_is_not_sent_and_explicit_retry_keeps_original_code_and_ttl() {
    let (owner, runtime, challenge, delivery) = fresh().await;
    let email = address("pending@example.test");
    let proof = match challenge.issue(Request::Register, &email).await.unwrap() {
        IssueOutcome::Issued(proof) => proof,
        _ => panic!("first issue creates a proof"),
    };
    sqlx::query(
        "UPDATE core_identity.email_rate_events SET occurred_at=clock_timestamp()-interval '1 day'",
    )
    .execute(&owner)
    .await
    .unwrap();
    let before: i64 = sqlx::query_scalar(
        "SELECT floor(extract(epoch FROM expires_at))::bigint FROM core_identity.email_challenges WHERE id=$1",
    ).bind(&proof.id).fetch_one(&runtime).await.unwrap();
    delivery
        .issue(&challenge, Request::Register, &email)
        .await
        .unwrap();
    let mutation = challenge
        .verify_current(Request::Register, &email, &proof.code)
        .await
        .unwrap();
    drop(mutation);
    let after: i64 = sqlx::query_scalar(
        "SELECT floor(extract(epoch FROM expires_at))::bigint FROM core_identity.email_challenges WHERE id=$1",
    ).bind(&proof.id).fetch_one(&runtime).await.unwrap();
    assert_eq!(before, after);
    let stored: i64 =
        sqlx::query_scalar("SELECT count(*) FROM core_identity.email_delivery_outbox")
            .fetch_one(&runtime)
            .await
            .unwrap();
    assert_eq!(stored, 0);
}

#[tokio::test]
#[ignore = "requires the isolated email delivery fixture"]
async fn explicit_retry_reuses_the_same_code_without_extending_challenge_ttl() {
    let (owner, runtime, challenge, delivery) = fresh().await;
    let email = address("retry@example.test");
    delivery
        .issue(&challenge, Request::Register, &email)
        .await
        .unwrap();
    let challenge_row_id = challenge_id(&runtime, &email).await;
    let first = attempt(&runtime, &email).await.unwrap();
    let first_row = sqlx::query("SELECT * FROM core_identity.email_delivery_outbox WHERE id=$1")
        .bind(&first)
        .fetch_one(&runtime)
        .await
        .unwrap();
    let original = delivery.open_payload(&first_row).unwrap().code.to_string();
    let deadline: String = sqlx::query_scalar(
        "SELECT expires_at::text FROM core_identity.email_challenges WHERE id=$1",
    )
    .bind(&challenge_row_id)
    .fetch_one(&runtime)
    .await
    .unwrap();
    let (port, captured) = smtp_sink(SinkReply::Accepted).await;
    let lease = delivery.claim().await.unwrap().unwrap();
    assert_eq!(
        delivery.dispatch(lease, &local_mailer(port)).await.unwrap(),
        Dispatch::Finished(SmtpOutcome::Accepted)
    );
    let message = captured.await.unwrap();
    assert!(message.contains(&original));
    sqlx::query(
        "UPDATE core_identity.email_rate_events
         SET occurred_at=clock_timestamp()-interval '61 seconds' WHERE kind='issue'",
    )
    .execute(&owner)
    .await
    .unwrap();
    delivery
        .issue(&challenge, Request::Register, &email)
        .await
        .unwrap();
    let second = attempt(&runtime, &email).await.unwrap();
    assert_ne!(first, second);
    let ids: (String, String) = sqlx::query_as(
        "SELECT o.challenge_id,c.expires_at::text
         FROM core_identity.email_delivery_outbox o JOIN core_identity.email_challenges c
             ON o.challenge_id=c.id WHERE o.id=$1",
    )
    .bind(&second)
    .fetch_one(&runtime)
    .await
    .unwrap();
    assert_eq!(ids.1, deadline);
    let second_row = sqlx::query("SELECT * FROM core_identity.email_delivery_outbox WHERE id=$1")
        .bind(&second)
        .fetch_one(&runtime)
        .await
        .unwrap();
    assert_eq!(
        delivery.open_payload(&second_row).unwrap().code.as_str(),
        original
    );
    assert!(delivery.claim().await.unwrap().is_some());
    assert_eq!(
        ids.0,
        first_row.try_get::<String, _>("challenge_id").unwrap()
    );
}

#[tokio::test]
#[ignore = "requires the isolated email delivery fixture"]
async fn scope_verification_needs_no_client_id_and_charges_missing_and_wrong_proofs() {
    let (_owner, runtime, challenges, delivery) = fresh().await;
    let missing = address("missing@example.test");
    assert!(matches!(
        challenges
            .verify_current(Request::Register, &missing, "000000")
            .await,
        Err(crate::identity::email::EmailError::InvalidProof)
    ));
    let missing_budget: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM core_identity.email_rate_events WHERE kind='verify'",
    )
    .fetch_one(&runtime)
    .await
    .unwrap();
    assert_eq!(missing_budget, 1);

    let email = address("scope@example.test");
    delivery
        .issue(&challenges, Request::Register, &email)
        .await
        .unwrap();
    let id = challenge_id(&runtime, &email).await;
    let row = sqlx::query(
        "SELECT o.* FROM core_identity.email_delivery_outbox o
         JOIN core_identity.email_challenges c ON c.id=o.challenge_id WHERE c.id=$1",
    )
    .bind(&id)
    .fetch_one(&runtime)
    .await
    .unwrap();
    let code = delivery.open_payload(&row).unwrap().code.to_string();
    let wrong = if code == "000000" { "000001" } else { "000000" };
    assert!(matches!(
        challenges
            .verify_current(Request::Register, &email, wrong)
            .await,
        Err(crate::identity::email::EmailError::InvalidProof)
    ));
    let mutation = challenges
        .verify_current(Request::Register, &email, &code)
        .await
        .unwrap();
    assert_eq!(mutation.challenge_id(), id);
    drop(mutation);
    let attempts: (i16, String) =
        sqlx::query_as("SELECT attempts,state FROM core_identity.email_challenges WHERE id=$1")
            .bind(id)
            .fetch_one(&runtime)
            .await
            .unwrap();
    assert_eq!(attempts, (1, "active".into()));
}

#[tokio::test]
#[ignore = "requires the isolated email delivery fixture"]
async fn ciphertext_is_context_bound_and_terminal_challenge_erases_queued_code() {
    let (owner, runtime, challenge, delivery) = fresh().await;
    let email = address("invalidate@example.test");
    delivery
        .issue(&challenge, Request::Register, &email)
        .await
        .unwrap();
    let challenge_id = challenge_id(&runtime, &email).await;
    let attempt_id = attempt(&runtime, &email).await.unwrap();
    let row = sqlx::query("SELECT * FROM core_identity.email_delivery_outbox WHERE id=$1")
        .bind(&attempt_id)
        .fetch_one(&runtime)
        .await
        .unwrap();
    assert!(delivery.open_payload(&row).is_ok());
    assert!(
        delivery
            .keys
            .decrypt(
                &attempt_id,
                "00000000000000000000000000000000",
                "challenge",
                delivery.keys.active(),
                &row.try_get::<Vec<u8>, _>("payload").unwrap()
            )
            .is_err()
    );
    sqlx::query("UPDATE core_identity.email_challenges SET state='locked',attempts=5 WHERE id=$1")
        .bind(challenge_id)
        .execute(&owner)
        .await
        .unwrap();
    let state: (String, Option<Vec<u8>>) = sqlx::query_as(
        "SELECT status,payload FROM core_identity.email_delivery_outbox WHERE id=$1",
    )
    .bind(attempt_id)
    .fetch_one(&runtime)
    .await
    .unwrap();
    assert_eq!(state.0, "canceled");
    assert_eq!(state.1, None);
}

#[test]
fn payload_validation_and_key_separation_reject_ambiguous_or_wrong_context() {
    let keys = MailKeys::new(
        "next",
        vec![
            ("next".into(), vec![71; 32]),
            ("previous".into(), vec![72; 32]),
        ],
    )
    .unwrap();
    let ciphertext = keys
        .encrypt(
            "0123456789abcdef0123456789abcdef",
            "fedcba9876543210fedcba9876543210",
            "challenge",
            b"synthetic payload",
        )
        .unwrap();
    assert!(
        keys.decrypt(
            "0123456789abcdef0123456789abcdef",
            "fedcba9876543210fedcba9876543210",
            "challenge",
            "next",
            &ciphertext
        )
        .is_ok()
    );
    for (challenge, kind, key_id) in [
        ("00000000000000000000000000000000", "challenge", "next"),
        (
            "fedcba9876543210fedcba9876543210",
            "security_notice",
            "next",
        ),
        ("fedcba9876543210fedcba9876543210", "challenge", "previous"),
        ("fedcba9876543210fedcba9876543210", "challenge", "missing"),
    ] {
        assert!(
            keys.decrypt(
                "0123456789abcdef0123456789abcdef",
                challenge,
                kind,
                key_id,
                &ciphertext
            )
            .is_err()
        );
    }
    assert!(MailKeys::new("zero", vec![("zero".into(), vec![0; 32])]).is_err());
}

#[path = "reset_smtp_cases.rs"]
mod reset_smtp;

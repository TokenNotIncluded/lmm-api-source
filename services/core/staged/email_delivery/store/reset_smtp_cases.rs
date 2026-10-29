//! Actual encrypted reset outbox -> lettre -> local SMTP DATA -> proof verifier.
use super::*;
use std::{
    io::Write as _,
    process::{Command, Stdio},
};

#[tokio::test]
#[ignore = "requires the isolated leaf mail fixture"]
async fn real_reset_mail_uses_configured_origin_and_existing_single_use_proof() {
    let (owner, runtime, challenges, delivery) = fresh().await;
    let email = address("smtp+reset@example.test");
    let account: i64 = sqlx::query_scalar(
        "INSERT INTO core_identity.accounts(kind) VALUES ('personal') RETURNING id",
    )
    .fetch_one(&owner)
    .await
    .unwrap();
    sqlx::query("INSERT INTO core_identity.users(id,personal_account_id) VALUES (1,$1)")
        .bind(account)
        .execute(&owner)
        .await
        .unwrap();
    sqlx::query("INSERT INTO core_identity.user_profiles(user_id,username,email,email_version) VALUES (1,'smtpreset',$1,1)")
        .bind(email.as_str()).execute(&owner).await.unwrap();
    sqlx::query("INSERT INTO core_identity.password_logins(login_name,user_id,password_hash) VALUES ('smtpreset',1,$1)")
        .bind("synthetic-verifier-placeholder-not-a-password".repeat(2)).execute(&owner).await.unwrap();
    sqlx::query("INSERT INTO core_identity.email_addresses(email_key,user_id,state,email_version,verified_at) VALUES ($1,1,'verified',1,clock_timestamp())")
        .bind(email.as_str()).execute(&owner).await.unwrap();
    delivery
        .issue(&challenges, Request::PasswordReset, &email)
        .await
        .unwrap();
    let id = challenge_id(&runtime, &email).await;
    let expires: i64 = sqlx::query_scalar("SELECT (extract(epoch FROM expires_at)*1000000)::bigint FROM core_identity.email_challenges WHERE id=$1")
        .bind(&id).fetch_one(&runtime).await.unwrap();
    let (port, capture) = smtp_sink(SinkReply::Accepted).await;
    let mailer = local_mailer(port);
    let lease = delivery.claim().await.unwrap().unwrap();
    let attempt_id = lease.attempt_id().to_owned();
    assert_eq!(
        delivery.dispatch(lease, &mailer).await.unwrap(),
        Dispatch::Finished(SmtpOutcome::Accepted)
    );
    let message = tokio::time::timeout(Duration::from_secs(3), capture)
        .await
        .unwrap()
        .unwrap();
    let mut decoder = Command::new("/usr/bin/python3")
        .args(["-I", "-c", "import email.policy,email.parser,sys; m=email.parser.BytesParser(policy=email.policy.default).parsebytes(sys.stdin.buffer.read()); sys.stdout.write(m.get_content())"])
        .env_clear().stdin(Stdio::piped()).stdout(Stdio::piped()).stderr(Stdio::piped()).spawn().unwrap();
    decoder
        .stdin
        .take()
        .unwrap()
        .write_all(message.as_bytes())
        .unwrap();
    let decoded = decoder.wait_with_output().unwrap();
    assert!(
        decoded.status.success(),
        "captured SMTP MIME message could not be decoded"
    );
    let body = String::from_utf8(decoded.stdout).unwrap();
    let link = body
        .lines()
        .find(|line| line.starts_with("https://mail-web.example.test/user/reset#"))
        .expect("SMTP body must carry the configured reset link");
    let url = reqwest::Url::parse(link).unwrap();
    assert!(url.query().is_none());
    let query = reqwest::Url::parse(&format!(
        "https://decode.invalid/?{}",
        url.fragment().unwrap()
    ))
    .unwrap();
    let fields: Vec<_> = query.query_pairs().into_owned().collect();
    assert_eq!(fields.len(), 2);
    assert_eq!(fields[0], ("email".to_owned(), email.as_str().to_owned()));
    assert_eq!(fields[1].0, "token");
    let proof = &fields[1].1;
    assert_eq!(proof.len(), 32);
    assert!(!body.contains("verification code"));
    let after: (i64, i64) = sqlx::query_as("SELECT count(*),min((extract(epoch FROM expires_at)*1000000)::bigint) FROM core_identity.email_challenges WHERE email_key=$1")
        .bind(email.as_str()).fetch_one(&runtime).await.unwrap();
    assert_eq!(
        after,
        (1, expires),
        "SMTP rendering must not replace or extend the proof"
    );
    let status: String =
        sqlx::query_scalar("SELECT status FROM core_identity.email_delivery_outbox WHERE id=$1")
            .bind(&attempt_id)
            .fetch_one(&runtime)
            .await
            .unwrap();
    assert_eq!(status, "accepted_by_smtp");
    challenges
        .verify_current(Request::PasswordReset, &email, proof)
        .await
        .unwrap()
        .finish(1)
        .await
        .unwrap();
    assert!(matches!(
        challenges
            .verify_current(Request::PasswordReset, &email, proof)
            .await,
        Err(crate::identity::email::EmailError::InvalidProof)
    ));
    assert!(delivery.claim().await.unwrap().is_none());
}

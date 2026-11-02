//! Synthetic DB and real WebAuthn signatures; never executed against live data.
use super::{tests::*, *};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use webauthn_rs::prelude::{Passkey, Uuid, WebauthnBuilder};

async fn setup(pool: &sqlx::PgPool) -> (Service, TestDevice, crate::identity::IssuedCredential) {
    crate::core_schema::init(pool).await.unwrap();
    install_policy_fixture(pool).await;
    let device = install_proof_fixture(pool).await;
    let root = crate::identity::IdentityStore::from_pool(pool.clone())
        .bootstrap_user(1, 6)
        .await
        .unwrap();
    (test_service(pool), device, root)
}
async fn grant(pool: &sqlx::PgPool, name: &str) {
    sqlx::raw_sql(&format!("GRANT EXECUTE ON FUNCTION core_trust_pricing.read_policy_edit_description(bigint), core_trust_pricing.read_policy_editors(bigint), core_trust_pricing.policy_editor_ready(bytea), core_trust_pricing.policy_edit_binding(text,jsonb), core_trust_pricing.authorized_policy_preview(bytea,bigint,bigint,bigint,jsonb,jsonb,jsonb,jsonb,bytea,bytea), core_trust_pricing.authorized_policy_resolve(bytea,bigint,bigint,bytea), core_trust_pricing.policy_editor_context(text,bigint), core_trust_pricing.authorized_policy_challenge(jsonb,bytea), core_trust_pricing.policy_challenge_context(bytea,bytea,bigint,bigint), core_trust_pricing.authorized_policy_confirm(jsonb,bytea) TO {name}"))
        .execute(pool).await.unwrap();
}
async fn snapshot(pool: &sqlx::PgPool) -> Value {
    sqlx::query_scalar("SELECT jsonb_build_object('policy',(SELECT to_jsonb(p) FROM core_trust_pricing.policies p),'editors',(SELECT jsonb_agg(to_jsonb(e) ORDER BY editor_id) FROM core_trust_pricing.policy_editors e),'audit',(SELECT count(*) FROM core_trust_pricing.policy_edit_audit),'confirmations',(SELECT count(*) FROM core_trust_pricing.policy_editor_confirmations),'consumed',(SELECT count(*) FROM core_trust_pricing.policy_editor_challenges WHERE consumed_at IS NOT NULL))")
        .fetch_one(pool).await.unwrap()
}
fn clone_confirmation(input: &Confirmation) -> Confirmation {
    Confirmation {
        token: input.token.clone(),
        digest: input.digest.clone(),
        challenge: input.challenge.clone(),
        assertion: serde_json::from_value(serde_json::to_value(&input.assertion).unwrap()).unwrap(),
    }
}
fn unregistered_device() -> (TestDevice, Passkey) {
    let w = WebauthnBuilder::new("localhost", &test_origin())
        .unwrap()
        .build()
        .unwrap();
    let (options, state) = w
        .start_passkey_registration(Uuid::from_bytes([9; 16]), "attacker", "attacker", None)
        .unwrap();
    let mut device = test_device();
    let response = device.do_registration(test_origin(), options).unwrap();
    (
        device,
        w.finish_passkey_registration(&response, &state).unwrap(),
    )
}

#[sqlx::test(migrations = false)]
async fn runtime_forges_root_session_but_cannot_confirm_without_owner_passkey(pool: sqlx::PgPool) {
    let (_service, _owner_device, _root) = setup(&pool).await;
    let runtime = runtime_fixture::RuntimeRole::create(&pool).await;
    grant(&pool, &runtime.name).await;
    let secret = "runtime_forged_root_session_0123456789";
    // Exercise the actual writable-identity attack, not just a denied SQL call.
    let id: i64=sqlx::query_scalar("INSERT INTO core_identity.credentials(digest,kind,user_id,user_version,owner_account_id,expires_at) SELECT $1,'session',id,auth_version,personal_account_id,clock_timestamp()+interval '1 hour' FROM core_identity.users WHERE id=1 RETURNING id")
        .bind(Sha256::digest(secret.as_bytes()).as_slice()).fetch_one(&runtime.pool).await.unwrap();
    sqlx::query("INSERT INTO core_identity.session_authentication(credential_id,user_id,user_version,factor_revision,factor_level,method,authenticated_at) SELECT $1,1,auth_version,0,1,'primary',clock_timestamp() FROM core_identity.users WHERE id=1")
        .bind(id).execute(&runtime.pool).await.unwrap();
    let service = test_service(&runtime.pool);
    let preview = service
        .preview(secret, &fixture().to_string())
        .await
        .expect("attack must reach the real signer as root");
    let challenge = service
        .challenge(secret, &preview.token, &preview.preview.digest, "owner")
        .await
        .unwrap();
    let (mut attacker, key) = unregistered_device();
    let mut options = challenge.options;
    options["publicKey"]["allowCredentials"][0]["id"] =
        serde_json::to_value(key.cred_id()).unwrap();
    let assertion = attacker
        .do_authentication(test_origin(), serde_json::from_value(options).unwrap())
        .unwrap();
    let input = Confirmation {
        token: preview.token,
        digest: preview.preview.digest,
        challenge: challenge.challenge,
        assertion,
    };
    let before = snapshot(&pool).await;
    assert!(matches!(
        service.confirm(secret, &input).await,
        Err(Error::Invalid)
    ));
    assert_eq!(snapshot(&pool).await, before);
    assert_eq!(
        service.resolve(secret, &input.token).await.unwrap().state,
        "pending"
    );
    let missing = serde_json::from_value::<Confirmation>(
        json!({"token":input.token,"digest":input.digest,"challenge":input.challenge}),
    );
    assert!(missing.is_err());
    let owner = editor::Owner::new(runtime.pool.clone(), test_config());
    assert!(matches!(
        owner.begin_registration("attacker", 1, 0).await,
        Err(Error::Auth(_))
    ));
    assert!(matches!(
        owner.revoke("owner", 1).await,
        Err(Error::Auth(_))
    ));
    for sql in [
        "SELECT secret FROM core_trust_pricing.policy_edit_signing_key",
        "UPDATE core_trust_pricing.policy_editors SET version=version+1",
        "UPDATE core_trust_pricing.policy_editors SET passkey='{}'",
        "UPDATE core_trust_pricing.policy_editor_challenges SET authentication='{}'",
        "INSERT INTO core_trust_pricing.policy_editor_enrollments DEFAULT VALUES",
    ] {
        let e = sqlx::query(sql).execute(&runtime.pool).await.unwrap_err();
        assert_eq!(
            e.as_database_error().and_then(|e| e.code()).as_deref(),
            Some("42501"),
            "{sql}"
        );
    }
    runtime.cleanup().await;
}

#[sqlx::test(migrations = false)]
async fn editor_revoke_and_rotation_fence_already_signed_challenges(pool: sqlx::PgPool) {
    let (service, mut old, root) = setup(&pool).await;
    let owner = editor::Owner::new(pool.clone(), test_config());
    let preview = service
        .preview(&root.secret, &fixture().to_string())
        .await
        .unwrap();
    let input = fixture_confirmation(
        &service,
        &mut old,
        &root.secret,
        &preview.token,
        &preview.preview.digest,
    )
    .await
    .unwrap();
    assert_eq!(owner.revoke("owner", 1).await.unwrap(), 2);
    let before = snapshot(&pool).await;
    assert!(matches!(
        service.confirm(&root.secret, &input).await,
        Err(Error::Invalid)
    ));
    assert_eq!(snapshot(&pool).await, before);
    let mut replacement = test_device();
    enroll_editor(&pool, &mut replacement, "owner", 2).await;
    let before = snapshot(&pool).await;
    assert!(matches!(
        service.confirm(&root.secret, &input).await,
        Err(Error::Invalid)
    ));
    assert_eq!(snapshot(&pool).await, before);
    let next = service
        .preview(&root.secret, &fixture().to_string())
        .await
        .unwrap();
    assert_eq!(
        fixture_confirm(
            &service,
            &mut replacement,
            &root.secret,
            &next.token,
            &next.preview.digest
        )
        .await
        .unwrap(),
        2
    );
    let revision: i64=sqlx::query_scalar("SELECT editor_version FROM core_trust_pricing.policy_editor_confirmations WHERE revision=2").fetch_one(&pool).await.unwrap();
    assert_eq!(revision, 3);
}

#[sqlx::test(migrations = false)]
async fn assertion_binding_rejects_substitution_and_has_atomic_counter_updates(pool: sqlx::PgPool) {
    let (service, mut device, root) = setup(&pool).await;
    let a = service
        .preview(&root.secret, &fixture().to_string())
        .await
        .unwrap();
    let b = service
        .preview(&root.secret, &fixture().to_string())
        .await
        .unwrap();
    let input = fixture_confirmation(
        &service,
        &mut device,
        &root.secret,
        &a.token,
        &a.preview.digest,
    )
    .await
    .unwrap();
    let before = snapshot(&pool).await;
    for mutation in 0..4 {
        let mut changed = clone_confirmation(&input);
        match mutation {
            0 => changed.token = b.token.clone(),
            1 => changed.digest = "00".repeat(32),
            2 => changed.challenge = "00".repeat(32),
            _ => {
                let mut assertion = serde_json::to_value(&changed.assertion).unwrap();
                assertion["response"]["signature"] = json!("AA");
                changed.assertion = serde_json::from_value(assertion).unwrap();
            }
        }
        assert!(matches!(
            service.confirm(&root.secret, &changed).await,
            Err(Error::Invalid)
        ));
        assert_eq!(snapshot(&pool).await, before);
    }
    let key_before: Value=sqlx::query_scalar("SELECT passkey#>'{cred,cred}' FROM core_trust_pricing.policy_editors WHERE editor_id='owner'").fetch_one(&pool).await.unwrap();
    assert_eq!(service.confirm(&root.secret, &input).await.unwrap(), 2);
    let key_after: Value=sqlx::query_scalar("SELECT passkey#>'{cred,cred}' FROM core_trust_pricing.policy_editors WHERE editor_id='owner'").fetch_one(&pool).await.unwrap();
    assert_eq!(key_before, key_after);
    let after = snapshot(&pool).await;
    assert!(matches!(
        service.confirm(&root.secret, &input).await,
        Err(Error::Invalid)
    ));
    assert_eq!(snapshot(&pool).await, after);
    assert_eq!(
        service
            .resolve(&root.secret, &input.token)
            .await
            .unwrap()
            .revision,
        Some(2)
    );
}

#[sqlx::test(migrations = false)]
async fn owner_approval_is_explicit_bound_and_one_time(pool: sqlx::PgPool) {
    let (service, mut device, root) = setup(&pool).await;
    let owner = editor::Owner::new(pool.clone(), test_config());
    let begin = owner.begin_registration("second", 1, 0).await.unwrap();
    let response = device
        .do_registration(
            test_origin(),
            serde_json::from_value(begin.options).unwrap(),
        )
        .unwrap();
    let review = owner
        .review_registration(&begin.enrollment, &response)
        .await
        .unwrap();
    assert!(matches!(
        owner
            .approve_registration(&begin.enrollment, &"00".repeat(32))
            .await,
        Err(Error::Invalid)
    ));
    match owner.registration_status(&begin.enrollment).await.unwrap() {
        editor::RegistrationStatus::Reviewed { review: recovered } => {
            assert_eq!(recovered.fingerprint, review.fingerprint)
        }
        _ => panic!("lost review response must be recoverable"),
    }
    let preview = service
        .preview(&root.secret, &fixture().to_string())
        .await
        .unwrap();
    assert!(matches!(
        service
            .challenge(
                &root.secret,
                &preview.token,
                &preview.preview.digest,
                "second"
            )
            .await,
        Err(Error::Invalid)
    ));
    assert_eq!(
        owner
            .approve_registration(&begin.enrollment, &review.fingerprint)
            .await
            .unwrap(),
        1
    );
    assert!(matches!(
        owner
            .approve_registration(&begin.enrollment, &review.fingerprint)
            .await,
        Err(Error::Invalid)
    ));
    assert!(matches!(
        owner.begin_registration("second", 1, 0).await,
        Err(Error::Conflict)
    ));
    assert!(matches!(
        owner.registration_status(&begin.enrollment).await.unwrap(),
        editor::RegistrationStatus::Approved { version: 1, .. }
    ));
    assert_eq!(
        owner
            .editor_status("second")
            .await
            .unwrap()
            .unwrap()
            .version,
        1
    );
}

#[sqlx::test(migrations = false)]
async fn expired_challenge_and_stale_policy_leave_counter_untouched(pool: sqlx::PgPool) {
    let (service, mut device, root) = setup(&pool).await;
    let a = service
        .preview(&root.secret, &fixture().to_string())
        .await
        .unwrap();
    let b = service
        .preview(&root.secret, &fixture().to_string())
        .await
        .unwrap();
    let expired = fixture_confirmation(
        &service,
        &mut device,
        &root.secret,
        &a.token,
        &a.preview.digest,
    )
    .await
    .unwrap();
    sqlx::query("UPDATE core_trust_pricing.policy_editor_challenges SET expires_at=clock_timestamp()-interval '1 second'").execute(&pool).await.unwrap();
    let before = snapshot(&pool).await;
    assert!(matches!(
        service.confirm(&root.secret, &expired).await,
        Err(Error::Invalid)
    ));
    assert_eq!(snapshot(&pool).await, before);
    let stale = fixture_confirmation(
        &service,
        &mut device,
        &root.secret,
        &b.token,
        &b.preview.digest,
    )
    .await
    .unwrap();
    let c = service
        .preview(&root.secret, &fixture().to_string())
        .await
        .unwrap();
    assert_eq!(
        fixture_confirm(
            &service,
            &mut device,
            &root.secret,
            &c.token,
            &c.preview.digest
        )
        .await
        .unwrap(),
        2
    );
    let before = snapshot(&pool).await;
    // Depending on the authenticator counter, either replay-counter or CAS fails.
    assert!(matches!(
        service.confirm(&root.secret, &stale).await,
        Err(Error::Invalid | Error::Conflict)
    ));
    assert_eq!(snapshot(&pool).await, before);
}

#[sqlx::test(migrations = false)]
async fn wrong_injected_signing_key_cannot_create_preview(pool: sqlx::PgPool) {
    let (_, _, root) = setup(&pool).await;
    let service = Service::new(pool.clone(), test_config(), SigningKey::new([0x99; 32]));
    assert!(matches!(
        service.preview(&root.secret, &fixture().to_string()).await,
        Err(Error::Auth(_))
    ));
    let count: i64 =
        sqlx::query_scalar("SELECT count(*) FROM core_trust_pricing.policy_edit_previews")
            .fetch_one(&pool)
            .await
            .unwrap();
    assert_eq!(count, 0);
}

#[sqlx::test(migrations = false)]
async fn valid_device_signature_without_uv_or_for_other_origin_is_rejected(pool: sqlx::PgPool) {
    let (service, mut device, root) = setup(&pool).await;
    for mutation in 0..3 {
        let preview = service
            .preview(&root.secret, &fixture().to_string())
            .await
            .unwrap();
        let challenge = service
            .challenge(
                &root.secret,
                &preview.token,
                &preview.preview.digest,
                "owner",
            )
            .await
            .unwrap();
        let mut options = challenge.options;
        let origin = match mutation {
            0 => {
                options["publicKey"]["userVerification"] = json!("discouraged");
                test_origin()
            }
            1 => webauthn_rs::prelude::Url::parse("https://localhost:444").unwrap(),
            _ => {
                options["publicKey"]["rpId"] = json!("attacker.invalid");
                webauthn_rs::prelude::Url::parse("https://attacker.invalid").unwrap()
            }
        };
        // Sign with the actual registered private key, not a corrupted signature.
        let assertion = device
            .do_authentication(origin, serde_json::from_value(options).unwrap())
            .unwrap();
        let input = Confirmation {
            token: preview.token,
            digest: preview.preview.digest,
            challenge: challenge.challenge,
            assertion,
        };
        let before = snapshot(&pool).await;
        assert!(matches!(
            service.confirm(&root.secret, &input).await,
            Err(Error::Invalid)
        ));
        assert_eq!(snapshot(&pool).await, before);
    }
}

#[sqlx::test(migrations = false)]
async fn registration_without_uv_never_becomes_an_approvable_candidate(pool: sqlx::PgPool) {
    let (_, mut device, _) = setup(&pool).await;
    let owner = editor::Owner::new(pool.clone(), test_config());
    let begin = owner.begin_registration("no-uv", 1, 0).await.unwrap();
    let mut options = begin.options;
    options["publicKey"]["authenticatorSelection"]["userVerification"] = json!("discouraged");
    let response = device
        .do_registration(test_origin(), serde_json::from_value(options).unwrap())
        .unwrap();
    assert!(matches!(
        owner
            .review_registration(&begin.enrollment, &response)
            .await,
        Err(Error::Invalid)
    ));
    let count:i64=sqlx::query_scalar("SELECT count(*) FROM core_trust_pricing.policy_editor_enrollments WHERE candidate IS NOT NULL AND editor_id='no-uv'")
        .fetch_one(&pool).await.unwrap();
    assert_eq!(count, 0);
}

#[sqlx::test(migrations = false)]
async fn late_audit_failure_rolls_back_policy_counter_and_consumption(pool: sqlx::PgPool) {
    let (service, mut device, root) = setup(&pool).await;
    let preview = service
        .preview(&root.secret, &fixture().to_string())
        .await
        .unwrap();
    let input = fixture_confirmation(
        &service,
        &mut device,
        &root.secret,
        &preview.token,
        &preview.preview.digest,
    )
    .await
    .unwrap();
    sqlx::raw_sql("CREATE FUNCTION public.fail_editor_audit() RETURNS trigger LANGUAGE plpgsql AS $$ BEGIN RAISE EXCEPTION 'synthetic late audit failure'; END $$; CREATE TRIGGER fail_editor_audit BEFORE INSERT ON core_trust_pricing.policy_editor_confirmations FOR EACH ROW EXECUTE FUNCTION public.fail_editor_audit();")
        .execute(&pool).await.unwrap();
    let before = snapshot(&pool).await;
    assert!(matches!(
        service.confirm(&root.secret, &input).await,
        Err(Error::Storage)
    ));
    assert_eq!(snapshot(&pool).await, before);
    assert_eq!(
        service
            .resolve(&root.secret, &input.token)
            .await
            .unwrap()
            .state,
        "pending"
    );
    sqlx::query("DROP TRIGGER fail_editor_audit ON core_trust_pricing.policy_editor_confirmations")
        .execute(&pool)
        .await
        .unwrap();
    assert_eq!(service.confirm(&root.secret, &input).await.unwrap(), 2);
}

#[sqlx::test(migrations = false)]
async fn formal_read_preserves_authority_and_scopes_registered_editors(pool: sqlx::PgPool) {
    use axum::{
        body::{Body, to_bytes},
        http::{Request, StatusCode, header},
    };
    use tower::ServiceExt;
    let (_, mut device, root) = setup(&pool).await;
    let identity = crate::identity::IdentityStore::from_pool(pool.clone());
    let other = identity.bootstrap_user(2, 6).await.unwrap();
    let admin = identity.bootstrap_user(3, 5).await.unwrap();
    sqlx::query("UPDATE core_trust_pricing.policies SET spec=jsonb_set(spec,'{admin_discount}','{\"numerator\":94,\"denominator\":100}'::jsonb)").execute(&pool).await.unwrap();
    let raw: Value =
        sqlx::query_scalar("SELECT spec FROM core_trust_pricing.policies WHERE singleton")
            .fetch_one(&pool)
            .await
            .unwrap();
    let runtime = runtime_fixture::RuntimeRole::create(&pool).await;
    grant(&pool, &runtime.name).await;
    let service = test_service(&runtime.pool);
    let before = snapshot(&pool).await;
    let read = service.read(&root.secret).await.unwrap();
    assert_eq!(read.revision, 1);
    assert_eq!(read.policy, raw);
    assert_eq!(read.policy["admin_discount"]["denominator"], 100);
    assert!(read.console.is_none());
    assert_eq!(read.editors.len(), 1);
    assert_eq!(read.editors[0].editor_id, "owner");
    assert!(matches!(read.editors[0].status, EditorAvailability::Active));
    assert!(
        service
            .read(&other.secret)
            .await
            .unwrap()
            .editors
            .is_empty()
    );
    assert_eq!(snapshot(&pool).await, before);
    let app = http::router(service.clone());
    for (secret, status) in [
        (None, StatusCode::UNAUTHORIZED),
        (Some(&admin.secret), StatusCode::FORBIDDEN),
        (Some(&root.secret), StatusCode::OK),
    ] {
        let mut request = Request::get("/core/v1/admin/trust-policy");
        if let Some(secret) = secret {
            request = request.header(header::AUTHORIZATION, format!("Bearer {secret}"));
        }
        let response = app
            .clone()
            .oneshot(request.body(Body::empty()).unwrap())
            .await
            .unwrap();
        assert_eq!(response.status(), status);
        if status == StatusCode::OK {
            assert_eq!(
                response.headers().get(header::CACHE_CONTROL).unwrap(),
                "no-store"
            );
            let body: Value =
                serde_json::from_slice(&to_bytes(response.into_body(), 100_000).await.unwrap())
                    .unwrap();
            assert_eq!(body["data"]["policy"], raw);
            assert!(body["data"]["console"].is_null());
            assert_eq!(
                body["data"]["editors"][0],
                json!({"editor_id":"owner","version":1,"status":"active"})
            );
        }
    }
    let mismatch = Service::new(
        runtime.pool.clone(),
        EditorConfig::new("localhost", "https://localhost:444").unwrap(),
        SigningKey::new([0x0b; 32]),
    );
    assert!(matches!(
        mismatch.read(&root.secret).await.unwrap().editors[0].status,
        EditorAvailability::ConfigurationMismatch
    ));
    let preview = service
        .preview(&root.secret, &fixture().to_string())
        .await
        .unwrap();
    fixture_confirm(
        &service,
        &mut device,
        &root.secret,
        &preview.token,
        &preview.preview.digest,
    )
    .await
    .unwrap();
    let read = service.read(&root.secret).await.unwrap();
    assert_eq!(read.revision, 2);
    assert!(read.console.is_some());
    editor::Owner::new(pool.clone(), test_config())
        .revoke("owner", 1)
        .await
        .unwrap();
    let read = service.read(&root.secret).await.unwrap();
    assert!(matches!(
        read.editors[0].status,
        EditorAvailability::Revoked
    ));
    assert_eq!(read.editors[0].version, 2);
    runtime.cleanup().await;
}

#[sqlx::test(migrations = false)]
async fn readiness_is_read_only_session_free_and_rejects_wrong_or_missing_key(pool: sqlx::PgPool) {
    crate::core_schema::init(&pool).await.unwrap();
    install_policy_fixture(&pool).await;
    let _device = install_proof_fixture(&pool).await;
    // No identity bootstrap/session: key readiness must not require either.
    let users: i64 = sqlx::query_scalar("SELECT count(*) FROM core_identity.users")
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(users, 0);
    let runtime = runtime_fixture::RuntimeRole::create(&pool).await;
    grant(&pool, &runtime.name).await;
    let service = test_service(&runtime.pool);
    let before = snapshot(&pool).await;
    service.check_ready().await.unwrap();
    let wrong = Service::new(
        runtime.pool.clone(),
        test_config(),
        SigningKey::new([0x99; 32]),
    );
    assert!(matches!(wrong.check_ready().await, Err(Error::Storage)));
    assert_eq!(snapshot(&pool).await, before);
    let mut tx = runtime.pool.begin().await.unwrap();
    let wrong_domain = service.proof(&mut tx, "resolve", json!({})).await.unwrap();
    let wrong_payload = service
        .proof(&mut tx, "ready", json!({"other":true}))
        .await
        .unwrap();
    for proof in [wrong_domain, wrong_payload, vec![0; 31]] {
        let accepted: bool =
            sqlx::query_scalar("SELECT core_trust_pricing.policy_editor_ready($1)")
                .bind(proof)
                .fetch_one(&mut *tx)
                .await
                .unwrap();
        assert!(!accepted);
    }
    let proof = service.proof(&mut tx, "ready", json!({})).await.unwrap();
    let accepted: bool = sqlx::query_scalar("SELECT core_trust_pricing.policy_editor_ready($1)")
        .bind(&proof)
        .fetch_one(&mut *tx)
        .await
        .unwrap();
    assert!(accepted);
    tx.commit().await.unwrap();
    let replay: bool = sqlx::query_scalar("SELECT core_trust_pricing.policy_editor_ready($1)")
        .bind(proof)
        .fetch_one(&runtime.pool)
        .await
        .unwrap();
    assert!(!replay);
    let denied = sqlx::query(
        "SELECT core_trust_pricing.policy_edit_verify('ready','{}'::jsonb,NULL::bytea)",
    )
    .execute(&runtime.pool)
    .await
    .unwrap_err();
    assert_eq!(
        denied.as_database_error().and_then(|e| e.code()).as_deref(),
        Some("42501")
    );
    sqlx::query("DELETE FROM core_trust_pricing.policy_edit_signing_key")
        .execute(&pool)
        .await
        .unwrap();
    assert!(matches!(service.check_ready().await, Err(Error::Storage)));
    let absent: bool =
        sqlx::query_scalar("SELECT core_trust_pricing.policy_editor_ready(NULL::bytea)")
            .fetch_one(&runtime.pool)
            .await
            .unwrap();
    assert!(!absent);
    assert_eq!(snapshot(&pool).await, before);
    runtime.cleanup().await;
}

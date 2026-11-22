//! PostgreSQL regressions for the original Core Session authorization boundary.
//! All identities and money below are synthetic in SQLx-owned databases.
//! These reads neither authorize a skill mutation nor create migration approval.
use super::{Authorization, check_ready, read};
use crate::{
    accounts::{Account, AccountKind},
    assistant_tools_core::Error,
    billing::{
        Credits,
        ledger::{Action as LedgerAction, Ledger, Outcome, Request as LedgerRequest},
    },
    funding::FundingPolicy,
    identity::{
        BrowserAuth, BrowserRegistration, BrowserSessionMetadata, IdentityStore, permissions,
    },
    test_runtime::RuntimeRole,
};
use sha2::{Digest, Sha256};
use sqlx::PgPool;
use totp_rs::{Algorithm, Builder, Secret};

const PASSWORD: &str = "synthetic passphrase";

struct Fixture {
    identity: IdentityStore,
    runtime: RuntimeRole,
    admin: String,
    root: String,
    user: String,
}

async fn fixture(owner: &PgPool) -> Fixture {
    crate::core_schema::init(owner).await.unwrap();
    let identity = IdentityStore::from_pool(owner.clone())
        .with_registration_enabled(true)
        .with_mfa_keys("test".into(), vec![("test".into(), vec![41; 32])])
        .unwrap();
    // Existing offline test bootstrap creates actual persisted Session assurance.
    // Its compatibility role arguments are not a restored trust-level policy.
    let admin = identity.bootstrap_user(900, 5).await.unwrap().secret;
    let root = identity.bootstrap_user(901, 6).await.unwrap().secret;
    let user = identity.bootstrap_user(10, 0).await.unwrap().secret;
    let runtime = RuntimeRole::create(owner).await;
    let (session, current, superuser): (String, String, bool) = sqlx::query_as(
        "SELECT session_user::text,current_user::text,r.rolsuper
         FROM pg_roles r WHERE r.rolname=current_user",
    )
    .fetch_one(&runtime.pool)
    .await
    .unwrap();
    let url = reqwest::Url::parse(&runtime.url).unwrap();
    assert!(
        session == runtime.name
            && current == runtime.name
            && url.username() == runtime.name
            && !superuser,
        "the read path must use its real restricted runtime connection"
    );

    // Nonempty immutable journal and balance rows make the all-Core fingerprint
    // sensitive to an accidental financial write, not just empty-table equality.
    let account: i64 =
        sqlx::query_scalar("SELECT personal_account_id FROM core_identity.users WHERE id=10")
            .fetch_one(owner)
            .await
            .unwrap();
    let ledger = Ledger::from_pool(owner.clone());
    ledger.open_wallet(account).await.unwrap();
    let receipt = ledger
        .execute(&LedgerRequest {
            scope: "admin-skills-read-fixture".into(),
            operation_key: "synthetic-credit".into(),
            actor_user_id: Some(10),
            reason: "Synthetic read-only authorization regression".into(),
            action: LedgerAction::Credit {
                account_id: account,
                amount_units: Credits::try_from(1000).unwrap(),
                payment_reference: "synthetic-admin-skills".into(),
            },
        })
        .await
        .unwrap();
    assert!(matches!(receipt, Outcome::Posted { .. }));

    let before = fingerprint(owner).await;
    check_ready(&runtime.pool).await.unwrap();
    assert_eq!(before, fingerprint(owner).await);
    Fixture {
        identity,
        runtime,
        admin,
        root,
        user,
    }
}

/// Include every Core/public table and every sequence's full stored row
/// (last_value, log_cnt and is_called). Emit hashes only on assertion failure.
async fn fingerprint(owner: &PgPool) -> Vec<(String, String)> {
    let relations: Vec<(String, String, String)> = sqlx::query_as(
        "SELECT n.nspname,c.relname,c.relkind::text
         FROM pg_class c JOIN pg_namespace n ON n.oid=c.relnamespace
         WHERE c.relkind IN ('r','p','S')
           AND (left(n.nspname,5)='core_' OR n.nspname='public')
         ORDER BY n.nspname,c.relname,c.relkind",
    )
    .fetch_all(owner)
    .await
    .unwrap();
    assert!(!relations.is_empty());
    let mut result = Vec::with_capacity(relations.len());
    for (schema, relation, kind) in relations {
        assert!([&schema, &relation].into_iter().all(|name| {
            !name.is_empty()
                && name
                    .bytes()
                    .all(|byte| byte.is_ascii_alphanumeric() || byte == b'_')
        }));
        let hash: String = sqlx::query_scalar(sqlx::AssertSqlSafe(format!(
            "SELECT encode(sha256(convert_to(
                 COALESCE(jsonb_agg(to_jsonb(t) ORDER BY to_jsonb(t)::text),'[]'::jsonb)::text,
                 'UTF8')),'hex') FROM \"{schema}\".\"{relation}\" t"
        )))
        .fetch_one(owner)
        .await
        .unwrap();
        result.push((format!("{kind}:{schema}.{relation}"), hash));
    }
    result
}

async fn observe(
    owner: &PgPool,
    f: &Fixture,
    secret: &str,
    target: i64,
) -> Result<Authorization, Error> {
    let before = fingerprint(owner).await;
    let result = read(&f.runtime.pool, secret, target).await;
    assert_eq!(before, fingerprint(owner).await);
    result
}

fn sqlstate(error: &sqlx::Error, expected: &str) {
    assert_eq!(
        error
            .as_database_error()
            .and_then(|error| error.code())
            .as_deref(),
        Some(expected)
    );
}

#[sqlx::test(migrations = false)]
async fn admin_skill_reads_require_real_admin_admission_and_strict_target_roles(owner: PgPool) {
    let f = fixture(&owner).await;
    f.identity.bootstrap_user(902, 5).await.unwrap();
    f.identity.bootstrap_user(903, 6).await.unwrap();
    for (secret, actor, target, allowed) in [
        (f.admin.as_str(), 900, 900, true),
        (f.root.as_str(), 901, 901, true),
        (f.admin.as_str(), 900, 10, true),
        (f.root.as_str(), 901, 10, true),
        (f.root.as_str(), 901, 900, true),
        (f.admin.as_str(), 900, 902, false),
        (f.admin.as_str(), 900, 901, false),
        (f.root.as_str(), 901, 903, false),
        (f.user.as_str(), 10, 10, false),
        (f.user.as_str(), 10, 900, false),
    ] {
        let result = observe(&owner, &f, secret, target).await;
        if allowed {
            let authority = result.unwrap();
            assert_eq!(authority.actor_user_id, actor);
            assert_eq!(authority.target_user_id, target);
            assert_eq!(authority.version, 1);
            assert_eq!(authority.authorization_binding_sha256.len(), 64);
            assert!(authority.checked_at_unix > 0);
        } else {
            assert_eq!(result.unwrap_err(), Error::Forbidden);
        }
    }
    assert_eq!(
        observe(&owner, &f, &f.admin, 0).await.unwrap_err(),
        Error::Invalid
    );
    assert_eq!(
        observe(&owner, &f, &f.admin, 999_999).await.unwrap_err(),
        Error::Forbidden
    );
    f.runtime.cleanup().await;
}

#[sqlx::test(migrations = false)]
async fn admin_skill_reads_reject_keys_unproved_and_stale_credentials_without_writes(
    owner: PgPool,
) {
    let f = fixture(&owner).await;
    let key = f
        .identity
        .issue_key(
            &f.admin,
            Account {
                kind: AccountKind::Personal,
                id: 900,
            },
            FundingPolicy::new_personal_key(),
            3600,
        )
        .await
        .unwrap();
    let team = f.identity.create_team(&f.admin).await.unwrap();
    let team_account = Account {
        kind: AccountKind::Team,
        id: team,
    };
    let team_key = f
        .identity
        .issue_key(
            &f.admin,
            team_account,
            FundingPolicy {
                version: 1,
                account_order: Some(vec![team_account]),
                personal_billing_preference: None,
            },
            3600,
        )
        .await
        .unwrap();
    for secret in [&key.secret, &team_key.secret] {
        assert_eq!(
            observe(&owner, &f, secret, 10).await.unwrap_err(),
            Error::Forbidden
        );
    }

    // A negative fixture only: a well-formed stored Session hash without the
    // persisted primary/MFA proof must never become administrator authority.
    let bare = format!(
        "lmms_{:032x}{:032x}",
        rand::random::<u128>(),
        rand::random::<u128>()
    );
    sqlx::query(
        "INSERT INTO core_identity.credentials
         (digest,kind,user_id,user_version,owner_account_id,expires_at)
         SELECT $1,'session',id,auth_version,personal_account_id,
                clock_timestamp()+interval '1 hour'
         FROM core_identity.users WHERE id=900",
    )
    .bind(Sha256::digest(bare.as_bytes()).to_vec())
    .execute(&owner)
    .await
    .unwrap();
    assert_eq!(
        observe(&owner, &f, &bare, 10).await.unwrap_err(),
        Error::Unauthorized
    );
    for foreign in [
        format!("lmm_at_{}", "a".repeat(43)),
        format!("human_grant_{}", "b".repeat(64)),
    ] {
        assert_eq!(
            observe(&owner, &f, &foreign, 10).await.unwrap_err(),
            Error::Unauthorized
        );
    }

    for (id, reason) in [
        (910, "revoked"),
        (911, "expired"),
        (912, "inactive_account"),
        (913, "stale_generation"),
        (914, "inactive_actor"),
    ] {
        let credential = f.identity.bootstrap_user(id, 5).await.unwrap();
        observe(&owner, &f, &credential.secret, 10).await.unwrap();
        match reason {
            "revoked" => {
                sqlx::query(
                    "UPDATE core_identity.credentials SET revoked_at=clock_timestamp() WHERE id=$1",
                )
                .bind(credential.id)
                .execute(&owner)
                .await
                .unwrap();
            }
            "expired" => {
                sqlx::query(
                    "UPDATE core_identity.credentials
                     SET expires_at=clock_timestamp()-interval '1 second' WHERE id=$1",
                )
                .bind(credential.id)
                .execute(&owner)
                .await
                .unwrap();
            }
            "inactive_account" => {
                sqlx::query(
                    "UPDATE core_identity.accounts SET active=FALSE
                     WHERE id=(SELECT personal_account_id FROM core_identity.users WHERE id=$1)",
                )
                .bind(id)
                .execute(&owner)
                .await
                .unwrap();
            }
            "stale_generation" => {
                sqlx::query(
                    "UPDATE core_identity.users SET auth_version=auth_version+1 WHERE id=$1",
                )
                .bind(id)
                .execute(&owner)
                .await
                .unwrap();
            }
            "inactive_actor" => {
                sqlx::query("UPDATE core_identity.users SET active=FALSE WHERE id=$1")
                    .bind(id)
                    .execute(&owner)
                    .await
                    .unwrap();
            }
            _ => unreachable!(),
        }
        assert_eq!(
            observe(&owner, &f, &credential.secret, 10)
                .await
                .unwrap_err(),
            Error::Unauthorized,
            "a previously valid Session must fail its current authority check: {reason}"
        );
    }
    f.runtime.cleanup().await;
}

#[sqlx::test(migrations = false)]
async fn admin_skill_binding_tracks_target_state_and_preserves_disabled_target_reads(
    owner: PgPool,
) {
    let f = fixture(&owner).await;
    let initial = observe(&owner, &f, &f.root, 10).await.unwrap();
    sqlx::query(
        "UPDATE core_identity.accounts SET active=FALSE
         WHERE id=(SELECT personal_account_id FROM core_identity.users WHERE id=10)",
    )
    .execute(&owner)
    .await
    .unwrap();
    let account_disabled = observe(&owner, &f, &f.root, 10).await.unwrap();
    assert_eq!(
        initial.target_auth_version,
        account_disabled.target_auth_version
    );
    assert_ne!(
        initial.authorization_binding_sha256,
        account_disabled.authorization_binding_sha256
    );

    sqlx::query("UPDATE core_identity.users SET active=FALSE WHERE id=10")
        .execute(&owner)
        .await
        .unwrap();
    let user_disabled = observe(&owner, &f, &f.root, 10).await.unwrap();
    assert!(user_disabled.target_auth_version > account_disabled.target_auth_version);
    assert_ne!(
        account_disabled.authorization_binding_sha256,
        user_disabled.authorization_binding_sha256
    );
    // Disabled targets remain administratively readable, including while both
    // gates are off. They acquire no credential or other capability from a read.
    observe(&owner, &f, &f.admin, 10).await.unwrap();
    sqlx::query(
        "UPDATE core_identity.accounts SET active=TRUE
         WHERE id=(SELECT personal_account_id FROM core_identity.users WHERE id=10)",
    )
    .execute(&owner)
    .await
    .unwrap();
    sqlx::query("UPDATE core_identity.users SET active=TRUE WHERE id=10")
        .execute(&owner)
        .await
        .unwrap();
    let restored = observe(&owner, &f, &f.root, 10).await.unwrap();
    sqlx::query("UPDATE core_identity.users SET auth_version=auth_version+1 WHERE id=10")
        .execute(&owner)
        .await
        .unwrap();
    let advanced = observe(&owner, &f, &f.root, 10).await.unwrap();
    assert!(advanced.target_auth_version > restored.target_auth_version);
    assert_ne!(
        restored.authorization_binding_sha256,
        advanced.authorization_binding_sha256
    );

    sqlx::query("UPDATE core_identity.users SET platform_role='admin' WHERE id=10")
        .execute(&owner)
        .await
        .unwrap();
    let promoted = observe(&owner, &f, &f.root, 10).await.unwrap();
    assert_eq!(promoted.target_role, "admin");
    assert_ne!(
        advanced.authorization_binding_sha256,
        promoted.authorization_binding_sha256
    );
    assert_eq!(
        observe(&owner, &f, &f.admin, 10).await.unwrap_err(),
        Error::Forbidden
    );

    // The actual schema makes the user/account mapping immutable even to its
    // owner. Test that contract instead of disabling a trigger to invent a
    // supported mapping change. Creating the spare account is outside the read
    // and rejected-write fingerprints.
    let spare_account: i64 = sqlx::query_scalar(
        "INSERT INTO core_identity.accounts(kind) VALUES('personal') RETURNING id",
    )
    .fetch_one(&owner)
    .await
    .unwrap();
    let before = fingerprint(&owner).await;
    let error = sqlx::query("UPDATE core_identity.users SET personal_account_id=$1 WHERE id=10")
        .bind(spare_account)
        .execute(&owner)
        .await
        .unwrap_err();
    sqlstate(&error, "23514");
    assert_eq!(before, fingerprint(&owner).await);
    assert_eq!(
        promoted.authorization_binding_sha256,
        observe(&owner, &f, &f.root, 10)
            .await
            .unwrap()
            .authorization_binding_sha256
    );

    sqlx::query("UPDATE core_identity.users SET platform_role='superadmin' WHERE id=10")
        .execute(&owner)
        .await
        .unwrap();
    assert_eq!(
        observe(&owner, &f, &f.root, 10).await.unwrap_err(),
        Error::Forbidden
    );
    observe(&owner, &f, &f.root, 901).await.unwrap();
    f.runtime.cleanup().await;
}

#[derive(Clone, Copy)]
enum PolicyFixture {
    Complete,
    HeaderOnly,
    MissingRow,
    WrongHash,
    WrongPayload,
}

async fn install_policy(owner: &PgPool, user: i64, revision: i64, shape: PolicyFixture) {
    let entries: Vec<(String, String)> = permissions::Action::CATALOG
        .into_iter()
        .map(|action| (action.catalog_name().unwrap().to_owned(), "deny".to_owned()))
        .collect();
    let canonical = permissions::complete_policy_snapshot(user, revision, &entries).unwrap();
    let payload = if matches!(shape, PolicyFixture::WrongPayload) {
        b"{}".to_vec()
    } else {
        permissions::canonical_policy_payload(&canonical).unwrap()
    };
    let digest = if matches!(shape, PolicyFixture::WrongHash) {
        vec![7; 32]
    } else {
        permissions::policy_sha256(&canonical).unwrap().to_vec()
    };
    let mut tx = owner.begin().await.unwrap();
    sqlx::query(
        "INSERT INTO core_identity.permission_policies
         (user_id,revision,contract,sha256,canonical_payload,verified_complete)
         VALUES($1,$2,'core.permissions.v1',$3,$4,TRUE)",
    )
    .bind(user)
    .bind(revision)
    .bind(digest)
    .bind(payload)
    .execute(&mut *tx)
    .await
    .unwrap();
    let limit = match shape {
        PolicyFixture::HeaderOnly => 0,
        PolicyFixture::MissingRow => 8,
        _ => entries.len(),
    };
    for (action, effect) in entries.into_iter().take(limit) {
        sqlx::query(
            "INSERT INTO core_identity.permission_policy_rows(user_id,action,effect)
             VALUES($1,$2,$3)",
        )
        .bind(user)
        .bind(action)
        .bind(effect)
        .execute(&mut *tx)
        .await
        .unwrap();
    }
    tx.commit().await.unwrap();
}

#[sqlx::test(migrations = false)]
async fn admin_skill_binding_changes_on_first_complete_policy_and_rejects_policy_rewrites(
    owner: PgPool,
) {
    let f = fixture(&owner).await;
    let native = observe(&owner, &f, &f.admin, 10).await.unwrap();
    // Legal first insertion, not a mutation of an immutable existing policy.
    install_policy(&owner, 900, 2, PolicyFixture::Complete).await;
    let bound = observe(&owner, &f, &f.admin, 10).await.unwrap();
    assert_eq!(native.actor_auth_version, bound.actor_auth_version);
    assert_ne!(
        native.authorization_binding_sha256,
        bound.authorization_binding_sha256
    );
    // All nine dynamic actions are deny. Static administrator UserRead and
    // admitted self-read still follow the current strict role contract.
    observe(&owner, &f, &f.admin, 900).await.unwrap();

    for statement in [
        "UPDATE core_identity.permission_policies SET revision=revision+1 WHERE user_id=900",
        "UPDATE core_identity.permission_policy_rows SET effect='allow' WHERE user_id=900 AND action='ChannelRead'",
        "DELETE FROM core_identity.permission_policy_rows WHERE user_id=900 AND action='ChannelRead'",
    ] {
        let before = fingerprint(&owner).await;
        let error = sqlx::query(statement).execute(&owner).await.unwrap_err();
        sqlstate(&error, "55000");
        assert_eq!(before, fingerprint(&owner).await);
        assert_eq!(
            bound.authorization_binding_sha256,
            observe(&owner, &f, &f.admin, 10)
                .await
                .unwrap()
                .authorization_binding_sha256
        );
    }
    let before = fingerprint(&owner).await;
    let denied = sqlx::query(
        "UPDATE core_identity.permission_policies SET revision=revision WHERE user_id=900",
    )
    .execute(&f.runtime.pool)
    .await
    .unwrap_err();
    sqlstate(&denied, "42501");
    assert_eq!(before, fingerprint(&owner).await);
    f.runtime.cleanup().await;
}

async fn mark_synthetic_imported_admin(owner: &PgPool, user: i64) {
    // Deliberate isolated authority fixture matching the existing independent
    // admin-catalog suite. It is not a c343 import receipt or production approval.
    sqlx::query(
        "INSERT INTO core_identity.privileged_import_facts(
           user_id,archive_sha256,a2_semantic_sha256,a2_file_sha256,a2_sql_sha256,
           migration_id,system_id,database_oid,instance_nonce,import_run,
           source_enabled,source_deleted,password_verifier_recognized,
           password_login_enabled,password_hash_sha256,login_name_sha256,
           source_role,source_second_factor_required,source_auth_version)
         VALUES(
           $1,decode(repeat('a',64),'hex'),decode(repeat('b',64),'hex'),
           decode(repeat('c',64),'hex'),decode(repeat('d',64),'hex'),
           'synthetic-admin-skills-policy',decode(repeat('e',64),'hex'),
           (SELECT oid FROM pg_database WHERE datname=current_database()),
           decode(repeat('f',64),'hex'),decode(repeat('1',64),'hex'),
           TRUE,FALSE,TRUE,TRUE,decode(repeat('2',64),'hex'),
           decode(repeat('3',64),'hex'),'admin',FALSE,1)",
    )
    .bind(user)
    .execute(owner)
    .await
    .unwrap();
}

#[sqlx::test(migrations = false)]
async fn admin_skill_reads_require_complete_imported_policy_and_reject_malformed_native_policy(
    owner: PgPool,
) {
    let f = fixture(&owner).await;
    for (id, imported, shape, allowed) in [
        (920, true, None, false),
        (921, true, Some(PolicyFixture::HeaderOnly), false),
        (922, true, Some(PolicyFixture::MissingRow), false),
        (923, true, Some(PolicyFixture::WrongHash), false),
        (924, true, Some(PolicyFixture::WrongPayload), false),
        (925, true, Some(PolicyFixture::Complete), true),
        (926, false, Some(PolicyFixture::HeaderOnly), false),
        (927, false, Some(PolicyFixture::MissingRow), false),
        (928, false, Some(PolicyFixture::WrongHash), false),
        (929, false, Some(PolicyFixture::WrongPayload), false),
    ] {
        let credential = f.identity.bootstrap_user(id, 5).await.unwrap();
        // Prove this is a valid admitted Session before introducing the exact
        // policy/fact condition whose fail-closed behavior is under test.
        observe(&owner, &f, &credential.secret, 10).await.unwrap();
        if imported {
            mark_synthetic_imported_admin(&owner, id).await;
        }
        if let Some(shape) = shape {
            install_policy(&owner, id, 7, shape).await;
        }
        let authority = observe(&owner, &f, &credential.secret, 10).await;
        if allowed {
            assert_eq!(authority.unwrap().actor_user_id, id);
        } else {
            assert_eq!(authority.unwrap_err(), Error::Forbidden);
        }
    }
    f.runtime.cleanup().await;
}

// Existing identity/MFA fixture algorithm, using real persisted enrollment.
fn code_at(secret: &str, epoch: u64) -> String {
    let secret = Secret::try_from_base32(secret).unwrap();
    Builder::new()
        .with_algorithm(Algorithm::SHA1)
        .with_digits(6)
        .with_step_duration(30)
        .with_skew(1)
        .with_secret(secret.as_bytes())
        .build()
        .unwrap()
        .generate(epoch)
        .to_string()
}

#[sqlx::test(migrations = false)]
async fn admin_skill_binding_uses_original_browser_credential_and_current_real_mfa_generation(
    owner: PgPool,
) {
    let f = fixture(&owner).await;
    let browser = BrowserAuth::new(f.identity.clone(), &[43; 32]).unwrap();
    let username = "skills-browser-admin";
    browser
        .register(&BrowserRegistration {
            username,
            password: PASSWORD,
            email: "",
        })
        .await
        .unwrap();
    let actor: i64 =
        sqlx::query_scalar("SELECT user_id FROM core_identity.user_profiles WHERE username=$1")
            .bind(username)
            .fetch_one(&owner)
            .await
            .unwrap();
    sqlx::query("UPDATE core_identity.users SET platform_role='admin' WHERE id=$1")
        .bind(actor)
        .execute(&owner)
        .await
        .unwrap();
    let first = browser
        .login(username, PASSWORD, &BrowserSessionMetadata::default())
        .await
        .unwrap();
    let second = browser
        .login(username, PASSWORD, &BrowserSessionMetadata::default())
        .await
        .unwrap();
    let first_secret = first.data["access_token"].as_str().unwrap();
    let second_secret = second.data["access_token"].as_str().unwrap();
    let first_principal = f.identity.authorize(first_secret).await.unwrap();
    let second_principal = f.identity.authorize(second_secret).await.unwrap();
    assert_ne!(
        first_principal.credential_id,
        second_principal.credential_id
    );
    assert_eq!(
        first_principal.credential_generation(),
        second_principal.credential_generation()
    );
    let first_bound = observe(&owner, &f, first_secret, 10).await.unwrap();
    let second_bound = observe(&owner, &f, second_secret, 10).await.unwrap();
    assert_eq!(first_bound.actor_user_id, actor);
    assert_eq!(
        first_bound.actor_auth_version,
        second_bound.actor_auth_version
    );
    assert_ne!(
        first_bound.authorization_binding_sha256,
        second_bound.authorization_binding_sha256
    );
    assert_eq!(
        first_bound.authorization_binding_sha256,
        observe(&owner, &f, first_secret, 10)
            .await
            .unwrap()
            .authorization_binding_sha256
    );

    let revoked = sqlx::query(
        "UPDATE core_identity.web_sessions SET revoked_at=clock_timestamp()
         WHERE current_credential_id=$1",
    )
    .bind(second_principal.credential_id)
    .execute(&owner)
    .await
    .unwrap();
    assert_eq!(revoked.rows_affected(), 1);
    assert_eq!(
        observe(&owner, &f, second_secret, 10).await.unwrap_err(),
        Error::Unauthorized
    );
    assert_eq!(
        first_bound.authorization_binding_sha256,
        observe(&owner, &f, first_secret, 10)
            .await
            .unwrap()
            .authorization_binding_sha256
    );

    // No fake web-session/proof rows or verified boolean: actual password login,
    // actual TOTP enrollment and actual generation-replacing MFA enablement.
    let setup = browser.mfa_setup(first_secret).await.unwrap();
    let now: i64 =
        sqlx::query_scalar("SELECT floor(extract(epoch FROM clock_timestamp()))::bigint")
            .fetch_one(&owner)
            .await
            .unwrap();
    let enabled = browser
        .mfa_enable(
            first_secret,
            &code_at(&setup.secret, u64::try_from(now).unwrap()),
        )
        .await
        .unwrap();
    let fresh_secret = enabled.data["access_token"].as_str().unwrap();
    assert_eq!(
        observe(&owner, &f, first_secret, 10).await.unwrap_err(),
        Error::Unauthorized
    );
    let fresh_bound = observe(&owner, &f, fresh_secret, 10).await.unwrap();
    assert_eq!(fresh_bound.actor_user_id, actor);
    assert!(fresh_bound.actor_auth_version > first_bound.actor_auth_version);
    assert_ne!(
        first_bound.authorization_binding_sha256,
        fresh_bound.authorization_binding_sha256
    );
    let principal = f.identity.authorize(fresh_secret).await.unwrap();
    let level: i16 = sqlx::query_scalar(
        "SELECT factor_level FROM core_identity.session_authentication WHERE credential_id=$1",
    )
    .bind(principal.credential_id)
    .fetch_one(&owner)
    .await
    .unwrap();
    assert_eq!(level, 2);
    f.runtime.cleanup().await;
}

//! Synthetic imported recovery verifiers use real MFA transactions and PostgreSQL.
use lmm_core::identity::{
    BrowserAuth, BrowserLoginOutcome, BrowserRegistration, BrowserSessionMetadata, IdentityStore,
    MfaCompletion, MfaError,
};
use sqlx::PgPool;
use totp_rs::{Algorithm, Builder, Secret};

const PASSWORD: &str = "migration password";
const BINDING: &str = "1111111111111111111111111111111111111111111111111111111111111111";

async fn challenge(browser: &BrowserAuth) -> String {
    match browser
        .login_with_mfa(
            "migration",
            PASSWORD,
            &BrowserSessionMetadata::default(),
            BINDING,
        )
        .await
        .unwrap()
    {
        BrowserLoginOutcome::Challenge(c) => c.flow_token,
        _ => panic!("imported MFA must reject password-only session creation"),
    }
}

#[sqlx::test(migrations = false)]
async fn imported_bcrypt_recovery_rejects_wrong_proofs_and_consumes_once(pool: PgPool) {
    let store = IdentityStore::from_pool(pool.clone())
        .with_registration_enabled(true)
        .with_mfa_keys("test".into(), vec![("test".into(), vec![47; 32])])
        .unwrap();
    store.init_database().await.unwrap();
    sqlx::raw_sql(sqlx::AssertSqlSafe(lmm_core::abuse::SCHEMA))
        .execute(&pool)
        .await
        .unwrap();
    let browser = BrowserAuth::new(store, &[43; 32]).unwrap();
    browser
        .register(&BrowserRegistration {
            username: "migration",
            password: PASSWORD,
            email: "",
        })
        .await
        .unwrap();
    let original = browser
        .login("migration", PASSWORD, &BrowserSessionMetadata::default())
        .await
        .unwrap();
    let access = original.data["access_token"].as_str().unwrap();
    let setup = browser.mfa_setup(access).await.unwrap();
    let now: i64 =
        sqlx::query_scalar("SELECT floor(extract(epoch FROM clock_timestamp()))::bigint")
            .fetch_one(&pool)
            .await
            .unwrap();
    let secret = Secret::try_from_base32(&setup.secret).unwrap();
    let totp = Builder::new()
        .with_algorithm(Algorithm::SHA1)
        .with_digits(6)
        .with_step_duration(30)
        .with_skew(1)
        .with_secret(secret.as_bytes())
        .build()
        .unwrap();
    browser
        .mfa_enable(access, &totp.generate(now as u64).to_string())
        .await
        .unwrap();
    let uid = original.data["user"]["id"].as_i64().unwrap();
    let hash = bcrypt::hash("ABCD-1234", 10).unwrap();
    sqlx::query("DELETE FROM core_identity.mfa_backup_codes WHERE user_id=$1")
        .bind(uid)
        .execute(&pool)
        .await
        .unwrap();
    sqlx::query("INSERT INTO core_identity.mfa_backup_codes(user_id,factor_revision,verifier) SELECT user_id,revision,$2 FROM core_identity.mfa_factors WHERE user_id=$1")
        .bind(uid).bind(format!("legacy-bcrypt:{hash}")).execute(&pool).await.unwrap();

    let first = challenge(&browser).await;
    assert!(matches!(
        browser
            .complete_mfa_login(
                &MfaCompletion {
                    flow_token: first.clone(),
                    code: "ABCD-1235".into(),
                },
                BINDING
            )
            .await,
        Err(MfaError::CodeInvalid)
    ));
    assert!(matches!(
        browser
            .complete_mfa_login(
                &MfaCompletion {
                    flow_token: first.clone(),
                    code: "ABCD-1234".into(),
                },
                "2222222222222222222222222222222222222222222222222222222222222222"
            )
            .await,
        Err(MfaError::FlowExpired)
    ));
    let second = challenge(&browser).await;
    let one = MfaCompletion {
        flow_token: first,
        code: "abcd1234".into(),
    };
    let two = MfaCompletion {
        flow_token: second,
        code: "ABCD-1234".into(),
    };
    let (a, b) = tokio::join!(
        browser.complete_mfa_login(&one, BINDING),
        browser.complete_mfa_login(&two, BINDING)
    );
    assert_eq!(
        usize::from(a.is_ok()) + usize::from(b.is_ok()),
        1,
        "one recovery code issues exactly one session"
    );
    let consumed: i64 = sqlx::query_scalar("SELECT count(*) FROM core_identity.mfa_backup_codes WHERE user_id=$1 AND consumed_at IS NOT NULL")
        .bind(uid).fetch_one(&pool).await.unwrap();
    assert_eq!(consumed, 1);
    let replay = MfaCompletion {
        flow_token: challenge(&browser).await,
        code: "ABCD-1234".into(),
    };
    assert!(matches!(
        browser.complete_mfa_login(&replay, BINDING).await,
        Err(MfaError::CodeInvalid)
    ));
}

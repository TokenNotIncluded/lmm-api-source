//! Disposable PostgreSQL and the restricted runtime role exercise the complete
//! authorization/ledger boundary. Siteverify is an explicit local HTTP fixture;
//! real signatures come from the upstream software WebAuthn authenticator.
use super::*;
use crate::{
    checkin_ledger::CheckinLedger,
    core_schema,
    identity::{
        BrowserAuth, BrowserBundle, BrowserRegistration, BrowserSessionMetadata, IdentityStore,
    },
    internal_rpc::pb::core_checkin_ledger_server::CoreCheckinLedger,
    test_runtime::RuntimeRole,
};
use axum::{Json as HttpJson, Router, extract::State, http::StatusCode, routing::post};
use serde_json::json;
use sqlx::PgPool;
use std::{
    collections::HashMap,
    sync::{
        Arc,
        atomic::{AtomicUsize, Ordering},
    },
};
use tonic::{Code, Request};
use webauthn_authenticator_rs::{WebauthnAuthenticator, softpasskey::SoftPasskey};
use webauthn_rs::prelude::Url;

const SERVICE: &str = "test-checkin-funding-private-service-token";
const ORIGIN: &str = "https://app.example.test";
const OPERATION: &str = "checkin-operation-00000001";
const OTHER_OPERATION: &str = "checkin-operation-00000002";

struct Fixture {
    role: RuntimeRole,
    browser: BrowserAuth,
    bundle: BrowserBundle,
    ledger: CheckinLedger,
    passkey: PasskeyConfig,
    calls: Arc<AtomicUsize>,
    provider: tokio::task::JoinHandle<()>,
}

async fn siteverify(
    State(calls): State<Arc<AtomicUsize>>,
    axum::Form(form): axum::Form<HashMap<String, String>>,
) -> (StatusCode, HttpJson<Value>) {
    assert_eq!(form["secret"], "test-private-secret");
    calls.fetch_add(1, Ordering::SeqCst);
    let (case, binding) = form["response"].split_once(':').unwrap();
    let mut body = json!({
        "success": case != "denied", "hostname":"app.example.test",
        "action":"daily_checkin", "cdata":binding,
    });
    match case {
        "wrong-host" => body["hostname"] = json!("other.example.test"),
        "wrong-action" => body["action"] = json!("login"),
        "wrong-binding" => body["cdata"] = json!("unrelated_challenge"),
        "missing-binding" => {
            body.as_object_mut().unwrap().remove("cdata");
        }
        "unavailable" => return (StatusCode::SERVICE_UNAVAILABLE, HttpJson(body)),
        _ => (),
    }
    (StatusCode::OK, HttpJson(body))
}

async fn fixture(owner: &PgPool, strict: bool) -> Fixture {
    core_schema::init(owner).await.unwrap();
    let role = RuntimeRole::create(owner).await;
    role.optional("web_identity").await;
    role.optional("core_checkin_challenges").await;
    let store = IdentityStore::from_pool(role.pool.clone())
        .with_registration_enabled(true)
        .with_mfa_keys("test".into(), vec![("test".into(), vec![47; 32])])
        .unwrap();
    let browser = BrowserAuth::new(store, &[43; 32]).unwrap();
    browser
        .register(&BrowserRegistration {
            username: "alice",
            password: "checkin fixture",
            email: "",
        })
        .await
        .unwrap();
    let bundle = browser
        .login(
            "alice",
            "checkin fixture",
            &BrowserSessionMetadata::default(),
        )
        .await
        .unwrap();
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let endpoint = format!("http://{}/siteverify", listener.local_addr().unwrap());
    let calls = Arc::new(AtomicUsize::new(0));
    let app = Router::new()
        .route("/siteverify", post(siteverify))
        .with_state(calls.clone());
    let provider = tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
    let passkey = PasskeyConfig::new("app.example.test", ORIGIN).unwrap();
    let config = ChallengeConfig::new(
        crate::config::turnstile_config::tests::verifier_at(endpoint),
        Some(passkey.clone()),
        strict,
    )
    .unwrap();
    let ledger = CheckinLedger::new(role.pool.clone(), SERVICE.as_bytes(), 100)
        .unwrap()
        .with_challenges(config);
    Fixture {
        role,
        browser,
        bundle,
        ledger,
        passkey,
        calls,
        provider,
    }
}

fn access(bundle: &BrowserBundle) -> &str {
    bundle.data["access_token"].as_str().unwrap()
}
fn request<T>(access: &str, body: T) -> Request<T> {
    let mut request = Request::new(body);
    request.metadata_mut().insert(
        "authorization",
        format!("Bearer {SERVICE}").parse().unwrap(),
    );
    request
        .metadata_mut()
        .insert("x-lmm-protocol", "1".parse().unwrap());
    request
        .metadata_mut()
        .insert("x-lmm-user-credential", access.parse().unwrap());
    request
}
fn authorization(
    challenge: &pb::CheckinChallenge,
    case: &str,
    assertion: String,
) -> pb::CheckinChallengeAuthorize {
    pb::CheckinChallengeAuthorize {
        challenge_id: challenge.challenge_id.clone(),
        operation_key: challenge.operation_key.clone(),
        turnstile_token: format!("{case}:{}", challenge.challenge_id),
        passkey_assertion_json: assertion,
    }
}
fn posting(
    context: &pb::CheckinLedgerContext,
    challenge: &pb::CheckinChallenge,
    amount: i64,
) -> pb::CheckinLedgerPost {
    pb::CheckinLedgerPost {
        day: context.day.clone(),
        amount_units: amount,
        trust_level: context.trust_level,
        operation_key: challenge.operation_key.clone(),
        authorization_id: challenge.challenge_id.clone(),
    }
}
fn status(error: Status, code: Code, message: &str) {
    assert_eq!(error.code(), code, "{error:?}");
    assert_eq!(error.message(), message);
}
impl Fixture {
    async fn challenge(&self, operation: &str) -> pb::CheckinChallenge {
        self.ledger
            .challenge(request(
                access(&self.bundle),
                pb::CheckinChallengeRequest {
                    operation_key: operation.into(),
                },
            ))
            .await
            .unwrap()
            .into_inner()
    }
    async fn context(&self) -> pb::CheckinLedgerContext {
        self.ledger
            .inspect(request(access(&self.bundle), pb::CheckinLedgerInspect {}))
            .await
            .unwrap()
            .into_inner()
    }
    async fn authorize(
        &self,
        challenge: &pb::CheckinChallenge,
        case: &str,
        assertion: String,
    ) -> Result<pb::CheckinAuthorization, Status> {
        self.ledger
            .authorize(request(
                access(&self.bundle),
                authorization(challenge, case, assertion),
            ))
            .await
            .map(|response| response.into_inner())
    }
    async fn cleanup(self) {
        self.provider.abort();
        self.role.cleanup().await;
    }
}

#[sqlx::test(migrations = false)]
async fn provider_binding_is_mandatory_and_success_posts_once(owner: PgPool) {
    let f = fixture(&owner, false).await;
    let context = f.context().await;
    let challenge = f.challenge(OPERATION).await;
    assert_eq!(challenge.challenge_id.len(), 64);
    assert_eq!(challenge.turnstile_cdata, challenge.challenge_id);
    assert_eq!(challenge.turnstile_action, "daily_checkin");
    assert!(!challenge.passkey_required);
    assert!(challenge.passkey_options_json.is_empty());
    assert!((119..=121).contains(&(challenge.expires_at - context.timestamp)));
    let input = posting(&context, &challenge, 31);
    status(
        f.ledger
            .post(request(access(&f.bundle), input.clone()))
            .await
            .unwrap_err(),
        Code::FailedPrecondition,
        "CHECKIN_VERIFICATION_REQUIRED",
    );
    for case in ["denied", "wrong-host", "wrong-action"] {
        status(
            f.authorize(&challenge, case, String::new())
                .await
                .unwrap_err(),
            Code::PermissionDenied,
            "CHECKIN_VERIFICATION_FAILED",
        );
    }
    status(
        f.authorize(&challenge, "ok", String::new())
            .await
            .unwrap_err(),
        Code::ResourceExhausted,
        "CHECKIN_RATE_LIMITED",
    );
    let challenge = f.challenge(OPERATION).await;
    assert_ne!(challenge.challenge_id, input.authorization_id);
    for case in ["wrong-binding", "unavailable"] {
        let expected = if case == "unavailable" {
            (Code::Unavailable, "CHECKIN_VERIFICATION_UNAVAILABLE")
        } else {
            (Code::PermissionDenied, "CHECKIN_VERIFICATION_FAILED")
        };
        status(
            f.authorize(&challenge, case, String::new())
                .await
                .unwrap_err(),
            expected.0,
            expected.1,
        );
    }
    let authorized = f.authorize(&challenge, "ok", String::new()).await.unwrap();
    assert_eq!(authorized.authorization_id, challenge.challenge_id);
    let calls = f.calls.load(Ordering::SeqCst);
    let recovered = f
        .authorize(&challenge, "unavailable", String::new())
        .await
        .unwrap();
    assert_eq!(recovered, authorized);
    assert_eq!(f.calls.load(Ordering::SeqCst), calls);
    let mut input = posting(&context, &challenge, 31);
    assert_eq!(context.trust_level, 0);
    // A pre-retirement intention can still be pending in the extension journal.
    input.trust_level = 4;
    let mut over_cap = input.clone();
    over_cap.amount_units = context.max_daily_units + 1;
    status(
        f.ledger
            .post(request(access(&f.bundle), over_cap))
            .await
            .unwrap_err(),
        Code::PermissionDenied,
        "funding authority denied",
    );
    // Fail after authorization consumption, at the real ledger call. The
    // transaction must retain neither a consumed proof nor a partial reward.
    sqlx::raw_sql(sqlx::AssertSqlSafe(format!(
        "REVOKE EXECUTE ON FUNCTION core_billing.post_ledger(jsonb) FROM {}",
        f.role.name,
    )))
    .execute(&owner)
    .await
    .unwrap();
    assert_eq!(
        f.ledger
            .post(request(access(&f.bundle), input.clone()))
            .await
            .unwrap_err()
            .code(),
        Code::Unavailable
    );
    let consumed: bool = sqlx::query_scalar(
        "SELECT consumed_at IS NOT NULL FROM core_checkin_challenges.challenges WHERE digest=$1",
    )
    .bind(challenge_digest(&challenge.challenge_id).unwrap())
    .fetch_one(&owner)
    .await
    .unwrap();
    assert!(!consumed);
    sqlx::raw_sql(sqlx::AssertSqlSafe(format!(
        "GRANT EXECUTE ON FUNCTION core_billing.post_ledger(jsonb) TO {}",
        f.role.name,
    )))
    .execute(&owner)
    .await
    .unwrap();
    let receipt = f
        .ledger
        .post(request(access(&f.bundle), input.clone()))
        .await
        .unwrap()
        .into_inner();
    assert!(!receipt.replayed);
    assert!(receipt.journal_id.is_some());
    assert!(receipt.timestamp > 0);
    let mut replay = input.clone();
    replay.authorization_id.clear();
    replay.operation_key.clear();
    replay.trust_level = 99;
    let recovered = f
        .ledger
        .post(request(access(&f.bundle), replay))
        .await
        .unwrap()
        .into_inner();
    assert!(recovered.replayed);
    assert_eq!(recovered.journal_id, receipt.journal_id);
    assert_eq!(recovered.timestamp, receipt.timestamp);
    let mut conflict = input;
    conflict.amount_units = 32;
    assert_eq!(
        f.ledger
            .post(request(access(&f.bundle), conflict))
            .await
            .unwrap_err()
            .code(),
        Code::AlreadyExists
    );
    let count: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM core_billing.ledger_operations WHERE scope='core-checkins'",
    )
    .fetch_one(&owner)
    .await
    .unwrap();
    assert_eq!(count, 1);
    let stored: i64 = sqlx::query_scalar("SELECT count(*) FROM core_checkin_challenges.receipts")
        .fetch_one(&owner)
        .await
        .unwrap();
    assert_eq!(stored, 1);
    let balance: i64 = sqlx::query_scalar("SELECT b.balance_units FROM core_billing.balance_state b JOIN core_billing.ledger_accounts a ON a.id=b.ledger_account_id WHERE a.owner_account_id=$1 AND a.bucket='wallet'")
        .bind(receipt.account_id).fetch_one(&owner).await.unwrap();
    assert_eq!(balance, 31);
    f.cleanup().await;
}

#[sqlx::test(migrations = false)]
async fn bindings_survive_access_rotation_but_not_other_sessions_and_zero_is_durable(
    owner: PgPool,
) {
    let mut f = fixture(&owner, false).await;
    let context = f.context().await;
    let challenge = f.challenge(OPERATION).await;
    // Model refresh holding the family lock before rotating the credential.
    // The check-in path must not wait while holding the reverse credential lock.
    let mut refresh_lock = owner.begin().await.unwrap();
    sqlx::query("SELECT sid FROM core_identity.web_sessions WHERE sid=$1 FOR UPDATE")
        .bind(f.bundle.data["session"]["sid"].as_str().unwrap())
        .execute(&mut *refresh_lock)
        .await
        .unwrap();
    let blocked = tokio::time::timeout(
        std::time::Duration::from_secs(1),
        f.ledger.challenge(request(
            access(&f.bundle),
            pb::CheckinChallengeRequest {
                operation_key: OTHER_OPERATION.into(),
            },
        )),
    )
    .await
    .expect("check-in must release credential authority without waiting for refresh");
    assert_eq!(blocked.unwrap_err().code(), Code::Unavailable);
    refresh_lock.rollback().await.unwrap();
    let other = f
        .browser
        .login(
            "alice",
            "checkin fixture",
            &BrowserSessionMetadata::default(),
        )
        .await
        .unwrap();
    status(
        f.ledger
            .authorize(request(
                access(&other),
                authorization(&challenge, "ok", String::new()),
            ))
            .await
            .unwrap_err(),
        Code::FailedPrecondition,
        "CHECKIN_VERIFICATION_REQUIRED",
    );
    let mut wrong_operation = authorization(&challenge, "ok", String::new());
    wrong_operation.operation_key = OTHER_OPERATION.into();
    status(
        f.ledger
            .authorize(request(access(&f.bundle), wrong_operation))
            .await
            .unwrap_err(),
        Code::FailedPrecondition,
        "CHECKIN_VERIFICATION_REQUIRED",
    );
    assert_eq!(f.calls.load(Ordering::SeqCst), 0);
    let original_sid = f.bundle.data["session"]["sid"].clone();
    f.bundle = f
        .browser
        .refresh(&f.bundle.refresh_token, None)
        .await
        .unwrap();
    assert_eq!(f.bundle.data["session"]["sid"], original_sid);
    f.authorize(&challenge, "ok", String::new()).await.unwrap();
    let input = posting(&context, &challenge, 0);
    let receipt = f
        .ledger
        .post(request(access(&f.bundle), input.clone()))
        .await
        .unwrap()
        .into_inner();
    assert!(!receipt.replayed);
    assert!(receipt.journal_id.is_none());
    assert!(receipt.timestamp > 0);
    let mut replay = input;
    replay.authorization_id.clear();
    let recovered = f
        .ledger
        .post(request(access(&other), replay))
        .await
        .unwrap()
        .into_inner();
    assert!(recovered.replayed);
    assert_eq!(recovered.timestamp, receipt.timestamp);
    let count: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM core_checkin_challenges.receipts WHERE amount_units=0",
    )
    .fetch_one(&owner)
    .await
    .unwrap();
    assert_eq!(count, 1);
    let ledger_count: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM core_billing.ledger_operations WHERE scope='core-checkins'",
    )
    .fetch_one(&owner)
    .await
    .unwrap();
    assert_eq!(ledger_count, 0);
    let before_password_change = f.challenge(OTHER_OPERATION).await;
    f.bundle = f
        .browser
        .change_password(
            access(&f.bundle),
            "checkin fixture",
            "checkin changed",
            None,
            None,
        )
        .await
        .unwrap();
    status(
        f.authorize(&before_password_change, "ok", String::new())
            .await
            .unwrap_err(),
        Code::FailedPrecondition,
        "CHECKIN_VERIFICATION_REQUIRED",
    );
    f.cleanup().await;
}

#[sqlx::test(migrations = false)]
async fn expiry_rate_limits_and_concurrent_posting_fail_closed(owner: PgPool) {
    let f = fixture(&owner, false).await;
    let context = f.context().await;
    let challenge = f.challenge(OPERATION).await;
    let no_config = CheckinLedger::new(f.role.pool.clone(), SERVICE.as_bytes(), 100).unwrap();
    status(
        no_config
            .post(request(
                access(&f.bundle),
                posting(&context, &challenge, 10),
            ))
            .await
            .unwrap_err(),
        Code::FailedPrecondition,
        "CHECKIN_VERIFICATION_REQUIRED",
    );
    let expired_id = "ab".repeat(32);
    sqlx::query("INSERT INTO core_checkin_challenges.challenges(digest,user_id,account_id,sid,user_version,operation_key,day,passkey_required,created_at,expires_at) SELECT $1,user_id,account_id,sid,user_version,operation_key,day,false,clock_timestamp()-interval '121 seconds',clock_timestamp()-interval '1 second' FROM core_checkin_challenges.challenges WHERE digest=$2")
        .bind(challenge_digest(&expired_id).unwrap()).bind(challenge_digest(&challenge.challenge_id).unwrap())
        .execute(&owner).await.unwrap();
    let mut expired = challenge.clone();
    expired.challenge_id = expired_id;
    status(
        f.authorize(&expired, "ok", String::new())
            .await
            .unwrap_err(),
        Code::FailedPrecondition,
        "CHECKIN_VERIFICATION_REQUIRED",
    );
    status(
        f.ledger
            .post(request(access(&f.bundle), posting(&context, &expired, 10)))
            .await
            .unwrap_err(),
        Code::FailedPrecondition,
        "CHECKIN_VERIFICATION_REQUIRED",
    );
    for _ in 0..4 {
        f.challenge(OPERATION).await;
    }
    status(
        f.ledger
            .challenge(request(
                access(&f.bundle),
                pb::CheckinChallengeRequest {
                    operation_key: OPERATION.into(),
                },
            ))
            .await
            .unwrap_err(),
        Code::ResourceExhausted,
        "CHECKIN_RATE_LIMITED",
    );
    f.authorize(&challenge, "ok", String::new()).await.unwrap();
    let input = posting(&context, &challenge, 10);
    let mut tasks = Vec::new();
    for _ in 0..12 {
        let ledger = f.ledger.clone();
        let input = input.clone();
        let access = access(&f.bundle).to_owned();
        tasks.push(tokio::spawn(async move {
            ledger.post(request(&access, input)).await
        }));
    }
    let mut initial = 0;
    for task in tasks {
        match task.await.unwrap() {
            Ok(response) => initial += usize::from(!response.get_ref().replayed),
            // SERIALIZABLE competitors retry the original intention; they never
            // receive a second journal or consume another challenge.
            Err(error) => assert_eq!(error.code(), Code::Unavailable, "{error:?}"),
        }
    }
    assert_eq!(initial, 1);
    let receipt = f
        .ledger
        .post(request(access(&f.bundle), input))
        .await
        .unwrap()
        .into_inner();
    assert!(receipt.replayed);
    let (receipts, consumed): (i64, i64) = sqlx::query_as("SELECT (SELECT count(*) FROM core_checkin_challenges.receipts),(SELECT count(*) FROM core_checkin_challenges.challenges WHERE consumed_at IS NOT NULL)")
        .fetch_one(&owner).await.unwrap();
    assert_eq!((receipts, consumed), (1, 1));
    f.cleanup().await;
}

#[sqlx::test(migrations = false)]
async fn strict_mode_requires_fresh_signed_passkey_user_verification(owner: PgPool) {
    let mut f = fixture(&owner, true).await;
    status(
        f.ledger
            .challenge(request(
                access(&f.bundle),
                pb::CheckinChallengeRequest {
                    operation_key: OPERATION.into(),
                },
            ))
            .await
            .unwrap_err(),
        Code::FailedPrecondition,
        "CHECKIN_PASSKEY_REQUIRED",
    );
    let mut device = WebauthnAuthenticator::new(SoftPasskey::new(true));
    let binding = "11".repeat(32);
    let registration = f
        .browser
        .begin_passkey_registration(&f.passkey, access(&f.bundle), &binding)
        .await
        .unwrap();
    let attestation = device
        .do_registration(
            Url::parse(ORIGIN).unwrap(),
            serde_json::from_value(registration.options).unwrap(),
        )
        .unwrap();
    f.bundle = f
        .browser
        .finish_passkey_registration(
            &f.passkey,
            access(&f.bundle),
            &binding,
            &registration.flow_token,
            serde_json::to_value(attestation).unwrap(),
            "Synthetic checkin authenticator",
        )
        .await
        .unwrap();
    let challenge = f.challenge(OPERATION).await;
    assert!(challenge.passkey_required);
    let options: Value = serde_json::from_str(&challenge.passkey_options_json).unwrap();
    assert_eq!(options["publicKey"]["userVerification"], "required");
    status(
        f.authorize(&challenge, "ok", String::new())
            .await
            .unwrap_err(),
        Code::PermissionDenied,
        "CHECKIN_VERIFICATION_FAILED",
    );
    // A caller can change its browser options. Sign the ORIGINAL challenge
    // with a valid key but without UV; the server's required policy must win.
    let mut no_uv_options = options.clone();
    no_uv_options["publicKey"]["userVerification"] = json!("discouraged");
    let no_uv = device
        .do_authentication(
            Url::parse(ORIGIN).unwrap(),
            serde_json::from_value(no_uv_options).unwrap(),
        )
        .unwrap();
    let signed_data: &[u8] = no_uv.response.authenticator_data.as_ref();
    assert_eq!(signed_data[32] & 0x04, 0);
    status(
        f.authorize(&challenge, "ok", serde_json::to_string(&no_uv).unwrap())
            .await
            .unwrap_err(),
        Code::PermissionDenied,
        "CHECKIN_VERIFICATION_FAILED",
    );
    let assertion = device
        .do_authentication(
            Url::parse(ORIGIN).unwrap(),
            serde_json::from_value(options).unwrap(),
        )
        .unwrap();
    let assertion = serde_json::to_string(&assertion).unwrap();
    // An authentic signature for another challenge must still fail.
    let other = f.challenge(OTHER_OPERATION).await;
    status(
        f.authorize(&other, "ok", assertion.clone())
            .await
            .unwrap_err(),
        Code::PermissionDenied,
        "CHECKIN_VERIFICATION_FAILED",
    );
    f.authorize(&challenge, "ok", assertion).await.unwrap();
    let context = f.context().await;
    let receipt = f
        .ledger
        .post(request(
            access(&f.bundle),
            posting(&context, &challenge, 20),
        ))
        .await
        .unwrap()
        .into_inner();
    assert!(!receipt.replayed);
    let revision: i64 = sqlx::query_scalar(
        "SELECT revision FROM core_passkeys.credentials WHERE revoked_at IS NULL",
    )
    .fetch_one(&owner)
    .await
    .unwrap();
    assert_eq!(revision, 2);
    f.cleanup().await;
}

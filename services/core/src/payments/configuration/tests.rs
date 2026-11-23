use super::*;
use crate::{identity::IdentityStore, internal_rpc::pb::core_payments_server::CorePayments};
use axum::{
    body::{Body, to_bytes},
    http::{Request, StatusCode},
};
use std::{
    fs::{self, OpenOptions},
    io::Write,
    os::unix::fs::{OpenOptionsExt, PermissionsExt},
};
use tower::ServiceExt;

const ORIGINAL: &str = "synthetic-epay-original-0123456789";
const REPLACEMENT: &str = "synthetic-epay-replacement-0123456789";

struct Files {
    directory: PathBuf,
    path: PathBuf,
    document: Vec<u8>,
}
impl Files {
    fn new() -> Self {
        let directory =
            std::env::temp_dir().join(format!("lmm-epay-config-{:032x}", rand::random::<u128>()));
        fs::create_dir(&directory).unwrap();
        fs::set_permissions(&directory, fs::Permissions::from_mode(0o700)).unwrap();
        let path = directory.join("payments.toml");
        let document = b"version = 2\n# Synthetic private configuration fixture\n".to_vec();
        let mut file = OpenOptions::new()
            .write(true)
            .create_new(true)
            .mode(0o600)
            .open(&path)
            .unwrap();
        file.write_all(&document).unwrap();
        file.sync_all().unwrap();
        Self {
            directory,
            path,
            document,
        }
    }
    fn store(&self, pool: &PgPool) -> PaymentsStore {
        PaymentsStore::new(pool.clone(), vec![channel(false)])
            .unwrap()
            .with_configuration_file(self.path.clone(), &self.document)
            .unwrap()
    }
}
impl Drop for Files {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.directory);
    }
}
fn channel(enabled: bool) -> Channel {
    Channel {
        id: 7,
        provider: "epay".into(),
        merchant: "merchant_fixed".into(),
        environment: "test".into(),
        currency: "LDC".into(),
        webhook_secrets: vec![ORIGINAL.into()],
        credits_per_minor: 500,
        credit_quote: None,
        max_amount_minor: 100_000,
        verification: ProviderVerification {
            enabled: Some(enabled),
            payment_type: "epay".into(),
            endpoint: "https://credit.linux.do/epay".into(),
            notify_url: "https://native.invalid/payments/callbacks/7".into(),
            return_url: "https://native.invalid/wallet".into(),
            ..Default::default()
        },
    }
}
async fn setup(pool: &PgPool) -> (Files, PaymentsStore, String, String, String) {
    crate::core_schema::init(pool).await.unwrap();
    let identity = IdentityStore::from_pool(pool.clone());
    let owner = identity.bootstrap_user(8401, 6).await.unwrap().secret;
    let admin = identity.bootstrap_user(8402, 5).await.unwrap().secret;
    let user = identity.bootstrap_user(8403, 0).await.unwrap().secret;
    let files = Files::new();
    let store = files.store(pool);
    (files, store, owner, admin, user)
}
fn update(revision: &str, secret: &str) -> serde_json::Value {
    json!({
        "expected_revision":revision,
        "endpoint":"https://credit.linux.do/epay",
        "notify_url":"https://native.invalid/api/wallet/callbacks/7",
        "return_url":"https://native.invalid/topup/result",
        "secret_key":secret,
    })
}
async fn call(
    store: &PaymentsStore,
    method: &str,
    secret: Option<&str>,
    input: Option<serde_json::Value>,
) -> (StatusCode, serde_json::Value) {
    let path = if method == "GET" {
        "/core/v1/payment-gateways/epay"
    } else {
        "/core/v1/payment-gateways/epay/7"
    };
    let mut request = Request::builder().method(method).uri(path);
    if let Some(secret) = secret {
        request = request.header("authorization", format!("Bearer {secret}"));
    }
    let body = match input {
        Some(value) => {
            request = request.header("content-type", "application/json");
            Body::from(serde_json::to_vec(&value).unwrap())
        }
        None => Body::empty(),
    };
    let response = http::router(store.clone())
        .oneshot(request.body(body).unwrap())
        .await
        .unwrap();
    let status = response.status();
    assert_eq!(response.headers()["cache-control"], "private, no-store");
    assert_eq!(response.headers()["vary"], "Authorization");
    let bytes = to_bytes(response.into_body(), 64 * 1024).await.unwrap();
    let text = String::from_utf8_lossy(&bytes);
    assert!(!text.contains(ORIGINAL) && !text.contains(REPLACEMENT));
    assert!(!text.contains("secret_file"));
    (status, serde_json::from_slice(&bytes).unwrap())
}
fn rpc_request() -> tonic::Request<pb::GetEPayConfigurationRequest> {
    let mut request = tonic::Request::new(pb::GetEPayConfigurationRequest {});
    request.metadata_mut().insert(
        "authorization",
        "Bearer synthetic-payment-config-service-token"
            .parse()
            .unwrap(),
    );
    request
        .metadata_mut()
        .insert("x-lmm-protocol", "1".parse().unwrap());
    request
}

#[sqlx::test(migrations = false)]
async fn epay_configuration_http_rpc_and_restart_share_effective_revision(pool: PgPool) {
    let (files, _bootstrap_store, owner, _, _) = setup(&pool).await;
    let runtime = crate::test_runtime::RuntimeRole::create(&pool).await;
    runtime.optional("core_payments").await;
    runtime.optional("core_subscriptions").await;
    let store = files.store(&runtime.pool);
    let rpc = super::super::rpc::PaymentsRpc::new(
        store.clone(),
        b"synthetic-payment-config-service-token",
        super::super::rpc::Permissions {
            read_user: true,
            ..Default::default()
        },
    )
    .unwrap();
    let denied_rpc = super::super::rpc::PaymentsRpc::new(
        store.clone(),
        b"synthetic-payment-config-service-token",
        super::super::rpc::Permissions::default(),
    )
    .unwrap();
    assert_eq!(
        denied_rpc
            .get_e_pay_configuration(rpc_request())
            .await
            .unwrap_err()
            .code(),
        tonic::Code::PermissionDenied
    );
    let (status, before) = call(&store, "GET", Some(&owner), None).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(before["data"]["contract"], CONTRACT);
    assert_eq!(before["data"]["bindings"][0]["editable"], true);
    assert_eq!(before["data"]["bindings"][0]["enabled"], false);
    let revision = before["data"]["revision"].as_str().unwrap();
    let frozen = store.freeze().unwrap();
    let (status, after) = call(
        &store,
        "PUT",
        Some(&owner),
        Some(update(revision, REPLACEMENT)),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{after}");
    assert_ne!(after["data"]["revision"], revision);
    let actual = after["data"]["revision"].as_str().unwrap();
    let projected = rpc
        .get_e_pay_configuration(rpc_request())
        .await
        .unwrap()
        .into_inner();
    assert_eq!(projected.revision, actual);
    assert_eq!(projected.bindings[0].merchant_id, "merchant_fixed");
    assert!(!projected.bindings[0].enabled);
    assert_eq!(projected.bindings[0].credits_per_minor, "500");
    assert_eq!(projected.bindings[0].min_amount_minor, "1");
    assert_eq!(frozen.channels[&7].webhook_secrets[0], ORIGINAL);
    assert_eq!(
        frozen.freeze().unwrap().channels[&7].webhook_secrets[0],
        ORIGINAL
    );
    assert_eq!(
        store.freeze().unwrap().channels[&7].webhook_secrets[0],
        REPLACEMENT
    );
    let restored = files.store(&pool);
    assert_eq!(
        restored.epay_configuration().await.unwrap().revision,
        actual
    );
    assert_eq!(
        restored.freeze().unwrap().channels[&7].webhook_secrets[0],
        REPLACEMENT
    );
    assert_eq!(fs::read(&files.path).unwrap(), files.document);
    for item in fs::read_dir(&files.directory).unwrap() {
        let path = item.unwrap().path();
        assert_eq!(
            fs::metadata(path).unwrap().permissions().mode() & 0o777,
            0o600
        );
    }
    // Blank key preserves the current immutable secret file during URL edits.
    let mut input = update(actual, "");
    input["return_url"] = json!("https://native.invalid/wallet/complete");
    let (status, latest) = call(&store, "PUT", Some(&owner), Some(input)).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(
        files.store(&pool).freeze().unwrap().channels[&7].webhook_secrets[0],
        REPLACEMENT
    );
    assert_eq!(
        rpc.get_e_pay_configuration(rpc_request())
            .await
            .unwrap()
            .into_inner()
            .revision,
        latest["data"]["revision"].as_str().unwrap()
    );
    runtime.cleanup().await;
}

#[sqlx::test(migrations = false)]
async fn epay_configuration_owner_cas_and_fixed_identity_are_enforced(pool: PgPool) {
    let (files, store, owner, admin, user) = setup(&pool).await;
    let revision = store.epay_configuration().await.unwrap().revision;
    for secret in [None, Some(admin.as_str()), Some(user.as_str())] {
        assert_eq!(
            call(&store, "GET", secret, None).await.0,
            StatusCode::FORBIDDEN
        );
        assert_eq!(
            call(&store, "PUT", secret, Some(update(&revision, REPLACEMENT)))
                .await
                .0,
            StatusCode::FORBIDDEN
        );
    }
    assert_eq!(
        call(
            &store,
            "PUT",
            Some(&owner),
            Some(update(&"00".repeat(32), ""))
        )
        .await
        .0,
        StatusCode::CONFLICT
    );
    let mut invalid = update(&revision, "");
    invalid["endpoint"] = json!("https://unrelated.invalid/epay");
    assert_eq!(
        call(&store, "PUT", Some(&owner), Some(invalid)).await.0,
        StatusCode::UNPROCESSABLE_ENTITY
    );
    for forbidden in [
        "merchant_id",
        "enabled",
        "credits_per_minor",
        "currency",
        "min_amount_minor",
        "max_amount_minor",
    ] {
        let mut input = update(&revision, "");
        input[forbidden] = json!("cannot_change");
        assert_eq!(
            call(&store, "PUT", Some(&owner), Some(input)).await.0,
            StatusCode::UNPROCESSABLE_ENTITY
        );
    }
    assert_eq!(store.epay_configuration().await.unwrap().revision, revision);
    assert_eq!(fs::read_dir(&files.directory).unwrap().count(), 1);
    // An external private-file edit cannot be silently overwritten by the UI CAS.
    fs::write(&files.path, b"version = 2\n# manually changed\n").unwrap();
    assert_eq!(
        call(
            &store,
            "PUT",
            Some(&owner),
            Some(update(&revision, REPLACEMENT))
        )
        .await
        .0,
        StatusCode::CONFLICT
    );
    assert_eq!(
        store.freeze().unwrap().channels[&7].webhook_secrets[0],
        ORIGINAL
    );
    // Existing read-only 0640 configurations still start, without claiming editability.
    fs::write(&files.path, &files.document).unwrap();
    fs::set_permissions(&files.path, fs::Permissions::from_mode(0o640)).unwrap();
    let readonly = files.store(&pool);
    assert!(!readonly.epay_configuration().await.unwrap().bindings[0].editable);
}

#[sqlx::test(migrations = false)]
async fn epay_configuration_audit_failures_report_unchanged_or_effective_state(pool: PgPool) {
    let (files, store, owner, _, _) = setup(&pool).await;
    let revision = store.epay_configuration().await.unwrap().revision;
    sqlx::raw_sql("CREATE FUNCTION public.reject_epay_prepared() RETURNS trigger LANGUAGE plpgsql AS $f$ BEGIN IF NEW.action='payment_configuration.prepare' THEN RAISE EXCEPTION 'synthetic audit rejection'; END IF; RETURN NEW; END $f$; CREATE TRIGGER reject_epay_prepared BEFORE INSERT ON core_identity.audit FOR EACH ROW EXECUTE FUNCTION public.reject_epay_prepared();")
        .execute(&pool).await.unwrap();
    let (status, failure) = call(
        &store,
        "PUT",
        Some(&owner),
        Some(update(&revision, REPLACEMENT)),
    )
    .await;
    assert_eq!(status, StatusCode::SERVICE_UNAVAILABLE);
    assert_eq!(
        failure["error"]["code"],
        "PAYMENT_CONFIGURATION_UNAVAILABLE"
    );
    assert_eq!(store.epay_configuration().await.unwrap().revision, revision);
    assert_eq!(fs::read_dir(&files.directory).unwrap().count(), 1);
    sqlx::raw_sql("DROP TRIGGER reject_epay_prepared ON core_identity.audit; CREATE FUNCTION public.reject_epay_applied() RETURNS trigger LANGUAGE plpgsql AS $f$ BEGIN IF NEW.action='payment_configuration.apply' THEN RAISE EXCEPTION 'synthetic receipt rejection'; END IF; RETURN NEW; END $f$; CREATE TRIGGER reject_epay_applied BEFORE INSERT ON core_identity.audit FOR EACH ROW EXECUTE FUNCTION public.reject_epay_applied();")
        .execute(&pool).await.unwrap();
    let (status, failure) = call(
        &store,
        "PUT",
        Some(&owner),
        Some(update(&revision, REPLACEMENT)),
    )
    .await;
    assert_eq!(status, StatusCode::SERVICE_UNAVAILABLE);
    assert_eq!(
        failure["error"]["code"],
        "PAYMENT_CONFIGURATION_DURABILITY_UNKNOWN"
    );
    let effective = store.epay_configuration().await.unwrap();
    assert_ne!(effective.revision, revision);
    assert_eq!(
        files
            .store(&pool)
            .epay_configuration()
            .await
            .unwrap()
            .revision,
        effective.revision
    );
    assert_eq!(
        store.freeze().unwrap().channels[&7].webhook_secrets[0],
        REPLACEMENT
    );
}

#[sqlx::test(migrations = false)]
async fn epay_configuration_disabled_admission_and_existing_history_are_guarded(pool: PgPool) {
    let (_files, store, owner, _, user) = setup(&pool).await;
    let identity = IdentityStore::from_pool(pool.clone());
    let account = identity.authorize(&user).await.unwrap().owner_account_id;
    let request = || pb::PrepareTopupRequest {
        mutation: Some(pb::MutationContext {
            idempotency_key: "epay_configuration_admission_01".into(),
            expected_version: None,
        }),
        account_id: account,
        payment_channel_id: 7,
        currency: "LDC".into(),
        amount_minor: 100,
        requested_credit_units: 0,
    };
    assert_eq!(
        store
            .freeze()
            .unwrap()
            .prepare(&user, request())
            .await
            .unwrap_err(),
        Error::Denied
    );
    assert_eq!(
        sqlx::query_scalar::<_, i64>("SELECT count(*) FROM core_payments.intents")
            .fetch_one(&pool)
            .await
            .unwrap(),
        0
    );
    // A real existing obligation blocks configuration edits even if the current
    // connection is disabled; editing cannot invalidate its signing key.
    let active = PaymentsStore::new(pool.clone(), vec![channel(true)]).unwrap();
    let intent = active.prepare(&user, request()).await.unwrap();
    assert!(intent.intent_id > 0);
    let before = store.epay_configuration().await.unwrap();
    assert!(!before.bindings[0].editable);
    let (status, _) = call(
        &store,
        "PUT",
        Some(&owner),
        Some(update(&before.revision, REPLACEMENT)),
    )
    .await;
    assert_eq!(status, StatusCode::CONFLICT);
    assert_eq!(
        store.epay_configuration().await.unwrap().revision,
        before.revision
    );
    assert_eq!(
        store.prepare(&user, request()).await.unwrap().intent_id,
        intent.intent_id
    );
    assert!(store.epay_checkout(intent.intent_id).await.is_ok());
}

#[sqlx::test(migrations = false)]
async fn epay_configuration_wallet_minimum_keeps_existing_intents_and_final_quotes(pool: PgPool) {
    use rand::SeedableRng;
    use rsa::pkcs8::EncodePublicKey;
    let (_files, _disabled, _, _, user) = setup(&pool).await;
    let identity = IdentityStore::from_pool(pool.clone());
    let account = identity.authorize(&user).await.unwrap().owner_account_id;
    let make_request = |key: &str, amount: i64| pb::PrepareTopupRequest {
        mutation: Some(pb::MutationContext {
            idempotency_key: key.into(),
            expected_version: None,
        }),
        account_id: account,
        payment_channel_id: 7,
        currency: "LDC".into(),
        amount_minor: amount,
        requested_credit_units: 0,
    };
    let original = PaymentsStore::new(pool.clone(), vec![channel(true)]).unwrap();
    let intent = original
        .prepare(&user, make_request("epay_minimum_existing_01", 100))
        .await
        .unwrap();
    let mut limited = channel(true);
    limited.verification.min_amount_minor = Some(10_000);
    let store = PaymentsStore::new(pool.clone(), vec![limited.clone()]).unwrap();
    assert_eq!(
        store
            .prepare(&user, make_request("epay_minimum_existing_01", 100))
            .await
            .unwrap()
            .intent_id,
        intent.intent_id
    );
    assert!(store.epay_checkout(intent.intent_id).await.is_ok());
    assert_eq!(
        store
            .prepare(&user, make_request("epay_minimum_below_01", 9_999))
            .await
            .unwrap_err(),
        Error::Invalid
    );
    let boundary = store
        .prepare(&user, make_request("epay_minimum_boundary_01", 10_000))
        .await
        .unwrap();
    assert_eq!(
        (boundary.amount_minor, boundary.credit_units),
        (10_000, 5_000_000)
    );
    limited.verification.min_amount_minor = Some(0);
    assert_eq!(
        PaymentsStore::new(pool.clone(), vec![limited.clone()]).err(),
        Some(Error::Invalid)
    );
    limited.verification.min_amount_minor = Some(100_001);
    assert_eq!(
        PaymentsStore::new(pool.clone(), vec![limited]).err(),
        Some(Error::Invalid)
    );

    // Quantity discounts must enforce the final payable amount in both quote
    // and prepare, while the provider's subscription contracts remain separate.
    let public = rsa::PrivateKey::new(&mut rand::rngs::StdRng::seed_from_u64(45), 2048)
        .unwrap()
        .to_public_key()
        .to_public_key_pem(Default::default())
        .unwrap();
    let mut quantity = Channel::pancake_isolated_test(
        8,
        "synthetic_quantity".into(),
        public,
        "STO_minimum".into(),
        "PROD_minimum".into(),
        "USD".into(),
        100_000,
    )
    .unwrap();
    quantity.credits_per_minor = 0;
    quantity.credit_quote = Some(CreditQuotePolicy {
        min_credit_units: 1,
        settlement_per_usd_numerator: 1,
        settlement_per_usd_denominator: 1,
        discounts: BTreeMap::from([(
            50_000_000,
            CreditQuoteRatio {
                numerator: 1,
                denominator: 2,
            },
        )]),
    });
    quantity.verification.min_amount_minor = Some(10_000);
    let quantity = PaymentsStore::new(pool.clone(), vec![quantity]).unwrap();
    assert_eq!(
        quantity
            .quote_discount(&user, 8, 50_000_000, "")
            .await
            .err(),
        Some(Error::Invalid)
    );
    let prepare = |key: &str, credits| pb::PrepareTopupRequest {
        mutation: Some(pb::MutationContext {
            idempotency_key: key.into(),
            expected_version: None,
        }),
        account_id: account,
        payment_channel_id: 8,
        currency: "USD".into(),
        amount_minor: 0,
        requested_credit_units: credits,
    };
    assert_eq!(
        quantity
            .prepare(&user, prepare("epay_minimum_quote_below_01", 50_000_000))
            .await
            .err(),
        Some(Error::Invalid)
    );
    let quote = quantity
        .quote_discount(&user, 8, 100_000_000, "")
        .await
        .unwrap();
    assert_eq!(quote.amount_minor, 20_000);
    let prepared = quantity
        .prepare(&user, prepare("epay_minimum_quote_pass_01", 100_000_000))
        .await
        .unwrap();
    assert_eq!(prepared.amount_minor, quote.amount_minor);
}

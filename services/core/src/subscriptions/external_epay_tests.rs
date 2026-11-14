use super::*;
use crate::test_runtime as runtime;
use crate::{
    identity::IdentityStore,
    internal_rpc::pb,
    payments::{Channel, PaymentsStore, ProviderVerification},
};
use md5::{Digest, Md5};
use std::collections::BTreeMap;
const SECRET: &str = "synthetic-subscription-epay-key";
struct Fixture {
    store: SubscriptionStore,
    payments: PaymentsStore,
    admin: String,
    user: String,
    other: String,
    account: i64,
    plan: Plan,
}
async fn fixture(pool: &PgPool, max: i64) -> Fixture {
    crate::core_schema::init(pool).await.unwrap();
    let identity = IdentityStore::from_pool(pool.clone());
    let admin = identity.bootstrap_user(8301, 6).await.unwrap().secret;
    let user = identity.bootstrap_user(8302, 0).await.unwrap().secret;
    let other = identity.bootstrap_user(8303, 0).await.unwrap().secret;
    let account = identity.authorize(&user).await.unwrap().owner_account_id;
    let channel = Channel {
        id: 7,
        provider: "epay".into(),
        merchant: "1001".into(),
        environment: "test".into(),
        currency: "CNY".into(),
        webhook_secrets: vec![SECRET.into()],
        credits_per_minor: 1,
        credit_quote: None,
        max_amount_minor: 10000,
        verification: ProviderVerification {
            payment_type: "alipay".into(),
            endpoint: "https://epay.example.test".into(),
            notify_url: "https://lmm.example.test/payments/callbacks/7".into(),
            return_url: "https://lmm.example.test/subscriptions".into(),
            ..Default::default()
        },
    };
    let payments = PaymentsStore::new(pool.clone(), vec![channel]).unwrap();
    let store = SubscriptionStore::new(pool.clone());
    let plan = store
        .save_plan(
            &admin,
            None,
            SavePlan {
                operation_key: "save_epay_plan_01".into(),
                expected_revision: None,
                terms: PlanTerms {
                    title: "Synthetic EPay".into(),
                    price_units: "100".into(),
                    duration_seconds: 3600,
                    duration_months: 0,
                    reset_period: "grant".into(),
                    reset_seconds: 0,
                    limit_units: "1000".into(),
                    models: vec!["*".into()],
                    groups: vec!["*".into()],
                    max_purchases_per_user: max,
                    enabled: true,
                    allow_balance_pay: true,
                },
            },
        )
        .await
        .unwrap();
    store
        .save_epay_price(
            &payments,
            &admin,
            "set_epay_price_01",
            &ExternalPrice {
                plan_id: plan.id,
                plan_revision: plan.revision,
                channel_id: 7,
                currency: "CNY".into(),
                amount_minor: "125".into(),
            },
        )
        .await
        .unwrap();
    Fixture {
        store,
        payments,
        admin,
        user,
        other,
        account,
        plan,
    }
}
fn signed(order: &str, transaction: &str, amount: &str) -> (Vec<u8>, Vec<u8>) {
    let mut p = BTreeMap::from([
        ("pid", "1001"),
        ("type", "alipay"),
        ("out_trade_no", order),
        ("trade_no", transaction),
        ("money", amount),
        ("trade_status", "TRADE_SUCCESS"),
    ]);
    let body = p
        .iter()
        .map(|(k, v)| format!("{k}={v}"))
        .collect::<Vec<_>>()
        .join("&")
        + SECRET;
    let sign = crate::hex_codec::encode(Md5::digest(body.as_bytes()));
    p.insert("sign", &sign);
    p.insert("sign_type", "MD5");
    (
        form_urlencoded::Serializer::new(String::new())
            .extend_pairs(&p)
            .finish()
            .into_bytes(),
        sign.into_bytes(),
    )
}
async fn prepare(f: &Fixture, key: &str) -> EPayPurchase {
    f.store
        .prepare_epay_purchase(
            &f.payments,
            &f.user,
            &EPayPurchaseRequest {
                operation_key: key.into(),
                plan_id: f.plan.id,
                channel_id: 7,
            },
        )
        .await
        .unwrap()
}
#[sqlx::test(migrations = false)]
async fn original_proof_grants_once_after_cancel_reprice_and_key_rotation(pool: PgPool) {
    let mut f = fixture(&pool, 3).await;
    let runtime = runtime::RuntimeRole::create(&pool).await;
    runtime.optional("core_subscriptions").await;
    f.store = SubscriptionStore::new(runtime.pool.clone());
    let o = prepare(&f, "prepare_epay_order_01").await;
    assert_eq!(o.amount_minor, "125");
    assert_eq!(o.account_id, f.account);
    assert_eq!(prepare(&f, "prepare_epay_order_01").await, o);
    assert_eq!(
        f.store
            .read_epay_purchase(&f.other, &o.order_id)
            .await
            .unwrap_err(),
        Error::NotFound
    );
    let checkout = f
        .store
        .confirm_epay_purchase(&f.payments, &f.user, &o.order_id)
        .await
        .unwrap();
    let url = reqwest::Url::parse(&checkout.checkout_url).unwrap();
    let params: BTreeMap<_, _> = url.query_pairs().collect();
    assert_eq!(
        params["notify_url"],
        "https://lmm.example.test/payments/callbacks/subscriptions/7"
    );
    assert_eq!(params["out_trade_no"], o.order_id.as_str());
    let snapshot_sql = "SELECT jsonb_build_object('order',(SELECT to_jsonb(o) FROM core_subscriptions.external_orders o WHERE id=$1),'identifiers',(SELECT jsonb_agg(to_jsonb(i) ORDER BY i.kind,i.identifier) FROM core_payments.provider_identifiers i WHERE subscription_order_id=$1))";
    let before: Value = sqlx::query_scalar(snapshot_sql)
        .bind(&o.order_id)
        .fetch_one(&pool)
        .await
        .unwrap();
    let (read, recovered) = f
        .store
        .read_epay_purchase_with_checkout(&f.payments, &f.user, &o.order_id)
        .await
        .unwrap();
    assert!(read.confirmed_at.is_some());
    assert_eq!(recovered, Some(checkout.clone()));
    let unavailable = PaymentsStore::new(pool.clone(), vec![]).unwrap();
    let (read, recovered) = f
        .store
        .read_epay_purchase_with_checkout(&unavailable, &f.user, &o.order_id)
        .await
        .unwrap();
    assert!(read.confirmed_at.is_some());
    assert!(recovered.is_none());
    let after: Value = sqlx::query_scalar(snapshot_sql)
        .bind(&o.order_id)
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(
        before, after,
        "checkout GET mutated confirmation or provider identity"
    );
    f.store
        .cancel_epay_purchase(&f.user, &o.order_id)
        .await
        .unwrap();
    assert!(
        f.store
            .confirm_epay_purchase(&f.payments, &f.user, &o.order_id)
            .await
            .is_err()
    );
    let mut edited = f.plan.terms.clone();
    edited.limit_units = "2000".into();
    edited.price_units = "999".into();
    f.store
        .save_plan(
            &f.admin,
            Some(f.plan.id),
            SavePlan {
                operation_key: "edit_epay_plan_01".into(),
                expected_revision: Some(1),
                terms: edited,
            },
        )
        .await
        .unwrap();
    let (raw, sig) = signed(&o.order_id, "epay-transaction-01", "1.25");
    let (wrong, wrong_sig) = signed(&o.order_id, "epay-transaction-01", "1.26");
    assert!(
        f.store
            .accept_epay_proof(&f.payments, &o.order_id, &wrong, &wrong_sig)
            .await
            .is_err()
    );
    let mut corrupt = sig.clone();
    corrupt[0] = b'z';
    assert!(
        f.store
            .accept_epay_proof(&f.payments, &o.order_id, &raw, &corrupt)
            .await
            .is_err()
    );
    let p = f
        .store
        .accept_epay_proof(&f.payments, &o.order_id, &raw, &sig)
        .await
        .unwrap();
    assert_eq!(
        f.store
            .accept_epay_proof(&f.payments, &o.order_id, &raw, &sig)
            .await
            .unwrap(),
        p
    );
    // Once Core accepted the original bytes, processing no longer needs the old key.
    let mut channel = f.payments.subscription_channel(7).unwrap().clone();
    channel.webhook_secrets = vec!["rotated-synthetic-key-123".into()];
    let rotated = PaymentsStore::new(pool.clone(), vec![channel]).unwrap();
    assert_eq!(
        f.store
            .accept_epay_proof(&rotated, &o.order_id, &raw, &sig)
            .await
            .unwrap(),
        p
    );
    assert_eq!(
        f.store
            .get_epay_proof(&o.order_id, &p.event_id, &p.payload_sha256)
            .await
            .unwrap(),
        p
    );
    let g = f
        .store
        .apply_epay_grant(&o.order_id, &p.proof_id, "original_grant_key_01")
        .await
        .unwrap();
    assert_eq!(
        f.store
            .apply_epay_grant(&o.order_id, &p.proof_id, "original_grant_key_01")
            .await
            .unwrap(),
        g
    );
    assert_eq!(
        f.store
            .apply_epay_grant(&o.order_id, &p.proof_id, "changed_grant_key_01")
            .await
            .unwrap_err(),
        Error::Conflict
    );
    let (limit,count,journals):(i64,i64,i64)=sqlx::query_as("SELECT s.limit_credits,(SELECT count(*) FROM core_subscriptions.external_grants),(SELECT count(*) FROM core_billing.ledger_journals) FROM core_charging.subscriptions s WHERE id=$1").bind(g.subscription_id).fetch_one(&pool).await.unwrap();
    assert_eq!((limit, count, journals), (1000, 1, 0));
    runtime.cleanup().await;
}
#[sqlx::test(migrations = false)]
async fn wallet_and_epay_share_one_limit_in_both_directions_and_concurrently(pool: PgPool) {
    let f = fixture(&pool, 1).await;
    sqlx::query("SELECT core_billing.open_ledger_wallet($1)")
        .bind(f.account)
        .execute(&pool)
        .await
        .unwrap();
    let request = json!({"scope":"epay-quota-fixture","operation_key":"fund-wallet-for-both","reason":"Synthetic test only","action":{"kind":"credit","account_id":f.account,"amount_units":1000,"payment_reference":"synthetic-quota-fixture"}});
    sqlx::query("SELECT core_billing.post_ledger($1)")
        .bind(request)
        .execute(&pool)
        .await
        .unwrap();
    for direction in 0..3 {
        let plan = f
            .store
            .save_plan(
                &f.admin,
                None,
                SavePlan {
                    operation_key: format!("quota_plan_{direction:016}"),
                    expected_revision: None,
                    terms: f.plan.terms.clone(),
                },
            )
            .await
            .unwrap();
        f.store
            .save_epay_price(
                &f.payments,
                &f.admin,
                &format!("quota_price_{direction:016}"),
                &ExternalPrice {
                    plan_id: plan.id,
                    plan_revision: 1,
                    channel_id: 7,
                    currency: "CNY".into(),
                    amount_minor: "125".into(),
                },
            )
            .await
            .unwrap();
        let q = f
            .store
            .quote(
                &f.user,
                QuoteRequest {
                    operation_key: format!("quota_wallet_{direction:016}"),
                    plan_id: plan.id,
                    renew_from: None,
                },
            )
            .await
            .unwrap();
        let o = f
            .store
            .prepare_epay_purchase(
                &f.payments,
                &f.user,
                &EPayPurchaseRequest {
                    operation_key: format!("quota_external_{direction:016}"),
                    plan_id: plan.id,
                    channel_id: 7,
                },
            )
            .await
            .unwrap();
        if direction == 0 {
            f.store
                .confirm_epay_purchase(&f.payments, &f.user, &o.order_id)
                .await
                .unwrap();
            assert_eq!(
                f.store.confirm(&f.user, &q.id).await.unwrap_err(),
                Error::Conflict
            );
        } else if direction == 1 {
            assert_eq!(
                f.store.confirm(&f.user, &q.id).await.unwrap().status,
                "posted"
            );
            assert_eq!(
                f.store
                    .confirm_epay_purchase(&f.payments, &f.user, &o.order_id)
                    .await
                    .unwrap_err(),
                Error::Conflict
            );
        } else {
            let (wallet, external) = tokio::join!(
                f.store.confirm(&f.user, &q.id),
                f.store
                    .confirm_epay_purchase(&f.payments, &f.user, &o.order_id)
            );
            assert_ne!(
                wallet.is_ok(),
                external.is_ok(),
                "only one direction may reserve the final slot"
            );
            assert_eq!(
                purchase_count(&mut pool.acquire().await.unwrap(), 8302, plan.id)
                    .await
                    .unwrap(),
                1
            );
            assert!(
                f.store.confirm(&f.user, &q.id).await.is_ok()
                    || f.store
                        .confirm_epay_purchase(&f.payments, &f.user, &o.order_id)
                        .await
                        .is_ok()
            );
        }
    }
}
#[sqlx::test(migrations = false)]
async fn wallet_and_subscription_payment_proof_cannot_share_transaction(pool: PgPool) {
    let f = fixture(&pool, 0).await;
    let o = prepare(&f, "namespace_epay_order_01").await;
    f.store
        .confirm_epay_purchase(&f.payments, &f.user, &o.order_id)
        .await
        .unwrap();
    let (raw, sig) = signed(&o.order_id, "shared-transaction-01", "1.25");
    f.store
        .accept_epay_proof(&f.payments, &o.order_id, &raw, &sig)
        .await
        .unwrap();
    let intent = f
        .payments
        .prepare(
            &f.user,
            pb::PrepareTopupRequest {
                requested_credit_units: 0,
                mutation: Some(pb::MutationContext {
                    idempotency_key: "namespace_native_order".into(),
                    ..Default::default()
                }),
                account_id: f.account,
                payment_channel_id: 7,
                currency: "CNY".into(),
                amount_minor: 125,
            },
        )
        .await
        .unwrap();
    let (raw, sig) = signed(
        &format!("top_{}", intent.intent_id),
        "shared-transaction-01",
        "1.25",
    );
    let e = pb::ProviderEvidence {
        payment: Some(pb::ExternalPaymentReference {
            provider: "epay".into(),
            merchant: "1001".into(),
            environment: "test".into(),
            transaction: "shared-transaction-01".into(),
        }),
        signed_payload: raw,
        signature: sig,
        provider_event_id: String::new(),
        accepted_proof_id: String::new(),
    };
    // The correct provider event identity is derived by the shared verifier.
    let (_, event, _) = f
        .payments
        .subscription_verify(
            &f.payments.subscription_binding(7).unwrap(),
            &format!("top_{}", intent.intent_id),
            125,
            &e.signed_payload,
            &e.signature,
        )
        .unwrap();
    let e = pb::ProviderEvidence {
        provider_event_id: event,
        ..e
    };
    assert_eq!(
        f.payments
            .accept_proof(pb::AcceptProviderProofRequest {
                intent_id: intent.intent_id,
                evidence: Some(e)
            })
            .await
            .unwrap_err(),
        crate::payments::Error::Conflict
    );
}

#[sqlx::test(migrations = false)]
async fn actual_core_rpc_and_tcp_callbacks_reject_poisoning_and_recover_one_at_a_time(
    pool: PgPool,
) {
    use sqlx::ConnectOptions;
    use std::{fs, os::unix::fs::PermissionsExt, path::PathBuf};
    let f = fixture(&pool, 0).await;
    let first = prepare(&f, "joint_epay_first_order").await;
    let second = prepare(&f, "joint_epay_second_order").await;
    f.store
        .confirm_epay_purchase(&f.payments, &f.user, &first.order_id)
        .await
        .unwrap();
    f.store
        .confirm_epay_purchase(&f.payments, &f.user, &second.order_id)
        .await
        .unwrap();
    let (raw1, _) = signed(&first.order_id, "joint-first-transaction", "1.25");
    let (raw2, _) = signed(&second.order_id, "joint-second-transaction", "1.25");
    let third = prepare(&f, "joint_epay_third_order").await;
    f.store
        .confirm_epay_purchase(&f.payments, &f.user, &third.order_id)
        .await
        .unwrap();
    let (raw3, _) = signed(&third.order_id, "joint-permission-transaction", "1.25");
    let fourth = prepare(&f, "joint_epay_provider_order").await;
    f.store
        .confirm_epay_purchase(&f.payments, &f.user, &fourth.order_id)
        .await
        .unwrap();
    let (raw4, _) = signed(&fourth.order_id, "joint-provider-transaction", "1.25");
    let dir = PathBuf::from(format!("/tmp/sub01-joint-{:016x}", rand::random::<u64>()));
    fs::create_dir(&dir).unwrap();
    fs::set_permissions(&dir, fs::Permissions::from_mode(0o700)).unwrap();
    let socket = dir.join("core.sock");
    let token = b"subscription-service-synthetic-token-12345";
    let rpc = epay_rpc::EPaySubscriptionsRpc::new(
        f.store.clone(),
        f.payments.clone(),
        token,
        epay_rpc::Permissions {
            user_orders: true,
            proof_ingress: true,
            proof_read: true,
            grant: true,
        },
    )
    .unwrap();
    let server = crate::internal_rpc::RpcServer::bind(
        &socket,
        b"transport-only-synthetic-token-12345",
        None,
    )
    .unwrap()
    .with_epay_subscriptions(rpc);
    let (stop, rx) = tokio::sync::watch::channel(false);
    let serving = tokio::spawn(server.serve(rx));
    let restricted_socket = dir.join("restricted.sock");
    let restricted_rpc = epay_rpc::EPaySubscriptionsRpc::new(
        f.store.clone(),
        f.payments.clone(),
        token,
        epay_rpc::Permissions {
            user_orders: true,
            proof_ingress: false,
            proof_read: false,
            grant: false,
        },
    )
    .unwrap();
    let restricted_server = crate::internal_rpc::RpcServer::bind(
        &restricted_socket,
        b"restricted-transport-synthetic-token-123",
        None,
    )
    .unwrap()
    .with_epay_subscriptions(restricted_rpc);
    let (restricted_stop, restricted_rx) = tokio::sync::watch::channel(false);
    let restricted_serving = tokio::spawn(restricted_server.serve(restricted_rx));
    // The constructor accepts this Stripe channel at the same ID. New EPay
    // purchases remain invalid, but an existing frozen obligation is retryable.
    let reassigned_payments = PaymentsStore::new(
        pool.clone(),
        vec![Channel::stripe_test(
            7,
            "acct_sub01_reassigned".into(),
            "whsec_sub01_reassigned_test_secret".into(),
            10000,
        )],
    )
    .unwrap();
    assert!(matches!(
        reassigned_payments.subscription_channel(7),
        Err(crate::payments::Error::Invalid)
    ));
    let reassigned_socket = dir.join("reassigned.sock");
    let reassigned_rpc = epay_rpc::EPaySubscriptionsRpc::new(
        f.store.clone(),
        reassigned_payments,
        token,
        epay_rpc::Permissions {
            user_orders: true,
            proof_ingress: true,
            proof_read: true,
            grant: true,
        },
    )
    .unwrap();
    let reassigned_server = crate::internal_rpc::RpcServer::bind(
        &reassigned_socket,
        b"reassigned-transport-synthetic-token-123",
        None,
    )
    .unwrap()
    .with_epay_subscriptions(reassigned_rpc);
    let (reassigned_stop, reassigned_rx) = tokio::sync::watch::channel(false);
    let reassigned_serving = tokio::spawn(reassigned_server.serve(reassigned_rx));
    let go_db = format!("sub01_joint_{:016x}_test", rand::random::<u64>());
    sqlx::query(sqlx::AssertSqlSafe(format!("CREATE DATABASE {go_db}")))
        .execute(&pool)
        .await
        .unwrap();
    let go_url = pool
        .connect_options()
        .as_ref()
        .clone()
        .database(&go_db)
        .to_url_lossy()
        .to_string();
    let fixture_path = dir.join("fixture.json");
    fs::write(&fixture_path,serde_json::to_vec(&json!({"Socket":socket,"Token":std::str::from_utf8(token).unwrap(),"DatabaseURL":go_url,"DatabaseName":go_db,"FirstOrder":first.order_id,"SecondOrder":second.order_id,"RawFirst":String::from_utf8(raw1).unwrap(),"RawSecond":String::from_utf8(raw2).unwrap(),"RestrictedSocket":restricted_socket,"ThirdOrder":third.order_id,"RawThird":String::from_utf8(raw3).unwrap(),"ReassignedSocket":reassigned_socket,"FourthOrder":fourth.order_id,"RawFourth":String::from_utf8(raw4).unwrap()})).unwrap()).unwrap();
    fs::set_permissions(&fixture_path, fs::Permissions::from_mode(0o600)).unwrap();
    let mut command = std::process::Command::new("mise");
    command
        .args([
            "exec",
            "--",
            "go",
            "test",
            "-p=1",
            "-run",
            "^TestSubscriptionEPayCoreTCPPoisoningRecovery$",
            "-count=1",
            "-v",
            ".",
        ])
        .current_dir(
            PathBuf::from(env!("CARGO_MANIFEST_DIR"))
                .join("../extensions/internal/modules/payments/pgtest"),
        )
        .env("SUB01_CORE_FIXTURE", &fixture_path)
        .env("GOMAXPROCS", "1")
        .env(
            "GOCACHE",
            PathBuf::from(std::env::var("HOME").unwrap()).join(".cache/lmm-api-go-current"),
        );
    let output = tokio::task::spawn_blocking(move || command.output())
        .await
        .unwrap()
        .unwrap();
    stop.send(true).unwrap();
    serving.await.unwrap().unwrap();
    restricted_stop.send(true).unwrap();
    restricted_serving.await.unwrap().unwrap();
    reassigned_stop.send(true).unwrap();
    reassigned_serving.await.unwrap().unwrap();
    sqlx::query(sqlx::AssertSqlSafe(format!("DROP DATABASE {go_db}")))
        .execute(&pool)
        .await
        .unwrap();
    fs::remove_dir_all(&dir).unwrap();
    assert!(
        output.status.success(),
        "Go TCP/Core/PG joint test failed:\n{}\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    println!("{}", String::from_utf8_lossy(&output.stdout));
    let (proofs,grants,journals):(i64,i64,i64)=sqlx::query_as("SELECT (SELECT count(*) FROM core_subscriptions.external_proofs),(SELECT count(*) FROM core_subscriptions.external_grants),(SELECT count(*) FROM core_billing.ledger_journals)").fetch_one(&pool).await.unwrap();
    assert_eq!((proofs, grants, journals), (4, 4, 0));
    let fourth_grants: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM core_subscriptions.external_grants WHERE order_id=$1",
    )
    .bind(&fourth.order_id)
    .fetch_one(&pool)
    .await
    .unwrap();
    assert_eq!(fourth_grants, 1);
}

#[sqlx::test(migrations = false)]
async fn admin_epay_price_get_is_root_only_and_reads_current_disabled_revision(pool: PgPool) {
    let f = fixture(&pool, 0).await;
    let mut disabled = f.plan.terms.clone();
    disabled.enabled = false;
    let revised = f
        .store
        .save_plan(
            &f.admin,
            Some(f.plan.id),
            SavePlan {
                operation_key: "disable_epay_plan_01".into(),
                expected_revision: Some(1),
                terms: disabled,
            },
        )
        .await
        .unwrap();
    f.store
        .save_epay_price(
            &f.payments,
            &f.admin,
            "set_disabled_price_01",
            &ExternalPrice {
                plan_id: f.plan.id,
                plan_revision: revised.revision,
                channel_id: 7,
                currency: "CNY".into(),
                amount_minor: "250".into(),
            },
        )
        .await
        .unwrap();
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let router = epay_price_router(f.store.clone(), f.payments.clone());
    let server = tokio::spawn(async move {
        axum::serve(listener, router).await.unwrap();
    });
    let client = reqwest::Client::new();
    let admin_url = format!(
        "http://{address}/core/v1/subscriptions/admin/plans/{}/epay-prices",
        f.plan.id
    );
    let public_url = format!(
        "http://{address}/core/v1/subscriptions/plans/{}/epay-prices",
        f.plan.id
    );
    let no_session = client.get(&admin_url).send().await.unwrap();
    assert_eq!(no_session.status(), reqwest::StatusCode::UNAUTHORIZED);
    let ordinary = client
        .get(&admin_url)
        .bearer_auth(&f.user)
        .send()
        .await
        .unwrap();
    assert_eq!(ordinary.status(), reqwest::StatusCode::FORBIDDEN);
    let response = client
        .get(&admin_url)
        .bearer_auth(&f.admin)
        .send()
        .await
        .unwrap();
    assert_eq!(response.status(), reqwest::StatusCode::OK);
    assert_eq!(response.headers()["cache-control"], "no-store");
    let body: Value = response.json().await.unwrap();
    assert_eq!(body["prices"].as_array().unwrap().len(), 1);
    assert_eq!(body["prices"][0]["plan_revision"], revised.revision);
    assert_eq!(body["prices"][0]["amount_minor"], "250");
    let public: Value = client
        .get(&public_url)
        .bearer_auth(&f.user)
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(public, json!({"prices":[]}));
    f.store
        .archive_plan(
            &f.admin,
            f.plan.id,
            ArchivePlan {
                operation_key: "archive_epay_plan_01".into(),
                expected_revision: revised.revision,
            },
        )
        .await
        .unwrap();
    let archived: Value = client
        .get(&admin_url)
        .bearer_auth(&f.admin)
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(archived, body);
    server.abort();
}

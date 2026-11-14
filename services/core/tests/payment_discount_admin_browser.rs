//! Opt-in independent browser fixture; synthetic local identities and provider proof only.
//! Run only after the coordinator freezes Core/UI and hands over the Cargo target.
use base64::{Engine, engine::general_purpose::STANDARD};
use lmm_core::{
    billing::ledger,
    identity::{CredentialKind, IdentityStore},
    internal_rpc::pb,
    payments::{self, Channel, CreditQuotePolicy, PaymentsStore, discounts},
};
use rand::{SeedableRng, rngs::StdRng};
use rsa::{Pkcs1v15Sign, PrivateKey, pkcs8::EncodePublicKey};
use serde_json::json;
use sha2::{Digest, Sha256};
use sqlx::PgPool;
use std::{
    collections::BTreeMap,
    fs::OpenOptions,
    io::Write,
    os::unix::fs::{OpenOptionsExt, PermissionsExt},
    path::PathBuf,
    time::{Duration, SystemTime, UNIX_EPOCH},
};

const MERCHANT: &str = "acct_admin_browser_synthetic";

fn private_json(path: PathBuf, value: serde_json::Value) {
    let mut file = OpenOptions::new()
        .write(true)
        .create_new(true)
        .mode(0o600)
        .open(path)
        .unwrap();
    file.write_all(serde_json::to_string_pretty(&value).unwrap().as_bytes())
        .unwrap();
    file.sync_all().unwrap();
}

#[sqlx::test]
#[ignore = "requires frozen browser runner and newly owned PostgreSQL 18.6"]
async fn payment_discount_admin_browser(pool: PgPool) {
    let root =
        PathBuf::from(std::env::var("LMM_ADMIN_BROWSER_OUTPUT").expect("private runner directory"));
    assert!(root.is_absolute());
    let metadata = std::fs::symlink_metadata(&root).unwrap();
    assert!(metadata.is_dir() && metadata.permissions().mode() & 0o077 == 0);
    assert!(!root.join("stop").exists());
    let identity = IdentityStore::from_pool(pool.clone());
    identity.init_database().await.unwrap();
    for ddl in [
        lmm_core::abuse::SCHEMA,
        ledger::SCHEMA,
        payments::SCHEMA,
        discounts::SCHEMA,
        discounts::MANAGEMENT_SCHEMA,
    ] {
        sqlx::raw_sql(ddl).execute(&pool).await.unwrap();
    }
    let issued = identity.bootstrap_user(1099, 6).await.unwrap();
    let actor = identity.authorize(&issued.secret).await.unwrap();
    assert!(matches!(actor.credential_kind, CredentialKind::Session));
    assert_eq!((actor.user_id, actor.platform_level), (1099, 6));
    let key = PrivateKey::new(&mut StdRng::seed_from_u64(43), 2048).unwrap();
    let mut channel = Channel::pancake_isolated_test(
        1,
        MERCHANT.into(),
        key.to_public_key()
            .to_public_key_pem(Default::default())
            .unwrap(),
        "STO_admin_browser".into(),
        "PROD_admin_browser".into(),
        "USD".into(),
        100_000,
    )
    .unwrap();
    channel.credits_per_minor = 0;
    channel.credit_quote = Some(CreditQuotePolicy {
        settlement_per_usd_numerator: 1,
        settlement_per_usd_denominator: 1,
        discounts: BTreeMap::new(),
    });
    let store = PaymentsStore::new(pool.clone(), vec![channel]).unwrap();
    for code in ["PENDING20", "EXHAUSTED20"] {
        store
            .save_discount(
                &issued.secret,
                None,
                discounts::Definition {
                    code: code.into(),
                    name: code.into(),
                    discount_percent: 20,
                    min_credit_units: 0,
                    max_uses: 1,
                    enabled: true,
                    starts_at: 0,
                    ends_at: 0,
                },
            )
            .await
            .unwrap();
        let intent = store
            .prepare_with_discount(
                &issued.secret,
                pb::PrepareTopupRequest {
                    mutation: Some(pb::MutationContext {
                        idempotency_key: format!("admin_browser_seed_{code}"),
                        expected_version: None,
                    }),
                    account_id: actor.owner_account_id,
                    payment_channel_id: 1,
                    currency: "USD".into(),
                    amount_minor: 0,
                    requested_credit_units: 1_000_000,
                },
                code,
            )
            .await
            .unwrap();
        if code == "EXHAUSTED20" {
            let event_id = "delivery_admin_browser_exhausted";
            let transaction = format!("PAY_admin_browser_{}", intent.intent_id);
            let amount = format!(
                "{}.{:02}",
                intent.amount_minor / 100,
                intent.amount_minor % 100
            );
            let payload = serde_json::to_vec(&json!({"id":event_id,"eventId":"payment_admin_browser_exhausted","eventType":"order.completed","mode":"test","storeId":"STO_admin_browser","data":{
                "orderMerchantExternalId":format!("top_{}",intent.intent_id),"orderId":format!("ORD_admin_browser_{}",intent.intent_id),"paymentId":transaction,"orderStatus":"completed","paymentStatus":"succeeded","chargedAmount":amount,"amount":amount,"currency":intent.currency,
                "orderMetadata":{"lmm_product_id":"PROD_admin_browser","lmm_merchant":MERCHANT}
            }})).unwrap();
            let now = SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_millis();
            let mut signed = format!("{now}.").into_bytes();
            signed.extend_from_slice(&payload);
            let signature = key
                .sign(Pkcs1v15Sign::new::<Sha256>(), &Sha256::digest(&signed))
                .unwrap();
            let mut evidence = pb::ProviderEvidence {
                payment: Some(pb::ExternalPaymentReference {
                    provider: "waffo_pancake".into(),
                    merchant: MERCHANT.into(),
                    environment: "test".into(),
                    transaction,
                }),
                signed_payload: payload,
                signature: format!("t={now},v1={}", STANDARD.encode(signature)).into_bytes(),
                provider_event_id: event_id.into(),
                accepted_proof_id: String::new(),
            };
            evidence.accepted_proof_id = store
                .accept_proof(pb::AcceptProviderProofRequest {
                    intent_id: intent.intent_id,
                    evidence: Some(evidence.clone()),
                })
                .await
                .unwrap()
                .proof_id;
            store
                .credit(pb::CreditTopupRequest {
                    mutation: Some(pb::MutationContext {
                        idempotency_key: "admin_browser_seed_credit".into(),
                        expected_version: None,
                    }),
                    intent_id: intent.intent_id,
                    evidence: Some(evidence),
                })
                .await
                .unwrap();
        }
    }
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let (shutdown_tx, shutdown_rx) = tokio::sync::oneshot::channel::<()>();
    let server = tokio::spawn(async move {
        axum::serve(listener, discounts::http::router(store))
            .with_graceful_shutdown(async {
                let _ = shutdown_rx.await;
            })
            .await
            .unwrap();
    });
    private_json(
        root.join("fixture.json"),
        json!({"address":format!("http://{address}"),"access_token":issued.secret,"user_id":actor.user_id,"credential_id":actor.credential_id,"sid":format!("{:064x}",actor.credential_id),"platform_level":actor.platform_level}),
    );
    let deadline = tokio::time::Instant::now() + Duration::from_secs(900);
    while !root.join("stop").exists() && tokio::time::Instant::now() < deadline {
        tokio::time::sleep(Duration::from_millis(200)).await;
    }
    let completed = root.join("stop").is_file()
        && std::fs::read(root.join("stop")).unwrap() == b"browser-complete\n";
    shutdown_tx.send(()).unwrap();
    tokio::time::timeout(Duration::from_secs(10), server)
        .await
        .unwrap()
        .unwrap();
    let counts: (i64,i64,i64,i64,i64,i64) = sqlx::query_as("SELECT (SELECT count(*) FROM core_payments.discount_management_batches),(SELECT count(*) FROM core_payments.discount_codes),(SELECT count(*) FROM core_payments.discount_code_tombstones),(SELECT count(*) FROM core_payments.discount_reservations WHERE NOT consumed),(SELECT count(*) FROM core_payments.discount_reservations WHERE consumed),(SELECT count(*) FROM core_billing.ledger_journals)").fetch_one(&pool).await.unwrap();
    private_json(
        root.join("sql-final.json"),
        json!({"browser_completed":completed,"batches":counts.0,"codes":counts.1,"archives":counts.2,"pending":counts.3,"consumed":counts.4,"journals":counts.5}),
    );
    assert!(
        completed,
        "browser did not finish before timeout; preserve evidence"
    );
    assert_eq!(
        counts,
        (1, 28, 2, 1, 1, 1),
        "one 25-code batch + one created/deleted code + two seeded codes"
    );
    let pending_archived: bool = sqlx::query_scalar("SELECT EXISTS(SELECT FROM core_payments.discount_code_tombstones t JOIN core_payments.discount_codes c ON c.id=t.code_id WHERE c.code='PENDING20')").fetch_one(&pool).await.unwrap();
    assert!(
        !pending_archived,
        "cleanup must preserve unpaid reservation"
    );
    let exact: (String,String,bool) = sqlx::query_as("SELECT min_credit_units::text,max_uses::text,enabled FROM core_payments.discount_codes WHERE code='BROWSER_EXACT_EDITED'").fetch_one(&pool).await.unwrap();
    assert_eq!(
        exact,
        (
            "9007199254740993".into(),
            "9223372036854775807".into(),
            false
        )
    );
}

//! Synthetic signed provider events exercise the real Core payment authority.
use hmac::{Hmac, Mac};
use lmm_core::{
    community_read::{CommunityRead, UsersInfoRequest},
    core_schema,
    identity::IdentityStore,
    internal_rpc::pb,
    payments::{Channel, PaymentsStore},
};
use serde_json::json;
use sha2::Sha256;
use sqlx::PgPool;
use std::time::{SystemTime, UNIX_EPOCH};

const SECRET: &str = "whsec_synthetic_cash_read_only";
const TEST_SECRET: &str = "whsec_synthetic_cash_read_test_only";
const MERCHANT: &str = "acct_synthetic_cash_read";
fn evidence(intent: &pb::PaymentIntent) -> pb::ProviderEvidence {
    let live = intent.environment == "live";
    let event = format!("evt_cash_read_{}", intent.intent_id);
    let transaction = format!("pi_cash_read_{}", intent.intent_id);
    let payload = serde_json::to_vec(&json!({"id":event,"type":"payment_intent.succeeded","livemode":live,
        "data":{"object":{"id":transaction,"object":"payment_intent","status":"succeeded",
        "amount":intent.amount_minor,"amount_received":intent.amount_minor,"currency":"usd","livemode":live,
        "metadata":{"lmm_core_intent_id":intent.intent_id.to_string(),"lmm_merchant":MERCHANT}}}})).unwrap();
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_secs();
    let mut mac = Hmac::<Sha256>::new_from_slice(if live {
        SECRET.as_bytes()
    } else {
        TEST_SECRET.as_bytes()
    })
    .unwrap();
    mac.update(format!("{now}.").as_bytes());
    mac.update(&payload);
    let mut signature = format!("t={now},v1=");
    use std::fmt::Write;
    for byte in mac.finalize().into_bytes() {
        write!(&mut signature, "{byte:02x}").unwrap();
    }
    pb::ProviderEvidence {
        payment: Some(pb::ExternalPaymentReference {
            provider: "stripe".into(),
            merchant: MERCHANT.into(),
            environment: intent.environment.clone(),
            transaction,
        }),
        signed_payload: payload,
        signature: signature.into_bytes(),
        provider_event_id: event,
        accepted_proof_id: String::new(),
    }
}

#[sqlx::test(migrations = false)]
async fn first_cash_requires_live_verified_credit_not_preparation_or_proof(pool: PgPool) {
    core_schema::init(&pool).await.unwrap();
    let identity = IdentityStore::from_pool(pool.clone());
    let live = identity.bootstrap_user(101, 0).await.unwrap();
    let test = identity.bootstrap_user(102, 0).await.unwrap();
    let payments = PaymentsStore::new(
        pool.clone(),
        vec![
            Channel::stripe_live(1, MERCHANT.into(), SECRET.into(), 100_000),
            Channel::stripe_test(2, MERCHANT.into(), TEST_SECRET.into(), 100_000),
        ],
    )
    .unwrap();
    let read = CommunityRead::new(pool.clone()).await.unwrap();
    let request = UsersInfoRequest {
        user_ids: vec![101, 102, 999],
    };
    let mut first_prepared = 0;
    for (index, secret) in [live.secret, test.secret].iter().enumerate() {
        let actor = identity.authorize(secret).await.unwrap();
        let intent = payments
            .prepare(
                secret,
                pb::PrepareTopupRequest {
                    mutation: Some(pb::MutationContext {
                        idempotency_key: format!("cash_read_prepare_{index}"),
                        expected_version: None,
                    }),
                    account_id: actor.owner_account_id,
                    payment_channel_id: index as i64 + 1,
                    currency: "USD".into(),
                    amount_minor: 137,
                },
            )
            .await
            .unwrap();
        let before = read.native_first_cash_payments(&request).await.unwrap();
        assert!(before.native_only);
        if index == 0 {
            assert_eq!(before.users.len(), 2);
            assert!(before.users.iter().all(|u| u.first_paid_at.is_none()));
            first_prepared=sqlx::query_scalar("SELECT floor(extract(epoch FROM created_at))::bigint FROM core_payments.intents WHERE id=$1").bind(intent.intent_id).fetch_one(&pool).await.unwrap();
        }
        let mut proof = evidence(&intent);
        let accepted = payments
            .accept_proof(pb::AcceptProviderProofRequest {
                intent_id: intent.intent_id,
                evidence: Some(proof.clone()),
            })
            .await
            .unwrap();
        proof.accepted_proof_id = accepted.proof_id;
        let uncredited = read.native_first_cash_payments(&request).await.unwrap();
        assert!(
            uncredited.users[index].first_paid_at.is_none(),
            "accepted input alone is not a payment"
        );
        tokio::time::sleep(std::time::Duration::from_millis(1100)).await;
        let credit = payments
            .credit(pb::CreditTopupRequest {
                mutation: Some(pb::MutationContext {
                    idempotency_key: format!("cash_read_credit_{index}"),
                    expected_version: None,
                }),
                intent_id: intent.intent_id,
                evidence: Some(proof),
            })
            .await
            .unwrap();
        assert_eq!(credit.state, 1);
    }
    let result = read.native_first_cash_payments(&request).await.unwrap();
    assert!(
        result.users[0].first_paid_at.unwrap() > first_prepared,
        "payment time must be successful credit time"
    );
    assert!(
        result.users[1].first_paid_at.is_none(),
        "test-mode credits never count as cash"
    );
    assert!(
        read.native_first_cash_payments(&UsersInfoRequest { user_ids: vec![] })
            .await
            .is_err()
    );
    assert!(
        read.native_first_cash_payments(&UsersInfoRequest { user_ids: vec![0] })
            .await
            .is_err()
    );
    assert!(
        read.native_first_cash_payments(&UsersInfoRequest {
            user_ids: vec![101; 101]
        })
        .await
        .is_err()
    );
    // Exercise the real Go decoder against a populated Core response as well.
    use std::os::unix::fs::DirBuilderExt;
    let dir = std::env::temp_dir().join(format!("lmm-core-go-cash-{}", std::process::id()));
    std::fs::DirBuilder::new().mode(0o700).create(&dir).unwrap();
    let socket = dir.join("core.sock");
    let listener = tokio::net::UnixListener::bind(&socket).unwrap();
    let router =
        lmm_core::community_read::http::router(read, b"synthetic-community-purpose-token-20261104")
            .unwrap();
    let server = tokio::spawn(async move {
        axum::serve(listener, router).await.unwrap();
    });
    let output = tokio::task::spawn_blocking(move || {
        std::process::Command::new("go")
            .args([
                "test",
                "./internal/modules/community",
                "-run",
                "^TestCoreCashPaymentsLive$",
                "-count=1",
            ])
            .current_dir(concat!(env!("CARGO_MANIFEST_DIR"), "/../extensions"))
            .env("LMM_TEST_CORE_CASH_SOCKET", socket)
            .output()
            .unwrap()
    })
    .await
    .unwrap();
    server.abort();
    let _ = server.await;
    std::fs::remove_dir_all(dir).unwrap();
    assert!(
        output.status.success(),
        "Go cash read: {} {}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
}

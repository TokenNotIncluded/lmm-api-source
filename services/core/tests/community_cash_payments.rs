//! Synthetic signed provider events exercise the real Core payment authority.
use hmac::{Hmac, Mac};
use lmm_core::{
    community_read::{CashPeriodRequest, CommunityRead, UsersInfoRequest},
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
    evidence_kind(intent, false)
}
fn evidence_kind(intent: &pb::PaymentIntent, refund: bool) -> pb::ProviderEvidence {
    let live = intent.environment == "live";
    let event = format!(
        "evt_cash_{}_{}",
        if refund { "refund" } else { "read" },
        intent.intent_id
    );
    let transaction = format!("pi_cash_read_{}", intent.intent_id);
    let payload = if refund {
        serde_json::to_vec(&json!({"id":event,"type":"refund.updated","livemode":live,"data":{"object":{"id":format!("re_cash_read_{}",intent.intent_id),"object":"refund","status":"succeeded","amount":17,"currency":"usd","payment_intent":transaction,"livemode":live,"metadata":{"lmm_core_intent_id":intent.intent_id.to_string(),"lmm_merchant":MERCHANT}}}})).unwrap()
    } else {
        serde_json::to_vec(&json!({"id":event,"type":"payment_intent.succeeded","livemode":live,
        "data":{"object":{"id":transaction,"object":"payment_intent","status":"succeeded",
        "amount":intent.amount_minor,"amount_received":intent.amount_minor,"currency":"usd","livemode":live,
        "metadata":{"lmm_core_intent_id":intent.intent_id.to_string(),"lmm_merchant":MERCHANT}}}})).unwrap()
    };
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
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_secs() as i64;
    let period = CashPeriodRequest {
        user_ids: vec![101, 102, 999],
        start: now - 3600,
        end: now + 60,
    };
    for (index, secret) in [live.secret.clone(), test.secret, live.secret]
        .iter()
        .enumerate()
    {
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
                    payment_channel_id: (index % 2) as i64 + 1,
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
        if index < 2 {
            assert!(
                uncredited.users[index].first_paid_at.is_none(),
                "accepted input alone is not a payment"
            );
        }
        assert!(
            read.first_cash_payments(&request).await.unwrap().users[0]
                .second_paid_at
                .is_none(),
            "proof acceptance and credit replay cannot create repeat payments"
        );
        if index == 0 {
            assert!(
                read.cash_payment_totals(&period).await.is_err(),
                "accepted payment without applied credit is not a zero total"
            );
        }
        tokio::time::sleep(std::time::Duration::from_millis(1100)).await;
        let credit_request = pb::CreditTopupRequest {
            mutation: Some(pb::MutationContext {
                idempotency_key: format!("cash_read_credit_{index}"),
                expected_version: None,
            }),
            intent_id: intent.intent_id,
            evidence: Some(proof),
        };
        let credit = payments.credit(credit_request.clone()).await.unwrap();
        assert_eq!(credit.state, 1);
        assert_eq!(payments.credit(credit_request).await.unwrap(), credit);
        if index == 0 {
            let mut proof = evidence_kind(&intent, true);
            proof.accepted_proof_id = payments
                .accept_proof(pb::AcceptProviderProofRequest {
                    intent_id: intent.intent_id,
                    evidence: Some(proof.clone()),
                })
                .await
                .unwrap()
                .proof_id;
            assert!(
                read.cash_payment_totals(&period).await.is_err(),
                "accepted refund must be applied before net totals are final"
            );
            let request = pb::ExternalRefundRequest {
                mutation: Some(pb::MutationContext {
                    idempotency_key: "cash_read_refund_00".into(),
                    expected_version: None,
                }),
                intent_id: intent.intent_id,
                evidence: Some(proof),
            };
            let refund = payments.external_refund(request.clone()).await.unwrap();
            assert_eq!(payments.external_refund(request).await.unwrap(), refund);
        }
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
    let totals = read.cash_payment_totals(&period).await.unwrap();
    assert_eq!(totals.users.len(), 2);
    let usd = &totals.users[0].currencies[0];
    assert_eq!(
        (
            usd.paid_micros,
            usd.refund_micros,
            usd.net_micros,
            usd.orders
        ),
        (2740000, 170000, 2570000, 2)
    );
    assert!(
        totals.users[1].currencies.is_empty(),
        "test credits are not cash revenue"
    );
    // Exercise the real Go decoder against a populated Core response as well.
    use std::os::unix::fs::DirBuilderExt;
    let dir = std::env::temp_dir().join(format!("lmm-core-go-cash-{}", std::process::id()));
    std::fs::DirBuilder::new().mode(0o700).create(&dir).unwrap();
    let socket = dir.join("core.sock");
    let listener = tokio::net::UnixListener::bind(&socket).unwrap();
    let router = lmm_core::community_read::http::router(
        read.clone(),
        b"synthetic-community-purpose-token-20261104",
    )
    .unwrap();
    let server = tokio::spawn(async move {
        axum::serve(listener, router).await.unwrap();
    });
    let output = tokio::task::spawn_blocking(move || {
        std::process::Command::new("go")
            .args([
                "test",
                "./internal/modules/community",
                "./internal/modules/acquisition",
                "-run",
                "^TestCore(CashPayments|Cost)Live$",
                "-v",
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
    if std::env::var_os("ACQUISITION_TEST_DATABASE_URL").is_some() {
        assert!(
            String::from_utf8_lossy(&output.stdout).contains("--- PASS: TestCoreCostLive"),
            "Core-backed cost integration must execute: {}",
            String::from_utf8_lossy(&output.stdout)
        );
        if std::env::var_os("ACQUISITION_FUNNEL_COMMUNITY_DATABASE_URL").is_some() {
            assert!(
                String::from_utf8_lossy(&output.stdout)
                    .contains("--- PASS: TestCoreCostLive/funnel"),
                "Core-backed funnel integration must execute: {}",
                String::from_utf8_lossy(&output.stdout)
            );
            assert!(
                String::from_utf8_lossy(&output.stdout)
                    .contains("--- PASS: TestCoreCostLive/report"),
                "Core-backed report integration must execute: {}",
                String::from_utf8_lossy(&output.stdout)
            );
        }
    }
}

#[sqlx::test(migrations = false)]
async fn repeat_cash_merges_historical_and_native_orders(pool: PgPool) {
    mixed_cash_history(pool, false).await;
}

#[sqlx::test(migrations = false)]
async fn prefence_native_cash_invalidates_historical_coverage(pool: PgPool) {
    mixed_cash_history(pool, true).await;
}

async fn mixed_cash_history(pool: PgPool, invalid_seal: bool) {
    core_schema::init(&pool).await.unwrap();
    let identity = IdentityStore::from_pool(pool.clone());
    let key = identity.bootstrap_user(101, 0).await.unwrap();
    let owner = identity
        .authorize(&key.secret)
        .await
        .unwrap()
        .owner_account_id;
    let base = json!({"migration_id":"mixed-cash-opening","source_entity_key":"user:101",
      "source_archive_sha256":"a".repeat(64),"mapping_sha256":"b".repeat(64),"source_row_sha256":"c".repeat(64),"observed_at":1793674367_i64,"account_id":owner});
    let mut opening = base.clone();
    for field in ["available_units", "reserved_units", "affiliate_units"] {
        opening[field] = json!(0);
    }
    let mut counters = base;
    counters["user_id"] = json!(101);
    for field in ["used_quota", "request_count", "aff_history"] {
        counters[field] = json!(0);
    }
    let mut tx = pool.begin().await.unwrap();
    sqlx::query("SET TRANSACTION ISOLATION LEVEL SERIALIZABLE")
        .execute(&mut *tx)
        .await
        .unwrap();
    sqlx::query("SET LOCAL synchronous_commit=on")
        .execute(&mut *tx)
        .await
        .unwrap();
    sqlx::query("SELECT core_billing.import_opening_balance($1)")
        .bind(opening)
        .execute(&mut *tx)
        .await
        .unwrap();
    sqlx::query("SELECT core_billing.import_account_opening_counters($1)")
        .bind(counters)
        .execute(&mut *tx)
        .await
        .unwrap();
    tx.commit().await.unwrap();

    let payments = PaymentsStore::new(
        pool.clone(),
        vec![Channel::stripe_live(
            1,
            MERCHANT.into(),
            SECRET.into(),
            100_000,
        )],
    )
    .unwrap();
    if !invalid_seal {
        sqlx::query("INSERT INTO core_history.finance_sources(source_key,contract,source_archive_sha256,mapping_sha256,tables_sha256,normalization_sha256,rows_sha256,observed_at,user_count,topup_count,credits_per_usd,complete,native_request_ids) VALUES(repeat('d',64),'core.site-finance-source.v1',repeat('a',64),repeat('b',64),repeat('c',64),repeat('e',64),repeat('f',64),1793674367,1,1,500000,true,'{}')").execute(&pool).await.unwrap();
        sqlx::query("INSERT INTO core_history.finance_users(user_id,account_id,source_key,source_row_sha256,raw_used_quota,normalized_used_credits) VALUES(101,$1,repeat('d',64),repeat('c',64),0,0)").bind(owner).execute(&pool).await.unwrap();
        sqlx::query("INSERT INTO core_history.finance_topups(source_key,source_order_id,source_row_sha256,classification,currency,gross_amount_micros,refunded_amount_micros) VALUES(repeat('d',64),1,repeat('c',64),'cash','USD',1000000,0)").execute(&pool).await.unwrap();
        let manifest = json!({"scope":"wallet-topup-history.v1","source_archive_sha256":"a".repeat(64),"complete":true,"source_row_count":1,
      "rows":[{"target_user_id":101,"target_personal_account_id":owner,"record":{"id":1,"user_id":101,"complete_time":1700000000_i64,"status":"success"}}],
      "identity_map":[{"source_user_id":101,"target_user_id":101,"target_personal_account_id":owner}],"per_source_user_counts":{"101":1}});
        sqlx::query("INSERT INTO core_migration.import_runs(migration_id,source_archive_sha256,mapping_sha256,manifest) VALUES('cash-history',repeat('a',64),repeat('b',64),$1)").bind(manifest).execute(&pool).await.unwrap();
    }
    for index in 0..2 {
        let intent = payments
            .prepare(
                &key.secret,
                pb::PrepareTopupRequest {
                    mutation: Some(pb::MutationContext {
                        idempotency_key: format!("mixed_cash_prepare_{index}"),
                        expected_version: None,
                    }),
                    account_id: owner,
                    payment_channel_id: 1,
                    currency: "USD".into(),
                    amount_minor: 137,
                },
            )
            .await
            .unwrap();
        let mut proof = evidence(&intent);
        let accepted = payments
            .accept_proof(pb::AcceptProviderProofRequest {
                intent_id: intent.intent_id,
                evidence: Some(proof.clone()),
            })
            .await
            .unwrap();
        proof.accepted_proof_id = accepted.proof_id;
        payments
            .credit(pb::CreditTopupRequest {
                mutation: Some(pb::MutationContext {
                    idempotency_key: format!("mixed_cash_credit_{index}"),
                    expected_version: None,
                }),
                intent_id: intent.intent_id,
                evidence: Some(proof),
            })
            .await
            .unwrap();
        if index == 0 && invalid_seal {
            sqlx::query("INSERT INTO core_history.finance_sources(source_key,contract,source_archive_sha256,mapping_sha256,tables_sha256,normalization_sha256,rows_sha256,observed_at,user_count,topup_count,credits_per_usd,complete,native_request_ids) VALUES(repeat('d',64),'core.site-finance-source.v1',repeat('a',64),repeat('b',64),repeat('c',64),repeat('e',64),repeat('f',64),1793674367,1,1,500000,true,'{}')").execute(&pool).await.unwrap();
            sqlx::query("INSERT INTO core_history.finance_users(user_id,account_id,source_key,source_row_sha256,raw_used_quota,normalized_used_credits) VALUES(101,$1,repeat('d',64),repeat('c',64),0,0)").bind(owner).execute(&pool).await.unwrap();
            sqlx::query("INSERT INTO core_history.finance_topups(source_key,source_order_id,source_row_sha256,classification,currency,gross_amount_micros,refunded_amount_micros) VALUES(repeat('d',64),1,repeat('c',64),'cash','USD',1000000,0)").execute(&pool).await.unwrap();
            let manifest = json!({"scope":"wallet-topup-history.v1","source_archive_sha256":"a".repeat(64),"complete":true,"source_row_count":1,
      "rows":[{"target_user_id":101,"target_personal_account_id":owner,"record":{"id":1,"user_id":101,"complete_time":1700000000_i64,"status":"success"}}],
      "identity_map":[{"source_user_id":101,"target_user_id":101,"target_personal_account_id":owner}],"per_source_user_counts":{"101":1}});
            sqlx::query("INSERT INTO core_migration.import_runs(migration_id,source_archive_sha256,mapping_sha256,manifest) VALUES('cash-history',repeat('a',64),repeat('b',64),$1)").bind(manifest).execute(&pool).await.unwrap();
        }
    }
    let read = CommunityRead::new(pool).await.unwrap();
    let request = UsersInfoRequest {
        user_ids: vec![101],
    };
    let native = read.native_first_cash_payments(&request).await.unwrap();
    let combined = read.first_cash_payments(&request).await.unwrap();
    if invalid_seal {
        assert!(!combined.users[0].history_complete);
        assert!(
            combined.users[0].first_paid_at.is_none() && combined.users[0].second_paid_at.is_none(),
            "an invalid native seal is not silently excluded"
        );
    } else {
        assert_eq!(combined.users[0].first_paid_at, Some(1700000000));
        assert_eq!(
            combined.users[0].second_paid_at,
            native.users[0].first_paid_at
        );
    }
}

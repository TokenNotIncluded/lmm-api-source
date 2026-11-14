//! Real PostgreSQL payment acceptance with synthetic endpoint signatures only.
//! SQLx creates a new owned database per test; never point DATABASE_URL at live data.
#![cfg(unix)]
use hmac::KeyInit as _;
use hmac::{Hmac, Mac};
use lmm_core::{
    billing::{
        Credits,
        ledger::{self, Action, Ledger, Outcome},
    },
    identity::IdentityStore,
    internal_rpc::pb,
    payments::{Channel, Error, PaymentsStore},
};
use serde_json::{Value, json};
use sha2::Sha256;
use sqlx::PgPool;
use std::time::{SystemTime, UNIX_EPOCH};

const SECRET: &str = "whsec_local_synthetic_endpoint_only";
const MERCHANT: &str = "acct_local_synthetic";
struct Fixture {
    store: IngressStore,
    ledger: Ledger,
    identity: IdentityStore,
    secret: String,
    account: i64,
}
#[derive(Clone)]
struct IngressStore(PaymentsStore);
impl std::ops::Deref for IngressStore {
    type Target = PaymentsStore;
    fn deref(&self) -> &Self::Target {
        &self.0
    }
}
impl IngressStore {
    async fn receive(
        &self,
        intent_id: i64,
        evidence: &mut Option<pb::ProviderEvidence>,
    ) -> Result<(), Error> {
        let evidence = evidence.as_mut().ok_or(Error::Invalid)?;
        if evidence.accepted_proof_id.is_empty() {
            let receipt = self
                .0
                .accept_proof(pb::AcceptProviderProofRequest {
                    intent_id,
                    evidence: Some(evidence.clone()),
                })
                .await?;
            evidence.accepted_proof_id = receipt.proof_id;
        }
        Ok(())
    }
    async fn credit(
        &self,
        mut request: pb::CreditTopupRequest,
    ) -> Result<pb::PaymentReceipt, Error> {
        self.receive(request.intent_id, &mut request.evidence)
            .await?;
        self.0.credit(request).await
    }
    async fn external_refund(
        &self,
        mut request: pb::ExternalRefundRequest,
    ) -> Result<pb::PaymentReceipt, Error> {
        self.receive(request.intent_id, &mut request.evidence)
            .await?;
        self.0.external_refund(request).await
    }
}
async fn fresh(pool: &PgPool) -> Fixture {
    let identity = IdentityStore::from_pool(pool.clone());
    lmm_core::core_schema::init(pool).await.unwrap();
    let issued = identity.bootstrap_user(101, 0).await.unwrap();
    let account = identity
        .authorize(&issued.secret)
        .await
        .unwrap()
        .owner_account_id;
    Fixture {
        store: IngressStore(
            PaymentsStore::new(
                pool.clone(),
                vec![Channel::stripe_test(
                    1,
                    MERCHANT.into(),
                    SECRET.into(),
                    100_000,
                )],
            )
            .unwrap(),
        ),
        ledger: Ledger::from_pool(pool.clone()),
        identity,
        secret: issued.secret,
        account,
    }
}
fn mutation(key: &str) -> Option<pb::MutationContext> {
    Some(pb::MutationContext {
        idempotency_key: key.into(),
        expected_version: None,
    })
}
fn prepare_request(f: &Fixture, key: &str, amount: i64) -> pb::PrepareTopupRequest {
    pb::PrepareTopupRequest {
        requested_credit_units: 0,
        mutation: mutation(key),
        account_id: f.account,
        payment_channel_id: 1,
        currency: "USD".into(),
        amount_minor: amount,
    }
}
async fn prepare(f: &Fixture, key: &str, amount: i64) -> pb::PaymentIntent {
    f.store
        .prepare(&f.secret, prepare_request(f, key, amount))
        .await
        .unwrap()
}
fn event(
    intent: &pb::PaymentIntent,
    id: &str,
    refund: Option<(&str, i64, &str, Option<&str>)>,
) -> Value {
    let mut metadata =
        json!({"lmm_core_intent_id":intent.intent_id.to_string(),"lmm_merchant":MERCHANT});
    let object = if let Some((refund_id, amount, status, hold)) = refund {
        if let Some(hold) = hold {
            metadata["lmm_hold_receipt_id"] = json!(hold);
        }
        json!({"id":refund_id,"object":"refund","status":status,"amount":amount,"currency":"usd","payment_intent":format!("pi_{}",intent.intent_id),"metadata":metadata})
    } else {
        json!({"id":format!("pi_{}",intent.intent_id),"object":"payment_intent","status":"succeeded","amount":intent.amount_minor,"amount_received":intent.amount_minor,"currency":"usd","livemode":false,"metadata":metadata})
    };
    json!({"id":id,"type":if refund.is_some(){"refund.updated"}else{"payment_intent.succeeded"},"livemode":false,"data":{"object":object}})
}
fn proof(value: Value) -> pb::ProviderEvidence {
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_secs();
    let payload = serde_json::to_vec(&value).unwrap();
    let mut mac = Hmac::<Sha256>::new_from_slice(SECRET.as_bytes()).unwrap();
    mac.update(format!("{now}.").as_bytes());
    mac.update(&payload);
    let mut signature = format!("t={now},v1=");
    use std::fmt::Write;
    for byte in mac.finalize().into_bytes() {
        write!(&mut signature, "{byte:02x}").unwrap();
    }
    let object = &value["data"]["object"];
    let transaction = if object["object"] == "refund" {
        object["payment_intent"].as_str().unwrap()
    } else {
        object["id"].as_str().unwrap()
    };
    pb::ProviderEvidence {
        payment: Some(pb::ExternalPaymentReference {
            provider: "stripe".into(),
            merchant: MERCHANT.into(),
            environment: "test".into(),
            transaction: transaction.into(),
        }),
        signed_payload: payload,
        signature: signature.into_bytes(),
        provider_event_id: value["id"].as_str().unwrap().into(),
        accepted_proof_id: String::new(),
    }
}
fn credit_request(
    intent: &pb::PaymentIntent,
    key: &str,
    evidence: pb::ProviderEvidence,
) -> pb::CreditTopupRequest {
    pb::CreditTopupRequest {
        mutation: mutation(key),
        intent_id: intent.intent_id,
        evidence: Some(evidence),
    }
}
async fn credit(f: &Fixture, intent: &pb::PaymentIntent, key: &str) -> pb::PaymentReceipt {
    f.store
        .credit(credit_request(
            intent,
            key,
            proof(event(
                intent,
                &format!("evt_credit_{}", intent.intent_id),
                None,
            )),
        ))
        .await
        .unwrap()
}

async fn referral(pool: &PgPool) -> (Fixture, i64) {
    let f = fresh(pool).await;
    let inviter = f.identity.bootstrap_user(102, 0).await.unwrap();
    let inviter_account = f
        .identity
        .authorize(&inviter.secret)
        .await
        .unwrap()
        .owner_account_id;
    f.identity.bootstrap_user(103, 0).await.unwrap();
    sqlx::raw_sql("UPDATE core_identity.users SET platform_role='admin' WHERE id=103;
      INSERT INTO core_console.options(key,value) VALUES('payment_setting.compliance_confirmed','true'),('payment_setting.compliance_terms_version','v1') ON CONFLICT(key) DO UPDATE SET value=EXCLUDED.value;")
        .execute(pool).await.unwrap();
    let mut opening = pool.begin().await.unwrap();
    sqlx::query("SET TRANSACTION ISOLATION LEVEL SERIALIZABLE")
        .execute(&mut *opening)
        .await
        .unwrap();
    for (user, account) in [(101, f.account), (102, inviter_account)] {
        sqlx::query("SELECT core_billing.import_opening_balance($1)")
            .bind(
                json!({"migration_id":"synthetic-referral","source_archive_sha256":"a".repeat(64),
                "mapping_sha256":"b".repeat(64),"source_row_sha256":"c".repeat(64),
                "source_entity_key":format!("user:{user}"),"account_id":account,"observed_at":1,
                "available_units":0,"reserved_units":0,"affiliate_units":0}),
            )
            .execute(&mut *opening)
            .await
            .unwrap();
    }
    opening.commit().await.unwrap();
    let fixture =
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/legacy_referrals_fixture.py");
    let plan = std::process::Command::new("python3")
        .arg(fixture)
        .output()
        .unwrap();
    assert!(
        plan.status.success(),
        "synthetic importer failed: {}",
        String::from_utf8_lossy(&plan.stderr)
    );
    sqlx::raw_sql(sqlx::AssertSqlSafe(
        std::str::from_utf8(&plan.stdout).unwrap(),
    ))
    .execute(pool)
    .await
    .unwrap();
    // Raw registration grant is retained for history, never posted a second time.
    assert_eq!(
        f.ledger
            .balance(inviter_account)
            .await
            .unwrap()
            .affiliate_units,
        0
    );
    (f, inviter_account)
}
async fn affiliate(f: &Fixture, account: i64) -> i64 {
    f.ledger.balance(account).await.unwrap().affiliate_units
}
async fn moderation(pool: &PgPool, key: &str, restore: bool) {
    use std::os::unix::fs::PermissionsExt;
    let directory = std::env::temp_dir().join(format!(
        "lmm-referral-moderation-{}-{:016x}",
        std::process::id(),
        rand::random::<u64>()
    ));
    std::fs::create_dir(&directory).unwrap();
    std::fs::set_permissions(&directory, std::fs::Permissions::from_mode(0o700)).unwrap();
    let input = directory.join("request.json");
    let output = directory.join("preview.json");
    std::fs::write(&input,serde_json::to_vec(&json!({"request_id":key,"actor":103,"user_id":101,
        "action":if restore {"restore_referral"} else {"ban_abuse"},
        "reason":if restore {"mistaken_ban"} else {"abuse"},"evidence":"synthetic confirmed decision","penalize":!restore})).unwrap()).unwrap();
    let options = pool.connect_options();
    let socket = options.get_socket().map_or_else(
        || std::path::Path::new(options.get_host()),
        std::path::PathBuf::as_path,
    );
    let owner: String = sqlx::query_scalar("SELECT current_user")
        .fetch_one(pool)
        .await
        .unwrap();
    let script = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../scripts/legacy_referral_moderation.py");
    let run = |mode: &str, source: &std::path::Path, confirmation: Option<&str>| {
        let mut cmd = std::process::Command::new("python3");
        cmd.arg(&script)
            .arg(mode)
            .arg("--socket")
            .arg(socket)
            .arg("--port")
            .arg(options.get_port().to_string())
            .arg("--database")
            .arg(options.get_database().unwrap())
            .arg("--login")
            .arg(options.get_username())
            .arg("--owner")
            .arg(&owner)
            .arg("--input")
            .arg(source);
        if let Some(hash) = confirmation {
            cmd.arg("--confirmation-sha256").arg(hash);
        } else {
            cmd.arg("--output").arg(&output);
        }
        cmd.output().unwrap()
    };
    let planned = run("plan", &input, None);
    assert!(
        planned.status.success(),
        "{}",
        String::from_utf8_lossy(&planned.stderr)
    );
    let preview: Value = serde_json::from_slice(&std::fs::read(&output).unwrap()).unwrap();
    let applied = run("apply", &output, preview["confirmation_sha256"].as_str());
    assert!(
        applied.status.success(),
        "{}",
        String::from_utf8_lossy(&applied.stderr)
    );
    std::fs::remove_dir_all(directory).unwrap();
}
async fn refund(f: &Fixture, intent: &pb::PaymentIntent, key: &str, amount: i64) {
    let refund_id = format!("re_{key}");
    let evidence = proof(event(
        intent,
        &format!("evt_{key}"),
        Some((&refund_id, amount, "succeeded", None)),
    ));
    f.store
        .external_refund(pb::ExternalRefundRequest {
            mutation: mutation(key),
            intent_id: intent.intent_id,
            evidence: Some(evidence),
        })
        .await
        .unwrap();
}
#[sqlx::test]
async fn payment_tail_claim_debt_ban_refund_restore_and_replay(pool: PgPool) {
    let (f, account) = referral(&pool).await;
    let intent = prepare(&f, "referral-prepare-0001", 1000).await;
    credit(&f, &intent, "referral-credit-00001").await;
    assert_eq!(affiliate(&f, account).await, 1000000); // no registration reissue
    credit(&f, &intent, "referral-credit-00001").await;
    f.store
        .credit(credit_request(
            &intent,
            "referral-credit-00002",
            proof(event(&intent, "evt_second_delivery", None)),
        ))
        .await
        .unwrap();
    assert_eq!(affiliate(&f, account).await, 1000000);
    let posted = f
        .ledger
        .execute(&ledger::Request {
            scope: "synthetic-claim".into(),
            operation_key: "claim-1".into(),
            actor_user_id: Some(102),
            reason: "synthetic claim".into(),
            action: Action::ClaimAffiliate {
                account_id: account,
                amount_units: Credits::try_from(1000000).unwrap(),
            },
        })
        .await
        .unwrap();
    assert!(matches!(posted, Outcome::Posted { .. }));
    // Ordinary disable/enable never touches money.
    sqlx::query("UPDATE core_identity.users SET active=NOT active WHERE id=101")
        .execute(&pool)
        .await
        .unwrap();
    assert_eq!(affiliate(&f, account).await, 0);
    sqlx::query("UPDATE core_identity.users SET active=NOT active WHERE id=101")
        .execute(&pool)
        .await
        .unwrap();
    moderation(&pool, "ban-1", false).await;
    moderation(&pool, "ban-1", false).await;
    assert_eq!(affiliate(&f, account).await, -1320000); // full contractual quota plus penalty
    refund(&f, &intent, "refund-half-00000001", 500).await;
    assert_eq!(affiliate(&f, account).await, -1320000);
    refund(&f, &intent, "refund-half-00000002", 500).await;
    moderation(&pool, "restore-1", true).await;
    moderation(&pool, "restore-1", true).await;
    assert_eq!(affiliate(&f, account).await, -1100000); // refund prevents award resurrection
    moderation(&pool, "ban-1", false).await; // old ban cannot revoke after appeal
    assert_eq!(affiliate(&f, account).await, -1100000);
    let conflict = sqlx::query("SELECT core_referrals.moderate('ban-1',103,101,'ban_abuse','abuse','changed evidence',true)").execute(&pool).await;
    assert!(conflict.is_err());
    let net: i64 = sqlx::query_scalar("SELECT count(*) FROM (SELECT journal_id FROM core_billing.ledger_entries GROUP BY journal_id HAVING sum(delta_units)<>0) t").fetch_one(&pool).await.unwrap();
    assert_eq!(net, 0);
    lmm_core::core_schema::check(&pool).await.unwrap();
}
#[sqlx::test]
async fn low_first_payment_cannot_reopen_on_later_cash(pool: PgPool) {
    let (f, account) = referral(&pool).await;
    let first = prepare(&f, "referral-low-first-001", 999).await;
    credit(&f, &first, "referral-low-credit-01").await;
    let second = prepare(&f, "referral-second-00001", 2000).await;
    credit(&f, &second, "referral-second-credit").await;
    assert_eq!(affiliate(&f, account).await, 0);
    let stage: String = sqlx::query_scalar("SELECT stage FROM core_referrals.contracts WHERE id=1")
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(stage, "first_top_up_ineligible");
}
#[sqlx::test]
async fn award_failure_rolls_back_payer_credit_and_marker(pool: PgPool) {
    let (f, account) = referral(&pool).await;
    f.ledger.open_wallet(f.account).await.unwrap();
    let intent = prepare(&f, "referral-rollback-001", 1000).await;
    sqlx::raw_sql("CREATE FUNCTION core_referrals.synthetic_fail() RETURNS trigger LANGUAGE plpgsql AS $$ BEGIN RAISE EXCEPTION 'synthetic ledger failure'; END $$; CREATE TRIGGER synthetic_failure BEFORE INSERT ON core_referrals.events FOR EACH ROW EXECUTE FUNCTION core_referrals.synthetic_fail();").execute(&pool).await.unwrap();
    let result = f
        .store
        .credit(credit_request(
            &intent,
            "referral-fail-credit01",
            proof(event(&intent, "evt_rollback", None)),
        ))
        .await;
    assert!(result.is_err());
    assert_eq!(affiliate(&f, account).await, 0);
    assert_eq!(
        f.ledger.balance(f.account).await.unwrap().available_units,
        0
    );
    let source: Option<String> =
        sqlx::query_scalar("SELECT first_source FROM core_referrals.contracts WHERE id=1")
            .fetch_one(&pool)
            .await
            .unwrap();
    assert!(source.is_none());
    sqlx::raw_sql("DROP TRIGGER synthetic_failure ON core_referrals.events; DROP FUNCTION core_referrals.synthetic_fail();").execute(&pool).await.unwrap();
    credit(&f, &intent, "referral-fail-credit01").await;
    assert_eq!(affiliate(&f, account).await, 1000000);
}

#[sqlx::test]
async fn max_cap_snapshot_and_pair_email_eligibility_use_real_payments(pool: PgPool) {
    let (f, account) = referral(&pool).await;
    sqlx::query("UPDATE core_referrals.contracts SET max_reward_credit=350000 WHERE id=1")
        .execute(&pool)
        .await
        .unwrap();
    let intent = prepare(&f, "referral-cap-prepare01", 1000).await;
    credit(&f, &intent, "referral-cap-credit001").await;
    assert_eq!(affiliate(&f, account).await, 250000);
    // A separate imported pending contract with the same inviter and email.
    let issued = f.identity.bootstrap_user(104, 0).await.unwrap();
    let a = f
        .identity
        .authorize(&issued.secret)
        .await
        .unwrap()
        .owner_account_id;
    sqlx::raw_sql("INSERT INTO core_identity.user_profiles(user_id,username,email) VALUES(102,'syntheticinviter','same@example.test'),(104,'syntheticinvitee','same@example.test') ON CONFLICT(user_id) DO UPDATE SET email=EXCLUDED.email;
     INSERT INTO core_referrals.bindings VALUES(104,102,0,'',false,repeat('a',64),104);
     INSERT INTO core_referrals.contracts(id,invitee,inviter,policy,registration_credit,reward_credit,max_reward_credit,min_top_up_credit,max_penalty_credit,penalty_percent,stage,status,reason,quota,archive_sha256)
     SELECT 2,104,inviter,policy,registration_credit,reward_credit,0,0,0,20,'awaiting_first_top_up','earned','',100000,archive_sha256 FROM core_referrals.contracts WHERE id=1;").execute(&pool).await.unwrap();
    let other = Fixture {
        store: f.store.clone(),
        ledger: f.ledger.clone(),
        identity: f.identity.clone(),
        secret: issued.secret,
        account: a,
    };
    let intent = prepare(&other, "referral-email-prepare", 1000).await;
    credit(&other, &intent, "referral-email-credit1").await;
    assert_eq!(affiliate(&f, account).await, 250000);
    let stage: String = sqlx::query_scalar("SELECT stage FROM core_referrals.contracts WHERE id=2")
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(stage, "first_top_up_ineligible");
}

#[sqlx::test]
async fn concurrent_retry_and_reconstructed_store_never_duplicate_tail(pool: PgPool) {
    let (f, account) = referral(&pool).await;
    let intent = prepare(&f, "referral-race-prepare1", 1000).await;
    let mut evidence = proof(event(&intent, "evt_race", None));
    let accepted = f
        .store
        .accept_proof(pb::AcceptProviderProofRequest {
            intent_id: intent.intent_id,
            evidence: Some(evidence.clone()),
        })
        .await
        .unwrap();
    evidence.accepted_proof_id = accepted.proof_id;
    let req = credit_request(&intent, "referral-race-credit01", evidence);
    let (a, b) = tokio::join!(f.store.0.credit(req.clone()), f.store.0.credit(req.clone()));
    for result in [a, b] {
        if let Err(e) = result {
            assert_eq!(e, Error::Unavailable);
            f.store.0.credit(req.clone()).await.unwrap();
        }
    }
    let recovered = PaymentsStore::new(
        pool.clone(),
        vec![Channel::stripe_test(
            1,
            MERCHANT.into(),
            SECRET.into(),
            100000,
        )],
    )
    .unwrap();
    recovered.credit(req).await.unwrap();
    assert_eq!(affiliate(&f, account).await, 1000000);
    let awards: i64 =
        sqlx::query_scalar("SELECT count(*) FROM core_referrals.events WHERE kind='first_top_up'")
            .fetch_one(&pool)
            .await
            .unwrap();
    assert_eq!(awards, 1);
}

#[sqlx::test]
async fn full_refund_after_claim_leaves_debt_and_future_award_offsets_it(pool: PgPool) {
    let (f, account) = referral(&pool).await;
    let intent = prepare(&f, "referral-debt-prepare1", 1000).await;
    credit(&f, &intent, "referral-debt-credit01").await;
    f.ledger
        .execute(&ledger::Request {
            scope: "synthetic-claim".into(),
            operation_key: "debt-claim-1".into(),
            actor_user_id: Some(102),
            reason: "synthetic claim".into(),
            action: Action::ClaimAffiliate {
                account_id: account,
                amount_units: Credits::try_from(1000000).unwrap(),
            },
        })
        .await
        .unwrap();
    refund(&f, &intent, "referral-debt-refund01", 1000).await;
    assert_eq!(affiliate(&f, account).await, -1100000);
    let issued = f.identity.bootstrap_user(104, 0).await.unwrap();
    let a = f
        .identity
        .authorize(&issued.secret)
        .await
        .unwrap()
        .owner_account_id;
    sqlx::raw_sql("INSERT INTO core_referrals.bindings VALUES(104,102,0,'',false,repeat('a',64),104);
     INSERT INTO core_referrals.contracts(id,invitee,inviter,policy,registration_credit,reward_credit,max_reward_credit,min_top_up_credit,max_penalty_credit,penalty_percent,stage,status,reason,quota,archive_sha256)
     SELECT 2,104,inviter,policy,0,reward_credit,0,0,0,20,'awaiting_first_top_up','earned','',0,archive_sha256 FROM core_referrals.contracts WHERE id=1;").execute(&pool).await.unwrap();
    let other = Fixture {
        store: f.store.clone(),
        ledger: f.ledger.clone(),
        identity: f.identity.clone(),
        secret: issued.secret,
        account: a,
    };
    let intent = prepare(&other, "referral-offset-prepare", 1000).await;
    credit(&other, &intent, "referral-offset-credit").await;
    assert_eq!(affiliate(&f, account).await, -100000);
    let rejected = f
        .ledger
        .execute(&ledger::Request {
            scope: "synthetic-claim".into(),
            operation_key: "debt-claim-2".into(),
            actor_user_id: Some(102),
            reason: "synthetic debt claim rejected".into(),
            action: Action::ClaimAffiliate {
                account_id: account,
                amount_units: Credits::try_from(500000).unwrap(),
            },
        })
        .await
        .unwrap();
    assert!(matches!(rejected, Outcome::Rejected { .. }));
}

#[path = "common/runtime.rs"]
mod runtime;

#[sqlx::test]
async fn restricted_runtime_credits_via_core_without_referral_post_authority(pool: PgPool) {
    let (f, account) = referral(&pool).await;
    let intent = prepare(&f, "referral-runtime-prepare", 1000).await;
    let role = runtime::RuntimeRole::create(&pool).await;
    role.optional("core_payments").await;
    let forbidden:bool=sqlx::query_scalar("SELECT NOT has_function_privilege($1,'core_referrals.delta(bigint,text,bigint)','EXECUTE') AND NOT has_function_privilege($1,'core_referrals.settle(bigint,text,text,text,bigint,bigint)','EXECUTE') AND NOT has_function_privilege($1,'core_referrals.moderate(text,bigint,bigint,text,text,text,boolean)','EXECUTE') AND NOT has_table_privilege($1,'core_referrals.contracts','INSERT,UPDATE,DELETE,TRUNCATE') AND NOT has_table_privilege($1,'core_billing.ledger_entries','INSERT,UPDATE,DELETE,TRUNCATE')").bind(&role.name).fetch_one(&pool).await.unwrap();
    assert!(forbidden);
    {
        let store = IngressStore(
            PaymentsStore::new(
                role.pool.clone(),
                vec![Channel::stripe_test(
                    1,
                    MERCHANT.into(),
                    SECRET.into(),
                    100000,
                )],
            )
            .unwrap(),
        );
        store
            .accept_proof(pb::AcceptProviderProofRequest {
                intent_id: intent.intent_id,
                evidence: Some(proof(event(&intent, "evt_runtime_credit", None))),
            })
            .await
            .unwrap();
        let gift = f
            .ledger
            .execute(&ledger::Request {
                scope: "synthetic-internal".into(),
                operation_key: "not-a-payment".into(),
                actor_user_id: Some(101),
                reason: "synthetic internal gift is not cash proof".into(),
                action: Action::Gift {
                    account_id: f.account,
                    amount_units: Credits::try_from(1).unwrap(),
                },
            })
            .await
            .unwrap();
        let Outcome::Posted { journal_id, .. } = gift else {
            panic!("synthetic gift failed")
        };
        let forged =
            sqlx::query("UPDATE core_payments.intents SET credit_journal_id=$2 WHERE id=$1")
                .bind(intent.intent_id)
                .bind(journal_id)
                .execute(&role.pool)
                .await
                .unwrap_err();
        assert_eq!(
            forged.as_database_error().unwrap().message(),
            "referral payment lacks matching Core proof and credit"
        );
        store
            .credit(credit_request(
                &intent,
                "referral-runtime-credit",
                proof(event(&intent, "evt_runtime_credit", None)),
            ))
            .await
            .unwrap();
    }
    assert_eq!(affiliate(&f, account).await, 1000000);
    let forged=sqlx::query("UPDATE core_payments.intents SET refunded_minor=amount_minor,refund_fenced=true WHERE id=$1").bind(intent.intent_id).execute(&role.pool).await.unwrap_err();
    assert_eq!(
        forged.as_database_error().unwrap().message(),
        "referral refund lacks original ledger compensation"
    );
    assert_eq!(affiliate(&f, account).await, 1000000);
    role.cleanup().await;
}

#[sqlx::test]
#[ignore = "requires explicitly supplied private frozen source and reviewed import plan"]
async fn frozen_source_import_then_trusted_payment_and_refund(pool: PgPool) {
    use std::collections::BTreeMap;
    let source_path = std::env::var("LMM_REFERRAL_SOURCE_FILE").expect("private source required");
    let plan_path =
        std::env::var("LMM_REFERRAL_IMPORT_PLAN").expect("reviewed private plan required");
    let raw = std::fs::read(source_path).unwrap();
    use sha2::Digest;
    assert_eq!(
        hex_codec::encode(Sha256::digest(&raw)),
        "fc7112f6f25121cb0894b556b9d6bc5ee0f1a502f2d799f93c01d5c32dd9e09a"
    );
    let doc: Value = serde_json::from_slice(&raw).unwrap();
    let plan = std::fs::read_to_string(plan_path).unwrap();
    assert_eq!(
        hex_codec::encode(Sha256::digest(plan.as_bytes())),
        "9d6ae3672b76be944960babbc3b458dcc12db7b152af8189be4a62ed7ce79e19"
    );
    lmm_core::core_schema::init(&pool).await.unwrap();
    let identity = IdentityStore::from_pool(pool.clone());
    let mut accounts = BTreeMap::new();
    let mut secrets = BTreeMap::new();
    for user in doc["users"].as_array().unwrap() {
        let id = user["id"].as_i64().unwrap();
        let issued = identity.bootstrap_user(id, 0).await.unwrap();
        let account = identity
            .authorize(&issued.secret)
            .await
            .unwrap()
            .owner_account_id;
        accounts.insert(id, account);
        secrets.insert(id, issued.secret);
    }
    let mut tx = pool.begin().await.unwrap();
    sqlx::query("SET TRANSACTION ISOLATION LEVEL SERIALIZABLE")
        .execute(&mut *tx)
        .await
        .unwrap();
    let mut affiliate_sum = 0_i128;
    for user in doc["users"].as_array().unwrap() {
        let id = user["id"].as_i64().unwrap();
        let affiliate = user["aff_quota"].as_i64().unwrap();
        affiliate_sum += i128::from(affiliate);
        sqlx::query("SELECT core_billing.import_opening_balance($1)")
            .bind(json!({"migration_id":"referral-native-20261104","source_archive_sha256":doc["source_archive_sha256"],
                "mapping_sha256":"b".repeat(64),"source_row_sha256":hex_codec::encode(Sha256::digest(serde_json::to_vec(user).unwrap())),
                "source_entity_key":format!("user:{id}"),"account_id":accounts[&id],"observed_at":doc["observed_at"],
                "available_units":0,"reserved_units":0,"affiliate_units":affiliate}))
            .execute(&mut *tx).await.unwrap_or_else(|_|panic!("private source opening refused; inspect private PG log"));
        let role = match user["role"].as_i64().unwrap() {
            1 => "user",
            10 => "admin",
            100 => "superadmin",
            _ => panic!("unknown source role"),
        };
        sqlx::query("UPDATE core_identity.users SET platform_role=$2,active=$3 WHERE id=$1")
            .bind(id)
            .bind(role)
            .bind(user["status"].as_i64() == Some(1) && user["deleted_at"].is_null())
            .execute(&mut *tx)
            .await
            .unwrap();
        sqlx::query("INSERT INTO core_identity.user_profiles(user_id,username,email) VALUES($1,$2,$3) ON CONFLICT(user_id) DO UPDATE SET email=EXCLUDED.email")
            .bind(id).bind(format!("synthetic_source_{id}")).bind(user["email"].as_str().unwrap()).execute(&mut *tx).await.unwrap();
    }
    tx.commit().await.unwrap();
    sqlx::raw_sql(sqlx::AssertSqlSafe::<&str>(&plan))
        .execute(&pool)
        .await
        .unwrap_or_else(|_| panic!("private source import refused; inspect private PG log"));
    let counts:(i64,i64,i64,i64,i64,i64)=sqlx::query_as("SELECT (SELECT count(*) FROM core_referrals.bindings),(SELECT count(*) FROM core_referrals.contracts),jsonb_array_length(document->'users')::bigint,jsonb_array_length(document->'rewards')::bigint,jsonb_array_length(document->'ledger_entries')::bigint,(SELECT count(*) FROM core_referrals.events) FROM core_referrals.source").fetch_one(&pool).await.unwrap();
    assert_eq!(counts, (1239, 133, 1239, 269, 472, 0));
    let actual:String=sqlx::query_scalar("SELECT sum(b.balance_units)::text FROM core_billing.balance_state b JOIN core_billing.ledger_accounts a ON a.id=b.ledger_account_id WHERE a.bucket='affiliate'").fetch_one(&pool).await.unwrap();
    assert_eq!(actual.parse::<i128>().unwrap(), affiliate_sum);
    // Restore only the source's published compliance facts in this isolated shadow.
    for key in [
        "payment_setting.compliance_confirmed",
        "payment_setting.compliance_terms_version",
    ] {
        sqlx::query("INSERT INTO core_console.options(key,value) VALUES($1,$2) ON CONFLICT(key) DO UPDATE SET value=EXCLUDED.value")
            .bind(key).bind(doc["options"][key].as_str().unwrap()).execute(&pool).await.unwrap();
    }
    let candidate:(i64,i64,i64,i64,i64)=sqlx::query_as("SELECT c.invitee,c.inviter,c.reward_credit,c.registration_credit,c.id FROM core_referrals.contracts c JOIN core_identity.users i ON i.id=c.invitee JOIN core_identity.users v ON v.id=c.inviter JOIN core_identity.user_profiles ip ON ip.user_id=i.id JOIN core_identity.user_profiles vp ON vp.user_id=v.id JOIN core_referrals.bindings ib ON ib.user_id=i.id JOIN core_referrals.bindings vb ON vb.user_id=v.id WHERE i.active AND v.active AND core_referrals.promotion_email(ip.email) AND core_referrals.promotion_email(vp.email) AND (ip.email='' OR lower(btrim(ip.email))<>lower(btrim(vp.email))) AND (ib.stripe_customer='' OR ib.stripe_customer<>vb.stripe_customer) ORDER BY c.id LIMIT 1").fetch_one(&pool).await.unwrap();
    let (invitee, inviter, reward, registration, contract) = candidate;
    let f = Fixture {
        store: IngressStore(
            PaymentsStore::new(
                pool.clone(),
                vec![Channel::stripe_test(
                    1,
                    MERCHANT.into(),
                    SECRET.into(),
                    100000,
                )],
            )
            .unwrap(),
        ),
        ledger: Ledger::from_pool(pool.clone()),
        identity,
        secret: secrets.remove(&invitee).unwrap(),
        account: accounts[&invitee],
    };
    let inviter_account = accounts[&inviter];
    let before = affiliate(&f, inviter_account).await;
    let intent = prepare(&f, "frozen-source-prepare-01", 200).await;
    credit(&f, &intent, "frozen-source-credit-01").await;
    assert_eq!(affiliate(&f, inviter_account).await, before + reward);
    refund(&f, &intent, "frozen-source-refund-01", 200).await;
    assert_eq!(affiliate(&f, inviter_account).await, before - registration);
    let state: (String, String) =
        sqlx::query_as("SELECT status,reason FROM core_referrals.contracts WHERE id=$1")
            .bind(contract)
            .fetch_one(&pool)
            .await
            .unwrap();
    assert_eq!(state, ("revoked".into(), "refund".into()));
    lmm_core::core_schema::check(&pool).await.unwrap();
    eprintln!(
        "private source verified: bindings=1239 rewards=269 historical_entries=472 executable_pending=133; registration import effects=0; trusted Store tail/refund exact"
    );
}

#[path = "../src/hex_codec.rs"]
mod hex_codec;

//! Actual PostgreSQL authority tests for imported, still exercisable rights.
//! Source fixtures are synthetic; a packet distribution creates no new money.
use lmm_core::{
    billing::ledger::Ledger,
    identity::IdentityStore,
    payments::{Channel, CreditQuotePolicy, Error as PaymentError, PaymentsStore, discounts},
    promotions::{
        CampaignSpec, IssueRequest, PromotionStore, RedeemRequest, SaveCampaign,
        red_packets::{ItemInput, Mutation, RedPacketError},
    },
    subscriptions::{PlanTerms, SavePlan, SubscriptionStore},
};
use rand::{SeedableRng, rngs::StdRng};
use rsa::{PrivateKey, pkcs8::EncodePublicKey};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use sqlx::PgPool;
use std::{collections::BTreeMap, sync::OnceLock};
#[path = "common/runtime.rs"]
mod runtime;

const MIGRATION: &str = "synthetic-current-discount-packet";
const ROOT: i64 = 81001;
fn digest(v: &str) -> Vec<u8> {
    Sha256::digest(v.as_bytes()).to_vec()
}
fn namespace() -> Vec<u8> {
    digest("synthetic-original-source-namespace")
}
struct Fixture {
    promotions: PromotionStore,
    payments: PaymentsStore,
    ledger: Ledger,
    runtime: runtime::RuntimeRole,
    root: String,
    users: Vec<String>,
    accounts: Vec<i64>,
    now: i64,
}
impl Fixture {
    async fn cleanup(self) {
        drop(self.promotions);
        drop(self.payments);
        self.runtime.cleanup().await;
    }
}
async fn fixture(pool: &PgPool) -> Fixture {
    lmm_core::core_schema::init(pool).await.unwrap();
    let identity = IdentityStore::from_pool(pool.clone());
    let root = identity.bootstrap_user(ROOT, 6).await.unwrap().secret;
    let ledger = Ledger::from_pool(pool.clone());
    let mut users = Vec::new();
    let mut accounts = Vec::new();
    for id in ROOT + 1..=ROOT + 3 {
        let session = identity.bootstrap_user(id, 0).await.unwrap().secret;
        let account = identity.authorize(&session).await.unwrap().owner_account_id;
        ledger.open_wallet(account).await.unwrap();
        users.push(session);
        accounts.push(account);
    }
    sqlx::query("INSERT INTO core_migration.import_runs(migration_id,source_archive_sha256,mapping_sha256,manifest) VALUES($1,encode(sha256('synthetic archive'::bytea),'hex'),encode(sha256('synthetic map'::bytea),'hex'),'{\"scope\":\"synthetic-only\"}')")
        .bind(MIGRATION).execute(pool).await.unwrap();
    let now = sqlx::query_scalar("SELECT floor(extract(epoch FROM clock_timestamp()))::bigint")
        .fetch_one(pool)
        .await
        .unwrap();
    let runtime = runtime::RuntimeRole::create(pool).await;
    runtime.optional("core_promotions").await;
    runtime.optional("core_payments").await;
    static KEY: OnceLock<PrivateKey> = OnceLock::new();
    let key = KEY.get_or_init(|| PrivateKey::new(&mut StdRng::seed_from_u64(81), 2048).unwrap());
    let mut channel = Channel::pancake_isolated_test(
        81001,
        "merchant_continuity_synthetic".into(),
        key.to_public_key()
            .to_public_key_pem(Default::default())
            .unwrap(),
        "STO_continuity".into(),
        "PROD_continuity".into(),
        "USD".into(),
        100_000,
    )
    .unwrap();
    channel.credits_per_minor = 0;
    channel.credit_quote = Some(CreditQuotePolicy {
        min_credit_units: 1,
        settlement_per_usd_numerator: 1,
        settlement_per_usd_denominator: 1,
        discounts: BTreeMap::new(),
    });
    Fixture {
        promotions: PromotionStore::new(runtime.pool.clone()),
        payments: PaymentsStore::new(runtime.pool.clone(), vec![channel]).unwrap(),
        ledger,
        runtime,
        root,
        users,
        accounts,
        now,
    }
}
fn definition(code: &str, max: i64) -> discounts::Definition {
    discounts::Definition {
        code: code.into(),
        name: "Synthetic original discount".into(),
        discount_percent: 20,
        min_credit_units: 2_500_000,
        max_uses: max,
        enabled: true,
        starts_at: 0,
        ends_at: 0,
    }
}
async fn imported_discount(
    pool: &PgPool,
    f: &Fixture,
    source: i64,
    owner: Option<i64>,
    used: i64,
    max: i64,
) -> i64 {
    let code = f
        .payments
        .save_discount(&f.root, None, definition(&format!("LEGACY{source}"), max))
        .await
        .unwrap();
    sqlx::query("INSERT INTO core_payments.legacy_discount_bases(code_id,import_migration_id,source_namespace_sha256,source_archive_sha256,source_discount_id,source_row_sha256,original_owner_user_id,original_used_count,original_min_amount,min_amount_unit) VALUES($1,$2,$3,$4,$5,$6,$7,$8,5,'legacy_batch_500000')")
        .bind(code.id).bind(MIGRATION).bind(namespace()).bind(digest("synthetic archive"))
        .bind(source).bind(digest(&format!("synthetic discount {source}"))).bind(owner).bind(used)
        .execute(pool).await.unwrap();
    code.id
}
async fn usage(pool: &PgPool, id: i64) -> (i64, i64) {
    sqlx::query_as(
        "SELECT used_count,reserved_count FROM core_payments.discount_code_usage WHERE code_id=$1",
    )
    .bind(id)
    .fetch_one(pool)
    .await
    .unwrap()
}
async fn pending(pool: &PgPool, f: &Fixture, order: i64, code: i64) {
    let key = format!("order:{order}");
    sqlx::query("INSERT INTO core_billing.legacy_pending_liabilities(migration_id,source_archive_sha256,mapping_sha256,source_table,source_row_key,source_row_sha256,owner_user_id,owner_account_id,status,settlement_state,payload_json,amount_raw_json,currency_raw_json,observed_at,request,result) VALUES($1,encode(sha256('old archive'::bytea),'hex'),encode(sha256('synthetic map'::bytea),'hex'),'legacy.top_ups',$2,encode(sha256(convert_to($2,'UTF8')),'hex'),$3,$4,'pending','unknown',jsonb_build_object('id',$5,'discount_code_id',$6),'100','\"USD\"',$7,'{}','{}')")
        .bind(MIGRATION).bind(&key).bind(ROOT + 1).bind(f.accounts[0]).bind(order).bind(code).bind(f.now)
        .execute(pool).await.unwrap();
}
async fn hold(pool: &PgPool, code: i64, order: i64, bound: bool) {
    sqlx::query("INSERT INTO core_payments.legacy_discount_holds(code_id,source_namespace_sha256,source_order_id,source_row_sha256,reason,pending_migration_id,pending_source_table,pending_source_row_key) VALUES($1,$2,$3,$4,'unreserved_order_still_payable',$5,$6,$7)")
        .bind(code).bind(namespace()).bind(order).bind(digest(&format!("synthetic hold {order}")))
        .bind(bound.then_some(MIGRATION)).bind(bound.then_some("legacy.top_ups"))
        .bind(bound.then(|| format!("order:{order}"))).execute(pool).await.unwrap();
}
async fn terminal(pool: &PgPool, f: &Fixture, order: i64, step: i64, state: &str) {
    sqlx::query("INSERT INTO core_billing.legacy_pending_liability_receipts(migration_id,source_table,source_row_key,evidence_sha256,observed_at,provider,provider_status,settlement_state,provider_payload_json,request,result) VALUES($1,'legacy.top_ups',$2,encode($3,'hex'),$4,'synthetic_provider',$5,$5,'{}','{}','{}')")
        .bind(MIGRATION).bind(format!("order:{order}"))
        .bind(digest(&format!("actual synthetic receipt {order} {step} {state}")))
        .bind(f.now + step).bind(state).execute(pool).await.unwrap();
}
async fn imported_packet(pool: &PgPool, f: &Fixture, source: i64, limit: i32) -> (i64, String) {
    let slug = format!("legacy-packet-{source}");
    let id = sqlx::query_scalar("INSERT INTO core_promotions.red_packets(slug,title,draw_mode,per_user_limit,start_at,end_at,enabled,created_by,created_at,updated_at) VALUES($1,'Original packet','sequence',$2,0,0,true,$3,$4,$4) RETURNING id")
        .bind(&slug).bind(limit).bind(ROOT).bind(f.now - 100).fetch_one(pool).await.unwrap();
    sqlx::query("INSERT INTO core_promotions.legacy_red_packet_bases(packet_id,import_migration_id,source_namespace_sha256,source_archive_sha256,source_packet_id,source_row_sha256) VALUES($1,$2,$3,$4,$5,$6)")
        .bind(id).bind(MIGRATION).bind(namespace()).bind(digest("synthetic archive"))
        .bind(source).bind(digest(&format!("synthetic packet {source}"))).execute(pool).await.unwrap();
    (id, slug)
}
async fn item(
    pool: &PgPool,
    f: &Fixture,
    packet: i64,
    source: i64,
    code: &str,
    kind: &str,
    discount: Option<i64>,
) -> i64 {
    let value = json!({"item_type":if kind=="discount" {"discount"} else {"redemption"},
        "code":code,"reward_type":kind,"quota":if kind=="quota" {31} else {0},"credited":false});
    let hash = if kind == "reset_voucher" {
        Sha256::digest(
            [
                b"core.subscription-reset-code.v1\0".as_slice(),
                code.as_bytes(),
            ]
            .concat(),
        )
        .to_vec()
    } else {
        digest(code)
    };
    sqlx::query_scalar("INSERT INTO core_promotions.legacy_red_packet_items(packet_id,source_namespace_sha256,source_item_id,source_reward_id,source_row_sha256,source_reward_row_sha256,reward_kind,code_digest,discount_code_id,reward,source_admission_enabled,weight,created_at) VALUES($1,$2,$3,$3,$4,$5,$6,$7,$8,$9,true,1,$10) RETURNING id")
        .bind(packet).bind(namespace()).bind(source).bind(digest(&format!("synthetic item {source}")))
        .bind(digest(&format!("synthetic reward {source}"))).bind(kind).bind(hash).bind(discount)
        .bind(sqlx::types::Json(value)).bind(f.now - 100).fetch_one(pool).await.unwrap()
}
async fn quota(f: &Fixture, key: &str) -> (String, String) {
    let campaign = f
        .promotions
        .save_campaign(
            &f.root,
            None,
            SaveCampaign {
                operation_key: format!("legacy_{key}_campaign_0001"),
                expected_revision: None,
                spec: CampaignSpec {
                    kind: "redemption".into(),
                    title: "Synthetic continuation quota".into(),
                    award_units: "31".into(),
                    budget_units: "31".into(),
                    max_claims: 1,
                    max_claims_per_user: 1,
                    starts_at: 0,
                    ends_at: f.now + 86400,
                    enabled: true,
                },
            },
        )
        .await
        .unwrap();
    let issued = f
        .promotions
        .issue_codes(
            &f.root,
            &campaign.id,
            IssueRequest {
                operation_key: format!("legacy_{key}_issue_000001"),
                quantity: 1,
            },
        )
        .await
        .unwrap();
    (issued.codes[0].id.clone(), issued.codes[0].code.clone())
}
async fn money_counts(pool: &PgPool) -> (i64, i64, i64, i64) {
    sqlx::query_as("SELECT (SELECT count(*) FROM core_billing.ledger_journals),(SELECT count(*) FROM core_promotions.claims),(SELECT count(*) FROM core_subscriptions.reset_vouchers),(SELECT count(*) FROM core_payments.intents)")
        .fetch_one(pool).await.unwrap()
}

#[sqlx::test]
async fn private_discount_and_actual_pending_receipts_preserve_capacity(pool: PgPool) {
    let f = fixture(&pool).await;
    let id = imported_discount(&pool, &f, 1, Some(ROOT + 1), 1, 3).await;
    pending(&pool, &f, 101, 1).await;
    hold(&pool, id, 101, true).await;
    hold(&pool, id, 102, false).await; // Old orphan has no invented native owner.
    assert_eq!(usage(&pool, id).await, (1, 2));
    assert_eq!(
        f.payments
            .quote_discount(&f.users[1], 81001, 5_000_000, "LEGACY1")
            .await
            .unwrap_err(),
        PaymentError::NotFound
    );
    assert_eq!(
        f.payments
            .quote_discount(&f.users[0], 81001, 5_000_000, "LEGACY1")
            .await
            .unwrap_err(),
        PaymentError::Conflict
    );
    terminal(&pool, &f, 101, 1, "terminal_no_payment").await;
    assert_eq!(usage(&pool, id).await, (1, 1));
    let quoted = f
        .payments
        .quote_discount(&f.users[0], 81001, 5_000_000, "LEGACY1")
        .await
        .unwrap();
    assert_eq!((quoted.amount_minor, quoted.credit_units), (800, 5_000_000));
    assert_eq!(
        f.payments
            .quote_discount(&f.users[0], 81001, 2_000_000, "LEGACY1")
            .await
            .unwrap_err(),
        PaymentError::Denied
    );
    // An actual later unresolved receipt must restore the hold. Same-time
    // contradictory receipts cannot manufacture a capacity release.
    terminal(&pool, &f, 101, 2, "unknown").await;
    terminal(&pool, &f, 101, 2, "terminal_no_payment").await;
    assert_eq!(usage(&pool, id).await, (1, 2));
    terminal(&pool, &f, 101, 3, "terminal_paid").await;
    assert_eq!(usage(&pool, id).await, (2, 1));
    terminal(&pool, &f, 101, 4, "terminal_no_payment").await;
    assert_eq!(
        usage(&pool, id).await,
        (2, 1),
        "paid/refunded use never returns capacity"
    );
    let still_unknown: String =
        sqlx::query_scalar("SELECT settlement_state FROM core_billing.legacy_pending_liabilities")
            .fetch_one(&pool)
            .await
            .unwrap();
    assert_eq!(
        still_unknown, "unknown",
        "source base is preserved, not current state"
    );
    assert_eq!(money_counts(&pool).await, (0, 0, 0, 0));
    assert!(
        sqlx::query("UPDATE core_payments.legacy_discount_bases SET original_used_count=0")
            .execute(&f.runtime.pool)
            .await
            .is_err()
    );
    assert!(
        sqlx::query("DELETE FROM core_payments.legacy_discount_holds")
            .execute(&f.runtime.pool)
            .await
            .is_err()
    );
    f.cleanup().await;
}

#[sqlx::test]
async fn quota_distribution_does_not_credit_and_keeps_old_claim_limit(pool: PgPool) {
    let f = fixture(&pool).await;
    let (code_id, code) = quota(&f, "quota").await;
    let (packet, slug) = imported_packet(&pool, &f, 11, 1).await;
    item(&pool, &f, packet, 11, &code, "quota", None).await;
    let before = money_counts(&pool).await;
    let (a, b) = tokio::join!(
        f.promotions.red_packet_claim(&f.users[0], &slug),
        f.promotions.red_packet_claim(&f.users[1], &slug)
    );
    assert_eq!(usize::from(a.is_ok()) + usize::from(b.is_ok()), 1);
    let winner = if a.is_ok() { 0 } else { 1 };
    let reward = a.or(b).unwrap();
    assert_eq!(reward["code"], code);
    assert_eq!(reward["credited"], false);
    assert_eq!(money_counts(&pool).await, before);
    assert_eq!(
        f.ledger
            .balance(f.accounts[winner])
            .await
            .unwrap()
            .available_units,
        0
    );
    assert_eq!(
        f.promotions
            .red_packet_claim(&f.users[winner], &slug)
            .await
            .err(),
        Some(RedPacketError::ClaimLimit)
    );
    assert!(
        f.promotions
            .red_packet_my_claims(&f.users[1 - winner], &slug)
            .await
            .unwrap()
            .is_empty()
    );
    let create = Mutation {
        title: "Cannot bind the old code twice".into(),
        draw_mode: "sequence".into(),
        per_user_limit: 1,
        items: Some(vec![ItemInput {
            item_type: "redemption".into(),
            code_id: Some(code_id),
            weight: 1,
            ..Default::default()
        }]),
        ..Default::default()
    };
    assert_eq!(
        f.promotions.red_packet_create(&f.root, create).await.err(),
        Some(RedPacketError::DuplicatedItem)
    );
    let request = RedeemRequest {
        operation_key: "legacy_quota_redeem_0001".into(),
        code,
    };
    f.promotions
        .redeem(&f.users[winner], request.clone())
        .await
        .unwrap();
    f.promotions
        .redeem(&f.users[winner], request)
        .await
        .unwrap();
    assert_eq!(
        f.ledger
            .balance(f.accounts[winner])
            .await
            .unwrap()
            .available_units,
        31
    );
    assert_eq!(money_counts(&pool).await, (1, 1, 0, 0));
    f.promotions
        .red_packet_delete(&f.root, packet)
        .await
        .unwrap();
    assert_eq!(
        f.promotions
            .red_packet_my_claims(&f.users[winner], &slug)
            .await
            .unwrap()
            .len(),
        1
    );
    assert_eq!(
        f.promotions
            .red_packet_claim(&f.users[2], &slug)
            .await
            .err(),
        Some(RedPacketError::NotFound)
    );
    let free: i64 = sqlx::query_scalar("SELECT count(*) FROM core_promotions.legacy_red_packet_items WHERE released_at IS NOT NULL").fetch_one(&pool).await.unwrap();
    assert_eq!(
        free, 0,
        "deleting a packet does not erase a claimed source binding"
    );
    f.cleanup().await;
}

#[sqlx::test]
async fn discount_packet_assigns_original_code_to_one_owner_and_preserves_history(pool: PgPool) {
    let f = fixture(&pool).await;
    let code = imported_discount(&pool, &f, 21, None, 0, 2).await;
    let (packet, slug) = imported_packet(&pool, &f, 21, 1).await;
    item(&pool, &f, packet, 21, "LEGACY21", "discount", Some(code)).await;
    let (a, b) = tokio::join!(
        f.promotions.red_packet_claim(&f.users[0], &slug),
        f.promotions.red_packet_claim(&f.users[1], &slug)
    );
    assert_eq!(usize::from(a.is_ok()) + usize::from(b.is_ok()), 1);
    let winner = if a.is_ok() { 0 } else { 1 };
    let value = a.or(b).unwrap();
    assert_eq!(value["min_amount"], 5);
    assert_eq!(value["min_credit_units"], "2500000");
    assert_eq!(money_counts(&pool).await, (0, 0, 0, 0));
    f.payments
        .quote_discount(&f.users[winner], 81001, 5_000_000, "LEGACY21")
        .await
        .unwrap();
    assert_eq!(
        f.payments
            .quote_discount(&f.users[1 - winner], 81001, 5_000_000, "LEGACY21")
            .await
            .unwrap_err(),
        PaymentError::NotFound
    );
    let mut changed = definition("RENAMED21", 2);
    changed.min_credit_units = 2_500_001;
    f.payments
        .save_discount(&f.root, Some(code), changed)
        .await
        .unwrap();
    let history = f
        .promotions
        .red_packet_my_claims(&f.users[winner], &slug)
        .await
        .unwrap();
    assert_eq!(history[0]["code"], "RENAMED21");
    assert_eq!(history[0]["min_credit_units"], "2500001");
    assert!(
        history[0].get("min_amount").is_none(),
        "no rounded old-unit amount"
    );
    let immutable_code: String =
        sqlx::query_scalar("SELECT reward->>'code' FROM core_promotions.red_packet_claims")
            .fetch_one(&pool)
            .await
            .unwrap();
    assert_eq!(immutable_code, "LEGACY21");
    assert!(
        sqlx::query("UPDATE core_promotions.legacy_red_packet_items SET released_at=1")
            .execute(&f.runtime.pool)
            .await
            .is_err()
    );
    assert!(
        sqlx::query("UPDATE core_promotions.legacy_red_packet_claim_links SET source_claim_id=1")
            .execute(&f.runtime.pool)
            .await
            .is_err()
    );
    f.cleanup().await;
}

#[sqlx::test]
async fn imported_historical_claims_count_and_unclaimed_deletion_releases_only_bindings(
    pool: PgPool,
) {
    let f = fixture(&pool).await;
    let (packet, slug) = imported_packet(&pool, &f, 31, 1).await;
    let (_, old) = quota(&f, "history").await;
    let old_item = item(&pool, &f, packet, 31, &old, "quota", None).await;
    let historical: i64 = sqlx::query_scalar("INSERT INTO core_promotions.red_packet_claims(packet_id,user_id,claim_index,item_id,promotion_claim_id,reward,created_at) SELECT packet_id,$2,1,NULL,NULL,reward,$3 FROM core_promotions.legacy_red_packet_items WHERE id=$1 RETURNING id")
        .bind(old_item).bind(ROOT + 1).bind(f.now - 50).fetch_one(&pool).await.unwrap();
    sqlx::query("INSERT INTO core_promotions.legacy_red_packet_claim_links(claim_id,item_id,source_claim_id,source_namespace_sha256,source_row_sha256) VALUES($1,$2,31,$3,$4)")
        .bind(historical).bind(old_item).bind(namespace()).bind(digest("synthetic original historical claim")).execute(&pool).await.unwrap();
    let (_, unused) = quota(&f, "unused").await;
    item(&pool, &f, packet, 32, &unused, "quota", None).await;
    assert_eq!(
        f.promotions
            .red_packet_claim(&f.users[0], &slug)
            .await
            .err(),
        Some(RedPacketError::ClaimLimit)
    );
    let before: Value = sqlx::query_scalar(
        "SELECT to_jsonb(c) FROM core_promotions.red_packet_claims c WHERE id=$1",
    )
    .bind(historical)
    .fetch_one(&pool)
    .await
    .unwrap();
    f.promotions
        .red_packet_delete(&f.root, packet)
        .await
        .unwrap();
    let after: Value = sqlx::query_scalar(
        "SELECT to_jsonb(c) FROM core_promotions.red_packet_claims c WHERE id=$1",
    )
    .bind(historical)
    .fetch_one(&pool)
    .await
    .unwrap();
    assert_eq!(before, after);
    let release: (bool, bool) = sqlx::query_as("SELECT bool_and(released_at IS NULL) FILTER(WHERE id=$1),bool_and(released_at IS NOT NULL) FILTER(WHERE id<>$1) FROM core_promotions.legacy_red_packet_items").bind(old_item).fetch_one(&pool).await.unwrap();
    assert_eq!(release, (true, true));
    assert_eq!(
        f.promotions
            .red_packet_my_claims(&f.users[0], &slug)
            .await
            .unwrap()
            .len(),
        1
    );
    assert_eq!(money_counts(&pool).await, (0, 0, 0, 0));
    f.cleanup().await;
}

#[sqlx::test]
async fn retired_reset_codes_are_not_distributed_or_converted_to_wallet_credit(pool: PgPool) {
    let f = fixture(&pool).await;
    f.runtime.optional("core_subscriptions").await;
    let subscriptions = SubscriptionStore::new(f.runtime.pool.clone());
    let plan = subscriptions
        .save_plan(
            &f.root,
            None,
            SavePlan {
                operation_key: "packet_reset_plan_0001".into(),
                expected_revision: None,
                terms: PlanTerms {
                    title: "Actual synthetic reset plan".into(),
                    price_units: "100".into(),
                    duration_months: 0,
                    duration_seconds: 3600,
                    reset_period: "grant".into(),
                    reset_seconds: 0,
                    limit_units: "1000".into(),
                    models: vec!["*".into()],
                    groups: vec!["*".into()],
                    max_purchases_per_user: 0,
                    enabled: true,
                    allow_balance_pay: true,
                },
            },
        )
        .await
        .unwrap();
    let (packet, slug) = imported_packet(&pool, &f, 41, 2).await;
    let expired = "11111111111111111111111111111111";
    let live = "22222222222222222222222222222222";
    let expired_voucher = "33333333333333333333333333333333";
    for (source, code, expiry, voucher_expiry) in [
        (41, expired, f.now - 60, f.now + 86400),
        (42, live, 0, f.now + 86400),
        (43, expired_voucher, 0, f.now - 60),
    ] {
        let hash = Sha256::digest(
            [
                b"core.subscription-reset-code.v1\0".as_slice(),
                code.as_bytes(),
            ]
            .concat(),
        )
        .to_vec();
        sqlx::query("INSERT INTO core_subscriptions.reset_codes(digest,plan_id,issuer_user_id,source_reference,expires_at,voucher_expires_at,enabled) VALUES($1,$2,$3,$4,$5,$6,true)")
            .bind(hash).bind(plan.id).bind(ROOT).bind(format!("synthetic-original-reset-code-{source}"))
            .bind(expiry).bind(voucher_expiry).execute(&pool).await.unwrap();
        item(&pool, &f, packet, source, code, "reset_voucher", None).await;
    }
    let public = f.promotions.red_packet_public(&slug).await.unwrap();
    assert_eq!(public["remaining_items"], 0);
    let app = lmm_core::promotions::router(f.runtime.pool.clone());
    let req = axum::http::Request::post("/core/v1/promotions/redemptions")
        .header("Authorization", format!("Bearer {}", f.users[0]))
        .header("Content-Type", "application/json")
        .body(axum::body::Body::from(
            json!({"operation_key":"retired_packet_code","code":live}).to_string(),
        ))
        .unwrap();
    use tower::ServiceExt;
    assert_eq!(
        app.oneshot(req).await.unwrap().status(),
        axum::http::StatusCode::NOT_FOUND
    );
    assert_eq!(money_counts(&pool).await, (0, 0, 0, 0));
    assert_eq!(
        f.promotions
            .red_packet_claim(&f.users[1], &slug)
            .await
            .err(),
        Some(RedPacketError::Exhausted)
    );
    let resets: i64 = sqlx::query_scalar("SELECT count(*) FROM core_subscriptions.reset_events")
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(
        resets, 0,
        "retired packet rights must leave subscription usage untouched"
    );
    assert_eq!(
        f.ledger
            .balance(f.accounts[0])
            .await
            .unwrap()
            .available_units,
        0
    );
    drop(subscriptions);
    f.cleanup().await;
}

#[sqlx::test]
async fn actual_code_lock_wait_rechecks_code_and_packet_expiry(pool: PgPool) {
    let f = fixture(&pool).await;
    for (source, packet_expiry) in [(51, false), (52, true)] {
        let code = imported_discount(&pool, &f, source, None, 0, 1).await;
        let (packet, slug) = imported_packet(&pool, &f, source, 1).await;
        item(
            &pool,
            &f,
            packet,
            source,
            &format!("LEGACY{source}"),
            "discount",
            Some(code),
        )
        .await;
        if packet_expiry {
            sqlx::query("UPDATE core_promotions.red_packets SET end_at=floor(extract(epoch FROM clock_timestamp()))::bigint+2 WHERE id=$1")
                .bind(packet).execute(&pool).await.unwrap();
        } else {
            sqlx::query("UPDATE core_payments.discount_codes SET ends_at=floor(extract(epoch FROM clock_timestamp()))::bigint+2 WHERE id=$1")
                .bind(code).execute(&pool).await.unwrap();
        }
        let mut held = pool.begin().await.unwrap();
        sqlx::query("SELECT id FROM core_payments.discount_codes WHERE id=$1 FOR UPDATE")
            .bind(code)
            .fetch_one(&mut *held)
            .await
            .unwrap();
        let store = f.promotions.clone();
        let secret = f.users[0].clone();
        let task = tokio::spawn(async move { store.red_packet_claim(&secret, &slug).await });
        let mut waiting = false;
        for _ in 0..100 {
            waiting = sqlx::query_scalar("SELECT EXISTS(SELECT FROM pg_stat_activity WHERE datname=current_database() AND wait_event_type='Lock' AND query LIKE '%lock_legacy_discount_code%')")
                .fetch_one(&pool).await.unwrap();
            if waiting {
                break;
            }
            tokio::time::sleep(std::time::Duration::from_millis(10)).await;
        }
        assert!(
            waiting,
            "the claim must actually wait for the coupon authority lock"
        );
        tokio::time::sleep(std::time::Duration::from_millis(2100)).await;
        held.commit().await.unwrap();
        let expected = if packet_expiry {
            RedPacketError::Expired
        } else {
            RedPacketError::Exhausted
        };
        assert_eq!(task.await.unwrap().err(), Some(expected));
    }
    let claims: i64 = sqlx::query_scalar("SELECT count(*) FROM core_promotions.red_packet_claims")
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(claims, 0);
    assert_eq!(money_counts(&pool).await, (0, 0, 0, 0));
    f.cleanup().await;
}

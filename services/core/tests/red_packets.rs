//! Synthetic-only PostgreSQL tests for red packets over Core redemption codes.
//! Every claim must be one canonical promotion grant; packets never mint value.
use axum::{
    body::Body,
    http::{Request, StatusCode},
};
use lmm_core::{
    billing::ledger::Ledger,
    identity::IdentityStore,
    promotions::{
        CampaignSpec, IssueRequest, PromotionStore, RedeemRequest, SaveCampaign,
        red_packets::{self, ItemInput, Mutation, RedPacketError},
    },
};
use serde_json::Value;
use sqlx::PgPool;
use tower::ServiceExt;
#[path = "common/runtime.rs"]
mod runtime;

struct Fixture {
    store: PromotionStore,
    ledger: Ledger,
    root: String,
    users: Vec<String>,
    accounts: Vec<i64>,
    now: i64,
}
async fn fixture(pool: &PgPool, n: i64) -> (Fixture, runtime::RuntimeRole) {
    let identity = IdentityStore::from_pool(pool.clone());
    lmm_core::core_schema::init(pool).await.unwrap();
    let root = identity.bootstrap_user(8500, 6).await.unwrap().secret;
    let ledger = Ledger::from_pool(pool.clone());
    let (mut users, mut accounts) = (Vec::new(), Vec::new());
    for i in 0..n {
        let session = identity.bootstrap_user(8501 + i, 0).await.unwrap().secret;
        let account = identity.authorize(&session).await.unwrap().owner_account_id;
        ledger.open_wallet(account).await.unwrap();
        accounts.push(account);
        users.push(session);
    }
    let now = sqlx::query_scalar("SELECT extract(epoch FROM clock_timestamp())::bigint")
        .fetch_one(pool)
        .await
        .unwrap();
    let runtime = runtime::RuntimeRole::create(pool).await;
    runtime.optional("core_promotions").await;
    (
        Fixture {
            store: PromotionStore::new(runtime.pool.clone()),
            ledger,
            root,
            users,
            accounts,
            now,
        },
        runtime,
    )
}
/// Issues `n` codes of `award` units and returns (code id, plaintext) pairs.
async fn codes(f: &Fixture, key: &str, award: i64, n: i32) -> Vec<(String, String)> {
    let c = f
        .store
        .save_campaign(
            &f.root,
            None,
            SaveCampaign {
                operation_key: format!("{key}_campaign_0001"),
                expected_revision: None,
                spec: CampaignSpec {
                    kind: "redemption".into(),
                    title: format!("Synthetic {key}"),
                    award_units: award.to_string(),
                    budget_units: (award * i64::from(n)).to_string(),
                    max_claims: i64::from(n),
                    max_claims_per_user: 10,
                    starts_at: f.now - 60,
                    ends_at: f.now + 86400,
                    enabled: true,
                },
            },
        )
        .await
        .unwrap();
    let issue = f
        .store
        .issue_codes(
            &f.root,
            &c.id,
            IssueRequest {
                operation_key: format!("{key}_issue_000001"),
                quantity: n,
            },
        )
        .await
        .unwrap();
    issue.codes.into_iter().map(|c| (c.id, c.code)).collect()
}
fn packet(items: &[(String, String)], limit: i64) -> Mutation {
    Mutation {
        title: "Synthetic packet".into(),
        draw_mode: "random".into(),
        per_user_limit: limit,
        items: Some(
            items
                .iter()
                .map(|(id, _)| ItemInput {
                    item_type: "redemption".into(),
                    code_id: Some(id.clone()),
                    weight: 1,
                    ..Default::default()
                })
                .collect(),
        ),
        ..Default::default()
    }
}
async fn balanced(pool: &PgPool) {
    let bad: i64 = sqlx::query_scalar("SELECT count(*) FROM (SELECT journal_id FROM core_billing.ledger_entries GROUP BY journal_id HAVING sum(delta_units)<>0) bad").fetch_one(pool).await.unwrap();
    assert_eq!(bad, 0, "unbalanced journal");
    let bad: i64 = sqlx::query_scalar("SELECT count(*) FROM core_promotions.campaigns c WHERE c.spent_units<>(SELECT coalesce(sum(amount_units),0) FROM core_promotions.claims l WHERE l.campaign_id=c.id) OR c.reserved_units<>c.award_units*(SELECT count(*) FROM core_promotions.codes d WHERE d.campaign_id=c.id AND NOT d.revoked AND NOT EXISTS(SELECT 1 FROM core_promotions.claims l WHERE l.code_digest=d.digest))").fetch_one(pool).await.unwrap();
    assert_eq!(bad, 0, "campaign budget drift");
    // Every packet claim is exactly one gift journal for exactly its code.
    let bad: i64 = sqlx::query_scalar("SELECT count(*) FROM core_promotions.red_packet_claims r JOIN core_promotions.red_packet_items i ON i.id=r.item_id JOIN core_promotions.claims l ON l.id=r.promotion_claim_id JOIN core_billing.ledger_journals j ON j.id=l.journal_id WHERE l.code_digest<>i.code_digest OR j.kind<>'gift' OR l.actor_user_id<>r.user_id").fetch_one(pool).await.unwrap();
    assert_eq!(bad, 0, "packet claim not bound to its grant");
}
async fn wallet(f: &Fixture, i: usize) -> i64 {
    f.ledger
        .balance(f.accounts[i])
        .await
        .unwrap()
        .available_units
}

#[sqlx::test]
async fn red_packet_claims_spend_bound_codes_once(pool: PgPool) {
    let (f, _runtime) = fixture(&pool, 6).await;
    let items = codes(&f, "packet", 100, 3).await;
    // Non-admins cannot create; legacy row ids and discount items are rejected.
    assert!(matches!(
        f.store
            .red_packet_create(&f.users[0], packet(&items, 1))
            .await,
        Err(RedPacketError::Core(lmm_core::promotions::Error::Forbidden))
    ));
    let mut legacy = packet(&items, 1);
    legacy.items = Some(vec![ItemInput {
        item_type: "redemption".into(),
        source_id: Some(7),
        ..Default::default()
    }]);
    assert!(matches!(
        f.store.red_packet_create(&f.root, legacy).await,
        Err(RedPacketError::Invalid(_))
    ));
    let mut discount = packet(&items, 1);
    discount.items.as_mut().unwrap()[0].item_type = "discount".into();
    assert_eq!(
        f.store.red_packet_create(&f.root, discount).await.err(),
        Some(RedPacketError::InvalidItem)
    );
    let created = f
        .store
        .red_packet_create(&f.root, packet(&items, 1))
        .await
        .unwrap();
    let slug = created["slug"].as_str().unwrap().to_owned();
    let id = created["id"].as_i64().unwrap();
    // A code can live in only one packet at a time.
    assert_eq!(
        f.store
            .red_packet_create(&f.root, packet(&items[..1], 1))
            .await
            .err(),
        Some(RedPacketError::DuplicatedItem)
    );
    let view = f.store.red_packet_public(&slug).await.unwrap();
    assert_eq!(
        (
            view["total_items"].as_i64(),
            view["remaining_items"].as_i64()
        ),
        (Some(3), Some(3))
    );

    let reward = f.store.red_packet_claim(&f.users[0], &slug).await.unwrap();
    assert_eq!(reward["quota"], 100);
    assert_eq!(reward["credited"], true);
    assert_eq!(wallet(&f, 0).await, 100);
    assert_eq!(
        f.store.red_packet_claim(&f.users[0], &slug).await.err(),
        Some(RedPacketError::ClaimLimit)
    );
    assert_eq!(wallet(&f, 0).await, 100, "no double credit");
    // A code redeemed directly is skipped by the packet: exactly once overall.
    let claimed: Vec<String> = sqlx::query_scalar("SELECT encode(i.code_digest,'hex') FROM core_promotions.red_packet_items i JOIN core_promotions.red_packet_claims r ON r.item_id=i.id").fetch_all(&pool).await.unwrap();
    let direct = items.iter().find(|(id, _)| !claimed.contains(id)).unwrap();
    f.store
        .redeem(
            &f.users[1],
            RedeemRequest {
                operation_key: "direct_redeem_code_01".into(),
                code: direct.1.clone(),
            },
        )
        .await
        .unwrap();
    assert_eq!(wallet(&f, 1).await, 100);
    f.store.red_packet_claim(&f.users[2], &slug).await.unwrap();
    assert_eq!(
        f.store.red_packet_claim(&f.users[3], &slug).await.err(),
        Some(RedPacketError::Exhausted)
    );
    let mine = f
        .store
        .red_packet_my_claims(&f.users[0], &slug)
        .await
        .unwrap();
    assert_eq!(mine.len(), 1);
    assert_eq!(mine[0]["claim_id"], reward["claim_id"]);
    assert!(
        f.store
            .red_packet_my_claims(&f.users[3], &slug)
            .await
            .unwrap()
            .is_empty()
    );
    let list = f.store.red_packet_admin_list(&f.root).await.unwrap();
    assert_eq!(list[0]["claim_count"], 2);
    assert_eq!(list[0]["remaining_items"], 0);
    // Claimed packets cannot replace stock.
    assert_eq!(
        f.store
            .red_packet_update(&f.root, id, packet(&items, 1))
            .await
            .err(),
        Some(RedPacketError::HasClaims)
    );
    balanced(&pool).await;
}

#[sqlx::test]
async fn red_packet_concurrent_claims_never_oversell(pool: PgPool) {
    let (f, _runtime) = fixture(&pool, 8).await;
    let items = codes(&f, "concurrent", 250, 3).await;
    let slug = f
        .store
        .red_packet_create(&f.root, packet(&items, 1))
        .await
        .unwrap()["slug"]
        .as_str()
        .unwrap()
        .to_owned();
    let mut tasks = Vec::new();
    for user in f.users.clone() {
        let (store, slug) = (f.store.clone(), slug.clone());
        tasks.push(tokio::spawn(async move {
            store.red_packet_claim(&user, &slug).await
        }));
    }
    let (mut ok, mut exhausted) = (0, 0);
    for task in tasks {
        match task.await.unwrap() {
            Ok(_) => ok += 1,
            Err(RedPacketError::Exhausted) => exhausted += 1,
            other => panic!("unexpected {other:?}"),
        }
    }
    assert_eq!((ok, exhausted), (3, 5));
    let total: i64 = sqlx::query_scalar(
        "SELECT coalesce(sum(amount_units),0)::bigint FROM core_promotions.claims",
    )
    .fetch_one(&pool)
    .await
    .unwrap();
    assert_eq!(total, 750);
    balanced(&pool).await;
}

#[sqlx::test]
async fn red_packet_delete_and_expiry_release_unclaimed_codes(pool: PgPool) {
    let (f, _runtime) = fixture(&pool, 2).await;
    let items = codes(&f, "release", 100, 3).await;
    let created = f
        .store
        .red_packet_create(&f.root, packet(&items, 1))
        .await
        .unwrap();
    let (id, slug) = (
        created["id"].as_i64().unwrap(),
        created["slug"].as_str().unwrap().to_owned(),
    );
    f.store.red_packet_claim(&f.users[0], &slug).await.unwrap();
    f.store.red_packet_delete(&f.root, id).await.unwrap();
    f.store.red_packet_delete(&f.root, id).await.unwrap();
    let released: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM core_promotions.red_packet_items WHERE released_at IS NOT NULL",
    )
    .fetch_one(&pool)
    .await
    .unwrap();
    assert_eq!(released, 2);
    let view = f.store.red_packet_public(&slug).await.unwrap();
    assert_eq!(
        (view["enabled"].as_bool(), view["remaining_items"].as_i64()),
        (Some(false), Some(0))
    );
    assert_eq!(
        f.store.red_packet_claim(&f.users[1], &slug).await.err(),
        Some(RedPacketError::NotFound)
    );
    // The claimant still sees the received reward after deletion.
    assert_eq!(
        f.store
            .red_packet_my_claims(&f.users[0], &slug)
            .await
            .unwrap()
            .len(),
        1
    );
    assert!(
        f.store
            .red_packet_admin_list(&f.root)
            .await
            .unwrap()
            .is_empty()
    );
    // Released codes keep their reserved budget and can back a new, expired packet.
    let unclaimed: Vec<(String, String)> = {
        let claimed: String = sqlx::query_scalar("SELECT encode(i.code_digest,'hex') FROM core_promotions.red_packet_items i JOIN core_promotions.red_packet_claims r ON r.item_id=i.id").fetch_one(&pool).await.unwrap();
        items.into_iter().filter(|(id, _)| *id != claimed).collect()
    };
    let mut expired = packet(&unclaimed, 1);
    expired.end_at = f.now - 1;
    let slug = f.store.red_packet_create(&f.root, expired).await.unwrap()["slug"]
        .as_str()
        .unwrap()
        .to_owned();
    assert_eq!(
        f.store.red_packet_claim(&f.users[1], &slug).await.err(),
        Some(RedPacketError::Expired)
    );
    assert_eq!(wallet(&f, 1).await, 0);
    let reserved: String = f.store.campaigns(&f.root, true).await.unwrap()[0]
        .reserved_units
        .clone();
    assert_eq!(reserved, "200");
    balanced(&pool).await;
}

#[sqlx::test]
async fn red_packet_http_uses_legacy_envelope(pool: PgPool) {
    let (f, runtime) = fixture(&pool, 1).await;
    let items = codes(&f, "http", 100, 1).await;
    let app = red_packets::router(runtime.pool.clone());
    let call = |method: &str, uri: &str, auth: Option<&str>, body: String| {
        let mut request = Request::builder()
            .method(method)
            .uri(uri)
            .header("content-type", "application/json");
        if let Some(secret) = auth {
            request = request.header("authorization", format!("Bearer {secret}"));
        }
        let app = app.clone();
        let request = request.body(Body::from(body)).unwrap();
        async move {
            let response = app.oneshot(request).await.unwrap();
            let status = response.status();
            let bytes = axum::body::to_bytes(response.into_body(), 1 << 20)
                .await
                .unwrap();
            (status, serde_json::from_slice::<Value>(&bytes).unwrap())
        }
    };
    let body = format!(
        r#"{{"title":"HTTP","items":[{{"item_type":"redemption","code_id":"{}","weight":1}}]}}"#,
        items[0].0
    );
    let (status, created) = call("POST", "/api/red-packet/admin", Some(&f.root), body).await;
    assert_eq!(status, StatusCode::OK);
    let slug = created["data"]["slug"].as_str().unwrap().to_owned();
    let (status, _) = call(
        "POST",
        &format!("/api/red-packet/public/{slug}/claim"),
        None,
        "{}".into(),
    )
    .await;
    assert_eq!(status, StatusCode::UNAUTHORIZED);
    let (status, view) = call(
        "GET",
        &format!("/api/red-packet/public/{slug}"),
        None,
        String::new(),
    )
    .await;
    assert_eq!(
        (status, view["data"]["remaining_items"].as_i64()),
        (StatusCode::OK, Some(1))
    );
    let (_, won) = call(
        "POST",
        &format!("/api/red-packet/public/{slug}/claim"),
        Some(&f.users[0]),
        "{}".into(),
    )
    .await;
    assert_eq!(
        (won["success"].as_bool(), won["data"]["quota"].as_i64()),
        (Some(true), Some(100))
    );
    let (status, again) = call(
        "POST",
        &format!("/api/red-packet/public/{slug}/claim"),
        Some(&f.users[0]),
        "{}".into(),
    )
    .await;
    assert_eq!(
        (status, again["success"].as_bool()),
        (StatusCode::OK, Some(false))
    );
    assert_eq!(again["message"], "已达到该红包的领取次数上限");
    let (status, _) = call(
        "GET",
        "/api/red-packet/admin",
        Some(&f.users[0]),
        String::new(),
    )
    .await;
    assert_eq!(status, StatusCode::FORBIDDEN);
    assert_eq!(wallet(&f, 0).await, 100);
    balanced(&pool).await;
}

#[sqlx::test(migrations = false)]
async fn packet_claim_racing_original_code_redeem_posts_one_reward(pool: PgPool) {
    let (f, runtime) = fixture(&pool, 2).await;
    let items = codes(&f, "packet_direct_race", 100, 1).await;
    let created = f
        .store
        .red_packet_create(&f.root, packet(&items, 1))
        .await
        .unwrap();
    let slug = created["slug"].as_str().unwrap();
    let (packet, direct) = tokio::join!(
        f.store.red_packet_claim(&f.users[0], slug),
        f.store.redeem(
            &f.users[1],
            RedeemRequest {
                code: items[0].1.clone(),
                operation_key: "direct_packet_race_01".into(),
            }
        ),
    );
    assert_ne!(packet.is_ok(), direct.is_ok());
    assert_eq!(wallet(&f, 0).await + wallet(&f, 1).await, 100);
    let claims: i64 = sqlx::query_scalar("SELECT count(*) FROM core_promotions.claims")
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(claims, 1);
    balanced(&pool).await;
    runtime.cleanup().await;
}

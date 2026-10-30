//! Synthetic-only PostgreSQL acceptance using real Core identity, budget rows
//! and canonical ledger. Promotional grants are not external payment credits.
use lmm_core::{
    billing::ledger::{self, Ledger},
    identity::IdentityStore,
    promotions::{
        self, CampaignSpec, ClaimRequest, Error, IssueRequest, PromotionStore, RedeemRequest,
        SaveCampaign,
    },
};
use sqlx::PgPool;
use std::sync::Arc;
use tokio::sync::Barrier;
#[path = "common/runtime.rs"]
mod runtime;

struct Fixture {
    identity: IdentityStore,
    store: PromotionStore,
    ledger: Ledger,
    root: String,
    users: Vec<String>,
    accounts: Vec<i64>,
    now: i64,
}
async fn fixture(pool: &PgPool, n: i64, full: bool) -> Fixture {
    let identity = IdentityStore::from_pool(pool.clone());
    if full {
        lmm_core::core_schema::init(pool).await.unwrap();
    } else {
        identity.init_database().await.unwrap();
        for schema in [ledger::SCHEMA, lmm_core::abuse::SCHEMA, promotions::SCHEMA] {
            sqlx::raw_sql(schema).execute(pool).await.unwrap();
        }
    }
    let root = identity.bootstrap_user(8400, 6).await.unwrap().secret;
    let mut users = Vec::new();
    let mut accounts = Vec::new();
    let ledger = Ledger::from_pool(pool.clone());
    for i in 0..n {
        let session = identity.bootstrap_user(8401 + i, 0).await.unwrap().secret;
        let account = identity.authorize(&session).await.unwrap().owner_account_id;
        ledger.open_wallet(account).await.unwrap();
        accounts.push(account);
        users.push(session);
    }
    let now = sqlx::query_scalar("SELECT extract(epoch FROM clock_timestamp())::bigint")
        .fetch_one(pool)
        .await
        .unwrap();
    Fixture {
        identity,
        store: PromotionStore::new(pool.clone()),
        ledger,
        root,
        users,
        accounts,
        now,
    }
}
fn spec(now: i64, kind: &str, budget: i64) -> CampaignSpec {
    CampaignSpec {
        kind: kind.into(),
        title: "Synthetic reward".into(),
        award_units: "100".into(),
        budget_units: budget.to_string(),
        max_claims: budget / 100,
        max_claims_per_user: if kind == "envelope" { 1 } else { 3 },
        starts_at: now - 60,
        ends_at: now + 3 * 86400,
        enabled: true,
    }
}
fn save(now: i64, kind: &str, budget: i64, key: &str) -> SaveCampaign {
    SaveCampaign {
        operation_key: key.into(),
        expected_revision: None,
        spec: spec(now, kind, budget),
    }
}
fn claim(key: &str) -> ClaimRequest {
    ClaimRequest {
        operation_key: key.into(),
    }
}
fn redeem(key: &str, code: &str) -> RedeemRequest {
    RedeemRequest {
        operation_key: key.into(),
        code: code.into(),
    }
}
async fn balanced(pool: &PgPool) {
    let bad:i64=sqlx::query_scalar("SELECT count(*) FROM (SELECT journal_id FROM core_billing.ledger_entries GROUP BY journal_id HAVING sum(delta_units)<>0) bad").fetch_one(pool).await.unwrap();
    assert_eq!(bad, 0);
    let bad:i64=sqlx::query_scalar("SELECT count(*) FROM core_promotions.campaigns c WHERE c.spent_units<>(SELECT coalesce(sum(amount_units),0) FROM core_promotions.claims l WHERE l.campaign_id=c.id) OR c.claims<>(SELECT count(*) FROM core_promotions.claims l WHERE l.campaign_id=c.id) OR c.reserved_units<>c.award_units*(SELECT count(*) FROM core_promotions.codes d WHERE d.campaign_id=c.id AND NOT d.revoked AND NOT EXISTS(SELECT 1 FROM core_promotions.claims l WHERE l.code_digest=d.digest))").fetch_one(pool).await.unwrap();
    assert_eq!(bad, 0);
    let credit: i64 =
        sqlx::query_scalar("SELECT count(*) FROM core_billing.ledger_journals WHERE kind='credit'")
            .fetch_one(pool)
            .await
            .unwrap();
    assert_eq!(
        credit, 0,
        "promotional gifts masqueraded as external payment"
    );
}

#[sqlx::test]
async fn redemption_codes_reserve_budget_and_recover_exact_issue_and_claim(pool: PgPool) {
    let mut f = fixture(&pool, 2, true).await;
    let runtime = runtime::RuntimeRole::create(&pool).await;
    runtime.optional("core_promotions").await;
    f.store = PromotionStore::new(runtime.pool.clone());
    assert_eq!(
        f.store
            .save_campaign(
                &f.users[0],
                None,
                save(f.now, "redemption", 200, "forbidden_campaign_01")
            )
            .await,
        Err(Error::Forbidden)
    );
    let c = f
        .store
        .save_campaign(
            &f.root,
            None,
            save(f.now, "redemption", 200, "redemption_campaign_01"),
        )
        .await
        .unwrap();
    let input = IssueRequest {
        operation_key: "issue_two_codes_0001".into(),
        quantity: 2,
    };
    let issue = f
        .store
        .issue_codes(&f.root, &c.id, input.clone())
        .await
        .unwrap();
    assert_eq!(
        f.store.issue_codes(&f.root, &c.id, input).await.unwrap(),
        issue
    );
    let status = f.store.campaigns(&f.root, true).await.unwrap();
    assert_eq!(status[0].reserved_units, "200");
    assert_eq!(status[0].spent_units, "0");
    assert_eq!(
        f.store
            .issue_codes(
                &f.root,
                &c.id,
                IssueRequest {
                    operation_key: "unfunded_extra_code_01".into(),
                    quantity: 1
                }
            )
            .await,
        Err(Error::Exhausted)
    );
    let first = f
        .store
        .redeem(
            &f.users[0],
            redeem("redeem_first_code_01", &issue.codes[0].code),
        )
        .await
        .unwrap();
    assert_eq!(
        f.store
            .redeem(
                &f.users[0],
                redeem("redeem_first_code_01", &issue.codes[0].code)
            )
            .await
            .unwrap(),
        first
    );
    assert_eq!(
        f.store
            .redeem(
                &f.users[0],
                redeem("same_code_new_key_01", &issue.codes[0].code)
            )
            .await
            .unwrap(),
        first
    );
    assert_eq!(
        f.store
            .redeem(
                &f.users[1],
                redeem("other_user_same_code", &issue.codes[0].code)
            )
            .await,
        Err(Error::Conflict)
    );
    assert_eq!(
        f.ledger
            .balance(f.accounts[0])
            .await
            .unwrap()
            .available_units,
        100
    );
    assert_eq!(
        f.store.revoke_code(&f.root, &issue.codes[0].id).await,
        Err(Error::Conflict)
    );
    let revoked = f
        .store
        .revoke_code(&f.root, &issue.codes[1].id)
        .await
        .unwrap();
    assert!(revoked.revoked);
    assert_eq!(
        f.store
            .revoke_code(&f.root, &issue.codes[1].id)
            .await
            .unwrap(),
        revoked
    );
    assert_eq!(
        f.store
            .redeem(
                &f.users[1],
                redeem("revoked_code_denied_01", &issue.codes[1].code)
            )
            .await,
        Err(Error::Conflict)
    );
    let status = f.store.campaigns(&f.root, true).await.unwrap();
    assert_eq!(status[0].reserved_units, "0");
    assert_eq!(status[0].spent_units, "100");
    let rows: Vec<String> =
        sqlx::query_scalar("SELECT request::text||result::text FROM core_promotions.commands")
            .fetch_all(&pool)
            .await
            .unwrap();
    for code in &issue.codes {
        assert!(
            rows.iter().all(|row| !row.contains(&code.code)),
            "plaintext redemption code persisted"
        );
    }
    let mut disabled = c.spec.clone();
    disabled.enabled = false;
    f.store
        .save_campaign(
            &f.root,
            Some(&c.id),
            SaveCampaign {
                operation_key: "disable_campaign_01".into(),
                expected_revision: Some(c.revision),
                spec: disabled,
            },
        )
        .await
        .unwrap();
    assert_eq!(
        f.store
            .redeem(
                &f.users[0],
                redeem("redeem_first_code_01", &issue.codes[0].code)
            )
            .await
            .unwrap(),
        first
    );
    assert_eq!(f.store.history(&f.users[0]).await.unwrap(), vec![first]);
    balanced(&pool).await;
    runtime.cleanup().await;
}

#[sqlx::test]
async fn utc_checkin_is_once_per_day_and_budget_and_current_identity_are_enforced(pool: PgPool) {
    let f = fixture(&pool, 3, false).await;
    let c = f
        .store
        .save_campaign(
            &f.root,
            None,
            save(f.now, "checkin", 200, "daily_campaign_0001"),
        )
        .await
        .unwrap();
    let first = f
        .store
        .checkin(&f.users[0], &c.id, claim("daily_claim_key_0001"))
        .await
        .unwrap();
    let second = f
        .store
        .checkin(&f.users[0], &c.id, claim("daily_claim_key_0002"))
        .await
        .unwrap();
    assert_eq!(first, second);
    assert_eq!(first.claimed_at / 86400, f.now / 86400);
    f.store
        .checkin(&f.users[1], &c.id, claim("second_user_day_0001"))
        .await
        .unwrap();
    assert_eq!(
        f.store
            .checkin(&f.users[2], &c.id, claim("exhausted_day_0001"))
            .await,
        Err(Error::Exhausted)
    );
    assert_eq!(
        f.ledger
            .balance(f.accounts[0])
            .await
            .unwrap()
            .available_units,
        100
    );
    let mut reduced = c.spec.clone();
    reduced.budget_units = "100".into();
    assert_eq!(
        f.store
            .save_campaign(
                &f.root,
                Some(&c.id),
                SaveCampaign {
                    operation_key: "cannot_reduce_spent".into(),
                    expected_revision: Some(1),
                    spec: reduced
                }
            )
            .await,
        Err(Error::Conflict)
    );
    let subject: String =
        sqlx::query_scalar("SELECT subject_key FROM core_promotions.claims WHERE id=$1")
            .bind(&first.id)
            .fetch_one(&pool)
            .await
            .unwrap();
    assert_eq!(subject, format!("8401:{}", first.claimed_at / 86400));
    sqlx::query("UPDATE core_identity.users SET active=false WHERE id=8401")
        .execute(&pool)
        .await
        .unwrap();
    assert!(matches!(
        f.store
            .checkin(&f.users[0], &c.id, claim("daily_claim_key_0001"))
            .await,
        Err(Error::Unauthorized | Error::Forbidden)
    ));
    balanced(&pool).await;
}

#[sqlx::test]
async fn concurrent_envelopes_never_exceed_budget_and_malformed_http_never_mints(pool: PgPool) {
    let f = fixture(&pool, 8, false).await;
    let c = f
        .store
        .save_campaign(
            &f.root,
            None,
            save(f.now, "envelope", 300, "envelope_campaign_01"),
        )
        .await
        .unwrap();
    let barrier = Arc::new(Barrier::new(f.users.len()));
    let mut jobs = tokio::task::JoinSet::new();
    for (i, secret) in f.users.iter().enumerate() {
        let s = f.store.clone();
        let secret = secret.clone();
        let id = c.id.clone();
        let barrier = barrier.clone();
        jobs.spawn(async move {
            barrier.wait().await;
            let input = claim(&format!("concurrent_claim_{i:04}"));
            // SERIALIZABLE aborts retry the exact request, as real clients must.
            for _ in 0..16 {
                match s.envelope(&secret, &id, input.clone()).await {
                    Err(Error::Unavailable) => tokio::task::yield_now().await,
                    result => return result,
                }
            }
            panic!("isolated transaction could not make progress")
        });
    }
    let mut granted = 0;
    let mut exhausted = 0;
    while let Some(result) = jobs.join_next().await {
        match result.unwrap() {
            Ok(_) => granted += 1,
            Err(Error::Exhausted) => exhausted += 1,
            other => panic!("unexpected result {other:?}"),
        }
    }
    assert_eq!((granted, exhausted), (3, 5));
    let rows = f.store.campaigns(&f.root, true).await.unwrap();
    assert_eq!(rows[0].spent_units, "300");
    assert_eq!(rows[0].claims, 3);
    use axum::{
        body::Body,
        http::{Request, StatusCode},
    };
    use tower::ServiceExt;
    let app = promotions::router(pool.clone());
    let response = app
        .clone()
        .oneshot(
            Request::post("/core/v1/promotions/redemptions")
                .header("content-type", "application/json")
                .body(Body::from("{}"))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
    let response=app.oneshot(Request::post(format!("/core/v1/promotions/campaigns/{}/checkin",c.id)).header("authorization",format!("Bearer {}",f.users[0])).header("content-type","application/json").body(Body::from(r#"{"operation_key":"forged_amount_request","amount_units":"999999","recipient_user_id":8402}"#)).unwrap()).await.unwrap();
    assert_eq!(response.status(), StatusCode::BAD_REQUEST);
    assert_eq!(
        f.identity.authorize(&f.users[0]).await.unwrap().user_id,
        8401
    );
    balanced(&pool).await;
}

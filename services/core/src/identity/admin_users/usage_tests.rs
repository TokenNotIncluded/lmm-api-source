//! User-detail statistics from actual private charging and ledger transactions.
use super::*;
use crate::billing::{
    charging::{Charging, Outcome, Price, Request as ChargeRequest, Source, Usage},
    transactional_ledger::TransactionalLedger,
};
use std::sync::Arc;

const WORKER: [u8; 32] = [52; 32];

async fn key(
    f: &Fixture,
    pool: &PgPool,
    session: &str,
    user: i64,
    owner: Account,
    funding: FundingPolicy,
) -> IssuedCredential {
    let issued = f
        .identity
        .issue_key(session, owner, funding, 3600)
        .await
        .unwrap();
    sqlx::query("INSERT INTO core_identity.api_key_settings(credential_id,name,group_name) VALUES($1,'User usage fixture','default')")
        .bind(issued.id).execute(pool).await.unwrap();
    sqlx::query("INSERT INTO core_identity.user_routing_groups(user_id,group_name,source_reference) VALUES($1,'default','user-usage-fixture') ON CONFLICT DO NOTHING")
        .bind(user).execute(pool).await.unwrap();
    issued
}

fn spec(id: &str, reserve: i64) -> ChargeRequest {
    ChargeRequest {
        id: id.into(),
        fingerprint: [53; 32],
        reserve,
        lease_seconds: 300,
        maximum_seconds: 600,
        price: Price {
            version: "user-usage-v1".into(),
            model: "test-model".into(),
            group: "default".into(),
            input_per_million: 1_000_000,
            cached_per_million: 500_000,
            output_per_million: 1_000_000,
            rounding: None,
            tool: None,
            text_request: None,
            audio: None,
            grounding: None,
            user_trust: None,
            image: None,
            upstream_cost_group_ppm: None,
            tiered: None,
        },
    }
}

async fn settle(engine: &Charging, secret: &str, id: &str, units: i64, outcome: Outcome) {
    engine
        .reserve(secret, &spec(id, units.max(100)), &WORKER)
        .await
        .unwrap();
    engine.start(id, &WORKER).await.unwrap();
    engine
        .settle(
            id,
            &WORKER,
            Usage {
                output: units,
                ..Default::default()
            },
            outcome,
        )
        .await
        .unwrap();
}

async fn usage(f: &Fixture, user: i64) -> Value {
    serde_json::to_value(f.store.user(&f.admin, user).await.unwrap()).unwrap()["usage"].clone()
}

#[sqlx::test(migrations = false)]
async fn exact_user_usage_tracks_receipts_refunds_and_payer_buckets_without_writes(pool: PgPool) {
    let f = fresh(&pool).await;
    let target = person(&f, &pool, 10, 0, "usage-target").await;
    let other = person(&f, &pool, 11, 0, "usage-other").await;
    let (ledger, account) = fund(&f, &pool, 10).await;
    let personal = key(
        &f,
        &pool,
        &target.secret,
        10,
        Account {
            kind: AccountKind::Personal,
            id: 10,
        },
        FundingPolicy::inherit_account_settings(),
    )
    .await;
    let other_key = key(
        &f,
        &pool,
        &other.secret,
        11,
        Account {
            kind: AccountKind::Personal,
            id: 11,
        },
        FundingPolicy::new_personal_key(),
    )
    .await;
    let team = f.identity.create_team(&target.secret).await.unwrap();
    let team_owned = key(
        &f,
        &pool,
        &target.secret,
        10,
        Account {
            kind: AccountKind::Team,
            id: team,
        },
        FundingPolicy::inherit_account_settings(),
    )
    .await;
    let team_paid = key(
        &f,
        &pool,
        &target.secret,
        10,
        Account {
            kind: AccountKind::Personal,
            id: 10,
        },
        FundingPolicy {
            version: 1,
            account_order: Some(vec![Account {
                kind: AccountKind::Team,
                id: team,
            }]),
            personal_billing_preference: None,
        },
    )
    .await;
    let team_account = f
        .identity
        .authorize(&team_owned.secret)
        .await
        .unwrap()
        .owner_account_id;
    let other_account = f
        .identity
        .authorize(&other.secret)
        .await
        .unwrap()
        .owner_account_id;
    for (id, payer) in [
        ("team-credit", team_account),
        ("other-credit", other_account),
    ] {
        ledger.open_wallet(payer).await.unwrap();
        ledger_action(
            &ledger,
            id,
            10,
            Action::Credit {
                account_id: payer,
                amount_units: Credits::try_from(1_000).unwrap(),
                payment_reference: format!("user-usage:{id}"),
            },
        )
        .await;
    }
    let engine = Charging::new(pool.clone(), Arc::new(TransactionalLedger));
    settle(
        &engine,
        &personal.secret,
        "usage-personal",
        20,
        Outcome::Completed,
    )
    .await;
    engine
        .refund(
            "usage-personal",
            "usage-refund",
            7,
            "Synthetic user-detail refund",
        )
        .await
        .unwrap();
    settle(
        &engine,
        &personal.secret,
        "usage-failed",
        5,
        Outcome::Failed,
    )
    .await;
    settle(
        &engine,
        &team_paid.secret,
        "usage-team-paid",
        30,
        Outcome::Completed,
    )
    .await;
    settle(
        &engine,
        &team_owned.secret,
        "usage-team-owned",
        40,
        Outcome::Completed,
    )
    .await;
    settle(
        &engine,
        &other_key.secret,
        "usage-other",
        50,
        Outcome::Completed,
    )
    .await;
    let mut tx = pool.begin().await.unwrap();
    sqlx::query("SET TRANSACTION ISOLATION LEVEL SERIALIZABLE")
        .execute(&mut *tx)
        .await
        .unwrap();
    sqlx::query("INSERT INTO core_charging.subscriptions(account_id,grant_reference,period,anchor,seconds,ends_at,limit_credits,models,groups) VALUES($1,'user-usage-subscription','custom',0,86400,253402300799,1000,ARRAY['test-model'],ARRAY['default'])")
        .bind(account).execute(&mut *tx).await.unwrap();
    sqlx::query("UPDATE core_billing.account_policies SET preference='subscription_first' WHERE account_id=$1")
        .bind(account).execute(&mut *tx).await.unwrap();
    tx.commit().await.unwrap();
    let subscribed = engine
        .reserve(&personal.secret, &spec("usage-subscription", 100), &WORKER)
        .await
        .unwrap();
    assert!(matches!(subscribed.source, Source::Subscription { .. }));
    engine.start("usage-subscription", &WORKER).await.unwrap();
    engine
        .settle(
            "usage-subscription",
            &WORKER,
            Usage {
                output: 15,
                ..Default::default()
            },
            Outcome::Completed,
        )
        .await
        .unwrap();
    engine
        .reserve(&personal.secret, &spec("usage-pending", 100), &WORKER)
        .await
        .unwrap();
    engine
        .reserve(&personal.secret, &spec("usage-released", 100), &WORKER)
        .await
        .unwrap();
    engine.release("usage-released", &WORKER).await.unwrap();

    let before = state(&pool).await;
    let result = usage(&f, 10).await;
    assert_eq!(
        result,
        json!({
            "status": "available",
            "unit": "core_credit",
            "scope": "actor_personal_usage",
            "historical_imports_included": false,
            "settled_requests": "4",
            "pending_requests": "1",
            "gross_settled_units": "70",
            "refunded_units": "7",
            "net_consumption_units": "63",
            "personal_wallet_consumption_units": "18",
            "team_wallet_consumption_units": "30",
            "subscription_consumption_units": "15",
            "sponsored_consumption_units": "0"
        })
    );
    assert_eq!(usage(&f, 11).await["net_consumption_units"], "50");
    assert_eq!(before, state(&pool).await);

    let mut tx = pool.begin().await.unwrap();
    sqlx::query("SET TRANSACTION ISOLATION LEVEL SERIALIZABLE")
        .execute(&mut *tx)
        .await
        .unwrap();
    sqlx::query("DELETE FROM core_charging.ledger_commands WHERE request_id='usage-personal' AND command->'change'->>'operation'='refund'")
        .execute(&mut *tx).await.unwrap();
    tx.commit().await.unwrap();
    assert!(matches!(
        f.store.user(&f.admin, 10).await,
        Err(AdminUsersError::Unavailable)
    ));
    f.role.cleanup().await;
}

#[sqlx::test(migrations = false)]
async fn exact_user_usage_retains_large_totals_across_receipt_batches(pool: PgPool) {
    let f = fresh(&pool).await;
    let target = person(&f, &pool, 10, 0, "usage-large").await;
    let (ledger, account) = fund(&f, &pool, 10).await;
    let issued = key(
        &f,
        &pool,
        &target.secret,
        10,
        Account {
            kind: AccountKind::Personal,
            id: 10,
        },
        FundingPolicy::new_personal_key(),
    )
    .await;
    let large = 4_503_599_627_370_497;
    ledger_action(
        &ledger,
        "large-usage-credit",
        10,
        Action::Credit {
            account_id: account,
            amount_units: Credits::try_from(large * 2).unwrap(),
            payment_reference: "large-user-usage".into(),
        },
    )
    .await;
    let engine = Charging::new(pool.clone(), Arc::new(TransactionalLedger));
    settle(
        &engine,
        &issued.secret,
        "usage-large-a",
        large,
        Outcome::Completed,
    )
    .await;
    settle(
        &engine,
        &issued.secret,
        "usage-large-b",
        large,
        Outcome::Completed,
    )
    .await;
    for index in 0..63 {
        settle(
            &engine,
            &issued.secret,
            &format!("usage-zero-{index}"),
            0,
            Outcome::Cancelled,
        )
        .await;
    }
    let result = usage(&f, 10).await;
    assert_eq!(result["settled_requests"], "65");
    assert_eq!(result["pending_requests"], "0");
    assert_eq!(result["gross_settled_units"], "9007199254740994");
    assert_eq!(result["net_consumption_units"], "9007199254740994");
    assert_eq!(
        result["personal_wallet_consumption_units"],
        "9007199254740994"
    );
    assert_eq!(result["refunded_units"], "0");
    assert_eq!(
        ledger.balance(account).await.unwrap().available_units,
        1_000
    );
    f.role.cleanup().await;
}

#[sqlx::test(migrations = false)]
async fn exact_user_usage_distinguishes_native_zero_unavailable_and_query_failure(pool: PgPool) {
    let f = fresh(&pool).await;
    person(&f, &pool, 10, 0, "usage-empty").await;
    let zero = usage(&f, 10).await;
    assert_eq!(zero["status"], "available");
    assert_eq!(zero["net_consumption_units"], "0");
    assert_eq!(zero["settled_requests"], "0");
    assert_eq!(zero["historical_imports_included"], false);
    sqlx::raw_sql(sqlx::AssertSqlSafe(format!(
        "REVOKE SELECT ON core_charging.usage_events FROM {}",
        f.role.name
    )))
    .execute(&pool)
    .await
    .unwrap();
    assert_eq!(
        usage(&f, 10).await,
        json!({
            "status": "unavailable",
            "code": "USAGE_PROJECTION_UNAVAILABLE"
        })
    );
    sqlx::raw_sql(sqlx::AssertSqlSafe(format!("GRANT SELECT ON core_charging.usage_events TO {}; REVOKE SELECT ON core_identity.api_key_settings FROM {}", f.role.name, f.role.name))).execute(&pool).await.unwrap();
    assert!(matches!(
        f.store.user(&f.admin, 10).await,
        Err(AdminUsersError::Unavailable)
    ));
    f.role.cleanup().await;
}

#[sqlx::test(migrations = false)]
async fn exact_user_usage_preserves_full_bigint_actor_after_real_settlement(pool: PgPool) {
    let user = 9_007_199_254_740_993_i64;
    let f = fresh(&pool).await;
    let target = person(&f, &pool, user, 0, "usage-bigint-actor").await;
    let (ledger, account) = fund(&f, &pool, user).await;
    let issued = key(
        &f,
        &pool,
        &target.secret,
        user,
        Account {
            kind: AccountKind::Personal,
            id: user,
        },
        FundingPolicy::new_personal_key(),
    )
    .await;
    let engine = Charging::new(pool.clone(), Arc::new(TransactionalLedger));
    settle(
        &engine,
        &issued.secret,
        "usage-bigint-actor",
        1,
        Outcome::Completed,
    )
    .await;
    // Establish actual accepted identity, charging and ledger facts before the
    // read. A regression must not be mistaken for a rejected setup operation.
    let settled: (i64, i64, i64, i64) = sqlx::query_as(
        "SELECT actor_user_id,owner_account_id,settled,refunded
         FROM core_charging.requests WHERE id='usage-bigint-actor' AND state='settled'",
    )
    .fetch_one(&pool)
    .await
    .unwrap();
    assert_eq!(settled, (user, account, 1, 0));
    assert_eq!(ledger.balance(account).await.unwrap().available_units, 999);

    let before = state(&pool).await;
    let detail = serde_json::to_value(
        f.store
            .user(&f.admin, user)
            .await
            .expect("exact user detail must preserve the settled BIGINT actor"),
    )
    .unwrap();
    assert_eq!(detail["user"]["id"], user.to_string());
    let result = &detail["usage"];
    assert_eq!(result["status"], "available");
    assert_eq!(result["scope"], "actor_personal_usage");
    assert_eq!(result["historical_imports_included"], false);
    assert_eq!(result["settled_requests"], "1");
    assert_eq!(result["pending_requests"], "0");
    assert_eq!(result["gross_settled_units"], "1");
    assert_eq!(result["refunded_units"], "0");
    assert_eq!(result["net_consumption_units"], "1");
    assert_eq!(result["personal_wallet_consumption_units"], "1");

    // The old administrator log response still projects numeric identifiers.
    // Its small-ID administrator selects the same large-ID target by username,
    // so the original public receipt constructor must still reject that row.
    let app = crate::usage::router(crate::usage::UsageStore::new(f.role.pool.clone()));
    let response = app
        .oneshot(
            Request::builder()
                .uri("/core/v1/usage/admin/logs?username=usage-bigint-actor")
                .header("authorization", format!("Bearer {}", f.admin))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::SERVICE_UNAVAILABLE);
    assert_eq!(response.headers()["cache-control"], "no-store");
    let bytes = to_bytes(response.into_body(), 64 * 1024).await.unwrap();
    let rejected: Value = serde_json::from_slice(&bytes).unwrap();
    assert_eq!(rejected["code"], "USAGE_UNAVAILABLE");
    assert!(rejected.get("data").is_none());
    assert_eq!(before, state(&pool).await);

    // Exact identities do not exempt the request from its immutable settlement
    // evidence. The same new read path must fail closed on a missing command.
    let mut tx = pool.begin().await.unwrap();
    sqlx::query("SET TRANSACTION ISOLATION LEVEL SERIALIZABLE")
        .execute(&mut *tx)
        .await
        .unwrap();
    let removed = sqlx::query(
        "DELETE FROM core_charging.ledger_commands
         WHERE request_id='usage-bigint-actor' AND command->'change'->>'operation'='settle'",
    )
    .execute(&mut *tx)
    .await
    .unwrap();
    assert_eq!(removed.rows_affected(), 1);
    tx.commit().await.unwrap();
    assert!(matches!(
        f.store.user(&f.admin, user).await,
        Err(AdminUsersError::Unavailable)
    ));
    f.role.cleanup().await;
}

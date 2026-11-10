// Copyright (C) 2026 LIghtJUNction. AGPL-3.0-or-later.
use super::*;
use crate::{core_schema, identity::IdentityStore};
use serde_json::json;

fn request(user_id: i64, end: i64) -> UserUsageRequest {
    UserUsageRequest {
        user_id,
        start: 0,
        end,
        bucket_seconds: 0,
        model_limit: 12,
    }
}

#[test]
fn lifetime_is_only_a_single_user_unbucketed_read() {
    let end = MAX_WINDOW + 1;
    assert!(request(71, end).validate().is_ok());
    assert_eq!(
        Window {
            start: 0,
            end,
            bucket_seconds: 0
        }
        .validate(),
        Err(Error::Invalid)
    );
    let mut bad = request(71, end);
    bad.bucket_seconds = 86_400;
    assert_eq!(bad.validate(), Err(Error::Invalid));
    bad = request(71, end);
    bad.start = 1;
    bad.end += 1;
    assert_eq!(bad.validate(), Err(Error::Invalid));
    for r in [
        request(0, end),
        request(71, -1),
        request(71, 253_402_300_800),
    ] {
        assert_eq!(r.validate(), Err(Error::Invalid));
    }
    let mut year = request(71, 1_800_000_000);
    year.start = year.end - 370 * 86_400;
    year.bucket_seconds = 86_400;
    assert!(year.validate().is_ok());
}

async fn source(pool: &PgPool, namespace: &str, complete: bool) -> i64 {
    sqlx::query_scalar("INSERT INTO core_history.sources(contract_version,source_archive_sha256,mapping_sha256,source_namespace,observed_at,complete) VALUES('core.history-source.v1',repeat('a',64),repeat('b',64),$1,1,$2) RETURNING source_id")
        .bind(namespace).bind(complete).fetch_one(pool).await.unwrap()
}

async fn owner(pool: &PgPool, user: i64) -> i64 {
    sqlx::query_scalar("SELECT personal_account_id FROM core_identity.users WHERE id=$1")
        .bind(user)
        .fetch_one(pool)
        .await
        .unwrap()
}

async fn aggregate(
    pool: &PgPool,
    source: i64,
    row: i64,
    user: i64,
    at: i64,
    tokens: i64,
    credits: i64,
) {
    sqlx::query("INSERT INTO core_history.quota_aggregates(source_id,source_row_id,source_row_sha256,user_id,owner_account_id,model_name,created_at,request_count,token_used,raw_quota,quota_credits,credits_basis) VALUES($1,$2,repeat('c',64),$3,$4,'past-model',$5,2,$6,$7,$7,'current_units')")
        .bind(source).bind(row).bind(user).bind(owner(pool,user).await).bind(at).bind(tokens).bind(credits).execute(pool).await.unwrap();
}

async fn log(
    pool: &PgPool,
    source: i64,
    row: i64,
    user: i64,
    at: i64,
    model: &str,
    tokens: i64,
    quota: Option<i64>,
    price: Option<i64>,
) {
    sqlx::query("INSERT INTO core_history.log_records(source_id,source_log_id,canonical_event_key,source_row_sha256,user_id,owner_account_id,created_at,log_type,prompt_tokens,completion_tokens,model_name,quota,pricing_credits_per_usd) VALUES($1,$2,$3,repeat('d',64),$4,$5,$6,2,$7,0,$8,$9,$10)")
        .bind(source).bind(row).bind(format!("app02:{source}:{row}")).bind(user).bind(owner(pool,user).await).bind(at).bind(tokens).bind(model).bind(quota).bind(price).execute(pool).await.unwrap();
}

// Actual charging/ledger lifecycle, not a fabricated settled row or mock total.
async fn native_charge(pool: &PgPool, user: i64) {
    use crate::{
        accounts::{Account, AccountKind},
        billing::{
            Credits,
            charging::{Charging, Outcome, Price, Request, Usage},
            ledger::{Action, Ledger, Request as LedgerRequest},
            transactional_ledger::TransactionalLedger,
        },
        funding::FundingPolicy,
    };
    let identity = IdentityStore::from_pool(pool.clone());
    let session = identity.bootstrap_user(user, 0).await.unwrap();
    let account = owner(pool, user).await;
    let ledger = Ledger::from_pool(pool.clone());
    ledger.open_wallet(account).await.unwrap();
    ledger
        .execute(&LedgerRequest {
            scope: "app02-fixture".into(),
            operation_key: format!("fund:{user}"),
            actor_user_id: Some(user),
            reason: "Private profile read test".into(),
            action: Action::Credit {
                account_id: account,
                amount_units: Credits::try_from(1000).unwrap(),
                payment_reference: format!("app02:fund:{user}"),
            },
        })
        .await
        .unwrap();
    let key = identity
        .issue_key(
            &session.secret,
            Account {
                kind: AccountKind::Personal,
                id: user,
            },
            FundingPolicy::new_personal_key(),
            3600,
        )
        .await
        .unwrap();
    sqlx::query("INSERT INTO core_identity.api_key_settings(credential_id,name,group_name) VALUES($1,'APP02 fixture','default')").bind(key.id).execute(pool).await.unwrap();
    sqlx::query("INSERT INTO core_identity.user_routing_groups(user_id,group_name,source_reference) VALUES($1,'default','app02-fixture')").bind(user).execute(pool).await.unwrap();
    let charging = Charging::new(pool.clone(), std::sync::Arc::new(TransactionalLedger));
    let id = format!("app02-native-{user}");
    let spec = Request {
        id: id.clone(),
        fingerprint: [3; 32],
        reserve: 100,
        lease_seconds: 300,
        maximum_seconds: 600,
        price: Price {
            rounding: None,
            tool: None,
            text_request: None,
            audio: None,
            grounding: None,
            user_trust: None,
            image: None,
            upstream_cost_group_ppm: None,
            tiered: None,
            version: "v1".into(),
            model: "native-model".into(),
            group: "default".into(),
            input_per_million: 1_000_000,
            cached_per_million: 1_000_000,
            output_per_million: 1_000_000,
        },
    };
    let worker = [9; 32];
    let usage = Usage {
        input: 10,
        output: 5,
        ..Default::default()
    };
    charging.reserve(&key.secret, &spec, &worker).await.unwrap();
    charging.start(&id, &worker).await.unwrap();
    charging
        .checkpoint(&id, &worker, "provider-final", usage)
        .await
        .unwrap();
    charging
        .settle(&id, &worker, usage, Outcome::Completed)
        .await
        .unwrap();
    charging
        .refund(&id, "app02-refund", 5, "Private fixture refund")
        .await
        .unwrap();
}

#[sqlx::test(migrations = false)]
async fn alltime_keeps_actor_scope_refunds_and_aggregate_precedence(pool: PgPool) {
    core_schema::init(&pool).await.unwrap();
    native_charge(&pool, 71).await;
    native_charge(&pool, 72).await;
    IdentityStore::from_pool(pool.clone())
        .bootstrap_user(73, 0)
        .await
        .unwrap();
    let end: i64 =
        sqlx::query_scalar("SELECT floor(extract(epoch FROM clock_timestamp()))::bigint+1")
            .fetch_one(&pool)
            .await
            .unwrap();
    let complete = source(&pool, "app02-complete", true).await;
    let incomplete = source(&pool, "app02-incomplete", false).await;
    aggregate(&pool, complete, 1, 71, 1, 30, 11).await;
    aggregate(&pool, complete, 2, 72, 1, 990, 999).await;
    aggregate(&pool, complete, 3, 71, end + 1, 770, 777).await;
    aggregate(&pool, incomplete, 1, 71, 1, 880, 888).await;
    // The same selected source's logs must not double count hourly aggregates.
    log(
        &pool,
        complete,
        1,
        71,
        1,
        "duplicate-log",
        5000,
        Some(5000),
        Some(500000),
    )
    .await;
    let read = CommunityRead::new(pool.clone()).await.unwrap();
    let all = read.user_usage(&request(71, end)).await.unwrap();
    assert!(all.active && all.pricing_available);
    assert_eq!(
        (all.totals.tokens, all.totals.requests, all.totals.credits),
        (45, 3, Some(21))
    );
    assert_eq!(all.history_source, "quota_aggregates");
    assert_eq!(all.model_count, 2);
    assert!(all.buckets.is_empty());
    let mut year = request(71, end);
    year.start = end - 370 * 86400;
    year.bucket_seconds = 86400;
    let year = read.user_usage(&year).await.unwrap();
    assert_eq!(
        (
            year.totals.tokens,
            year.totals.requests,
            year.totals.credits
        ),
        (15, 1, Some(10))
    );
    assert_eq!(year.buckets.len(), 1);
    let empty = read.user_usage(&request(73, end)).await.unwrap();
    assert!(empty.pricing_available);
    assert_eq!(
        (
            empty.totals.tokens,
            empty.totals.requests,
            empty.totals.credits
        ),
        (0, 0, Some(0))
    );
    // Existing site-wide endpoints keep the original window rejection.
    assert_eq!(
        read.model_buckets(&Window {
            start: 0,
            end,
            bucket_seconds: 0
        })
        .await
        .unwrap_err(),
        Error::Invalid
    );
    assert!(matches!(
        read.user_rankings(&UsersRequest {
            start: 0,
            end,
            limit: 10
        })
        .await,
        Err(Error::Invalid)
    ));
}

#[sqlx::test(migrations = false)]
async fn mixed_history_keeps_tokens_known_model_and_nullable_total_in_real_router(pool: PgPool) {
    use axum::{
        body::{Body, to_bytes},
        http::{Request, StatusCode},
    };
    use tower::ServiceExt;
    core_schema::init(&pool).await.unwrap();
    let identity = IdentityStore::from_pool(pool.clone());
    identity.bootstrap_user(71, 0).await.unwrap();
    identity.bootstrap_user(72, 0).await.unwrap();
    let end = MAX_WINDOW + 1;
    let complete = source(&pool, "app02-log-complete", true).await;
    let incomplete = source(&pool, "app02-log-incomplete", false).await;
    log(
        &pool,
        complete,
        1,
        71,
        1,
        "known",
        10,
        Some(100),
        Some(500000),
    )
    .await;
    log(
        &pool,
        complete,
        2,
        71,
        2,
        "unknown-price",
        2000,
        Some(999),
        None,
    )
    .await;
    log(&pool, complete, 3, 71, 3, "mixed", 3, Some(3), Some(500000)).await;
    log(&pool, complete, 4, 71, 4, "mixed", 4, None, Some(500000)).await;
    log(&pool, complete, 5, 71, 5, "free", 0, Some(0), Some(500000)).await;
    log(&pool, complete, 6, 72, 1, "other-owner", 20000, None, None).await;
    log(&pool, incomplete, 1, 71, 1, "incomplete", 20000, None, None).await;
    log(
        &pool,
        complete,
        7,
        71,
        end + 1,
        "after-window",
        20000,
        None,
        None,
    )
    .await;
    // Incomplete aggregate data must not hide the complete imported logs.
    aggregate(&pool, incomplete, 1, 71, 1, 880, 888).await;
    let read = CommunityRead::new(pool.clone()).await.unwrap();
    assert_eq!(read.history_source, "log_records");
    let all = read.user_usage(&request(71, end)).await.unwrap();
    assert_eq!(
        (all.totals.tokens, all.totals.requests, all.totals.credits),
        (2017, 5, None)
    );
    assert!(!all.pricing_available);
    assert_eq!(all.model_count, 4);
    assert_eq!(all.models[0].model_name, "unknown-price");
    assert_eq!(
        all.models
            .iter()
            .find(|m| m.model_name == "known")
            .unwrap()
            .credits,
        Some(100)
    );
    assert_eq!(
        all.models
            .iter()
            .find(|m| m.model_name == "mixed")
            .unwrap()
            .credits,
        None
    );
    assert_eq!(
        all.models
            .iter()
            .find(|m| m.model_name == "free")
            .unwrap()
            .credits,
        Some(0)
    );
    assert!(all.cny_per_usd.is_none());
    let known = read.user_usage(&request(71, 1)).await.unwrap();
    assert!(known.pricing_available);
    assert_eq!(known.totals.credits, Some(100));
    sqlx::query("SELECT core_console.write_display_exchange_rate('7.12500')")
        .execute(&pool)
        .await
        .unwrap();
    let displayed = read.user_usage(&request(71, end)).await.unwrap();
    assert_eq!(displayed.cny_per_usd.as_deref(), Some("7.12500"));
    assert_eq!(displayed.totals.credits, None);
    assert!(!displayed.pricing_available);
    // Legacy malformed display options are unknown, not a guessed/default FX.
    sqlx::query("UPDATE core_console.options SET value='NaN' WHERE key='USDExchangeRate'")
        .execute(&pool)
        .await
        .unwrap();
    assert!(
        read.user_usage(&request(71, end))
            .await
            .unwrap()
            .cny_per_usd
            .is_none()
    );
    let token = "app02-community-host-fixture-token";
    let app = super::super::http::router(read, token.as_bytes()).unwrap();
    for (auth, status) in [
        (
            "wrong-community-host-fixture-token",
            StatusCode::UNAUTHORIZED,
        ),
        (token, StatusCode::OK),
    ] {
        let response = app
            .clone()
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/internal/community/v1/usage/user")
                    .header("authorization", format!("Bearer {auth}"))
                    .header("x-lmm-protocol", "1")
                    .header("content-type", "application/json")
                    .body(Body::from(
                        json!({"user_id":71,"start":0,"end":end,"model_limit":12}).to_string(),
                    ))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), status);
        if status == StatusCode::OK {
            let value: serde_json::Value =
                serde_json::from_slice(&to_bytes(response.into_body(), 1 << 20).await.unwrap())
                    .unwrap();
            assert_eq!(value["totals"]["tokens"], 2017);
            assert!(value["totals"]["credits"].is_null());
            assert_eq!(value["pricing_available"], false);
            assert!(value["cny_per_usd"].is_null());
        }
    }
}

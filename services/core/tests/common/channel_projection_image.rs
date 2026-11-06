//! Independent image projection acceptance, included privately by usage module.
//! Real image admission/reserve/provider response/settle/refund; owner rebuild.
use crate::{
    accounts::{Account, AccountKind},
    billing::{
        Credits,
        charging::Charging,
        image_price::ImagePriceConfig,
        ledger::{Action, Ledger, Request as LedgerRequest},
        transactional_ledger::TransactionalLedger,
    },
    funding::FundingPolicy,
    identity::IdentityStore,
    model_http::image_execution::{DrawingInput, ImageService, ProviderSpec},
    relay::Limits,
    test_runtime as runtime,
};
use axum::{Json, Router, routing::post};
use serde_json::{Value, json};
use sqlx::PgPool;
use std::{
    sync::{
        Arc,
        atomic::{AtomicUsize, Ordering},
    },
    time::Duration,
};
use tokio::sync::{Notify, Semaphore};

async fn money(pool: &PgPool) -> Value {
    sqlx::query_scalar("SELECT jsonb_build_object(\
      'operations',(SELECT jsonb_agg(to_jsonb(t) ORDER BY scope,operation_key) FROM core_billing.ledger_operations t),\
      'journals',(SELECT jsonb_agg(to_jsonb(t) ORDER BY id) FROM core_billing.ledger_journals t),\
      'entries',(SELECT jsonb_agg(to_jsonb(t) ORDER BY journal_id,ledger_account_id) FROM core_billing.ledger_entries t),\
      'balances',(SELECT jsonb_agg(to_jsonb(t) ORDER BY ledger_account_id) FROM core_billing.balance_state t),\
      'commands',(SELECT jsonb_agg(to_jsonb(t) ORDER BY request_id,operation_id) FROM core_charging.ledger_commands t))")
        .fetch_one(pool).await.unwrap()
}
async fn projected(pool: &PgPool) -> Value {
    sqlx::query_scalar("SELECT jsonb_build_object(\
      'contributions',(SELECT jsonb_agg(to_jsonb(t) ORDER BY request_id,channel_id) FROM core_channels.usage_contributions t),\
      'rollups',(SELECT jsonb_agg(to_jsonb(t) ORDER BY channel_id,day,bucket) FROM core_channels.usage_rollups t),\
      'state',(SELECT jsonb_agg(to_jsonb(t)) FROM core_channels.projection_state t),\
      'dirty',(SELECT jsonb_agg(to_jsonb(t) ORDER BY request_id) FROM core_channels.projection_dirty t))")
        .fetch_one(pool).await.unwrap()
}
async fn contribution(pool: &PgPool) -> (String, String, i64, i64, i64) {
    sqlx::query_as("SELECT gross_credits::text,refunded_credits::text,settled_requests,pending_requests,unknown_requests FROM core_channels.usage_contributions WHERE channel_id=83")
        .fetch_one(pool).await.unwrap()
}

#[sqlx::test(migrations = false)]
async fn channel_projection_image_real_funding_replay_and_owner_rebuild(pool: PgPool) {
    crate::core_schema::init(&pool).await.unwrap();
    let identity = IdentityStore::from_pool(pool.clone());
    let session = identity.bootstrap_user(7, 1).await.unwrap();
    let actor = identity.authorize(&session.secret).await.unwrap();
    let key = identity
        .issue_key(
            &session.secret,
            Account {
                kind: AccountKind::Personal,
                id: 7,
            },
            FundingPolicy::new_personal_key(),
            3600,
        )
        .await
        .unwrap();
    let ledger = Ledger::from_pool(pool.clone());
    ledger.open_wallet(actor.owner_account_id).await.unwrap();
    ledger
        .execute(&LedgerRequest {
            scope: "projection-image".into(),
            operation_key: "synthetic-funds".into(),
            actor_user_id: Some(7),
            reason: "isolated image projection fixture".into(),
            action: Action::Credit {
                account_id: actor.owner_account_id,
                amount_units: Credits::try_from(10000).unwrap(),
                payment_reference: "synthetic:projection-image".into(),
            },
        })
        .await
        .unwrap();
    sqlx::query("INSERT INTO core_identity.user_routing_groups(user_id,group_name,source_reference) VALUES(7,'allowed','projection-image')").execute(&pool).await.unwrap();
    sqlx::query("INSERT INTO core_identity.api_key_settings(credential_id,name,group_name) VALUES($1,'projection image','allowed')").bind(key.id).execute(&pool).await.unwrap();
    let role = runtime::RuntimeRole::create(&pool).await;
    role.optional("core_charging").await;
    role.optional("core_image").await;
    let entered = Arc::new(Notify::new());
    let release = Arc::new(Semaphore::new(0));
    let hits = Arc::new(AtomicUsize::new(0));
    let provider=Router::new().route("/v1/images/generations",post({
        let entered=entered.clone();let release=release.clone();let hits=hits.clone();
        move |headers:axum::http::HeaderMap| {let entered=entered.clone();let release=release.clone();let hits=hits.clone();async move {
            hits.fetch_add(1,Ordering::SeqCst);assert_eq!(headers["authorization"],"Bearer synthetic-image-provider");entered.notify_one();release.acquire().await.unwrap().forget();
            Json(json!({"created":1,"data":[{"b64_json":"iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAQAAAC1HAwCAAAAC0lEQVR42mP8/x8AAwMCAO+/lZkAAAAASUVORK5CYII="}]}))
        }}
    }));
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let endpoint = format!("http://{}/v1", listener.local_addr().unwrap());
    let server = tokio::spawn(async move { axum::serve(listener, provider).await.unwrap() });
    let prices:ImagePriceConfig=serde_json::from_value(json!({"revision":1,"rules":[{"upstream_id":83,"vendor":"openai","billing_group":"allowed","upstream_model":"supplier-image","semantic_model":"local-image","endpoint":"generations","size":"1024x1024","quality":"standard","usd_micros_per_image":{"numerator":4000,"denominator":1},"group_multiplier":{"numerator":1,"denominator":1},"max_n":4,"count_basis":"returned_payloads","max_budget":10000,"source_revision":"synthetic-projection-image-v1"}]})).unwrap();
    let charging = Charging::new(role.pool.clone(), Arc::new(TransactionalLedger));
    let service = ImageService::new(
        role.pool.clone(),
        charging.clone(),
        prices,
        vec![ProviderSpec {
            upstream_id: 83,
            vendor: "openai".into(),
            upstream_model: "supplier-image".into(),
            base_url: endpoint,
            secret: "synthetic-image-provider".into(),
            local_http: true,
        }],
        &Limits::default(),
    )
    .unwrap();
    let input = DrawingInput {
        prompt: "projection image".into(),
        model: "local-image".into(),
        group: String::new(),
        size: "1024x1024".into(),
        quality: "standard".into(),
        n: 1,
    };
    let pending = tokio::spawn({
        let service = service.clone();
        let input = input.clone();
        let secret = key.secret.clone();
        async move {
            service
                .execute_api(
                    &secret,
                    &"d".repeat(64),
                    &["allowed".into()],
                    input,
                    None,
                    None,
                )
                .await
        }
    });
    tokio::time::timeout(Duration::from_secs(5), entered.notified())
        .await
        .unwrap();
    let dirty: i64 = sqlx::query_scalar("SELECT count(*) FROM core_channels.projection_dirty")
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(dirty, 0, "held Image contribution must not be stale");
    assert_eq!(
        contribution(&pool).await,
        ("0".into(), "0".into(), 0, 1, 0),
        "real reservation must be pending, not consumption"
    );
    release.add_permits(1);
    let receipt = pending.await.unwrap().unwrap();
    assert_eq!(receipt.charged_credits, Some(2000));
    assert_eq!(
        contribution(&pool).await,
        ("2000".into(), "0".into(), 1, 0, 0)
    );
    charging
        .refund(
            &receipt.operation_id,
            "image-correction",
            1,
            "isolated image correction",
        )
        .await
        .unwrap();
    assert_eq!(
        contribution(&pool).await,
        ("2000".into(), "1".into(), 1, 0, 0)
    );
    let available:i64=sqlx::query_scalar("SELECT balance_units FROM core_billing.balance_state b JOIN core_billing.ledger_accounts a ON a.id=b.ledger_account_id WHERE a.owner_account_id=$1 AND bucket='wallet'")
        .bind(actor.owner_account_id).fetch_one(&pool).await.unwrap();
    assert_eq!(
        available, 8001,
        "real funding must capture 2000 then refund 1"
    );
    let before_money = money(&pool).await;
    let before_projection = projected(&pool).await;
    let replay = service
        .execute_api(
            &key.secret,
            &"d".repeat(64),
            &["allowed".into()],
            input,
            None,
            None,
        )
        .await
        .unwrap();
    assert_eq!(replay.operation_id, receipt.operation_id);
    assert_eq!(hits.load(Ordering::SeqCst), 1);
    charging
        .refund(
            &receipt.operation_id,
            "image-correction",
            1,
            "isolated image correction",
        )
        .await
        .unwrap();
    assert_eq!(money(&pool).await, before_money);
    assert_eq!(projected(&pool).await, before_projection);
    assert_eq!(
        crate::core_schema::rebuild_channel_usage(&pool)
            .await
            .unwrap(),
        1
    );
    assert_eq!(
        money(&pool).await,
        before_money,
        "derived rebuild wrote funding ledger"
    );
    assert_eq!(
        projected(&pool).await,
        before_projection,
        "untrusted validator disagrees with live image contribution"
    );
    let mut c = role.pool.acquire().await.unwrap();
    let totals = super::super::channel::channel_totals(&mut c, 83)
        .await
        .unwrap();
    assert_eq!(totals.net_used_credits, "1999");
    assert_eq!(totals.unknown_provenance_requests, "0");
    assert!(!totals.historical_coverage);
    drop(c);
    server.abort();
    role.cleanup().await;
}

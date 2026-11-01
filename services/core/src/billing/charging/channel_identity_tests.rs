//! Real image admission under a runtime role without core_model privileges.
use crate::test_runtime as runtime;

use crate::{
    billing::{charging::Charging, transactional_ledger::TransactionalLedger},
    model_http::{
        image_execution::{DrawingInput, Error, ImageService, ProviderSpec},
        image_price::ImagePriceConfig,
    },
    relay::Limits,
};
use axum::{Json, Router, routing::post};
use serde_json::{Value, json};
use sqlx::PgPool;
use std::sync::{
    Arc,
    atomic::{AtomicUsize, Ordering},
};
use tokio::{
    net::TcpListener,
    sync::{Notify, Semaphore},
};

async fn financial_snapshot(c: &PgPool) -> Value {
    sqlx::query_scalar("SELECT jsonb_build_object(\
        'commands',(SELECT jsonb_agg(to_jsonb(c) ORDER BY request_id,operation_id) FROM core_charging.ledger_commands c),\
        'operations',(SELECT jsonb_agg(to_jsonb(o) ORDER BY scope,operation_key) FROM core_billing.ledger_operations o),\
        'journals',(SELECT jsonb_agg(to_jsonb(j) ORDER BY id) FROM core_billing.ledger_journals j),\
        'entries',(SELECT jsonb_agg(to_jsonb(e) ORDER BY journal_id,ledger_account_id) FROM core_billing.ledger_entries e),\
        'balances',(SELECT jsonb_agg(to_jsonb(b) ORDER BY ledger_account_id) FROM core_billing.balance_state b))")
        .fetch_one(c).await.unwrap()
}

#[sqlx::test(migrations = false)]
async fn image_admission_reserves_its_binding_and_settles_after_retirement(pool: PgPool) {
    use crate::{
        accounts::{Account, AccountKind},
        billing::{
            Credits,
            ledger::{Action, Ledger, Request},
        },
        funding::FundingPolicy,
        identity::IdentityStore,
    };
    crate::core_schema::init(&pool).await.unwrap();
    let identity = IdentityStore::from_pool(pool.clone());
    let session = identity.bootstrap_user(7, 1).await.unwrap();
    let actor = identity.authorize(&session.secret).await.unwrap();
    let issued = identity
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
    let key_id = issued.id;
    let key = issued.secret;
    let ledger = Ledger::from_pool(pool.clone());
    ledger.open_wallet(actor.owner_account_id).await.unwrap();
    ledger
        .execute(&Request {
            scope: "image-identity-test".into(),
            operation_key: "synthetic-funds".into(),
            actor_user_id: Some(7),
            reason: "Isolated image admission fixture".into(),
            action: Action::Credit {
                account_id: actor.owner_account_id,
                amount_units: Credits::try_from(1_000_000).unwrap(),
                payment_reference: "synthetic:image-identity".into(),
            },
        })
        .await
        .unwrap();
    sqlx::query("INSERT INTO core_identity.user_routing_groups(user_id,group_name,source_reference) VALUES(7,'allowed','image-identity-test')")
        .execute(&pool).await.unwrap();
    sqlx::query("INSERT INTO core_identity.api_key_settings(credential_id,name,group_name) VALUES($1,'image identity key','allowed')")
        .bind(key_id).execute(&pool).await.unwrap();
    let role = runtime::RuntimeRole::create(&pool).await;
    role.optional("core_charging").await;
    role.optional("core_image").await;
    let core_model_allowed: bool =
        sqlx::query_scalar("SELECT has_schema_privilege(current_user,'core_model','USAGE')")
            .fetch_one(&role.pool)
            .await
            .unwrap();
    assert!(!core_model_allowed);
    let entered = Arc::new(Notify::new());
    let release = Arc::new(Semaphore::new(0));
    let hits = Arc::new(AtomicUsize::new(0));
    let app = Router::new().route("/v1/images/generations",post({
        let entered=entered.clone(); let release=release.clone(); let hits=hits.clone();
        move || { let entered=entered.clone(); let release=release.clone(); let hits=hits.clone(); async move {
            hits.fetch_add(1,Ordering::SeqCst);
            entered.notify_one();
            release.acquire().await.unwrap().forget();
            Json(json!({"created":1,"data":[{"b64_json":"iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAQAAAC1HAwCAAAAC0lEQVR42mP8/x8AAwMCAO+/lZkAAAAASUVORK5CYII="}]}))
        }}
    }));
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let endpoint = format!("http://{}/v1", listener.local_addr().unwrap());
    let server = tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
    let prices: ImagePriceConfig = serde_json::from_value(json!({"revision":1,"rules":[{
        "upstream_id":83,"vendor":"openai","billing_group":"allowed","upstream_model":"supplier-image",
        "semantic_model":"local-model","endpoint":"generations","size":"1024x1024","quality":"standard",
        "usd_micros_per_image":{"numerator":4000,"denominator":1},
        "group_multiplier":{"numerator":1,"denominator":1},"max_n":4,"count_basis":"returned_payloads",
        "max_budget":10000,"source_revision":"synthetic-image-v1"}]})).unwrap();
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
            secret: "synthetic-image-key".into(),
            local_http: true,
        }],
        &Limits::default(),
    )
    .unwrap();
    let input = DrawingInput {
        prompt: "synthetic image".into(),
        model: "local-model".into(),
        group: String::new(),
        size: "1024x1024".into(),
        quality: "standard".into(),
        n: 1,
    };
    let request = tokio::spawn({
        let service = service.clone();
        let key = key.clone();
        let input = input.clone();
        async move {
            service
                .execute_api(
                    &key,
                    &"a".repeat(64),
                    &["allowed".into()],
                    input,
                    None,
                    None,
                )
                .await
        }
    });
    tokio::time::timeout(std::time::Duration::from_secs(5), entered.notified())
        .await
        .unwrap();
    let identities: Vec<(i64,String,bool,String)> = sqlx::query_as("SELECT i.id,i.origin,b.historical_coverage,o.kind FROM core_channels.identities i JOIN core_channels.counter_baselines b ON b.channel_id=i.id JOIN core_channels.balance_observations o ON o.channel_id=i.id ORDER BY i.id")
        .fetch_all(&pool).await.unwrap();
    assert_eq!(
        identities,
        vec![(83, "bootstrap".into(), false, "unknown".into())]
    );
    sqlx::query("UPDATE core_channels.identities SET retired_at=clock_timestamp() WHERE id=83")
        .execute(&pool)
        .await
        .unwrap();
    release.add_permits(1);
    let receipt = request.await.unwrap().unwrap();
    assert_eq!(receipt.charged_credits, Some(2000));
    charging
        .refund(
            &receipt.operation_id,
            "after-retirement",
            1,
            "synthetic image correction",
        )
        .await
        .unwrap();
    let before = financial_snapshot(&pool).await;
    assert_eq!(
        service
            .execute_api(
                &key,
                &"b".repeat(64),
                &["allowed".into()],
                input,
                None,
                None
            )
            .await
            .unwrap_err(),
        Error::Conflict
    );
    assert_eq!(hits.load(Ordering::SeqCst), 1);
    assert_eq!(financial_snapshot(&pool).await, before);
    server.abort();
    role.cleanup().await;
}

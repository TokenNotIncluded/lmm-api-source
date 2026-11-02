//! Opt-in local integration against a disposable clone of a completed owner import.
//! This exercises real Charging, not fabricated ledger or projection rows.
use axum::{
    Json, Router,
    body::{Body, to_bytes},
    http::{Request as HttpRequest, StatusCode},
    routing::post,
};
use lmm_core::{
    accounts::{Account, AccountKind},
    billing::{
        Credits,
        charging::{Charging, Outcome, Price, Request, Usage},
        ledger::{Action, Ledger, Request as LedgerRequest},
        transactional_ledger::TransactionalLedger,
    },
    funding::FundingPolicy,
    identity::IdentityStore,
    model_http::{ChargePolicy, ModelGateway, ProviderPolicy, UpstreamCredential},
    relay::Limits,
    routing::{CredentialRef, Endpoint, PriceConfig, PriceSnapshot, RoutingConfig},
};
use serde_json::{Value, json};
use sqlx::{PgPool, postgres::PgPoolOptions};
use std::{collections::BTreeMap, sync::Arc, time::Duration};
use tower::ServiceExt;

async fn opening_snapshot(pool: &PgPool) -> Value {
    sqlx::query_scalar("SELECT jsonb_build_object('sources',(SELECT jsonb_agg(to_jsonb(s) ORDER BY source_key) FROM core_channels.opening_sources s),'baselines',(SELECT jsonb_agg(to_jsonb(b) ORDER BY channel_id) FROM core_channels.opening_baselines b))")
        .fetch_one(pool).await.unwrap()
}

#[tokio::test]
#[ignore = "requires an explicitly named disposable clone of an imported synthetic fixture"]
async fn imported_opening_survives_real_charging_and_opaque_id_growth() {
    let url = std::env::var("LMM_OPENING_LIVE_DATABASE_URL").expect("private owned fixture URL");
    let name = std::env::var("LMM_OPENING_LIVE_DATABASE_NAME").expect("explicit fixture name");
    assert!(name.starts_with("co_live_"), "refuse non-fixture database");
    let pool = PgPoolOptions::new()
        .max_connections(4)
        .connect(&url)
        .await
        .unwrap();
    let actual: String = sqlx::query_scalar("SELECT current_database()")
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(actual, name);
    let local_socket_only: bool = sqlx::query_scalar(
        "SELECT inet_server_addr() IS NULL AND current_setting('listen_addresses')=''",
    )
    .fetch_one(&pool)
    .await
    .unwrap();
    assert!(
        local_socket_only,
        "refuse a network-listening PostgreSQL server"
    );
    let before = opening_snapshot(&pool).await;
    assert!(before["sources"].as_array().is_some_and(|x| !x.is_empty()));
    assert!(
        before["baselines"]
            .as_array()
            .is_some_and(|x| !x.is_empty())
    );
    let prior_ids: Vec<String> =
        sqlx::query_scalar("SELECT id FROM core_charging.requests ORDER BY id")
            .fetch_all(&pool)
            .await
            .unwrap();
    assert!(!prior_ids.is_empty());
    let sealed_ids = before["sources"][0]["native_request_ids"]
        .as_array()
        .unwrap();
    let sealed_ids: Vec<&str> = sealed_ids.iter().map(|v| v.as_str().unwrap()).collect();
    assert_eq!(sealed_ids.len(), prior_ids.len());
    assert!(prior_ids.iter().all(|id| sealed_ids.contains(&id.as_str())));
    let price: Value =
        sqlx::query_scalar("SELECT price FROM core_charging.requests ORDER BY id LIMIT 1")
            .fetch_one(&pool)
            .await
            .unwrap();
    let identity = IdentityStore::from_pool(pool.clone());
    let session = identity.bootstrap_user(700001, 1).await.unwrap();
    let principal = identity.authorize(&session.secret).await.unwrap();
    let key = identity
        .issue_key(
            &session.secret,
            Account {
                kind: AccountKind::Personal,
                id: 700001,
            },
            FundingPolicy::new_personal_key(),
            3600,
        )
        .await
        .unwrap();
    let ledger = Ledger::from_pool(pool.clone());
    ledger
        .open_wallet(principal.owner_account_id)
        .await
        .unwrap();
    ledger
        .execute(&LedgerRequest {
            scope: "opening-live-test".into(),
            operation_key: "synthetic-funds".into(),
            actor_user_id: Some(700001),
            reason: "isolated synthetic fixture".into(),
            action: Action::Credit {
                account_id: principal.owner_account_id,
                amount_units: Credits::try_from(1000000).unwrap(),
                payment_reference: "synthetic:opening-live".into(),
            },
        })
        .await
        .unwrap();
    let charging = Charging::new(pool.clone(), Arc::new(TransactionalLedger));
    let request = Request {
        id: "opening:opaque:post-import-1".into(),
        fingerprint: [91; 32],
        reserve: 41,
        lease_seconds: 60,
        maximum_seconds: 120,
        price: serde_json::from_value::<Price>(price).unwrap(),
    };
    let worker = [92; 32];
    charging
        .reserve(&key.secret, &request, &worker)
        .await
        .unwrap();
    charging.start(&request.id, &worker).await.unwrap();
    charging
        .settle(
            &request.id,
            &worker,
            Usage {
                input: 40,
                output: 1,
                ..Default::default()
            },
            Outcome::Completed,
        )
        .await
        .unwrap();
    charging
        .refund(
            &request.id,
            "opening-refund",
            5,
            "isolated synthetic correction",
        )
        .await
        .unwrap();
    let ids: Vec<String> = sqlx::query_scalar("SELECT id FROM core_charging.requests ORDER BY id")
        .fetch_all(&pool)
        .await
        .unwrap();
    assert_eq!(ids.len(), prior_ids.len() + 1);
    assert!(prior_ids.iter().all(|id| ids.contains(id)));
    assert!(ids.contains(&request.id));
    assert_eq!(
        opening_snapshot(&pool).await,
        before,
        "real funding changed sealed opening facts"
    );
    let global_unknown: i64 = sqlx::query_scalar("SELECT count(*) FROM core_channels.usage_contributions WHERE request_id=$1 AND channel_id=0 AND unknown_requests=1")
        .bind(&request.id).fetch_one(&pool).await.unwrap();
    assert_eq!(
        global_unknown, 1,
        "generic Charging has no channel attribution"
    );
    let wallet: i64 = sqlx::query_scalar("SELECT balance_units FROM core_billing.balance_state b JOIN core_billing.ledger_accounts a ON a.id=b.ledger_account_id WHERE a.owner_account_id=$1 AND a.bucket='wallet'")
        .bind(principal.owner_account_id).fetch_one(&pool).await.unwrap();
    assert_eq!(wallet, 999964);

    // Exercise the actual model gateway against a channel whose opening was
    // imported by the owner workflow. The upstream is a synthetic loopback
    // HTTP service, never an external provider.
    sqlx::query("INSERT INTO core_identity.user_routing_groups(user_id,group_name,source_reference) VALUES(700001,'allowed','opening-live')")
        .execute(&pool).await.unwrap();
    sqlx::query("INSERT INTO core_identity.api_key_settings(credential_id,name,group_name) VALUES($1,'opening-live','allowed')")
        .bind(key.id).execute(&pool).await.unwrap();
    let provider = Router::new().route("/v1/chat/completions", post(|| async {
        Json(json!({"id":"synthetic-opening","object":"chat.completion","created":0,"model":"provider-model","choices":[{"index":0,"message":{"role":"assistant","content":"opening proof"},"finish_reason":"stop"}],"usage":{"prompt_tokens":40,"completion_tokens":1,"total_tokens":41,"prompt_tokens_details":{"cached_tokens":0}}}))
    }));
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let endpoint = format!("http://{}/v1/", listener.local_addr().unwrap());
    let server = tokio::spawn(async move { axum::serve(listener, provider).await.unwrap() });
    let mut config: RoutingConfig = serde_json::from_value(json!({"version":1,"groups":[{"name":"allowed","enabled":true}],"upstreams":[{"id":880001,"endpoint":"https://provider.example/v1/","protocol":"openai_chat","credential":{"id":771,"revision":1},"enabled":true}],"routes":[{"id":880001,"model":"opening-model","group":"allowed","enabled":true,"speeds":["standard"],"targets":[{"id":880001,"upstream_id":880001,"upstream_model":"provider-model","enabled":true,"priority":0,"weight":1,"speeds":["standard"],"payload":"adapt","timeout_ms":5000}],"failover":{"max_attempts":1,"retry_on":[],"allow_replay_after_send":false}}],"aliases":[]})).unwrap();
    config.upstreams[0].endpoint = Endpoint::local_loopback(endpoint).unwrap();
    let prices: PriceConfig = serde_json::from_value(json!({"version":1,"rules":[{"id":880001,"model":"opening-model","group":"allowed","speed":"standard","rates":{"input":2000000,"output":2000000,"cache_read":2000000,"cache_write":0,"request":0},"group_multiplier_ppm":1000000,"speed_multiplier_ppm":1000000}]})).unwrap();
    let router = Arc::new(
        lmm_core::routing::Router::new(config, Arc::new(PriceSnapshot::compile(prices).unwrap()))
            .unwrap(),
    );
    let gateway = ModelGateway::new_with_provider_policy(
        pool.clone(),
        router,
        vec![UpstreamCredential {
            reference: CredentialRef {
                id: 771,
                revision: 1,
            },
            secret: "synthetic-opening-provider".into(),
        }],
        BTreeMap::from([(key.id, "allowed".into())]),
        Limits {
            total_timeout: Duration::from_secs(3),
            max_output_tokens: 64,
            ..Limits::default()
        },
        ChargePolicy {
            input_token_ceiling: 1000,
            lease_seconds: 60,
        },
        ProviderPolicy::new(Vec::new(), 1000).unwrap(),
    )
    .unwrap()
    .with_local_streaming();
    let response = gateway.router().oneshot(HttpRequest::builder().method("POST").uri("/v1/chat/completions").header("authorization",format!("Bearer {}",key.secret)).header("content-type","application/json").header("idempotency-key","opening-attributed").body(Body::from(json!({"model":"opening-model","messages":[{"role":"user","content":"hello"}],"max_tokens":32}).to_string())).unwrap()).await.unwrap();
    let status = response.status();
    let attributed_id = response
        .headers()
        .get("x-request-id")
        .unwrap()
        .to_str()
        .unwrap()
        .to_owned();
    let body = to_bytes(response.into_body(), 1 << 20).await.unwrap();
    assert_eq!(status, StatusCode::OK, "{}", String::from_utf8_lossy(&body));
    assert_eq!(
        serde_json::from_slice::<Value>(&body).unwrap()["choices"][0]["message"]["content"],
        "opening proof"
    );
    charging
        .refund(
            &attributed_id,
            "opening-attributed-refund",
            5,
            "isolated synthetic correction",
        )
        .await
        .unwrap();
    let contribution: (String,String,i64) = sqlx::query_as("SELECT gross_credits::text,refunded_credits::text,settled_requests FROM core_channels.usage_contributions WHERE request_id=$1 AND channel_id=880001")
        .bind(&attributed_id).fetch_one(&pool).await.unwrap();
    assert_eq!(contribution, ("41".into(), "5".into(), 1));
    assert_eq!(
        opening_snapshot(&pool).await,
        before,
        "attributed funding changed imported opening facts"
    );
    let count: i64 = sqlx::query_scalar("SELECT count(*) FROM core_charging.requests")
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(count, i64::try_from(prior_ids.len()).unwrap() + 2);
    assert_ne!(attributed_id, request.id);
    server.abort();
    pool.close().await;
}

//! Manual cloud gate for one browser-reviewed publication and one real wallet capture.
use super::support;
use axum::{
    Json, Router,
    extract::Request,
    http::{HeaderMap, header},
    routing::{get, post},
};
use lmm_core::{
    channel_http::{ManagedChannels, managed_router},
    http::{Capabilities, PublicStatus, configured_router_with_rpc},
    legacy_auth::{self, LegacyAuthConfig},
    model_http::{ChargePolicy, ModelGateway, ProviderPolicy, UpstreamCredential},
    relay::Limits,
    routing::{Endpoint, PriceSnapshot, Router as ServingRouter},
};
use serde_json::{Value, json};
use sqlx::PgPool;
use std::{
    collections::BTreeMap,
    net::SocketAddr,
    process::Command,
    sync::{
        Arc,
        atomic::{AtomicUsize, Ordering},
    },
    time::Duration,
};
use tokio::net::TcpListener;
use tower::ServiceExt;

#[path = "../../tests/common/model_fixture.rs"]
mod model_fixture;

const PUBLIC_ENDPOINT: &str = "https://provider.example/v1/";
const PASSWORD: &str = "Browser-QA-27!";
const WEB_ORIGIN: &str = "http://127.0.0.1:41781";

async fn supplier(hits: Arc<AtomicUsize>, model: String, sibling: String) -> SocketAddr {
    let catalog = json!({"data":[
        {"id":sibling,"pricing":{"prompt":"0.000099","completion":"0.000099"}},
        {"id":model,"pricing":{"prompt":"0.000002","completion":"0.000004",
            "input_cache_read":"0","input_cache_write":"0","request":"0"}}
    ]});
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let app = Router::new()
        .route(
            "/v1/models",
            get(move |headers: HeaderMap| {
                let catalog = catalog.clone();
                async move {
                    assert_eq!(headers[header::AUTHORIZATION], "Bearer synthetic-channel-only");
                    Json(catalog)
                }
            }),
        )
        .route(
            "/v1/chat/completions",
            post(move |headers: HeaderMap, Json(body): Json<Value>| {
                let hits = hits.clone();
                let model = model.clone();
                async move {
                    assert_eq!(headers[header::AUTHORIZATION], "Bearer synthetic-channel-only");
                    assert_eq!(body["model"], model.as_str());
                    assert_eq!(body["stream"], true);
                    assert_eq!(hits.fetch_add(1, Ordering::SeqCst), 0);
                    let chunk = |choices: Value, usage: Option<Value>| {
                        let mut frame = json!({"id":"synthetic-stream","object":"chat.completion.chunk",
                            "created":1,"model":model,"choices":choices});
                        if let Some(usage) = usage {
                            frame["usage"] = usage;
                        }
                        format!("data: {frame}\n\n")
                    };
                    let wire = [
                        chunk(json!([{"index":0,"delta":{"role":"assistant","content":"browser fixture answer"},"finish_reason":null}]), None),
                        chunk(json!([{"index":0,"delta":{},"finish_reason":"stop"}]), None),
                        chunk(json!([]), Some(json!({"prompt_tokens":10,"completion_tokens":20,"total_tokens":30,
                            "prompt_tokens_details":{"cached_tokens":0,"cache_write_tokens":0}}))),
                        "data: [DONE]\n\n".to_owned(),
                    ].concat();
                    ([(header::CONTENT_TYPE, "text/event-stream")], wire)
                }
            }),
        );
    tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
    address
}

// The regular catalog host deliberately rejects non-public supplier origins.
// Follow native_model_http/official_publication's isolated transport fixture:
// recover the exact durable release, substitute only its one TCP destination,
// and dispatch the original HTTP request through ModelGateway on the same DB.
async fn paid_gateway(pool: &PgPool, address: SocketAddr, model: &str) -> ModelGateway {
    let mut release = support::active(pool).await;
    let bindings: Value = sqlx::query_scalar("SELECT public.routing_control_bindings($1)")
        .bind(release.routing.version as i64)
        .fetch_one(pool)
        .await
        .unwrap();
    assert_eq!(release.routing.upstreams.len(), 1);
    let channel_id = release.routing.upstreams[0].id;
    let context = bindings["contexts"]
        .as_array()
        .unwrap()
        .iter()
        .find(|c| c["upstream_id"] == channel_id && c["upstream_model"] == model)
        .unwrap();
    assert_eq!(context["endpoint"], PUBLIC_ENDPOINT);
    assert_eq!(context["contract_revision"], "browser-v1");
    let input = context["max_input_tokens"].as_i64().unwrap();
    let output = context["max_output_tokens"].as_u64().unwrap();
    assert_eq!((input, output), (100, 20));
    let mut expected = serde_json::to_value(&release).unwrap();
    let upstream = &mut release.routing.upstreams[0];
    assert_eq!(upstream.endpoint.as_str(), PUBLIC_ENDPOINT);
    assert_eq!(
        (upstream.credential.id, upstream.credential.revision),
        (771, 1)
    );
    let reference = upstream.credential;
    let local = format!("http://{address}/v1/");
    upstream.endpoint = Endpoint::local_loopback(local.clone()).unwrap();
    expected["routing"]["upstreams"][0]["endpoint"] = json!(local);
    assert_eq!(serde_json::to_value(&release).unwrap(), expected);
    let routing = ServingRouter::new(
        release.routing,
        Arc::new(PriceSnapshot::compile(release.pricing).unwrap()),
    )
    .unwrap();
    ModelGateway::new_with_provider_policy(
        pool.clone(),
        Arc::new(routing),
        vec![UpstreamCredential {
            reference,
            secret: "synthetic-channel-only".into(),
        }],
        BTreeMap::new(),
        Limits {
            total_timeout: Duration::from_secs(5),
            hook_timeout: Duration::from_secs(2),
            max_output_tokens: output.try_into().unwrap(),
            ..Default::default()
        },
        ChargePolicy {
            input_token_ceiling: input,
            lease_seconds: 60,
        },
        ProviderPolicy::new(Vec::new(), input).unwrap(),
    )
    .unwrap()
    .with_local_streaming()
}

#[sqlx::test(migrations = false)]
#[ignore = "requires the built web app and cloud Chromium"]
async fn browser_onboarding_wallet(pool: PgPool) {
    let (identity, ledger, _, _, _, account) = model_fixture::fixture(&pool, 1000).await;
    identity.bootstrap_user(8, 6).await.unwrap();
    lmm_core::console_compat::record_initialization(&pool, env!("CARGO_PKG_VERSION"))
        .await
        .unwrap();
    let hash = identity
        .hash_installer_password(PASSWORD.into())
        .await
        .unwrap();
    for (id, name) in [(7_i64, "browserpayer"), (8, "browseradmin")] {
        sqlx::query("UPDATE core_identity.user_profiles SET username=$2,display_name=$2,password_login_allowed=TRUE WHERE user_id=$1")
            .bind(id).bind(name).execute(&pool).await.unwrap();
        sqlx::query("INSERT INTO core_identity.password_logins(login_name,user_id,password_hash) VALUES ($1,$2,$3)")
            .bind(name).bind(id).bind(&hash).execute(&pool).await.unwrap();
    }
    let nonce: String =
        sqlx::query_scalar("SELECT left(replace(gen_random_uuid()::text,'-',''),12)")
            .fetch_one(&pool)
            .await
            .unwrap();
    let group = format!("plus-{nonce}");
    let other_group = format!("pro-{nonce}");
    let model = format!("intake-{nonce}");
    let sibling = format!("sibling-{nonce}");
    let scenario = json!({"model":model,"sibling":sibling,"group":group,"otherGroup":other_group});
    sqlx::query("INSERT INTO core_identity.user_routing_groups(user_id,group_name,enabled,source_reference) VALUES(7,$1,true,'synthetic:browser')")
        .bind(&group).execute(&pool).await.unwrap();

    let hits = Arc::new(AtomicUsize::new(0));
    let provider = supplier(hits.clone(), model.clone(), sibling).await;
    let live = support::gateway(&pool)
        .with_channel_secret_key(&[83; 32])
        .unwrap()
        .with_local_streaming()
        .with_balance_probe_loopback("https://provider.example", &format!("http://{provider}/"))
        .unwrap();
    let identity = identity
        .with_financial_profile_enabled(true)
        .with_dashboard_routing(live.dashboard_routing());
    let management = ManagedChannels::recover(pool.clone(), live).await.unwrap();
    let auth = LegacyAuthConfig::new(WEB_ORIGIN, &[43; 32])
        .unwrap()
        .with_insecure_local_http()
        .unwrap();
    let db = pool.clone();
    let served_model = model.clone();
    let dispatch = move |request: Request| {
        let db = db.clone();
        let model = served_model.clone();
        async move {
            paid_gateway(&db, provider, &model)
                .await
                .router()
                .oneshot(request)
                .await
                .unwrap()
        }
    };
    let paid = Router::new()
        .route("/v1/models", get(dispatch.clone()))
        .route("/v1/chat/completions", post(dispatch));
    let app = configured_router_with_rpc(
        pool.clone(),
        Capabilities {
            identity: true,
            legacy_web_auth: true,
            console: true,
            models: true,
            ..Default::default()
        },
        None,
        PublicStatus {
            password_login_enabled: true,
            ..Default::default()
        },
    )
    .merge(lmm_core::console_compat::router(
        lmm_core::console_compat::ConsoleCompat::new(pool.clone(), identity.clone()),
    ))
    .merge(legacy_auth::router(identity, auth))
    .merge(managed_router(pool.clone(), management))
    .merge(paid);
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let core_url = format!("http://{}", listener.local_addr().unwrap());
    tokio::spawn(async move {
        axum::serve(
            listener,
            app.into_make_service_with_connect_info::<SocketAddr>(),
        )
        .await
        .unwrap()
    });

    let output = tokio::task::spawn_blocking(move || {
        Command::new("node")
            .arg(concat!(
                env!("CARGO_MANIFEST_DIR"),
                "/../../tests/console/browser_acceptance/onboarding-settlement.mjs"
            ))
            .env("BROWSER_CORE_URL", core_url)
            .env("BROWSER_ACCOUNT_PASSWORD", PASSWORD)
            .env("BROWSER_SCENARIO", scenario.to_string())
            .output()
            .unwrap()
    })
    .await
    .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let mut proof: Value = serde_json::from_slice(&output.stdout).unwrap();
    let terminal = &proof["terminal"];
    let audited: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM core_identity.audit WHERE actor_user_id=8 AND action='channel.configuration.publish' AND target->'change'->>'reason'='missing-price' AND target->'change'->>'confirmation_digest'=$1",
    )
    .bind(terminal["confirmation_digest"].as_str().unwrap())
    .fetch_one(&pool)
    .await
    .unwrap();
    assert_eq!(audited, 1);
    let synced: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM core_identity.audit WHERE actor_user_id=8 AND action='channel.models.synchronize' AND target->'change'->>'reason'='catalog-refresh'",
    )
    .fetch_one(&pool)
    .await
    .unwrap();
    assert_eq!(synced, 2);
    let payment = &proof["payment"];
    let id = payment["request_id"].as_str().unwrap();
    let key_id = payment["key_id"].as_i64().unwrap();
    assert_eq!(payment["stream_terminal"], true);
    assert_eq!(proof["publication"]["group"], group.as_str());
    assert_eq!(proof["publication"]["other_group"], other_group.as_str());
    assert_eq!(hits.load(Ordering::SeqCst), 1);
    let settled_for_key: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM core_charging.requests WHERE actor_user_id=7 AND key_id=$1 AND state='settled'",
    )
    .bind(key_id)
    .fetch_one(&pool)
    .await
    .unwrap();
    assert_eq!(settled_for_key, 1);
    let settled: (String, i64, i64, i64, i64, i64, Value) = sqlx::query_as(
        "SELECT state,actor_user_id,key_id,settled,input_tokens,output_tokens,price FROM core_charging.requests WHERE id=$1",
    )
    .bind(id).fetch_one(&pool).await.unwrap();
    assert_eq!(
        (
            settled.0.as_str(),
            settled.1,
            settled.2,
            settled.3,
            settled.4,
            settled.5
        ),
        ("settled", 7, key_id, 50, 10, 20)
    );
    let release = support::active(&pool).await;
    assert_eq!(
        proof["publication"]["version"]["routing"],
        release.routing.version
    );
    assert_eq!(
        proof["publication"]["version"]["pricing"],
        release.pricing.version
    );
    let published_groups: Vec<_> = release
        .routing
        .routes
        .iter()
        .filter(|route| route.enabled && route.model == model)
        .map(|route| route.group.as_str())
        .collect();
    assert_eq!(published_groups, vec![group.as_str()]);
    assert!(
        !release
            .pricing
            .rules
            .iter()
            .any(|rule| { rule.model == model && rule.group == other_group })
    );
    let rule = release
        .pricing
        .rules
        .iter()
        .find(|r| r.model == model && r.group == group)
        .unwrap();
    let frozen_version = format!("routing-{}-rule-{}", release.pricing.version, rule.id);
    assert_eq!(settled.6["version"], frozen_version);
    let balance = ledger.balance(account).await.unwrap();
    assert_eq!((balance.available_units, balance.reserved_units), (950, 0));
    let holds: Vec<(i64, Option<i64>, bool, i64, i64)> = sqlx::query_as(
        "SELECT reservation_journal_id,capture_journal_id,closed,captured,refunded
         FROM core_charging.ledger_holds WHERE request_id=$1",
    )
    .bind(id)
    .fetch_all(&pool)
    .await
    .unwrap();
    assert_eq!(holds.len(), 1);
    let (reservation, capture, closed, amount, refunded) = holds[0];
    assert_eq!((closed, amount, refunded), (true, 50, 0));
    let captured = (
        reservation,
        capture.expect("settled request has a capture journal"),
    );
    assert_ne!(captured.0, captured.1);
    let entries: Vec<(i64, String, Option<i64>, i64)> = sqlx::query_as(
        "SELECT e.journal_id,a.bucket,a.owner_account_id,e.delta_units FROM core_billing.ledger_entries e
         JOIN core_billing.ledger_accounts a ON a.id=e.ledger_account_id
         WHERE e.journal_id IN ($1,$2) ORDER BY e.journal_id,a.id",
    )
    .bind(captured.0).bind(captured.1).fetch_all(&pool).await.unwrap();
    for journal in [captured.0, captured.1] {
        let legs: Vec<_> = entries.iter().filter(|entry| entry.0 == journal).collect();
        assert!(legs.len() >= 2);
        assert_eq!(legs.iter().map(|entry| entry.3).sum::<i64>(), 0);
    }
    let wallet_net: i64 = entries
        .iter()
        .filter(|entry| entry.1 == "wallet" && entry.2 == Some(account))
        .map(|entry| entry.3)
        .sum();
    assert_eq!(wallet_net, -50);
    proof["database"] = json!({
        "payer_account_id":account, "settled_requests_for_key":settled_for_key, "captured_credits":50, "reserved_after":0,
        "frozen_price_version":frozen_version, "reserve_journal":captured.0, "capture_journal":captured.1,
        "entries":entries.into_iter().map(|(journal,bucket,owner,delta)|
            json!({"journal_id":journal,"bucket":bucket,"account_id":owner,"delta":delta})).collect::<Vec<_>>()
    });
    std::fs::write(
        std::env::var("BROWSER_RECEIPT").expect("cloud receipt destination"),
        serde_json::to_vec_pretty(&proof).unwrap(),
    )
    .unwrap();
}

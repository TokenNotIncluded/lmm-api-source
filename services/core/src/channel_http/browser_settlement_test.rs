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

const MODEL: &str = "browser-supplier-new";
const PUBLIC_ENDPOINT: &str = "https://provider.example/v1/";
const PASSWORD: &str = "Synthetic-Browser-Password-27!";
const WEB_ORIGIN: &str = "http://127.0.0.1:41781";

async fn supplier(hits: Arc<AtomicUsize>) -> SocketAddr {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let app = Router::new()
        .route(
            "/v1/models",
            get(|headers: HeaderMap| async move {
                assert_eq!(headers[header::AUTHORIZATION], "Bearer synthetic-channel-only");
                Json(json!({"data":[
                    {"id":"browser-expensive-sibling","pricing":{"prompt":"0.000099","completion":"0.000099"}},
                    {"id":MODEL,"pricing":{"prompt":"0.000002","completion":"0.000004",
                        "input_cache_read":"0","input_cache_write":"0","request":"0"}}
                ]}))
            }),
        )
        .route(
            "/v1/chat/completions",
            post(move |headers: HeaderMap, Json(body): Json<Value>| async move {
                assert_eq!(headers[header::AUTHORIZATION], "Bearer synthetic-channel-only");
                assert_eq!(body["model"], MODEL);
                assert_eq!(hits.fetch_add(1, Ordering::SeqCst), 0);
                Json(json!({
                    "id":"synthetic-browser-answer",
                    "choices":[{"index":0,"message":{"role":"assistant","content":"browser fixture answer"},
                        "finish_reason":"stop"}],
                    "usage":{"prompt_tokens":10,"completion_tokens":20,"total_tokens":30,
                        "prompt_tokens_details":{"cached_tokens":0,"cache_write_tokens":0}}
                }))
            }),
        );
    tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
    address
}

// The regular catalog host deliberately rejects non-public supplier origins.
// Follow native_model_http/official_publication's isolated transport fixture:
// recover the exact durable release, substitute only its one TCP destination,
// and dispatch the original HTTP request through ModelGateway on the same DB.
async fn paid_gateway(pool: &PgPool, address: SocketAddr) -> ModelGateway {
    let mut release = support::active(pool).await;
    let bindings: Value = sqlx::query_scalar("SELECT public.routing_control_bindings($1)")
        .bind(release.routing.version as i64)
        .fetch_one(pool)
        .await
        .unwrap();
    let context = bindings["contexts"]
        .as_array()
        .unwrap()
        .iter()
        .find(|c| c["upstream_id"] == 1 && c["upstream_model"] == MODEL)
        .unwrap();
    assert_eq!(context["endpoint"], PUBLIC_ENDPOINT);
    assert_eq!(context["contract_revision"], "browser-v1");
    let input = context["max_input_tokens"].as_i64().unwrap();
    let output = context["max_output_tokens"].as_u64().unwrap();
    assert_eq!((input, output), (100, 20));
    let mut expected = serde_json::to_value(&release).unwrap();
    assert_eq!(release.routing.upstreams.len(), 1);
    let upstream = &mut release.routing.upstreams[0];
    assert_eq!(upstream.id, 1);
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
}

#[sqlx::test(migrations = false)]
#[ignore = "requires the built web app and cloud Chromium"]
async fn browser_onboarding_wallet(pool: PgPool) {
    let (identity, ledger, _, _, _, account) = model_fixture::fixture(&pool, 1000).await;
    identity.bootstrap_user(8, 6).await.unwrap();
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
    sqlx::query("INSERT INTO core_identity.user_routing_groups(user_id,group_name,enabled,source_reference) VALUES(7,'default',true,'synthetic:browser')")
        .execute(&pool).await.unwrap();

    let hits = Arc::new(AtomicUsize::new(0));
    let provider = supplier(hits.clone()).await;
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
    let paid = Router::new().route(
        "/v1/chat/completions",
        post(move |request: Request| {
            let db = db.clone();
            async move {
                paid_gateway(&db, provider)
                    .await
                    .router()
                    .oneshot(request)
                    .await
                    .unwrap()
            }
        }),
    );
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
    let payment = &proof["payment"];
    let id = payment["request_id"].as_str().unwrap();
    let key_id = payment["key_id"].as_i64().unwrap();
    assert_eq!(hits.load(Ordering::SeqCst), 1);
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
    let rule = release
        .pricing
        .rules
        .iter()
        .find(|r| r.model == MODEL && r.group == "default")
        .unwrap();
    let frozen_version = format!("routing-{}-rule-{}", release.pricing.version, rule.id);
    assert_eq!(settled.6["version"], frozen_version);
    let balance = ledger.balance(account).await.unwrap();
    assert_eq!((balance.available_units, balance.reserved_units), (950, 0));
    let captured: (i64, i64) = sqlx::query_as(
        "SELECT reservation_journal_id,capture_journal_id FROM core_charging.ledger_holds
         WHERE request_id=$1 AND closed AND captured=50 AND refunded=0",
    )
    .bind(id)
    .fetch_one(&pool)
    .await
    .unwrap();
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
        "payer_account_id":account, "captured_credits":50, "reserved_after":0,
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

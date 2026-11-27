//! Native client routes against counting TCP providers and real Core funds.
//! All credentials and data are synthetic; this never calls an external API.
use axum::{
    Json, Router,
    body::{Body, to_bytes},
    extract::State,
    http::{HeaderMap, Request, StatusCode},
    response::{IntoResponse, Response},
    routing::post,
};
use lmm_core::{
    legacy_auth::{self, LegacyAuthConfig},
    model_http::{ChargePolicy, ModelGateway, UpstreamCredential},
    relay::Limits,
    routing::*,
};
use serde_json::{Value, json};
use sqlx::PgPool;
use std::{
    collections::{BTreeMap, HashSet},
    sync::{
        Arc, Mutex,
        atomic::{AtomicUsize, Ordering},
    },
    time::Duration,
};
use tokio::{net::TcpListener, task::JoinHandle};
use tower::ServiceExt;
#[path = "common/model_fixture.rs"]
mod model_fixture;
use model_fixture::fixture;

#[derive(Clone)]
struct Supplier {
    protocol: Protocol,
    hits: Arc<AtomicUsize>,
    missing_usage: bool,
    requests: Arc<Mutex<Vec<Value>>>,
}
async fn supplier(
    State(state): State<Supplier>,
    headers: HeaderMap,
    Json(request): Json<Value>,
) -> Response {
    state.hits.fetch_add(1, Ordering::SeqCst);
    state.requests.lock().unwrap().push(request.clone());
    let (key, expected) = match state.protocol {
        Protocol::AnthropicMessages => ("x-api-key", "synthetic-upstream-key"),
        Protocol::Gemini => ("x-goog-api-key", "synthetic-upstream-key"),
        _ => ("authorization", "Bearer synthetic-upstream-key"),
    };
    assert_eq!(headers[key], expected);
    if key != "authorization" {
        assert!(!headers.contains_key("authorization"));
    }
    if state.protocol == Protocol::OpenAiChat {
        assert_eq!(request["model"], "provider-model");
        return Json(json!({
            "id":"synthetic-chat", "choices":[{"index":0,
                "message":{"role":"assistant","content":"native answer"},"finish_reason":"stop"}],
            "usage":{"prompt_tokens":10,"completion_tokens":20,"total_tokens":30,
                "prompt_tokens_details":{"cached_tokens":2}}
        }))
        .into_response();
    }
    if state.protocol == Protocol::OpenAiResponses {
        assert_eq!(request["model"], "provider-model");
        return Json(json!({
            "id":"synthetic-response", "status":"completed",
            "output":[{"id":"synthetic-message","type":"message","role":"assistant",
                "content":[{"type":"output_text","text":"native answer","annotations":[]}]}],
            "usage":{"input_tokens":10,"output_tokens":20,"total_tokens":30,
                "input_tokens_details":{"cached_tokens":2}}
        }))
        .into_response();
    }
    if state.protocol == Protocol::AnthropicMessages {
        assert_eq!(request["model"], "provider-model");
        let mut usage = json!({"input_tokens":8,"cache_read_input_tokens":2,"cache_creation_input_tokens":0,"output_tokens":20});
        if state.missing_usage {
            usage = Value::Null;
        }
        if request["stream"] == true {
            let events = [
                json!({"type":"message_start","message":{"id":"synthetic-message","usage":{"input_tokens":8,"cache_read_input_tokens":2,"cache_creation_input_tokens":0,"output_tokens":0}}}),
                json!({"type":"content_block_start","index":0,"content_block":{"type":"text","text":""}}),
                json!({"type":"content_block_delta","index":0,"delta":{"type":"text_delta","text":"native answer"}}),
                json!({"type":"content_block_stop","index":0}),
                json!({"type":"message_delta","delta":{"stop_reason":"end_turn"},"usage":{"output_tokens":20}}),
                json!({"type":"message_stop"}),
            ];
            let body = events
                .iter()
                .map(|event| {
                    format!(
                        "event: {}\ndata: {event}\n\n",
                        event["type"].as_str().unwrap()
                    )
                })
                .collect::<String>();
            return ([("content-type", "text/event-stream")], body).into_response();
        }
        return Json(json!({"id":"synthetic-message","type":"message","role":"assistant","model":"provider-model","content":[{"type":"text","text":"native answer"}],"stop_reason":"end_turn","usage":usage})).into_response();
    }
    let usage = if state.missing_usage {
        Value::Null
    } else {
        json!({"promptTokenCount":10,"cachedContentTokenCount":2,"candidatesTokenCount":20,"thoughtsTokenCount":0,"totalTokenCount":30})
    };
    let value = json!({"candidates":[{"index":0,"content":{"role":"model","parts":[{"text":"native answer"}]},"finishReason":"STOP"}],"usageMetadata":usage});
    // Gemini streaming is selected only by its configured endpoint.
    Json(value).into_response()
}
async fn gemini_stream(
    State(state): State<Supplier>,
    headers: HeaderMap,
    Json(request): Json<Value>,
) -> Response {
    let response = supplier(State(state), headers, Json(request)).await;
    let body = to_bytes(response.into_body(), 100_000).await.unwrap();
    (
        [("content-type", "text/event-stream")],
        format!("data: {}\n\n", String::from_utf8(body.to_vec()).unwrap()),
    )
        .into_response()
}
struct Server {
    endpoint: String,
    hits: Arc<AtomicUsize>,
    requests: Arc<Mutex<Vec<Value>>>,
    task: JoinHandle<()>,
}
impl Drop for Server {
    fn drop(&mut self) {
        self.task.abort();
    }
}
async fn server(protocol: Protocol, missing_usage: bool) -> Server {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let endpoint = format!("http://{}/v1", listener.local_addr().unwrap());
    let hits = Arc::new(AtomicUsize::new(0));
    let requests = Arc::new(Mutex::new(Vec::new()));
    let app = Router::new()
        .route("/v1/chat/completions", post(supplier))
        .route("/v1/responses", post(supplier))
        .route("/v1/messages", post(supplier))
        .route("/v1/models/provider-model:generateContent", post(supplier))
        .route(
            "/v1/models/provider-model:streamGenerateContent",
            post(gemini_stream),
        )
        .with_state(Supplier {
            protocol,
            hits: hits.clone(),
            missing_usage,
            requests: requests.clone(),
        });
    let task = tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
    Server {
        endpoint,
        hits,
        requests,
        task,
    }
}
fn gateway(pool: &PgPool, server: &Server, protocol: Protocol, key_id: i64) -> ModelGateway {
    gateway_with_priority(pool, server, protocol, key_id, false)
}

fn gateway_with_priority(pool: &PgPool, server: &Server, protocol: Protocol, key_id: i64, priority: bool) -> ModelGateway {
    let mut pricing: PriceConfig = serde_json::from_value(json!({"version":1,"rules":[{"id":1,"model":"native-model","group":"allowed","speed":"standard","rates":{"input":2000000,"output":4000000,"cache_read":500000,"cache_write":0,"request":0},"group_multiplier_ppm":1000000,"speed_multiplier_ppm":1000000}]})).unwrap();
    let mut speeds = vec![Speed::Standard];
    let policy = priority.then(|| {
        let mut fast = pricing.rules[0].clone();
        fast.id = 2;
        fast.speed = Speed::Fast;
        fast.speed_multiplier_ppm = 2_000_000;
        pricing.rules.push(fast);
        speeds.push(Speed::Fast);
        serde_json::from_value(json!({
            "honor_request_service_tier":true,"pass_through_request_enabled":true,
            "thinking_model_blacklist":[],"chat_completions_to_responses_policy":{},
            "ping_interval_enabled":false,"ping_interval_seconds":30
        })).unwrap()
    });
    let prices = PriceSnapshot::compile(pricing).unwrap();
    let routes = RoutingConfig {
        channel_overrides: Default::default(),
        global_model_policy: policy,
        channel_affinity: None,
        routing_reliability: None,
        version: 1,
        groups: vec![ModelGroup {
            name: "allowed".into(),
            enabled: true,
        }],
        upstreams: vec![UpstreamConfig {
            id: 1,
            endpoint: Endpoint::local_loopback(server.endpoint.clone()).unwrap(),
            protocol,
            credential: CredentialRef { id: 7, revision: 1 },
            enabled: true,
        }],
        routes: vec![ModelRoute {
            id: 1,
            model: "native-model".into(),
            group: "allowed".into(),
            enabled: true,
            speeds: speeds.clone(),
            targets: vec![RouteTarget {
                id: 1,
                upstream_id: 1,
                upstream_model: "provider-model".into(),
                enabled: true,
                priority: 0,
                weight: 1,
                speeds,
                client_protocols: None,
                payload: PayloadPolicy::Adapt,
                timeout_ms: 5000,
            }],
            failover: FailoverPolicy::default(),
        }],
        aliases: vec![],
    };
    ModelGateway::new(
        pool.clone(),
        Arc::new(lmm_core::routing::Router::new(routes, Arc::new(prices)).unwrap()),
        vec![UpstreamCredential {
            reference: CredentialRef { id: 7, revision: 1 },
            secret: "synthetic-upstream-key".into(),
        }],
        BTreeMap::from([(key_id, "allowed".into())]),
        Limits {
            total_timeout: Duration::from_secs(3),
            hook_timeout: Duration::from_secs(2),
            max_output_tokens: 20,
            ..Default::default()
        },
        ChargePolicy {
            input_token_ceiling: 100,
            lease_seconds: 30,
        },
    )
    .unwrap()
    .with_local_streaming()
}
fn input(protocol: Protocol, stream: bool) -> (&'static str, Value, &'static str) {
    if protocol == Protocol::AnthropicMessages {
        (
            "/v1/messages",
            json!({"model":"native-model","messages":[{"role":"user","content":"hello"}],"max_tokens":20,"stream":stream}),
            "x-api-key",
        )
    } else {
        (
            if stream {
                "/v1beta/models/native-model:streamGenerateContent?alt=sse"
            } else {
                "/v1beta/models/native-model:generateContent"
            },
            json!({"contents":[{"role":"user","parts":[{"text":"hello"}]}],"generationConfig":{"maxOutputTokens":20}}),
            "x-goog-api-key",
        )
    }
}
#[sqlx::test(migrations = false)]
async fn native_anthropic_and_gemini_settle_real_funds_and_stream_checkpoints(pool: PgPool) {
    let (_, ledger, _, key, key_id, account) = fixture(&pool, 1000).await;
    for protocol in [Protocol::AnthropicMessages, Protocol::Gemini] {
        let server = server(protocol, false).await;
        let gateway = gateway(&pool, &server, protocol, key_id);
        for stream in [false, true] {
            let (path, body, header) = input(protocol, stream);
            let response = gateway
                .router()
                .oneshot(
                    Request::builder()
                        .method("POST")
                        .uri(path)
                        .header(header, &key)
                        .header("content-type", "application/json")
                        .header("idempotency-key", format!("native-{protocol:?}-{stream}"))
                        .body(Body::from(serde_json::to_vec(&body).unwrap()))
                        .unwrap(),
                )
                .await
                .unwrap();
            assert_eq!(response.status(), StatusCode::OK);
            let id = response.headers()["x-request-id"]
                .to_str()
                .unwrap()
                .to_owned();
            let body = String::from_utf8(
                to_bytes(response.into_body(), 1_000_000)
                    .await
                    .unwrap()
                    .to_vec(),
            )
            .unwrap();
            assert!(body.contains("native answer"), "{body}");
            if protocol == Protocol::AnthropicMessages {
                assert!(body.contains(if stream { "message_stop" } else { "end_turn" }));
            } else {
                assert!(body.contains("STOP"));
            }
            let charge:(String,i64,i64,i64)=sqlx::query_as("SELECT state,settled,input_tokens,output_tokens FROM core_charging.requests WHERE id=$1").bind(&id).fetch_one(&pool).await.unwrap();
            assert_eq!(charge, ("settled".into(), 49, 10, 20));
            let checkpoints: Vec<(i64, i64, i64, Value, bool, Vec<u8>)> = sqlx::query_as(
                "SELECT sequence,packet_bytes,prepared_bytes,provider_usage,terminal_observed,packet_sha256 \
                 FROM core_model.output_checkpoints WHERE request_id=$1 ORDER BY sequence",
            )
            .bind(&id)
            .fetch_all(&pool)
            .await
            .unwrap();
            assert!(!checkpoints.is_empty());
            assert!(checkpoints.last().unwrap().4);
            if stream {
                assert!(checkpoints.len() > 1);
                assert!(checkpoints.iter().any(|row| row.1 > 0));
            } else {
                // A complete buffered response retains recovery authority, but
                // its sole empty terminal observation claims no stream bytes.
                assert_eq!(checkpoints.len(), 1);
                assert_eq!(checkpoints[0].0, 1);
                assert_eq!((checkpoints[0].1, checkpoints[0].2), (0, 0));
            }
            let mut prepared = 0_usize;
            for (index, (sequence, bytes, cumulative, usage, terminal, hash)) in
                checkpoints.iter().enumerate()
            {
                use sha2::{Digest, Sha256};
                assert_eq!(*sequence, index as i64 + 1);
                let bytes = usize::try_from(*bytes).unwrap();
                let packet = if stream {
                    &body.as_bytes()[prepared..prepared + bytes]
                } else {
                    &[]
                };
                assert_eq!(hash.as_slice(), Sha256::digest(packet).as_slice());
                prepared += bytes;
                assert_eq!(*cumulative, prepared as i64);
                if *terminal {
                    assert_eq!(usage["input_tokens"], 10);
                    assert_eq!(usage["cached_input_tokens"], 2);
                    assert_eq!(usage["output_tokens"], 20);
                }
            }
            assert_eq!(prepared, if stream { body.len() } else { 0 });
        }
        assert_eq!(server.hits.load(Ordering::SeqCst), 2);
    }
    let balance = ledger.balance(account).await.unwrap();
    assert_eq!(balance.available_units, 804);
    assert_eq!(balance.reserved_units, 0);
    let bad:i64=sqlx::query_scalar("SELECT count(*) FROM (SELECT journal_id FROM core_billing.ledger_entries GROUP BY journal_id HAVING sum(delta_units)<>0) e").fetch_one(&pool).await.unwrap();
    assert_eq!(bad, 0);
}
#[sqlx::test(migrations = false)]
async fn native_missing_usage_holds_funds_and_ambiguous_credentials_never_dispatch(pool: PgPool) {
    let (_, ledger, _, key, key_id, account) = fixture(&pool, 1000).await;
    let server = server(Protocol::AnthropicMessages, true).await;
    let gateway = gateway(&pool, &server, Protocol::AnthropicMessages, key_id);
    let (path, body, header) = input(Protocol::AnthropicMessages, false);
    let request = || {
        Request::builder()
            .method("POST")
            .uri(path)
            .header(header, &key)
            .header("content-type", "application/json")
    };
    let response = gateway
        .router()
        .oneshot(
            request()
                .header("authorization", "Bearer different-key")
                .body(Body::from(serde_json::to_vec(&body).unwrap()))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::BAD_REQUEST);
    assert_eq!(server.hits.load(Ordering::SeqCst), 0);
    let response = gateway
        .router()
        .oneshot(
            request()
                .body(Body::from(serde_json::to_vec(&body).unwrap()))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::BAD_GATEWAY);
    let _ = to_bytes(response.into_body(), 100_000).await;
    let balance = ledger.balance(account).await.unwrap();
    assert!(balance.reserved_units > 0);
    assert_eq!(balance.available_units + balance.reserved_units, 1000);
    let states: Vec<String> = sqlx::query_scalar("SELECT state FROM core_charging.requests")
        .fetch_all(&pool)
        .await
        .unwrap();
    assert_eq!(states, vec!["reconcile"]);
    assert_eq!(server.hits.load(Ordering::SeqCst), 1);
}

/// Exercise the public management and four model routes over real loopback
/// HTTP. Only supplier responses are synthetic; identity and funds use PostgreSQL.
#[sqlx::test(migrations = false)]
async fn dashboard_key_four_protocol_http_lifecycle_and_settlement(pool: PgPool) {
    four_protocol_lifecycle(pool, false).await;
}

#[sqlx::test(migrations = false)]
async fn official_scalar_four_protocol_http_lifecycle_and_settlement(pool: PgPool) {
    four_protocol_lifecycle(pool, true).await;
}

async fn four_protocol_lifecycle(pool: PgPool, publish_official: bool) {
    let (identity, ledger, session, _, _, account) = fixture(&pool, 1000).await;
    sqlx::query("INSERT INTO core_identity.user_routing_groups(user_id,group_name,enabled,source_reference) VALUES (7,'allowed',true,'synthetic:key-protocol-lifecycle')")
        .execute(&pool).await.unwrap();
    let mut suppliers = Vec::new();
    for protocol in [
        Protocol::OpenAiChat,
        Protocol::OpenAiResponses,
        Protocol::AnthropicMessages,
        Protocol::Gemini,
    ] {
        suppliers.push((protocol, server(protocol, false).await));
    }
    let prices: PriceConfig = serde_json::from_value(json!({
        "version":1,"rules":[{"id":1,"model":"native-model","group":"allowed","speed":"standard",
            "rates":{"input":2000000,"output":4000000,"cache_read":500000,"cache_write":0,"request":0},
            "group_multiplier_ppm":1000000,"speed_multiplier_ppm":1000000}]
    })).unwrap();
    let config = RoutingConfig {
        channel_overrides: Default::default(),
        global_model_policy: None,
        channel_affinity: None,
        routing_reliability: None,
        version: 1,
        groups: vec![ModelGroup {
            name: "allowed".into(),
            enabled: true,
        }],
        upstreams: suppliers
            .iter()
            .enumerate()
            .map(|(index, (protocol, supplier))| UpstreamConfig {
                id: (index + 1) as u64,
                endpoint: Endpoint::local_loopback(supplier.endpoint.clone()).unwrap(),
                protocol: *protocol,
                credential: CredentialRef { id: 7, revision: 1 },
                enabled: true,
            })
            .collect(),
        routes: vec![ModelRoute {
            id: 1,
            model: "native-model".into(),
            group: "allowed".into(),
            enabled: true,
            speeds: vec![Speed::Standard],
            targets: suppliers
                .iter()
                .enumerate()
                .map(|(index, (protocol, _))| RouteTarget {
                    id: (index + 1) as u64,
                    upstream_id: (index + 1) as u64,
                    upstream_model: "provider-model".into(),
                    enabled: true,
                    priority: 0,
                    weight: 1,
                    speeds: vec![Speed::Standard],
                    client_protocols: Some(vec![*protocol]),
                    payload: PayloadPolicy::Adapt,
                    timeout_ms: 5000,
                })
                .collect(),
            failover: FailoverPolicy::default(),
        }],
        aliases: vec![],
    };
    let mut release = store::ReleaseDocument {
        contract_version: CONTRACT_VERSION,
        routing: config,
        pricing: prices,
    };
    let mut publisher = None;
    let mut frozen_tariffs = Vec::new();
    if publish_official {
        let admin = identity.bootstrap_user(8, 6).await.unwrap().secret;
        let control = official_publication::Publisher::new(&pool, admin, &release).await;
        release = control.publish(4_000_000).await;
        official_publication::use_local_suppliers(&mut release, &suppliers);
        let rule = &release.pricing.rules[0];
        let price_version = format!("routing-{}-rule-{}", release.pricing.version, rule.id);
        let digest = rule.tiered.as_ref().unwrap().source_digest.clone();
        publisher = Some((control, price_version, digest));
    }
    let gateway = four_protocol_gateway(&pool, release, vec![]);
    let routing = gateway.dashboard_routing();
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let origin = format!("http://{}", listener.local_addr().unwrap());
    let auth = LegacyAuthConfig::new(&origin, &[43; 32])
        .unwrap()
        .with_insecure_local_http()
        .unwrap();
    let app =
        legacy_auth::router(identity.with_dashboard_routing(routing), auth).merge(gateway.router());
    let serving = tokio::spawn(async move {
        axum::serve(
            listener,
            app.into_make_service_with_connect_info::<std::net::SocketAddr>(),
        )
        .await
        .unwrap();
    });
    let client = reqwest::Client::builder()
        .no_proxy()
        .timeout(Duration::from_secs(5))
        .build()
        .unwrap();
    let expires: i64 =
        sqlx::query_scalar("SELECT extract(epoch FROM clock_timestamp())::bigint + 600")
            .fetch_one(&pool)
            .await
            .unwrap();
    let created = client
        .post(format!("{origin}/api/token/"))
        .bearer_auth(&session)
        .header("origin", &origin)
        .json(&json!({
            "name":"Synthetic four-protocol lifecycle","remain_quota":512,
            "expired_time":expires,"unlimited_quota":false,"one_time_reveal":true,
            "group":"allowed","auto_groups":[],"cross_group_retry":false,
            "model_limits_enabled":true,"model_limits":"native-model","allow_ips":"127.0.0.1/32"
        }))
        .send()
        .await
        .unwrap();
    assert_eq!(created.status(), StatusCode::OK);
    assert_eq!(created.headers()["cache-control"], "no-store");
    let created: Value = created.json().await.unwrap();
    assert_eq!(created["success"], true);
    let key_id = created["data"]["id"].as_i64().unwrap();
    let key = created["data"]["key"].as_str().unwrap();
    assert!(key.starts_with("lmmk_"));
    assert_eq!(created["data"]["one_time_reveal"], true);
    assert_eq!(created["data"]["unlimited_quota"], false);
    assert_eq!(created["data"]["remain_quota"], 512);
    assert_eq!(created["data"]["expired_time"], expires);
    let reveal = client
        .post(format!("{origin}/api/token/{key_id}/key"))
        .bearer_auth(&session)
        .header("origin", &origin)
        .send()
        .await
        .unwrap();
    assert_eq!(reveal.status(), StatusCode::FORBIDDEN);
    let mut ids = HashSet::new();
    let mut cases = Vec::new();
    for (index, (protocol, supplier)) in suppliers.iter().enumerate() {
        let (path, header, body) = match protocol {
            Protocol::OpenAiChat => (
                "/v1/chat/completions",
                "authorization",
                json!({
                    "model":"native-model","messages":[{"role":"user","content":"hello"}],"max_tokens":20
                }),
            ),
            Protocol::OpenAiResponses => (
                "/v1/responses",
                "authorization",
                json!({
                    "model":"native-model","input":"hello","max_output_tokens":20
                }),
            ),
            Protocol::AnthropicMessages => (
                "/v1/messages",
                "x-api-key",
                json!({
                    "model":"native-model","messages":[{"role":"user","content":"hello"}],"max_tokens":20
                }),
            ),
            Protocol::Gemini => (
                "/v1beta/models/native-model:generateContent",
                "x-goog-api-key",
                json!({
                    "contents":[{"role":"user","parts":[{"text":"hello"}]}],
                    "generationConfig":{"maxOutputTokens":20}
                }),
            ),
            _ => unreachable!(),
        };
        let credential = if header == "authorization" {
            format!("Bearer {key}")
        } else {
            key.into()
        };
        let mut request = client
            .post(format!("{origin}{path}"))
            .header(header, &credential)
            .header("idempotency-key", format!("synthetic-key-protocol-{index}"))
            .json(&body);
        if *protocol == Protocol::AnthropicMessages {
            request = request.header("anthropic-version", "2023-06-01");
        }
        let response = request.send().await.unwrap();
        assert_eq!(response.status(), StatusCode::OK, "{protocol:?}");
        let request_id = response.headers()["x-request-id"]
            .to_str()
            .unwrap()
            .to_owned();
        assert!(ids.insert(request_id.clone()));
        let answer: Value = response.json().await.unwrap();
        match protocol {
            Protocol::OpenAiChat => {
                assert_eq!(answer["choices"][0]["message"]["content"], "native answer");
                assert_eq!(answer["choices"][0]["finish_reason"], "stop");
                assert_eq!(answer["usage"]["prompt_tokens"], 10);
                assert_eq!(answer["usage"]["completion_tokens"], 20);
            }
            Protocol::OpenAiResponses => {
                assert_eq!(answer["output"][0]["content"][0]["text"], "native answer");
                assert_eq!(answer["status"], "completed");
                assert_eq!(answer["usage"]["input_tokens"], 10);
                assert_eq!(answer["usage"]["output_tokens"], 20);
            }
            Protocol::AnthropicMessages => {
                assert_eq!(answer["content"][0]["text"], "native answer");
                assert_eq!(answer["stop_reason"], "end_turn");
                assert_eq!(answer["usage"]["input_tokens"], 8);
                assert_eq!(answer["usage"]["cache_read_input_tokens"], 2);
                assert_eq!(answer["usage"]["output_tokens"], 20);
            }
            Protocol::Gemini => {
                assert_eq!(
                    answer["candidates"][0]["content"]["parts"][0]["text"],
                    "native answer"
                );
                assert_eq!(answer["candidates"][0]["finishReason"], "STOP");
                assert_eq!(answer["usageMetadata"]["promptTokenCount"], 10);
                assert_eq!(answer["usageMetadata"]["candidatesTokenCount"], 20);
            }
            _ => unreachable!(),
        }
        assert_eq!(supplier.hits.load(Ordering::SeqCst), 1);
        {
            let observed = supplier.requests.lock().unwrap();
            assert_eq!(observed.len(), 1);
            let request = &observed[0];
            let text = match protocol {
                Protocol::OpenAiChat => {
                    assert_eq!(request["messages"][0]["role"], "user");
                    let content = &request["messages"][0]["content"];
                    content
                        .as_str()
                        .unwrap_or_else(|| content[0]["text"].as_str().unwrap())
                }
                Protocol::OpenAiResponses => {
                    request["input"][0]["content"][0]["text"].as_str().unwrap()
                }
                Protocol::AnthropicMessages => request["messages"][0]["content"][0]["text"]
                    .as_str()
                    .unwrap(),
                Protocol::Gemini => request["contents"][0]["parts"][0]["text"].as_str().unwrap(),
                _ => unreachable!(),
            };
            assert_eq!(text, "hello");
        }
        let charge:(String,i64,i64,i64,i64)=sqlx::query_as(
            "SELECT state,key_id,settled,input_tokens,output_tokens FROM core_charging.requests WHERE id=$1"
        ).bind(&request_id).fetch_one(&pool).await.unwrap();
        assert_eq!(charge, ("settled".into(), key_id, 49, 10, 20));
        if let Some((_, version, digest)) = &publisher {
            let frozen = official_publication::frozen(&pool, &request_id).await;
            assert_eq!(frozen["version"].as_str(), Some(version.as_str()));
            assert_eq!(
                frozen["tiered"]["tariff"]["source_digest"].as_str(),
                Some(digest.as_str())
            );
            official_publication::check_frozen(&frozen, *protocol);
            official_publication::check_attempt(&pool, &request_id, index + 1, *protocol).await;
            frozen_tariffs.push((request_id.clone(), frozen));
        }
        let hold:(i64,bool,bool)=sqlx::query_as(
            "SELECT captured,closed,capture_journal_id IS NOT NULL FROM core_charging.ledger_holds WHERE request_id=$1"
        ).bind(&request_id).fetch_one(&pool).await.unwrap();
        assert_eq!(hold, (49, true, true));
        let balance = ledger.balance(account).await.unwrap();
        assert_eq!(balance.available_units, 1000 - 49 * (index as i64 + 1));
        assert_eq!(balance.reserved_units, 0);
        println!(
            "key_protocol_case protocol={protocol:?} request_id_unique=true settled_credits=49 reserved_credits=0"
        );
        cases.push((path, header, body));
    }
    if let Some((control, _, digest)) = &publisher {
        assert_eq!(frozen_tariffs.len(), 4);
        let repriced = control.publish(8_000_000).await;
        let newer = serde_json::to_value(repriced.pricing).unwrap();
        assert_eq!(newer["version"], 3);
        assert_ne!(
            newer["rules"][0]["tiered"]["source_digest"].as_str(),
            Some(digest.as_str())
        );
        for (id, original) in &frozen_tariffs {
            assert_eq!(official_publication::frozen(&pool, id).await, *original);
        }
    }
    let before = client
        .get(format!("{origin}/api/token/{key_id}"))
        .bearer_auth(&session)
        .send()
        .await
        .unwrap();
    assert_eq!(before.status(), StatusCode::OK);
    let before: Value = before.json().await.unwrap();
    assert_eq!(before["data"]["used_quota"], 196);
    assert_eq!(before["data"]["remain_quota"], 316);
    let entries: i64 = sqlx::query_scalar("SELECT count(*) FROM core_billing.ledger_entries")
        .fetch_one(&pool)
        .await
        .unwrap();
    let removed = client
        .delete(format!("{origin}/api/token/{key_id}/"))
        .bearer_auth(&session)
        .header("origin", &origin)
        .send()
        .await
        .unwrap();
    assert_eq!(removed.status(), StatusCode::OK);
    assert_eq!(removed.json::<Value>().await.unwrap()["success"], true);
    let after = client
        .get(format!("{origin}/api/token/{key_id}"))
        .bearer_auth(&session)
        .send()
        .await
        .unwrap();
    assert_eq!(after.status(), StatusCode::OK);
    let after: Value = after.json().await.unwrap();
    assert_eq!(after["data"]["status"], 2);
    assert_eq!(after["data"]["used_quota"], 196);
    for (index, (path, header, body)) in cases.iter().enumerate() {
        let credential = if *header == "authorization" {
            format!("Bearer {key}")
        } else {
            key.into()
        };
        let mut request = client
            .post(format!("{origin}{path}"))
            .header(*header, credential)
            .header("idempotency-key", format!("synthetic-key-revoked-{index}"))
            .json(body);
        if *path == "/v1/messages" {
            request = request.header("anthropic-version", "2023-06-01");
        }
        let rejected = request.send().await.unwrap();
        assert!(matches!(
            rejected.status(),
            StatusCode::UNAUTHORIZED | StatusCode::FORBIDDEN
        ));
    }
    for (_, supplier) in &suppliers {
        assert_eq!(supplier.hits.load(Ordering::SeqCst), 1);
    }
    let requests: i64 =
        sqlx::query_scalar("SELECT count(*) FROM core_charging.requests WHERE key_id=$1")
            .bind(key_id)
            .fetch_one(&pool)
            .await
            .unwrap();
    assert_eq!(requests, 4);
    let after_entries: i64 = sqlx::query_scalar("SELECT count(*) FROM core_billing.ledger_entries")
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(after_entries, entries);
    let balance = ledger.balance(account).await.unwrap();
    assert_eq!(balance.available_units, 804);
    assert_eq!(balance.reserved_units, 0);
    let unbalanced:i64=sqlx::query_scalar(
        "SELECT count(*) FROM (SELECT journal_id FROM core_billing.ledger_entries GROUP BY journal_id HAVING sum(delta_units)<>0) e"
    ).fetch_one(&pool).await.unwrap();
    assert_eq!(unbalanced, 0);
    serving.abort();
    assert!(serving.await.unwrap_err().is_cancelled());
    println!(
        "key_protocol_lifecycle protocols=4 requests=4 settled_credits=196 revoked_rejections=4 no_extra_dispatch_or_ledger_entries=true"
    );
}

fn four_protocol_gateway(
    pool: &PgPool,
    release: store::ReleaseDocument,
    contexts: Vec<lmm_core::model_http::ExternalModelContext>,
) -> ModelGateway {
    let router = lmm_core::routing::Router::new(
        release.routing,
        Arc::new(PriceSnapshot::compile(release.pricing).unwrap()),
    )
    .unwrap();
    ModelGateway::new_with_provider_policy(
        pool.clone(),
        Arc::new(router),
        vec![UpstreamCredential {
            reference: CredentialRef { id: 7, revision: 1 },
            secret: "synthetic-upstream-key".into(),
        }],
        BTreeMap::new(),
        Limits {
            total_timeout: Duration::from_secs(3),
            hook_timeout: Duration::from_secs(2),
            max_output_tokens: 20,
            ..Default::default()
        },
        ChargePolicy {
            input_token_ceiling: 100,
            lease_seconds: 30,
        },
        lmm_core::model_http::ProviderPolicy::new(contexts, 100).unwrap(),
    )
    .unwrap()
}

#[path = "native_model_http/gemini_tier_controls.rs"]
mod gemini_tier_controls;
#[path = "native_model_http/official_publication.rs"]
mod official_publication;

//! Independent acceptance: real TCP Core worker process, restricted PG role,
//! independently scripted loopback supplier. No author's test or assertion reuse.
use super::*;
use crate::routing::{
    self, CredentialRef, Endpoint, PriceConfig, PriceSnapshot, Protocol, RoutingConfig,
};
use crate::test_runtime::RuntimeRole;
use axum::extract::OriginalUri;
use futures_util::StreamExt;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::{
    fs,
    os::unix::fs::OpenOptionsExt,
    path::PathBuf,
    process::{Child, Command, Stdio},
    sync::atomic::{AtomicUsize, Ordering},
};
use tokio::sync::Semaphore;

const MODEL: &str = "gpt-4.1-2025-04-14";
const PUBLIC: &str = "independent-compact";
const HOLD: i64 = 2650; // 800 input * 3 Credits + 50 output * 5 Credits.
fn artifacts() -> PathBuf {
    PathBuf::from(
        std::env::var("COMPACT_INDEPENDENT_ARTIFACTS").expect("private output directory required"),
    )
}
fn record(name: &str, value: Value) {
    fs::write(
        artifacts().join(format!("{name}.json")),
        serde_json::to_vec_pretty(&value).unwrap(),
    )
    .unwrap();
}
fn opaque() -> Value {
    json!({"type":"compaction","id":"opaque-independent","encrypted_content":"opaque-independent-λ".repeat(2000)})
}
fn usage(input: u64, output: u64) -> Value {
    json!({"input_tokens":input,"output_tokens":output,"total_tokens":input+output,"input_tokens_details":{"cached_tokens":0}})
}
#[derive(Clone, Copy)]
enum Mode {
    Normal,
    Pretty,
    Gate,
    Tail,
    Duplicate,
    Never,
    HoldJson,
}
#[derive(Clone)]
struct Supplier {
    base: String,
    hits: Arc<AtomicUsize>,
    count_hits: Arc<AtomicUsize>,
    seen: Arc<Mutex<Vec<Value>>>,
    mode: Arc<Mutex<Mode>>,
    entered: Arc<Semaphore>,
    release: Arc<Semaphore>,
}
async fn supplier_reply(
    State(s): State<Supplier>,
    OriginalUri(uri): OriginalUri,
    h: HeaderMap,
    Json(body): Json<Value>,
) -> Response {
    assert_eq!(h[header::AUTHORIZATION], "Bearer independent-loopback-only");
    assert_eq!(body["model"], MODEL);
    s.hits.fetch_add(1, Ordering::SeqCst);
    s.seen.lock().unwrap().push(body.clone());
    let mode = *s.mode.lock().unwrap();
    let compact = uri.path().ends_with("/compact");
    let out = if compact {
        json!({"id":"cmp-independent","object":"response.compaction","output":[opaque()],"usage":usage(11,2)})
    } else {
        json!({"id":"resp-independent","object":"response","status":"completed","model":MODEL,
        "output":[{"type":"function_call","id":"fc-independent","call_id":"call-independent","name":"audit_lookup","arguments":"{\"city\":\"台北\"}"}],"usage":usage(7,3)})
    };
    if body["stream"] != true {
        if matches!(mode, Mode::HoldJson) {
            s.entered.add_permits(1);
            s.release.acquire().await.unwrap().forget();
        }
        let raw = if matches!(mode, Mode::Pretty) {
            serde_json::to_vec_pretty(&out).unwrap()
        } else {
            serde_json::to_vec(&out).unwrap()
        };
        return ([(header::CONTENT_TYPE, "application/json")], raw).into_response();
    }
    assert_eq!(body["store"], false);
    assert_eq!(body["tools"][0]["name"], "audit_lookup");
    let created = json!({"type":"response.created","response":{"id":"resp-independent","status":"in_progress"}});
    let delta = json!({"type":"response.function_call_arguments.delta","response_id":"resp-independent","item_id":"fc-independent","output_index":0,"delta":"台北-λ"});
    let terminal = json!({"type":"response.completed","response":out});
    // Supplier formatting deliberately differs from the Core's encoded client packets.
    let initial = format!(
        "event: response.created\ndata: {created}\n\nevent: response.function_call_arguments.delta\ndata: {delta}\n\n"
    );
    let last = format!("event: response.completed\ndata: {terminal}\n\n");
    let (tx, rx) = tokio::sync::mpsc::channel::<Result<Bytes, std::io::Error>>(8);
    tokio::spawn(async move {
        let wire = format!("{initial}{last}");
        for chunk in wire.as_bytes().chunks(113) {
            if tx.send(Ok(Bytes::copy_from_slice(chunk))).await.is_err() {
                return;
            }
        }
        s.entered.add_permits(1);
        match mode {
            Mode::Never => std::future::pending::<()>().await,
            Mode::Gate | Mode::Tail | Mode::Duplicate => {
                s.release.acquire().await.unwrap().forget();
            }
            _ => (),
        }
        let tail = match mode {
            Mode::Tail => "BAD AFTER TERMINAL".to_owned(),
            Mode::Duplicate => last,
            _ => String::new(),
        };
        if !tail.is_empty() {
            let _ = tx.send(Ok(Bytes::from(tail))).await;
        }
        // Dropping tx is the actual HTTP-body EOF, independent from the terminal event.
    });
    (
        [(header::CONTENT_TYPE, "text/event-stream")],
        Body::from_stream(tokio_stream::wrappers::ReceiverStream::new(rx)),
    )
        .into_response()
}
async fn supplier() -> (Supplier, tokio::task::JoinHandle<()>) {
    let l = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let s = Supplier {
        base: format!("http://{}/v1", l.local_addr().unwrap()),
        hits: Arc::new(AtomicUsize::new(0)),
        count_hits: Arc::new(AtomicUsize::new(0)),
        seen: Arc::new(Mutex::new(Vec::new())),
        mode: Arc::new(Mutex::new(Mode::Normal)),
        entered: Arc::new(Semaphore::new(0)),
        release: Arc::new(Semaphore::new(0)),
    };
    let count = s.count_hits.clone();
    let app = Router::new()
        .route("/v1/responses/compact", post(supplier_reply))
        .route("/v1/responses", post(supplier_reply))
        .route(
            "/v1/responses/input_tokens",
            post(move || {
                let count = count.clone();
                async move {
                    count.fetch_add(1, Ordering::SeqCst);
                    Json(json!({"input_tokens":1}))
                }
            }),
        )
        .with_state(s.clone());
    let task = tokio::spawn(async move {
        axum::serve(l, app).await.unwrap();
    });
    (s, task)
}
fn gateway(pool: &PgPool, base: &str, exact: bool, official: bool) -> ModelGateway {
    let mut config:RoutingConfig=serde_json::from_value(json!({"version":1,"groups":[{"name":"allowed","enabled":true}],"aliases":[],"upstreams":[{"id":1,"endpoint":"https://api.openai.com/v1","protocol":"openai_responses","credential":{"id":7,"revision":1},"enabled":true}],"routes":[{"id":1,"model":PUBLIC,"group":"allowed","enabled":true,"speeds":["standard"],"targets":[{"id":1,"upstream_id":1,"upstream_model":MODEL,"enabled":true,"priority":0,"weight":1,"speeds":["standard"],"payload":"adapt","timeout_ms":10000}],"failover":{"max_attempts":1,"retry_on":[],"allow_replay_after_send":false}}]})).unwrap();
    if !official {
        config.upstreams[0].endpoint = Endpoint::local_loopback(base.into()).unwrap();
    }
    let prices:PriceConfig=serde_json::from_value(json!({"version":1,"rules":[{"id":1,"model":PUBLIC,"group":"allowed","speed":"standard","rates":{"input":6000000,"output":10000000,"cache_read":6000000,"cache_write":0,"request":0},"group_multiplier_ppm":1000000,"speed_multiplier_ppm":1000000}]})).unwrap();
    let context = ExternalModelContext {
        responses_compaction: true,
        upstream_id: 1,
        upstream_model: MODEL.into(),
        protocol: Protocol::OpenAiResponses,
        endpoint: "https://api.openai.com/v1".into(),
        max_input_tokens: 800,
        max_output_tokens: 50,
        contract_revision: "independent-official-profile-syntax-only-not-supplier-acceptance"
            .into(),
        missing_cached_tokens_is_zero: false,
        missing_cache_creation_tokens_is_zero: false,
        reservation: if exact {
            provider_policy::ReservationProfile::OpenaiResponsesExactV1 {
                count_timeout_ms: std::num::NonZeroU64::new(300).unwrap(),
            }
        } else {
            Default::default()
        },
        grounding: None,
        pre_acceptance_rejection_statuses: vec![],
    };
    ModelGateway::new_with_provider_policy(
        pool.clone(),
        Arc::new(
            routing::Router::new(config, Arc::new(PriceSnapshot::compile(prices).unwrap()))
                .unwrap(),
        ),
        vec![UpstreamCredential {
            reference: CredentialRef { id: 7, revision: 1 },
            secret: "independent-loopback-only".into(),
        }],
        BTreeMap::new(),
        Limits {
            max_output_tokens: 50,
            read_timeout: Duration::from_millis(800),
            total_timeout: Duration::from_secs(3),
            ..Default::default()
        },
        ChargePolicy {
            input_token_ceiling: 800,
            lease_seconds: 30,
        },
        ProviderPolicy::new(vec![context], 800)
            .expect("official exact context itself must be legal"),
    )
    .unwrap()
}
#[derive(Serialize, Deserialize)]
struct WorkerConfig {
    database: String,
    supplier: String,
    exact: bool,
    ready: String,
}
#[tokio::test]
#[ignore = "private child Core process only; parent owns disposable PG and loopback supplier"]
async fn compact_independent_core_worker() {
    let path = std::env::var("COMPACT_INDEPENDENT_WORKER_CONFIG").unwrap();
    let c: WorkerConfig = serde_json::from_slice(&fs::read(path).unwrap()).unwrap();
    let pool = PgPool::connect(&c.database).await.unwrap();
    let g = gateway(&pool, &c.supplier, c.exact, false);
    g.recover_expired(100).await.unwrap();
    g.reconcile_completed_streams_page(100, None).await.unwrap();
    let l = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    fs::write(c.ready, format!("http://{}", l.local_addr().unwrap())).unwrap();
    axum::serve(
        l,
        g.router()
            .into_make_service_with_connect_info::<SocketAddr>(),
    )
    .await
    .unwrap();
}
struct Core {
    child: Child,
    base: String,
    config: PathBuf,
    ready: PathBuf,
}
impl Core {
    async fn start(database: &str, supplier: &Supplier, exact: bool) -> Self {
        let nonce = format!("{:016x}", rand::random::<u64>());
        let config = artifacts().join(format!("worker-{nonce}.private.json"));
        let ready = artifacts().join(format!("worker-{nonce}.ready"));
        let c = WorkerConfig {
            database: database.into(),
            supplier: supplier.base.clone(),
            exact,
            ready: ready.to_str().unwrap().into(),
        };
        use std::io::Write;
        let mut f = fs::OpenOptions::new()
            .create_new(true)
            .write(true)
            .mode(0o600)
            .open(&config)
            .unwrap();
        f.write_all(&serde_json::to_vec(&c).unwrap()).unwrap();
        drop(f);
        let log = fs::OpenOptions::new()
            .create_new(true)
            .write(true)
            .mode(0o600)
            .open(artifacts().join(format!("worker-{nonce}.log")))
            .unwrap();
        let child = Command::new(std::env::current_exe().unwrap())
            .args([
                "--exact",
                "model_http::compact_independent::compact_independent_core_worker",
                "--ignored",
                "--nocapture",
                "--test-threads=1",
            ])
            .env("COMPACT_INDEPENDENT_WORKER_CONFIG", &config)
            .env_remove("DATABASE_URL")
            .stdout(Stdio::from(log.try_clone().unwrap()))
            .stderr(Stdio::from(log))
            .spawn()
            .unwrap();
        let mut core = Self {
            child,
            base: String::new(),
            config,
            ready,
        };
        for _ in 0..200 {
            if let Ok(base) = fs::read_to_string(&core.ready) {
                core.base = base;
                return core;
            }
            assert!(
                core.child.try_wait().unwrap().is_none(),
                "Core worker exited; inspect owned worker log"
            );
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
        panic!("Core child startup timeout")
    }
}
impl Drop for Core {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
        let _ = fs::remove_file(&self.config);
        let _ = fs::remove_file(&self.ready);
    }
}
fn http() -> reqwest::Client {
    reqwest::Client::builder()
        .no_proxy()
        .timeout(Duration::from_secs(8))
        .build()
        .unwrap()
}
async fn call(
    core: &Core,
    key: &str,
    nonce: &str,
    compact: bool,
    body: Value,
) -> reqwest::Response {
    http()
        .post(format!(
            "{}/v1/responses{}",
            core.base,
            if compact { "/compact" } else { "" }
        ))
        .bearer_auth(key)
        .header("idempotency-key", nonce)
        .json(&body)
        .send()
        .await
        .unwrap()
}
fn request(input: Value) -> Value {
    json!({"model":PUBLIC,"input":input})
}
fn stream_request(input: Value) -> Value {
    json!({"model":PUBLIC,"input":input,"store":false,"stream":true,"max_output_tokens":50,"tools":[{"type":"function","name":"audit_lookup","parameters":{"type":"object","properties":{"city":{"type":"string"}},"additionalProperties":false}}],"include":["reasoning.encrypted_content"],"reasoning":{"effort":"low"}})
}
async fn runtime(pool: &PgPool) -> RuntimeRole {
    let r = RuntimeRole::create(pool).await;
    r.optional("core_charging").await;
    r.optional("core_model").await;
    r
}
async fn row(pool: &PgPool, id: &str) -> (String, i64, i64) {
    sqlx::query_as("SELECT state,reserved,settled FROM core_charging.requests WHERE id=$1")
        .bind(id)
        .fetch_one(pool)
        .await
        .unwrap()
}
async fn checkpoint(pool: &PgPool, id: &str, bytes: &[u8]) -> (bool, Value) {
    let rows:Vec<(Vec<u8>,i64,bool)>=sqlx::query_as("SELECT packet_sha256,packet_bytes,terminal_observed FROM core_model.output_checkpoints WHERE request_id=$1 ORDER BY sequence").bind(id).fetch_all(pool).await.unwrap();
    let expected = Sha256::digest(bytes).to_vec();
    (
        rows.len() == 1 && rows[0].0 == expected && rows[0].1 == bytes.len() as i64,
        json!({"actual_sha256":crate::hex_codec::encode(&expected),"actual_len":bytes.len(),"stored":rows.iter().map(|r|json!({"sha256":crate::hex_codec::encode(&r.0),"len":r.1,"terminal":r.2})).collect::<Vec<_>>()}),
    )
}

#[sqlx::test(migrations = false)]
async fn compact_independent_exact_profile_must_reject_before_reserve_and_dispatch(pool: PgPool) {
    let f = native_http::test_support::fixture(pool, 20000, &[PUBLIC]).await;
    let r = runtime(&f.pool).await;
    let (s, task) = supplier().await;
    let official = gateway(&r.pool, &s.base, true, true);
    let mut plan = official
        .bridge
        .routing
        .route(crate::routing::RouteRequest {
            model: PUBLIC,
            group: "allowed",
            speed: None,
            protocol: Protocol::OpenAiResponses,
            replay: crate::routing::ReplayPermission::Denied,
        })
        .unwrap();
    let attempt = plan.start().unwrap();
    assert!(matches!(
        official
            .bridge
            .provider_policy
            .count_plan(&attempt)
            .unwrap(),
        crate::relay::input_count::CountPlan::OpenAiResponsesExactV1(_)
    ));
    let gate_rejects = official
        .bridge
        .provider_policy
        .require_compaction(&attempt)
        .is_err();
    let core = Core::start(&r.url, &s, true).await;
    let mut statuses = Vec::new();
    let mut observations = Vec::new();
    for (nonce, is_compact, input) in [
        ("independent-exact-compact", true, json!("text history")),
        ("independent-exact-opaque", false, json!([opaque()])),
    ] {
        let response = call(&core, &f.key, nonce, is_compact, request(input)).await;
        let status = response.status();
        let id = response
            .headers()
            .get("x-request-id")
            .map(|v| v.to_str().unwrap().to_owned())
            .unwrap_or_default();
        let _body = response.bytes().await.unwrap();
        statuses.push(status.is_client_error());
        let count: i64 =
            sqlx::query_scalar("SELECT count(*) FROM core_charging.requests WHERE id=$1")
                .bind(&id)
                .fetch_one(&f.pool)
                .await
                .unwrap();
        observations.push(json!({"id":id,"status":status.as_u16(),"row_count":count,"charge":if count>0 {Some(row(&f.pool,&id).await)}else{None}}));
    }
    let count: i64 = sqlx::query_scalar("SELECT count(*) FROM core_charging.requests")
        .fetch_one(&f.pool)
        .await
        .unwrap();
    record(
        "exact-profile",
        json!({"official_route_exact_plan_verified":true,"official_gate_rejects":gate_rejects,"local_transport_only":true,"observations":observations,"provider_calls":s.hits.load(Ordering::SeqCst),"count_calls":s.count_hits.load(Ordering::SeqCst),"charging_rows":count}),
    );
    drop(core);
    drop(official);
    task.abort();
    r.cleanup().await;
    assert!(
        gate_rejects
            && statuses.into_iter().all(|s| s)
            && count == 0
            && s.hits.load(Ordering::SeqCst) == 0,
        "exact profile silently admitted compact/opaque; see exact-profile.json"
    );
}

#[sqlx::test(migrations = false)]
async fn compact_independent_json_checkpoint_matches_actual_client_http_bytes(pool: PgPool) {
    let f = native_http::test_support::fixture(pool, 20000, &[PUBLIC]).await;
    let r = runtime(&f.pool).await;
    let (s, task) = supplier().await;
    *s.mode.lock().unwrap() = Mode::Pretty;
    let core = Core::start(&r.url, &s, false).await;
    let response = call(
        &core,
        &f.key,
        "independent-pretty-json",
        true,
        request(json!("summarize")),
    )
    .await;
    assert_eq!(response.status(), StatusCode::OK);
    let id = response.headers()["x-request-id"]
        .to_str()
        .unwrap()
        .to_owned();
    let bytes = response.bytes().await.unwrap();
    assert_eq!(
        serde_json::from_slice::<Value>(&bytes).unwrap()["object"],
        "response.compaction"
    );
    let (matches, evidence) = checkpoint(&f.pool, &id, &bytes).await;
    record(
        "json-packet",
        json!({"id":id,"checkpoint":evidence,"charge":row(&f.pool,&id).await,"provider_calls":s.hits.load(Ordering::SeqCst)}),
    );
    drop(core);
    task.abort();
    r.cleanup().await;
    assert!(
        matches,
        "stored JSON checkpoint differs from actual client HTTP body; see json-packet.json"
    );
}

#[sqlx::test(migrations = false)]
async fn compact_independent_sse_eof_tools_roundtrip_and_process_restart(pool: PgPool) {
    let f = native_http::test_support::fixture(pool, 20000, &[PUBLIC]).await;
    let r = runtime(&f.pool).await;
    let (s, task) = supplier().await;
    let core = Core::start(&r.url, &s, false).await;
    let compact = call(
        &core,
        &f.key,
        "independent-roundtrip-compact",
        true,
        request(json!("history")),
    )
    .await;
    assert_eq!(compact.status(), StatusCode::OK);
    let compact: Value = compact.json().await.unwrap();
    let mut items = compact["output"].as_array().unwrap().clone();
    items.push(json!({"role":"user","content":"use lookup"}));
    *s.mode.lock().unwrap() = Mode::Gate;
    let response = call(
        &core,
        &f.key,
        "independent-tools-sse",
        false,
        stream_request(json!(items)),
    )
    .await;
    assert_eq!(response.status(), StatusCode::OK);
    let id = response.headers()["x-request-id"]
        .to_str()
        .unwrap()
        .to_owned();
    let mut stream = response.bytes_stream();
    let mut wire = Vec::new();
    while !String::from_utf8_lossy(&wire).contains("function_call_arguments.delta") {
        wire.extend_from_slice(&stream.next().await.unwrap().unwrap());
    }
    s.entered.acquire().await.unwrap().forget();
    let before = row(&f.pool, &id).await;
    assert_eq!(before, ("streaming".into(), HOLD, 0));
    assert!(
        tokio::time::timeout(Duration::from_millis(150), stream.next())
            .await
            .is_err(),
        "terminal must not escape before actual body EOF"
    );
    assert!(!String::from_utf8_lossy(&wire).contains("response.completed"));
    s.release.add_permits(1);
    while let Some(chunk) = stream.next().await {
        wire.extend_from_slice(&chunk.unwrap());
    }
    assert_eq!(row(&f.pool, &id).await, ("settled".into(), HOLD, 36));
    let packets = wire.split_inclusive(|_| false).count();
    let _ = packets;
    let text = std::str::from_utf8(&wire).unwrap();
    let packets: Vec<_> = text.split_inclusive("\n\n").collect();
    let stored:Vec<(Vec<u8>,i64,bool)>=sqlx::query_as("SELECT packet_sha256,packet_bytes,terminal_observed FROM core_model.output_checkpoints WHERE request_id=$1 ORDER BY sequence").bind(&id).fetch_all(&f.pool).await.unwrap();
    assert_eq!(packets.len(), stored.len());
    for (packet, (digest, len, _)) in packets.iter().zip(&stored) {
        assert_eq!(*digest, Sha256::digest(packet.as_bytes()).to_vec());
        assert_eq!(*len, packet.len() as i64);
    }
    assert!(stored.last().unwrap().2);
    let terminal: Value = serde_json::from_str(
        packets
            .last()
            .unwrap()
            .split_once("data: ")
            .unwrap()
            .1
            .trim(),
    )
    .unwrap();
    items.extend(terminal["response"]["output"].as_array().unwrap().clone());
    items.push(json!({"type":"function_call_output","call_id":"call-independent","output":"weather=clear"}));
    *s.mode.lock().unwrap() = Mode::Normal;
    let reply = call(
        &core,
        &f.key,
        "independent-tools-json",
        false,
        request(json!(items)),
    )
    .await;
    assert_eq!(reply.status(), StatusCode::OK);
    let result: Value = reply.json().await.unwrap();
    assert_eq!(result["output"][0]["call_id"], "call-independent");
    let last = s.seen.lock().unwrap().last().unwrap().clone();
    assert!(
        last["input"]
            .as_array()
            .unwrap()
            .iter()
            .any(|v| v["type"] == "function_call_output" && v["output"] == "weather=clear")
    );
    assert_eq!(last["input"][0], opaque());
    let mut pending_ids = Vec::new();
    for (nonce, mode) in [
        ("independent-tail", Mode::Tail),
        ("independent-duplicate", Mode::Duplicate),
        ("independent-no-eof", Mode::Never),
    ] {
        *s.mode.lock().unwrap() = mode;
        let response = call(
            &core,
            &f.key,
            nonce,
            false,
            stream_request(json!([opaque()])),
        )
        .await;
        let id = response.headers()["x-request-id"]
            .to_str()
            .unwrap()
            .to_owned();
        let mut body = response.bytes_stream();
        let mut partial = Vec::new();
        let mut failed = false;
        s.entered.acquire().await.unwrap().forget();
        if !matches!(mode, Mode::Never) {
            s.release.add_permits(1);
        }
        while let Some(chunk) = body.next().await {
            match chunk {
                Ok(v) => partial.extend_from_slice(&v),
                Err(_) => {
                    failed = true;
                    break;
                }
            }
        }
        assert!(failed);
        assert!(!String::from_utf8_lossy(&partial).contains("event: response.completed"));
        assert_eq!(row(&f.pool, &id).await, ("reconcile".into(), HOLD, 0));
        pending_ids.push((nonce, id));
    }
    let balance = f.ledger.balance(f.account).await.unwrap();
    assert_eq!(
        (balance.available_units, balance.reserved_units),
        (20000 - 43 - 36 - 36 - 3 * HOLD, 3 * HOLD)
    );
    let hits = s.hits.load(Ordering::SeqCst);
    drop(core);
    let restarted = Core::start(&r.url, &s, false).await;
    for (nonce, id) in &pending_ids {
        let response = call(
            &restarted,
            &f.key,
            nonce,
            false,
            stream_request(json!([opaque()])),
        )
        .await;
        let _ = response.bytes().await;
        assert_eq!(row(&f.pool, id).await, ("reconcile".into(), HOLD, 0));
    }
    let repeated = call(
        &restarted,
        &f.key,
        "independent-roundtrip-compact",
        true,
        request(json!("history")),
    )
    .await;
    assert_eq!(repeated.status(), StatusCode::CONFLICT);
    let _ = repeated.bytes().await;
    assert_eq!(s.hits.load(Ordering::SeqCst), hits);
    let after = f.ledger.balance(f.account).await.unwrap();
    assert_eq!(
        (after.available_units, after.reserved_units),
        (balance.available_units, balance.reserved_units)
    );
    record(
        "sse-business",
        json!({"actual_packets":stored.len(),"terminal_after_eof":true,"checkpoint_sha_len_all_match":true,"pending":pending_ids,"provider_calls_before_restart":hits,"provider_calls_after_restart":s.hits.load(Ordering::SeqCst),"available":after.available_units,"reserved":after.reserved_units,"settled_total":115,"real_core_process_restart":true}),
    );
    drop(restarted);
    task.abort();
    r.cleanup().await;
}

#[sqlx::test(migrations = false)]
async fn compact_independent_same_key_revocation_and_original_wallet_budget(pool: PgPool) {
    let f = native_http::test_support::fixture(pool, HOLD, &[PUBLIC]).await;
    let r = runtime(&f.pool).await;
    let (s, task) = supplier().await;
    *s.mode.lock().unwrap() = Mode::HoldJson;
    let core = Core::start(&r.url, &s, false).await;
    let response = call(
        &core,
        &f.key,
        "independent-revoke-during",
        true,
        request(json!("admitted")),
    );
    let (response, ()) = tokio::join!(response, async {
        s.entered.acquire().await.unwrap().forget();
        let id: String = sqlx::query_scalar("SELECT id FROM core_charging.requests")
            .fetch_one(&f.pool)
            .await
            .unwrap();
        assert_eq!(row(&f.pool, &id).await, ("streaming".into(), HOLD, 0));
        sqlx::query(
            "UPDATE core_identity.credentials SET revoked_at=clock_timestamp() WHERE id=$1",
        )
        .bind(f.key_id)
        .execute(&f.pool)
        .await
        .unwrap();
        s.release.add_permits(1);
    });
    assert_eq!(response.status(), StatusCode::OK);
    let _ = response.bytes().await.unwrap();
    drop(core);
    let core = Core::start(&r.url, &s, false).await;
    assert_eq!(
        call(
            &core,
            &f.key,
            "independent-revoke-during",
            true,
            request(json!("admitted"))
        )
        .await
        .status(),
        StatusCode::UNAUTHORIZED
    );
    // A newly authorized key still uses the same original wallet, not a new allowance.
    let identity = IdentityStore::from_pool(f.pool.clone());
    let actor = identity.authorize(&f.session).await.unwrap();
    let key = identity
        .issue_key(
            &f.session,
            actor.owner,
            crate::funding::FundingPolicy::new_personal_key(),
            3600,
        )
        .await
        .unwrap();
    sqlx::query("INSERT INTO core_identity.api_key_settings(credential_id,name,group_name,model_limits_enabled,model_limits) VALUES($1,'independent-budget','allowed',TRUE,$2)").bind(key.id).bind(vec![PUBLIC]).execute(&f.pool).await.unwrap();
    let denied = call(
        &core,
        &key.secret,
        "independent-budget-insufficient",
        false,
        request(json!([opaque()])),
    )
    .await;
    assert!(denied.status().is_client_error());
    let _ = denied.bytes().await;
    let balance = f.ledger.balance(f.account).await.unwrap();
    assert_eq!(
        (balance.available_units, balance.reserved_units),
        (HOLD - 43, 0)
    );
    assert_eq!(s.hits.load(Ordering::SeqCst), 1);
    record(
        "revocation-budget",
        json!({"admitted_work_settled":43,"same_key_after_restart_rejected":true,"remaining_wallet":balance.available_units,"full_context_hold_required":HOLD,"insufficient_new_call_rejected":true,"provider_calls":1}),
    );
    drop(core);
    task.abort();
    r.cleanup().await;
}

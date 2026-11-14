//! Independent business assertions, prepared before the implementation freeze.
//! Only this file is tester-owned. Parent registration, production code, schema,
//! prefix-68 installation and commit/ACK fault adapters belong to the author.
//! Every network request below uses numeric loopback; money uses the real ledger.
use super::*;
use crate::routing::{
    CredentialRef, Endpoint, PriceConfig, PriceSnapshot, Protocol, RoutingConfig,
};
use crate::test_runtime::RuntimeRole;
use axum::extract::OriginalUri;
use futures_util::{FutureExt, StreamExt, future::BoxFuture};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::{
    collections::HashMap,
    fs,
    io::Write,
    os::unix::fs::OpenOptionsExt,
    panic::AssertUnwindSafe,
    path::PathBuf,
    process::{Child, Command, Stdio},
    sync::atomic::{AtomicUsize, Ordering},
};
use tokio::sync::Semaphore;
use tokio_util::sync::CancellationToken;

const MODEL: &str = "gpt-4.1-2025-04-14";
const PUBLIC: &str = "independent-continuity";
const HOLD: i64 = 2650; // Reviewed 800 input * 3 + 50 output * 5 Credits.
const COST: i64 = 36; // Supplier receipt: 7 input * 3 + 3 output * 5.
const CALL: &str = "call-preserve-independent";
const SECRET: &str = "independent-loopback-only";
const LEASE: i64 = 8;

fn artifacts() -> PathBuf {
    PathBuf::from(
        std::env::var("RESPONSES_CONTINUITY_INDEPENDENT_ARTIFACTS")
            .expect("independent runner must provide a new private artifact directory"),
    )
}
fn save(name: &str, value: &Value) {
    let mut file = fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .mode(0o600)
        .open(artifacts().join(format!("{name}.json")))
        .unwrap();
    file.write_all(&serde_json::to_vec_pretty(value).unwrap())
        .unwrap();
}
fn local_handle(id: &str) -> String {
    // Independent oracle from the public interface, not the production helper.
    format!(
        "resp_lmm_{}",
        crate::hex_codec::encode(Sha256::digest(id.as_bytes()))
    )
}
fn usage(input: u64) -> Value {
    json!({"input_tokens":input,"output_tokens":3,"total_tokens":input+3,
        "input_tokens_details":{"cached_tokens":0}})
}
fn tools() -> Value {
    json!([{"type":"function","name":"audit_lookup","parameters":{
        "type":"object","properties":{"city":{"type":"string"}},"additionalProperties":false}}])
}
fn request(input: Value) -> Value {
    json!({"model":PUBLIC,"input":input,"max_output_tokens":50,"tools":tools()})
}
fn stored(input: Value) -> Value {
    let mut v = request(input);
    v["store"] = true.into();
    v
}
fn following(handle: &str, input: Value) -> Value {
    let mut v = stored(input);
    v["previous_response_id"] = handle.into();
    v
}

#[derive(Clone, Copy)]
enum Mode {
    Normal,
    JsonGate,
    EofGate,
    Tail,
    Duplicate,
    Never,
    WrongStore,
}
struct ProviderState {
    seen: Vec<Value>,
    stored: HashMap<String, String>,
    opaque: HashMap<String, String>,
    forced_id: Option<String>,
    default_store: bool,
    input_usage: u64,
    mode: Mode,
}
#[derive(Clone)]
struct Supplier {
    base: String,
    state: Arc<Mutex<ProviderState>>,
    calls: Arc<AtomicUsize>,
    counts: Arc<AtomicUsize>,
    entered: Arc<Semaphore>,
    release: Arc<Semaphore>,
    stop: CancellationToken,
}
struct SupplierServer {
    supplier: Supplier,
    task: tokio::task::JoinHandle<()>,
}
impl Drop for SupplierServer {
    fn drop(&mut self) {
        self.supplier.stop.cancel();
        self.task.abort();
    }
}
impl Supplier {
    fn seen(&self) -> Vec<Value> {
        self.state.lock().unwrap().seen.clone()
    }
    fn mode(&self, mode: Mode) {
        self.state.lock().unwrap().mode = mode;
    }
    fn calls(&self) -> usize {
        self.calls.load(Ordering::SeqCst)
    }
    fn counts(&self) -> usize {
        self.counts.load(Ordering::SeqCst)
    }
    async fn entered(&self) {
        tokio::time::timeout(Duration::from_secs(4), self.entered.acquire())
            .await
            .expect("supplier did not reach the owned barrier")
            .unwrap()
            .forget();
    }
}
async fn supplier_reply(
    State(s): State<Supplier>,
    OriginalUri(uri): OriginalUri,
    headers: HeaderMap,
    Json(body): Json<Value>,
) -> Response {
    assert_eq!(headers[header::AUTHORIZATION], format!("Bearer {SECRET}"));
    let call = s.calls.fetch_add(1, Ordering::SeqCst) + 1;
    let compact = uri.path().ends_with("/compact");
    let prepared = {
        let mut state = s.state.lock().unwrap();
        state.seen.push(body.clone());
        let mut history = String::new();
        if let Some(parent) = body.get("previous_response_id").and_then(Value::as_str) {
            match state.stored.get(parent) {
                Some(prior) => history.push_str(prior),
                None => {
                    return (
                        StatusCode::NOT_FOUND,
                        Json(json!({"error":{
                    "message":"supplier has no retained parent","type":"invalid_request_error"}})),
                    )
                        .into_response();
                }
            }
        }
        if let Some(items) = body["input"].as_array() {
            for item in items {
                if let Some(cipher) = item.get("encrypted_content").and_then(Value::as_str) {
                    match state.opaque.get(cipher) {
                        Some(prior) => history.push_str(prior),
                        None => {
                            return (
                                StatusCode::BAD_REQUEST,
                                Json(json!({"error":{
                            "message":"unknown opaque context","type":"invalid_request_error"}})),
                            )
                                .into_response();
                        }
                    }
                }
            }
        }
        history.push('|');
        history.push_str(&body["input"].to_string());
        let provider_id = state
            .forced_id
            .clone()
            .unwrap_or_else(|| format!("resp_supplier_{call}"));
        let effective_store = body
            .get("store")
            .and_then(Value::as_bool)
            .unwrap_or(state.default_store);
        let receipt_store = if matches!(state.mode, Mode::WrongStore) {
            !effective_store
        } else {
            effective_store
        };
        let output = if compact {
            let cipher = format!("{}:{call}", "opaque-independent-λ".repeat(2000));
            state.opaque.insert(cipher.clone(), history.clone());
            json!({"id":format!("cmp_supplier_{call}"),"object":"response.compaction",
                "output":[{"id":"opaque-item-preserve","type":"compaction","encrypted_content":cipher}],
                "usage":usage(state.input_usage)})
        } else {
            if effective_store {
                state.stored.insert(provider_id.clone(), history.clone());
            }
            // Item ID and argument intentionally contain the envelope's original
            // ID. Recursive/textual ID replacement would corrupt these values.
            json!({"id":provider_id,"object":"response","status":"completed","model":body["model"],
                "store":receipt_store,"output":[{"type":"function_call","id":provider_id,
                    "call_id":CALL,"name":"audit_lookup","arguments":json!({"provider_id":provider_id,"city":"台北"}).to_string()},
                    {"type":"message","id":"msg-preserve","role":"assistant","status":"completed",
                    "content":[{"type":"output_text","text":history.chars().take(350).collect::<String>(),"annotations":[]}]}],
                "usage":usage(state.input_usage)})
        };
        (output, provider_id, state.mode)
    };
    let (output, provider_id, mode) = prepared;
    if body["stream"] != true {
        if matches!(mode, Mode::JsonGate) {
            s.entered.add_permits(1);
            tokio::select! {
                permit = s.release.acquire() => permit.unwrap().forget(),
                _ = s.stop.cancelled() => return StatusCode::SERVICE_UNAVAILABLE.into_response(),
            }
        }
        // Noncanonical supplier JSON catches checkpoints of the wrong encoding.
        return (
            [(header::CONTENT_TYPE, "application/json")],
            serde_json::to_vec_pretty(&output).unwrap(),
        )
            .into_response();
    }
    let created =
        json!({"type":"response.created","response":{"id":provider_id,"status":"in_progress"}});
    let delta = json!({"type":"response.function_call_arguments.delta","response_id":provider_id,
        "item_id":provider_id,"output_index":0,"delta":output["output"][0]["arguments"]});
    let terminal = json!({"type":"response.completed","response":output});
    let early = format!(
        "event: response.created\ndata: {created}\n\nevent: response.function_call_arguments.delta\ndata: {delta}\n\n"
    );
    let last = format!("event: response.completed\ndata: {terminal}\n\n");
    let (tx, rx) = tokio::sync::mpsc::channel::<Result<Bytes, std::io::Error>>(8);
    tokio::spawn(async move {
        let wire = format!("{early}{last}");
        for chunk in wire.as_bytes().chunks(113) {
            if tx.send(Ok(Bytes::copy_from_slice(chunk))).await.is_err() {
                return;
            }
        }
        if matches!(
            mode,
            Mode::EofGate | Mode::Tail | Mode::Duplicate | Mode::Never
        ) {
            s.entered.add_permits(1);
            if matches!(mode, Mode::Never) {
                s.stop.cancelled().await;
                return;
            }
            tokio::select! {
                permit = s.release.acquire() => permit.unwrap().forget(),
                _ = s.stop.cancelled() => return,
            }
        }
        let tail = match mode {
            Mode::Tail => "INVALID AFTER TERMINAL".to_owned(),
            Mode::Duplicate => last,
            _ => String::new(),
        };
        if !tail.is_empty() {
            let _ = tx.send(Ok(Bytes::from(tail))).await;
        }
        // Only dropping tx is actual supplier HTTP body EOF.
    });
    (
        [(header::CONTENT_TYPE, "text/event-stream")],
        Body::from_stream(tokio_stream::wrappers::ReceiverStream::new(rx)),
    )
        .into_response()
}
async fn supplier() -> SupplierServer {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let s = Supplier {
        base: format!("http://{}/v1", listener.local_addr().unwrap()),
        state: Arc::new(Mutex::new(ProviderState {
            seen: Vec::new(),
            stored: HashMap::new(),
            opaque: HashMap::new(),
            forced_id: None,
            default_store: true,
            input_usage: 7,
            mode: Mode::Normal,
        })),
        calls: Arc::new(AtomicUsize::new(0)),
        counts: Arc::new(AtomicUsize::new(0)),
        entered: Arc::new(Semaphore::new(0)),
        release: Arc::new(Semaphore::new(0)),
        stop: CancellationToken::new(),
    };
    let counts = s.counts.clone();
    let app = Router::new()
        .route("/v1/responses", post(supplier_reply))
        .route("/v1/responses/compact", post(supplier_reply))
        .route(
            "/v1/responses/input_tokens",
            post(move || {
                let counts = counts.clone();
                async move {
                    counts.fetch_add(1, Ordering::SeqCst);
                    Json(json!({"input_tokens":1}))
                }
            }),
        )
        .with_state(s.clone());
    let task = tokio::spawn(async move {
        axum::serve(listener, app).await.unwrap();
    });
    SupplierServer { supplier: s, task }
}

#[derive(Clone, Serialize, Deserialize)]
struct WorkerConfig {
    database: String,
    suppliers: Vec<String>,
    targets: Vec<usize>,
    revisions: Vec<u64>,
    models: Vec<String>,
    default_store: Option<bool>,
    exact: bool,
    multiplier: i64,
    ready: String,
}
impl WorkerConfig {
    fn new(lab: &Lab) -> Self {
        Self {
            database: lab.runtime.url.clone(),
            suppliers: lab
                .servers
                .iter()
                .map(|s| s.supplier.base.clone())
                .collect(),
            targets: vec![0],
            revisions: vec![1, 1],
            models: vec![MODEL.into(), MODEL.into()],
            default_store: Some(true),
            exact: false,
            multiplier: 1,
            ready: String::new(),
        }
    }
}
fn gateway(pool: &PgPool, config: &WorkerConfig, official: bool) -> ModelGateway {
    let upstreams: Vec<_> = config
        .suppliers
        .iter()
        .enumerate()
        .map(|(i, _)| {
            json!({
        "id":i+1,"endpoint":"https://api.openai.com/v1","protocol":"openai_responses",
        "credential":{"id":i+7,"revision":config.revisions[i]},"enabled":true})
        })
        .collect();
    let targets: Vec<_> = config
        .targets
        .iter()
        .enumerate()
        .map(|(priority, i)| {
            json!({
        "id":i+1,"upstream_id":i+1,"upstream_model":config.models[*i],"enabled":true,
        "priority":priority,"weight":1,"speeds":["standard"],"payload":"adapt","timeout_ms":3000})
        })
        .collect();
    let mut routes:RoutingConfig = serde_json::from_value(json!({"version":1,"groups":[{"name":"allowed","enabled":true}],
        "aliases":[],"upstreams":upstreams,"routes":[{"id":1,"model":PUBLIC,"group":"allowed","enabled":true,
        "speeds":["standard"],"targets":targets,"failover":{"max_attempts":1,"retry_on":[],"allow_replay_after_send":false}}]})).unwrap();
    if !official {
        for (i, upstream) in routes.upstreams.iter_mut().enumerate() {
            upstream.endpoint = Endpoint::local_loopback(config.suppliers[i].clone()).unwrap();
        }
    }
    let prices:PriceConfig = serde_json::from_value(json!({"version":1,"rules":[{"id":1,"model":PUBLIC,"group":"allowed",
        "speed":"standard","rates":{"input":6000000*config.multiplier,"output":10000000*config.multiplier,
        "cache_read":6000000*config.multiplier,"cache_write":0,"request":0},"group_multiplier_ppm":1000000,"speed_multiplier_ppm":1000000}]})).unwrap();
    let contexts = config.suppliers.iter().enumerate().map(|(i,_)| {
        let mut context = json!({"upstream_id":i+1,"upstream_model":config.models[i],"protocol":"openai_responses",
            "endpoint":"https://api.openai.com/v1","max_input_tokens":800,"max_output_tokens":50,
            "contract_revision":"independent-continuity-loopback-contract-v1","responses_compaction":true,
            "missing_cached_tokens_is_zero":false,"missing_cache_creation_tokens_is_zero":false});
        if let Some(default_store) = config.default_store { context["responses_continuity"] = json!({"default_store":default_store}); }
        let mut context:ExternalModelContext = serde_json::from_value(context).unwrap();
        // Typed profile avoids guessing serde enum spelling. Official HTTPS
        // contract is admitted separately; all dispatched transports are local.
        context.reservation = if config.exact { provider_policy::ReservationProfile::OpenaiResponsesExactV1 {
            count_timeout_ms:std::num::NonZeroU64::new(300).unwrap() } } else { Default::default() };
        context
    }).collect();
    ModelGateway::new_with_provider_policy(
        pool.clone(),
        Arc::new(
            crate::routing::Router::new(routes, Arc::new(PriceSnapshot::compile(prices).unwrap()))
                .unwrap(),
        ),
        config
            .suppliers
            .iter()
            .enumerate()
            .map(|(i, _)| UpstreamCredential {
                reference: CredentialRef {
                    id: (i + 7) as u64,
                    revision: config.revisions[i],
                },
                secret: SECRET.into(),
            })
            .collect(),
        BTreeMap::new(),
        Limits {
            max_output_tokens: 50,
            read_timeout: Duration::from_millis(1200),
            total_timeout: Duration::from_secs(3),
            hook_timeout: Duration::from_millis(500),
            ..Default::default()
        },
        ChargePolicy {
            input_token_ceiling: 800,
            lease_seconds: LEASE,
        },
        ProviderPolicy::new(contexts, 800).unwrap(),
    )
    .unwrap()
}

#[tokio::test]
#[ignore = "private Core HTTP subprocess only; no direct test-runner invocation"]
async fn responses_continuity_independent_core_worker() {
    let path = std::env::var("RESPONSES_CONTINUITY_INDEPENDENT_WORKER_CONFIG").unwrap();
    let config: WorkerConfig = serde_json::from_slice(&fs::read(path).unwrap()).unwrap();
    let pool = PgPool::connect(&config.database).await.unwrap();
    let gateway = gateway(&pool, &config, false);
    gateway.recover_expired(100).await.unwrap();
    gateway
        .reconcile_completed_streams_page(100, None)
        .await
        .unwrap();
    if config.default_store.is_some() {
        gateway
            .reconcile_responses_continuity_page(100, None)
            .await
            .unwrap();
    }
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let pending_ready = PathBuf::from(&config.ready).with_extension("new");
    fs::write(
        &pending_ready,
        format!("http://{}", listener.local_addr().unwrap()),
    )
    .unwrap();
    fs::rename(pending_ready, &config.ready).unwrap();
    axum::serve(
        listener,
        gateway
            .router()
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
    async fn start(mut config: WorkerConfig, crash: Option<&str>) -> Self {
        let nonce = format!("{:016x}", rand::random::<u64>());
        let path = artifacts().join(format!("continuity-worker-{nonce}.private.json"));
        let ready = artifacts().join(format!("continuity-worker-{nonce}.ready"));
        config.ready = ready.to_str().unwrap().into();
        let mut file = fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .mode(0o600)
            .open(&path)
            .unwrap();
        file.write_all(&serde_json::to_vec(&config).unwrap())
            .unwrap();
        drop(file);
        let log = fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .mode(0o600)
            .open(artifacts().join(format!("continuity-worker-{nonce}.log")))
            .unwrap();
        let mut command = Command::new(std::env::current_exe().unwrap());
        command.args(["--exact","model_http::responses_continuity_independent::responses_continuity_independent_core_worker",
            "--ignored","--nocapture","--test-threads=1"])
            .env("RESPONSES_CONTINUITY_INDEPENDENT_WORKER_CONFIG",&path)
            .env_remove("DATABASE_URL").env_remove("LMM_RESPONSES_CONTINUITY_CRASH_AT")
            .stdout(Stdio::from(log.try_clone().unwrap())).stderr(Stdio::from(log));
        if let Some(crash) = crash {
            command.env("LMM_RESPONSES_CONTINUITY_CRASH_AT", crash);
        }
        let mut core = Self {
            child: command.spawn().unwrap(),
            base: String::new(),
            config: path,
            ready,
        };
        for _ in 0..250 {
            if let Ok(base) = fs::read_to_string(&core.ready) {
                core.base = base;
                return core;
            }
            assert!(
                core.child.try_wait().unwrap().is_none(),
                "Core startup failed; retain owned worker log"
            );
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
        panic!("Core startup timed out; retain owned worker log")
    }
    async fn exited97(&mut self) {
        for _ in 0..250 {
            if let Some(status) = self.child.try_wait().unwrap() {
                assert_eq!(status.code(), Some(97));
                return;
            }
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
        panic!("contracted OS crash hook did not exit97")
    }
}
impl Drop for Core {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
        let _ = fs::remove_file(&self.config);
        let _ = fs::remove_file(&self.ready);
        let _ = fs::remove_file(self.ready.with_extension("new"));
    }
}
fn http() -> reqwest::Client {
    reqwest::Client::builder()
        .no_proxy()
        .timeout(Duration::from_secs(8))
        .build()
        .unwrap()
}
async fn send(
    core: &Core,
    key: &str,
    nonce: &str,
    body: &Value,
    compact: bool,
) -> Result<reqwest::Response, reqwest::Error> {
    http()
        .post(format!(
            "{}/v1/responses{}",
            core.base,
            if compact { "/compact" } else { "" }
        ))
        .bearer_auth(key)
        .header("idempotency-key", nonce)
        .json(body)
        .send()
        .await
}
async fn call(core: &Core, key: &str, nonce: &str, body: &Value) -> reqwest::Response {
    send(core, key, nonce, body, false).await.unwrap()
}
async fn json_reply(response: reqwest::Response) -> (String, Value, Vec<u8>) {
    assert_eq!(response.status(), StatusCode::OK);
    let id = response.headers()["x-request-id"]
        .to_str()
        .unwrap()
        .to_owned();
    let bytes = response.bytes().await.unwrap().to_vec();
    let value = serde_json::from_slice(&bytes).unwrap();
    (id, value, bytes)
}
struct Lab {
    fixture: native_http::test_support::Fixture,
    runtime: RuntimeRole,
    servers: Vec<SupplierServer>,
}
impl Lab {
    async fn new(pool: PgPool, funds: i64) -> Self {
        let fixture = native_http::test_support::fixture(pool, funds, &[PUBLIC]).await;
        let runtime = RuntimeRole::create(&fixture.pool).await;
        runtime.optional("core_charging").await;
        runtime.optional("core_model").await;
        Self {
            fixture,
            runtime,
            servers: vec![supplier().await, supplier().await],
        }
    }
    fn s(&self) -> &Supplier {
        &self.servers[0].supplier
    }
    async fn cleanup(self) {
        drop(self.servers);
        self.runtime.cleanup().await;
    }
}
async fn scenario<F>(pool: PgPool, funds: i64, run: F)
where
    F: for<'a> FnOnce(&'a Lab) -> BoxFuture<'a, ()>,
{
    let lab = Lab::new(pool, funds).await;
    let result = AssertUnwindSafe(run(&lab)).catch_unwind().await;
    lab.cleanup().await;
    if let Err(panic) = result {
        std::panic::resume_unwind(panic);
    }
}
async fn charge(pool: &PgPool, id: &str) -> (String, i64, i64) {
    sqlx::query_as("SELECT state,reserved,settled FROM core_charging.requests WHERE id=$1")
        .bind(id)
        .fetch_one(pool)
        .await
        .unwrap()
}
async fn mapping(pool: &PgPool, id: &str) -> Option<Value> {
    sqlx::query_scalar(
        "SELECT to_jsonb(c) FROM core_model.responses_continuations c WHERE request_id=$1",
    )
    .bind(id)
    .fetch_optional(pool)
    .await
    .unwrap()
}
async fn snapshot(pool: &PgPool) -> Value {
    sqlx::query_scalar("SELECT jsonb_build_object('requests',(SELECT coalesce(jsonb_agg(to_jsonb(r) ORDER BY id),'[]'::jsonb) FROM core_charging.requests r),'maps',(SELECT coalesce(jsonb_agg(to_jsonb(c) ORDER BY request_id),'[]'::jsonb) FROM core_model.responses_continuations c),'journals',(SELECT count(*) FROM core_billing.ledger_journals),'usage',(SELECT count(*) FROM core_charging.usage_events),'balances',(SELECT coalesce(jsonb_agg(to_jsonb(b) ORDER BY ledger_account_id),'[]'::jsonb) FROM core_billing.balance_state b))")
        .fetch_one(pool).await.unwrap()
}
async fn assert_packets(pool: &PgPool, id: &str, packets: &[&[u8]]) {
    let stored:Vec<(Vec<u8>,i64,bool)>=sqlx::query_as("SELECT packet_sha256,packet_bytes,terminal_observed FROM core_model.output_checkpoints WHERE request_id=$1 ORDER BY sequence")
        .bind(id).fetch_all(pool).await.unwrap();
    assert_eq!(stored.len(), packets.len(), "actual emitted packet count");
    for ((digest, len, _), actual) in stored.iter().zip(packets) {
        assert_eq!(
            *digest,
            Sha256::digest(actual).to_vec(),
            "actual emitted packet SHA"
        );
        assert_eq!(
            *len,
            actual.len() as i64,
            "actual emitted packet byte length"
        );
    }
    assert!(
        stored.last().unwrap().2,
        "last emitted packet must have trusted terminal evidence"
    );
}
fn assert_item_identity(body: &Value, provider: &str) {
    assert_eq!(body["output"][0]["id"], provider);
    assert_eq!(body["output"][0]["call_id"], CALL);
    let args: Value =
        serde_json::from_str(body["output"][0]["arguments"].as_str().unwrap()).unwrap();
    assert_eq!(args["provider_id"], provider);
    assert_eq!(args["city"], "台北");
}
async fn assert_active(lab: &Lab, id: &str, body: &Value) {
    assert_eq!(body["id"], local_handle(id));
    let map = mapping(&lab.fixture.pool, id)
        .await
        .expect("stored response must have durable mapping");
    assert_eq!(map["handle"], body["id"]);
    assert_eq!(map["active"], true);
    assert_eq!(
        charge(&lab.fixture.pool, id).await,
        ("settled".into(), HOLD, COST)
    );
}
async fn denied(core: &Core, key: &str, nonce: &str, body: &Value) -> (u16, Value) {
    let response = call(core, key, nonce, body).await;
    let status = response.status();
    let bytes = response.bytes().await.unwrap();
    assert!(
        status.is_client_error(),
        "denial must be explicit client error, got {status}"
    );
    let value: Value = serde_json::from_slice(&bytes).unwrap();
    (status.as_u16(), value)
}
async fn key_for(lab: &Lab, session: &str) -> String {
    let identity = IdentityStore::from_pool(lab.fixture.pool.clone());
    let actor = identity.authorize(session).await.unwrap();
    let key = identity
        .issue_key(
            session,
            actor.owner,
            crate::funding::FundingPolicy::new_personal_key(),
            3600,
        )
        .await
        .unwrap();
    sqlx::query("INSERT INTO core_identity.api_key_settings(credential_id,name,group_name,model_limits_enabled,model_limits) VALUES($1,'independent-continuity','allowed',TRUE,$2)")
        .bind(key.id).bind(vec![PUBLIC]).execute(&lab.fixture.pool).await.unwrap();
    key.secret
}

#[sqlx::test(migrations = false)]
async fn responses_continuity_independent_json_tools_and_actual_packet(pool: PgPool) {
    scenario(pool,20000,|lab| Box::pin(async move {
        let core=Core::start(WorkerConfig::new(lab),None).await;
        let mut body=stored(json!("history-only-secret-A"));
        body["instructions"]="ROOT-INSTRUCTION-NOT-INHERITED".into();
        let (id,root,bytes)=json_reply(call(&core,&lab.fixture.key,"independent-json-root",&body).await).await;
        assert_active(lab,&id,&root).await;
        assert_item_identity(&root,"resp_supplier_1");
        assert_packets(&lab.fixture.pool,&id,&[&bytes]).await;
        let next=following(root["id"].as_str().unwrap(),json!([{
            "type":"function_call_output","call_id":CALL,"output":"weather=clear-台北"}]));
        let (next_id,result,next_bytes)=json_reply(call(&core,&lab.fixture.key,"independent-json-tools",&next).await).await;
        assert_active(lab,&next_id,&result).await;
        assert_packets(&lab.fixture.pool,&next_id,&[&next_bytes]).await;
        let seen=lab.s().seen();
        assert_eq!(seen[1]["previous_response_id"],"resp_supplier_1");
        assert_eq!(seen[1]["input"],next["input"]);
        assert!(seen[1].get("instructions").is_none(),"prior instructions must not be inherited");
        assert!(result["output"][1]["content"][0]["text"].as_str().unwrap().contains("history-only-secret-A"));
        assert_eq!(mapping(&lab.fixture.pool,&next_id).await.unwrap()["parent_request_id"],id);
        assert_eq!(lab.s().counts(),0);
        save("a1-json",&json!({"root":id,"child":next_id,"actual_sha":crate::hex_codec::encode(Sha256::digest(&bytes)),
            "len":bytes.len(),"provider_parent":seen[1]["previous_response_id"],"charge_each":COST}));
    })).await;
}

#[sqlx::test(migrations = false)]
async fn responses_continuity_independent_sse_pending_eof_and_packet_identity(pool: PgPool) {
    scenario(pool,20000,|lab| Box::pin(async move {
        let core=Core::start(WorkerConfig::new(lab),None).await;
        lab.s().mode(Mode::EofGate);
        let mut body=stored(json!("stream-hidden-history")); body["stream"]=true.into();
        let response=call(&core,&lab.fixture.key,"independent-sse-root",&body).await;
        assert_eq!(response.status(),StatusCode::OK);
        let id=response.headers()["x-request-id"].to_str().unwrap().to_owned();
        let handle=local_handle(&id);
        let mut stream=response.bytes_stream(); let mut wire=Vec::new();
        while !String::from_utf8_lossy(&wire).contains("function_call_arguments.delta") {
            wire.extend_from_slice(&stream.next().await.unwrap().unwrap());
        }
        lab.s().entered().await;
        assert_eq!(charge(&lab.fixture.pool,&id).await,("streaming".into(),HOLD,0));
        assert!(mapping(&lab.fixture.pool,&id).await.is_none());
        let before=snapshot(&lab.fixture.pool).await;
        let pending=denied(&core,&lab.fixture.key,"independent-pending-parent",&following(&handle,json!("too early"))).await;
        let absent=denied(&core,&lab.fixture.key,"independent-absent-parent",&following(&local_handle("absent"),json!("too early"))).await;
        assert_eq!(pending,absent,"early visible SSE handle must not reveal candidate eligibility");
        assert_eq!(snapshot(&lab.fixture.pool).await,before);
        assert_eq!(lab.s().calls(),1);
        assert!(tokio::time::timeout(Duration::from_millis(150),stream.next()).await.is_err());
        assert!(!String::from_utf8_lossy(&wire).contains("response.completed"));
        lab.s().release.add_permits(1);
        while let Some(chunk)=stream.next().await { wire.extend_from_slice(&chunk.unwrap()); }
        let text=std::str::from_utf8(&wire).unwrap();
        let packets:Vec<_>=text.split_inclusive("\n\n").map(str::as_bytes).collect();
        assert_packets(&lab.fixture.pool,&id,&packets).await;
        let events:Vec<Value>=text.split("\n\n").filter(|s|!s.is_empty()).map(|packet| {
            serde_json::from_str(packet.lines().find_map(|line|line.strip_prefix("data: ")).unwrap()).unwrap()
        }).collect();
        assert_eq!(events[0]["response"]["id"],handle);
        assert_eq!(events[1]["response_id"],handle);
        assert_eq!(events[1]["item_id"],"resp_supplier_1");
        let terminal=&events.last().unwrap()["response"];
        assert_active(lab,&id,terminal).await;
        assert_item_identity(terminal,"resp_supplier_1");
        save("a1-a2-sse",&json!({"request":id,"handle":handle,"packets":packets.len(),"actual_eof_required":true,
            "pending_denial":pending,"actual_body_sha":crate::hex_codec::encode(Sha256::digest(&wire)),"len":wire.len()}));
    })).await;
}

#[sqlx::test(migrations = false)]
async fn responses_continuity_independent_scope_and_live_key_authorization(pool: PgPool) {
    scenario(pool,20000,|lab| Box::pin(async move {
        let core=Core::start(WorkerConfig::new(lab),None).await;
        let (_,root,_)=json_reply(call(&core,&lab.fixture.key,"independent-scope-root",&stored(json!("scope-secret"))).await).await;
        let handle=root["id"].as_str().unwrap();
        let same_user=key_for(lab,&lab.fixture.session).await;
        let identity=IdentityStore::from_pool(lab.fixture.pool.clone());
        let other=identity.bootstrap_user(8,1).await.unwrap();
        sqlx::query("INSERT INTO core_identity.user_routing_groups(user_id,group_name,enabled,source_reference) VALUES(8,'allowed',TRUE,'independent-continuity')")
            .execute(&lab.fixture.pool).await.unwrap();
        let other_user=key_for(lab,&other.secret).await;
        let before=snapshot(&lab.fixture.pool).await;
        for (label,key) in [("same-user",&same_user),("other-user",&other_user)] {
            let known=denied(&core,key,&format!("independent-{label}-known"),&following(handle,json!("probe"))).await;
            let unknown=denied(&core,key,&format!("independent-{label}-unknown"),&following(&local_handle("unknown"),json!("probe"))).await;
            assert_eq!(known,unknown,"authorized caller must not distinguish foreign ID from unknown");
            assert!(!known.1.to_string().contains("scope-secret"));
        }
        assert_eq!(snapshot(&lab.fixture.pool).await,before);
        denied(&core,&lab.fixture.session,"independent-session-not-key",&following(handle,json!("API key required"))).await;
        assert_eq!(snapshot(&lab.fixture.pool).await,before);
        assert_eq!(lab.s().calls(),1);
        for (label,change,restore) in [
            ("model","UPDATE core_identity.api_key_settings SET model_limits=ARRAY['other-model'] WHERE credential_id=$1",
                "UPDATE core_identity.api_key_settings SET model_limits=ARRAY['independent-continuity'] WHERE credential_id=$1"),
            ("ip","UPDATE core_identity.api_key_settings SET allow_ips=ARRAY['192.0.2.7'::inet] WHERE credential_id=$1",
                "UPDATE core_identity.api_key_settings SET allow_ips='{}'::inet[] WHERE credential_id=$1"),
            ("group","UPDATE core_identity.user_routing_groups SET enabled=false WHERE user_id=(SELECT user_id FROM core_identity.credentials WHERE id=$1)",
                "UPDATE core_identity.user_routing_groups SET enabled=true WHERE user_id=(SELECT user_id FROM core_identity.credentials WHERE id=$1)"),
        ] {
            sqlx::query(change).bind(lab.fixture.key_id).execute(&lab.fixture.pool).await.unwrap();
            let before=snapshot(&lab.fixture.pool).await;
            denied(&core,&lab.fixture.key,&format!("independent-live-{label}"),&following(handle,json!("current authorization required"))).await;
            assert_eq!(snapshot(&lab.fixture.pool).await,before);
            assert_eq!(lab.s().calls(),1);
            sqlx::query(restore).bind(lab.fixture.key_id).execute(&lab.fixture.pool).await.unwrap();
        }
        sqlx::query("UPDATE core_identity.credentials SET revoked_at=clock_timestamp() WHERE id=$1")
            .bind(lab.fixture.key_id).execute(&lab.fixture.pool).await.unwrap();
        let revoked_before=snapshot(&lab.fixture.pool).await;
        for parent in [handle,local_handle("unknown").as_str()] {
            let response=call(&core,&lab.fixture.key,"independent-revoked-parent",&following(parent,json!("probe"))).await;
            assert_eq!(response.status(),StatusCode::UNAUTHORIZED);
            let _=response.bytes().await.unwrap();
        }
        // The original completed nonce is also reauthorized before replay.
        assert_eq!(call(&core,&lab.fixture.key,"independent-scope-root",&stored(json!("scope-secret"))).await.status(),StatusCode::UNAUTHORIZED);
        assert_eq!(snapshot(&lab.fixture.pool).await,revoked_before);
        assert_eq!((lab.s().calls(),lab.s().counts()),(1,0));
        save("a2-scopes",&json!({"uniform_foreign_and_unknown":true,"revoked_replay_status":401,
            "supplier_calls":lab.s().calls(),"count_calls":lab.s().counts(),"money_unchanged":true}));
    })).await;
}

#[sqlx::test(migrations = false)]
async fn responses_continuity_independent_equal_provider_ids_and_binding_changes(pool: PgPool) {
    scenario(pool, 40000, |lab| {
        Box::pin(async move {
            for s in &lab.servers {
                s.supplier.state.lock().unwrap().forced_id =
                    Some("resp_same_at_two_providers".into());
            }
            let config = WorkerConfig::new(lab);
            let core = Core::start(config.clone(), None).await;
            let (a_id, a, _) = json_reply(
                call(
                    &core,
                    &lab.fixture.key,
                    "independent-collision-a",
                    &stored(json!("only-provider-A-knows")),
                )
                .await,
            )
            .await;
            drop(core);
            let mut only_b = config.clone();
            only_b.targets = vec![1];
            let core = Core::start(only_b, None).await;
            let (b_id, b, _) = json_reply(
                call(
                    &core,
                    &lab.fixture.key,
                    "independent-collision-b",
                    &stored(json!("only-provider-B-knows")),
                )
                .await,
            )
            .await;
            assert_ne!(a["id"], b["id"]);
            assert_ne!(a_id, b_id);
            assert_eq!(
                mapping(&lab.fixture.pool, &a_id).await.unwrap()["provider_response_id"],
                "resp_same_at_two_providers"
            );
            assert_eq!(
                mapping(&lab.fixture.pool, &b_id).await.unwrap()["provider_response_id"],
                "resp_same_at_two_providers"
            );
            drop(core);
            let mut rebalance = config.clone();
            rebalance.targets = vec![1, 0];
            let core = Core::start(rebalance, None).await;
            let b_calls = lab.servers[1].supplier.calls();
            let a_calls = lab.s().calls();
            let before_rebalance = snapshot(&lab.fixture.pool).await;
            let response = call(
                &core,
                &lab.fixture.key,
                "independent-rebalanced-parent",
                &following(a["id"].as_str().unwrap(), json!("resume A")),
            )
            .await;
            if response.status().is_success() {
                let (_, result, _) = json_reply(response).await;
                let history = result["output"][1]["content"][0]["text"].as_str().unwrap();
                assert!(history.contains("only-provider-A-knows"));
                assert!(!history.contains("only-provider-B-knows"));
            } else {
                assert!(response.status().is_client_error());
                let _ = response.bytes().await;
                assert_eq!(snapshot(&lab.fixture.pool).await, before_rebalance);
                assert_eq!(lab.s().calls(), a_calls);
            }
            assert_eq!(
                lab.servers[1].supplier.calls(),
                b_calls,
                "rebalance cannot substitute provider B"
            );
            drop(core);
            for (label, variant) in [
                ("revision", {
                    let mut c = config.clone();
                    c.revisions[0] = 2;
                    c
                }),
                ("disable-original", {
                    let mut c = config.clone();
                    c.targets = vec![1];
                    c
                }),
                ("endpoint", {
                    let mut c = config.clone();
                    c.suppliers[0] = c.suppliers[1].clone();
                    c
                }),
                ("model-remap", {
                    let mut c = config.clone();
                    c.models[0] = "gpt-4.1-mini-2025-04-14".into();
                    c
                }),
                ("contract-drift", {
                    let mut c = config.clone();
                    c.default_store = Some(false);
                    c
                }),
            ] {
                let before = snapshot(&lab.fixture.pool).await;
                let calls: usize = lab.servers.iter().map(|s| s.supplier.calls()).sum();
                let core = Core::start(variant, None).await;
                denied(
                    &core,
                    &lab.fixture.key,
                    &format!("independent-binding-{label}"),
                    &following(a["id"].as_str().unwrap(), json!("no substitution")),
                )
                .await;
                assert_eq!(
                    snapshot(&lab.fixture.pool).await,
                    before,
                    "binding change rejected before reserve"
                );
                assert_eq!(
                    lab.servers
                        .iter()
                        .map(|s| s.supplier.calls())
                        .sum::<usize>(),
                    calls
                );
            }
            save(
                "a3-binding",
                &json!({"local_a":a["id"],"local_b":b["id"],"provider_id_collision":true,
            "rotation_disable_remap_contract_rejected":true}),
            );
        })
    })
    .await;
}

#[sqlx::test(migrations = false)]
async fn responses_continuity_independent_full_history_current_price_original_wallet(pool: PgPool) {
    scenario(pool,HOLD*2+COST,|lab| Box::pin(async move {
        let config=WorkerConfig::new(lab);
        let core=Core::start(config.clone(),None).await;
        let (_,root,_)=json_reply(call(&core,&lab.fixture.key,"independent-budget-root",&stored(json!("retained-history-".repeat(700)))).await).await;
        drop(core);
        let mut next_config=config;next_config.multiplier=2;
        let core=Core::start(next_config,None).await;
        lab.s().state.lock().unwrap().input_usage=700;
        lab.s().mode(Mode::JsonGate);
        let body=following(root["id"].as_str().unwrap(),json!("x"));
        let (response,())=tokio::join!(call(&core,&lab.fixture.key,"independent-full-budget",&body),async {
            lab.s().entered().await;
            let reserve:(i64,i64)=sqlx::query_as("SELECT reserved,settled FROM core_charging.requests WHERE state='streaming'")
                .fetch_one(&lab.fixture.pool).await.unwrap();
            assert_eq!(reserve,(HOLD*2,0));
            let balance=lab.fixture.ledger.balance(lab.fixture.account).await.unwrap();
            assert_eq!((balance.available_units,balance.reserved_units),(0,HOLD*2));
            lab.s().release.add_permits(1);
        });
        let (id,child,_)=json_reply(response).await;
        assert_eq!(charge(&lab.fixture.pool,&id).await,("settled".into(),HOLD*2,4230));
        let balance=lab.fixture.ledger.balance(lab.fixture.account).await.unwrap();
        assert_eq!((balance.available_units,balance.reserved_units),(1070,0));
        assert!(balance.available_units>6+50*10 && balance.available_units<HOLD*2,
            "wallet affords delta input but cannot afford the reviewed full context");
        lab.s().mode(Mode::Normal);
        let before=snapshot(&lab.fixture.pool).await;
        denied(&core,&lab.fixture.key,"independent-delta-not-enough",&following(child["id"].as_str().unwrap(),json!("x"))).await;
        // A rejected budget check may have its own durable charge record, but
        // cannot create a continuation, post money, reserve funds, or dispatch.
        let after=snapshot(&lab.fixture.pool).await;
        assert_eq!(before["maps"],after["maps"]);
        assert_eq!(before["journals"],after["journals"]);
        assert_eq!(before["balances"],after["balances"]);
        assert_eq!((lab.s().calls(),lab.s().counts()),(2,0));
        save("a4-budget",&json!({"full_hold_at_current_price":HOLD*2,"original_root_cost":COST,
            "new_turn_cost":4230,"available":balance.available_units,"delta_only_would_fit":true,"provider_calls":2}));
    })).await;
}

#[sqlx::test(migrations = false)]
async fn responses_continuity_independent_actual_kill_restart_same_database(pool: PgPool) {
    scenario(pool, 20000, |lab| {
        Box::pin(async move {
            let config = WorkerConfig::new(lab);
            let mut core = Core::start(config.clone(), None).await;
            let body = stored(json!("durable-provider-context-before-SIGKILL"));
            let (id, root, _) =
                json_reply(call(&core, &lab.fixture.key, "independent-restart-root", &body).await)
                    .await;
            assert_active(lab, &id, &root).await;
            let before = snapshot(&lab.fixture.pool).await;
            let old_pid = core.child.id();
            core.child.kill().unwrap();
            let killed = core.child.wait().unwrap();
            use std::os::unix::process::ExitStatusExt;
            assert_eq!(killed.signal(), Some(9));
            drop(core);
            let core = Core::start(config, None).await;
            assert_ne!(core.child.id(), old_pid);
            let repeat = call(&core, &lab.fixture.key, "independent-restart-root", &body).await;
            assert_eq!(repeat.status(), StatusCode::CONFLICT);
            let _ = repeat.bytes().await;
            assert_eq!(snapshot(&lab.fixture.pool).await, before);
            assert_eq!(lab.s().calls(), 1);
            let (_, child, _) = json_reply(
                call(
                    &core,
                    &lab.fixture.key,
                    "independent-restart-child",
                    &following(
                        root["id"].as_str().unwrap(),
                        json!("continue after process death"),
                    ),
                )
                .await,
            )
            .await;
            assert_eq!(lab.s().seen()[1]["previous_response_id"], "resp_supplier_1");
            assert!(
                child["output"][1]["content"][0]["text"]
                    .as_str()
                    .unwrap()
                    .contains("before-SIGKILL")
            );
            let balance = lab
                .fixture
                .ledger
                .balance(lab.fixture.account)
                .await
                .unwrap();
            assert_eq!(
                (balance.available_units, balance.reserved_units),
                (20000 - COST * 2, 0)
            );
            save(
                "a5-process-restart",
                &json!({"old_pid":old_pid,"new_pid":core.child.id(),"signal":9,
            "same_pg":true,"provider_survived":true,"provider_calls":2,"total_cost":COST*2}),
            );
        })
    })
    .await;
}

async fn no_success(result: Result<reqwest::Response, reqwest::Error>) {
    if let Ok(response) = result {
        let mut stream = response.bytes_stream();
        let mut bytes = Vec::new();
        while let Some(chunk) = stream.next().await {
            match chunk {
                Ok(chunk) => bytes.extend_from_slice(&chunk),
                Err(_) => break,
            }
        }
        assert!(
            !String::from_utf8_lossy(&bytes).contains("event: response.completed"),
            "failed work published a successful SSE terminal"
        );
        if let Ok(value) = serde_json::from_slice::<Value>(&bytes) {
            assert_ne!(
                value["status"], "completed",
                "failed work returned successful JSON"
            );
        }
    }
}
async fn crash_case(pool: PgPool, stage: &'static str) {
    scenario(pool,20000,move |lab|Box::pin(async move {
        let config=WorkerConfig::new(lab);
        let mut core=Core::start(config.clone(),Some(stage)).await;
        let body=stored(json!(format!("supplier-history-survives-{stage}")));
        no_success(send(&core,&lab.fixture.key,"independent-crash-root",&body,false).await).await;
        core.exited97().await;
        let id:String=sqlx::query_scalar("SELECT id FROM core_charging.requests").fetch_one(&lab.fixture.pool).await.unwrap();
        let handle=local_handle(&id);
        let before=charge(&lab.fixture.pool,&id).await;
        let candidate=mapping(&lab.fixture.pool,&id).await;
        let terminals:i64=sqlx::query_scalar("SELECT count(*) FROM core_model.output_checkpoints WHERE request_id=$1 AND terminal_observed")
            .bind(&id).fetch_one(&lab.fixture.pool).await.unwrap();
        match stage {
            "after_terminal"=>{
                assert!(candidate.is_none());assert_eq!(terminals,0);
                assert_eq!((before.1,before.2),(HOLD,0));
            }
            "after_candidate"=>{
                assert_eq!(candidate.as_ref().unwrap()["active"],false);assert_eq!(terminals,1);
                assert_eq!((before.1,before.2),(HOLD,0));
            }
            "after_settlement"=>{
                assert_eq!(candidate.as_ref().unwrap()["active"],false);assert_eq!(terminals,1);
                assert_eq!(before,("settled".into(),HOLD,COST));
            }
            "after_activation"=>{
                assert_eq!(candidate.as_ref().unwrap()["active"],true);assert_eq!(terminals,1);
                assert_eq!(before,("settled".into(),HOLD,COST));
            }
            _=>unreachable!(),
        }
        drop(core);
        if matches!(stage,"after_terminal"|"after_candidate") {
            // Observe real lease expiry; never edit a charging row's clock,
            // evidence, outcome, or reserved money to manufacture recovery.
            tokio::time::sleep(Duration::from_secs((LEASE+1) as u64)).await;
        }
        let mut restart=config;restart.multiplier=2;
        let core=Core::start(restart,None).await;
        let after_recovery=snapshot(&lab.fixture.pool).await;
        let repeated=call(&core,&lab.fixture.key,"independent-crash-root",&body).await;
        assert!(repeated.status().is_client_error());let _=repeated.bytes().await;
        assert_eq!(snapshot(&lab.fixture.pool).await,after_recovery);
        assert_eq!(lab.s().calls(),1,"restart/retry cannot regenerate the original request");
        if stage=="after_terminal" {
            assert_eq!(charge(&lab.fixture.pool,&id).await,("reconcile".into(),HOLD,0));
            assert!(mapping(&lab.fixture.pool,&id).await.is_none());
            denied(&core,&lab.fixture.key,"independent-unconfirmed-after-restart",&following(&handle,json!("unconfirmed"))).await;
            assert_eq!(lab.s().calls(),1);
            let balance=lab.fixture.ledger.balance(lab.fixture.account).await.unwrap();
            assert_eq!((balance.available_units,balance.reserved_units),(20000-HOLD,HOLD));
        } else {
            assert_eq!(charge(&lab.fixture.pool,&id).await,("settled".into(),HOLD,COST),"recovery settles OLD frozen price");
            assert_eq!(mapping(&lab.fixture.pool,&id).await.unwrap()["active"],true);
            let (_,child,_)=json_reply(call(&core,&lab.fixture.key,"independent-crash-child",&following(&handle,json!("resume retained history"))).await).await;
            assert!(child["output"][1]["content"][0]["text"].as_str().unwrap().contains(stage));
            assert_eq!(lab.s().seen()[1]["previous_response_id"],"resp_supplier_1");
            let balance=lab.fixture.ledger.balance(lab.fixture.account).await.unwrap();
            assert_eq!((balance.available_units,balance.reserved_units),(20000-COST-COST*2,0));
            let captures:i64=sqlx::query_scalar("SELECT count(*) FROM core_billing.ledger_journals WHERE kind='capture'")
                .fetch_one(&lab.fixture.pool).await.unwrap();
            assert_eq!(captures,2,"one original settlement plus one separately charged child");
        }
        assert_eq!(lab.s().counts(),0);
        save(&format!("a6-{stage}"),&json!({"exit":97,"stage":stage,"request":id,"before":before,
            "after":charge(&lab.fixture.pool,&id).await,"terminal_count":terminals,
            "provider_calls":lab.s().calls(),"old_price_preserved":true,"real_restart":true}));
    })).await;
}
macro_rules! crash_test {
    ($name:ident,$stage:literal) => {
        #[sqlx::test(migrations = false)]
        async fn $name(pool: PgPool) {
            crash_case(pool, $stage).await;
        }
    };
}
crash_test!(
    responses_continuity_independent_crash_after_terminal,
    "after_terminal"
);
crash_test!(
    responses_continuity_independent_crash_after_candidate,
    "after_candidate"
);
crash_test!(
    responses_continuity_independent_crash_after_settlement,
    "after_settlement"
);
crash_test!(
    responses_continuity_independent_crash_after_activation,
    "after_activation"
);

#[sqlx::test(migrations = false)]
async fn responses_continuity_independent_bad_tail_duplicate_and_no_eof(pool: PgPool) {
    scenario(pool,20000,|lab|Box::pin(async move {
        let config=WorkerConfig::new(lab);
        let core=Core::start(config.clone(),None).await;
        let mut pending=Vec::new();
        for (label,mode) in [("tail",Mode::Tail),("duplicate",Mode::Duplicate),("never-eof",Mode::Never)] {
            lab.s().mode(mode);
            let mut body=stored(json!(label));body["stream"]=true.into();
            let response=call(&core,&lab.fixture.key,&format!("independent-bad-{label}"),&body).await;
            assert_eq!(response.status(),StatusCode::OK);
            let id=response.headers()["x-request-id"].to_str().unwrap().to_owned();
            lab.s().entered().await;
            if !matches!(mode,Mode::Never) { lab.s().release.add_permits(1); }
            no_success(Ok(response)).await;
            assert_eq!(charge(&lab.fixture.pool,&id).await,("reconcile".into(),HOLD,0));
            assert!(mapping(&lab.fixture.pool,&id).await.is_none());
            let terminal:i64=sqlx::query_scalar("SELECT count(*) FROM core_model.output_checkpoints WHERE request_id=$1 AND terminal_observed")
                .bind(&id).fetch_one(&lab.fixture.pool).await.unwrap();
            assert_eq!(terminal,0);
            pending.push((label,id,body));
        }
        let before=snapshot(&lab.fixture.pool).await;
        drop(core);
        lab.s().mode(Mode::Normal);
        let core=Core::start(config,None).await;
        for (label,id,body) in &pending {
            let retry=call(&core,&lab.fixture.key,&format!("independent-bad-{label}"),body).await;
            assert!(retry.status().is_client_error());let _=retry.bytes().await;
            denied(&core,&lab.fixture.key,&format!("independent-bad-parent-{label}"),
                &following(&local_handle(id),json!("must not become eligible"))).await;
        }
        assert_eq!(snapshot(&lab.fixture.pool).await,before);
        assert_eq!(lab.s().calls(),3);
        let balance=lab.fixture.ledger.balance(lab.fixture.account).await.unwrap();
        assert_eq!((balance.available_units,balance.reserved_units),(20000-HOLD*3,HOLD*3));
        save("a6-invalid-terminal",&json!({"pending":pending.iter().map(|(label,id,_)|json!({"case":label,"id":id})).collect::<Vec<_>>(),
            "holds_retained":HOLD*3,"provider_calls":3,"success_terminal_published":false}));
    })).await;
}

#[sqlx::test(migrations = false)]
async fn responses_continuity_independent_candidate_settlement_activation_failures(pool: PgPool) {
    scenario(pool,20000,|lab|Box::pin(async move {
        // Fault DDL exists only inside this SQLx-owned disposable database.
        // No production function, permission, trigger or source is replaced.
        sqlx::raw_sql("CREATE SCHEMA independent_continuity_fault; CREATE FUNCTION independent_continuity_fault.reject() RETURNS trigger LANGUAGE plpgsql AS $$ BEGIN RAISE EXCEPTION 'independent persistence failure' USING ERRCODE='P0001'; END $$")
            .execute(&lab.fixture.pool).await.unwrap();
        let config=WorkerConfig::new(lab);
        let mut known=Vec::<String>::new();
        for (label,trigger) in [
            ("candidate","CREATE TRIGGER independent_fault BEFORE INSERT ON core_model.responses_continuations FOR EACH ROW EXECUTE FUNCTION independent_continuity_fault.reject()"),
            ("settlement","CREATE TRIGGER independent_fault BEFORE INSERT ON core_billing.ledger_journals FOR EACH ROW WHEN (NEW.kind='capture') EXECUTE FUNCTION independent_continuity_fault.reject()"),
            ("activation","CREATE TRIGGER independent_fault BEFORE UPDATE ON core_model.responses_continuations FOR EACH ROW WHEN (NEW.active AND NOT OLD.active) EXECUTE FUNCTION independent_continuity_fault.reject()"),
        ] {
            let core=Core::start(config.clone(),None).await;
            sqlx::raw_sql(trigger).execute(&lab.fixture.pool).await.unwrap();
            let body=stored(json!(format!("fault-{label}")));
            no_success(send(&core,&lab.fixture.key,&format!("independent-fault-{label}"),&body,false).await).await;
            let ids:Vec<String>=sqlx::query_scalar("SELECT id FROM core_charging.requests ORDER BY id").fetch_all(&lab.fixture.pool).await.unwrap();
            let added:Vec<_>=ids.iter().filter(|id|!known.contains(id)).cloned().collect();assert_eq!(added.len(),1);
            let id=&added[0];known=ids;
            let observation=charge(&lab.fixture.pool,id).await;
            let candidate=mapping(&lab.fixture.pool,id).await;
            let terminal:i64=sqlx::query_scalar("SELECT count(*) FROM core_model.output_checkpoints WHERE request_id=$1 AND terminal_observed")
                .bind(id).fetch_one(&lab.fixture.pool).await.unwrap();
            if label=="candidate" {
                assert!(candidate.is_none());assert_eq!(terminal,0,"checkpoint and candidate must roll back together");
            } else { assert_eq!(candidate.as_ref().unwrap()["active"],false);assert_eq!(terminal,1); }
            assert_eq!(observation.2,if label=="activation" {COST} else {0});
            let table=if label=="settlement" {"core_billing.ledger_journals"} else {"core_model.responses_continuations"};
            sqlx::raw_sql(sqlx::AssertSqlSafe(format!("DROP TRIGGER independent_fault ON {table}"))).execute(&lab.fixture.pool).await.unwrap();
            if label=="activation" {
                // Repair through real HTTP lookup before any recovery sweep.
                // Settled candidate eligibility is repaired without recharging
                // its parent; the new child pays its own separate turn.
                let (_,child,_)=json_reply(call(&core,&lab.fixture.key,"independent-lookup-repair",
                    &following(&local_handle(id),json!("repair settled parent"))).await).await;
                assert!(child["output"][1]["content"][0]["text"].as_str().unwrap().contains("fault-activation"));
                assert_eq!(mapping(&lab.fixture.pool,id).await.unwrap()["active"],true);
                assert_eq!(charge(&lab.fixture.pool,id).await.2,COST);
            }
            drop(core);
            if observation.0=="streaming" { tokio::time::sleep(Duration::from_secs((LEASE+1) as u64)).await; }
            let core=Core::start(config.clone(),None).await;
            let calls=lab.s().calls();
            let retry=call(&core,&lab.fixture.key,&format!("independent-fault-{label}"),&body).await;
            assert!(retry.status().is_client_error());let _=retry.bytes().await;
            assert_eq!(lab.s().calls(),calls);
            if label=="candidate" { assert_eq!(charge(&lab.fixture.pool,id).await,("reconcile".into(),HOLD,0)); }
            else {
                assert_eq!(charge(&lab.fixture.pool,id).await,("settled".into(),HOLD,COST));
                assert_eq!(mapping(&lab.fixture.pool,id).await.unwrap()["active"],true);
            }
            save(&format!("a6-db-fault-{label}"),&json!({"id":id,"observed":observation,"terminal":terminal,
                "after_restart":charge(&lab.fixture.pool,id).await,"provider_calls_unchanged":true}));
        }
        let balance=lab.fixture.ledger.balance(lab.fixture.account).await.unwrap();
        assert_eq!((balance.available_units,balance.reserved_units),(20000-HOLD-COST*3,HOLD));
        assert_eq!((lab.s().calls(),lab.s().counts()),(4,0));
    })).await;
}

#[sqlx::test(migrations = false)]
async fn responses_continuity_independent_previous_compact_opaque_roundtrip(pool: PgPool) {
    scenario(pool,20000,|lab|Box::pin(async move {
        let core=Core::start(WorkerConfig::new(lab),None).await;
        let (_,root,_)=json_reply(call(&core,&lab.fixture.key,"independent-roundtrip-stored",&stored(json!("history-only-before-compaction"))).await).await;
        let compact=json!({"model":PUBLIC,"previous_response_id":root["id"],"input":"compact prior retained context"});
        let response=send(&core,&lab.fixture.key,"independent-roundtrip-compact",&compact,true).await.unwrap();
        let (compact_id,result,bytes)=json_reply(response).await;
        assert_eq!(result["object"],"response.compaction");
        assert_packets(&lab.fixture.pool,&compact_id,&[&bytes]).await;
        assert_eq!(lab.s().seen()[1]["previous_response_id"],"resp_supplier_1");
        assert!(mapping(&lab.fixture.pool,&compact_id).await.is_none_or(|m|m["active"]==false));
        let mut input=result["output"].as_array().unwrap().clone();
        input.push(json!({"type":"function_call_output","call_id":CALL,"output":"opaque-tools-output"}));
        let (id,replayed,replay_bytes)=json_reply(call(&core,&lab.fixture.key,"independent-roundtrip-opaque",&stored(json!(input))).await).await;
        assert_active(lab,&id,&replayed).await;
        assert_packets(&lab.fixture.pool,&id,&[&replay_bytes]).await;
        assert_eq!(lab.s().seen()[2]["input"][0],result["output"][0]);
        assert_eq!(lab.s().seen()[2]["input"][1]["call_id"],CALL);
        assert!(replayed["output"][1]["content"][0]["text"].as_str().unwrap().contains("history-only-before-compaction"));
        let before=snapshot(&lab.fixture.pool).await;
        denied(&core,&lab.fixture.key,"independent-no-compact-parent",&following(result["id"].as_str().unwrap(),json!("not a stored response"))).await;
        assert_eq!(snapshot(&lab.fixture.pool).await,before);
        assert_eq!((lab.s().calls(),lab.s().counts()),(3,0));
        let balance=lab.fixture.ledger.balance(lab.fixture.account).await.unwrap();
        assert_eq!((balance.available_units,balance.reserved_units),(20000-COST*3,0));
        save("a7-compact-roundtrip",&json!({"stored":root["id"],"compact":result["id"],"replay":replayed["id"],
            "actual_opaque_preserved":true,"full_hold_each":HOLD,"provider_calls":3,"total_cost":COST*3}));
    })).await;
}

#[sqlx::test(migrations = false)]
async fn responses_continuity_independent_store_tristate_and_exact_refusal(pool: PgPool) {
    scenario(pool,40000,|lab|Box::pin(async move {
        let config=WorkerConfig::new(lab);
        for default in [true,false] {
            lab.s().state.lock().unwrap().default_store=default;
            let mut c=config.clone();c.default_store=Some(default);
            let core=Core::start(c,None).await;
            for explicit in [Some(true),Some(false),None] {
                let mut body=request(json!("store policy"));
                if let Some(value)=explicit {body["store"]=value.into();}
                let label=format!("independent-store-{default}-{explicit:?}");
                let (id,result,bytes)=json_reply(call(&core,&lab.fixture.key,&label,&body).await).await;
                assert_packets(&lab.fixture.pool,&id,&[&bytes]).await;
                let last=lab.s().seen().last().unwrap().clone();
                assert_eq!(last.get("store"),body.get("store"),"wire must preserve omission/false/true");
                if explicit.unwrap_or(default) {assert_active(lab,&id,&result).await;}
                else {
                    assert!(mapping(&lab.fixture.pool,&id).await.is_none_or(|m|m["active"]==false));
                    let before=snapshot(&lab.fixture.pool).await;
                    denied(&core,&lab.fixture.key,&format!("{label}-parent"),&following(result["id"].as_str().unwrap(),json!("cannot resume false store"))).await;
                    assert_eq!(snapshot(&lab.fixture.pool).await,before);
                }
            }
        }
        lab.s().state.lock().unwrap().default_store=true;
        let mut exact=config.clone();exact.exact=true;
        let official=gateway(&lab.runtime.pool,&exact,true);
        let mut plan=official.bridge.routing.route(crate::routing::RouteRequest {model:PUBLIC,group:"allowed",speed:None,
            protocol:Protocol::OpenAiResponses,replay:crate::routing::ReplayPermission::Denied}).unwrap();
        let attempt=plan.start().unwrap();
        assert!(matches!(official.bridge.provider_policy.count_plan(&attempt).unwrap(),crate::relay::input_count::CountPlan::OpenAiResponsesExactV1(_)));
        let core=Core::start(exact,None).await;
        let before=snapshot(&lab.fixture.pool).await;let calls=lab.s().calls();
        for (label,body) in [("store",stored(json!("exact must refuse"))),
            ("omitted",request(json!("reviewed default is true"))),
            ("previous",following(&local_handle("unknown"),json!("exact parent"))),
            ("opaque",stored(json!([{"type":"compaction","id":"independent-profile-item","encrypted_content":"opaque-profile-λ"}])))] {
            denied(&core,&lab.fixture.key,&format!("independent-exact-{label}"),&body).await;
        }
        assert_eq!(snapshot(&lab.fixture.pool).await,before);
        assert_eq!((lab.s().calls(),lab.s().counts()),(calls,0));
        drop(core);drop(official);
        // Without the new guarantee, the old explicit stateless compaction path
        // must still preserve provider IDs and retain its old eligibility.
        let mut old=config.clone();old.default_store=None;
        let old_gateway=gateway(&lab.runtime.pool,&old,false);
        let serialized=serde_json::to_value(old_gateway.bridge.provider_policy.context_document()).unwrap();
        assert!(serialized.as_array().unwrap().iter().all(|context|context.get("responses_continuity").is_none()),
            "old serialized contracts must omit the new optional guarantee");
        drop(old_gateway);
        let core=Core::start(old,None).await;
        let response=send(&core,&lab.fixture.key,"independent-old-compact",&json!({"model":PUBLIC,"input":"stateless"}),true).await.unwrap();
        let (id,result,_)=json_reply(response).await;
        assert!(result["id"].as_str().unwrap().starts_with("cmp_supplier_"));
        assert!(mapping(&lab.fixture.pool,&id).await.is_none());
        let mut stateless=request(result["output"].clone());stateless["store"]=false.into();
        let expected_provider=format!("resp_supplier_{}",lab.s().calls()+1);
        let (stateless_id,stateless_result,_)=json_reply(call(&core,&lab.fixture.key,"independent-old-opaque",&stateless).await).await;
        assert_eq!(stateless_result["id"],expected_provider);
        assert!(mapping(&lab.fixture.pool,&stateless_id).await.is_none());
        let before=snapshot(&lab.fixture.pool).await;let calls=lab.s().calls();
        denied(&core,&lab.fixture.key,"independent-no-store-contract",&stored(json!("no reviewed storage guarantee"))).await;
        assert_eq!(snapshot(&lab.fixture.pool).await,before);
        assert_eq!(lab.s().calls(),calls);
        drop(core);
        // A contradictory provider store receipt is untrusted post-dispatch
        // work: keep the hold, publish no success, activate no continuation.
        lab.s().mode(Mode::WrongStore);
        let core=Core::start(config,None).await;
        let before_ids:Vec<String>=sqlx::query_scalar("SELECT id FROM core_charging.requests").fetch_all(&lab.fixture.pool).await.unwrap();
        no_success(send(&core,&lab.fixture.key,"independent-wrong-store",&stored(json!("reject false receipt")),false).await).await;
        let after_ids:Vec<String>=sqlx::query_scalar("SELECT id FROM core_charging.requests").fetch_all(&lab.fixture.pool).await.unwrap();
        let added:Vec<_>=after_ids.iter().filter(|id|!before_ids.contains(id)).collect();assert_eq!(added.len(),1);
        assert_eq!(charge(&lab.fixture.pool,added[0]).await,("reconcile".into(),HOLD,0));
        assert!(mapping(&lab.fixture.pool,added[0]).await.is_none());
        save("a8-store-exact",&json!({"wire_tristate_preserved":true,"both_reviewed_defaults":true,
            "official_exact_profile_verified":true,"exact_rejection_before_reserve_and_network":true,"count_calls":0,
            "legacy_compact_provider_id_preserved":true,"wrong_store_hold":HOLD}));
    })).await;
}

async fn assert_runtime_boundary(pool: &PgPool, role: &str) {
    for privilege in [
        "SELECT",
        "INSERT",
        "UPDATE",
        "DELETE",
        "TRUNCATE",
        "REFERENCES",
        "TRIGGER",
    ] {
        let allowed: bool = sqlx::query_scalar(
            "SELECT has_table_privilege($1,'core_model.responses_continuations',$2)",
        )
        .bind(role)
        .bind(privilege)
        .fetch_one(pool)
        .await
        .unwrap();
        assert!(
            !allowed,
            "continuation runtime cannot have direct {privilege}"
        );
    }
    for function in [
        "core_model.resolve_continuation(text,text)",
        "core_model.record_continuation(text,bytea,text,text,text,bytea,bigint)",
        "core_model.activate_continuation(text)",
        "core_model.activate_continuations_after(integer,text)",
    ] {
        let (runtime,public):(bool,bool)=sqlx::query_as("SELECT has_function_privilege($1,$2,'EXECUTE'),EXISTS(SELECT FROM pg_proc p CROSS JOIN LATERAL aclexplode(coalesce(p.proacl,acldefault('f',p.proowner))) a WHERE p.oid=$2::regprocedure AND a.grantee=0 AND a.privilege_type='EXECUTE')")
            .bind(role).bind(function).fetch_one(pool).await.unwrap();
        assert!(runtime, "runtime missing {function}");
        assert!(!public, "PUBLIC can execute {function}");
    }
}
#[sqlx::test(migrations = false)]
async fn responses_continuity_independent_fresh69_runtime_capability_and_immutability(
    pool: PgPool,
) {
    scenario(pool,20000,|lab|Box::pin(async move {
        assert_eq!(crate::core_schema::schema_manifest().components.len(),69);
        assert_runtime_boundary(&lab.fixture.pool,&lab.runtime.name).await;
        let core=Core::start(WorkerConfig::new(lab),None).await;
        let (id,_,_)=json_reply(call(&core,&lab.fixture.key,"independent-acl-root",&stored(json!("must-not-be-stored-in-PG"))).await).await;
        let before=snapshot(&lab.fixture.pool).await;
        for statement in ["SELECT * FROM core_model.responses_continuations LIMIT 0",
            "INSERT INTO core_model.responses_continuations DEFAULT VALUES",
            "UPDATE core_model.responses_continuations SET active=active WHERE false",
            "DELETE FROM core_model.responses_continuations WHERE false",
            "TRUNCATE core_model.responses_continuations"] {
            let error=sqlx::query(statement).execute(&lab.runtime.pool).await.expect_err("runtime directly accessed continuation table");
            assert_eq!(error.as_database_error().and_then(|e|e.code()).as_deref(),Some("42501"));
        }
        let fields:(String,Vec<u8>,i64)=sqlx::query_as("SELECT provider_response_id,policy_sha256,terminal_sequence FROM core_model.responses_continuations WHERE request_id=$1")
            .bind(&id).fetch_one(&lab.fixture.pool).await.unwrap();
        let mut tx=lab.runtime.pool.begin().await.unwrap();
        sqlx::query("SET TRANSACTION ISOLATION LEVEL SERIALIZABLE").execute(&mut *tx).await.unwrap();
        assert!(sqlx::query("SELECT core_model.record_continuation($1,$2,$3,NULL,$4,$5,$6)")
            .bind(&id).bind(vec![0u8;32]).bind(&fields.0).bind(PUBLIC).bind(&fields.1).bind(fields.2)
            .execute(&mut *tx).await.is_err(),"even exact candidate replay must require original worker capability");
        tx.rollback().await.unwrap();
        assert!(sqlx::query("UPDATE core_model.responses_continuations SET provider_response_id='forged-binding' WHERE request_id=$1")
            .bind(&id).execute(&lab.fixture.pool).await.is_err(),"candidate binding must remain immutable even to table owner");
        assert_eq!(snapshot(&lab.fixture.pool).await,before);
        let map=mapping(&lab.fixture.pool,&id).await.unwrap().to_string();
        for forbidden in [lab.fixture.key.as_str(),SECRET,"must-not-be-stored-in-PG"] {assert!(!map.contains(forbidden));}
        save("a9-fresh-runtime",&json!({"components":69,"execute_only":true,"public_execute":false,
            "real_runtime_dml_sqlstate":"42501","wrong_worker_rejected":true,"immutable_binding":true,"no_body_or_secret":true}));
    })).await;
}

fn quoted(value: &str) -> String {
    format!("\"{}\"", value.replace('"', "\"\""))
}
async fn installed_catalog(pool: &PgPool) -> String {
    let definitions: Vec<String> =
        sqlx::query_scalar(crate::core_schema::schema_manifest().catalog.query_utf8)
            .fetch_all(pool)
            .await
            .unwrap();
    let mut hash = Sha256::new();
    for definition in definitions {
        hash.update((definition.len() as u64).to_be_bytes());
        hash.update(definition.as_bytes());
    }
    crate::hex_codec::encode(hash.finalize())
}
async fn preserved_state(pool: &PgPool) -> Value {
    let relations:Vec<(String,String,String)>=sqlx::query_as("SELECT n.nspname::text,c.relname::text,c.relkind::text FROM pg_class c JOIN pg_namespace n ON n.oid=c.relnamespace WHERE n.nspname LIKE 'core\\_%' ESCAPE '\\' AND c.relkind IN ('r','p','S') ORDER BY n.nspname COLLATE \"C\",c.relname COLLATE \"C\"")
        .fetch_all(pool).await.unwrap();
    let mut rows = serde_json::Map::new();
    for (schema, table, kind) in relations {
        if schema == "core_meta"
            && matches!(table.as_str(), "component_contract" | "runtime_contract")
        {
            continue;
        }
        let name = format!("{}.{}", quoted(&schema), quoted(&table));
        let sql = if kind == "S" {
            format!(
                "SELECT jsonb_build_object('last_value',last_value,'is_called',is_called) FROM {name}"
            )
        } else {
            format!(
                "SELECT coalesce(jsonb_agg(to_jsonb(t) ORDER BY to_jsonb(t)::text COLLATE \"C\"),'[]'::jsonb) FROM {name} t"
            )
        };
        let data: Value = sqlx::query_scalar(sqlx::AssertSqlSafe::<&str>(&sql))
            .fetch_one(pool)
            .await
            .unwrap();
        rows.insert(format!("{schema}.{table}"), data);
    }
    // Compare every pre-existing object's OID, owner and explicit ACL, including
    // columns/sequences/functions. New component69 objects are allowed additions.
    let objects:Vec<(String,Value)>=sqlx::query_as("SELECT 'relation:'||c.oid::text,jsonb_build_object('schema',n.nspname,'name',c.relname,'owner',c.relowner,'acl',c.relacl,'kind',c.relkind) FROM pg_class c JOIN pg_namespace n ON n.oid=c.relnamespace WHERE n.nspname LIKE 'core\\_%' ESCAPE '\\' OR n.nspname='public' UNION ALL SELECT 'function:'||p.oid::text,jsonb_build_object('schema',n.nspname,'name',p.proname,'args',p.proargtypes::text,'owner',p.proowner,'acl',p.proacl) FROM pg_proc p JOIN pg_namespace n ON n.oid=p.pronamespace WHERE n.nspname LIKE 'core\\_%' ESCAPE '\\' OR n.nspname='public' UNION ALL SELECT 'schema:'||oid::text,jsonb_build_object('name',nspname,'owner',nspowner,'acl',nspacl) FROM pg_namespace WHERE nspname LIKE 'core\\_%' ESCAPE '\\' OR nspname='public' UNION ALL SELECT 'column:'||a.attrelid::text||':'||a.attnum::text,jsonb_build_object('name',a.attname,'acl',a.attacl) FROM pg_attribute a JOIN pg_class c ON c.oid=a.attrelid JOIN pg_namespace n ON n.oid=c.relnamespace WHERE a.attnum>0 AND NOT a.attisdropped AND (n.nspname LIKE 'core\\_%' ESCAPE '\\' OR n.nspname='public')")
        .fetch_all(pool).await.unwrap();
    let objects: serde_json::Map<String, Value> = objects.into_iter().collect();
    let components: Vec<(String, Vec<u8>)> = sqlx::query_as(
        "SELECT name,fingerprint FROM core_meta.component_contract ORDER BY name COLLATE \"C\"",
    )
    .fetch_all(pool)
    .await
    .unwrap();
    json!({"data":rows,"objects":objects,"components":components})
}
fn assert_preserved(before: &Value, after: &Value) {
    for section in ["data", "objects"] {
        for (name, value) in before[section].as_object().unwrap() {
            assert_eq!(
                &after[section][name], value,
                "predecessor {section}: {name} changed"
            );
        }
    }
    let after_components = after["components"].as_array().unwrap();
    for component in before["components"].as_array().unwrap() {
        assert!(
            after_components.contains(component),
            "old component fingerprint changed: {component}"
        );
    }
}
async fn offline(database_oid: u32) {
    use sqlx::Connection;
    let mut control = sqlx::PgConnection::connect(&std::env::var("DATABASE_URL").unwrap())
        .await
        .unwrap();
    let control_oid: i64 =
        sqlx::query_scalar("SELECT oid::bigint FROM pg_database WHERE datname=current_database()")
            .fetch_one(&mut control)
            .await
            .unwrap();
    assert_ne!(
        control_oid,
        i64::from(database_oid),
        "offline observer must not connect to upgrade target"
    );
    tokio::time::timeout(Duration::from_secs(5), async {
        loop {
            let count: i64 =
                sqlx::query_scalar("SELECT count(*) FROM pg_stat_activity WHERE datid::bigint=$1")
                    .bind(i64::from(database_oid))
                    .fetch_one(&mut control)
                    .await
                    .unwrap();
            if count == 0 {
                break;
            }
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
    })
    .await
    .expect("target still has live writers/readers; do not bypass the upgrade fence");
    control.close().await.unwrap();
}

/// A9 independent assertions for the author's precise-68 fixture adapter.
///
/// Fixture input must be the SQLx-owned PG18.6 target, with the reviewed 68
/// installer, roles/fence and REAL pre-existing settled + pending HTTP charges.
/// The caller owns role/database cleanup even on panic. It must close all other
/// target pools before invoking this function. No initializer or pin is forged
/// here. The helper is deliberately not counted as a test until that driver is
/// registered and actually run; it is not a substitute for mid-DDL rollback or
/// COMMIT/ACK-loss injection, whose adapters are still missing from the handoff.
pub(crate) async fn responses_continuity_independent_upgrade_68_to69(
    pool: PgPool,
    installer_url: &str,
    request: crate::core_schema::UpgradeRequest,
) {
    use crate::core_schema::upgrade_database;
    use sqlx::postgres::PgPoolOptions;
    let facts:(String,i64,i32)=sqlx::query_as("SELECT current_database()::text,d.oid::bigint,current_setting('server_version_num')::int FROM pg_database d WHERE datname=current_database()")
        .fetch_one(&pool).await.unwrap();
    assert_eq!(facts.0, request.database.name);
    assert_eq!(facts.1, i64::from(request.database.oid));
    assert_eq!(facts.2, 180006);
    let count: i64 = sqlx::query_scalar("SELECT count(*) FROM core_meta.component_contract")
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(
        count, 68,
        "must start from an actual predecessor, not a modified fresh69"
    );
    let exists: bool =
        sqlx::query_scalar("SELECT to_regclass('core_model.responses_continuations') IS NOT NULL")
            .fetch_one(&pool)
            .await
            .unwrap();
    assert!(!exists);
    let mut manifest = crate::core_schema::schema_manifest();
    assert_eq!(manifest.components.len(), 69);
    let current_manifest = serde_json::to_vec(&manifest).unwrap();
    let mut current_manifest = current_manifest;
    current_manifest.push(b'\n');
    assert_eq!(
        request.successor_manifest_sha256,
        crate::hex_codec::encode(Sha256::digest(&current_manifest))
    );
    manifest.components.truncate(68);
    let mut predecessor_manifest = serde_json::to_vec(&manifest).unwrap();
    predecessor_manifest.push(b'\n');
    let pin: Value = serde_json::from_str(include_str!(
        "../core_schema/upgrade/payment_discount_management_target.json"
    ))
    .unwrap();
    assert_eq!(
        pin["manifest_sha256"],
        crate::hex_codec::encode(Sha256::digest(&predecessor_manifest))
    );
    assert_eq!(request.predecessor_manifest_sha256, pin["manifest_sha256"]);
    assert_eq!(request.predecessor_catalog_sha256, pin["catalog_sha256"]);
    assert_eq!(
        installed_catalog(&pool).await,
        request.predecessor_catalog_sha256
    );
    let (settled,pending,journals):(i64,i64,i64)=sqlx::query_as("SELECT (SELECT count(*) FROM core_charging.requests WHERE state='settled' AND settled>0),(SELECT count(*) FROM core_charging.requests WHERE state IN ('streaming','reconcile') AND reserved>0 AND settled=0),(SELECT count(*) FROM core_billing.ledger_journals)")
        .fetch_one(&pool).await.unwrap();
    assert!(
        settled > 0 && pending > 0 && journals > 0,
        "fixture must include real old money, a settled order and a retained hold"
    );
    let baseline = preserved_state(&pool).await;
    let options = (*pool.connect_options()).clone();
    pool.close().await;
    let mut wrong = request.clone();
    wrong.database.oid = wrong.database.oid.checked_add(1).unwrap();
    offline(request.database.oid).await;
    assert!(upgrade_database(installer_url, wrong).await.is_err());
    let inspect = PgPoolOptions::new()
        .max_connections(1)
        .connect_with(options.clone())
        .await
        .unwrap();
    assert_eq!(
        preserved_state(&inspect).await,
        baseline,
        "rejected incarnation must leave predecessor intact"
    );
    assert_eq!(
        installed_catalog(&inspect).await,
        request.predecessor_catalog_sha256
    );
    inspect.close().await;
    offline(request.database.oid).await;
    let applied = upgrade_database(installer_url, request.clone())
        .await
        .unwrap();
    assert_eq!(applied.status, "committed");
    assert_eq!(applied.components, 69);
    offline(request.database.oid).await;
    let replay = upgrade_database(installer_url, request.clone())
        .await
        .unwrap();
    assert_eq!(replay.status, "already_committed");
    assert_eq!(replay.request_sha256, applied.request_sha256);
    assert_eq!(
        replay.successor_catalog_sha256,
        applied.successor_catalog_sha256
    );
    let inspect = PgPoolOptions::new()
        .max_connections(1)
        .connect_with(options)
        .await
        .unwrap();
    let after = preserved_state(&inspect).await;
    assert_preserved(&baseline, &after);
    assert_eq!(after["components"].as_array().unwrap().len(), 69);
    assert_eq!(
        installed_catalog(&inspect).await,
        applied.successor_catalog_sha256
    );
    assert_runtime_boundary(&inspect, &request.roles.runtime).await;
    let receipt_tables:Vec<(String,String)>=sqlx::query_as("SELECT table_schema,table_name FROM information_schema.columns WHERE table_schema='core_meta' AND column_name IN ('upgrade_id','request_sha256','request') GROUP BY table_schema,table_name HAVING count(*)=3")
        .fetch_all(&inspect).await.unwrap();
    let mut receipts = Vec::<Value>::new();
    for (schema, table) in receipt_tables {
        let sql = format!(
            "SELECT to_jsonb(t) FROM {}.{} t WHERE upgrade_id=$1",
            quoted(&schema),
            quoted(&table)
        );
        receipts.extend(
            sqlx::query_scalar::<_, Value>(sqlx::AssertSqlSafe::<&str>(&sql))
                .bind(&request.upgrade_id)
                .fetch_all(&inspect)
                .await
                .unwrap(),
        );
    }
    assert_eq!(
        receipts.len(),
        1,
        "original replay must resolve one immutable upgrade receipt"
    );
    assert_eq!(
        receipts[0]["request"],
        serde_json::to_value(&request).unwrap()
    );
    assert_eq!(
        receipts[0]["request_sha256"]
            .as_str()
            .unwrap()
            .strip_prefix("\\x")
            .unwrap(),
        applied.request_sha256
    );
    for digest in ["preserved_data_sha256", "preserved_permissions_sha256"] {
        assert_eq!(
            receipts[0][digest].as_str().unwrap().len(),
            66,
            "receipt lacks full {digest}"
        );
    }
    save(
        "a9-upgrade-68-to69",
        &json!({"upgrade_id":request.upgrade_id,"old_settled":settled,"old_pending":pending,
        "old_journals":journals,"actual_data_acl_oid_preserved":true,"applied":applied.status,"replay":replay.status,
        "receipt_sha":applied.request_sha256,"successor_catalog":applied.successor_catalog_sha256}),
    );
    inspect.close().await;
}

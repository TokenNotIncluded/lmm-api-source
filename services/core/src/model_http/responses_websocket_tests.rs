//! Native WS business regressions: real sockets, restricted PostgreSQL and owned
//! Core children. Product startup and test-only phase gates are separate cases.
use super::*;
use crate::test_runtime::RuntimeRole;
use axum::extract::ws::{Message as ServerMessage, WebSocketUpgrade};
use futures_util::{SinkExt, StreamExt};
use native_http::test_support::{Fixture, fixture};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::{
    fs,
    io::Write,
    os::unix::{fs::OpenOptionsExt, process::ExitStatusExt},
    path::PathBuf,
    process::{Child, Command, Stdio},
    sync::atomic::{AtomicUsize, Ordering},
};
use tokio::sync::Semaphore;
use tokio_tungstenite::{
    WebSocketStream,
    tungstenite::{Message, client::IntoClientRequest},
};
const MODEL: &str = "independent-continuity-review";
const PROVIDER_MODEL: &str = "gpt-4.1-2025-04-14";
const SECRET: &str = super::responses_continuity_review_support::SECRET;
const FUNDS: i64 = 100_000;
const HOLD: i64 = 5050;
const COST: i64 = 39;
type Socket = WebSocketStream<tokio::net::TcpStream>;

// The production TOML dependency intentionally enables parsing only. This
// bounded synthetic sidecar encoder does not change dependency features.
fn document_toml(value: &Value) -> String {
    fn inline(value: &Value) -> String {
        match value {
            Value::Object(fields) => format!(
                "{{{}}}",
                fields
                    .iter()
                    .map(|(key, value)| format!(
                        "{}={}",
                        serde_json::to_string(key).unwrap(),
                        inline(value)
                    ))
                    .collect::<Vec<_>>()
                    .join(",")
            ),
            Value::Array(values) => format!(
                "[{}]",
                values.iter().map(inline).collect::<Vec<_>>().join(",")
            ),
            Value::Null => panic!("synthetic sidecar must omit absent TOML fields"),
            _ => value.to_string(),
        }
    }
    value
        .as_object()
        .unwrap()
        .iter()
        .map(|(key, value)| {
            format!(
                "{}={}\n",
                serde_json::to_string(key).unwrap(),
                inline(value)
            )
        })
        .collect()
}

fn artifacts() -> PathBuf {
    PathBuf::from(std::env::var("LMM_WS_ARTIFACTS").expect("new private WS artifact directory"))
}
fn save(name: &str, bytes: &[u8]) -> PathBuf {
    let path = artifacts().join(name);
    let mut file = fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .mode(0o600)
        .open(&path)
        .unwrap();
    file.write_all(bytes).unwrap();
    path
}
#[derive(Clone)]
struct Supplier {
    base: String,
    creates: Arc<Mutex<Vec<String>>>,
    controls: Arc<Mutex<Vec<String>>>,
    handshakes: Arc<AtomicUsize>,
    http: Arc<AtomicUsize>,
    release: Arc<Semaphore>,
    completed: Arc<Mutex<BTreeMap<String, Value>>>,
}
struct Peer {
    supplier: Supplier,
    task: tokio::task::JoinHandle<()>,
}
impl Drop for Peer {
    fn drop(&mut self) {
        self.task.abort();
    }
}
fn receipt(full: bool) -> Value {
    json!({"input_tokens":if full{100}else{7},"output_tokens":3,"total_tokens":if full{103}else{10},
        "input_tokens_details":{"cached_tokens":if full{20}else{2},"cache_write_tokens":if full{30}else{3},
            "cache_write_tokens_details":{"ttl_30m_tokens":if full{10}else{1},"ttl_12h_tokens":if full{20}else{2}}}})
}
async fn supplier_upgrade(
    State(s): State<Supplier>,
    headers: HeaderMap,
    ws: WebSocketUpgrade,
) -> Response {
    assert_eq!(headers[header::AUTHORIZATION], format!("Bearer {SECRET}"));
    assert_eq!(headers["openai-beta"], "responses_websockets=2026-02-06");
    s.handshakes.fetch_add(1, Ordering::SeqCst);
    ws.on_upgrade(move |mut socket|async move {
        loop {
            let raw = match socket.recv().await {
                Some(Ok(ServerMessage::Text(raw))) => raw,
                Some(Ok(ServerMessage::Ping(_) | ServerMessage::Pong(_))) => continue,
                _ => break,
            };
            let body:Value=serde_json::from_str(raw.as_str()).unwrap();
            if body["type"]!="response.create" { emit_control(&s,&mut socket,raw.as_str()).await;continue; }
            let index={let mut seen=s.creates.lock().unwrap();seen.push(raw.to_string());seen.len()};
            let id=format!("ws-response-{index}");
            let parent=body["previous_response_id"].as_str();
            if let Some(parent)=parent { assert!(s.completed.lock().unwrap().contains_key(parent),"supplier must retain the actual prior context"); }
            // Real providers may omit the gateway-local stream identity.
            let created=json!({"type":"response.created","response":{"id":id,"status":"in_progress"}});
            if socket.send(ServerMessage::Text(created.to_string().into())).await.is_err(){break;}
            let input=body["input"].as_str().or_else(||body["input"][0]["content"][0]["text"].as_str()).unwrap_or("structured");
            if input=="unknown" { let _=socket.close().await;break; }
            let mut cancelled=false;
            if matches!(input,"held"|"controls") {
                loop { tokio::select! {
                    permit=s.release.acquire()=>{permit.unwrap().forget();break;},
                    incoming=socket.recv()=>{
                        match incoming {
                            Some(Ok(ServerMessage::Text(control)))=>{
                                let value:Value=serde_json::from_str(control.as_str()).unwrap();
                                if value["type"]=="response.cancel" && value["event_id"]=="cancel-success" {
                                    s.controls.lock().unwrap().push(control.to_string());
                                    cancelled=true;break;
                                }
                                emit_control(&s,&mut socket,control.as_str()).await;
                            },
                            Some(Ok(ServerMessage::Ping(_) | ServerMessage::Pong(_)))=>(),
                            _=>break,
                        }
                    }
                }}
            }
            let mut usage=receipt(parent.is_some());
            if input=="missing" {usage["input_tokens_details"].as_object_mut().unwrap().remove("cache_write_tokens");}
            if input=="missing12h" {usage["input_tokens_details"]["cache_write_tokens_details"].as_object_mut().unwrap().remove("ttl_12h_tokens");}
            if input=="contradiction" {usage["input_tokens_details"]["cache_write_tokens_details"]["ttl_12h_tokens"]=json!(4);}
            let terminal=json!({"type":if cancelled{"response.cancelled"}else{"response.completed"},"response":{"id":id,"status":if cancelled{"cancelled"}else{"completed"},"output":[],"usage":usage}});
            let ping_after_terminal=input=="controls";
            s.completed.lock().unwrap().insert(id,body);
            if socket.send(ServerMessage::Text(terminal.to_string().into())).await.is_err(){break;}
            if ping_after_terminal {let _=socket.send(ServerMessage::Ping(vec![7].into())).await;}
        }
    })
}
async fn emit_control(s: &Supplier, socket: &mut axum::extract::ws::WebSocket, raw: &str) {
    s.controls.lock().unwrap().push(raw.to_owned());
    let body: Value = serde_json::from_str(raw).unwrap();
    if body["event_id"]
        .as_str()
        .is_some_and(|id| id.starts_with("pending-"))
    {
        return; // Deliberately leave a named control unresolved at the provider.
    }
    let reference = if body["type"] == "test.emit_control_error" {
        &body["target_event_id"]
    } else {
        &body["event_id"]
    };
    let event =
        json!({"type":"error","error":{"event_id":reference,"code":"synthetic_control_rejected"}});
    let _ = socket
        .send(ServerMessage::Text(event.to_string().into()))
        .await;
}
impl Peer {
    async fn start() -> Self {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let supplier = Supplier {
            base: format!("http://{}/v1", listener.local_addr().unwrap()),
            creates: Default::default(),
            controls: Default::default(),
            handshakes: Default::default(),
            http: Default::default(),
            release: Arc::new(Semaphore::new(0)),
            completed: Default::default(),
        };
        async fn http(State(s): State<Supplier>) -> Json<Value> {
            s.http.fetch_add(1, Ordering::SeqCst);
            Json(
                json!({"id":"http-control","status":"completed","output":[],"usage":receipt(false)}),
            )
        }
        let app = Router::new()
            .route("/v1/responses", get(supplier_upgrade).post(http))
            .with_state(supplier.clone());
        let task = tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
        Self { supplier, task }
    }
}
#[derive(Clone, Serialize, Deserialize)]
struct WorkerConfig {
    database: String,
    document: String,
    ready: String,
    local: bool,
    recover: bool,
}
#[tokio::test]
#[ignore = "owned real Core WS child; not a business test"]
async fn ws_core_worker() {
    let config: WorkerConfig =
        serde_json::from_slice(&fs::read(std::env::var("LMM_WS_WORKER_CONFIG").unwrap()).unwrap())
            .unwrap();
    let pool = PgPool::connect(&config.database).await.unwrap();
    let gateway =
        crate::config::model_gateway(pool, config.document.as_bytes(), config.local).unwrap();
    if config.recover {
        gateway.recover_expired(100).await.unwrap();
        for _ in 0..4 {
            gateway.reconcile_completed_streams(100).await.unwrap();
        }
    }
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let path = PathBuf::from(config.ready);
    let temp = path.with_extension("new");
    fs::write(&temp, format!("http://{}", listener.local_addr().unwrap())).unwrap();
    fs::rename(temp, path).unwrap();
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
}
impl Drop for Core {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}
impl Core {
    async fn start(
        lab: &Lab,
        document: &Value,
        gate: Option<&str>,
        recover: bool,
        product: bool,
    ) -> Self {
        let nonce = format!("{:016x}", rand::random::<u64>());
        let document = document_toml(document);
        let config = WorkerConfig {
            database: lab.runtime.url.clone(),
            document: document.clone(),
            ready: artifacts()
                .join(format!("{nonce}.ready"))
                .to_str()
                .unwrap()
                .into(),
            local: !document.contains("openai_responses_exact_v1"),
            recover,
        };
        let path = save(
            &format!("{nonce}.private.json"),
            &serde_json::to_vec(&config).unwrap(),
        );
        let log = save(&format!("{nonce}.log"), b"");
        let log = fs::OpenOptions::new().append(true).open(log).unwrap();
        let mut command;
        let mut base = String::new();
        if product {
            command = Command::new(
                std::env::var_os("LMM_WS_CORE_BINARY")
                    .expect("matching immutable product binary required"),
            );
            let sidecar = save(&format!("{nonce}.model.toml"), document.as_bytes());
            let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
            let address = listener.local_addr().unwrap();
            drop(listener);
            base = format!("http://{address}");
            command
                .arg("--legacy-env")
                .env("LMM_CORE_DATABASE_URL", &config.database)
                .env("LMM_CORE_LISTEN", address.to_string())
                .env("LMM_CORE_MODEL_CONFIG_FILE", sidecar)
                .env("LMM_CORE_MODEL_LOCAL_HTTP", "true")
                .env("LMM_CORE_MAX_INFLIGHT", "16");
        } else {
            command = Command::new(std::env::current_exe().unwrap());
            command
                .args([
                    "--exact",
                    "model_http::responses_websocket_tests::ws_core_worker",
                    "--ignored",
                    "--nocapture",
                    "--test-threads=1",
                ])
                .env("LMM_WS_WORKER_CONFIG", path);
        }
        // No inherited production configuration, credentials or proxy environment.
        let inherited: Vec<_> = std::env::vars_os()
            .filter(|(key, _)| key.as_encoded_bytes().starts_with(b"LMM_CORE_"))
            .map(|(key, _)| key)
            .collect();
        for key in inherited {
            if !product
                || !matches!(
                    key.to_str(),
                    Some(
                        "LMM_CORE_DATABASE_URL"
                            | "LMM_CORE_LISTEN"
                            | "LMM_CORE_MODEL_CONFIG_FILE"
                            | "LMM_CORE_MODEL_LOCAL_HTTP"
                            | "LMM_CORE_MAX_INFLIGHT"
                    )
                )
            {
                command.env_remove(key);
            }
        }
        command
            .env("LMM_WS_OFFICIAL_LOOPBACK_BASE", &lab.peer.supplier.base)
            .env_remove("DATABASE_URL")
            .env_remove("LMM_WS_GATE_STAGE")
            .env_remove("LMM_RESPONSES_CONTINUITY_CRASH_AT")
            .stdout(Stdio::from(log.try_clone().unwrap()))
            .stderr(Stdio::from(log));
        if let Some(stage) = gate {
            command
                .env("LMM_WS_GATE_STAGE", stage)
                .env("LMM_WS_GATE_DIR", &lab.directory);
        }
        let mut core = Self {
            child: command.spawn().unwrap(),
            base,
        };
        for _ in 0..500 {
            assert!(
                core.child.try_wait().unwrap().is_none(),
                "WS child exited; retain log"
            );
            if product {
                if reqwest::Client::new()
                    .get(format!("{}/health/ready", core.base))
                    .send()
                    .await
                    .is_ok()
                {
                    return core;
                }
            } else if let Ok(base) = fs::read_to_string(&config.ready) {
                core.base = base;
                return core;
            }
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
        panic!("WS child startup timed out")
    }
    fn kill9(&mut self) {
        self.child.kill().unwrap();
        assert_eq!(self.child.wait().unwrap().signal(), Some(9));
    }
}
struct Lab {
    fixture: Fixture,
    runtime: RuntimeRole,
    peer: Peer,
    directory: PathBuf,
    secret: PathBuf,
}
impl Lab {
    async fn new(pool: PgPool) -> Self {
        let fixture = fixture(pool, FUNDS, &[MODEL]).await;
        let runtime = RuntimeRole::create(&fixture.pool).await;
        runtime.optional("core_charging").await;
        runtime.optional("core_model").await;
        let peer = Peer::start().await;
        let directory = artifacts().join(format!("ws-{:016x}", rand::random::<u64>()));
        fs::create_dir(&directory).unwrap();
        let secret = directory.join("provider.secret");
        let mut f = fs::OpenOptions::new()
            .create_new(true)
            .write(true)
            .mode(0o600)
            .open(&secret)
            .unwrap();
        f.write_all(SECRET.as_bytes()).unwrap();
        Self {
            fixture,
            runtime,
            peer,
            directory,
            secret,
        }
    }
    fn document(&self, enabled: bool) -> Value {
        json!({"version":1,"input_token_ceiling":800,"lease_seconds":16,"timeout_seconds":3,"max_output_tokens":50,
            "responses_websocket":enabled,"responses_websocket_origins":["https://trusted.ws.test"],
            "credentials":[{"reference":{"id":7,"revision":1},"secret_file":self.secret}],"key_groups":[],
            "routing":{"version":1,"aliases":[],"groups":[{"name":"allowed","enabled":true}],"upstreams":[{"id":1,"endpoint":self.peer.supplier.base,"protocol":"openai_responses","credential":{"id":7,"revision":1},"enabled":true}],
                "routes":[{"id":1,"model":MODEL,"group":"allowed","enabled":true,"speeds":["standard"],"targets":[{"id":1,"upstream_id":1,"upstream_model":PROVIDER_MODEL,"enabled":true,"priority":0,"weight":1,"speeds":["standard"],"payload":"adapt","timeout_ms":5000}],"failover":{"max_attempts":1,"retry_on":[],"allow_replay_after_send":false}}]},
            "prices":{"version":1,"rules":[{"id":1,"model":MODEL,"group":"allowed","speed":"standard","rates":{"input":0,"output":0,"cache_read":0,"cache_write":0,"request":0},"group_multiplier_ppm":1000000,"speed_multiplier_ppm":1000000,"billing_mode":"tiered_decimal","tiered":{"revision":4,"source_digest":"65".repeat(32),"uses_cache_read":true,"uses_cache_write":true,"uses_cache_write_1h":false,"uses_cache_write_12h":true,"tiers":[{"rates":{"input":{"numerator":6,"denominator":1},"output":{"numerator":10,"denominator":1},"cache_read":{"numerator":2,"denominator":1},"cache_write":{"numerator":8,"denominator":1},"cache_write_12h":{"numerator":12,"denominator":1}}}]}}]}})
    }
    fn calls(&self) -> usize {
        self.peer.supplier.creates.lock().unwrap().len()
    }
    async fn money(&self) -> Value {
        sqlx::query_scalar("SELECT jsonb_build_object('balances',(SELECT jsonb_agg(to_jsonb(b) ORDER BY ledger_account_id) FROM core_billing.balance_state b),'journals',(SELECT count(*) FROM core_billing.ledger_journals))").fetch_one(&self.fixture.pool).await.unwrap()
    }
    async fn charges(&self) -> Vec<(String, String, i64, i64)> {
        sqlx::query_as("SELECT id,state,reserved,settled FROM core_charging.requests ORDER BY id")
            .fetch_all(&self.fixture.pool)
            .await
            .unwrap()
    }
    async fn balance(&self, available: i64, reserved: i64) {
        let b = self
            .fixture
            .ledger
            .balance(self.fixture.account)
            .await
            .unwrap();
        assert_eq!((b.available_units, b.reserved_units), (available, reserved));
    }
    async fn cleanup(self) {
        drop(self.peer);
        self.runtime.cleanup().await;
    }
}
async fn connect(
    core: &Core,
    key: &str,
    origin: Option<&str>,
    protocol: Option<&str>,
) -> Result<(Socket, axum::http::Response<Option<Vec<u8>>>), tokio_tungstenite::tungstenite::Error>
{
    let url = core.base.replacen("http://", "ws://", 1) + "/v1/responses";
    let mut request = url.into_client_request().unwrap();
    request.headers_mut().insert(
        header::AUTHORIZATION,
        format!("Bearer {key}").parse().unwrap(),
    );
    if let Some(origin) = origin {
        request
            .headers_mut()
            .insert(header::ORIGIN, origin.parse().unwrap());
    }
    if let Some(protocol) = protocol {
        request
            .headers_mut()
            .insert("sec-websocket-protocol", protocol.parse().unwrap());
    }
    let address = core.base.strip_prefix("http://").unwrap();
    let tcp = tokio::net::TcpStream::connect(address).await.unwrap();
    tokio_tungstenite::client_async(request, tcp).await
}
async fn send<S: tokio::io::AsyncRead + tokio::io::AsyncWrite + Unpin>(
    socket: &mut WebSocketStream<S>,
    body: Value,
) {
    socket
        .send(Message::Text(body.to_string().into()))
        .await
        .unwrap();
}
async fn receive<S: tokio::io::AsyncRead + tokio::io::AsyncWrite + Unpin>(
    socket: &mut WebSocketStream<S>,
) -> (String, Value) {
    loop {
        let message = tokio::time::timeout(Duration::from_secs(8), socket.next())
            .await
            .unwrap()
            .unwrap()
            .unwrap();
        if let Message::Text(text) = message {
            let value = serde_json::from_str(text.as_str()).unwrap();
            let mut transcript = fs::OpenOptions::new()
                .create(true)
                .append(true)
                .mode(0o600)
                .open(artifacts().join("received-events.jsonl"))
                .unwrap();
            writeln!(transcript, "{text}").unwrap();
            return (text.to_string(), value);
        }
    }
}
fn create(input: &str, event: &str) -> Value {
    json!({"type":"response.create","event_id":event,"stream_id":"lane-A","model":MODEL,"input":input,"max_output_tokens":50,"prompt_cache_options":{"ttl":"12h"}})
}
async fn terminal<S: tokio::io::AsyncRead + tokio::io::AsyncWrite + Unpin>(
    socket: &mut WebSocketStream<S>,
) -> (String, Value) {
    loop {
        let pair = receive(socket).await;
        if pair.1["type"] == "response.completed" {
            return pair;
        }
        assert_ne!(pair.1["type"], "error", "{}", pair.0);
    }
}
async fn await_state(lab: &Lab, id: &str, state: &str) -> (String, String, i64, i64) {
    for _ in 0..300 {
        let rows = lab.charges().await;
        if let Some(row) = rows.into_iter().find(|r| r.0 == id && r.1 == state) {
            return row;
        }
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    panic!("missing expected charge state {state}")
}

#[sqlx::test(migrations = false)]
async fn ws01_product_configuration_is_opt_in_and_http_stays_http(pool: PgPool) {
    let lab = Lab::new(pool).await;
    let mut disabled = lab.document(false);
    disabled
        .as_object_mut()
        .unwrap()
        .remove("responses_websocket");
    let core = Core::start(&lab, &disabled, None, false, true).await;
    let error = connect(&core, &lab.fixture.key, None, None)
        .await
        .unwrap_err();
    assert!(
        matches!(error,tokio_tungstenite::tungstenite::Error::Http(ref r) if r.status()==StatusCode::METHOD_NOT_ALLOWED)
    );
    assert_eq!(lab.peer.supplier.handshakes.load(Ordering::SeqCst), 0);
    drop(core);
    let core = Core::start(&lab, &lab.document(true), None, false, true).await;
    let (mut socket, response) = connect(&core, &lab.fixture.key, None, Some("responses"))
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::SWITCHING_PROTOCOLS);
    assert_eq!(response.headers()["sec-websocket-protocol"], "responses");
    send(&mut socket, create("root", "product-turn")).await;
    terminal(&mut socket).await;
    let response=reqwest::Client::new().post(format!("{}/v1/responses",core.base)).bearer_auth(&lab.fixture.key)
        .header("idempotency-key","ws-http-control").json(&json!({"model":MODEL,"input":"root","max_output_tokens":50,"prompt_cache_options":{"ttl":"12h"}})).send().await.unwrap();
    let status = response.status();
    let body = response.text().await.unwrap();
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(lab.peer.supplier.http.load(Ordering::SeqCst), 1);
    assert_eq!(lab.calls(), 1);
    drop(socket);
    drop(core);
    lab.cleanup().await;
}

#[sqlx::test(migrations = false)]
async fn ws02_browser_priority_origin_live_key_and_oauth_authorization(pool: PgPool) {
    let lab = Lab::new(pool).await;
    let core = Core::start(&lab, &lab.document(true), None, false, false).await;
    let protocol = format!("responses, openai-insecure-api-key.{}", lab.fixture.key);
    let (mut socket, response) = connect(
        &core,
        "invalid-explicit-bearer",
        Some("https://trusted.ws.test"),
        Some(&protocol),
    )
    .await
    .unwrap();
    assert_eq!(response.headers()["sec-websocket-protocol"], "responses");
    assert!(!format!("{:?}", response.headers()).contains(&lab.fixture.key));
    send(&mut socket, create("root", "browser-key")).await;
    terminal(&mut socket).await;
    assert_eq!(lab.calls(), 1);
    for origin in [
        "https://foreign.invalid",
        "null",
        "https://trusted.ws.test/path",
    ] {
        let error = connect(&core, &lab.fixture.key, Some(origin), None)
            .await
            .unwrap_err();
        assert!(
            matches!(error,tokio_tungstenite::tungstenite::Error::Http(ref r) if r.status()==StatusCode::FORBIDDEN)
        );
    }
    let error = connect(&core, &lab.fixture.session, None, None)
        .await
        .unwrap_err();
    assert!(
        matches!(error,tokio_tungstenite::tungstenite::Error::Http(ref r) if r.status()==StatusCode::FORBIDDEN)
    );
    let before = lab.money().await;
    IdentityStore::from_pool(lab.fixture.pool.clone())
        .revoke(&lab.fixture.session, lab.fixture.key_id)
        .await
        .unwrap();
    send(&mut socket, create("root", "revoked-next-turn")).await;
    let error = receive(&mut socket).await.1;
    assert_eq!(error["type"], "error");
    assert_eq!(error["status"], 401);
    assert_eq!(lab.money().await, before);
    assert_eq!(lab.calls(), 1);
    drop(socket);
    drop(core);
    // Reuse the established BrowserAuth/PKCE/consent fixture, not fabricated OAuth rows.
    let oauth = super::responses_continuity_review_support::oauth_fixture(
        super::responses_continuity_review_independent::fixture_gateway,
        &lab.fixture,
        &lab.runtime.pool,
        &[
            lab.peer.supplier.base.clone(),
            lab.peer.supplier.base.clone(),
        ],
    )
    .await;
    let gateway = crate::config::model_gateway(
        lab.runtime.pool.clone(),
        document_toml(&lab.document(true)).as_bytes(),
        true,
    )
    .unwrap()
    .with_oauth2_issuer(oauth.gateway.oauth2.clone().unwrap());
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let task = tokio::spawn(async move {
        axum::serve(
            listener,
            gateway
                .router()
                .into_make_service_with_connect_info::<SocketAddr>(),
        )
        .await
        .unwrap()
    });
    async fn oauth_connect(
        address: SocketAddr,
        token: &str,
    ) -> Result<
        (Socket, axum::http::Response<Option<Vec<u8>>>),
        tokio_tungstenite::tungstenite::Error,
    > {
        let mut request = format!("ws://{address}/v1/responses")
            .into_client_request()
            .unwrap();
        request.headers_mut().insert(
            header::AUTHORIZATION,
            format!("Bearer {token}").parse().unwrap(),
        );
        tokio_tungstenite::client_async(
            request,
            tokio::net::TcpStream::connect(address).await.unwrap(),
        )
        .await
    }
    let (mut oauth_socket, _) = oauth_connect(address, &oauth.first.token).await.unwrap();
    let mut body = create("root", "oauth-turn");
    body.as_object_mut().unwrap().remove("prompt_cache_options");
    send(&mut oauth_socket, body).await;
    terminal(&mut oauth_socket).await;
    let before = lab.money().await;
    let denied = oauth_connect(address, &oauth.denied_scope_token)
        .await
        .unwrap_err();
    assert!(
        matches!(denied,tokio_tungstenite::tungstenite::Error::Http(ref response)
        if response.status()==StatusCode::FORBIDDEN)
    );
    assert_eq!(lab.money().await, before);
    assert_eq!(lab.calls(), 2);
    drop(oauth_socket);
    task.abort();
    let _ = task.await;
    lab.cleanup().await;
}

// A test-only TLS terminator forwards the unchanged WS byte stream to Core.
// The TLS ingress replaces, rather than accepts, a client-supplied scheme.
struct TlsFront {
    child: Child,
    proxy: Option<std::thread::JoinHandle<std::io::Result<()>>>,
    address: SocketAddr,
    label: String,
}
impl TlsFront {
    // Socket/pipe io::copy can select blocking splice and withhold a short
    // upgraded response until EOF. Forward each bounded chunk immediately.
    fn forward_tls_bytes(
        reader: &mut impl std::io::Read,
        writer: &mut impl std::io::Write,
    ) -> std::io::Result<()> {
        let mut buffer = [0; 16 * 1024];
        loop {
            let count = match reader.read(&mut buffer) {
                Ok(0) => return Ok(()),
                Ok(count) => count,
                Err(error) if error.kind() == std::io::ErrorKind::Interrupted => continue,
                Err(error) => return Err(error),
            };
            writer.write_all(&buffer[..count])?;
            writer.flush()?;
        }
    }

    fn start(core: &Core, label: &str, cert: &std::path::Path, key: &std::path::Path) -> Self {
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let address = listener.local_addr().unwrap();
        drop(listener);
        let error = fs::OpenOptions::new()
            .create_new(true)
            .write(true)
            .mode(0o600)
            .open(artifacts().join(format!("{label}.tls.log")))
            .unwrap();
        let mut child = Command::new("openssl")
            .args(["s_server", "-quiet", "-naccept", "1", "-accept"])
            .arg(address.to_string())
            .arg("-cert")
            .arg(cert)
            .arg("-key")
            .arg(key)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(error)
            .spawn()
            .unwrap();
        let mut from_client = child.stdout.take().unwrap();
        let mut to_client = child.stdin.take().unwrap();
        let backend: SocketAddr = core.base.strip_prefix("http://").unwrap().parse().unwrap();
        let proxy = std::thread::spawn(move || {
            let mut head = Vec::new();
            while !head.ends_with(b"\r\n\r\n") {
                let mut byte = [0];
                std::io::Read::read_exact(&mut from_client, &mut byte)?;
                head.push(byte[0]);
                if head.len() > 16 * 1024 {
                    return Err(std::io::Error::other("oversized TLS test header"));
                }
            }
            let raw = std::str::from_utf8(&head).map_err(std::io::Error::other)?;
            let mut upstream = std::net::TcpStream::connect(backend)?;
            upstream.set_read_timeout(Some(Duration::from_secs(15)))?;
            upstream.set_write_timeout(Some(Duration::from_secs(15)))?;
            for line in raw.trim_end_matches("\r\n").split("\r\n") {
                if !line.to_ascii_lowercase().starts_with("x-forwarded-proto:") {
                    write!(upstream, "{line}\r\n")?;
                }
            }
            upstream.write_all(b"X-Forwarded-Proto: https\r\n\r\n")?;
            let mut downstream = upstream.try_clone()?;
            let download = std::thread::spawn(move || {
                Self::forward_tls_bytes(&mut downstream, &mut to_client)
            });
            let uploaded = Self::forward_tls_bytes(&mut from_client, &mut upstream);
            let _ = upstream.shutdown(std::net::Shutdown::Both);
            let _ = download.join();
            uploaded
        });
        Self {
            child,
            proxy: Some(proxy),
            address,
            label: label.into(),
        }
    }
    async fn request(
        &mut self,
        key: &str,
        origin: &str,
        cert: &std::path::Path,
    ) -> reqwest::Response {
        let certificate = reqwest::Certificate::from_pem(&fs::read(cert).unwrap()).unwrap();
        let client = reqwest::Client::builder()
            .no_proxy()
            .http1_only()
            .tls_certs_only([certificate])
            .resolve("ws.same-origin.test", self.address)
            .connect_timeout(Duration::from_secs(2))
            .timeout(Duration::from_secs(5))
            .build()
            .unwrap();
        for _ in 0..100 {
            assert!(
                self.child.try_wait().unwrap().is_none(),
                "TLS terminator exited; retain log"
            );
            let response = client
                .get(format!(
                    "https://ws.same-origin.test:{}/v1/responses",
                    self.address.port()
                ))
                .bearer_auth(key)
                .header("Origin", origin)
                .header("X-Forwarded-Proto", "http")
                .header("Connection", "Upgrade")
                .header("Upgrade", "websocket")
                .header("Sec-WebSocket-Version", "13")
                .header("Sec-WebSocket-Key", "dGhlIHNhbXBsZSBub25jZQ==")
                .send()
                .await;
            match response {
                Ok(response) => return response,
                Err(error) if error.is_connect() => {
                    tokio::time::sleep(Duration::from_millis(20)).await
                }
                Err(error) => panic!("real TLS handshake: {error}"),
            }
        }
        panic!("TLS terminator never listened")
    }
}
impl Drop for TlsFront {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let waited = self.child.wait();
        let joined = self.proxy.take().unwrap().join();
        save(
            &format!("{}.tls-cleanup.json", self.label),
            &serde_json::to_vec(&json!({
                "pid":self.child.id(),"wait":format!("{waited:?}"),"proxy":format!("{joined:?}")
            }))
            .unwrap(),
        );
        assert!(waited.is_ok(), "owned TLS child was not reaped");
        assert!(joined.is_ok(), "owned TLS proxy thread panicked");
    }
}

#[sqlx::test(migrations = false)]
async fn ws02_https_same_origin_and_explicit_cross_origin_are_distinct(pool: PgPool) {
    let lab = Lab::new(pool).await;
    let cert = artifacts().join("ws02-https-cert.pem");
    let key = artifacts().join("ws02-https-key.pem");
    let log = fs::OpenOptions::new()
        .create_new(true)
        .write(true)
        .mode(0o600)
        .open(artifacts().join("ws02-https-cert.log"))
        .unwrap();
    assert!(
        Command::new("openssl")
            .args([
                "req",
                "-x509",
                "-newkey",
                "rsa:2048",
                "-noenc",
                "-days",
                "1",
                "-subj",
                "/CN=ws.same-origin.test",
                "-addext",
                "subjectAltName=DNS:ws.same-origin.test",
                "-addext",
                "basicConstraints=critical,CA:FALSE",
                "-addext",
                "extendedKeyUsage=serverAuth",
                "-keyout"
            ])
            .arg(&key)
            .arg("-out")
            .arg(&cert)
            .stdout(Stdio::null())
            .stderr(log)
            .status()
            .unwrap()
            .success()
    );
    let mut document = lab.document(true);
    document["responses_websocket_origins"] = json!([]);
    let core = Core::start(&lab, &document, None, false, false).await;
    let mut front = TlsFront::start(&core, "https-same-origin", &cert, &key);
    let origin = format!("https://ws.same-origin.test:{}", front.address.port());
    let response = front.request(&lab.fixture.key, &origin, &cert).await;
    assert_eq!(response.status(), StatusCode::SWITCHING_PROTOCOLS);
    assert_eq!(
        response.headers()["sec-websocket-accept"],
        "s3pPLMBiTxaQ9kYGzzhZRbK+xOo="
    );
    let mut socket = WebSocketStream::from_raw_socket(
        response.upgrade().await.unwrap(),
        tokio_tungstenite::tungstenite::protocol::Role::Client,
        None,
    )
    .await;
    send(&mut socket, create("root", "real-https-turn")).await;
    terminal(&mut socket).await;
    assert_eq!(lab.calls(), 1);
    lab.balance(FUNDS - COST, 0).await;
    drop(socket);
    drop(front);
    let before = lab.money().await;
    for (label, origin) in [
        ("https-foreign", "https://foreign.ws.test"),
        ("https-wrong-port", "https://ws.same-origin.test:1"),
        ("https-wrong-scheme", "http://ws.same-origin.test"),
        ("https-origin-path", "https://ws.same-origin.test/path"),
    ] {
        let mut front = TlsFront::start(&core, label, &cert, &key);
        // Match the actual TLS port for the scheme-negative case.
        let origin = if label == "https-wrong-scheme" {
            format!("http://ws.same-origin.test:{}", front.address.port())
        } else {
            origin.into()
        };
        let response = front.request(&lab.fixture.key, &origin, &cert).await;
        assert_eq!(response.status(), StatusCode::FORBIDDEN, "{label}");
        drop(response);
        drop(front);
        assert_eq!(lab.money().await, before);
        assert_eq!(lab.calls(), 1);
    }
    // A plaintext request may not claim HTTPS merely by choosing that Origin.
    let mut request = "ws://ws.same-origin.test/v1/responses"
        .into_client_request()
        .unwrap();
    request
        .headers_mut()
        .insert("origin", "https://ws.same-origin.test".parse().unwrap());
    request.headers_mut().insert(
        "authorization",
        format!("Bearer {}", lab.fixture.key).parse().unwrap(),
    );
    let error = tokio_tungstenite::client_async(
        request,
        tokio::net::TcpStream::connect(core.base.strip_prefix("http://").unwrap())
            .await
            .unwrap(),
    )
    .await
    .unwrap_err();
    assert!(
        matches!(error,tokio_tungstenite::tungstenite::Error::Http(ref r) if r.status()==StatusCode::FORBIDDEN)
    );
    drop(core);
    let core = Core::start(&lab, &lab.document(true), None, false, false).await;
    for (label, origin, status) in [
        (
            "https-explicit-trusted",
            "https://trusted.ws.test:443",
            StatusCode::SWITCHING_PROTOCOLS,
        ),
        (
            "https-trusted-wrong-scheme",
            "http://trusted.ws.test",
            StatusCode::FORBIDDEN,
        ),
        (
            "https-trusted-wrong-port",
            "https://trusted.ws.test:444",
            StatusCode::FORBIDDEN,
        ),
    ] {
        let mut front = TlsFront::start(&core, label, &cert, &key);
        let response = front.request(&lab.fixture.key, origin, &cert).await;
        assert_eq!(response.status(), status, "{label}");
        drop(response);
        drop(front);
    }
    assert_eq!(lab.money().await, before);
    assert_eq!(lab.calls(), 1);
    drop(core);
    lab.cleanup().await;
}

#[sqlx::test(migrations = false)]
async fn ws03_old_response_envelope_raw_schema_controls_and_late_errors(pool: PgPool) {
    let lab = Lab::new(pool).await;
    let core = Core::start(&lab, &lab.document(true), None, false, false).await;
    let (mut socket, _) = connect(&core, &lab.fixture.key, None, None).await.unwrap();
    let raw = format!(
        r#"{{"type":"response.create","event_id":"create-A","stream_id":"lane-A","response":{{"model":"{MODEL}","input":"controls","max_output_tokens":50,"tools":[{{"type":"function","name":"f","parameters":{{"type":"object","properties":{{"z":{{"const":9007199254740993}},"a":{{"type":"string"}}}}}}}}],"prompt_cache_options":{{"ttl":"12h"}}}}}}"#
    );
    socket.send(Message::Text(raw.into())).await.unwrap();
    assert_eq!(receive(&mut socket).await.1["type"], "response.created");
    let wire = lab.peer.supplier.creates.lock().unwrap()[0].clone();
    assert!(wire.contains("9007199254740993"));
    assert!(wire.find("\"z\"").unwrap() < wire.find("\"a\"").unwrap());
    let before = lab.money().await;
    send(&mut socket, create("root", "busy-create")).await;
    let busy = receive(&mut socket).await.1;
    assert_eq!(busy["status"], 409);
    assert_eq!(busy["event_id"], "busy-create");
    assert_eq!(lab.calls(), 1);
    assert_eq!(lab.money().await, before);
    send(
        &mut socket,
        json!({"type":"response.cancel","event_id":"bad-cancel","stream_id":"foreign"}),
    )
    .await;
    let e = receive(&mut socket).await.1;
    assert_eq!(e["status"], 400);
    assert_eq!(e["event_id"], "bad-cancel");
    assert_eq!(e["stream_id"], "foreign");
    assert!(lab.peer.supplier.controls.lock().unwrap().is_empty());
    let control =
        r#"{"type":"input_audio_buffer.clear","event_id":"control-A","stream_id":"lane-A"}"#;
    socket.send(Message::Text(control.into())).await.unwrap();
    let e = receive(&mut socket).await.1;
    assert_eq!(e["event_id"], "control-A");
    assert_eq!(e["stream_id"], "lane-A");
    assert_eq!(e["error"]["code"], "synthetic_control_rejected");
    assert_eq!(
        lab.peer.supplier.controls.lock().unwrap().as_slice(),
        [control]
    );
    assert_eq!(lab.money().await, before);
    lab.peer.supplier.release.add_permits(1);
    let (terminal_bytes, completed) = terminal(&mut socket).await;
    assert_eq!(completed["stream_id"], "lane-A");
    let row:(Vec<u8>,i64)=sqlx::query_as("SELECT packet_sha256,packet_bytes FROM core_model.output_checkpoints WHERE terminal_observed").fetch_one(&lab.fixture.pool).await.unwrap();
    assert_eq!(row.0, Sha256::digest(terminal_bytes.as_bytes()).to_vec());
    assert_eq!(row.1, terminal_bytes.len() as i64);
    let before = lab.money().await;
    // Idle controls also have a reader and keep their original error identity.
    send(
        &mut socket,
        json!({"type":"input_audio_buffer.clear","event_id":"idle-control","stream_id":"old-lane"}),
    )
    .await;
    let e = receive(&mut socket).await.1;
    assert_eq!(e["event_id"], "idle-control");
    assert_eq!(e["stream_id"], "old-lane");
    assert_eq!(lab.money().await, before);
    send(&mut socket,json!({"type":"response.create","event_id":"invalid-model","stream_id":3,"model":MODEL,"input":"x"})).await;
    let e = receive(&mut socket).await.1;
    assert_eq!(e["event_id"], "invalid-model");
    assert!(e.get("stream_id").is_none());
    send(&mut socket, create("root", "valid-after-error")).await;
    terminal(&mut socket).await;
    assert_eq!(lab.calls(), 2);
    drop(socket);
    drop(core);
    lab.cleanup().await;
}

#[sqlx::test(migrations = false)]
async fn ws04_actual_route_plan_targets_and_frozen_provider_binding(pool: PgPool) {
    let lab = Lab::new(pool).await;
    let other = Peer::start().await;
    let third = Peer::start().await;
    let mut document = lab.document(true);
    document["routing"]["upstreams"].as_array_mut().unwrap().extend([
        json!({"id":2,"endpoint":other.supplier.base,"protocol":"openai_chat","credential":{"id":7,"revision":1},"enabled":true}),
        json!({"id":3,"endpoint":third.supplier.base,"protocol":"openai_responses","credential":{"id":7,"revision":1},"enabled":true})]);
    let target = document["routing"]["routes"][0]["targets"][0].clone();
    let mut unsupported = target.clone();
    unsupported["id"] = json!(2);
    unsupported["upstream_id"] = json!(2);
    unsupported["priority"] = json!(0);
    let mut supported = target.clone();
    supported["priority"] = json!(1);
    let mut tail = target;
    tail["id"] = json!(3);
    tail["upstream_id"] = json!(3);
    tail["priority"] = json!(2);
    document["routing"]["routes"][0]["targets"] = json!([unsupported, supported, tail]);
    document["routing"]["routes"][0]["failover"]["max_attempts"] = json!(3);
    let gateway = crate::config::model_gateway(
        lab.runtime.pool.clone(),
        document_toml(&document).as_bytes(),
        true,
    )
    .unwrap();
    let plans = gateway
        .bridge
        .routing
        .route_responses_ws_groups(MODEL, &["allowed".into()], Some([7; 32]), None)
        .unwrap();
    let ids = plans[0]
        .planned_attempts()
        .iter()
        .map(|a| a.target_id())
        .collect::<Vec<_>>();
    assert_eq!(ids, vec![1, 3]);
    let core = Core::start(&lab, &document, None, false, false).await;
    let (mut socket, _) = connect(&core, &lab.fixture.key, None, None).await.unwrap();
    send(&mut socket, create("root", "actual-plan")).await;
    let (_, terminal) = terminal(&mut socket).await;
    assert_eq!(lab.calls(), 1);
    assert!(other.supplier.creates.lock().unwrap().is_empty());
    assert!(third.supplier.creates.lock().unwrap().is_empty());
    let row: (Value,) = sqlx::query_as(
        "SELECT detail FROM core_charging.usage_events WHERE event_id='reservation-contract:v1'",
    )
    .fetch_one(&lab.fixture.pool)
    .await
    .unwrap();
    let serialized = row.0.to_string();
    assert!(serialized.contains("responses-ws-context-max-v1"));
    assert!(serialized.contains(PROVIDER_MODEL));
    // A fresh socket cannot borrow another connection's provider response ID.
    let before = lab.money().await;
    let (mut other_socket, _) = connect(&core, &lab.fixture.key, None, None).await.unwrap();
    let mut continued = create("new delta", "foreign-parent");
    continued["previous_response_id"] = terminal["response"]["id"].clone();
    send(&mut other_socket, continued).await;
    assert_eq!(receive(&mut other_socket).await.1["status"], 403);
    assert_eq!(lab.calls(), 1);
    assert_eq!(lab.money().await, before);
    drop(other_socket);
    drop(socket);
    drop(core);
    drop(other);
    drop(third);
    lab.cleanup().await;
}

#[sqlx::test(migrations = false)]
async fn ws05_complete_context_rev4_cache_funds_checkpoint_and_live_budget(pool: PgPool) {
    let lab = Lab::new(pool).await;
    let core = Core::start(&lab, &lab.document(true), None, false, false).await;
    let (mut socket, _) = connect(&core, &lab.fixture.key, None, None).await.unwrap();
    send(&mut socket, create("held", "root-context")).await;
    assert_eq!(receive(&mut socket).await.1["type"], "response.created");
    let rows = lab.charges().await;
    assert_eq!((rows[0].2, rows[0].3), (HOLD, 0));
    lab.balance(FUNDS - HOLD, HOLD).await;
    lab.peer.supplier.release.add_permits(1);
    let (_, root) = terminal(&mut socket).await;
    lab.balance(FUNDS - COST, 0).await;
    let mut child = create("new delta", "child-context");
    child["previous_response_id"] = root["response"]["id"].clone();
    send(&mut socket, child).await;
    terminal(&mut socket).await;
    let rows = lab.charges().await;
    assert_eq!(rows.iter().map(|r| r.3).sum::<i64>(), 384);
    assert!(rows.iter().any(|r| r.3 == 345));
    assert!(rows.iter().all(|r| r.2 == HOLD && r.1 == "settled"));
    lab.balance(FUNDS - 384, 0).await;
    let body: Value = serde_json::from_str(&lab.peer.supplier.creates.lock().unwrap()[1]).unwrap();
    assert_eq!(body["previous_response_id"], root["response"]["id"]);
    assert_eq!(body["input"][0]["content"][0]["text"], "new delta");
    assert_eq!(body["prompt_cache_options"]["ttl"], "12h");
    let captures: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM core_billing.ledger_journals WHERE kind='capture'",
    )
    .fetch_one(&lab.fixture.pool)
    .await
    .unwrap();
    assert_eq!(captures, 2);
    let gateway = crate::config::model_gateway(
        lab.runtime.pool.clone(),
        document_toml(&lab.document(true)).as_bytes(),
        true,
    )
    .unwrap();
    let identity = IdentityStore::from_pool(lab.runtime.pool.clone())
        .with_dashboard_routing(gateway.bridge.routing.clone());
    let input=serde_json::from_value(json!({"id":lab.fixture.key_id,"name":"ws-budget","expired_time":-1,"remain_quota":0,"unlimited_quota":false,"model_limits_enabled":true,"model_limits":MODEL,"group":"allowed"})).unwrap();
    identity
        .dashboard_update_key(&lab.fixture.session, &input, false)
        .await
        .unwrap();
    let before = lab.money().await;
    send(&mut socket, create("root", "budget-rejected")).await;
    assert_eq!(receive(&mut socket).await.1["type"], "error");
    assert_eq!(lab.calls(), 2);
    assert_eq!(lab.money().await, before);
    assert_eq!(lab.charges().await.len(), 2);
    drop(socket);
    drop(core);
    lab.cleanup().await;
}

#[sqlx::test(migrations = false)]
async fn ws06_official_exact_rejects_before_handshake_and_unknown_usage_holds(pool: PgPool) {
    let lab = Lab::new(pool).await;
    let mut document = lab.document(true);
    document["routing"]["upstreams"][0]["endpoint"] = json!("https://api.openai.com/v1");
    document["external_models"] = json!([{"upstream_id":1,"upstream_model":PROVIDER_MODEL,"protocol":"openai_responses","endpoint":"https://api.openai.com/v1","max_input_tokens":800,"max_output_tokens":50,"contract_revision":"synthetic-official-exact-rejection","reservation":{"mode":"openai_responses_exact_v1","count_timeout_ms":100}}]);
    let core = Core::start(&lab, &document, None, false, false).await;
    let (mut socket, _) = connect(&core, &lab.fixture.key, None, None).await.unwrap();
    let before = lab.money().await;
    send(&mut socket, create("root", "exact-forbidden")).await;
    let e = receive(&mut socket).await.1;
    assert_eq!(e["type"], "error");
    assert_eq!(
        e["error"]["message"],
        RelayError::Unsupported("Responses WebSocket requires context-max reservation").to_string()
    );
    assert_eq!(lab.money().await, before);
    assert!(lab.charges().await.is_empty());
    assert_eq!(lab.peer.supplier.handshakes.load(Ordering::SeqCst), 0);
    drop(socket);
    drop(core);
    for (index, input) in ["missing", "missing12h", "contradiction"]
        .iter()
        .enumerate()
    {
        let core = Core::start(&lab, &lab.document(true), None, false, false).await;
        let (mut socket, _) = connect(&core, &lab.fixture.key, None, None).await.unwrap();
        let before: std::collections::BTreeSet<_> =
            lab.charges().await.into_iter().map(|r| r.0).collect();
        send(&mut socket, create(input, &format!("unknown-{index}"))).await;
        let e = receive(&mut socket).await.1;
        assert_eq!(e["type"], "response.created");
        let e = receive(&mut socket).await.1;
        assert_eq!(e["type"], "error");
        let row = lab
            .charges()
            .await
            .into_iter()
            .find(|r| !before.contains(&r.0))
            .unwrap();
        let row = await_state(&lab, &row.0, "reconcile").await;
        assert_eq!((row.2, row.3), (HOLD, 0));
        drop(socket);
        drop(core);
        let before = lab.money().await;
        let calls = lab.calls();
        tokio::time::sleep(Duration::from_secs(17)).await;
        let core = Core::start(&lab, &lab.document(true), None, true, false).await;
        assert_eq!(lab.money().await, before);
        assert_eq!(lab.calls(), calls);
        drop(core);
    }
    lab.cleanup().await;
}

#[sqlx::test(migrations = false)]
async fn ws07_client_disconnect_confirmed_terminal_and_provider_unknown(pool: PgPool) {
    let lab = Lab::new(pool).await;
    let core = Core::start(&lab, &lab.document(true), None, false, false).await;
    let (mut socket, _) = connect(&core, &lab.fixture.key, None, None).await.unwrap();
    send(&mut socket, create("held", "client-disconnect")).await;
    receive(&mut socket).await;
    let row = lab.charges().await.pop().unwrap();
    drop(socket);
    lab.peer.supplier.release.add_permits(1);
    assert_eq!(await_state(&lab, &row.0, "settled").await.3, COST);
    lab.balance(FUNDS - COST, 0).await;
    let (mut socket, _) = connect(&core, &lab.fixture.key, None, None).await.unwrap();
    send(&mut socket, create("unknown", "provider-disconnect")).await;
    receive(&mut socket).await;
    let row = lab
        .charges()
        .await
        .into_iter()
        .find(|r| r.1 != "settled")
        .unwrap();
    assert_eq!(await_state(&lab, &row.0, "reconcile").await.3, 0);
    lab.balance(FUNDS - COST - HOLD, HOLD).await;
    drop(socket);
    drop(core);
    lab.cleanup().await;
}

#[sqlx::test(migrations = false)]
async fn ws08_real_sigkill_checkpoint_recovery_and_repeat_do_not_redispatch(pool: PgPool) {
    let lab = Lab::new(pool).await;
    for stage in ["reserved", "dispatched", "checkpoint", "settled"] {
        let before_ids: std::collections::BTreeSet<_> =
            lab.charges().await.into_iter().map(|r| r.0).collect();
        let mut core = Core::start(&lab, &lab.document(true), Some(stage), false, false).await;
        let (mut socket, _) = connect(&core, &lab.fixture.key, None, None).await.unwrap();
        send(&mut socket, create("root", stage)).await;
        for _ in 0..400 {
            if lab.directory.join(format!("{stage}.reached")).exists() {
                break;
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
        assert!(lab.directory.join(format!("{stage}.reached")).exists());
        let row = lab
            .charges()
            .await
            .into_iter()
            .find(|r| !before_ids.contains(&r.0))
            .unwrap();
        let frozen:Value=sqlx::query_scalar("SELECT detail FROM core_charging.usage_events WHERE request_id=$1 AND event_id='reservation-contract:v1'").bind(&row.0).fetch_one(&lab.fixture.pool).await.unwrap();
        let calls = lab.calls();
        core.kill9();
        drop(socket);
        drop(core);
        tokio::time::sleep(Duration::from_secs(17)).await;
        let core = Core::start(&lab, &lab.document(true), None, true, false).await;
        let settled = matches!(stage, "checkpoint" | "settled");
        let recovered =
            await_state(&lab, &row.0, if settled { "settled" } else { "reconcile" }).await;
        assert_eq!(recovered.3, if settled { COST } else { 0 });
        assert_eq!(recovered.2, HOLD);
        assert_eq!(lab.calls(), calls);
        let after:Value=sqlx::query_scalar("SELECT detail FROM core_charging.usage_events WHERE request_id=$1 AND event_id='reservation-contract:v1'").bind(&row.0).fetch_one(&lab.fixture.pool).await.unwrap();
        assert_eq!(after, frozen);
        drop(core);
        let money = lab.money().await;
        let core = Core::start(&lab, &lab.document(true), None, true, false).await;
        assert_eq!(lab.money().await, money);
        assert_eq!(lab.calls(), calls);
        drop(core);
    }
    let unbalanced:i64=sqlx::query_scalar("SELECT count(*) FROM (SELECT journal_id FROM core_billing.ledger_entries GROUP BY journal_id HAVING sum(delta_units)<>0) j").fetch_one(&lab.fixture.pool).await.unwrap();
    assert_eq!(unbalanced, 0);
    lab.cleanup().await;
}

#[sqlx::test(migrations = false)]
async fn ws09_ping_frame_limits_pending_controls_and_shutdown_preserve_hold(pool: PgPool) {
    let lab = Lab::new(pool).await;
    let mut document = lab.document(true);
    document["timeout_seconds"] = json!(30);
    document["lease_seconds"] = json!(60);
    document["routing"]["routes"][0]["targets"][0]["timeout_ms"] = json!(30_000);
    let core = Core::start(&lab, &document, None, false, false).await;
    let (mut socket, _) = connect(&core, &lab.fixture.key, None, None).await.unwrap();
    socket
        .send(Message::Ping(vec![1, 2, 3].into()))
        .await
        .unwrap();
    let reply = tokio::time::timeout(Duration::from_secs(3), socket.next())
        .await
        .unwrap()
        .unwrap()
        .unwrap();
    assert_eq!(reply, Message::Pong(vec![1, 2, 3].into()));
    send(&mut socket, create("controls", "bounded-controls")).await;
    let created = receive(&mut socket).await.1;
    assert_eq!(created["type"], "response.created", "{created}");
    assert_eq!(created["stream_id"], "lane-A");
    let before = lab.money().await;
    // Unidentifiable controls, including owned cancels, cannot consume named
    // pending slots. A wrong-owner anonymous cancel still fails before send.
    send(
        &mut socket,
        json!({"type":"response.cancel","stream_id":"foreign"}),
    )
    .await;
    assert_eq!(receive(&mut socket).await.1["status"], 400);
    assert!(lab.peer.supplier.controls.lock().unwrap().is_empty());
    for _ in 0..32 {
        for kind in ["input_audio_buffer.clear", "response.cancel"] {
            send(
                &mut socket,
                json!({"type":kind,"stream_id":"lane-A","response_id":created["response"]["id"]}),
            )
            .await;
            assert_eq!(
                receive(&mut socket).await.1["error"]["code"],
                "synthetic_control_rejected"
            );
        }
    }
    assert_eq!(lab.peer.supplier.controls.lock().unwrap().len(), 64);
    assert_eq!(lab.money().await, before);
    // Duplicate rejected control ID is not forwarded a second time or billed.
    let body = json!({"type":"input_audio_buffer.clear","event_id":"once","stream_id":"lane-A"});
    send(&mut socket, body.clone()).await;
    receive(&mut socket).await;
    send(&mut socket, body).await;
    let error = receive(&mut socket).await.1;
    assert_eq!(error["status"], 400);
    assert_eq!(lab.peer.supplier.controls.lock().unwrap().len(), 65);
    assert_eq!(
        lab.peer
            .supplier
            .controls
            .lock()
            .unwrap()
            .iter()
            .filter(|raw| serde_json::from_str::<Value>(raw).unwrap()["event_id"] == "once")
            .count(),
        1
    );
    assert_eq!(lab.money().await, before);
    // Oversized client input must close without a second create or release.
    socket
        .send(Message::Text("x".repeat(2 * 1024 * 1024 + 1).into()))
        .await
        .unwrap();
    let _ = tokio::time::timeout(Duration::from_secs(5), socket.next()).await;
    tokio::time::sleep(Duration::from_millis(50)).await;
    assert_eq!(lab.calls(), 1);
    assert_eq!(lab.charges().await[0].3, 0);
    lab.balance(FUNDS - HOLD, HOLD).await;
    drop(socket);
    drop(core);

    // Keep the original UNKNOWN hold above. A fresh connection exercises real
    // pending records without weakening that oversized-frame financial check.
    let core = Core::start(&lab, &document, None, false, false).await;
    let (mut socket, _) = connect(&core, &lab.fixture.key, None, None).await.unwrap();
    send(&mut socket, create("controls", "named-pending-turn")).await;
    receive(&mut socket).await;
    let before = lab.money().await;
    for n in 0..31 {
        send(&mut socket,json!({"type":"input_audio_buffer.clear","event_id":format!("pending-first-{n}"),"stream_id":"lane-A"})).await;
    }
    send(&mut socket,json!({"type":"input_audio_buffer.clear","event_id":"pending-first-0","stream_id":"lane-A"})).await;
    assert_eq!(
        receive(&mut socket).await.1["error"]["message"],
        "invalid_request: duplicate control identity"
    );
    send(&mut socket,json!({"type":"input_audio_buffer.clear","event_id":"pending-first-31","stream_id":"lane-A"})).await;
    send(&mut socket,json!({"type":"input_audio_buffer.clear","event_id":"pending-over-limit","stream_id":"lane-A"})).await;
    assert_eq!(
        receive(&mut socket).await.1["error"]["message"],
        "invalid_request: pending control limit"
    );
    // Anonymous traffic does not release or bypass the 32 named pending slots.
    send(
        &mut socket,
        json!({"type":"response.cancel","stream_id":"lane-A"}),
    )
    .await;
    assert_eq!(
        receive(&mut socket).await.1["error"]["code"],
        "synthetic_control_rejected"
    );
    send(&mut socket,json!({"type":"input_audio_buffer.clear","event_id":"pending-still-over-limit","stream_id":"lane-A"})).await;
    assert_eq!(
        receive(&mut socket).await.1["error"]["message"],
        "invalid_request: pending control limit"
    );
    assert_eq!(lab.money().await, before);
    let sent = lab.peer.supplier.controls.lock().unwrap().clone();
    assert_eq!(
        sent.iter()
            .filter(
                |raw| serde_json::from_str::<Value>(raw).unwrap()["event_id"]
                    .as_str()
                    .is_some_and(|id| id.starts_with("pending-first-"))
            )
            .count(),
        32
    );
    assert!(
        !sent
            .iter()
            .any(|raw| raw.contains("pending-over-limit")
                || raw.contains("pending-still-over-limit"))
    );
    // A real correlated supplier error frees exactly one slot, and the next
    // control traverses Core to the supplier with its original bytes.
    send(
        &mut socket,
        json!({"type":"test.emit_control_error","target_event_id":"pending-first-0"}),
    )
    .await;
    let error = receive(&mut socket).await.1;
    assert_eq!(error["event_id"], "pending-first-0");
    assert_eq!(error["stream_id"], "lane-A");
    let replacement = json!({"type":"input_audio_buffer.clear","event_id":"pending-replacement","stream_id":"lane-A"});
    send(&mut socket, replacement.clone()).await;
    for _ in 0..300 {
        if lab
            .peer
            .supplier
            .controls
            .lock()
            .unwrap()
            .iter()
            .any(|raw| raw == &replacement.to_string())
        {
            break;
        }
        tokio::time::sleep(Duration::from_millis(5)).await;
    }
    assert!(
        lab.peer
            .supplier
            .controls
            .lock()
            .unwrap()
            .iter()
            .any(|raw| raw == &replacement.to_string())
    );
    assert_eq!(lab.money().await, before);
    lab.peer.supplier.release.add_permits(1);
    terminal(&mut socket).await;
    lab.balance(FUNDS - HOLD - COST, HOLD).await;

    // Completed turns release pending entries but keep their event IDs for
    // deduplication. A successful cancel is itself confirmed terminal evidence.
    send(&mut socket, create("controls", "cancel-pending-turn")).await;
    let created = receive(&mut socket).await.1;
    let before = lab.money().await;
    send(
        &mut socket,
        json!({"type":"input_audio_buffer.clear","event_id":"pending-first-1"}),
    )
    .await;
    assert_eq!(
        receive(&mut socket).await.1["error"]["message"],
        "invalid_request: duplicate control identity"
    );
    for n in 0..31 {
        send(&mut socket,json!({"type":"input_audio_buffer.clear","event_id":format!("pending-cancel-{n}"),"stream_id":"lane-A"})).await;
    }
    assert_eq!(lab.money().await, before);
    send(&mut socket,json!({"type":"response.cancel","event_id":"cancel-success","stream_id":"lane-A","response_id":created["response"]["id"]})).await;
    let cancelled = receive(&mut socket).await.1;
    assert_eq!(cancelled["type"], "response.cancelled");
    assert_eq!(cancelled["stream_id"], "lane-A");
    assert_eq!(cancelled["response"]["id"], created["response"]["id"]);
    lab.balance(FUNDS - HOLD - 2 * COST, HOLD).await;

    send(&mut socket, create("controls", "after-successful-cancel")).await;
    receive(&mut socket).await;
    let before = lab.money().await;
    for n in 0..32 {
        send(&mut socket,json!({"type":"input_audio_buffer.clear","event_id":format!("pending-after-cancel-{n}"),"stream_id":"lane-A"})).await;
    }
    send(
        &mut socket,
        json!({"type":"input_audio_buffer.clear","event_id":"pending-last-over-limit"}),
    )
    .await;
    assert_eq!(
        receive(&mut socket).await.1["error"]["message"],
        "invalid_request: pending control limit"
    );
    let sent = lab.peer.supplier.controls.lock().unwrap().clone();
    assert_eq!(
        sent.iter()
            .filter(
                |raw| serde_json::from_str::<Value>(raw).unwrap()["event_id"]
                    .as_str()
                    .is_some_and(|id| id.starts_with("pending-after-cancel-"))
            )
            .count(),
        32
    );
    assert!(
        !sent
            .iter()
            .any(|raw| raw.contains("pending-last-over-limit"))
    );
    assert_eq!(lab.money().await, before);
    lab.peer.supplier.release.add_permits(1);
    terminal(&mut socket).await;
    lab.balance(FUNDS - HOLD - 3 * COST, HOLD).await;
    assert_eq!(lab.calls(), 4);
    assert_eq!(
        lab.charges().await.iter().map(|row| row.3).sum::<i64>(),
        3 * COST
    );
    drop(socket);
    drop(core);
    lab.cleanup().await;
}

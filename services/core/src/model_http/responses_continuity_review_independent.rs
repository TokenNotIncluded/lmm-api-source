//! Independent continuity regressions through real TCP and PostgreSQL.
//! No test installs a grant or changes a historical financial row to get a pass.
use super::responses_continuity_review_support::{
    MODEL, OAuthFixture, OAuthToken, PUBLIC, RetryFixture, RetryLayout, SECRET,
};
use super::responses_plan_observer::PlanTrace;
use super::*;
use crate::billing::charging::{Outcome, Usage};
use crate::routing::{
    CredentialRef, Endpoint, PriceConfig, PriceSnapshot, Protocol, RoutingConfig,
};
use axum::extract::{OriginalUri, State};
use futures_util::{FutureExt, future::BoxFuture};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use sqlx::Row;
use std::{
    collections::BTreeSet,
    fs,
    io::Write,
    os::unix::{fs::OpenOptionsExt, process::ExitStatusExt},
    panic::AssertUnwindSafe,
    path::PathBuf,
    process::{Child, Command, Stdio},
    sync::atomic::{AtomicUsize, Ordering},
};

// Runtime authority is provisioned by the canonical public fixture.
use crate::test_runtime as public_runtime_fixture;
use public_runtime_fixture::RuntimeRole;

// Keep PostgreSQL wire-fault support in the normal library test graph.
const _: usize = std::mem::size_of::<crate::test_pg_commit_fault::PgCommitFault>();

const HOLD: i64 = 2650;
const COST: i64 = 36;
const FUNDS: i64 = 100_000;
const LEASE: u64 = 8;
const CAPTURE_GATE: i64 = 8_240_691;

fn artifacts() -> PathBuf {
    PathBuf::from(
        std::env::var("RESPONSES_CONTINUITY_REVIEW_ARTIFACTS")
            .expect("new private review evidence directory required"),
    )
}
fn evidence(name: &str, value: Value) {
    let mut file = fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .mode(0o600)
        .open(artifacts().join(format!("{name}.json")))
        .unwrap();
    file.write_all(&serde_json::to_vec_pretty(&value).unwrap())
        .unwrap();
}
#[derive(Clone)]
struct Supplier {
    base: String,
    seen: Arc<Mutex<Vec<Value>>>,
    counts: Arc<AtomicUsize>,
    default_store: Arc<Mutex<bool>>,
    reviewed_cache_receipt: Arc<Mutex<bool>>,
    cache_usage: Arc<Mutex<Option<Value>>>,
    retained_parents: Arc<Mutex<BTreeMap<String, Value>>>,
    unexpected: Arc<Mutex<Vec<String>>>,
}
struct Server {
    supplier: Supplier,
    task: tokio::task::JoinHandle<()>,
}
impl Drop for Server {
    fn drop(&mut self) {
        self.task.abort();
    }
}
async fn provider(
    State(s): State<Supplier>,
    headers: HeaderMap,
    Json(body): Json<Value>,
) -> Response {
    assert_eq!(
        headers[header::AUTHORIZATION].to_str().unwrap(),
        format!("Bearer {SECRET}")
    );
    let number = {
        let mut seen = s.seen.lock().unwrap();
        seen.push(body.clone());
        seen.len()
    };
    let store = body
        .get("store")
        .and_then(Value::as_bool)
        .unwrap_or(*s.default_store.lock().unwrap());
    let mut response = json!({"id":format!("resp_review_supplier_{number}"),"object":"response","model":MODEL,
        "status":"completed","store":store,"output":[{"id":"msg_review","type":"message","role":"assistant",
        "status":"completed","content":[{"type":"output_text","text":"ok","annotations":[]}]}],
        "usage":{"input_tokens":7,"output_tokens":3,"total_tokens":10,"input_tokens_details":{"cached_tokens":0}}});
    if *s.reviewed_cache_receipt.lock().unwrap() {
        response["usage"]["input_tokens_details"]["cache_write_tokens"] = json!(0);
        response["usage"]["input_tokens_details"]["cache_write_tokens_details"] =
            json!({"ttl_30m_tokens":0,"ttl_12h_tokens":0});
    }
    if let Some(usage) = s.cache_usage.lock().unwrap().clone() {
        response["usage"] = usage;
        if let Some(parent) = body["previous_response_id"].as_str() {
            let previous = s
                .retained_parents
                .lock()
                .unwrap()
                .get(parent)
                .cloned()
                .expect("supplier did not retain this parent");
            response["output"][0]["content"][0]["text"] =
                json!(format!("{} -> {}", previous, body["input"]));
        }
        if store {
            s.retained_parents.lock().unwrap().insert(
                format!("resp_review_supplier_{number}"),
                body["input"].clone(),
            );
        }
    }
    if body["stream"] == true {
        let created = json!({"type":"response.created","response":{"id":response["id"],"status":"in_progress"}});
        let terminal = json!({"type":"response.completed","response":response});
        return ([(header::CONTENT_TYPE,"text/event-stream")],
            format!("event: response.created\ndata: {created}\n\nevent: response.completed\ndata: {terminal}\n\n")).into_response();
    }
    Json(response).into_response()
}
async fn supplier() -> Server {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let s = Supplier {
        base: format!("http://{}/v1", listener.local_addr().unwrap()),
        seen: Default::default(),
        counts: Arc::new(AtomicUsize::new(0)),
        default_store: Arc::new(Mutex::new(true)),
        reviewed_cache_receipt: Arc::new(Mutex::new(false)),
        cache_usage: Arc::new(Mutex::new(None)),
        retained_parents: Arc::new(Mutex::new(BTreeMap::new())),
        unexpected: Default::default(),
    };
    let counts = s.counts.clone();
    let router = Router::new()
        .route("/v1/responses", axum::routing::post(provider))
        .route(
            "/v1/responses/input_tokens",
            axum::routing::post(move || {
                let counts = counts.clone();
                async move {
                    counts.fetch_add(1, Ordering::SeqCst);
                    Json(json!({"input_tokens":1}))
                }
            }),
        )
        .fallback(
            |State(s): State<Supplier>, OriginalUri(uri): OriginalUri| async move {
                s.unexpected.lock().unwrap().push(uri.to_string());
                StatusCode::BAD_GATEWAY
            },
        )
        .with_state(s.clone());
    let task = tokio::spawn(async move {
        axum::serve(listener, router).await.unwrap();
    });
    Server { supplier: s, task }
}
#[derive(Clone, Serialize, Deserialize)]
struct Config {
    database: String,
    bases: Vec<String>,
    capabilities: [Option<bool>; 2],
    order: [usize; 2],
    ready: String,
    #[serde(default)]
    reviewed_cache_tariff: bool,
    #[serde(default)]
    nonflat_cache_tariff: bool,
}
fn gateway(pool: &PgPool, config: &Config) -> ModelGateway {
    let mut routes:RoutingConfig=serde_json::from_value(json!({"version":1,"groups":[{"name":"allowed","enabled":true}],
        "aliases":[],"upstreams":[{"id":1,"endpoint":"https://api.openai.com/v1","protocol":"openai_responses",
        "credential":{"id":7,"revision":1},"enabled":true},{"id":2,"endpoint":"https://api.openai.com/v1","protocol":"openai_responses",
        "credential":{"id":8,"revision":1},"enabled":true}],"routes":[{"id":1,"model":PUBLIC,"group":"allowed","enabled":true,
        "speeds":["standard"],"targets":config.order.iter().enumerate().map(|(priority,index)|json!({"id":index+1,
        "upstream_id":index+1,"upstream_model":MODEL,"enabled":true,"priority":priority,"weight":1,"speeds":["standard"],
        "payload":"adapt","timeout_ms":2000})).collect::<Vec<_>>(),
        "failover":{"max_attempts":1,"retry_on":[],"allow_replay_after_send":false}}]})).unwrap();
    for (index, upstream) in routes.upstreams.iter_mut().enumerate() {
        upstream.endpoint = Endpoint::local_loopback(config.bases[index].clone()).unwrap();
    }
    let mut price = json!({"id":1,"model":PUBLIC,"group":"allowed",
        "speed":"standard","rates":{"input":6000000,"output":10000000,"cache_read":6000000,"cache_write":0,"request":0},
        "group_multiplier_ppm":1000000,"speed_multiplier_ppm":1000000});
    if config.reviewed_cache_tariff {
        // Synthetic exact USD/M tariff: every input partition costs6, output10.
        // The original800/50 bound and measured7/3 retain HOLD2650/COST36.
        price["billing_mode"] = json!("tiered_decimal");
        price["rates"] = json!({"input":0,"output":0,"cache_read":0,"cache_write":0,"request":0});
        price["tiered"] = json!({"revision":4,"source_digest":"64".repeat(32),
            "uses_cache_read":true,"uses_cache_write":true,"uses_cache_write_1h":false,"uses_cache_write_12h":true,
            "tiers":[{"up_to_input_tokens":null,"rates":{
                "input":{"numerator":6,"denominator":1},"output":{"numerator":10,"denominator":1},
                "cache_read":{"numerator":6,"denominator":1},"cache_write":{"numerator":6,"denominator":1},
                "cache_write_12h":{"numerator":6,"denominator":1}}}]});
    }
    if config.nonflat_cache_tariff {
        assert!(config.reviewed_cache_tariff);
        price["tiered"]["source_digest"] = json!("65".repeat(32));
        let rates = &mut price["tiered"]["tiers"][0]["rates"];
        rates["cache_read"] = json!({"numerator":2,"denominator":1});
        rates["cache_write"] = json!({"numerator":8,"denominator":1});
        rates["cache_write_12h"] = json!({"numerator":12,"denominator":1});
    }
    let prices: PriceConfig = serde_json::from_value(json!({"version":1,"rules":[price]})).unwrap();
    let contexts=(0..2).map(|index| {
        let mut value=json!({"upstream_id":index+1,"upstream_model":MODEL,"protocol":"openai_responses",
            "endpoint":"https://api.openai.com/v1","max_input_tokens":800,"max_output_tokens":50,
            "contract_revision":"review-loopback-only-v1","responses_compaction":true,
            "missing_cached_tokens_is_zero":false,"missing_cache_creation_tokens_is_zero":false});
        if let Some(default_store)=config.capabilities[index] {value["responses_continuity"]=json!({"default_store":default_store});}
        serde_json::from_value::<ExternalModelContext>(value).unwrap()
    }).collect();
    ModelGateway::new_with_provider_policy(
        pool.clone(),
        Arc::new(
            crate::routing::Router::new(routes, Arc::new(PriceSnapshot::compile(prices).unwrap()))
                .unwrap(),
        ),
        (0..2)
            .map(|index| UpstreamCredential {
                reference: CredentialRef {
                    id: index + 7,
                    revision: 1,
                },
                secret: SECRET.into(),
            })
            .collect(),
        BTreeMap::new(),
        Limits {
            max_output_tokens: 50,
            total_timeout: Duration::from_secs(2),
            header_timeout: Duration::from_secs(2),
            hook_timeout: Duration::from_millis(500),
            ..Default::default()
        },
        ChargePolicy {
            input_token_ceiling: 800,
            lease_seconds: LEASE as i64,
        },
        ProviderPolicy::new(contexts, 800).unwrap(),
    )
    .unwrap()
}

/// Common gateway configuration for OAuth, legacy and group fixtures.
pub(super) fn fixture_gateway(
    pool: &PgPool,
    bases: &[String],
    capabilities: [Option<bool>; 2],
    order: [usize; 2],
) -> ModelGateway {
    gateway(
        pool,
        &Config {
            database: String::new(),
            bases: bases.to_vec(),
            capabilities,
            order,
            ready: String::new(),
            reviewed_cache_tariff: false,
            nonflat_cache_tariff: false,
        },
    )
}
#[tokio::test]
#[ignore = "private real HTTP child, not a business case; parent owns PG and provider"]
async fn responses_continuity_review_independent_core_worker() {
    let path = std::env::var("RESPONSES_CONTINUITY_REVIEW_WORKER_CONFIG").unwrap();
    let config: Config = serde_json::from_slice(&fs::read(path).unwrap()).unwrap();
    let pool = PgPool::connect(&config.database).await.unwrap();
    let gateway = gateway(&pool, &config);
    // No startup recovery: the parent deliberately controls individual host
    // pages and real background sweeps, including their persistent cursors.
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let temporary = PathBuf::from(&config.ready).with_extension("new");
    fs::write(
        &temporary,
        format!("http://{}", listener.local_addr().unwrap()),
    )
    .unwrap();
    fs::rename(temporary, &config.ready).unwrap();
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
    async fn start(mut config: Config, crash: Option<&str>) -> Self {
        let nonce = format!("{:016x}", rand::random::<u64>());
        let path = artifacts().join(format!("review-worker-{nonce}.private.json"));
        let ready = artifacts().join(format!("review-worker-{nonce}.ready"));
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
            .open(artifacts().join(format!("review-worker-{nonce}.log")))
            .unwrap();
        let mut command = Command::new(std::env::current_exe().unwrap());
        command.args(["--exact","model_http::responses_continuity_review_independent::responses_continuity_review_independent_core_worker",
            "--ignored","--nocapture","--test-threads=1"]).env("RESPONSES_CONTINUITY_REVIEW_WORKER_CONFIG",&path)
            .env_remove("DATABASE_URL").env_remove("LMM_RESPONSES_CONTINUITY_CRASH_AT")
            .env_remove("LMM_RESPONSES_CONTINUITY_LOSE_ACK_AT")
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
                "review child startup failed; retain private log"
            );
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
        panic!("review child startup timeout")
    }
    async fn exited97(&mut self) {
        for _ in 0..250 {
            if let Some(status) = self.child.try_wait().unwrap() {
                assert_eq!(status.code(), Some(97));
                return;
            }
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
        panic!("after_candidate must exit97 in actual child")
    }
    fn kill9(&mut self) {
        self.child.kill().unwrap();
        assert_eq!(self.child.wait().unwrap().signal(), Some(9));
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
fn client() -> reqwest::Client {
    reqwest::Client::builder()
        .no_proxy()
        .timeout(Duration::from_secs(12))
        .build()
        .unwrap()
}
async fn post(
    base: &str,
    key: &str,
    nonce: &str,
    body: &Value,
) -> Result<reqwest::Response, reqwest::Error> {
    client()
        .post(format!("{base}/v1/responses"))
        .bearer_auth(key)
        .header("idempotency-key", nonce)
        .json(body)
        .send()
        .await
}
struct Lab {
    fixture: native_http::test_support::Fixture,
    runtime: RuntimeRole,
    servers: Vec<Server>,
}
impl Lab {
    async fn new(pool: PgPool) -> Self {
        let fixture = native_http::test_support::fixture(pool, FUNDS, &[PUBLIC]).await;
        let runtime = RuntimeRole::create(&fixture.pool).await;
        runtime.optional("core_charging").await;
        runtime.optional("core_model").await;
        Self {
            fixture,
            runtime,
            servers: vec![supplier().await, supplier().await],
        }
    }
    fn config(&self, capabilities: [Option<bool>; 2], order: [usize; 2]) -> Config {
        for (index, server) in self.servers.iter().enumerate() {
            *server.supplier.default_store.lock().unwrap() = capabilities[index].unwrap_or(false);
        }
        Config {
            database: self.runtime.url.clone(),
            bases: self
                .servers
                .iter()
                .map(|s| s.supplier.base.clone())
                .collect(),
            capabilities,
            order,
            ready: String::new(),
            reviewed_cache_tariff: false,
            nonflat_cache_tariff: false,
        }
    }
    fn calls(&self) -> usize {
        self.servers
            .iter()
            .map(|s| s.supplier.seen.lock().unwrap().len())
            .sum()
    }
    fn counts(&self) -> usize {
        self.servers
            .iter()
            .map(|s| s.supplier.counts.load(Ordering::SeqCst))
            .sum()
    }
    fn unexpected(&self) -> usize {
        self.servers
            .iter()
            .map(|s| s.supplier.unexpected.lock().unwrap().len())
            .sum()
    }
    async fn cleanup(self) {
        drop(self.servers);
        self.runtime.cleanup().await;
    }
}
async fn run<F>(pool: PgPool, test: F)
where
    F: for<'a> FnOnce(&'a Lab) -> BoxFuture<'a, ()>,
{
    let lab = Lab::new(pool).await;
    let result = AssertUnwindSafe(test(&lab)).catch_unwind().await;
    lab.cleanup().await;
    if let Err(panic) = result {
        std::panic::resume_unwind(panic);
    }
}
async fn ids(pool: &PgPool) -> BTreeSet<String> {
    sqlx::query_scalar("SELECT id FROM core_charging.requests")
        .fetch_all(pool)
        .await
        .unwrap()
        .into_iter()
        .collect()
}
async fn charge(pool: &PgPool, id: &str) -> (String, i64, i64) {
    sqlx::query_as("SELECT state,reserved,settled FROM core_charging.requests WHERE id=$1")
        .bind(id)
        .fetch_one(pool)
        .await
        .unwrap()
}
async fn active(pool: &PgPool) -> BTreeSet<String> {
    sqlx::query_scalar("SELECT request_id FROM core_model.responses_continuations WHERE active")
        .fetch_all(pool)
        .await
        .unwrap()
        .into_iter()
        .collect()
}
async fn cancelled(pool: &PgPool) -> BTreeSet<String> {
    sqlx::query_scalar(
        "SELECT id FROM core_charging.requests WHERE state='settled' AND outcome='cancelled'",
    )
    .fetch_all(pool)
    .await
    .unwrap()
    .into_iter()
    .collect()
}
async fn eligible(pool: &PgPool) -> BTreeSet<String> {
    sqlx::query_scalar("SELECT c.request_id FROM core_model.responses_continuations c JOIN core_charging.requests r ON r.id=c.request_id JOIN core_model.output_checkpoints p ON p.request_id=c.request_id AND p.sequence=c.terminal_sequence WHERE NOT c.active AND r.state='settled' AND r.outcome='completed' AND p.terminal_observed")
        .fetch_all(pool).await.unwrap().into_iter().collect()
}
async fn money(pool: &PgPool) -> Value {
    sqlx::query_scalar("SELECT jsonb_build_object('balances',(SELECT jsonb_agg(to_jsonb(b) ORDER BY ledger_account_id) FROM core_billing.balance_state b),'journals',(SELECT count(*) FROM core_billing.ledger_journals))")
        .fetch_one(pool).await.unwrap()
}
fn stored_body() -> Value {
    json!({"model":PUBLIC,"input":"review seed","max_output_tokens":50,"store":true})
}

// Use the existing ID helper ONLY to arrange fixture key order. All states,
// money, returned cursors and provider calls are observed independently in PG
// and real HTTP; no business assertion computes its expected result this way.
fn ordered_nonces(key: i64, count: usize) -> Vec<(String, String)> {
    let mut values: Vec<_> = (0..count)
        .map(|index| {
            let nonce = format!("independent-review-seed-{index}");
            let mut headers = HeaderMap::new();
            headers.insert("idempotency-key", HeaderValue::from_str(&nonce).unwrap());
            (request_id(key, &headers).unwrap(), nonce)
        })
        .collect();
    values.sort();
    values
}
async fn seed_candidate(lab: &Lab, config: &Config, id: &str, nonce: &str) {
    let before = ids(&lab.fixture.pool).await;
    let mut core = Core::start(config.clone(), Some("after_candidate")).await;
    let response = post(&core.base, &lab.fixture.key, nonce, &stored_body()).await;
    if let Ok(response) = response {
        assert!(
            !response.status().is_success(),
            "crash must not return a successful response"
        );
    }
    core.exited97().await;
    assert_eq!(
        ids(&lab.fixture.pool)
            .await
            .difference(&before)
            .cloned()
            .collect::<Vec<_>>(),
        vec![id.to_owned()]
    );
    assert_eq!(
        charge(&lab.fixture.pool, id).await,
        ("streaming".into(), HOLD, 0)
    );
    let row:(bool,i64)=sqlx::query_as("SELECT c.active,p.packet_bytes FROM core_model.responses_continuations c JOIN core_model.output_checkpoints p ON p.request_id=c.request_id AND p.sequence=c.terminal_sequence WHERE c.request_id=$1 AND p.terminal_observed")
        .bind(id).fetch_one(&lab.fixture.pool).await.unwrap();
    assert!(
        !row.0 && row.1 > 0,
        "real candidate and terminal must commit before crash"
    );
}
async fn install_capture_gate(pool: &PgPool) {
    sqlx::raw_sql(sqlx::AssertSqlSafe(format!("CREATE SCHEMA review_seed_gate; CREATE FUNCTION review_seed_gate.wait_capture() RETURNS trigger LANGUAGE plpgsql AS $$ BEGIN PERFORM pg_advisory_xact_lock({CAPTURE_GATE}::bigint); RETURN NEW; END $$; CREATE TRIGGER review_capture_gate BEFORE INSERT ON core_billing.ledger_journals FOR EACH ROW WHEN (NEW.kind='capture') EXECUTE FUNCTION review_seed_gate.wait_capture()")))
        .execute(pool).await.unwrap();
}
async fn seed_completion_only(lab: &Lab, config: &Config, id: &str, nonce: &str) {
    let mut core = Core::start(config.clone(), None).await;
    install_capture_gate(&lab.fixture.pool).await;
    let mut owner = lab.fixture.pool.begin().await.unwrap();
    sqlx::query("SELECT pg_advisory_xact_lock($1)")
        .bind(CAPTURE_GATE)
        .execute(&mut *owner)
        .await
        .unwrap();

    let base = core.base.clone();
    let key = lab.fixture.key.clone();
    let nonce = nonce.to_owned();
    let mut requests = tokio::task::JoinSet::new();
    requests.spawn(async move {
        let mut body = stored_body();
        body["store"] = false.into();
        post(&base, &key, &nonce, &body).await
    });
    tokio::time::timeout(Duration::from_secs(5),async {
        loop {
            let blocked:bool=sqlx::query_scalar("SELECT EXISTS(SELECT FROM pg_locks WHERE locktype='advisory' AND database=(SELECT oid FROM pg_database WHERE datname=current_database()) AND NOT granted AND objsubid=1 AND classid=0 AND objid=$1::bigint::oid)")
                .bind(CAPTURE_GATE).fetch_one(&lab.fixture.pool).await.unwrap();
            if blocked {break;}tokio::time::sleep(Duration::from_millis(5)).await;
        }
    }).await.expect("real capture did not reach the PG barrier");
    core.kill9();
    owner.rollback().await.unwrap();
    if let Ok(response) = requests.join_next().await.unwrap().unwrap() {
        assert!(
            !response.status().is_success(),
            "no success before settlement"
        );
    }
    assert_eq!(
        charge(&lab.fixture.pool, id).await,
        ("streaming".into(), HOLD, 0)
    );
    let checkpoint:i64=sqlx::query_scalar("SELECT count(*) FROM core_model.output_checkpoints WHERE request_id=$1 AND terminal_observed")
        .bind(id).fetch_one(&lab.fixture.pool).await.unwrap();
    assert_eq!(checkpoint, 1);
    let maps: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM core_model.responses_continuations WHERE request_id=$1",
    )
    .bind(id)
    .fetch_one(&lab.fixture.pool)
    .await
    .unwrap();
    assert_eq!(maps, 0);
    sqlx::raw_sql("DROP TRIGGER review_capture_gate ON core_billing.ledger_journals; DROP SCHEMA review_seed_gate CASCADE")
        .execute(&lab.fixture.pool).await.unwrap();
}
async fn expire(gateway: &ModelGateway, lab: &Lab) {
    tokio::time::sleep(Duration::from_secs(LEASE + 1)).await;
    gateway.recover_expired(100).await.unwrap();
    let bad: i64 =
        sqlx::query_scalar("SELECT count(*) FROM core_charging.requests WHERE state<>'reconcile'")
            .fetch_one(&lab.fixture.pool)
            .await
            .unwrap();
    assert_eq!(bad, 0);
}
async fn host_settle(gateway: &ModelGateway, id: &str, outcome: Outcome) {
    let old = gateway.bridge.charging.resolve(id).await.unwrap().unwrap();
    assert_eq!(old.state, "reconcile");
    let settled = gateway
        .bridge
        .charging
        .reconcile(
            id,
            old.revision,
            Usage {
                input: 7,
                cached: 0,
                output: 3,
                ..Default::default()
            },
            outcome,
            &format!("independent review: real provider7/3; host outcome {outcome:?}"),
        )
        .await
        .unwrap();
    assert_eq!(settled.state, "settled");
    assert_eq!(settled.settled, COST);
}

async fn activation_fixture(lab: &Lab) -> (ModelGateway, Vec<String>, BTreeSet<String>) {
    let config = lab.config([Some(true), Some(true)], [0, 1]);
    let planned = ordered_nonces(lab.fixture.key_id, 5);
    for (id, nonce) in &planned {
        seed_candidate(lab, &config, id, nonce).await;
    }
    let gateway = gateway(&lab.runtime.pool, &config);
    expire(&gateway, lab).await;
    for (index, (id, _)) in planned.iter().enumerate() {
        host_settle(
            &gateway,
            id,
            if index < 3 {
                Outcome::Cancelled
            } else {
                Outcome::Completed
            },
        )
        .await;
    }
    let prefix: Vec<_> = planned[..3].iter().map(|(id, _)| id.clone()).collect();
    let tail: BTreeSet<_> = planned[3..].iter().map(|(id, _)| id.clone()).collect();
    for id in &prefix {
        let can_activate: bool = sqlx::query_scalar("SELECT core_model.activate_continuation($1)")
            .bind(id)
            .fetch_one(&lab.runtime.pool)
            .await
            .unwrap();
        assert!(!can_activate);
    }
    assert_eq!(eligible(&lab.fixture.pool).await, tail);
    assert!(active(&lab.fixture.pool).await.is_empty());
    let balance = lab
        .fixture
        .ledger
        .balance(lab.fixture.account)
        .await
        .unwrap();
    assert_eq!(
        (balance.available_units, balance.reserved_units),
        (FUNDS - COST * 5, 0)
    );
    (gateway, prefix, tail)
}

// This primitive uses only the published SQL function and its request_id
// column. It permits either an eligible-only scan or a scanned-row result;
// neither may report EOF while an eligible suffix remains. It does not shadow
// the implementation's SELECT, synthesize a cursor from a guessed total, or
// assume that returned rows were successfully activated.
async fn activation_sql_page(
    lab: &Lab,
    limit: i32,
    after: Option<&str>,
    tag: &str,
) -> Option<String> {
    let before = active(&lab.fixture.pool).await;
    let rows = sqlx::query("SELECT * FROM core_model.activate_continuations_after($1,$2)")
        .bind(limit)
        .bind(after)
        .fetch_all(&lab.runtime.pool)
        .await
        .unwrap();
    let returned: Vec<String> = rows
        .iter()
        .map(|row| row.try_get("request_id").unwrap())
        .collect();
    let after_active = active(&lab.fixture.pool).await;
    let delta: Vec<_> = after_active.difference(&before).cloned().collect();
    let remaining = eligible(&lab.fixture.pool).await;
    evidence(
        tag,
        json!({"limit":limit,"after":after,"returned":returned,"newly_active":delta,"eligible_remaining":remaining}),
    );
    assert!(returned.len() <= limit as usize, "SQL page exceeds limit");
    assert!(
        delta.len() <= limit as usize,
        "activation exceeded the page budget"
    );
    assert!(returned.windows(2).all(|pair| pair[0] < pair[1]));
    assert!(
        returned
            .iter()
            .all(|id| after.is_none_or(|after| id.as_str() > after))
    );
    let next = if returned.len() == limit as usize {
        returned.last().cloned()
    } else {
        None
    };
    if next.is_none() {
        let last = returned.last().map(String::as_str).or(after);
        assert!(
            !remaining
                .iter()
                .any(|id| last.is_none_or(|last| id.as_str() > last)),
            "SQL EOF truncated eligible tail behind unactivatable prefix"
        );
    }
    next
}
async fn sql_pages(pool: PgPool, limit: i32) {
    run(pool, move |lab| {
        Box::pin(async move {
            let (_gateway, prefix, tail) = activation_fixture(lab).await;
            let before = money(&lab.fixture.pool).await;
            let calls = lab.calls();
            let mut cursor = None;
            let mut ended = false;
            for round in 0..20 {
                let next = activation_sql_page(
                    lab,
                    limit,
                    cursor.as_deref(),
                    &format!("sql-page-{limit}-{round}"),
                )
                .await;
                if next.is_none() {
                    ended = true;
                    break;
                }
                assert_ne!(
                    next, cursor,
                    "cursor must move even when activation count is zero"
                );
                cursor = next;
            }
            assert!(ended, "activation traversal never reached EOF");
            assert_eq!(active(&lab.fixture.pool).await, tail);
            assert_eq!(
                cancelled(&lab.fixture.pool).await,
                prefix.into_iter().collect()
            );
            assert_eq!(money(&lab.fixture.pool).await, before);
            assert_eq!(lab.calls(), calls);
            assert_eq!(lab.counts(), 0);
        })
    })
    .await;
}
macro_rules! sql_page_case {
    ($name:ident,$limit:literal) => {
        #[sqlx::test(migrations = false)]
        async fn $name(pool: PgPool) {
            sql_pages(pool, $limit).await;
        }
    };
}
sql_page_case!(responses_continuity_review_independent_sql_limit1, 1);
sql_page_case!(responses_continuity_review_independent_sql_limit2, 2);

struct Sources {
    gateway: ModelGateway,
    completion: BTreeSet<String>,
    activation: BTreeSet<String>,
    cancelled: BTreeSet<String>,
}
async fn sources_fixture(lab: &Lab, completion_ahead: bool) -> Sources {
    let config = lab.config([Some(true), Some(true)], [0, 1]);
    let planned = ordered_nonces(lab.fixture.key_id, 8);
    // Actual terminal checkpoints without candidates are on the opposite side
    // of the activation frontier. A single shared max/min cursor must not skip
    // either chain. No malformed rows or arbitrary final counters are inserted.
    let completion_indexes: Vec<usize> = if completion_ahead {
        vec![5, 6, 7]
    } else {
        vec![0, 1, 2]
    };
    for (index, (id, nonce)) in planned.iter().enumerate() {
        if completion_indexes.contains(&index) {
            seed_completion_only(lab, &config, id, nonce).await;
        } else {
            seed_candidate(lab, &config, id, nonce).await;
        }
    }
    let gateway = gateway(&lab.runtime.pool, &config);
    expire(&gateway, lab).await;
    let mut cancelled_ids = BTreeSet::new();
    let mut ready = BTreeSet::new();
    let mut completion = BTreeSet::new();
    for (index, (id, _)) in planned.iter().enumerate() {
        if completion_indexes.contains(&index) {
            completion.insert(id.clone());
            continue;
        }
        let cancel = cancelled_ids.len() < 2;
        host_settle(
            &gateway,
            id,
            if cancel {
                Outcome::Cancelled
            } else {
                Outcome::Completed
            },
        )
        .await;
        if cancel {
            cancelled_ids.insert(id.clone());
        } else {
            ready.insert(id.clone());
        }
    }
    assert_eq!(eligible(&lab.fixture.pool).await, ready);
    assert!(active(&lab.fixture.pool).await.is_empty());
    Sources {
        gateway,
        completion,
        activation: ready,
        cancelled: cancelled_ids,
    }
}
async fn all_settled(lab: &Lab, source: &Sources) -> bool {
    for id in &source.completion {
        if charge(&lab.fixture.pool, id).await.0 != "settled" {
            return false;
        }
    }
    active(&lab.fixture.pool).await == source.activation
}
async fn finish_sources(lab: &Lab, source: &Sources, calls: usize) {
    assert!(
        all_settled(lab, source).await,
        "a recovery source was skipped/starved"
    );
    for id in &source.completion {
        assert_eq!(
            charge(&lab.fixture.pool, id).await,
            ("settled".into(), HOLD, COST)
        );
    }
    assert_eq!(cancelled(&lab.fixture.pool).await, source.cancelled);
    let balance = lab
        .fixture
        .ledger
        .balance(lab.fixture.account)
        .await
        .unwrap();
    assert_eq!(
        (balance.available_units, balance.reserved_units),
        (FUNDS - COST * 8, 0)
    );
    let captures: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM core_billing.ledger_journals WHERE kind='capture'",
    )
    .fetch_one(&lab.fixture.pool)
    .await
    .unwrap();
    assert_eq!(captures, 8);
    assert_eq!(lab.calls(), calls, "recovery must not generate again");
    assert_eq!(lab.counts(), 0);
}
async fn separate_sources(pool: PgPool, limit: i32) {
    run(pool, move |lab| {
        Box::pin(async move {
            let source = sources_fixture(lab, false).await;
            let calls = lab.calls();
            let mut activation_cursor = None;
            let mut completion_cursor = None;
            for round in 0..30 {
                activation_cursor = activation_sql_page(
                    lab,
                    limit,
                    activation_cursor.as_deref(),
                    &format!("dual-activation-{limit}-{round}"),
                )
                .await;
                let page = source.gateway
                    .reconcile_completed_streams_page(limit, completion_cursor.as_deref())
                    .await.unwrap();
                assert_eq!(page.activated,0,"legacy settlement pages must not inflate their financial count with activations");
                let count=page.settled;
                let next=page.next;
                evidence(
                    &format!("dual-completion-{limit}-{round}"),
                    json!({"limit":limit,"after":completion_cursor,
                "completed":count,"next":next,"activation_cursor":activation_cursor}),
                );
                assert!(count <= limit as usize);
                if next.is_some() {
                    assert_ne!(next, completion_cursor);
                }
                completion_cursor = next;
                if all_settled(lab, &source).await {
                    break;
                }
            }
            finish_sources(lab, &source, calls).await;
        })
    })
    .await;
}
async fn combined_sources(pool: PgPool, limit: i32) {
    run(pool,move |lab|Box::pin(async move {
        let source=sources_fixture(lab,true).await;let calls=lab.calls();let mut cursor=None;let mut eof=false;
        // Both source frontiers belong to the public continuation cursor.
        // None means this static fixture traversal is complete.
        for round in 0..30 {
            let before=active(&lab.fixture.pool).await;
            let page=source.gateway.reconcile_responses_continuity_page(limit,cursor.as_ref()).await.unwrap();
            let count=page.activated;
            assert!(page.settled<=limit as usize);
            let next=page.next;
            let after=active(&lab.fixture.pool).await;let delta=after.difference(&before).count();
            evidence(&format!("combined-{limit}-{round}"),json!({"after":cursor,"next":next,
                "reported_activated":count,"actual_activated":delta,"eligible":eligible(&lab.fixture.pool).await}));
            assert_eq!(count,delta,"scanned or failed rows cannot be counted as activations");
            assert!(delta<=limit as usize);
            if next.is_none() {eof=true;assert!(all_settled(lab,&source).await,"combined EOF lost a recovery source frontier");break;}
            assert_ne!(next,cursor,"combined cursor stalled");cursor=next;
        }
        assert!(eof);finish_sources(lab,&source,calls).await;
    })).await;
}
async fn background_sources(pool: PgPool, limit: i32) {
    run(pool, move |lab| {
        Box::pin(async move {
            let source = sources_fixture(lab, true).await;
            let calls = lab.calls();
            let mut progress = Vec::new();
            // Same gateway instance and method as the actual main scheduler. Never
            // feed hand-maintained cursors or initialize activation with None here.
            for round in 0..30 {
                let completed = source
                    .gateway
                    .reconcile_completed_streams(limit)
                    .await
                    .unwrap();
                let current = active(&lab.fixture.pool).await;
                progress.push(json!({"round":round,"completed":completed,"active":current}));
                if all_settled(lab, &source).await {
                    break;
                }
            }
            evidence(
                &format!("background-{limit}"),
                json!({"rounds":progress,"ineligible_prefix":source.cancelled,
            "eligible_tail":source.activation,"completion_source":source.completion}),
            );
            finish_sources(lab, &source, calls).await;
            let before = money(&lab.fixture.pool).await;
            for _ in 0..4 {
                source
                    .gateway
                    .reconcile_completed_streams(limit)
                    .await
                    .unwrap();
            }
            assert_eq!(
                money(&lab.fixture.pool).await,
                before,
                "later scans cannot charge twice"
            );
            assert_eq!(lab.calls(), calls);
        })
    })
    .await;
}
macro_rules! recovery_case {
    ($name:ident,$function:ident,$limit:literal) => {
        #[sqlx::test(migrations = false)]
        async fn $name(pool: PgPool) {
            $function(pool, $limit).await;
        }
    };
}
recovery_case!(
    responses_continuity_review_independent_dual_cursor_limit1,
    separate_sources,
    1
);
recovery_case!(
    responses_continuity_review_independent_dual_cursor_limit2,
    separate_sources,
    2
);
recovery_case!(
    responses_continuity_review_independent_combined_cursor_limit1,
    combined_sources,
    1
);
recovery_case!(
    responses_continuity_review_independent_combined_cursor_limit2,
    combined_sources,
    2
);
recovery_case!(
    responses_continuity_review_independent_background_limit1,
    background_sources,
    1
);
recovery_case!(
    responses_continuity_review_independent_background_limit2,
    background_sources,
    2
);

async fn controls(
    pool: PgPool,
    label: &'static str,
    caps: [Option<bool>; 2],
    order: [usize; 2],
    store: Option<bool>,
) {
    controls_profile(pool, label, caps, order, store, CacheProfile::Basic).await;
}
#[derive(Clone, Copy, PartialEq, Eq)]
enum CacheProfile {
    Basic,
    MissingTariff,
    ReviewedGeneric,
    ReviewedNative,
}
async fn controls_profile(
    pool: PgPool,
    label: &'static str,
    caps: [Option<bool>; 2],
    order: [usize; 2],
    store: Option<bool>,
    profile: CacheProfile,
) {
    run(pool,move |lab|Box::pin(async move {
        let mut config=lab.config(caps,order);
        config.reviewed_cache_tariff=matches!(profile,CacheProfile::ReviewedGeneric|CacheProfile::ReviewedNative);
        for server in &lab.servers {
            *server.supplier.reviewed_cache_receipt.lock().unwrap()=config.reviewed_cache_tariff;
        }
        let core=Core::start(config,None).await;
        let mut body=json!({"model":PUBLIC,"input":"ordinary reviewed request","max_output_tokens":50,
            "temperature":0.375,"top_p":0.625,"parallel_tool_calls":false});
        if matches!(profile,CacheProfile::Basic|CacheProfile::ReviewedGeneric|CacheProfile::ReviewedNative) {
            body["input"]=json!([{"role":"user","content":[{"type":"input_text","text":"ordinary reviewed request"}]}]);
        }
        if profile!=CacheProfile::Basic {
            body["prompt_cache_options"]=json!({"mode":"implicit","ttl":"12h"});
        }
        if let Some(store)=store {body["store"]=store.into();}
        let before_money=money(&lab.fixture.pool).await;
        let response=post(&core.base,&lab.fixture.key,label,&body).await.unwrap();
        let status=response.status();let request_id=response.headers().get("x-request-id").map(|id|id.to_str().unwrap().to_owned());
        let bytes=response.bytes().await.unwrap();let result:Value=serde_json::from_slice(&bytes).unwrap();
        let seen:Vec<Vec<Value>>=lab.servers.iter().map(|s|s.supplier.seen.lock().unwrap().clone()).collect();
        evidence(label,json!({"request":body,"status":status.as_u16(),"response":result,"supplier_requests":seen}));
        assert_eq!(lab.unexpected(),0,"unsupported supplier endpoint was contacted");
        if profile==CacheProfile::MissingTariff {
            let expected_message="unsupported_feature: cache lifetime controls require a reviewed twelve-hour tariff";
            assert_eq!(status,StatusCode::BAD_REQUEST);
            assert_eq!(result["error"]["code"],"unsupported_feature");
            assert_eq!(result["error"]["type"],"unsupported_feature");
            assert_eq!(result["error"]["message"],expected_message);
            assert_eq!(lab.calls(),0);assert_eq!(lab.counts(),0);
            assert!(ids(&lab.fixture.pool).await.is_empty(),"rejected tariff request reserved money");
            let balance=lab.fixture.ledger.balance(lab.fixture.account).await.unwrap();
            assert_eq!((balance.available_units,balance.reserved_units),(FUNDS,0));
            assert_eq!(money(&lab.fixture.pool).await,before_money,"tariff rejection changed ledger state");
            evidence(&format!("{label}-capability"),json!({"guard":"PASS","native_reviewed_12h":
                "NOT_REQUESTED",
                "supplier_calls":0,"reservations":0,"available":balance.available_units,"reserved":balance.reserved_units}));
            return;
        }
        if !status.is_success() {
            // A reviewed later target must be selected before dispatch. Only
            // a route with no reviewed target may refuse this valid request.
            assert!(store==Some(true) && caps.iter().all(Option::is_none) && status.is_client_error(),
                "ordinary legal controls regressed to {status}");
            assert_eq!(lab.calls(),0);assert!(ids(&lab.fixture.pool).await.is_empty());
            let balance=lab.fixture.ledger.balance(lab.fixture.account).await.unwrap();
            assert_eq!((balance.available_units,balance.reserved_units),(FUNDS,0));return;
        }
        assert_eq!(status,StatusCode::OK);assert_eq!(lab.calls(),1);assert_eq!(lab.counts(),0);
        let target=seen.iter().position(|requests|!requests.is_empty()).unwrap();
        if store!=Some(true) {assert_eq!(target,order[0],"adding an unused target guarantee cannot reroute an ordinary request");}
        if store==Some(true) {assert!(caps[target].is_some(),"unreviewed target accepted persistent store");}
        let wire=&seen[target][0];
        for field in ["temperature","top_p","parallel_tool_calls"] {
            assert_eq!(wire.get(field),body.get(field),"selected supplier silently lost/changed {field}");
        }
        if matches!(profile,CacheProfile::ReviewedGeneric|CacheProfile::ReviewedNative) {
            assert_eq!(body["prompt_cache_options"],json!({"mode":"implicit","ttl":"12h"}));
            assert_eq!(wire.get("prompt_cache_options"),Some(&body["prompt_cache_options"]),"reviewed12h control lost on actual supplier wire");
            assert_eq!(target,order[0]);
        }
        assert_eq!(wire["input"],body["input"]);assert_eq!(wire["max_output_tokens"],50);
        if caps[target].is_some() {assert_eq!(wire.get("store"),body.get("store"),"admitted native store tri-state changed");}
        else {assert_eq!(wire["store"],false,"old stateless conversion must remain stateless");}
        let id=request_id.unwrap();assert_eq!(charge(&lab.fixture.pool,&id).await,("settled".into(),HOLD,COST));
        if matches!(profile,CacheProfile::ReviewedGeneric|CacheProfile::ReviewedNative) {
            let revision:Value=sqlx::query_scalar("SELECT price->'tiered'->'tariff'->'revision' FROM core_charging.requests WHERE id=$1")
                .bind(&id).fetch_one(&lab.fixture.pool).await.unwrap();
            assert_eq!(revision,json!(4),"12h request did not freeze reviewed revision4 price");
        }
        let mappings:i64=sqlx::query_scalar("SELECT count(*) FROM core_model.responses_continuations WHERE request_id=$1 AND active")
            .bind(&id).fetch_one(&lab.fixture.pool).await.unwrap();
        let expected=caps[target].is_some() && store.unwrap_or(caps[target].unwrap_or(false));
        assert_eq!(mappings,i64::from(expected));
        if caps[target].is_some() {
            let checkpoint:(Vec<u8>,i64)=sqlx::query_as("SELECT packet_sha256,packet_bytes FROM core_model.output_checkpoints WHERE request_id=$1 AND terminal_observed")
                .bind(&id).fetch_one(&lab.fixture.pool).await.unwrap();
            assert_eq!(checkpoint.0,Sha256::digest(&bytes).to_vec());assert_eq!(checkpoint.1,bytes.len() as i64);
        }
        let balance=lab.fixture.ledger.balance(lab.fixture.account).await.unwrap();
        assert_eq!((balance.available_units,balance.reserved_units),(FUNDS-COST,0));
    })).await;
}
macro_rules! control_case {
    ($name:ident,$caps:expr,$order:expr,$store:expr) => {
        #[sqlx::test(migrations = false)]
        async fn $name(pool: PgPool) {
            controls(pool, stringify!($name), $caps, $order, $store).await;
        }
    };
}
control_case!(
    responses_continuity_review_independent_plain_omitted,
    [None, None],
    [0, 1],
    None
);
control_case!(
    responses_continuity_review_independent_plain_false,
    [None, None],
    [0, 1],
    Some(false)
);
control_case!(
    responses_continuity_review_independent_plain_true,
    [None, None],
    [0, 1],
    Some(true)
);
control_case!(
    responses_continuity_review_independent_native_default_true_omitted,
    [Some(true), Some(true)],
    [0, 1],
    None
);
control_case!(
    responses_continuity_review_independent_native_default_true_false,
    [Some(true), Some(true)],
    [0, 1],
    Some(false)
);
control_case!(
    responses_continuity_review_independent_native_default_true_true,
    [Some(true), Some(true)],
    [0, 1],
    Some(true)
);
control_case!(
    responses_continuity_review_independent_native_default_false_omitted,
    [Some(false), Some(false)],
    [0, 1],
    None
);
control_case!(
    responses_continuity_review_independent_native_default_false_false,
    [Some(false), Some(false)],
    [0, 1],
    Some(false)
);
control_case!(
    responses_continuity_review_independent_native_default_false_true,
    [Some(false), Some(false)],
    [0, 1],
    Some(true)
);
control_case!(
    responses_continuity_review_independent_plain_primary_native_backup_omitted,
    [None, Some(true)],
    [0, 1],
    None
);
control_case!(
    responses_continuity_review_independent_plain_primary_native_backup_false,
    [None, Some(true)],
    [0, 1],
    Some(false)
);
control_case!(
    responses_continuity_review_independent_plain_primary_native_backup_true,
    [None, Some(true)],
    [0, 1],
    Some(true)
);
control_case!(
    responses_continuity_review_independent_native_primary_plain_backup_omitted,
    [Some(true), None],
    [0, 1],
    None
);
control_case!(
    responses_continuity_review_independent_native_primary_plain_backup_false,
    [Some(true), None],
    [0, 1],
    Some(false)
);
control_case!(
    responses_continuity_review_independent_native_primary_plain_backup_true,
    [Some(true), None],
    [0, 1],
    Some(true)
);
control_case!(
    responses_continuity_review_independent_secondary_plain_omitted,
    [Some(true), None],
    [1, 0],
    None
);
control_case!(
    responses_continuity_review_independent_secondary_plain_false,
    [Some(true), None],
    [1, 0],
    Some(false)
);
control_case!(
    responses_continuity_review_independent_secondary_plain_true,
    [Some(true), None],
    [1, 0],
    Some(true)
);

macro_rules! cache_control_case {
    ($name:ident,$caps:expr,$store:expr,$profile:ident) => {
        #[sqlx::test(migrations = false)]
        async fn $name(pool: PgPool) {
            controls_profile(
                pool,
                stringify!($name),
                $caps,
                [0, 1],
                $store,
                CacheProfile::$profile,
            )
            .await;
        }
    };
}
cache_control_case!(
    responses_continuity_review_independent_cache_missing_generic_omitted,
    [None, None],
    None,
    MissingTariff
);
cache_control_case!(
    responses_continuity_review_independent_cache_missing_generic_false,
    [None, None],
    Some(false),
    MissingTariff
);
cache_control_case!(
    responses_continuity_review_independent_cache_missing_native_omitted,
    [Some(true), Some(true)],
    None,
    MissingTariff
);
cache_control_case!(
    responses_continuity_review_independent_cache_missing_native_false,
    [Some(true), Some(true)],
    Some(false),
    MissingTariff
);
cache_control_case!(
    responses_continuity_review_independent_cache_reviewed_generic_wire_funds,
    [None, None],
    Some(false),
    ReviewedGeneric
);
cache_control_case!(
    responses_continuity_review_independent_cache_native_tiered_wire_funds,
    [Some(true), Some(true)],
    Some(true),
    ReviewedNative
);

#[sqlx::test(migrations = false)]
async fn responses_continuity_review_independent_public_runtime_execute_only(pool: PgPool) {
    run(pool,|lab|Box::pin(async move {
        let functions=["core_model.resolve_continuation(text,text)",
            "core_model.record_continuation(text,bytea,text,text,text,bytea,bigint)",
            "core_model.activate_continuation(text)","core_model.activate_continuations_after(integer,text)"];
        for function in functions {
            let allowed:bool=sqlx::query_scalar("SELECT has_function_privilege($1,$2,'EXECUTE')")
                .bind(&lab.runtime.name).bind(function).fetch_one(&lab.fixture.pool).await.unwrap();
            assert!(allowed,"PUBLIC tests/common/runtime.rs omitted69 EXECUTE: {function}");
            let public:bool=sqlx::query_scalar("SELECT EXISTS(SELECT FROM pg_proc p CROSS JOIN LATERAL aclexplode(coalesce(p.proacl,acldefault('f',p.proowner))) a WHERE p.oid=$1::regprocedure AND a.grantee=0 AND a.privilege_type='EXECUTE')")
                .bind(function).fetch_one(&lab.fixture.pool).await.unwrap();assert!(!public);
        }
        for privilege in ["SELECT","INSERT","UPDATE","DELETE","TRUNCATE","REFERENCES","TRIGGER"] {
            let allowed:bool=sqlx::query_scalar("SELECT has_table_privilege($1,'core_model.responses_continuations',$2)")
                .bind(&lab.runtime.name).bind(privilege).fetch_one(&lab.fixture.pool).await.unwrap();assert!(!allowed);
        }
        let before=money(&lab.fixture.pool).await;
        for statement in ["SELECT * FROM core_model.responses_continuations LIMIT 0",
            "INSERT INTO core_model.responses_continuations DEFAULT VALUES",
            "UPDATE core_model.responses_continuations SET active=active WHERE false",
            "DELETE FROM core_model.responses_continuations WHERE false",
            "TRUNCATE core_model.responses_continuations"] {
            let error=sqlx::query(statement).execute(&lab.runtime.pool).await.expect_err("runtime CRUD was possible");
            assert_eq!(error.as_database_error().and_then(|e|e.code()).as_deref(),Some("42501"));
        }
        let missing:String=format!("resp_lmm_{}","0".repeat(64));
        let mut tx=lab.runtime.pool.begin().await.unwrap();
        sqlx::query("SET TRANSACTION ISOLATION LEVEL SERIALIZABLE").execute(&mut *tx).await.unwrap();
        let absent:Option<Value>=sqlx::query_scalar("SELECT core_model.resolve_continuation($1,$2)")
            .bind(missing).bind(&lab.fixture.key).fetch_one(&mut *tx).await.unwrap();assert!(absent.is_none());tx.commit().await.unwrap();
        let activated:bool=sqlx::query_scalar("SELECT core_model.activate_continuation('review-absent')")
            .fetch_one(&lab.runtime.pool).await.unwrap();assert!(!activated);
        let page:Vec<sqlx::postgres::PgRow>=sqlx::query("SELECT * FROM core_model.activate_continuations_after(1,NULL)")
            .fetch_all(&lab.runtime.pool).await.unwrap();assert!(page.is_empty());
        let mut tx=lab.runtime.pool.begin().await.unwrap();
        sqlx::query("SET TRANSACTION ISOLATION LEVEL SERIALIZABLE").execute(&mut *tx).await.unwrap();
        let error=sqlx::query("SELECT core_model.record_continuation('review-absent',$1,'provider-id',NULL,$2,$3,1)")
            .bind(vec![0u8;32]).bind(PUBLIC).bind(vec![0u8;32]).execute(&mut *tx).await.expect_err("record created a candidate without its charging request");
        // EXECUTE was independently verified above. Security denial42501 is
        // valid here; a missing row may instead be P0002/23514. Neither permits
        // a forged record, and neither is confused with a connection failure.
        assert!(matches!(error.as_database_error().and_then(|e|e.code()).as_deref(),Some("42501"|"P0002"|"23514")));
        tx.rollback().await.unwrap();
        assert!(ids(&lab.fixture.pool).await.is_empty());assert_eq!(money(&lab.fixture.pool).await,before);
        evidence("public-runtime",json!({"fixture":"services/core/tests/common/runtime.rs","all_four_execute":true,
            "direct_crud_sqlstate":"42501","public_execute":false,"record_missing_request_denied":true}));
    })).await;
}

// Legacy seed must use actual Core admission/reserve/dispatch and protected
// checkpoint_output with its worker, plus REAL loopback supplier bytes. Frozen
// source is cc52 ordinary legacy_precharge, personal wallet, rates3/5, usage7/3.
// false means historical checkpoint-only evidence (no observation/final/no-
// acceptance union source); true adds the NORMAL corroborated legacy terminal
// through existing Core persistence, under the SAME request ID. End in
// reconcile by Core API, never UPDATE immutable proof/outcome/checkpoint rows.
// Return the actual supplier terminal JSON bytes, not a success boolean.
//
// OAuth factory must issue genuine lmm_at_ grants for the existing fixture's
// personal wallet: two invoke-authorized client/grant identities and one VALID
// token without invoke scope. Use the real restricted-runtime issuer and
// normal grants; no fake tokens, owner-pool runtime auth, or permissive Gate.
// Route uses reviewed continuity default_store=true so omitted store triggers
// the exact regression, while ordinary OAuth remains stateless.
//
// Retry factory configures the EXISTING API key (including cross_group_retry)
// and authorized auto groups through normal setup. Unsupported first supplier
// is index0; reviewed native supplier index1. Same-group name is `allowed`;
// two-group order is `review-first`,`review-second`. Rates/bounds stay3/5,
// 800/50. first-nonnative means Chat, not a fake rejected Responses provider.

fn assert_immutable_plans(traces: &[PlanTrace]) {
    assert!(
        !traces.is_empty(),
        "real request must expose its immutable plan boundary"
    );
    for trace in traces {
        let keys: BTreeSet<_> = trace
            .after
            .iter()
            .map(|attempt| attempt.route_key.as_str())
            .collect();
        assert_eq!(keys.len(), trace.after.len());
        let retained: Vec<_> = trace
            .before
            .iter()
            .filter(|attempt| keys.contains(attempt.route_key.as_str()))
            .cloned()
            .collect();
        assert_eq!(
            trace.after, retained,
            "capability filter introduced/reordered attempts or changed a frozen binding"
        );
        for attempt in trace.before.iter().chain(&trace.after) {
            let old_bytes = format!(
                "r{}-t{}-u{}",
                attempt.route_id, attempt.target_id, attempt.upstream_id
            );
            assert_eq!(
                attempt.route_key.as_bytes(),
                old_bytes.as_bytes(),
                "AttemptKey changed legacy route-key bytes"
            );
            assert_eq!(attempt.binding["route_id"], old_bytes);
            assert_eq!(attempt.binding["target_id"], attempt.target_id);
            assert_eq!(attempt.binding["upstream_id"], attempt.upstream_id);
        }
        if let Some(selected) = &trace.selected {
            assert!(
                trace.after.contains(selected),
                "dispatch used a different attempt than the filtered immutable plan"
            );
        }
    }
}

struct Hosted {
    base: String,
    task: tokio::task::JoinHandle<()>,
}
impl Drop for Hosted {
    fn drop(&mut self) {
        self.task.abort();
    }
}
async fn host(gateway: ModelGateway) -> Hosted {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let base = format!("http://{}", listener.local_addr().unwrap());
    let task = tokio::spawn(async move {
        axum::serve(
            listener,
            gateway
                .router()
                .into_make_service_with_connect_info::<SocketAddr>(),
        )
        .await
        .unwrap();
    });
    Hosted { base, task }
}
fn bases(lab: &Lab) -> Vec<String> {
    lab.servers
        .iter()
        .map(|s| s.supplier.base.clone())
        .collect()
}
async fn legacy_seed(lab: &Lab, id: &str, corroborated: bool) -> i64 {
    let bytes = super::responses_continuity_review_support::seed_legacy_checkpoint(
        fixture_gateway,
        &lab.fixture,
        &lab.runtime.pool,
        id,
        &lab.servers[0].supplier.base,
        corroborated,
    )
    .await;
    let terminal: Value = serde_json::from_slice(&bytes).unwrap();
    assert_eq!(terminal["usage"]["input_tokens"], 7);
    assert_eq!(terminal["usage"]["output_tokens"], 3);
    let (state, reserved, settled) = charge(&lab.fixture.pool, id).await;
    assert_eq!(state, "reconcile");
    assert!(reserved > 0);
    assert_eq!(settled, 0);
    let frozen:bool=sqlx::query_scalar("SELECT r.source_subscription_id IS NULL AND e.detail->'legacy_precharge' IS NOT NULL AND e.detail->'legacy_precharge'<>'null'::jsonb FROM core_charging.requests r JOIN core_charging.usage_events e ON e.request_id=r.id AND e.event_id='reservation-contract:v1' WHERE r.id=$1")
        .bind(id).fetch_one(&lab.fixture.pool).await.unwrap();
    assert!(
        frozen,
        "must hit the real legacy-precharge post-materialize exclusion"
    );
    let proof:(Vec<u8>,i64)=sqlx::query_as("SELECT packet_sha256,packet_bytes FROM core_model.output_checkpoints WHERE request_id=$1 AND terminal_observed")
        .bind(id).fetch_one(&lab.fixture.pool).await.unwrap();
    assert_eq!(proof.0, Sha256::digest(&bytes).to_vec());
    assert_eq!(proof.1, bytes.len() as i64);
    let raw: Vec<String> =
        sqlx::query_scalar("SELECT request_id FROM core_model.completed_streams_after(100,NULL)")
            .fetch_all(&lab.runtime.pool)
            .await
            .unwrap();
    assert!(
        raw.iter().any(|raw| raw == id),
        "prefix must really enter the inner materialized page"
    );
    let supplementary:i64=sqlx::query_scalar("SELECT count(*) FROM core_charging.usage_events WHERE request_id=$1 AND event_id IN ('provider-observation:terminal','model-provider-terminal:v1','model-no-acceptance:v1','usage:provider-final')")
        .bind(id).fetch_one(&lab.fixture.pool).await.unwrap();
    if corroborated {
        assert!(
            supplementary >= 2,
            "must also qualify for the legitimate same-ID UNION evidence branch"
        );
    } else {
        assert_eq!(
            supplementary, 0,
            "an accidentally recoverable prefix does not reproduce post-filter starvation"
        );
    }
    reserved
}
#[sqlx::test(migrations = false)]
async fn responses_continuity_review_independent_completed_legacy_prefix_tick_limit1(pool: PgPool) {
    run(pool,|lab|Box::pin(async move {
        let planned=ordered_nonces(lab.fixture.key_id,4);let mut retained=0;
        for (id,_) in &planned[..3] {retained+=legacy_seed(lab,id,false).await;}
        let config=lab.config([Some(true),Some(true)],[0,1]);
        let (tail,nonce)=&planned[3];seed_completion_only(lab,&config,tail,nonce).await;
        let gateway=gateway(&lab.runtime.pool,&config);
        tokio::time::sleep(Duration::from_secs(LEASE+1)).await;gateway.recover_expired(100).await.unwrap();
        let calls=lab.calls();let mut trace=Vec::new();
        for round in 0..16 {
            let recovered=gateway.reconcile_completed_streams(1).await.unwrap();
            trace.push(json!({"round":round,"financial_count":recovered,"tail":charge(&lab.fixture.pool,tail).await}));
            assert!(recovered<=1);
            if charge(&lab.fixture.pool,tail).await.0=="settled" {break;}
        }
        evidence("completed-legacy-prefix",json!({"limit":1,"prefix":planned[..3],"trace":trace,"retained":retained}));
        assert_eq!(charge(&lab.fixture.pool,tail).await,("settled".into(),HOLD,COST),"legacy rows were materialized then filtered and stranded the real completed tail");
        for (id,_) in &planned[..3] {let row=charge(&lab.fixture.pool,id).await;assert_eq!(row.0,"reconcile");assert_eq!(row.2,0);}
        let balance=lab.fixture.ledger.balance(lab.fixture.account).await.unwrap();
        assert_eq!((balance.available_units,balance.reserved_units),(FUNDS-COST-retained,retained));
        assert_eq!(lab.calls(),calls);assert_eq!(lab.counts(),0);assert_eq!(lab.unexpected(),0);
        let before=money(&lab.fixture.pool).await;
        for _ in 0..5 {assert_eq!(gateway.reconcile_completed_streams(1).await.unwrap(),0,"skip markers are not financial recoveries");}
        assert_eq!(money(&lab.fixture.pool).await,before);
    })).await;
}
#[sqlx::test(migrations = false)]
async fn responses_continuity_review_independent_completed_skip_same_id_corroboration(
    pool: PgPool,
) {
    run(pool,|lab|Box::pin(async move {
        let (id,_)=ordered_nonces(lab.fixture.key_id,1).pop().unwrap();
        legacy_seed(lab,&id,true).await;
        let config=lab.config([Some(true),Some(true)],[0,1]);let gateway=gateway(&lab.runtime.pool,&config);let calls=lab.calls();
        let page=gateway.reconcile_completed_streams_page(1,None).await.unwrap();
        assert_eq!(page.activated,0);
        let count=page.settled;let next=page.next;
        evidence("completed-same-id",json!({"id":id,"count":count,"next":next,"charge":charge(&lab.fixture.pool,&id).await}));
        assert_eq!(count,1,"same-ID skip marker masked valid corroborated legacy UNION branch");
        let result=charge(&lab.fixture.pool,&id).await;assert_eq!(result.0,"settled");assert_eq!(result.2,COST);
        let balance=lab.fixture.ledger.balance(lab.fixture.account).await.unwrap();
        assert_eq!((balance.available_units,balance.reserved_units),(FUNDS-COST,0));
        let before=money(&lab.fixture.pool).await;
        for _ in 0..4 {assert_eq!(gateway.reconcile_completed_streams(1).await.unwrap(),0);}
        assert_eq!(money(&lab.fixture.pool).await,before);assert_eq!(lab.calls(),calls);assert_eq!(lab.unexpected(),0);
    })).await;
}

fn oauth_id(token: &OAuthToken, nonce: &str) -> String {
    let mut hash = Sha256::new();
    hash.update(b"lmm-model-oauth-request-v1\0");
    hash.update(token.client_id.as_bytes());
    hash.update([0]);
    hash.update(token.grant_id.as_bytes());
    hash.update(nonce.as_bytes());
    crate::hex_codec::encode(hash.finalize())
}
async fn oauth_ordinary(pool: PgPool, store: Option<bool>) {
    run(pool,move |lab|Box::pin(async move {
        let OAuthFixture {gateway,first,second,denied_scope_token:_}=super::responses_continuity_review_support::oauth_fixture(fixture_gateway,
            &lab.fixture,&lab.runtime.pool,&bases(lab)).await;
        assert!(gateway.bridge.provider_policy.context_document().iter().any(|context|
            context.upstream_id==1 && context.responses_continuity.as_ref().is_some_and(|contract|contract.default_store)),
            "OAuth regression fixture must retain the admitted continuity route");
        assert!(first.token.starts_with("lmm_at_") && second.token.starts_with("lmm_at_"));
        assert_ne!((&first.client_id,&first.grant_id),(&second.client_id,&second.grant_id));
        let host=host(gateway).await;let nonce="independent-oauth-original-id";
        let mut body=json!({"model":PUBLIC,"input":"ordinary OAuth, no previous handle","max_output_tokens":50,"temperature":0.375,"top_p":0.625});
        if let Some(store)=store {body["store"]=store.into();}
        let mut observed=Vec::new();
        for token in [&first,&second] {
            let response=post(&host.base,&token.token,nonce,&body).await.unwrap();
            let status=response.status();let id=response.headers().get("x-request-id").map(|v|v.to_str().unwrap().to_owned());
            let result:Value=response.json().await.unwrap();
            evidence(&format!("oauth-{:?}-{}",store,observed.len()),json!({"status":status.as_u16(),"id":id,"body":result}));
            assert_eq!(status,StatusCode::OK,"ordinary scoped OAuth regressed into native-only403");
            let id=id.unwrap();assert_eq!(id,oauth_id(token,nonce),"OAuth client/grant request-ID domain changed");
            let stored:(Option<i64>,Value,i64)=sqlx::query_as("SELECT key_id,oauth_source,payer_account_id FROM core_charging.requests WHERE id=$1")
                .bind(&id).fetch_one(&lab.fixture.pool).await.unwrap();
            assert!(stored.0.is_none());assert_eq!(stored.1["client_id"],token.client_id);assert_eq!(stored.1["grant_id"],token.grant_id);
            assert_eq!(stored.2,lab.fixture.account);assert_eq!(charge(&lab.fixture.pool,&id).await,("settled".into(),HOLD,COST));observed.push(id);
        }
        assert_ne!(observed[0],observed[1]);assert_eq!(lab.calls(),2);
        let before=money(&lab.fixture.pool).await;
        let replay=post(&host.base,&first.token,nonce,&body).await.unwrap();
        assert_eq!(replay.headers()["x-request-id"].to_str().unwrap(),observed[0]);assert_eq!(replay.status(),StatusCode::CONFLICT);
        let _=replay.bytes().await;assert_eq!(lab.calls(),2);assert_eq!(money(&lab.fixture.pool).await,before);
        for server in &lab.servers {for wire in server.supplier.seen.lock().unwrap().iter() {
            assert_eq!(wire["temperature"],body["temperature"]);assert_eq!(wire["top_p"],body["top_p"]);
            assert_eq!(wire["store"],false,"ordinary OAuth must not silently activate new persistent storage");
        }}
        let maps:i64=sqlx::query_scalar("SELECT count(*) FROM core_model.responses_continuations").fetch_one(&lab.fixture.pool).await.unwrap();assert_eq!(maps,0);
        let balance=lab.fixture.ledger.balance(lab.fixture.account).await.unwrap();assert_eq!((balance.available_units,balance.reserved_units),(FUNDS-2*COST,0));
        assert_eq!(lab.counts(),0);assert_eq!(lab.unexpected(),0);
    })).await;
}
#[sqlx::test(migrations = false)]
async fn responses_continuity_review_independent_oauth_omitted_original_id(pool: PgPool) {
    oauth_ordinary(pool, None).await;
}
#[sqlx::test(migrations = false)]
async fn responses_continuity_review_independent_oauth_false_original_id(pool: PgPool) {
    oauth_ordinary(pool, Some(false)).await;
}
#[sqlx::test(migrations = false)]
async fn responses_continuity_review_independent_oauth_scope_denied(pool: PgPool) {
    run(pool,|lab|Box::pin(async move {
        let fixture=super::responses_continuity_review_support::oauth_fixture(fixture_gateway, &lab.fixture,&lab.runtime.pool,&bases(lab)).await;
        assert!(fixture.denied_scope_token.starts_with("lmm_at_"));let host=host(fixture.gateway).await;
        for (index,store) in [None,Some(false)].into_iter().enumerate() {
            let mut body=json!({"model":PUBLIC,"input":"valid token without invoke scope","max_output_tokens":50});
            if let Some(store)=store {body["store"]=store.into();}
            let response=post(&host.base,&fixture.denied_scope_token,&format!("oauth-scope-denied-{index}"),&body).await.unwrap();
            assert_eq!(response.status(),StatusCode::FORBIDDEN,"valid OAuth token lacking invoke scope must remain403");let _=response.bytes().await;
        }
        assert!(ids(&lab.fixture.pool).await.is_empty());assert_eq!(lab.calls(),0);assert_eq!(lab.counts(),0);assert_eq!(lab.unexpected(),0);
        let balance=lab.fixture.ledger.balance(lab.fixture.account).await.unwrap();assert_eq!((balance.available_units,balance.reserved_units),(FUNDS,0));
    })).await;
}

fn assert_retry_layout(gateway: &ModelGateway, layout: RetryLayout) {
    let document = gateway.bridge.routing.catalog_document().unwrap();
    let first = document
        .routing
        .upstreams
        .iter()
        .find(|upstream| upstream.id == 1)
        .unwrap();
    let second = document
        .routing
        .upstreams
        .iter()
        .find(|upstream| upstream.id == 2)
        .unwrap();
    assert!(first.enabled && second.enabled);
    assert_eq!(second.protocol, Protocol::OpenAiResponses);
    if layout.non_native() {
        assert_ne!(first.protocol, Protocol::OpenAiResponses);
    } else {
        assert_eq!(first.protocol, Protocol::OpenAiResponses);
    }
    let contexts = gateway.bridge.provider_policy.context_document();
    assert!(
        contexts
            .iter()
            .find(|context| context.upstream_id == 1)
            .is_none_or(|context| context.responses_continuity.is_none())
    );
    assert!(
        contexts
            .iter()
            .any(|context| context.upstream_id == 2 && context.responses_continuity.is_some())
    );
    if layout.later_group() {
        for (group, upstream) in [("review-first", 1), ("review-second", 2)] {
            assert!(
                document
                    .routing
                    .routes
                    .iter()
                    .any(|route| route.model == PUBLIC
                        && route.group == group
                        && route.enabled
                        && route
                            .targets
                            .iter()
                            .any(|target| target.upstream_id == upstream && target.enabled)),
                "fixture omitted {group}"
            );
        }
    } else {
        let route = document
            .routing
            .routes
            .iter()
            .find(|route| route.model == PUBLIC && route.group == "allowed" && route.enabled)
            .unwrap();
        let first = route
            .targets
            .iter()
            .find(|target| target.upstream_id == 1 && target.enabled)
            .unwrap();
        let second = route
            .targets
            .iter()
            .find(|target| target.upstream_id == 2 && target.enabled)
            .unwrap();
        assert!(
            first.priority < second.priority,
            "fixture must retain the unsupported first target"
        );
    }
}
async fn retry_route(pool: PgPool, layout: RetryLayout, cross: bool, label: &'static str) {
    run(pool,move |lab|Box::pin(async move {
        let RetryFixture {gateway,traces}=super::responses_continuity_review_support::retry_fixture(fixture_gateway,
            &lab.fixture,&lab.runtime.pool,&bases(lab),layout,cross).await;
        assert_retry_layout(&gateway,layout);
        let actual:bool=sqlx::query_scalar("SELECT cross_group_retry FROM core_identity.api_key_settings WHERE credential_id=$1")
            .bind(lab.fixture.key_id).fetch_one(&lab.fixture.pool).await.unwrap();assert_eq!(actual,cross);
        let host=host(gateway).await;let mut body=stored_body();body["temperature"]=json!(0.375);body["top_p"]=json!(0.625);
        let response=post(&host.base,&lab.fixture.key,label,&body).await.unwrap();let status=response.status();
        let id=response.headers().get("x-request-id").map(|v|v.to_str().unwrap().to_owned());let result:Value=response.json().await.unwrap();
        let trace=traces.lock().unwrap().clone();
        assert_immutable_plans(&trace);
        evidence(label,json!({"cross_group_retry":cross,"status":status.as_u16(),"id":id,"response":result}));
        evidence(&format!("{label}-plans"),json!(trace));
        assert!(lab.servers[0].supplier.seen.lock().unwrap().is_empty(),"unsupported first supplier was called");
        assert_eq!(lab.unexpected(),0);assert_eq!(lab.counts(),0);
        if layout.later_group() && !cross {
            assert!(trace.iter().all(|plan|plan.selected.is_none()),"forbidden cross-group retry selected an attempt");
            assert!(status.is_client_error(),"cross-group retry=false was ignored");assert_eq!(lab.calls(),0);assert!(ids(&lab.fixture.pool).await.is_empty());
            let balance=lab.fixture.ledger.balance(lab.fixture.account).await.unwrap();assert_eq!((balance.available_units,balance.reserved_units),(FUNDS,0));return;
        }
        assert_eq!(status,StatusCode::OK,"authorized reviewed later target/group was not selected");assert_eq!(lab.calls(),1);
        {
            let seen=lab.servers[1].supplier.seen.lock().unwrap();assert_eq!(seen.len(),1);
            for field in ["temperature","top_p","store"] {assert_eq!(seen[0][field],body[field]);}
        }
        let id=id.unwrap();assert_eq!(charge(&lab.fixture.pool,&id).await,("settled".into(),HOLD,COST));
        let group:String=sqlx::query_scalar("SELECT price->>'group' FROM core_charging.requests WHERE id=$1").bind(&id).fetch_one(&lab.fixture.pool).await.unwrap();
        assert_eq!(group,if layout.later_group() {"review-second"} else {"allowed"});
        let binding:Value=sqlx::query_scalar("SELECT binding FROM core_model.responses_continuations WHERE request_id=$1 AND active")
            .bind(&id).fetch_one(&lab.fixture.pool).await.unwrap();assert_eq!(binding["upstream_id"],2);
        let selected:Vec<_>=trace.iter().filter_map(|plan|plan.selected.as_ref()).collect();
        assert_eq!(selected.len(),1,"one supplier call must have one actual selected attempt");
        assert_eq!(binding,selected[0].binding,"durable binding differs from the attempt actually retained and dispatched");
        let frozen:Value=sqlx::query_scalar("SELECT detail->'binding' FROM core_charging.usage_events WHERE request_id=$1 AND event_id='reservation-contract:v1'")
            .bind(&id).fetch_one(&lab.fixture.pool).await.unwrap();assert_eq!(binding,frozen);
        let balance=lab.fixture.ledger.balance(lab.fixture.account).await.unwrap();assert_eq!((balance.available_units,balance.reserved_units),(FUNDS-COST,0));
    })).await;
}
macro_rules! retry_case {
    ($name:ident,$layout:ident,$cross:literal) => {
        #[sqlx::test(migrations = false)]
        async fn $name(pool: PgPool) {
            retry_route(pool, RetryLayout::$layout, $cross, stringify!($name)).await;
        }
    };
}
retry_case!(
    responses_continuity_review_independent_target_no_continuity_cross_false,
    SameGroupNoContinuity,
    false
);

#[sqlx::test(migrations = false)]
async fn responses_continuity_review_independent_filter_retains_actual_plan_order(pool: PgPool) {
    run(pool, |lab| {
        Box::pin(async move {
            let config = lab.config([Some(true), Some(true)], [0, 1]);
            let gateway = gateway(&lab.runtime.pool, &config);
            let document = gateway.bridge.routing.catalog_document().unwrap();
            let mut routing = serde_json::to_value(document.routing).unwrap();
            // Plans only; these HTTPS endpoints are never prepared or dispatched.
            for upstream in routing["upstreams"].as_array_mut().unwrap() {
                upstream["endpoint"] = json!("https://api.openai.com/v1");
            }
            routing["routes"][0]["failover"]["max_attempts"] = json!(3);
            let mut upstream = routing["upstreams"][1].clone();
            upstream["id"] = json!(3);
            routing["upstreams"].as_array_mut().unwrap().push(upstream);
            let mut third = routing["routes"][0]["targets"][1].clone();
            third["id"] = json!(3);
            third["upstream_id"] = json!(3);
            routing["routes"][0]["targets"]
                .as_array_mut()
                .unwrap()
                .push(third);
            for (index, target) in routing["routes"][0]["targets"]
                .as_array_mut()
                .unwrap()
                .iter_mut()
                .enumerate()
            {
                target["priority"] = json!(0);
                target["weight"] = json!(index + 1);
            }
            let router = crate::routing::Router::new(
                serde_json::from_value(routing).unwrap(),
                Arc::new(PriceSnapshot::compile(document.pricing).unwrap()),
            )
            .unwrap();
            let mut plan = router
                .route(crate::routing::RouteRequest {
                    model: PUBLIC,
                    group: "allowed",
                    speed: None,
                    protocol: Protocol::OpenAiResponses,
                    replay: crate::routing::ReplayPermission::Denied,
                })
                .unwrap();
            let original = plan.planned_attempts();
            assert_eq!(original.len(), 3);
            let keep: std::collections::HashSet<_> = original[..2]
                .iter()
                .map(|attempt| attempt.target_id())
                .collect();
            plan.retain_targets(&keep).unwrap();
            let before: Vec<_> = plan
                .planned_attempts()
                .into_iter()
                .map(|attempt| {
                    (
                        attempt.route_id(),
                        attempt.target_id(),
                        attempt.upstream_id(),
                        attempt.credential_ref(),
                    )
                })
                .collect();
            let all_eligible: std::collections::HashSet<_> =
                original.iter().map(|attempt| attempt.key()).collect();
            let filtered = plan.responses_capability(&all_eligible).unwrap();
            let after: Vec<_> = filtered
                .planned_attempts()
                .into_iter()
                .map(|attempt| {
                    (
                        attempt.route_id(),
                        attempt.target_id(),
                        attempt.upstream_id(),
                        attempt.credential_ref(),
                    )
                })
                .collect();
            evidence(
                "immutable-plan-filter",
                json!({"before":before,"after":after}),
            );
            assert_eq!(
                after, before,
                "filter re-routed/resampled and resurrected a target removed from this exact plan"
            );
            assert_eq!(lab.calls(), 0);
            assert_eq!(lab.counts(), 0);
            assert!(ids(&lab.fixture.pool).await.is_empty());
            let balance = lab
                .fixture
                .ledger
                .balance(lab.fixture.account)
                .await
                .unwrap();
            assert_eq!(
                (balance.available_units, balance.reserved_units),
                (FUNDS, 0)
            );
        })
    })
    .await;
}
retry_case!(
    responses_continuity_review_independent_target_no_continuity_cross_true,
    SameGroupNoContinuity,
    true
);
retry_case!(
    responses_continuity_review_independent_target_non_native_cross_false,
    SameGroupNonNative,
    false
);
retry_case!(
    responses_continuity_review_independent_target_non_native_cross_true,
    SameGroupNonNative,
    true
);
retry_case!(
    responses_continuity_review_independent_group_no_continuity_cross_false,
    LaterGroupNoContinuity,
    false
);
retry_case!(
    responses_continuity_review_independent_group_no_continuity_cross_true,
    LaterGroupNoContinuity,
    true
);
retry_case!(
    responses_continuity_review_independent_group_non_native_cross_false,
    LaterGroupNonNative,
    false
);
retry_case!(
    responses_continuity_review_independent_group_non_native_cross_true,
    LaterGroupNonNative,
    true
);

const CACHE_HOLD: i64 = 5050;
const CACHE_ROOT_COST: i64 = 39;
const CACHE_CONTEXT_COST: i64 = 345;

fn native_cache_usage(input: u64, read: u64, writes: u64, long: u64) -> Value {
    json!({"input_tokens":input,"output_tokens":3,"total_tokens":input+3,
        "input_tokens_details":{"cached_tokens":read,"cache_write_tokens":writes,
            "cache_write_tokens_details":{"ttl_30m_tokens":writes-long,"ttl_12h_tokens":long}}})
}
fn native_cache_body() -> Value {
    json!({"model":PUBLIC,"input":[{"role":"user","content":[{"type":"input_text","text":"cache root context"}]}],
        "max_output_tokens":50,"store":true,"temperature":0.375,"top_p":0.625,
        "parallel_tool_calls":false,"prompt_cache_options":{"mode":"implicit","ttl":"12h"}})
}
fn native_cache_config(lab: &Lab, usage: Value) -> Config {
    let mut config = lab.config([Some(true), Some(true)], [0, 1]);
    config.reviewed_cache_tariff = true;
    config.nonflat_cache_tariff = true;
    for server in &lab.servers {
        *server.supplier.cache_usage.lock().unwrap() = Some(usage.clone());
    }
    config
}
fn native_cache_id(lab: &Lab, nonce: &str) -> String {
    let mut headers = HeaderMap::new();
    headers.insert("idempotency-key", HeaderValue::from_str(nonce).unwrap());
    request_id(lab.fixture.key_id, &headers).unwrap()
}
async fn native_cache_checkpoint(
    lab: &Lab,
    id: &str,
    packet: &[u8],
    input: u64,
    read: u64,
    writes: u64,
    long: u64,
) {
    let rows:Vec<(Vec<u8>,i64,Value)>=sqlx::query_as("SELECT packet_sha256,packet_bytes,provider_usage FROM core_model.output_checkpoints WHERE request_id=$1 AND terminal_observed")
        .bind(id).fetch_all(&lab.fixture.pool).await.unwrap();
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0].0, Sha256::digest(packet).to_vec());
    assert_eq!(rows[0].1, packet.len() as i64);
    let usage = &rows[0].2;
    assert_eq!(usage["input_tokens"], input);
    assert_eq!(usage["cached_input_tokens"], read);
    assert_eq!(usage["cache_creation_input_tokens"], writes);
    assert_eq!(usage["openai_cache_write_details"]["ttl_12h_tokens"], long);
    let revision: Value = sqlx::query_scalar(
        "SELECT price->'tiered'->'tariff'->'revision' FROM core_charging.requests WHERE id=$1",
    )
    .bind(id)
    .fetch_one(&lab.fixture.pool)
    .await
    .unwrap();
    assert_eq!(revision, json!(4));
    let count: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM core_model.responses_continuations WHERE request_id=$1 AND active",
    )
    .bind(id)
    .fetch_one(&lab.fixture.pool)
    .await
    .unwrap();
    assert_eq!(count, 1);
}
fn native_cache_wire(lab: &Lab, index: usize, body: &Value) {
    let seen = lab.servers[0].supplier.seen.lock().unwrap();
    let wire = &seen[index];
    for key in [
        "input",
        "temperature",
        "top_p",
        "parallel_tool_calls",
        "prompt_cache_options",
        "max_output_tokens",
        "store",
    ] {
        assert_eq!(
            wire.get(key),
            body.get(key),
            "native cache wire changed {key}"
        );
    }
    assert!(lab.servers[1].supplier.seen.lock().unwrap().is_empty());
    assert_eq!(lab.counts(), 0);
    assert_eq!(lab.unexpected(), 0);
}

#[sqlx::test(migrations = false)]
async fn responses_continuity_review_independent_native_cache_context_restart_replay(pool: PgPool) {
    run(pool,|lab|Box::pin(async move {
        let config=native_cache_config(lab,native_cache_usage(7,2,3,2));
        let mut core=Core::start(config.clone(),None).await;let body=native_cache_body();let nonce="native-cache-root";
        let id=native_cache_id(lab,nonce);let response=post(&core.base,&lab.fixture.key,nonce,&body).await.unwrap();assert_eq!(response.status(),StatusCode::OK);
        let packet=response.bytes().await.unwrap();let output:Value=serde_json::from_slice(&packet).unwrap();let handle=output["id"].as_str().unwrap().to_owned();
        assert_eq!(charge(&lab.fixture.pool,&id).await,("settled".into(),CACHE_HOLD,CACHE_ROOT_COST));
        native_cache_wire(lab,0,&body);native_cache_checkpoint(lab,&id,&packet,7,2,3,2).await;
        let root_money=money(&lab.fixture.pool).await;
        let duplicate=post(&core.base,&lab.fixture.key,nonce,&body).await.unwrap();assert_eq!(duplicate.status(),StatusCode::CONFLICT);drop(duplicate);
        assert_eq!(lab.calls(),1);assert_eq!(money(&lab.fixture.pool).await,root_money);
        core.kill9();drop(core);let core=Core::start(config.clone(),None).await;
        *lab.servers[0].supplier.cache_usage.lock().unwrap()=Some(native_cache_usage(100,20,30,20));
        let mut child=body.clone();child["input"]=json!([{"role":"user","content":[{"type":"input_text","text":"tiny current turn"}]}]);
        child["previous_response_id"]=handle.into();let child_id=native_cache_id(lab,"native-cache-child");
        let response=post(&core.base,&lab.fixture.key,"native-cache-child",&child).await.unwrap();assert_eq!(response.status(),StatusCode::OK);
        let packet=response.bytes().await.unwrap();
        let child_output:Value=serde_json::from_slice(&packet).unwrap();
        let full_context=child_output["output"][0]["content"][0]["text"].as_str().unwrap();
        assert!(full_context.contains("cache root context") && full_context.contains("tiny current turn"));
        assert_eq!(charge(&lab.fixture.pool,&child_id).await,("settled".into(),CACHE_HOLD,CACHE_CONTEXT_COST));
        native_cache_checkpoint(lab,&child_id,&packet,100,20,30,20).await;
        let seen=lab.servers[0].supplier.seen.lock().unwrap().clone();
        assert_eq!(seen[1]["previous_response_id"],"resp_review_supplier_1");
        for field in ["input","temperature","top_p","parallel_tool_calls","prompt_cache_options","max_output_tokens","store"] {assert_eq!(seen[1][field],child[field]);}
        assert_eq!(lab.calls(),2);assert_eq!(lab.counts(),0);assert_eq!(lab.unexpected(),0);
        let balance=lab.fixture.ledger.balance(lab.fixture.account).await.unwrap();assert_eq!((balance.available_units,balance.reserved_units),(FUNDS-CACHE_ROOT_COST-CACHE_CONTEXT_COST,0));
        let before=money(&lab.fixture.pool).await;
        assert_eq!(post(&core.base,&lab.fixture.key,"native-cache-child",&child).await.unwrap().status(),StatusCode::CONFLICT);
        let gateway=gateway(&lab.runtime.pool,&config);
        for _ in 0..3 {assert_eq!(gateway.reconcile_completed_streams(2).await.unwrap(),0);}
        assert_eq!(money(&lab.fixture.pool).await,before);assert_eq!(lab.calls(),2);
        evidence("native-cache-context",json!({"root_charge":39,"whole_context_charge":345,"each_hold":5050,"supplier_calls":2,"restart":"SIGKILL","no_repeat_money":true}));
    })).await;
}

#[sqlx::test(migrations = false)]
async fn responses_continuity_review_independent_native_cache_sse_nonzero(pool: PgPool) {
    run(pool,|lab|Box::pin(async move {
        let config=native_cache_config(lab,native_cache_usage(7,2,3,2));let core=Core::start(config,None).await;
        let mut body=native_cache_body();body["stream"]=true.into();let id=native_cache_id(lab,"native-cache-sse");
        let response=post(&core.base,&lab.fixture.key,"native-cache-sse",&body).await.unwrap();assert_eq!(response.status(),StatusCode::OK);
        let bytes=response.bytes().await.unwrap();let text=std::str::from_utf8(&bytes).unwrap();
        let frames:Vec<_>=text.split("\n\n").filter(|frame|frame.contains("event: response.completed\n")).collect();assert_eq!(frames.len(),1);
        let terminal=format!("{}\n\n",frames[0]);
        native_cache_checkpoint(lab,&id,terminal.as_bytes(),7,2,3,2).await;
        assert_eq!(charge(&lab.fixture.pool,&id).await,("settled".into(),CACHE_HOLD,CACHE_ROOT_COST));native_cache_wire(lab,0,&body);
        let balance=lab.fixture.ledger.balance(lab.fixture.account).await.unwrap();assert_eq!((balance.available_units,balance.reserved_units),(FUNDS-CACHE_ROOT_COST,0));
        evidence("native-cache-sse",json!({"hold":5050,"settled":39,"terminal_count":1,"actual_eof":true,"bytes":bytes.len()}));
    })).await;
}

#[sqlx::test(migrations = false)]
async fn responses_continuity_review_independent_native_cache_candidate_recovery(pool: PgPool) {
    run(pool,|lab|Box::pin(async move {
        let config=native_cache_config(lab,native_cache_usage(7,2,3,2));let mut core=Core::start(config.clone(),Some("after_candidate")).await;
        let body=native_cache_body();let id=native_cache_id(lab,"native-cache-crash");
        if let Ok(response)=post(&core.base,&lab.fixture.key,"native-cache-crash",&body).await {assert!(!response.status().is_success());}
        core.exited97().await;assert_eq!(charge(&lab.fixture.pool,&id).await.1,CACHE_HOLD);assert_eq!(charge(&lab.fixture.pool,&id).await.2,0);
        let pending_balance=lab.fixture.ledger.balance(lab.fixture.account).await.unwrap();
        assert_eq!((pending_balance.available_units,pending_balance.reserved_units),(FUNDS-CACHE_HOLD,CACHE_HOLD));
        let checkpoint:(Vec<u8>,i64,Value)=sqlx::query_as("SELECT packet_sha256,packet_bytes,provider_usage FROM core_model.output_checkpoints WHERE request_id=$1 AND terminal_observed")
            .bind(&id).fetch_one(&lab.fixture.pool).await.unwrap();assert!(checkpoint.1>0);assert_eq!(checkpoint.2["openai_cache_write_details"]["ttl_12h_tokens"],2);
        let gateway=gateway(&lab.runtime.pool,&config);tokio::time::sleep(Duration::from_secs(LEASE+1)).await;gateway.recover_expired(100).await.unwrap();
        let mut recovered=0;for _ in 0..8 {recovered+=gateway.reconcile_completed_streams(1).await.unwrap();if charge(&lab.fixture.pool,&id).await.0=="settled" && active(&lab.fixture.pool).await.contains(&id) {break;}}
        assert_eq!(recovered,1);assert!(active(&lab.fixture.pool).await.contains(&id));assert_eq!(charge(&lab.fixture.pool,&id).await,("settled".into(),CACHE_HOLD,CACHE_ROOT_COST));
        let restarted=Core::start(config,None).await;
        assert_eq!(post(&restarted.base,&lab.fixture.key,"native-cache-crash",&body).await.unwrap().status(),StatusCode::CONFLICT);
        let before=money(&lab.fixture.pool).await;for _ in 0..3 {assert_eq!(gateway.reconcile_completed_streams(1).await.unwrap(),0);}
        assert_eq!(money(&lab.fixture.pool).await,before);assert_eq!(lab.calls(),1);assert_eq!(lab.counts(),0);
        let after:(Vec<u8>,i64,Value)=sqlx::query_as("SELECT packet_sha256,packet_bytes,provider_usage FROM core_model.output_checkpoints WHERE request_id=$1 AND terminal_observed")
            .bind(&id).fetch_one(&lab.fixture.pool).await.unwrap();assert_eq!(after,checkpoint);
        let balance=lab.fixture.ledger.balance(lab.fixture.account).await.unwrap();assert_eq!((balance.available_units,balance.reserved_units),(FUNDS-CACHE_ROOT_COST,0));
        evidence("native-cache-recovery",json!({"app_exit":97,"actual_wire_ack_loss":false,"hold":5050,"settled":39,"settlement_count":1,"supplier_calls":1}));
    })).await;
}

async fn native_cache_unprovable(pool: PgPool, kind: &'static str) {
    run(pool,move |lab|Box::pin(async move {
        let mut usage=native_cache_usage(7,2,3,2);
        match kind {
            "missing-aggregate"=>{usage["input_tokens_details"].as_object_mut().unwrap().remove("cache_write_tokens");},
            "missing-12h"=>{usage["input_tokens_details"]["cache_write_tokens_details"].as_object_mut().unwrap().remove("ttl_12h_tokens");},
            "contradiction"=>{usage["input_tokens_details"]["cache_write_tokens_details"]["ttl_12h_tokens"]=json!(4);},
            _=>panic!("invalid private supplier mode"),
        }
        let config=native_cache_config(lab,usage);let core=Core::start(config.clone(),None).await;let body=native_cache_body();let nonce=format!("native-cache-{kind}");let id=native_cache_id(lab,&nonce);
        let response=post(&core.base,&lab.fixture.key,&nonce,&body).await.unwrap();assert!(!response.status().is_success());let error=response.bytes().await.unwrap();
        let charge_before=charge(&lab.fixture.pool,&id).await;assert_eq!(charge_before.1,CACHE_HOLD);assert_eq!(charge_before.2,0);assert_ne!(charge_before.0,"settled");
        let balance=lab.fixture.ledger.balance(lab.fixture.account).await.unwrap();assert_eq!((balance.available_units,balance.reserved_units),(FUNDS-CACHE_HOLD,CACHE_HOLD));
        assert!(!active(&lab.fixture.pool).await.contains(&id));let before=money(&lab.fixture.pool).await;
        let gateway=gateway(&lab.runtime.pool,&config);tokio::time::sleep(Duration::from_secs(LEASE+1)).await;gateway.recover_expired(100).await.unwrap();
        for _ in 0..4 {assert_eq!(gateway.reconcile_completed_streams(1).await.unwrap(),0);}
        assert_eq!(money(&lab.fixture.pool).await,before);assert_eq!(charge(&lab.fixture.pool,&id).await,("reconcile".into(),CACHE_HOLD,0));
        assert_eq!(post(&core.base,&lab.fixture.key,&nonce,&body).await.unwrap().status(),StatusCode::CONFLICT);
        assert_eq!(lab.calls(),1);assert_eq!(lab.counts(),0);assert_eq!(lab.unexpected(),0);
        evidence(&format!("native-cache-{kind}"),json!({"error":String::from_utf8_lossy(&error),"hold":5050,"settled":0,"supplier_calls":1,"inferred_from_ttl":false}));
    })).await;
}
macro_rules! native_cache_unknown_case {
    ($name:ident,$kind:literal) => {
        #[sqlx::test(migrations = false)]
        async fn $name(pool: PgPool) {
            native_cache_unprovable(pool, $kind).await;
        }
    };
}
native_cache_unknown_case!(
    responses_continuity_review_independent_native_cache_missing_aggregate,
    "missing-aggregate"
);
native_cache_unknown_case!(
    responses_continuity_review_independent_native_cache_missing_12h,
    "missing-12h"
);
native_cache_unknown_case!(
    responses_continuity_review_independent_native_cache_contradiction,
    "contradiction"
);

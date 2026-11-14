//! Developer business checks: real TCP Core child, isolated PG, durable HTTP supplier.
use super::*;
use crate::{
    routing::{self, CredentialRef, Endpoint, PriceConfig, PriceSnapshot, RoutingConfig},
    test_runtime::RuntimeRole,
};
use axum::{extract::State, routing::post};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::{
    collections::HashMap,
    fs,
    io::Write,
    os::unix::fs::OpenOptionsExt,
    path::PathBuf,
    process::{Child, Command, Stdio},
    sync::atomic::{AtomicBool, AtomicUsize, Ordering},
};

#[path = "responses_continuity_cache_tests.rs"]
mod cache_tests;

#[derive(Clone)]
struct Supplier {
    base: String,
    seen: Arc<Mutex<Vec<Value>>>,
    stored: Arc<Mutex<HashMap<String, String>>>,
    calls: Arc<AtomicUsize>,
    counts: Arc<AtomicUsize>,
    bad: Arc<AtomicBool>,
    cache_usage: Arc<Mutex<Option<Value>>>,
    trailing_stream_bytes: Arc<AtomicBool>,
}
async fn provider(State(s): State<Supplier>, Json(body): Json<Value>) -> Response {
    s.calls.fetch_add(1, Ordering::SeqCst);
    s.seen.lock().unwrap().push(body.clone());
    let usage = s.cache_usage.lock().unwrap().clone().unwrap_or_else(|| {
        json!({"input_tokens":17,"output_tokens":3,"total_tokens":20,
            "input_tokens_details":{"cached_tokens":0}})
    });
    let history = if let Some(parent) = body["previous_response_id"].as_str() {
        s.stored
            .lock()
            .unwrap()
            .get(parent)
            .expect("Core must send supplier parent, not local handle")
            .clone()
    } else {
        String::new()
    };
    if body.get("max_output_tokens").is_none() {
        if s.bad.load(Ordering::SeqCst) {
            return Json(json!({"id":"cmp_unknown","object":"response.compaction",
                "output":[{"type":"compaction","encrypted_content":"opaque"}]}))
            .into_response();
        }
        return Json(json!({"id":"cmp_provider","object":"response.compaction",
            "output":[{"type":"compaction","encrypted_content":format!("opaque:{}",history)}],
            "usage":usage}))
        .into_response();
    }
    let n = s.calls.load(Ordering::SeqCst);
    let id = format!("resp_provider_{n}");
    let text = format!(
        "{history}|{}",
        body.get("input").cloned().unwrap_or(Value::Null)
    );
    let stored = body["store"].as_bool().unwrap_or(true);
    if stored {
        s.stored.lock().unwrap().insert(id.clone(), text.clone());
    }
    let out = json!({"id":id,"object":"response","status":"completed","store":stored,"model":"native-model",
        "output":[{"type":"message","role":"assistant","content":[{"type":"output_text","text":text,"annotations":[]}]}],
        "usage":usage});
    if body["stream"] == true {
        let mut frames = format!(
            "event: response.created\ndata: {}\n\nevent: response.completed\ndata: {}\n\n",
            json!({"type":"response.created","response":{"id":id,"status":"in_progress"}}),
            json!({"type":"response.completed","response":out})
        );
        if s.trailing_stream_bytes.load(Ordering::SeqCst) {
            frames.push_str("invalid trailing bytes");
        }
        return ([(header::CONTENT_TYPE, "text/event-stream")], frames).into_response();
    }
    (
        [(header::CONTENT_TYPE, "application/json")],
        serde_json::to_string_pretty(&out).unwrap(),
    )
        .into_response()
}
async fn supplier() -> (Supplier, tokio::task::JoinHandle<()>) {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let s = Supplier {
        base: format!("http://{}/v1", listener.local_addr().unwrap()),
        seen: Default::default(),
        stored: Default::default(),
        calls: Default::default(),
        counts: Default::default(),
        bad: Default::default(),
        cache_usage: Default::default(),
        trailing_stream_bytes: Default::default(),
    };
    let app = Router::new()
        .route("/v1/responses", post(provider))
        .route("/v1/responses/compact", post(provider))
        .route(
            "/v1/responses/input_tokens",
            post(|State(s): State<Supplier>| async move {
                s.counts.fetch_add(1, Ordering::SeqCst);
                Json(json!({"input_tokens":7}))
            }),
        )
        .with_state(s.clone());
    let task = tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
    (s, task)
}
#[derive(Serialize, Deserialize)]
struct WorkerConfig {
    database: String,
    suppliers: Vec<String>,
    mode: String,
    ready: String,
}
fn gateway(pool: &PgPool, c: &WorkerConfig) -> ModelGateway {
    let exact = c.mode == "exact";
    let model = if exact {
        "gpt-4.1-2025-04-14"
    } else if c.mode == "remap" {
        "other-model"
    } else {
        "native-model"
    };
    let revision = if c.mode == "rotate" { 2 } else { 1 };
    let mut config:RoutingConfig=serde_json::from_value(json!({"version":1,"groups":[{"name":"allowed","enabled":true}],"aliases":[],
        "upstreams":[{"id":1,"endpoint":"https://synthetic.invalid/v1","protocol":"openai_responses","credential":{"id":7,"revision":revision},"enabled":c.mode!="disable"},
        {"id":2,"endpoint":"https://synthetic2.invalid/v1","protocol":"openai_responses","credential":{"id":8,"revision":1},"enabled":true}],
        "routes":[{"id":1,"model":"continuity-public","group":"allowed","enabled":true,"speeds":["standard"],"targets":[
        {"id":1,"upstream_id":1,"upstream_model":model,"enabled":true,"priority":if c.mode=="prefer_b" {1} else {0},"weight":1,"speeds":["standard"],"payload":"adapt","timeout_ms":10000},
        {"id":2,"upstream_id":2,"upstream_model":"native-model","enabled":true,"priority":if c.mode=="prefer_b" {0} else {1},"weight":1,"speeds":["standard"],"payload":"adapt","timeout_ms":10000}],
        "failover":{"max_attempts":1,"retry_on":[],"allow_replay_after_send":false}}]})).unwrap();
    for (upstream, url) in config.upstreams.iter_mut().zip(&c.suppliers) {
        upstream.endpoint = Endpoint::local_loopback(url.clone()).unwrap();
    }
    let price: PriceConfig = if c.mode.starts_with("rev4") {
        cache_tests::price(&c.mode)
    } else {
        serde_json::from_value(json!({"version":1,"rules":[{"id":1,"model":"continuity-public","group":"allowed","speed":"standard",
            "rates":{"input":2000000,"output":4000000,"cache_read":2000000,"cache_write":0,"request":0},"group_multiplier_ppm":1000000,"speed_multiplier_ppm":1000000}]})).unwrap()
    };
    let contexts = [(1, model), (2, "native-model")]
        .into_iter()
        .map(|(id, model)| ExternalModelContext {
            responses_compaction: true,
            responses_continuity: if c.mode == "predecessor" {
                None
            } else {
                Some(ResponsesContinuityContract {
                    default_store: true,
                })
            },
            upstream_id: id,
            upstream_model: model.into(),
            protocol: crate::routing::Protocol::OpenAiResponses,
            endpoint: if exact {
                "https://api.openai.com/v1".into()
            } else {
                format!("https://supplier{id}.synthetic.invalid/v1")
            },
            max_input_tokens: if c.mode.starts_with("rev4") {
                800
            } else {
                1000
            },
            max_output_tokens: if c.mode.starts_with("rev4") { 50 } else { 100 },
            contract_revision: "continuity-dev-v1".into(),
            missing_cached_tokens_is_zero: false,
            missing_cache_creation_tokens_is_zero: false,
            reservation: if exact && id == 1 {
                super::super::provider_policy::ReservationProfile::OpenaiResponsesExactV1 {
                    count_timeout_ms: std::num::NonZeroU64::new(1000).unwrap(),
                }
            } else {
                Default::default()
            },
            grounding: None,
            pre_acceptance_rejection_statuses: vec![],
        })
        .collect();
    ModelGateway::new_with_provider_policy(
        pool.clone(),
        Arc::new(
            routing::Router::new(config, Arc::new(PriceSnapshot::compile(price).unwrap())).unwrap(),
        ),
        vec![
            UpstreamCredential {
                reference: CredentialRef { id: 7, revision },
                secret: "synthetic-provider-credential".into(),
            },
            UpstreamCredential {
                reference: CredentialRef { id: 8, revision: 1 },
                secret: "synthetic-provider-credential".into(),
            },
        ],
        BTreeMap::new(),
        Limits {
            max_output_tokens: 100,
            total_timeout: Duration::from_secs(2),
            header_timeout: Duration::from_secs(2),
            hook_timeout: Duration::from_millis(500),
            ..Default::default()
        },
        ChargePolicy {
            input_token_ceiling: if c.mode.starts_with("rev4") {
                800
            } else {
                1000
            },
            lease_seconds: 8,
        },
        ProviderPolicy::new(
            contexts,
            if c.mode.starts_with("rev4") {
                800
            } else {
                1000
            },
        )
        .unwrap(),
    )
    .unwrap()
}
fn artifacts() -> PathBuf {
    let p = PathBuf::from(std::env::var("LMM_CONTINUITY_DEV_ARTIFACTS").unwrap());
    fs::create_dir_all(&p).unwrap();
    p
}
#[tokio::test]
#[ignore = "actual child HTTP service entry, not an acceptance case"]
async fn continuity_dev_worker() {
    let c: WorkerConfig = serde_json::from_slice(
        &fs::read(std::env::var("LMM_CONTINUITY_DEV_WORKER").unwrap()).unwrap(),
    )
    .unwrap();
    let pool = sqlx::postgres::PgPoolOptions::new()
        .max_connections(3)
        .connect(&c.database)
        .await
        .unwrap();
    let g = gateway(&pool, &c);
    g.recover_expired(100).await.unwrap();
    if c.mode != "predecessor" {
        g.reconcile_responses_continuity_page(100, None)
            .await
            .unwrap();
    }
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    fs::write(
        &c.ready,
        format!("http://{}", listener.local_addr().unwrap()),
    )
    .unwrap();
    axum::serve(
        listener,
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
    async fn start(
        database: &str,
        a: &Supplier,
        b: &Supplier,
        mode: &str,
        crash: Option<&str>,
    ) -> Self {
        let nonce = format!("{:016x}", rand::random::<u64>());
        let dir = artifacts();
        let config = dir.join(format!("dev-{nonce}.private.json"));
        let ready = dir.join(format!("dev-{nonce}.ready"));
        let c = WorkerConfig {
            database: database.into(),
            suppliers: vec![a.base.clone(), b.base.clone()],
            mode: mode.into(),
            ready: ready.to_str().unwrap().into(),
        };
        let mut file = fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .mode(0o600)
            .open(&config)
            .unwrap();
        file.write_all(&serde_json::to_vec(&c).unwrap()).unwrap();
        drop(file);
        let log = fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .mode(0o600)
            .open(dir.join(format!("dev-{nonce}.log")))
            .unwrap();
        let mut command = Command::new(std::env::current_exe().unwrap());
        command
            .args([
                "--exact",
                "model_http::responses_continuity::tests::continuity_dev_worker",
                "--ignored",
                "--nocapture",
                "--test-threads=1",
            ])
            .env("LMM_CONTINUITY_DEV_WORKER", &config)
            .env_remove("DATABASE_URL")
            .env_remove("LMM_RESPONSES_CONTINUITY_CRASH_AT")
            .env_remove("LMM_RESPONSES_CONTINUITY_LOSE_ACK_AT")
            .stdout(Stdio::from(log.try_clone().unwrap()))
            .stderr(Stdio::from(log));
        if let Some(crash) = crash {
            if let Some(stage) = crash.strip_prefix("ack:") {
                command.env("LMM_RESPONSES_CONTINUITY_LOSE_ACK_AT", stage);
            } else {
                command.env("LMM_RESPONSES_CONTINUITY_CRASH_AT", crash);
            }
        }
        let child = command.spawn().unwrap();
        let mut core = Self {
            child,
            base: String::new(),
            config,
            ready,
        };
        for _ in 0..500 {
            if let Ok(base) = fs::read_to_string(&core.ready) {
                core.base = base;
                return core;
            }
            assert!(
                core.child.try_wait().unwrap().is_none(),
                "worker exited before ready; inspect private log"
            );
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
        panic!("HTTP child readiness timeout")
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
async fn request(
    core: &Core,
    key: &str,
    id: &str,
    compact: bool,
    body: Value,
) -> Result<reqwest::Response, reqwest::Error> {
    reqwest::Client::builder()
        .no_proxy()
        .timeout(Duration::from_secs(8))
        .build()
        .unwrap()
        .post(format!(
            "{}/v1/responses{}",
            core.base,
            if compact { "/compact" } else { "" }
        ))
        .bearer_auth(key)
        .header("idempotency-key", id)
        .json(&body)
        .send()
        .await
}
fn body(input: Value) -> Value {
    json!({"model":"continuity-public","input":input})
}
async fn runtime(pool: &PgPool) -> RuntimeRole {
    let r = RuntimeRole::create(pool).await;
    r.optional("core_charging").await;
    r.optional("core_model").await;
    r
}
async fn packet(pool: &PgPool, id: &str, bytes: &[u8]) {
    let stored:(Vec<u8>,i64)=sqlx::query_as("SELECT packet_sha256,packet_bytes FROM core_model.output_checkpoints WHERE request_id=$1 AND terminal_observed").bind(id).fetch_one(pool).await.unwrap();
    assert_eq!(stored, (Sha256::digest(bytes).to_vec(), bytes.len() as i64));
}

#[sqlx::test(migrations = false)]
async fn continuity_pg_real_http_store_restart_sse_compact_and_binding(pool: PgPool) {
    let f = native_http::test_support::fixture(pool, 10000, &["continuity-public"]).await;
    let rt = runtime(&f.pool).await;
    let (a, ta) = supplier().await;
    let (b, tb) = supplier().await;
    let core = Core::start(&rt.url, &a, &b, "normal", None).await;
    let response = request(&core, &f.key, "root", false, body(json!("history")))
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let id = response.headers()["x-request-id"]
        .to_str()
        .unwrap()
        .to_owned();
    let bytes = response.bytes().await.unwrap();
    packet(&f.pool, &id, &bytes).await;
    let root: Value = serde_json::from_slice(&bytes).unwrap();
    let handle = root["id"].as_str().unwrap().to_owned();
    assert!(handle.starts_with("resp_lmm_"));
    assert!(a.seen.lock().unwrap()[0].get("store").is_none());
    drop(core);
    let core = Core::start(&rt.url, &a, &b, "prefer_b", None).await;
    let response=request(&core,&f.key,"next",false,json!({"model":"continuity-public","input":"next","previous_response_id":handle,"store":true,"stream":true})).await.unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let next_id = response.headers()["x-request-id"]
        .to_str()
        .unwrap()
        .to_owned();
    let wire = response.bytes().await.unwrap();
    let text = std::str::from_utf8(&wire).unwrap();
    assert!(text.contains("history") && text.contains("next"));
    let packets: Vec<_> = text.split_inclusive("\n\n").map(str::as_bytes).collect();
    let checkpoints:Vec<(Vec<u8>,i64)>=sqlx::query_as("SELECT packet_sha256,packet_bytes FROM core_model.output_checkpoints WHERE request_id=$1 ORDER BY sequence").bind(&next_id).fetch_all(&f.pool).await.unwrap();
    assert_eq!(packets.len(), checkpoints.len());
    for (bytes, stored) in packets.into_iter().zip(checkpoints) {
        assert_eq!((Sha256::digest(bytes).to_vec(), bytes.len() as i64), stored);
    }
    assert_eq!(a.calls.load(Ordering::SeqCst), 2);
    assert_eq!(b.calls.load(Ordering::SeqCst), 0);
    assert_eq!(
        a.seen.lock().unwrap()[1]["previous_response_id"],
        "resp_provider_1"
    );
    let response = request(
        &core,
        &f.key,
        "compact-parent",
        true,
        json!({"model":"continuity-public","previous_response_id":handle}),
    )
    .await
    .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let compact: Value = response.json().await.unwrap();
    assert!(
        compact["output"][0]["encrypted_content"]
            .as_str()
            .unwrap()
            .contains("history")
    );
    let response = request(
        &core,
        &f.key,
        "opaque-replay",
        false,
        json!({"model":"continuity-public","input":compact["output"],"store":true}),
    )
    .await
    .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    assert_eq!(b.calls.load(Ordering::SeqCst), 1);
    drop(core);
    for mode in ["rotate", "remap", "disable", "exact"] {
        let core = Core::start(&rt.url, &a, &b, mode, None).await;
        let response=request(&core,&f.key,&format!("denied-{mode}"),false,json!({"model":"continuity-public","previous_response_id":handle,"input":"denied","stream":true})).await.unwrap();
        assert!(!response.status().is_success());
    }
    assert_eq!(a.calls.load(Ordering::SeqCst), 3);
    assert_eq!(b.calls.load(Ordering::SeqCst), 1);
    assert!(
        sqlx::query("SELECT * FROM core_model.responses_continuations")
            .fetch_all(&rt.pool)
            .await
            .is_err()
    );
    ta.abort();
    tb.abort();
    rt.cleanup().await;
}

#[sqlx::test(migrations = false)]
async fn continuity_pg_denial_false_unknown_and_full_context_budget(pool: PgPool) {
    let f = native_http::test_support::fixture(pool, 1250, &["continuity-public"]).await;
    let rt = runtime(&f.pool).await;
    let (a, ta) = supplier().await;
    let (b, tb) = supplier().await;
    let core = Core::start(&rt.url, &a, &b, "normal", None).await;
    let response = request(
        &core,
        &f.key,
        "stored",
        false,
        json!({"model":"continuity-public","input":"large old history","store":true}),
    )
    .await
    .unwrap();
    let root: Value = response.json().await.unwrap();
    let handle = root["id"].as_str().unwrap();
    for previous in [
        format!("resp_lmm_{}", "0".repeat(64)),
        "resp_provider_1".into(),
    ] {
        let response=request(&core,&f.key,"unknown",false,json!({"model":"continuity-public","previous_response_id":previous,"input":"x","stream":true})).await.unwrap();
        assert_eq!(response.status(), StatusCode::BAD_REQUEST);
    }
    let response = request(
        &core,
        &f.key,
        "hidden-history-budget",
        false,
        json!({"model":"continuity-public","previous_response_id":handle,"input":"x"}),
    )
    .await
    .unwrap();
    // 1250 - 23 = 1227 is enough for another full1200 hold; consume once more.
    assert_eq!(response.status(), StatusCode::OK);
    let second: Value = response.json().await.unwrap();
    let response=request(&core,&f.key,"insufficient",false,json!({"model":"continuity-public","previous_response_id":second["id"],"input":"x","stream":true})).await.unwrap();
    // 1204 still funds a full1200 hold, so this call also succeeds, not a guessed incremental refusal.
    assert_eq!(response.status(), StatusCode::OK);
    let _ = response.bytes().await.unwrap();
    let before = a.calls.load(Ordering::SeqCst);
    let response=request(&core,&f.key,"insufficient-final",false,json!({"model":"continuity-public","previous_response_id":handle,"input":"x","stream":true})).await.unwrap();
    assert!(!response.status().is_success());
    assert_eq!(a.calls.load(Ordering::SeqCst), before);
    sqlx::query("UPDATE core_identity.credentials SET revoked_at=clock_timestamp() WHERE id=$1")
        .bind(f.key_id)
        .execute(&f.pool)
        .await
        .unwrap();
    let response = request(
        &core,
        &f.key,
        "revoked",
        false,
        json!({"model":"continuity-public","previous_response_id":handle,"input":"x"}),
    )
    .await
    .unwrap();
    assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
    assert_eq!(a.calls.load(Ordering::SeqCst), before);
    drop(core);
    ta.abort();
    tb.abort();
    rt.cleanup().await;
}

#[sqlx::test(migrations = false)]
async fn continuity_pg_real_process_crash_windows_preserve_original_charge(pool: PgPool) {
    let f = native_http::test_support::fixture(pool, 20000, &["continuity-public"]).await;
    let rt = runtime(&f.pool).await;
    let (a, ta) = supplier().await;
    let (b, tb) = supplier().await;
    for stage in [
        "after_terminal",
        "after_candidate",
        "after_settlement",
        "after_activation",
    ] {
        let old_ids: Vec<String> = sqlx::query_scalar("SELECT id FROM core_charging.requests")
            .fetch_all(&f.pool)
            .await
            .unwrap();
        let mut core = Core::start(&rt.url, &a, &b, "normal", Some(stage)).await;
        assert!(
            request(
                &core,
                &f.key,
                stage,
                false,
                json!({"model":"continuity-public","input":stage,"store":true})
            )
            .await
            .is_err()
        );
        assert_eq!(core.child.wait().unwrap().code(), Some(97));
        drop(core);
        let id: String =
            sqlx::query_scalar("SELECT id FROM core_charging.requests WHERE NOT(id=ANY($1))")
                .bind(&old_ids)
                .fetch_one(&f.pool)
                .await
                .unwrap();
        let handle = super::handle(&id);
        if stage == "after_terminal" {
            let c:i64=sqlx::query_scalar("SELECT count(*) FROM core_model.output_checkpoints WHERE request_id=$1 AND terminal_observed").bind(&id).fetch_one(&f.pool).await.unwrap();
            assert_eq!(c, 0);
            let c: i64 = sqlx::query_scalar(
                "SELECT count(*) FROM core_model.responses_continuations WHERE request_id=$1",
            )
            .bind(&id)
            .fetch_one(&f.pool)
            .await
            .unwrap();
            assert_eq!(c, 0);
        }
        if stage == "after_terminal" || stage == "after_candidate" {
            tokio::time::sleep(Duration::from_secs(9)).await;
        }
        let before = a.calls.load(Ordering::SeqCst);
        let core = Core::start(&rt.url, &a, &b, "normal", None).await;
        let response=request(&core,&f.key,&format!("continue-{stage}"),false,json!({"model":"continuity-public","input":"next","previous_response_id":handle,"store":false})).await.unwrap();
        if stage == "after_terminal" {
            assert_eq!(response.status(), StatusCode::BAD_REQUEST);
            assert_eq!(a.calls.load(Ordering::SeqCst), before);
        } else {
            assert_eq!(response.status(), StatusCode::OK);
            let _ = response.bytes().await.unwrap();
            assert_eq!(a.calls.load(Ordering::SeqCst), before + 1);
        }
        let duplicate = request(
            &core,
            &f.key,
            stage,
            false,
            json!({"model":"continuity-public","input":stage,"store":true}),
        )
        .await
        .unwrap();
        assert_eq!(duplicate.status(), StatusCode::CONFLICT);
        let (state, reserved, settled): (String, i64, i64) =
            sqlx::query_as("SELECT state,reserved,settled FROM core_charging.requests WHERE id=$1")
                .bind(&id)
                .fetch_one(&f.pool)
                .await
                .unwrap();
        assert_eq!(reserved, 1200);
        if stage == "after_terminal" {
            assert_eq!((state.as_str(), settled), ("reconcile", 0));
        } else {
            assert_eq!((state.as_str(), settled), ("settled", 23));
        }
    }
    ta.abort();
    tb.abort();
    rt.cleanup().await;
}

#[sqlx::test(migrations = false)]
async fn continuity_pg_owner_key_boundary_and_explicit_false(pool: PgPool) {
    let f = native_http::test_support::fixture(pool, 10000, &["continuity-public"]).await;
    let rt = runtime(&f.pool).await;
    let (a, ta) = supplier().await;
    let (b, tb) = supplier().await;
    let core = Core::start(&rt.url, &a, &b, "normal", None).await;
    let response = request(
        &core,
        &f.key,
        "owner-root",
        false,
        json!({"model":"continuity-public","input":"private","store":true}),
    )
    .await
    .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let stored: Value = response.json().await.unwrap();
    let handle = stored["id"].as_str().unwrap();
    let identity = IdentityStore::from_pool(f.pool.clone());
    for user in [7, 8] {
        let session = if user == 7 {
            f.session.clone()
        } else {
            identity.bootstrap_user(user, 1).await.unwrap().secret
        };
        let key = identity
            .issue_key(
                &session,
                crate::accounts::Account {
                    kind: crate::accounts::AccountKind::Personal,
                    id: user,
                },
                crate::funding::FundingPolicy::new_personal_key(),
                3600,
            )
            .await
            .unwrap();
        sqlx::query("INSERT INTO core_identity.api_key_settings(credential_id,name,group_name,model_limits_enabled,model_limits) VALUES($1,'continuity-other','allowed',TRUE,ARRAY['continuity-public'])").bind(key.id).execute(&f.pool).await.unwrap();
        if user != 7 {
            sqlx::query("INSERT INTO core_identity.user_routing_groups(user_id,group_name,enabled,source_reference) VALUES($1,'allowed',TRUE,'continuity-owner-test')").bind(user).execute(&f.pool).await.unwrap();
        }
        let response=request(&core,&key.secret,"other-owner-denied",false,json!({"model":"continuity-public","input":"x","previous_response_id":handle,"stream":true})).await.unwrap();
        assert_eq!(response.status(), StatusCode::BAD_REQUEST);
    }
    assert_eq!(a.calls.load(Ordering::SeqCst), 1);
    let response = request(
        &core,
        &f.key,
        "no-store",
        false,
        json!({"model":"continuity-public","input":"stateless","store":false}),
    )
    .await
    .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let request_id = response.headers()["x-request-id"]
        .to_str()
        .unwrap()
        .to_owned();
    let result: Value = response.json().await.unwrap();
    assert_eq!(a.seen.lock().unwrap().last().unwrap()["store"], false);
    let rows: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM core_model.responses_continuations WHERE request_id=$1",
    )
    .bind(&request_id)
    .fetch_one(&f.pool)
    .await
    .unwrap();
    assert_eq!(rows, 0);
    let response = request(
        &core,
        &f.key,
        "stateless-parent",
        false,
        json!({"model":"continuity-public","input":"x","previous_response_id":result["id"]}),
    )
    .await
    .unwrap();
    assert_eq!(response.status(), StatusCode::BAD_REQUEST);
    assert_eq!(a.calls.load(Ordering::SeqCst), 2);
    assert_eq!(b.calls.load(Ordering::SeqCst), 0);
    drop(core);
    ta.abort();
    tb.abort();
    rt.cleanup().await;
}

pub(crate) async fn seed_predecessor_charges(installer: &PgPool, control: &PgPool) {
    let f = native_http::test_support::installed_fixture(
        installer.clone(),
        50000,
        &["continuity-public"],
    )
    .await;
    let rt = runtime(control).await;
    let (a, ta) = supplier().await;
    let (b, tb) = supplier().await;
    let core = Core::start(&rt.url, &a, &b, "predecessor", None).await;
    let response = request(
        &core,
        &f.key,
        "prior68-settled-http",
        true,
        body(json!("preserved predecessor")),
    )
    .await
    .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let _ = response.bytes().await.unwrap();
    a.bad.store(true, Ordering::SeqCst);
    let response = request(
        &core,
        &f.key,
        "prior68-unknown-http",
        true,
        body(json!("preserved pending")),
    )
    .await
    .unwrap();
    assert_eq!(response.status(), StatusCode::BAD_GATEWAY);
    let _ = response.bytes().await.unwrap();
    drop(core);
    ta.abort();
    tb.abort();
    rt.cleanup().await;
}

#[sqlx::test(migrations = false)]
async fn continuity_pg_post_commit_ack_loss_uses_actual_single_receipt(pool: PgPool) {
    let f = native_http::test_support::fixture(pool, 10000, &["continuity-public"]).await;
    let rt = runtime(&f.pool).await;
    let (a, ta) = supplier().await;
    let (b, tb) = supplier().await;
    for stage in ["candidate", "settlement", "activation"] {
        let core = Core::start(&rt.url, &a, &b, "normal", Some(&format!("ack:{stage}"))).await;
        let response = request(
            &core,
            &f.key,
            stage,
            false,
            json!({"model":"continuity-public","input":stage,"store":true}),
        )
        .await
        .unwrap();
        assert_eq!(response.status(), StatusCode::OK);
        let id = response.headers()["x-request-id"]
            .to_str()
            .unwrap()
            .to_owned();
        let bytes = response.bytes().await.unwrap();
        packet(&f.pool, &id, &bytes).await;
        let (state, held, cost): (String, i64, i64) =
            sqlx::query_as("SELECT state,reserved,settled FROM core_charging.requests WHERE id=$1")
                .bind(&id)
                .fetch_one(&f.pool)
                .await
                .unwrap();
        assert_eq!((state.as_str(), held, cost), ("settled", 1200, 23));
        let active: bool = sqlx::query_scalar(
            "SELECT active FROM core_model.responses_continuations WHERE request_id=$1",
        )
        .bind(&id)
        .fetch_one(&f.pool)
        .await
        .unwrap();
        assert!(active);
        let retry = request(
            &core,
            &f.key,
            stage,
            false,
            json!({"model":"continuity-public","input":stage,"store":true}),
        )
        .await
        .unwrap();
        assert_eq!(retry.status(), StatusCode::CONFLICT);
        drop(core);
    }
    let captures: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM core_billing.ledger_journals WHERE kind='capture'",
    )
    .fetch_one(&f.pool)
    .await
    .unwrap();
    assert_eq!(captures, 3);
    assert_eq!(a.calls.load(Ordering::SeqCst), 3);
    let balance = f.ledger.balance(f.account).await.unwrap();
    assert_eq!((balance.available_units, balance.reserved_units), (9931, 0));
    ta.abort();
    tb.abort();
    rt.cleanup().await;
}

#[sqlx::test(migrations = false)]
async fn continuity_pg_actual_wire_commit_ack_loss(pool: PgPool) {
    let f = native_http::test_support::fixture(pool, 10000, &["continuity-public"]).await;
    let rt = runtime(&f.pool).await;
    let (a, ta) = supplier().await;
    let (b, tb) = supplier().await;
    for (stage, writer) in [
        ("candidate", "core_model.record_continuation("),
        (
            "settlement",
            "UPDATE core_charging.requests SET state='settled'",
        ),
        ("activation", "SELECT core_model.activate_continuation("),
    ] {
        let proxy = crate::test_pg_commit_fault::PgCommitFault::start(&rt.url, writer).await;
        let core = Core::start(&proxy.url, &a, &b, "normal", None).await;
        let response = request(
            &core,
            &f.key,
            &format!("wire-{stage}"),
            false,
            json!({"model":"continuity-public","input":stage,"store":true}),
        )
        .await
        .unwrap();
        assert_eq!(response.status(), StatusCode::OK);
        let id = response.headers()["x-request-id"]
            .to_str()
            .unwrap()
            .to_owned();
        let bytes = response.bytes().await.unwrap();
        packet(&f.pool, &id, &bytes).await;
        proxy.assert_commit_ack_dropped().await;
        let state: (String, i64, i64) =
            sqlx::query_as("SELECT state,reserved,settled FROM core_charging.requests WHERE id=$1")
                .bind(&id)
                .fetch_one(&f.pool)
                .await
                .unwrap();
        assert_eq!(state, ("settled".into(), 1200, 23));
        let active: bool = sqlx::query_scalar(
            "SELECT active FROM core_model.responses_continuations WHERE request_id=$1",
        )
        .bind(&id)
        .fetch_one(&f.pool)
        .await
        .unwrap();
        assert!(active);
        drop(core);
        drop(proxy);
    }
    let captures: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM core_billing.ledger_journals WHERE kind='capture'",
    )
    .fetch_one(&f.pool)
    .await
    .unwrap();
    assert_eq!(captures, 3);
    assert_eq!(a.calls.load(Ordering::SeqCst), 3);
    let balance = f.ledger.balance(f.account).await.unwrap();
    assert_eq!((balance.available_units, balance.reserved_units), (9931, 0));
    ta.abort();
    tb.abort();
    rt.cleanup().await;
}

#[sqlx::test(migrations = false)]
async fn continuity_pg_controls_false_packet_and_group_denial(pool: PgPool) {
    let f = native_http::test_support::fixture(pool, 10000, &["continuity-public"]).await;
    let rt = runtime(&f.pool).await;
    let (a, ta) = supplier().await;
    let (b, tb) = supplier().await;
    let core = Core::start(&rt.url, &a, &b, "normal", None).await;
    for store in [None, Some(false), Some(true)] {
        let mut input =
            json!({"model":"continuity-public","input":"controls","temperature":0.7,"top_p":0.8});
        if let Some(value) = store {
            input["store"] = value.into();
        }
        let response = request(&core, &f.key, &format!("controls-{store:?}"), false, input)
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::OK);
        let id = response.headers()["x-request-id"]
            .to_str()
            .unwrap()
            .to_owned();
        let bytes = response.bytes().await.unwrap();
        packet(&f.pool, &id, &bytes).await;
        let seen = a.seen.lock().unwrap();
        let wire = seen.last().unwrap();
        assert_eq!(wire["temperature"], 0.7);
        assert_eq!(wire["top_p"], 0.8);
        assert_eq!(wire.get("store").and_then(Value::as_bool), store);
    }
    let calls = a.calls.load(Ordering::SeqCst);
    let before = f.ledger.balance(f.account).await.unwrap();
    let charges: i64 = sqlx::query_scalar("SELECT count(*) FROM core_charging.requests")
        .fetch_one(&f.pool)
        .await
        .unwrap();
    sqlx::query("UPDATE core_identity.user_routing_groups SET enabled=false WHERE user_id=7")
        .execute(&f.pool)
        .await
        .unwrap();
    let response = request(
        &core,
        &f.key,
        "group-revoked",
        false,
        json!({"model":"continuity-public","input":"denied","store":true}),
    )
    .await
    .unwrap();
    assert!(response.status().is_client_error());
    assert_eq!(a.calls.load(Ordering::SeqCst), calls);
    let after = f.ledger.balance(f.account).await.unwrap();
    assert_eq!(
        (after.available_units, after.reserved_units),
        (before.available_units, before.reserved_units)
    );
    assert_eq!(
        sqlx::query_scalar::<_, i64>("SELECT count(*) FROM core_charging.requests")
            .fetch_one(&f.pool)
            .await
            .unwrap(),
        charges
    );

    drop(core);
    ta.abort();
    tb.abort();
    rt.cleanup().await;
}
async fn pagination_case(pool: PgPool, background: bool) {
    use crate::billing::charging::{Outcome, Usage};
    let f = native_http::test_support::fixture(pool, 20000, &["continuity-public"]).await;
    let rt = runtime(&f.pool).await;
    let (a, ta) = supplier().await;
    let (b, tb) = supplier().await;
    for n in 0..4 {
        let mut core = Core::start(&rt.url, &a, &b, "normal", Some("after_candidate")).await;
        assert!(
            request(
                &core,
                &f.key,
                &format!("pagination-{n}"),
                false,
                json!({"model":"continuity-public","input":"page","store":true})
            )
            .await
            .is_err()
        );
        assert_eq!(core.child.wait().unwrap().code(), Some(97));
        drop(core);
    }
    let ids: Vec<String> = sqlx::query_scalar(
        "SELECT request_id FROM core_model.responses_continuations ORDER BY request_id",
    )
    .fetch_all(&f.pool)
    .await
    .unwrap();
    assert_eq!(ids.len(), 4);
    tokio::time::sleep(Duration::from_secs(9)).await;
    let c = WorkerConfig {
        database: rt.url.clone(),
        suppliers: vec![a.base.clone(), b.base.clone()],
        mode: "normal".into(),
        ready: String::new(),
    };
    let g = gateway(&rt.pool, &c);
    g.recover_expired(100).await.unwrap();
    let usage = Usage {
        input: 17,
        output: 3,
        ..Default::default()
    };
    for (i, id) in ids.iter().take(3).enumerate() {
        let row = g.bridge.charging.resolve(id).await.unwrap().unwrap();
        g.bridge
            .charging
            .reconcile(
                id,
                row.revision,
                usage,
                if i < 2 {
                    Outcome::Failed
                } else {
                    Outcome::Completed
                },
                "trusted pagination fixture supplier receipt",
            )
            .await
            .unwrap();
    }
    let balance = f.ledger.balance(f.account).await.unwrap();
    if background {
        for _ in 0..8 {
            g.reconcile_completed_streams(1).await.unwrap();
        }
    } else {
        let mut cursor = None;
        let mut done = false;
        for _ in 0..8 {
            let page = g
                .reconcile_responses_continuity_page(1, cursor.as_ref())
                .await
                .unwrap();
            cursor = page.next;
            if cursor.is_none() {
                done = true;
                break;
            }
        }
        assert!(
            done,
            "independent scans must finish without resetting the shorter scan"
        );
    }
    let active: Vec<bool> = sqlx::query_scalar(
        "SELECT active FROM core_model.responses_continuations ORDER BY request_id",
    )
    .fetch_all(&f.pool)
    .await
    .unwrap();
    assert_eq!(active, vec![false, false, true, true]);
    assert_eq!(a.calls.load(Ordering::SeqCst), 4);
    let final_balance = f.ledger.balance(f.account).await.unwrap();
    assert_eq!(final_balance.reserved_units, 0);
    assert_eq!(final_balance.available_units, 20000 - 23 * 4);
    assert!(balance.reserved_units > 0);
    drop(g);
    ta.abort();
    tb.abort();
    rt.cleanup().await;
}
#[sqlx::test(migrations = false)]
async fn continuity_pg_background_two_cursors_cross_failed_prefix(pool: PgPool) {
    pagination_case(pool, true).await;
}
#[sqlx::test(migrations = false)]
async fn continuity_pg_explicit_two_cursors_cross_failed_prefix(pool: PgPool) {
    pagination_case(pool, false).await;
}

//! A real paid TLS model proposes the private read and a separate math barrier.
//! Extension result content is synthetic; Core identity, storage and money are real.
use super::*;
use crate::{
    assistant_model::{
        AssistantModel, PersonalTextPolicy,
        agent::{
            registry::AgentRegistry,
            store::{AgentPolicy, Completion, ToolCall, TurnSnapshot},
            tests::{gateway_endpoint_group_input_ceiling, response_sse},
        },
    },
    assistant_tools_core::admin_skills,
    billing::{
        Credits,
        ledger::{Action, Ledger, Request},
    },
    identity::{IdentityStore, IssuedCredential},
};
use sqlx::PgPool;
use std::{
    collections::VecDeque,
    sync::{
        Arc, Mutex,
        atomic::{AtomicUsize, Ordering},
    },
    time::Duration,
};

#[path = "confirmed_actions_tls_fixture.rs"]
mod tls_fixture;

const TARGET: i64 = 901;
const MARKER: &str = "synthetic-private-target-strategy";

struct Provider {
    hits: Arc<AtomicUsize>,
    requests: Arc<Mutex<Vec<Value>>>,
    queue: Arc<Mutex<VecDeque<Value>>>,
    task: tokio::task::JoinHandle<()>,
}
impl Drop for Provider {
    fn drop(&mut self) {
        self.task.abort();
    }
}
struct Fixture {
    agent: AgentService,
    session: IssuedCredential,
    provider: Provider,
    _tls: tls_fixture::TlsProxy,
    runtime_role: crate::test_runtime::RuntimeRole,
}

impl Fixture {
    async fn close(self) {
        assert!(self.agent.base.drain(Duration::from_secs(5)).await);
        drop(self.provider);
        drop(self._tls);
        drop(self.agent);
        self.runtime_role.cleanup().await;
    }
}

async fn fixture(pool: &PgPool) -> Fixture {
    fixture_with_tools(pool, &[]).await
}

async fn fixture_with_tools(pool: &PgPool, extra_tools: &[&str]) -> Fixture {
    crate::core_schema::init(pool).await.unwrap();
    let runtime_role = crate::test_runtime::RuntimeRole::create(pool).await;
    for component in [
        "web_identity",
        "core_model",
        "core_charging",
        "core_assistant",
        "core_assistant_agent",
        "core_assistant_tools",
    ] {
        runtime_role.optional(component).await;
    }
    // Only schema, identity bootstrap, funds and catalogue seed use the owner.
    // The actual gateway, Agent and private-read services use runtime authority.
    let identity = IdentityStore::from_pool(pool.clone())
        .with_mfa_keys("test".into(), vec![("test".into(), vec![41; 32])])
        .unwrap();
    let session = identity.bootstrap_user(900, 5).await.unwrap();
    identity.bootstrap_user(TARGET, 0).await.unwrap();
    let actor = identity.authorize(&session.secret).await.unwrap();
    sqlx::query("INSERT INTO core_identity.user_routing_groups(user_id,group_name,enabled,source_reference) VALUES(900,'allowed',TRUE,'synthetic-admin-replay')")
        .execute(pool).await.unwrap();
    let ledger = Ledger::from_pool(pool.clone());
    ledger.open_wallet(actor.owner_account_id).await.unwrap();
    ledger
        .execute(&Request {
            scope: "admin-replay-test".into(),
            operation_key: "synthetic-funds".into(),
            actor_user_id: Some(actor.user_id),
            reason: "Disposable paid replay fixture".into(),
            action: Action::Credit {
                account_id: actor.owner_account_id,
                amount_units: Credits::try_from(400000).unwrap(),
                payment_reference: "synthetic:admin-replay".into(),
            },
        })
        .await
        .unwrap();

    let hits = Arc::new(AtomicUsize::new(0));
    let requests = Arc::new(Mutex::new(Vec::new()));
    let queue = Arc::new(Mutex::new(VecDeque::<Value>::new()));
    let (counter, observed, commands) = (hits.clone(), requests.clone(), queue.clone());
    let app = axum::Router::new().route(
        "/v1/chat/completions",
        axum::routing::post(move |axum::Json(request): axum::Json<Value>| {
            let (counter, observed, commands) = (counter.clone(), observed.clone(), commands.clone());
            async move {
                let streaming = request["stream"] == true;
                observed.lock().unwrap().push(request);
                let n = counter.fetch_add(1, Ordering::SeqCst);
                let message = commands.lock().unwrap().pop_front().expect("unexpected provider request");
                let finish = if message.get("tool_calls").is_some() { "tool_calls" } else { "stop" };
                let response = json!({
                    "id": format!("synthetic-admin-replay-{n}"),
                    "choices": [{"index":0,"finish_reason":finish,"message":message}],
                    "usage":{"prompt_tokens":10,"completion_tokens":20,"prompt_tokens_details":{"cached_tokens":2}}
                });
                if streaming {
                    axum::response::IntoResponse::into_response((
                        [(axum::http::header::CONTENT_TYPE, "text/event-stream")],
                        response_sse(&response),
                    ))
                } else {
                    axum::response::IntoResponse::into_response(axum::Json(response))
                }
            }
        }),
    );
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let backend = format!("http://{}/v1", listener.local_addr().unwrap());
    let task = tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
    let tls = tls_fixture::TlsProxy::start(&backend);
    let gateway =
        gateway_endpoint_group_input_ceiling(&runtime_role.pool, &tls.endpoint, "allowed", 8192);
    let document = gateway.dashboard_routing().catalog_document().unwrap();
    let routing = crate::routing::store::PostgresRoutingStore::new(pool.clone());
    for upstream in &document.routing.upstreams {
        routing
            .register_credential_reference(upstream.credential)
            .await
            .unwrap();
    }
    let version = document.pricing.version;
    routing.stage_prices(document.pricing).await.unwrap();
    routing
        .publish(document.routing, version, None)
        .await
        .unwrap();
    let model = PersonalTextPolicy {
        model: "local-model".into(),
        group: "allowed".into(),
        budget_credits: 10000,
        max_output_tokens: 32,
        ttl_seconds: 300,
        gift_tools: false,
        sponsor: None,
        service_funding: None,
    };
    let mut base = AssistantModel::new(runtime_role.pool.clone(), gateway, model.clone()).unwrap();
    base.transport = base
        .transport
        .with_loopback_test_root(&tls.endpoint, tls.root.clone())
        .unwrap();
    let artifact = include_bytes!(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../../contracts/assistant-tools-v2.json"
    ));
    let registry = AgentRegistry::from_reviewed(
        artifact,
        &crate::hex_codec::encode(Sha256::digest(artifact)),
    )
    .unwrap();
    assert_eq!(
        registry.get("get_admin_user_skills").unwrap().name,
        "get_admin_user_skills"
    );
    let mut names = BTreeSet::from([
        "get_admin_user_skills".to_owned(),
        "calculate_math".to_owned(),
    ]);
    names.extend(extra_tools.iter().map(|name| (*name).to_owned()));
    let agent = AgentService::new(
        base,
        AgentPolicy {
            model,
            max_rounds: 3,
            budget_credits: 30000,
            max_tool_calls: 8,
            ttl_seconds: 300,
            allowed_tools: names.clone(),
            initial_tools: names,
            system_prompt: "Use the actual private skill read, then the separate math result."
                .into(),
            market: None,
        },
        registry,
    )
    .unwrap();
    agent.check_ready().await.unwrap();
    Fixture {
        agent,
        session,
        provider: Provider {
            hits,
            requests,
            queue,
            task,
        },
        _tls: tls,
        runtime_role,
    }
}

async fn wait(f: &Fixture, conversation: &str, phase: &str) -> TurnSnapshot {
    for _ in 0..600 {
        match f.agent.get(&f.session.secret, conversation, "turn1").await {
            Ok(snapshot) if snapshot.phase == phase => return snapshot,
            Ok(snapshot) => assert!(
                !matches!(snapshot.phase.as_str(), "unknown" | "rejected" | "expired"),
                "unexpected phase {}",
                snapshot.phase
            ),
            Err(Error::Conflict) => {}
            Err(error) => panic!("unexpected agent error: {error:?}"),
        }
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    panic!("agent did not reach {phase}");
}

async fn start(f: &Fixture, label: &str) -> (String, String, ToolCall, ToolCall) {
    {
        let mut queue = f.provider.queue.lock().unwrap();
        queue.clear();
        queue.push_back(json!({"role":"assistant","content":null,"tool_calls":[
            {"id":"private_skill","type":"function","function":{"name":"get_admin_user_skills","arguments":format!("{{\"target_user_id\":{TARGET}}}")}},
            {"id":"independent_math","type":"function","function":{"name":"calculate_math","arguments":"{\"expression\":\"1+1\"}"}}
        ]}));
        queue.push_back(json!({"role":"assistant","content":"Synthetic final answer."}));
    }
    let conversation = f
        .agent
        .base
        .conversation(&f.session.secret, label, true)
        .await
        .unwrap()
        .core_conversation_id;
    f.agent
        .start(
            &f.session.secret,
            &conversation,
            "turn1",
            "Read the specified user's skills and then calculate one plus one.",
        )
        .await
        .unwrap();
    let snapshot = wait(f, &conversation, "tools_pending").await;
    assert!(snapshot.settled_credits > 0);
    assert_eq!(snapshot.held_credits, 0);
    assert_eq!(snapshot.pending_calls.len(), 2);
    let mut calls = snapshot.pending_calls;
    let barrier = calls.pop().unwrap();
    let private = calls.pop().unwrap();
    assert_eq!(private.scope.name, "get_admin_user_skills");
    assert_eq!(barrier.scope.name, "calculate_math");
    let turn_id = sqlx::query_scalar("SELECT id FROM core_assistant.agent_turns WHERE conversation_id=$1 AND client_turn_id='turn1'")
        .bind(&conversation).fetch_one(&f.agent.base.pool).await.unwrap();
    assert!(
        f.agent
            .claim(&f.session.secret, &private.id, "original-private-claim")
            .await
            .unwrap()
            .may_execute
    );
    (conversation, turn_id, private, barrier)
}

async fn body(f: &Fixture) -> Vec<u8> {
    let observed = admin_skills::read(&f.agent.base.pool, &f.session.secret, TARGET)
        .await
        .unwrap();
    serde_json::to_vec(&json!({
        "ok":true,
        "target_user_id":TARGET,
        "owner_scope":"current_administrator_self_or_strictly_lower_role",
        "profile":{"profile_key":"technical_cost_sensitive","tags":["advanced"],"strategy":MARKER,"source":"admin","enabled":true,"updated_by":900,"created_at":1,"updated_at":1},
        "memories":[],
        "memories_truncated":false,
        "next_before_id":0,
        "sensitive_values_redacted":true,
        "write_rule":"administrator confirmation required",
        "authorization_binding_sha256":observed.authorization_binding_sha256,
        "checked_at_unix":1
    })).unwrap()
}

async fn accept(f: &Fixture, call: &ToolCall, result: &[u8]) -> Result<(), Error> {
    let receipt = format!("service:{}", call.id);
    f.agent
        .complete(
            &f.session.secret,
            Completion {
                operation_id: &call.id,
                claim_key: "original-private-claim",
                state: "completed",
                result_json: result,
                result_sha256: &Sha256::digest(result),
                domain_receipt_id: &receipt,
            },
        )
        .await
        .map(|_| ())
}

async fn barrier(f: &Fixture, call: &ToolCall) {
    assert!(
        f.agent
            .claim(&f.session.secret, &call.id, "original-math-claim")
            .await
            .unwrap()
            .may_execute
    );
    let result = br#"{"ok":true,"result":2}"#;
    f.agent
        .complete(
            &f.session.secret,
            Completion {
                operation_id: &call.id,
                claim_key: "original-math-claim",
                state: "completed",
                result_json: result,
                result_sha256: &Sha256::digest(result),
                domain_receipt_id: "synthetic-math-receipt",
            },
        )
        .await
        .unwrap();
}

async fn rounds(pool: &PgPool, turn_id: &str) -> i64 {
    sqlx::query_scalar("SELECT count(*) FROM core_assistant.agent_rounds WHERE turn_id=$1")
        .bind(turn_id)
        .fetch_one(pool)
        .await
        .unwrap()
}

#[sqlx::test(migrations = false)]
async fn accepted_private_skills_cannot_start_another_round_after_target_authority_changes(
    pool: PgPool,
) {
    let f = fixture(&pool).await;
    let changes = [
        "UPDATE core_identity.users SET platform_role='admin' WHERE id=901",
        "UPDATE core_identity.users SET active=FALSE WHERE id=901",
        "UPDATE core_identity.accounts SET active=FALSE WHERE id=(SELECT personal_account_id FROM core_identity.users WHERE id=901)",
        "UPDATE core_identity.users SET auth_version=auth_version+1 WHERE id=901",
    ];
    for (index, change) in changes.into_iter().enumerate() {
        sqlx::query("UPDATE core_identity.users SET platform_role='user',active=TRUE WHERE id=901")
            .execute(&pool)
            .await
            .unwrap();
        sqlx::query("UPDATE core_identity.accounts SET active=TRUE WHERE id=(SELECT personal_account_id FROM core_identity.users WHERE id=901)").execute(&pool).await.unwrap();
        let baseline = f.provider.hits.load(Ordering::SeqCst);
        let (cv, turn, private, math) = start(&f, &format!("target-change-{index}")).await;
        let result = body(&f).await;
        accept(&f, &private, &result).await.unwrap();
        assert!(f.agent.base.drain(Duration::from_secs(5)).await);
        let before = f
            .agent
            .get(&f.session.secret, &cv, "turn1")
            .await
            .unwrap()
            .settled_credits;
        sqlx::query(change).execute(&pool).await.unwrap();
        barrier(&f, &math).await;
        let stopped = wait(&f, &cv, "completed").await;
        assert!(
            stopped
                .final_text
                .as_deref()
                .unwrap()
                .contains("已停止后续调用")
        );
        assert_eq!(stopped.settled_credits, before);
        assert_eq!(stopped.held_credits, 0);
        assert_eq!(rounds(&pool, &turn).await, 1);
        assert_eq!(f.provider.hits.load(Ordering::SeqCst), baseline + 1);
        let saved: Vec<u8> = sqlx::query_scalar(
            "SELECT result_json FROM core_assistant.agent_tool_operations WHERE id=$1",
        )
        .bind(&private.id)
        .fetch_one(&pool)
        .await
        .unwrap();
        assert_eq!(saved, result);
        assert!(f.agent.base.drain(Duration::from_secs(5)).await);
    }

    // Current authority permits an older immutable observation; its timestamp
    // is not an artificial expiry. The real second provider request sees it.
    sqlx::query("UPDATE core_identity.users SET platform_role='user',active=TRUE WHERE id=901")
        .execute(&pool)
        .await
        .unwrap();
    sqlx::query("UPDATE core_identity.accounts SET active=TRUE WHERE id=(SELECT personal_account_id FROM core_identity.users WHERE id=901)").execute(&pool).await.unwrap();
    let baseline = f.provider.hits.load(Ordering::SeqCst);
    let (cv, turn, private, math) = start(&f, "unchanged-authority").await;
    accept(&f, &private, &body(&f).await).await.unwrap();
    barrier(&f, &math).await;
    assert_eq!(
        wait(&f, &cv, "completed").await.final_text.as_deref(),
        Some("Synthetic final answer.")
    );
    assert_eq!(rounds(&pool, &turn).await, 2);
    assert_eq!(f.provider.hits.load(Ordering::SeqCst), baseline + 2);
    let request = f.provider.requests.lock().unwrap().last().unwrap().clone();
    assert!(
        request["messages"]
            .as_array()
            .unwrap()
            .iter()
            .any(|message| {
                message["role"] == "tool"
                    && message["content"]
                        .as_str()
                        .is_some_and(|content| content.contains(MARKER))
            })
    );
    f.close().await;
}

#[sqlx::test(migrations = false)]
async fn private_skill_completion_rechecks_authority_before_first_persistence(pool: PgPool) {
    let f = fixture(&pool).await;
    let (_cv, turn, private, _math) = start(&f, "completion-authority").await;
    let result = body(&f).await;
    sqlx::query("UPDATE core_identity.users SET auth_version=auth_version+1 WHERE id=901")
        .execute(&pool)
        .await
        .unwrap();
    assert!(matches!(
        accept(&f, &private, &result).await,
        Err(Error::Forbidden)
    ));
    let state: (String, bool, i64) = sqlx::query_as(
        "SELECT state,result_json IS NULL,(SELECT count(*) FROM core_assistant.agent_tool_observations a WHERE a.operation_id=o.id)
         FROM core_assistant.agent_tool_operations o WHERE id=$1",
    ).bind(&private.id).fetch_one(&pool).await.unwrap();
    assert_eq!(state, ("claimed".into(), true, 0));
    assert_eq!(rounds(&pool, &turn).await, 1);
    assert_eq!(f.provider.hits.load(Ordering::SeqCst), 1);
    f.close().await;
}

#[sqlx::test(migrations = false)]
async fn private_skill_authority_is_rechecked_after_reservation_before_network(pool: PgPool) {
    let f = fixture(&pool).await;
    let (cv, turn, private, math) = start(&f, "network-authority").await;
    accept(&f, &private, &body(&f).await).await.unwrap();
    assert!(f.agent.base.drain(Duration::from_secs(5)).await);
    let permits = f
        .agent
        .base
        .permits
        .clone()
        .acquire_many_owned(u32::try_from(f.agent.base.capacity).unwrap())
        .await
        .unwrap();
    barrier(&f, &math).await;
    let work = f
        .agent
        .reserve_round(&f.session.secret, &turn)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(rounds(&pool, &turn).await, 2);
    assert_eq!(f.provider.hits.load(Ordering::SeqCst), 1);
    sqlx::query("UPDATE core_identity.users SET platform_role='admin' WHERE id=901")
        .execute(&pool)
        .await
        .unwrap();
    assert!(matches!(
        f.agent
            .check_model_authority(&f.session.secret, &turn)
            .await,
        Err(Error::Forbidden)
    ));
    // Execute the same conservative failure branch used by run_turn before
    // dispatch; this is a deterministic PG gate test, not a timing race.
    f.agent
        .unknown_round(&turn, &work.request_id, &work.round_id, &work.worker)
        .await
        .unwrap();
    drop(permits);
    let snapshot = f.agent.get(&f.session.secret, &cv, "turn1").await.unwrap();
    assert_eq!(snapshot.phase, "unknown");
    assert_eq!(f.provider.hits.load(Ordering::SeqCst), 1);
    let charging: String =
        sqlx::query_scalar("SELECT state FROM core_charging.requests WHERE id=$1")
            .bind(work.request_id)
            .fetch_one(&pool)
            .await
            .unwrap();
    assert_eq!(charging, "reconcile");
    let cancelled = f
        .agent
        .cancel(&f.session.secret, &cv, "turn1")
        .await
        .unwrap();
    assert_eq!(cancelled.phase, "unknown");
    assert!(matches!(
        f.agent
            .start(
                &f.session.secret,
                &cv,
                "turn2",
                "Do not bypass an unsettled model."
            )
            .await,
        Err(Error::Conflict)
    ));
    assert_eq!(f.provider.hits.load(Ordering::SeqCst), 1);
    f.close().await;
}

#[sqlx::test(migrations = false)]
async fn unknown_private_skill_payloads_are_not_reused_as_model_context(pool: PgPool) {
    let f = fixture(&pool).await;
    let (_cv, turn, private, _math) = start(&f, "unknown-private-result").await;
    let result =
        br#"{"status":"unknown","target_user_id":null,"authorization_binding_sha256":false}"#;
    f.agent
        .complete(
            &f.session.secret,
            Completion {
                operation_id: &private.id,
                claim_key: "original-private-claim",
                state: "unknown",
                result_json: result,
                result_sha256: &Sha256::digest(result),
                domain_receipt_id: "",
            },
        )
        .await
        .unwrap();
    f.agent
        .check_model_authority(&f.session.secret, &turn)
        .await
        .unwrap();
    assert!(
        f.agent
            .reserve_round(&f.session.secret, &turn)
            .await
            .unwrap()
            .is_none()
    );
    assert_eq!(rounds(&pool, &turn).await, 1);
    assert_eq!(f.provider.hits.load(Ordering::SeqCst), 1);
    f.close().await;
}

#[path = "admin_skill_cancel_pg_tests.rs"]
mod cancel_tests;

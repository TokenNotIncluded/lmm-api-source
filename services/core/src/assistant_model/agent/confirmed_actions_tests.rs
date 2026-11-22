//! Real paid Agent rounds, native HttpOnly Session and TCP confirmation API.
//! The only upstream is a counted loopback fixture. No external model is used.
use super::*;
use crate::{
    assistant_tools_core::{
        actions,
        confirmation::{self, BrowserConfirmation},
    },
    identity::{
        BrowserAuth, BrowserBundle, BrowserRegistration, BrowserSessionMetadata, IdentityStore,
    },
};
use sqlx::Row;

#[path = "confirmed_actions_tls_fixture.rs"]
mod tls_fixture;

// One counted provider round; None produces its final text completion.
type ModelCommand = Option<(String, String)>;
type ModelCommandQueue = Arc<Mutex<std::collections::VecDeque<ModelCommand>>>;

struct ConfirmFixture {
    agent: AgentService,
    identity: IdentityStore,
    browser: BrowserAuth,
    bundle: BrowserBundle,
    user: i64,
    account: i64,
    provider: Provider,
    tls_proxy: tls_fixture::TlsProxy,
    queue: ModelCommandQueue,
    origin: String,
    http: reqwest::Client,
    task: tokio::task::JoinHandle<()>,
}
impl ConfirmFixture {
    fn access(&self) -> &str {
        self.bundle.data["access_token"].as_str().unwrap()
    }
    fn cookie(&self) -> String {
        format!("new_api_refresh={}", self.bundle.refresh_token)
    }
    async fn close(self) {
        assert!(self.agent.base.drain(Duration::from_secs(5)).await);
        self.task.abort();
        assert!(self.task.await.unwrap_err().is_cancelled());
        drop(self.provider);
        drop(self.tls_proxy);
    }
    async fn prepare(&self, name: &str, args: Value, text: &str, list_first: bool) -> String {
        {
            let mut queue = self.queue.lock().unwrap();
            if list_first {
                queue.push_back(Some(("list_my_api_keys".into(), "{}".into())));
            }
            queue.push_back(Some((name.into(), serde_json::to_string(&args).unwrap())));
            queue.push_back(None);
        }
        let cv = self
            .agent
            .base
            .conversation(
                self.access(),
                &crate::assistant_model::random("confirmation"),
                true,
            )
            .await
            .unwrap()
            .core_conversation_id;
        self.agent
            .start(self.access(), &cv, "turn1", text)
            .await
            .unwrap();
        let snapshot = wait(&self.agent, self.access(), &cv, "completed").await;
        assert!(snapshot.final_text.is_some());
        assert!(snapshot.settled_credits > 0);
        let row=sqlx::query("SELECT c.id,o.result_json FROM core_assistant_tools.confirmations c JOIN core_assistant.agent_tool_operations o ON o.id=c.operation_id JOIN core_assistant.agent_turns t ON t.id=o.turn_id WHERE t.conversation_id=$1")
            .bind(cv).fetch_one(&self.agent.base.pool).await.unwrap();
        let original: Value =
            serde_json::from_slice(&row.get::<Vec<u8>, _>("result_json")).unwrap();
        assert_eq!(original["executed"], false);
        assert_eq!(original["status"], "prepared");
        row.get("id")
    }
    async fn inspect(&self, id: &str) -> (reqwest::StatusCode, Value, Option<String>) {
        let response = self
            .http
            .get(format!(
                "{}/api/user/auth/assistant/confirmations/{id}",
                self.origin
            ))
            .header("cookie", self.cookie())
            .header("accept", "application/json")
            .send()
            .await
            .unwrap();
        let code = response.status();
        let cookie = response
            .headers()
            .get("set-cookie")
            .map(|v| v.to_str().unwrap().split(';').next().unwrap().to_owned());
        let value = response.json().await.unwrap();
        (code, value, cookie)
    }
    async fn post(
        &self,
        id: &str,
        csrf: &str,
        nonce: &str,
        decision: Value,
    ) -> (reqwest::StatusCode, Value) {
        let response = self
            .http
            .post(format!(
                "{}/api/user/auth/assistant/confirmations/{id}",
                self.origin
            ))
            .header("origin", &self.origin)
            .header("cookie", format!("{}; {nonce}", self.cookie()))
            .header("x-lmm-assistant-csrf", csrf)
            .json(&decision)
            .send()
            .await
            .unwrap();
        let code = response.status();
        (code, response.json().await.unwrap())
    }
}
async fn fixture(pool: &PgPool, ttl: i64) -> ConfirmFixture {
    fixture_options(pool, ttl, false, false).await
}
async fn fixture_market(pool: &PgPool, ttl: i64) -> ConfirmFixture {
    fixture_options(pool, ttl, true, false).await
}
async fn fixture_options(
    pool: &PgPool,
    ttl: i64,
    market_enabled: bool,
    invitation_enabled: bool,
) -> ConfirmFixture {
    assert!(!(market_enabled && invitation_enabled));
    crate::core_schema::init(pool).await.unwrap();
    let identity = IdentityStore::from_pool(pool.clone())
        .with_registration_enabled(true)
        .with_api_key_encryption_key(&[46; 32])
        .unwrap()
        .with_mfa_keys("test".into(), vec![("test".into(), vec![44; 32])])
        .unwrap();
    let browser = BrowserAuth::new(identity.clone(), &[45; 32]).unwrap();
    browser
        .register(&BrowserRegistration {
            username: "confirm-user",
            password: "test-password123",
            email: "",
        })
        .await
        .unwrap();
    let bundle = browser
        .login(
            "confirm-user",
            "test-password123",
            &BrowserSessionMetadata::default(),
        )
        .await
        .unwrap();
    let user = bundle.data["user"]["id"].as_i64().unwrap();
    let principal = identity
        .authorize(bundle.data["access_token"].as_str().unwrap())
        .await
        .unwrap();
    let account = principal.owner_account_id;
    sqlx::query("INSERT INTO core_identity.user_routing_groups(user_id,group_name,enabled,source_reference) VALUES($1,'allowed',TRUE,'synthetic-confirmation')")
        .bind(user).execute(pool).await.unwrap();
    let ledger = crate::billing::ledger::Ledger::from_pool(pool.clone());
    ledger.open_wallet(account).await.unwrap();
    ledger
        .execute(&crate::billing::ledger::Request {
            scope: "confirmation-test".into(),
            operation_key: "fund".into(),
            actor_user_id: Some(user),
            reason: "Disposable synthetic funds".into(),
            action: crate::billing::ledger::Action::Credit {
                account_id: account,
                amount_units: crate::billing::Credits::try_from(200000).unwrap(),
                payment_reference: "synthetic:confirmation".into(),
            },
        })
        .await
        .unwrap();
    let queue = Arc::new(Mutex::new(std::collections::VecDeque::<ModelCommand>::new()));
    let observed = Arc::new(Mutex::new(Vec::new()));
    let hits = Arc::new(AtomicUsize::new(0));
    let (commands, requests, counter) = (queue.clone(), observed.clone(), hits.clone());
    let app=axum::Router::new().route("/v1/chat/completions",axum::routing::post(
        move|axum::Json(request):axum::Json<Value>|{
            let (commands,requests,counter)=(commands.clone(),requests.clone(),counter.clone());
            async move{
                requests.lock().unwrap().push(request.clone());
                let n=counter.fetch_add(1,Ordering::SeqCst);
                let command=commands.lock().unwrap().pop_front().expect("unexpected model request");
                let (finish,message)=match command {
                    Some((name,args))=>("tool_calls",json!({"role":"assistant","content":null,"tool_calls":[
                        {"id":format!("call_{n}"),"type":"function","function":{"name":name,"arguments":args}}]})),
                    None=>("stop",json!({"role":"assistant","content":"The requested preview is ready. Use its browser confirmation; no action has executed."})),
                };
                let response=json!({"id":format!("confirm-model-{n}"),"choices":[{"index":0,"finish_reason":finish,"message":message}],
                    "usage":{"prompt_tokens":10,"completion_tokens":20,"prompt_tokens_details":{"cached_tokens":2}}});
                if request["stream"]==true {
                    axum::response::IntoResponse::into_response(([(axum::http::header::CONTENT_TYPE,"text/event-stream")],response_sse(&response)))
                } else {axum::response::IntoResponse::into_response(axum::Json(response))}
            }
        }));
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let endpoint = format!("http://{}/v1", listener.local_addr().unwrap());
    let task = tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
    let tls_proxy = tls_fixture::TlsProxy::start(&endpoint);
    let provider = Provider {
        endpoint: tls_proxy.endpoint.clone(),
        hits,
        requests: observed,
        task,
    };
    let gateway = gateway_endpoint_group_input_ceiling(pool, &provider.endpoint, "allowed", 8192);
    let document = gateway.dashboard_routing().catalog_document().unwrap();
    let routing_store = crate::routing::store::PostgresRoutingStore::new(pool.clone());
    for upstream in &document.routing.upstreams {
        routing_store
            .register_credential_reference(upstream.credential)
            .await
            .unwrap();
    }
    let price_version = document.pricing.version;
    routing_store.stage_prices(document.pricing).await.unwrap();
    routing_store
        .publish(document.routing, price_version, None)
        .await
        .unwrap();
    let identity = identity.with_dashboard_routing(gateway.dashboard_routing());
    let model = PersonalTextPolicy {
        model: "local-model".into(),
        group: "allowed".into(),
        budget_credits: 10000,
        max_output_tokens: 32,
        ttl_seconds: ttl,
        gift_tools: false,
        sponsor: None,
        service_funding: None,
    };
    let mut base = AssistantModel::new(pool.clone(), gateway, model.clone()).unwrap();
    base.transport = base
        .transport
        .with_loopback_test_root(&provider.endpoint, tls_proxy.root.clone())
        .unwrap();
    let existing: Value = serde_json::from_slice(include_bytes!(
        "../../../../../contracts/assistant-tools-v2.json"
    ))
    .unwrap();
    let mut definitions: Vec<Value> = existing["tools"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|t| {
            if invitation_enabled {
                return t["name"] == "send_invitation";
            }
            t["name"] == "list_my_api_keys"
                || (market_enabled
                    && matches!(
                        t["name"].as_str(),
                        Some(
                            "connect_market_tool"
                                | "call_market_tool"
                                | "get_connected_market_tools"
                        )
                    ))
        })
        .cloned()
        .collect();
    assert_eq!(definitions.len(), if market_enabled { 4 } else { 1 });
    for (name, schema, version) in [
        (actions::KEY_TOOL, actions::KEY_SCHEMA, actions::VERSION),
        (
            actions::PROFILE_TOOL,
            actions::PROFILE_SCHEMA,
            actions::VERSION,
        ),
        (
            actions::CREATE_TOOL,
            actions::CREATE_SCHEMA,
            actions::CREATE_VERSION,
        ),
    ] {
        if invitation_enabled {
            continue;
        }
        definitions.push(json!({"name":name,"description":"Prepare one exact self action for explicit browser confirmation.",
            "version":version,"schema_json":schema,"schema_sha256":crate::hex_codec::encode(Sha256::digest(schema.as_bytes())),
            "price_version":"builtin-free-v1","price_sha256":"5d6fb460c679922a1976e87ef729440d5394187e9b12331f33faf12904a64192","domain":"registered_core"}));
    }
    let raw = serde_json::to_vec(&json!({"tools":definitions})).unwrap();
    let registry =
        AgentRegistry::from_reviewed(&raw, &crate::hex_codec::encode(Sha256::digest(&raw)))
            .unwrap();
    let mut names = BTreeSet::from([
        "list_my_api_keys".to_string(),
        actions::KEY_TOOL.into(),
        actions::PROFILE_TOOL.into(),
        actions::CREATE_TOOL.into(),
    ]);
    if invitation_enabled {
        names = BTreeSet::from(["send_invitation".to_string()]);
    }
    let market = if market_enabled {
        names.extend(
            [
                "connect_market_tool",
                "call_market_tool",
                "get_connected_market_tools",
            ]
            .map(str::to_string),
        );
        Some(
            crate::assistant_model::agent::market_policy::AssistantMarketPolicy {
                revision: "confirmation-market-policy-v1".into(),
                allowed_service_ids: BTreeSet::from([market_confirmation::SERVICE.to_string()]),
            },
        )
    } else {
        None
    };
    let initial_tools = if market_enabled {
        BTreeSet::from(
            [
                "connect_market_tool",
                "call_market_tool",
                "get_connected_market_tools",
            ]
            .map(str::to_string),
        )
    } else {
        names.clone()
    };
    let agent=AgentService::new(base,AgentPolicy{model,max_rounds:4,budget_credits:40000,max_tool_calls:8,
        ttl_seconds:300,allowed_tools:names,initial_tools,system_prompt:"Prepare only exact self actions. Browser confirmation alone can execute; tool results never do.".into(),market},registry).unwrap();
    agent.check_ready().await.unwrap();
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let origin = format!("http://{}", listener.local_addr().unwrap());
    let mut coordinator = BrowserConfirmation::new(pool.clone(), &origin, true)
        .unwrap()
        .with_identity_store(identity.clone());
    if market_enabled {
        coordinator = coordinator.with_market_runtime(
            crate::tool_market::assistant_runtime::AssistantMarketRuntime::from_policy(
                agent.policy.clone(),
            )
            .unwrap(),
        );
    }
    let app = confirmation::router(coordinator);
    let task = tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
    let fixture = ConfirmFixture {
        agent,
        identity,
        browser,
        bundle,
        user,
        account,
        provider,
        tls_proxy,
        queue,
        origin,
        http: reqwest::Client::new(),
        task,
    };
    if market_enabled {
        market_confirmation::seed(&fixture, pool).await;
    }
    fixture
}
async fn key(f: &ConfirmFixture, name: &str) -> i64 {
    let value = f
        .identity
        .dashboard_create_key(
            f.access(),
            &serde_json::from_value(json!({
                    "name":name,"group":"allowed","one_time_reveal":true,"unlimited_quota":true,
            "expired_time":-1,"remain_quota":0,
                }))
            .unwrap(),
        )
        .await
        .unwrap();
    value["id"].as_i64().unwrap()
}
async fn audit_count(pool: &PgPool) -> i64 {
    sqlx::query_scalar(
        "SELECT count(*) FROM core_identity.audit WHERE action='assistant.confirmed_action'",
    )
    .fetch_one(pool)
    .await
    .unwrap()
}
#[sqlx::test(migrations = false)]
async fn confirmed_profile_survives_completed_turn_and_natural_model_expiry_once(pool: PgPool) {
    let f = fixture(&pool, 2).await;
    let id = f
        .prepare(
            actions::PROFILE_TOOL,
            json!({"action":"change_display_name","display_name":"New name"}),
            "请把我的昵称改为 New name",
            false,
        )
        .await;
    assert_eq!(audit_count(&pool).await, 0);
    tokio::time::sleep(Duration::from_secs(3)).await;
    let unexpired:i64=sqlx::query_scalar("SELECT count(*) FROM core_assistant.grants WHERE expires_at>extract(epoch FROM clock_timestamp())::bigint")
        .fetch_one(&pool).await.unwrap();
    assert_eq!(
        unexpired, 0,
        "model grants actually expired before browser approval"
    );
    let (status, view, nonce) = f.inspect(&id).await;
    assert_eq!(status, reqwest::StatusCode::OK);
    assert_eq!(view["confirmation"]["executed"], false);
    let nonce = nonce.unwrap();
    let csrf = view["csrf"].as_str().unwrap();
    let (a, b) = tokio::join!(
        f.post(&id, csrf, &nonce, json!({"decision":"approve"})),
        f.post(&id, csrf, &nonce, json!({"decision":"approve"}))
    );
    assert_eq!(a.0, reqwest::StatusCode::OK);
    assert_eq!(b.0, reqwest::StatusCode::OK);
    assert_eq!(a.1["receipt"], b.1["receipt"]);
    assert_eq!(a.1["executed"], true);
    assert_eq!(
        a.1["receipt"]["execution_receipt"]["after"]["display_name"],
        "New name"
    );
    assert_eq!(a.1["receipt"]["execution_receipt"]["account_id"], f.account);
    assert_eq!(audit_count(&pool).await, 1);
    let current: String =
        sqlx::query_scalar("SELECT display_name FROM core_identity.user_profiles WHERE user_id=$1")
            .bind(f.user)
            .fetch_one(&pool)
            .await
            .unwrap();
    assert_eq!(current, "New name");
    let immutable:Value=serde_json::from_slice(&sqlx::query_scalar::<_,Vec<u8>>(
        "SELECT o.result_json FROM core_assistant.agent_tool_operations o JOIN core_assistant_tools.confirmations q ON q.operation_id=o.id WHERE q.id=$1"
    ).bind(&id).fetch_one(&pool).await.unwrap()).unwrap();
    assert_eq!(immutable["executed"], false);
    assert_eq!(immutable["status"], "prepared");
    assert_eq!(f.provider.hits.load(Ordering::SeqCst), 2);
    assert_eq!(f.inspect(&id).await.1["confirmation"], a.1["receipt"]);
    f.close().await;
}

#[sqlx::test(migrations = false)]
async fn confirmed_key_disable_delete_preserve_exact_owner_and_private_material(pool: PgPool) {
    let f = fixture(&pool, 300).await;
    let first = key(&f, "first").await;
    let second = key(&f, "second").await;
    let before: Vec<u8> =
        sqlx::query_scalar("SELECT digest FROM core_identity.credentials WHERE id=$1")
            .bind(first)
            .fetch_one(&pool)
            .await
            .unwrap();
    for (target, action) in [(first, "disable"), (second, "delete")] {
        let id = f
            .prepare(
                actions::KEY_TOOL,
                json!({"action":action,"token_id":target}),
                &format!("请{action} ID: {target}"),
                true,
            )
            .await;
        let (_, view, nonce) = f.inspect(&id).await;
        let (status, receipt) = f
            .post(
                &id,
                view["csrf"].as_str().unwrap(),
                &nonce.unwrap(),
                json!({"decision":"approve"}),
            )
            .await;
        assert_eq!(status, reqwest::StatusCode::OK);
        assert_eq!(receipt["executed"], true);
        assert_eq!(
            receipt["receipt"]["execution_receipt"]["resource_id"],
            target
        );
        assert!(!receipt.to_string().contains("lmmk_"));
        assert!(!receipt.to_string().contains("encrypted_secret"));
        assert_eq!(f.inspect(&id).await.1["confirmation"], receipt["receipt"]);
    }
    assert_eq!(audit_count(&pool).await, 2);
    assert!(
        !sqlx::query_scalar::<_, bool>(
            "SELECT enabled FROM core_identity.api_key_settings WHERE credential_id=$1"
        )
        .bind(first)
        .fetch_one(&pool)
        .await
        .unwrap()
    );
    assert!(
        sqlx::query_scalar::<_, bool>(
            "SELECT revoked_at IS NOT NULL FROM core_identity.credentials WHERE id=$1"
        )
        .bind(second)
        .fetch_one(&pool)
        .await
        .unwrap()
    );
    let after: Vec<u8> =
        sqlx::query_scalar("SELECT digest FROM core_identity.credentials WHERE id=$1")
            .bind(first)
            .fetch_one(&pool)
            .await
            .unwrap();
    assert_eq!(before, after);
    assert_eq!(f.provider.hits.load(Ordering::SeqCst), 6);
    f.close().await;
}

async fn propose_denied(f: &ConfirmFixture, args: Value, text: &str, list_first: bool) {
    {
        let mut queue = f.queue.lock().unwrap();
        if list_first {
            queue.push_back(Some(("list_my_api_keys".into(), "{}".into())));
        }
        queue.push_back(Some((
            actions::KEY_TOOL.into(),
            serde_json::to_string(&args).unwrap(),
        )));
        queue.push_back(None);
    }
    let cv = f
        .agent
        .base
        .conversation(f.access(), &crate::assistant_model::random("denied"), true)
        .await
        .unwrap()
        .core_conversation_id;
    f.agent.start(f.access(), &cv, "turn1", text).await.unwrap();
    wait(&f.agent, f.access(), &cv, "completed").await;
    let count:i64=sqlx::query_scalar("SELECT count(*) FROM core_assistant_tools.confirmations q JOIN core_assistant.agent_turns t ON t.id=q.turn_id WHERE t.conversation_id=$1")
        .bind(&cv).fetch_one(&f.agent.base.pool).await.unwrap();
    assert_eq!(count, 0);
    let result:Vec<u8>=sqlx::query_scalar("SELECT o.result_json FROM core_assistant.agent_tool_operations o JOIN core_assistant.agent_turns t ON t.id=o.turn_id WHERE t.conversation_id=$1 AND o.tool_name='prepare_api_key_action'")
        .bind(&cv).fetch_one(&f.agent.base.pool).await.unwrap();
    let result: Value = serde_json::from_slice(&result).unwrap();
    assert_eq!(result["status"], "not_prepared");
    assert_eq!(result["executed"], false);
}
#[sqlx::test(migrations = false)]
async fn confirmed_actions_reject_missing_selection_stale_resources_and_revocation(pool: PgPool) {
    let f = fixture(&pool, 300).await;
    let target = key(&f, "duplicate").await;
    let _other = key(&f, "duplicate").await;
    propose_denied(
        &f,
        json!({"action":"delete","token_id":target}),
        &format!("delete ID: {target}"),
        false,
    )
    .await;
    propose_denied(
        &f,
        json!({"action":"delete","token_id":target}),
        "delete duplicate",
        true,
    )
    .await;
    propose_denied(
        &f,
        json!({"action":"delete","token_id":target}),
        &format!("不要删除 ID: {target}"),
        true,
    )
    .await;
    assert_eq!(audit_count(&pool).await, 0);

    let stale = f
        .prepare(
            actions::PROFILE_TOOL,
            json!({"action":"change_display_name","display_name":"Prepared"}),
            "change my name to Prepared",
            false,
        )
        .await;
    let (_, view, nonce) = f.inspect(&stale).await;
    let nonce = nonce.unwrap();
    let csrf = view["csrf"].as_str().unwrap();
    let url = format!("{}/api/user/auth/assistant/confirmations/{stale}", f.origin);
    let missing = f
        .http
        .post(&url)
        .header("origin", &f.origin)
        .header("x-lmm-assistant-csrf", csrf)
        .json(&json!({"decision":"approve"}))
        .send()
        .await
        .unwrap();
    assert!(matches!(missing.status().as_u16(), 401 | 403));
    let foreign = f
        .http
        .post(&url)
        .header("origin", "https://other.invalid")
        .header("cookie", format!("{}; {nonce}", f.cookie()))
        .header("x-lmm-assistant-csrf", csrf)
        .json(&json!({"decision":"approve"}))
        .send()
        .await
        .unwrap();
    assert_eq!(foreign.status(), reqwest::StatusCode::FORBIDDEN);
    sqlx::query(
        "UPDATE core_identity.user_profiles SET display_name='Changed separately' WHERE user_id=$1",
    )
    .bind(f.user)
    .execute(&pool)
    .await
    .unwrap();
    assert_eq!(
        f.post(&stale, csrf, &nonce, json!({"decision":"approve"}))
            .await
            .0,
        reqwest::StatusCode::CONFLICT
    );
    assert_eq!(audit_count(&pool).await, 0);

    let rejected = f
        .prepare(
            actions::PROFILE_TOOL,
            json!({"action":"change_display_name","display_name":"Rejected"}),
            "change name to Rejected",
            false,
        )
        .await;
    let (_, view, nonce) = f.inspect(&rejected).await;
    let nonce = nonce.unwrap();
    let csrf = view["csrf"].as_str().unwrap();
    assert_eq!(
        f.post(&rejected, csrf, &nonce, json!({"decision":"reject"}))
            .await
            .1["executed"],
        false
    );
    assert_eq!(
        f.post(&rejected, csrf, &nonce, json!({"decision":"approve"}))
            .await
            .0,
        reqwest::StatusCode::CONFLICT
    );

    let revoked = f
        .prepare(
            actions::PROFILE_TOOL,
            json!({"action":"change_display_name","display_name":"Revoked"}),
            "change name to Revoked",
            false,
        )
        .await;
    let (_, view, nonce) = f.inspect(&revoked).await;
    let grant:String=sqlx::query_scalar("SELECT paid.assistant_grant_id FROM core_assistant_tools.confirmations q JOIN core_assistant.agent_tool_operations o ON o.id=q.operation_id JOIN core_assistant.agent_rounds r ON r.id=o.round_id JOIN core_charging.requests paid ON paid.id=r.model_request_id WHERE q.id=$1")
        .bind(&revoked).fetch_one(&pool).await.unwrap();
    f.agent.base.revoke(f.access(), &grant).await.unwrap();
    assert_eq!(
        f.post(
            &revoked,
            view["csrf"].as_str().unwrap(),
            &nonce.unwrap(),
            json!({"decision":"approve"})
        )
        .await
        .0,
        reqwest::StatusCode::FORBIDDEN
    );
    assert_eq!(audit_count(&pool).await, 0);
    assert_eq!(
        sqlx::query_scalar::<_, String>(
            "SELECT display_name FROM core_identity.user_profiles WHERE user_id=$1"
        )
        .bind(f.user)
        .fetch_one(&pool)
        .await
        .unwrap(),
        "Changed separately"
    );
    f.close().await;
}
fn code_at(secret: &str, epoch: u64) -> String {
    use totp_rs::{Algorithm, Builder, Secret};
    let secret = Secret::try_from_base32(secret).unwrap();
    Builder::new()
        .with_algorithm(Algorithm::SHA1)
        .with_digits(6)
        .with_step_duration(30)
        .with_skew(1)
        .with_secret(secret.as_bytes())
        .build()
        .unwrap()
        .generate(epoch)
        .to_string()
}
async fn epoch(pool: &PgPool) -> u64 {
    u64::try_from(
        sqlx::query_scalar::<_, i64>("SELECT floor(extract(epoch FROM clock_timestamp()))::bigint")
            .fetch_one(pool)
            .await
            .unwrap(),
    )
    .unwrap()
}
#[sqlx::test(migrations = false)]
async fn confirmed_key_mfa_attempts_are_durable_and_secrets_never_enter_model_receipts(
    pool: PgPool,
) {
    let mut f = fixture(&pool, 300).await;
    let setup = f.browser.mfa_setup(f.access()).await.unwrap();
    let enrollment_code = code_at(&setup.secret, epoch(&pool).await);
    f.bundle = f
        .browser
        .mfa_enable(f.access(), &enrollment_code)
        .await
        .unwrap();
    let target = key(&f, "factor-key").await;
    let id = f
        .prepare(
            actions::KEY_TOOL,
            json!({"action":"disable","token_id":target}),
            &format!("disable ID: {target}"),
            true,
        )
        .await;
    let (_, view, nonce) = f.inspect(&id).await;
    let nonce = nonce.unwrap();
    let csrf = view["csrf"].as_str().unwrap();
    assert_eq!(view["confirmation"]["requires_two_factor"], true);
    let bad = f
        .post(
            &id,
            csrf,
            &nonce,
            json!({"decision":"approve","two_factor_code":"xxxxxx"}),
        )
        .await;
    assert_eq!(bad.1["error"], "two_factor_code_invalid");
    assert_eq!(bad.1["executed"], false);
    assert_eq!(
        sqlx::query_scalar::<_, i32>(
            "SELECT failed_attempts FROM core_identity.mfa_factors WHERE user_id=$1"
        )
        .bind(f.user)
        .fetch_one(&pool)
        .await
        .unwrap(),
        1
    );
    assert_eq!(audit_count(&pool).await, 0);
    assert!(
        sqlx::query_scalar::<_, bool>(
            "SELECT enabled FROM core_identity.api_key_settings WHERE credential_id=$1"
        )
        .bind(target)
        .fetch_one(&pool)
        .await
        .unwrap()
    );
    let current = code_at(&setup.secret, epoch(&pool).await + 30);
    let good = f
        .post(
            &id,
            csrf,
            &nonce,
            json!({"decision":"approve","two_factor_code":current}),
        )
        .await;
    assert_eq!(good.0, reqwest::StatusCode::OK);
    assert_eq!(good.1["executed"], true);
    let again = f
        .post(&id, csrf, &nonce, json!({"decision":"approve"}))
        .await;
    assert_eq!(again.1["receipt"], good.1["receipt"]);
    assert_eq!(audit_count(&pool).await, 1);
    let second = key(&f, "second-factor-key").await;
    let next = f
        .prepare(
            actions::KEY_TOOL,
            json!({"action":"delete","token_id":second}),
            &format!("delete ID: {second}"),
            true,
        )
        .await;
    let (_, view, nonce) = f.inspect(&next).await;
    let nonce = nonce.unwrap();
    let csrf = view["csrf"].as_str().unwrap();
    assert_eq!(
        f.post(
            &next,
            csrf,
            &nonce,
            json!({"decision":"approve","two_factor_code":current})
        )
        .await
        .1["error"],
        "two_factor_code_invalid"
    );
    for attempt in 2..=5 {
        let denied = f
            .post(
                &next,
                csrf,
                &nonce,
                json!({"decision":"approve","two_factor_code":"xxxxxx"}),
            )
            .await;
        assert_eq!(
            denied.1["error"],
            if attempt == 5 {
                "two_factor_locked"
            } else {
                "two_factor_code_invalid"
            }
        );
    }
    assert_eq!(audit_count(&pool).await, 1);
    assert!(
        !sqlx::query_scalar::<_, bool>(
            "SELECT revoked_at IS NOT NULL FROM core_identity.credentials WHERE id=$1"
        )
        .bind(second)
        .fetch_one(&pool)
        .await
        .unwrap()
    );
    assert_eq!(
        sqlx::query_scalar::<_, i32>(
            "SELECT failed_attempts FROM core_identity.mfa_factors WHERE user_id=$1"
        )
        .bind(f.user)
        .fetch_one(&pool)
        .await
        .unwrap(),
        5
    );
    let model_input = serde_json::to_string(&*f.provider.requests.lock().unwrap()).unwrap();
    assert!(!model_input.contains("two_factor_code"));
    assert!(!model_input.contains("encrypted_secret"));
    f.close().await;
}
#[sqlx::test(migrations = false)]
async fn confirmed_execution_requires_one_bound_audit_and_terminal_receipt_is_immutable(
    pool: PgPool,
) {
    let f = fixture(&pool, 300).await;
    let id = f
        .prepare(
            actions::PROFILE_TOOL,
            json!({"action":"change_display_name","display_name":"Approved"}),
            "change name to Approved",
            false,
        )
        .await;
    let (_, view, nonce) = f.inspect(&id).await;
    let nonce = nonce.unwrap();
    let csrf = view["csrf"].as_str().unwrap();
    let mut forged = pool.begin().await.unwrap();
    let error=sqlx::query("UPDATE core_assistant_tools.confirmations SET state='approved',confirmed_at=extract(epoch FROM clock_timestamp())::bigint WHERE id=$1")
        .bind(&id).execute(&mut *forged).await.unwrap_err();
    assert_eq!(
        error.as_database_error().and_then(|e| e.code()).as_deref(),
        Some("23514")
    );
    forged.rollback().await.unwrap();
    let approved = f
        .post(&id, csrf, &nonce, json!({"decision":"approve"}))
        .await;
    assert_eq!(approved.1["executed"], true);
    for sql in [
        "UPDATE core_assistant_tools.confirmations SET execution_receipt=execution_receipt WHERE id=$1",
        "DELETE FROM core_assistant_tools.confirmations WHERE id=$1",
    ] {
        let error = sqlx::query(sqlx::AssertSqlSafe(sql))
            .bind(&id)
            .execute(&pool)
            .await
            .unwrap_err();
        assert_eq!(
            error.as_database_error().and_then(|e| e.code()).as_deref(),
            Some("23514")
        );
    }
    assert_eq!(audit_count(&pool).await, 1);
    assert_eq!(
        f.post(&id, csrf, &nonce, json!({"decision":"approve"}))
            .await
            .1["receipt"],
        approved.1["receipt"]
    );
    f.close().await;
}

#[path = "confirmed_actions_creation_tests.rs"]
mod creation;

#[path = "confirmed_actions_market_tests.rs"]
mod market_confirmation;

#[path = "market_runtime_tests.rs"]
mod market_runtime_tests;

#[path = "confirmed_invitation_tests.rs"]
mod invitation_confirmation;

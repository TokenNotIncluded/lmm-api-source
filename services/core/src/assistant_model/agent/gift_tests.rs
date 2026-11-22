//! Real Session, official runtime grants, counted loopback model and canonical
//! wallet effects. Registration below is explicit synthetic owner attestation.
use super::*;
use crate::{
    assistant_model::{Binding, ServiceFundingPolicy, State},
    assistant_tools_core::context::VerifiedToolContext,
    billing::{
        Credits,
        ledger::{Action, Ledger, Request},
    },
    identity::{IdentityStore, IssuedCredential},
};
use sqlx::{Postgres, Row, Transaction};

const FIRST: &str = "I want to connect my desktop application to this service and understand how to configure a safe endpoint before issuing my first request.";
const SECOND: &str = "My second question concerns the command line client configuration and the steps needed to inspect usage without exposing credentials in logs.";

async fn gift_provider(barrier: bool) -> Provider {
    let hits = Arc::new(AtomicUsize::new(0));
    let requests = Arc::new(Mutex::new(Vec::new()));
    let counter = hits.clone();
    let observed = requests.clone();
    let app = axum::Router::new().route("/v1/chat/completions", axum::routing::post(
        move |axum::Json(request): axum::Json<Value>| {
            let counter = counter.clone();
            let observed = observed.clone();
            async move {
                let streaming = request["stream"] == true;
                observed.lock().unwrap().push(request);
                let n = counter.fetch_add(1, Ordering::SeqCst);
                let mut message = json!({"role":"assistant","content":"A bounded synthetic answer; inspect the durable tool result for any credit."});
                let mut finish = "stop";
                if n == 1 || n == 3 {
                    let mut calls = Vec::new();
                    if barrier && n == 1 {
                        calls.push(json!({"id":"extension_barrier","type":"function","function":{"name":"calculate_math","arguments":"{\"expression\":\"1+1\"}"}}));
                    }
                    calls.push(json!({"id":"real_provider_gift","type":"function","function":{"name":"prepare_new_user_gift","arguments":"{\"amount_credits\":25,\"reason\":\"Synthetic welcome\"}"}}));
                    message = json!({"role":"assistant","content":null,"tool_calls":calls});
                    finish = "tool_calls";
                }
                let response = json!({"id":format!("synthetic-gift-model-{n}"),"choices":[{"index":0,"finish_reason":finish,"message":message}],"usage":{"prompt_tokens":10,"completion_tokens":20,"prompt_tokens_details":{"cached_tokens":2}}});
                if streaming {
                    axum::response::IntoResponse::into_response(([(axum::http::header::CONTENT_TYPE,"text/event-stream")], response_sse(&response)))
                } else {
                    axum::response::IntoResponse::into_response(axum::Json(response))
                }
            }
        },
    ));
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let endpoint = format!("http://{}/v1", listener.local_addr().unwrap());
    let task = tokio::spawn(async move {
        axum::serve(listener, app).await.unwrap();
    });
    Provider {
        endpoint,
        hits,
        requests,
        task,
    }
}

struct GiftFixture {
    agent: AgentService,
    session: IssuedCredential,
    other: IssuedCredential,
    customer_account: i64,
    payer_account: i64,
    provider: Provider,
    role: runtime_authority::RuntimeRole,
}
impl GiftFixture {
    async fn close(self) {
        drop(self.agent);
        drop(self.provider);
        self.role.cleanup().await;
    }
}

async fn fixture(pool: &PgPool, barrier: bool, ttl: i64) -> GiftFixture {
    crate::core_schema::init(pool).await.unwrap();
    let identity = IdentityStore::from_pool(pool.clone());
    let session = identity.bootstrap_user(7, 1).await.unwrap();
    let other = identity.bootstrap_user(8, 1).await.unwrap();
    let payer = identity.bootstrap_user(90001, 6).await.unwrap();
    let customer_account = identity
        .authorize(&session.secret)
        .await
        .unwrap()
        .owner_account_id;
    let payer_account = identity
        .authorize(&payer.secret)
        .await
        .unwrap()
        .owner_account_id;
    assert_ne!(customer_account, 7);
    assert_ne!(payer_account, 90001);
    let ledger = Ledger::from_pool(pool.clone());
    for account in [customer_account, payer_account] {
        ledger.open_wallet(account).await.unwrap();
    }
    ledger
        .execute(&Request {
            scope: "agent-gift-test".into(),
            operation_key: "synthetic-platform-funds".into(),
            actor_user_id: Some(90001),
            reason: "Isolated platform wallet".into(),
            action: Action::Credit {
                account_id: payer_account,
                amount_units: Credits::try_from(100000).unwrap(),
                payment_reference: "synthetic:agent-gift".into(),
            },
        })
        .await
        .unwrap();
    // This deliberately does not claim a real Google registration or import.
    use hmac::{Hmac, KeyInit, Mac};
    let key: Vec<u8> = sqlx::query_scalar("SELECT secret FROM core_gift.risk_key WHERE singleton")
        .fetch_one(pool)
        .await
        .unwrap();
    let mut mac = Hmac::<Sha256>::new_from_slice(&key).unwrap();
    mac.update(b"identity\0synthetic-onboarding@example.invalid");
    sqlx::query("UPDATE core_identity.user_profiles SET email='synthetic-onboarding@example.invalid' WHERE user_id=7").execute(pool).await.unwrap();
    sqlx::query("UPDATE core_gift.policy SET enabled=TRUE,exclusive_cap=1000")
        .execute(pool)
        .await
        .unwrap();
    sqlx::query("INSERT INTO core_gift.qualifications(user_id,email_hash,network_hash,evidence_digest,evidence_kind,verified_at,registration_hold,reward_used) VALUES(7,$1,$2,$3,'owner_reviewed_registration_v1',clock_timestamp(),FALSE,FALSE)")
        .bind(mac.finalize().into_bytes().to_vec()).bind(vec![21u8;32]).bind(vec![22u8;32]).execute(pool).await.unwrap();
    let role = runtime_authority::RuntimeRole::create(pool).await;
    role.optional("core_charging").await;
    role.optional("core_assistant").await;
    role.optional("core_assistant_agent").await;
    let provider = gift_provider(barrier).await;
    let model = PersonalTextPolicy {
        model: "local-model".into(),
        group: "allowed".into(),
        budget_credits: 10000,
        max_output_tokens: 32,
        ttl_seconds: 300,
        gift_tools: true,
        sponsor: None,
        service_funding: Some(ServiceFundingPolicy {
            payer_user_id: 90001,
            payer_account_id: payer_account,
        }),
    };
    let base = AssistantModel::new(
        role.pool.clone(),
        gateway_endpoint_group_input_ceiling(&role.pool, &provider.endpoint, "allowed", 8192),
        model.clone(),
    )
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
    crate::gift::agent::validate_installed_tool(
        registry.get(crate::gift::agent::TOOL_NAME).unwrap(),
    )
    .unwrap();
    let mut tools = BTreeSet::from([crate::gift::agent::TOOL_NAME.to_owned()]);
    if barrier {
        tools.insert("calculate_math".into());
    }
    let agent = AgentService::new(base, AgentPolicy {
        model, max_rounds: 3, budget_credits: 30000, max_tool_calls: 8, ttl_seconds: ttl,
        allowed_tools: tools.clone(), initial_tools: tools,
        system_prompt: "Use only installed tools. A credit exists only when its Core receipt says issued=true.".into(), market: None,
    }, registry).unwrap();
    agent.check_ready().await.unwrap();
    let forbidden: bool = sqlx::query_scalar("SELECT NOT has_table_privilege(current_user,'core_meta.assistant_agent_gift_upgrades','SELECT,INSERT,UPDATE,DELETE,TRUNCATE')").fetch_one(&role.pool).await.unwrap();
    assert!(forbidden);
    GiftFixture {
        agent,
        session,
        other,
        customer_account,
        payer_account,
        provider,
        role,
    }
}

async fn wait_turn(
    f: &GiftFixture,
    cv: &str,
    turn: &str,
    phase: &str,
) -> super::super::store::TurnSnapshot {
    for _ in 0..400 {
        match f.agent.get(&f.session.secret, cv, turn).await {
            Ok(s) if s.phase == phase => return s,
            Ok(s) => assert!(
                !matches!(s.phase.as_str(), "unknown" | "rejected" | "expired"),
                "unexpected phase {}",
                s.phase
            ),
            Err(Error::Conflict) => {}
            Err(e) => panic!("unexpected Agent status {e:?}"),
        }
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    panic!("Agent did not reach {phase}")
}

async fn warm_and_start(f: &GiftFixture) -> String {
    let cv = f
        .agent
        .base
        .conversation(&f.session.secret, "synthetic-gift-agent", true)
        .await
        .unwrap()
        .core_conversation_id;
    f.agent
        .start(&f.session.secret, &cv, "warm", FIRST)
        .await
        .unwrap();
    assert_eq!(
        wait_turn(f, &cv, "warm", "completed").await.settled_credits,
        49
    );
    f.agent
        .start(&f.session.secret, &cv, "gift", SECOND)
        .await
        .unwrap();
    cv
}

async fn complete_barrier(f: &GiftFixture, cv: &str) {
    let waiting = wait_turn(f, cv, "gift", "tools_pending").await;
    let call = waiting.pending_calls.first().unwrap();
    assert_eq!(call.scope.name, "calculate_math");
    assert!(
        f.agent
            .claim(&f.session.secret, &call.id, "original-math-claim")
            .await
            .unwrap()
            .may_execute
    );
    let result = br#"{"result":2}"#;
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

async fn pending_id(pool: &PgPool) -> String {
    sqlx::query_scalar("SELECT id FROM core_assistant.agent_tool_operations WHERE tool_name='prepare_new_user_gift' AND state='pending'").fetch_one(pool).await.unwrap()
}
async fn serial(pool: &PgPool) -> Transaction<'_, Postgres> {
    let mut tx = pool.begin().await.unwrap();
    sqlx::query("SET TRANSACTION ISOLATION LEVEL SERIALIZABLE")
        .execute(&mut *tx)
        .await
        .unwrap();
    tx
}
async fn proof(
    tx: &mut Transaction<'_, Postgres>,
    session: &str,
    op: &str,
) -> (VerifiedToolContext, super::super::FrozenProposal) {
    let actor = crate::identity::authorize_in_transaction(tx, session)
        .await
        .unwrap();
    let context = VerifiedToolContext::verify(tx, &actor).await.unwrap();
    let proposal = super::super::verify_proposal_in_transaction(tx, op, &context)
        .await
        .unwrap();
    (context, proposal)
}
async fn counts(pool: &PgPool) -> (i64, i64) {
    sqlx::query_as("SELECT (SELECT count(*) FROM core_gift.decisions),(SELECT count(*) FROM core_billing.ledger_journals WHERE kind='gift')").fetch_one(pool).await.unwrap()
}
fn is_check(error: &sqlx::Error) -> bool {
    error.as_database_error().and_then(|e| e.code()).as_deref() == Some("23514")
}

#[sqlx::test(migrations = false)]
async fn settled_agent_gift_posts_once_under_concurrent_wakeups_and_replay(pool: PgPool) {
    let f = fixture(&pool, true, 300).await;
    let cv = warm_and_start(&f).await;
    wait_turn(&f, &cv, "gift", "tools_pending").await;
    let mut gate = pool.begin().await.unwrap();
    sqlx::query("SELECT pg_advisory_xact_lock(671124215333)")
        .execute(&mut *gate)
        .await
        .unwrap();
    complete_barrier(&f, &cv).await;
    let turn: String = sqlx::query_scalar("SELECT id FROM core_assistant.agent_turns WHERE conversation_id=$1 AND client_turn_id='gift'").bind(&cv).fetch_one(&pool).await.unwrap();
    for _ in 0..4 {
        f.agent.kick(&f.session.secret, &turn);
    }
    let mut blocked = false;
    for _ in 0..100 {
        blocked = sqlx::query_scalar("SELECT EXISTS(SELECT FROM pg_stat_activity WHERE datname=current_database() AND wait_event='advisory' AND pid<>pg_backend_pid())").fetch_one(&pool).await.unwrap();
        if blocked {
            break;
        }
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    assert!(
        blocked,
        "real workers must overlap before the gift lock is released"
    );
    gate.commit().await.unwrap();
    let done = wait_turn(&f, &cv, "gift", "completed").await;
    assert_eq!((done.settled_credits, done.held_credits), (98, 0));
    assert_eq!(counts(&pool).await, (1, 1));
    let row = sqlx::query("SELECT d.id,d.amount_credits,d.ledger_journal_id,o.result_json,o.domain_receipt_id,r.state AS round_state,q.state AS request_state FROM core_gift.decisions d JOIN core_assistant.agent_rounds r ON r.model_request_id=d.model_request_id JOIN core_charging.requests q ON q.id=r.model_request_id JOIN core_assistant.agent_tool_operations o ON o.round_id=r.id AND o.tool_name='prepare_new_user_gift'").fetch_one(&pool).await.unwrap();
    let result: Value = serde_json::from_slice(&row.get::<Vec<u8>, _>("result_json")).unwrap();
    assert_eq!(result["ok"], true);
    assert_eq!(result["issued"], true);
    assert_eq!(result["gift_id"], row.get::<String, _>("id"));
    assert_eq!(result["credit_amount"], 25);
    assert_eq!(
        result["ledger_transaction_id"],
        row.get::<i64, _>("ledger_journal_id")
    );
    assert_eq!(
        row.get::<String, _>("domain_receipt_id"),
        format!("core-gift-{}", row.get::<String, _>("id"))
    );
    assert_eq!(row.get::<String, _>("round_state"), "settled");
    assert_eq!(row.get::<String, _>("request_state"), "settled");
    let ledger = Ledger::from_pool(pool.clone());
    assert_eq!(
        ledger
            .balance(f.customer_account)
            .await
            .unwrap()
            .available_units,
        25
    );
    let payer = ledger.balance(f.payer_account).await.unwrap();
    assert_eq!((payer.available_units, payer.reserved_units), (99853, 0));
    let retry = f
        .agent
        .start(&f.session.secret, &cv, "gift", SECOND)
        .await
        .unwrap();
    assert_eq!(retry.phase, "completed");
    assert_eq!(counts(&pool).await, (1, 1));
    assert_eq!(f.provider.hits.load(Ordering::SeqCst), 3);
    let paid: i64 = sqlx::query_scalar("SELECT count(*) FROM core_charging.requests WHERE actor_user_id=7 AND owner_account_id=$1 AND payer_account_id=$2 AND state='settled' AND settled=49 AND reserved=8256").bind(f.customer_account).bind(f.payer_account).fetch_one(&pool).await.unwrap();
    assert_eq!(paid, 3);
    f.close().await;
}

#[sqlx::test(migrations = false)]
async fn deferred_agent_gift_requires_both_tool_result_and_real_ledger_effect(pool: PgPool) {
    let f = fixture(&pool, true, 300).await;
    let cv = warm_and_start(&f).await;
    wait_turn(&f, &cv, "gift", "tools_pending").await;
    let op = pending_id(&pool).await;
    let mut forward = serial(&f.role.pool).await;
    let fake = json!({"ok":true,"issued":true,"gift_id":"f".repeat(64),"credit_amount":25,"ledger_transaction_id":999999,"claimed_at_unix":1});
    let bytes = serde_json::to_vec(&fake).unwrap();
    sqlx::query("UPDATE core_assistant.agent_tool_operations SET state='completed',claim_key=$2,result_json=$3,result_sha256=$4,domain_receipt_id=$5 WHERE id=$1")
        .bind(&op).bind(format!("core-{op}")).bind(&bytes).bind(Sha256::digest(&bytes).to_vec()).bind(format!("core-gift-{}", "f".repeat(64))).execute(&mut *forward).await.unwrap();
    let error = forward.commit().await.unwrap_err();
    assert!(is_check(&error), "unbacked completed result: {error}");
    assert_eq!(counts(&pool).await, (0, 0));
    // The exact real issuance path writes INSERT -> claimed UPDATE, but a
    // missing tool completion must roll the entire real journal back at COMMIT.
    let mut reverse = serial(&f.role.pool).await;
    let (context, proposal) = proof(&mut reverse, &f.session.secret, &op).await;
    let issued = crate::gift::agent::execute(&mut reverse, &context, &proposal)
        .await
        .unwrap();
    assert_eq!(issued.result["issued"], true);
    let error = reverse.commit().await.unwrap_err();
    assert!(
        is_check(&error),
        "journal without its completed tool: {error}"
    );
    assert_eq!(counts(&pool).await, (0, 0));
    assert_eq!(
        Ledger::from_pool(pool.clone())
            .balance(f.customer_account)
            .await
            .unwrap()
            .available_units,
        0
    );
    complete_barrier(&f, &cv).await;
    wait_turn(&f, &cv, "gift", "completed").await;
    assert_eq!(counts(&pool).await, (1, 1));
    assert_eq!(f.provider.hits.load(Ordering::SeqCst), 3);
    // A later ordinary turn can commit after the issuance's deferred checks;
    // no completed gift is revalidated against a later turn's phase or TTL.
    f.agent
        .start(
            &f.session.secret,
            &cv,
            "after-gift",
            "Read my durable prior gift receipt after the completed turn",
        )
        .await
        .unwrap();
    wait_turn(&f, &cv, "after-gift", "completed").await;
    assert_eq!(counts(&pool).await, (1, 1));
    let repeated: Vec<u8> = sqlx::query_scalar("SELECT o.result_json FROM core_assistant.agent_tool_operations o JOIN core_assistant.agent_turns t ON t.id=o.turn_id WHERE t.client_turn_id='after-gift' AND o.tool_name='prepare_new_user_gift'").fetch_one(&pool).await.unwrap();
    let repeated: Value = serde_json::from_slice(&repeated).unwrap();
    assert_eq!(repeated["issued"], false);
    assert_eq!(repeated["reason"], "already_exists");
    assert!(repeated.get("ledger_transaction_id").is_none());
    f.close().await;
}

#[sqlx::test(migrations = false)]
async fn agent_gift_denies_wrong_session_revoked_grant_and_unqualified_actor(pool: PgPool) {
    let f = fixture(&pool, true, 300).await;
    let cv = warm_and_start(&f).await;
    wait_turn(&f, &cv, "gift", "tools_pending").await;
    let op = pending_id(&pool).await;
    let mut wrong = serial(&f.role.pool).await;
    let actor = crate::identity::authorize_in_transaction(&mut wrong, &f.other.secret)
        .await
        .unwrap();
    let context = VerifiedToolContext::verify(&mut wrong, &actor)
        .await
        .unwrap();
    assert!(matches!(
        super::super::verify_proposal_in_transaction(&mut wrong, &op, &context).await,
        Err(Error::Forbidden)
    ));
    wrong.rollback().await.unwrap();
    let mut revoked = serial(&f.role.pool).await;
    let (context, proposal) = proof(&mut revoked, &f.session.secret, &op).await;
    sqlx::query("UPDATE core_assistant.grants g SET revoked=TRUE FROM core_charging.requests q JOIN core_assistant.agent_rounds r ON r.model_request_id=q.id JOIN core_assistant.agent_tool_operations o ON o.round_id=r.id WHERE o.id=$1 AND g.id=q.assistant_grant_id").bind(&op).execute(&mut *revoked).await.unwrap();
    let denied = crate::gift::agent::execute(&mut revoked, &context, &proposal)
        .await
        .unwrap();
    assert_eq!(denied.result["issued"], false);
    revoked.rollback().await.unwrap();
    let mut streaming = serial(&f.role.pool).await;
    let request: String = sqlx::query_scalar("SELECT r.model_request_id FROM core_assistant.agent_rounds r JOIN core_assistant.agent_tool_operations o ON o.round_id=r.id WHERE o.id=$1").bind(&op).fetch_one(&mut *streaming).await.unwrap();
    assert_eq!(
        crate::gift::execute_in_transaction(
            &mut streaming,
            7,
            &cv,
            &request,
            crate::gift::GiftDecision {
                amount_credits: 25,
                reason: "must not alias streaming".into()
            }
        )
        .await,
        Err(crate::gift::Error::Forbidden)
    );
    streaming.rollback().await.unwrap();
    sqlx::query("UPDATE core_gift.qualifications SET registration_hold=TRUE WHERE user_id=7")
        .execute(&pool)
        .await
        .unwrap();
    complete_barrier(&f, &cv).await;
    wait_turn(&f, &cv, "gift", "completed").await;
    let bytes: Vec<u8> = sqlx::query_scalar(
        "SELECT result_json FROM core_assistant.agent_tool_operations WHERE id=$1",
    )
    .bind(&op)
    .fetch_one(&pool)
    .await
    .unwrap();
    let result: Value = serde_json::from_slice(&bytes).unwrap();
    assert_eq!(result["ok"], false);
    assert_eq!(result["issued"], false);
    assert!(result.get("ledger_transaction_id").is_none());
    assert_eq!(counts(&pool).await, (0, 0));
    assert_eq!(
        Ledger::from_pool(pool.clone())
            .balance(f.customer_account)
            .await
            .unwrap()
            .available_units,
        0
    );
    assert_eq!(f.provider.hits.load(Ordering::SeqCst), 3);
    f.close().await;
}

#[sqlx::test(migrations = false)]
async fn agent_gift_expiry_and_disabled_tool_policy_fail_without_issuance(pool: PgPool) {
    let f = fixture(&pool, true, 2).await;
    let mut disabled = (*f.agent.policy).clone();
    disabled.model.gift_tools = false;
    let base = AssistantModel::new(
        f.role.pool.clone(),
        gateway_endpoint_group_input_ceiling(&f.role.pool, &f.provider.endpoint, "allowed", 8192),
        disabled.model.clone(),
    )
    .unwrap();
    assert!(matches!(
        AgentService::new(base, disabled, f.agent.registry.clone()),
        Err(Error::Forbidden)
    ));
    let cv = warm_and_start(&f).await;
    wait_turn(&f, &cv, "gift", "tools_pending").await;
    let op = pending_id(&pool).await;
    // Real elapsed expiry, with no rewritten grant/turn timestamp and no request retry.
    tokio::time::sleep(Duration::from_millis(2200)).await;
    let mut expired = serial(&f.role.pool).await;
    let actor = crate::identity::authorize_in_transaction(&mut expired, &f.session.secret)
        .await
        .unwrap();
    let context = VerifiedToolContext::verify(&mut expired, &actor)
        .await
        .unwrap();
    assert!(matches!(
        super::super::verify_proposal_in_transaction(&mut expired, &op, &context).await,
        Err(Error::Forbidden)
    ));
    expired.rollback().await.unwrap();
    let cancelled = f
        .agent
        .cancel(&f.session.secret, &cv, "gift")
        .await
        .unwrap();
    assert_eq!(cancelled.phase, "expired");
    assert_eq!(counts(&pool).await, (0, 0));
    assert_eq!(f.provider.hits.load(Ordering::SeqCst), 2);
    assert_eq!(
        Ledger::from_pool(pool.clone())
            .balance(f.customer_account)
            .await
            .unwrap()
            .available_units,
        0
    );
    f.close().await;
}

#[sqlx::test(migrations = false)]
async fn personal_streaming_gift_preserves_service_payer_and_original_source_boundary(
    pool: PgPool,
) {
    let f = fixture(&pool, false, 300).await;
    let base = &f.agent.base;
    let cv = base
        .conversation(&f.session.secret, "personal-streaming-service-gift", true)
        .await
        .unwrap()
        .core_conversation_id;
    let mut last = None;
    for (turn, text) in [("first", FIRST), ("second", SECOND)] {
        let g = base.turn(&f.session.secret, &cv, turn, true).await.unwrap();
        let binding = Binding {
            delegation_id: g.delegation_id,
            core_conversation_id: g.core_conversation_id,
            turn_id: g.turn_id,
            round: 0,
            operation_key: format!("{turn}:0"),
            text_sha256: Sha256::digest(text.as_bytes()).to_vec(),
        };
        let result = base
            .reserve_text(&f.session.secret, &binding, text)
            .await
            .unwrap();
        assert_eq!(result.state, State::Settled);
        if turn == "first" {
            assert!(result.gift_receipt.is_none());
        }
        last = Some((binding, result));
    }
    let (binding, result) = last.unwrap();
    let gift = result.gift_receipt.as_ref().unwrap();
    assert_eq!(gift.owner_user_id, 7);
    assert_eq!(gift.credit_amount, 25);
    assert!(gift.claimed_at_unix.is_some() && gift.ledger_transaction_id.is_some());
    assert_eq!(
        base.reserve_text(&f.session.secret, &binding, SECOND)
            .await
            .unwrap()
            .gift_receipt,
        result.gift_receipt
    );
    assert_eq!(counts(&pool).await, (1, 1));
    let tools: i64 =
        sqlx::query_scalar("SELECT count(*) FROM core_assistant.agent_tool_operations")
            .fetch_one(&pool)
            .await
            .unwrap();
    assert_eq!(
        tools, 0,
        "PersonalText still uses its original streaming source"
    );
    let ledger = Ledger::from_pool(pool.clone());
    assert_eq!(
        ledger
            .balance(f.customer_account)
            .await
            .unwrap()
            .available_units,
        25
    );
    assert_eq!(
        ledger
            .balance(f.payer_account)
            .await
            .unwrap()
            .available_units,
        99902
    );
    assert_eq!(f.provider.hits.load(Ordering::SeqCst), 2);
    f.close().await;
}

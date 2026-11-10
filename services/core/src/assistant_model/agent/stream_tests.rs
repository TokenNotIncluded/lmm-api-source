// Copyright (C) 2026 LIghtJUNction. AGPL-3.0-or-later.
//! Counted HTTP/SSE provider + real Core Charging/Ledger on disposable PostgreSQL.
use super::*;
use crate::assistant_model::live;
use tokio::sync::Notify;

async fn streaming_provider(complete: bool) -> (Provider, Arc<Notify>, Arc<Notify>) {
    let hits = Arc::new(AtomicUsize::new(0));
    let requests = Arc::new(Mutex::new(Vec::new()));
    let release = Arc::new(Notify::new());
    let closed = Arc::new(Notify::new());
    let (counter, observed, gate, disconnected) = (
        hits.clone(),
        requests.clone(),
        release.clone(),
        closed.clone(),
    );
    let app = axum::Router::new().route("/v1/chat/completions", axum::routing::post(move |axum::Json(request): axum::Json<Value>| {
        let (counter, observed, gate, disconnected) = (counter.clone(), observed.clone(), gate.clone(), disconnected.clone());
        async move {
            assert_eq!(request["stream"], true);
            assert_eq!(request["stream_options"]["include_usage"], true);
            observed.lock().unwrap().push(request);
            counter.fetch_add(1, Ordering::SeqCst);
            let (sender, receiver) = tokio::sync::mpsc::channel::<Result<axum::body::Bytes, std::io::Error>>(1);
            tokio::spawn(async move {
                let first = format!("data: {}\n\n",json!({"id":"counted-stream","choices":[{"index":0,"delta":{"role":"assistant","content":"hello 世界"},"finish_reason":null}]}));
                // Deliberately split JSON and multibyte Unicode across HTTP chunks.
                for byte in first.bytes() {
                    if sender.send(Ok(axum::body::Bytes::from(vec![byte]))).await.is_err() { disconnected.notify_one(); return; }
                }
                tokio::select! {
                    _ = sender.closed() => { disconnected.notify_one(); return; }
                    _ = gate.notified() => {}
                }
                if complete {
                    let rest = format!("data: {}\n\ndata: {}\n\ndata: {}\n\ndata: [DONE]\n\n",
                        json!({"id":"counted-stream","choices":[{"index":0,"delta":{"content":" done"},"finish_reason":null}]}),
                        json!({"id":"counted-stream","choices":[{"index":0,"delta":{},"finish_reason":"stop"}]}),
                        json!({"id":"counted-stream","choices":[],"usage":{"prompt_tokens":10,"completion_tokens":20,"prompt_tokens_details":{"cached_tokens":2}}}));
                    let _ = sender.send(Ok(axum::body::Bytes::from(rest))).await;
                }
            });
            axum::response::Response::builder().header("content-type","text/event-stream")
                .body(axum::body::Body::from_stream(tokio_stream::wrappers::ReceiverStream::new(receiver))).unwrap()
        }
    }));
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let endpoint = format!("http://{}/v1", listener.local_addr().unwrap());
    let task = tokio::spawn(async move {
        axum::serve(listener, app).await.unwrap();
    });
    (
        Provider {
            endpoint,
            hits,
            requests,
            task,
        },
        release,
        closed,
    )
}

async fn stream_setup(
    pool: &PgPool,
    complete: bool,
) -> (
    AgentService,
    String,
    String,
    i64,
    Provider,
    Arc<Notify>,
    Arc<Notify>,
) {
    let (mut agent, session, account, old_provider) = setup(pool, "calculate_math").await;
    drop(old_provider);
    let (provider, release, closed) = streaming_provider(complete).await;
    agent.base.gateway = gateway_endpoint(pool, &provider.endpoint);
    let conversation = agent
        .base
        .conversation(&session, "live-stream-conversation", true)
        .await
        .unwrap()
        .core_conversation_id;
    agent
        .start(&session, &conversation, "turn1", "stream original input")
        .await
        .unwrap();
    (
        agent,
        session,
        conversation,
        account,
        provider,
        release,
        closed,
    )
}

async fn prefix(
    agent: &AgentService,
    session: &str,
    cv: &str,
) -> tokio::sync::watch::Receiver<live::Snapshot> {
    let subject = agent.output_subject(session, cv, "turn1").await.unwrap();
    tokio::time::timeout(Duration::from_secs(2), async {
        loop {
            match agent.base.live.subscribe(&subject) {
                Ok(mut rx) => loop {
                    let snapshot = rx.borrow_and_update().clone();
                    if snapshot.text == "hello 世界" {
                        return rx;
                    }
                    rx.changed().await.unwrap();
                },
                Err(Error::NotFound) => tokio::task::yield_now().await,
                Err(error) => panic!("unexpected observe error {error:?}"),
            }
        }
    })
    .await
    .expect("actual first prefix before releasing provider terminal")
}

async fn held_and_settled(pool: &PgPool) -> (i64, i64) {
    sqlx::query_as("SELECT coalesce(sum(q.reserved) FILTER(WHERE q.state IN ('reserved','streaming','reconcile')),0)::bigint,coalesce(sum(q.settled),0)::bigint FROM core_assistant.agent_rounds r JOIN core_charging.requests q ON q.id=r.model_request_id")
        .fetch_one(pool).await.unwrap()
}

#[sqlx::test(migrations = false)]
async fn actual_prefix_precedes_terminal_then_original_usage_settles_once(pool: PgPool) {
    let (agent, session, cv, _, provider, release, _) = stream_setup(&pool, true).await;
    let _rx = prefix(&agent, &session, &cv).await;
    let before = held_and_settled(&pool).await;
    assert!(before.0 > 0);
    assert_eq!(before.1, 0);
    let digest = Sha256::digest(b"stream original input").to_vec();
    let subject = agent.output_subject(&session, &cv, "turn1").await.unwrap();
    assert_eq!(subject.text_sha256, digest);
    let mut foreign = subject.clone();
    foreign.owner_user_id += 1;
    assert!(matches!(
        agent.base.live.subscribe(&foreign),
        Err(Error::Forbidden)
    ));
    release.notify_one();
    let final_turn = wait(&agent, &session, &cv, "completed").await;
    assert_eq!(final_turn.final_text.as_deref(), Some("hello 世界 done"));
    let after = held_and_settled(&pool).await;
    assert_eq!(after.0, 0);
    assert!(after.1 > 0);
    let _ = agent.get(&session, &cv, "turn1").await.unwrap();
    let _ = agent
        .start(&session, &cv, "turn1", "stream original input")
        .await
        .unwrap();
    assert_eq!(held_and_settled(&pool).await, after);
    assert_eq!(provider.hits.load(Ordering::SeqCst), 1);
}

#[sqlx::test(migrations = false)]
async fn truncated_real_stream_retains_hold_and_never_replays_on_recovery(pool: PgPool) {
    let (agent, session, cv, _, provider, release, _) = stream_setup(&pool, false).await;
    let _rx = prefix(&agent, &session, &cv).await;
    let held = held_and_settled(&pool).await;
    release.notify_one();
    wait_unknown(&agent, &session, &cv).await;
    assert_eq!(held_and_settled(&pool).await, held);
    assert!(held.0 > 0);
    assert_eq!(held.1, 0);
    let _ = agent
        .start(&session, &cv, "turn1", "stream original input")
        .await
        .unwrap();
    assert_eq!(provider.hits.load(Ordering::SeqCst), 1);
}

async fn wait_unknown(agent: &AgentService, session: &str, cv: &str) {
    tokio::time::timeout(Duration::from_secs(3), async {
        loop {
            match agent.get(session, cv, "turn1").await {
                Ok(turn) if turn.phase == "unknown" => break,
                Ok(_) | Err(Error::Conflict) => {}
                Err(error) => panic!("unexpected recovery error {error:?}"),
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .unwrap();
}

#[sqlx::test(migrations = false)]
async fn cancellation_committed_by_other_process_stops_original_http_and_survives_restart(
    pool: PgPool,
) {
    let (agent, session, cv, _, provider, _, closed) = stream_setup(&pool, true).await;
    let _rx = prefix(&agent, &session, &cv).await;
    let held = held_and_settled(&pool).await;
    // A separate base owns an empty live registry, as after a process restart.
    let restarted = AgentService::new(
        AssistantModel::new(
            pool.clone(),
            gateway_endpoint(&pool, &provider.endpoint),
            agent.policy.model.clone(),
        )
        .unwrap(),
        (*agent.policy).clone(),
        agent.registry.clone(),
    )
    .unwrap();
    restarted.cancel(&session, &cv, "turn1").await.unwrap();
    tokio::time::timeout(Duration::from_secs(2), closed.notified())
        .await
        .expect("durable cancellation stops other process's HTTP body");
    wait_unknown(&agent, &session, &cv).await;
    assert_eq!(held_and_settled(&pool).await, held);
    assert_eq!(held.1, 0);
    assert!(held.0 > 0);
    assert_eq!(
        restarted.get(&session, &cv, "turn1").await.unwrap().phase,
        "unknown"
    );
    assert_eq!(
        restarted
            .start(&session, &cv, "turn1", "stream original input")
            .await
            .unwrap()
            .phase,
        "unknown"
    );
    assert_eq!(provider.hits.load(Ordering::SeqCst), 1);
}

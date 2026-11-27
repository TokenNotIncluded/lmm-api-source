//! Real local suppliers and PostgreSQL: one logical bill across safe attempts.
use super::*;
use axum::{Json, Router, extract::State, response::IntoResponse, routing::post};
use std::sync::atomic::{AtomicUsize, Ordering};
use tokio::net::TcpListener;
use tower::ServiceExt;

async fn dispatch_retry_pg_case(pool: PgPool, failures: u8) {
    let f = fixture(pool, 10_000_000).await;
    let runtime = crate::test_runtime::RuntimeRole::create(&f.pool).await;
    runtime.optional("core_charging").await;
    runtime.optional("core_model").await;
    let runtime_fixture = Fixture {
        pool: runtime.pool.clone(),
        ledger: Ledger::from_pool(runtime.pool.clone()),
        session: f.session.clone(),
        key: f.key.clone(),
        key_id: f.key_id,
        account: f.account,
    };
    let supplier = CompatibilityProvider::start().await;
    let g = gateway(&runtime_fixture, &supplier.endpoint, vec![]);
    let id = crate::hex_codec::encode(Sha256::digest(format!("dispatch-retry-{failures}")));
    let (context, request) = admit(&g, &runtime_fixture, &id, "small-model", 16).await;
    let worker = g.bridge.active.lock().unwrap()[&id].worker;
    let worker_digest = crate::hex_codec::encode(Sha256::digest(worker));
    // Sequence increments survive rollback. All other writes use the real
    // charging transaction, including the injected usage-event INSERT failure.
    // SECURITY DEFINER grants no DDL/sequence authority to the runtime caller.
    sqlx::raw_sql(sqlx::AssertSqlSafe(format!(
        r#"
        CREATE SEQUENCE public.dispatch_retry_attempt;
        CREATE FUNCTION public.inject_dispatch_retry() RETURNS trigger
        LANGUAGE plpgsql SECURITY DEFINER SET search_path=pg_catalog AS $$
        DECLARE attempt BIGINT; proof JSONB; stored_worker TEXT; actual_group TEXT;
        BEGIN
            IF NEW.event_id <> 'route-dispatch:1' THEN RETURN NEW; END IF;
            attempt := nextval('public.dispatch_retry_attempt');
            IF session_user <> TG_ARGV[0] OR NEW.request_id <> TG_ARGV[1] THEN
                RAISE EXCEPTION 'unexpected dispatch caller or request' USING ERRCODE='22023';
            END IF;
            SELECT encode(worker_digest,'hex'),price->>'group' INTO STRICT stored_worker,actual_group
                FROM core_charging.requests WHERE id=NEW.request_id;
            SELECT detail INTO STRICT proof FROM core_charging.usage_events
                WHERE request_id=NEW.request_id AND event_id='reservation-contract:v1';
            IF stored_worker <> TG_ARGV[2] OR NEW.detail IS DISTINCT FROM
                jsonb_build_object('kind','model_route_dispatch','number',1,
                    'route_id',proof->'binding'->>'route_id','actual_group',actual_group,
                    'wire_sha256',proof->>'wire_sha256','inbound_protocol','chat') THEN
                RAISE EXCEPTION 'dispatch changed frozen worker or payload' USING ERRCODE='22023';
            END IF;
            IF attempt <= TG_ARGV[3]::BIGINT THEN
                RAISE EXCEPTION 'synthetic rolled-back dispatch' USING ERRCODE='40001';
            END IF;
            RETURN NEW;
        END $$;
        CREATE TRIGGER inject_dispatch_retry BEFORE INSERT ON core_charging.usage_events
            FOR EACH ROW EXECUTE FUNCTION public.inject_dispatch_retry('{role}','{id}','{worker_digest}','{failures}');
        "#,
        role = runtime.name,
    )))
    .execute(&f.pool)
    .await
    .unwrap();

    let relay::Session {
        head,
        body,
        completion,
        ..
    } = g.relay.start(request, context).unwrap();
    let head = head.await.unwrap();
    let output = axum::body::to_bytes(axum::body::Body::from_stream(body), 1 << 20).await;
    let completed = completion.await.unwrap();
    assert_eq!(completed.report.request_id, id);
    assert_eq!(completed.settlement, relay::Settlement::Confirmed);
    let succeeded = failures < 3;
    if succeeded {
        assert_eq!(head.unwrap().status, 200);
        assert!(!output.unwrap().is_empty());
        assert!(completed.error.is_none());
        assert!(completed.report.upstream_attempted);
    } else {
        assert_eq!(head.unwrap_err(), RelayError::BillingUnavailable);
        assert_eq!(completed.error, Some(RelayError::BillingUnavailable));
        assert!(!completed.report.upstream_attempted);
        assert_eq!(completed.report.delivery, routing::DeliveryState::NotSent);
    }
    let attempts: i64 = sqlx::query_scalar("SELECT last_value FROM public.dispatch_retry_attempt")
        .fetch_one(&f.pool)
        .await
        .unwrap();
    assert_eq!(attempts, i64::from((failures + 1).min(3)));
    assert_eq!(
        supplier.bodies.lock().unwrap().len(),
        usize::from(succeeded)
    );
    assert_eq!(requests(&f.pool).await, 1);
    let dispatches: i64 = sqlx::query_scalar("SELECT count(*) FROM core_charging.usage_events WHERE request_id=$1 AND event_id='route-dispatch:1'")
        .bind(&id).fetch_one(&f.pool).await.unwrap();
    assert_eq!(dispatches, i64::from(succeeded));
    let persisted_worker: String = sqlx::query_scalar(
        "SELECT encode(worker_digest,'hex') FROM core_charging.requests WHERE id=$1",
    )
    .bind(&id)
    .fetch_one(&f.pool)
    .await
    .unwrap();
    assert_eq!(persisted_worker, worker_digest);
    let charge = receipt(&g, &id).await;
    assert_eq!(charge.state, "settled");
    assert_eq!(charge.settled, if succeeded { 2 } else { 0 });
    assert_eq!(
        charge.outcome.as_deref(),
        Some(if succeeded { "completed" } else { "cancelled" })
    );
    let hold: (i64, i64, i64, bool) = sqlx::query_as("SELECT count(*),min(reserved),min(captured),bool_and(closed) FROM core_charging.ledger_holds WHERE request_id=$1")
        .bind(&id).fetch_one(&f.pool).await.unwrap();
    assert_eq!(hold, (1, charge.reserved, charge.settled, true));
    for kind in ["reserve", "capture"] {
        let journals: i64 = sqlx::query_scalar("SELECT count(*) FROM core_billing.ledger_journals WHERE scope='core-charging' AND kind=$1 AND reason=$2")
            .bind(kind).bind(format!("Model request {id}")).fetch_one(&f.pool).await.unwrap();
        assert_eq!(journals, 1, "{kind}");
    }
    assert_balance(&f, 10_000_000 - charge.settled, 0).await;
    drop(g);
    runtime.cleanup().await;
}

#[sqlx::test(migrations = false)]
async fn route_dispatch_retry_pg_first_rollback_sends_once(pool: PgPool) {
    dispatch_retry_pg_case(pool, 1).await;
}

#[sqlx::test(migrations = false)]
async fn route_dispatch_retry_pg_two_rollbacks_send_once(pool: PgPool) {
    dispatch_retry_pg_case(pool, 2).await;
}

#[sqlx::test(migrations = false)]
async fn route_dispatch_retry_pg_exhaustion_never_sends(pool: PgPool) {
    dispatch_retry_pg_case(pool, 3).await;
}

#[derive(Clone, Copy)]
enum FailureReply {
    Unauthorized,
    Rejected,
    RateLimited,
    RetryAfter,
    ToolEvidence,
    NestedToolEvidence,
    Timeout,
    Partial,
}
struct FailedSupplier {
    base: String,
    hits: Arc<AtomicUsize>,
    task: tokio::task::JoinHandle<()>,
}
impl Drop for FailedSupplier {
    fn drop(&mut self) {
        self.task.abort();
    }
}
impl FailedSupplier {
    async fn start(mode: FailureReply) -> Self {
        async fn response(
            State((mode, hits)): State<(FailureReply, Arc<AtomicUsize>)>,
            headers: HeaderMap,
            Json(_): Json<serde_json::Value>,
        ) -> axum::response::Response {
            assert!(!headers.contains_key("x-lmm-session-id"));
            hits.fetch_add(1, Ordering::SeqCst);
            match mode {
                FailureReply::Unauthorized => (axum::http::StatusCode::UNAUTHORIZED,Json(json!({"error":{"message":"synthetic rejected key"}}))).into_response(),
                FailureReply::Rejected => (axum::http::StatusCode::SERVICE_UNAVAILABLE, Json(json!({"error":{"message":"synthetic pre-acceptance rejection"}}))).into_response(),
                FailureReply::RateLimited => (axum::http::StatusCode::TOO_MANY_REQUESTS, Json(json!({"error":{"message":"synthetic pre-acceptance rate limit"}}))).into_response(),
                FailureReply::RetryAfter => (axum::http::StatusCode::TOO_MANY_REQUESTS, [("retry-after", "120")], Json(json!({"error":{"message":"synthetic pre-acceptance rate limit"}}))).into_response(),
                FailureReply::ToolEvidence => (axum::http::StatusCode::SERVICE_UNAVAILABLE, Json(json!({"error":{"message":"synthetic failure after work"},"usage":{"prompt_tokens":1},"tool_calls":[{"id":"already-called"}]}))).into_response(),
                FailureReply::NestedToolEvidence => (axum::http::StatusCode::SERVICE_UNAVAILABLE, Json(json!({"error":{"message":"failed after work","metadata":{"tool_calls":[{"id":"already-called"}]}}}))).into_response(),
                FailureReply::Timeout => { tokio::time::sleep(Duration::from_secs(5)).await; Json(json!({"error":{"message":"late"}})).into_response() },
                FailureReply::Partial => ([ ("content-type", "text/event-stream") ], "data: {\"id\":\"partial\",\"choices\":[{\"index\":0,\"delta\":{\"content\":\"partial content\"},\"finish_reason\":null}]}\n\n").into_response(),
            }
        }
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let base = format!("http://{}/v1", listener.local_addr().unwrap());
        let hits = Arc::new(AtomicUsize::new(0));
        let app = Router::new()
            .route("/v1/chat/completions", post(response))
            .with_state((mode, hits.clone()));
        let task = tokio::spawn(async move {
            axum::serve(listener, app).await.unwrap();
        });
        Self { base, hits, task }
    }
}

async fn refused_endpoint() -> String {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    format!("http://{}/v1", listener.local_addr().unwrap())
}

fn failover_gateway(
    f: &Fixture,
    first: &str,
    second: &str,
    reviewed_rejection: bool,
    cross_group: bool,
    equal_priority: bool,
) -> ModelGateway {
    let original = gateway(f, first, vec![]);
    let mut doc = original.bridge.routing.catalog_document().unwrap();
    let mut upstream = doc.routing.upstreams[0].clone();
    upstream.id = 2;
    upstream.endpoint = Endpoint::local_loopback(second.into()).unwrap();
    doc.routing.upstreams.push(upstream);
    let mut target = doc.routing.routes[0].targets[0].clone();
    target.id = 3;
    target.upstream_id = 2;
    target.priority = u16::from(!equal_priority);
    doc.routing.routes[0].failover = routing::FailoverPolicy {
        max_attempts: 2,
        retry_on: vec![
            routing::RetryCause::Connect,
            routing::RetryCause::Timeout,
            routing::RetryCause::RateLimited,
            routing::RetryCause::ServerError,
        ],
        allow_replay_after_send: true, // Still never grants request replay permission.
    };
    if cross_group {
        doc.routing.groups.push(routing::ModelGroup {
            name: "backup".into(),
            enabled: true,
        });
        let mut backup = doc.routing.routes[0].clone();
        backup.id = 3;
        backup.group = "backup".into();
        backup.targets = vec![target];
        doc.routing.routes.push(backup);
        let mut price = doc.pricing.rules[0].clone();
        price.id = 3;
        price.group = "backup".into();
        price.rates.input = 6_000_000;
        doc.pricing.rules.push(price);
    } else {
        doc.routing.routes[0].targets.push(target);
    }
    let policy = if reviewed_rejection {
        let mut context = contexts(GLOBAL_INPUT).remove(0);
        context.pre_acceptance_rejection_statuses = vec![401, 429, 503];
        vec![context]
    } else {
        vec![]
    };
    ModelGateway::new_with_provider_policy(
        f.pool.clone(),
        Arc::new(
            routing::Router::new(
                doc.routing,
                Arc::new(PriceSnapshot::compile(doc.pricing).unwrap()),
            )
            .unwrap(),
        ),
        vec![UpstreamCredential {
            reference: CredentialRef { id: 7, revision: 1 },
            secret: "synthetic-selected-bound-provider-key".into(),
        }],
        BTreeMap::from([(f.key_id, "allowed".into())]),
        Limits {
            max_output_tokens: GLOBAL_OUTPUT,
            total_timeout: Duration::from_secs(2),
            header_timeout: Duration::from_millis(250),
            hook_timeout: Duration::from_secs(2),
            ..Default::default()
        },
        original.bridge.policy,
        ProviderPolicy::new(policy, GLOBAL_INPUT).unwrap(),
    )
    .unwrap()
    .with_local_streaming()
}

async fn call(
    g: &ModelGateway,
    f: &Fixture,
    key: &str,
    stream: bool,
    affinity: Option<&str>,
) -> (u16, String, String) {
    let mut request = axum::http::Request::builder()
        .method("POST")
        .uri("/v1/chat/completions")
        .header("authorization", format!("Bearer {}", f.key))
        .header("content-type", "application/json")
        .header("idempotency-key", key);
    if let Some(value) = affinity {
        request = request.header("x-lmm-session-id", value);
    }
    let response = g.router().oneshot(request.body(axum::body::Body::from(json!({"model":"small-model","messages":[{"role":"user","content":"hello"}],"max_tokens":16,"stream":stream,"service_tier":"ultrafast","speed":"fast"}).to_string())).unwrap()).await.unwrap();
    let status = response.status().as_u16();
    let id = response
        .headers()
        .get("x-request-id")
        .map(|v| v.to_str().unwrap().to_owned())
        .unwrap_or_default();
    let bytes = axum::body::to_bytes(response.into_body(), 1 << 20)
        .await
        .unwrap();
    tokio::time::timeout(Duration::from_secs(10), async {
        while g.bridge.active.lock().unwrap().contains_key(&id) {
            tokio::task::yield_now().await;
        }
    })
    .await
    .unwrap();
    (status, String::from_utf8(bytes.to_vec()).unwrap(), id)
}

async fn evidence(f: &Fixture, id: &str) -> Vec<serde_json::Value> {
    sqlx::query_scalar(
        "SELECT detail FROM core_charging.usage_events WHERE request_id=$1 ORDER BY event_id",
    )
    .bind(id)
    .fetch_all(&f.pool)
    .await
    .unwrap()
}

#[sqlx::test(migrations = false)]
async fn route_failover_pg_safe_failures_keep_one_frozen_bill(pool: PgPool) {
    let f = fixture(pool, 10_000_000).await;
    let good = CompatibilityProvider::start().await;
    let rejection = FailedSupplier::start(FailureReply::Rejected).await;
    let rate_limit = FailedSupplier::start(FailureReply::RateLimited).await;
    for (index, (first, reviewed, stream, kind)) in [
        (refused_endpoint().await, false, false, "connect"),
        (rejection.base.clone(), true, true, "server_error"),
        (rate_limit.base.clone(), true, false, "rate_limited"),
    ]
    .into_iter()
    .enumerate()
    {
        let g = failover_gateway(&f, &first, &good.endpoint, reviewed, false, false);
        let key = format!("safe-failover-{index}");
        let (status, body, id) = call(&g, &f, &key, stream, None).await;
        assert_eq!(status, 200, "{body}");
        let charge = g.bridge.charging.resolve(&id).await.unwrap().unwrap();
        assert_eq!(
            (charge.state.as_str(), charge.settled, charge.usage.input),
            ("settled", 2, 2)
        );
        assert_eq!(charge.price.group, "allowed");
        let events = evidence(&f, &id).await;
        assert_eq!(events.iter().filter(|e| e["kind"] == "final").count(), 1);
        assert_eq!(
            events
                .iter()
                .filter(|e| e["kind"] == "model_route_dispatch")
                .count(),
            2
        );
        assert!(events.iter().any(|e| {
            e["failure"]["failure"]["delivery"] == if reviewed { "rejected" } else { "not_sent" }
                && e["failure"]["failure"]["kind"] == kind
        }));
        assert_eq!(call(&g, &f, &key, stream, None).await.0, 409);
        let bodies = good.bodies.lock().unwrap();
        assert_eq!(bodies.len(), index + 1);
        assert!(bodies[index].get("service_tier").is_none());
        assert!(bodies[index].get("speed").is_none());
    }
    assert_eq!(rejection.hits.load(Ordering::SeqCst), 1);
    assert_eq!(rate_limit.hits.load(Ordering::SeqCst), 1);
    assert_balance(&f, 9_999_994, 0).await;
}

#[sqlx::test(migrations = false)]
async fn key_snapshot_failover_admin_uses_last_dispatched_binding(pool: PgPool) {
    use axum::extract::OriginalUri;
    type Dispatches = Arc<std::sync::Mutex<Vec<(String, String)>>>;

    async fn reply(
        State(keys): State<Dispatches>,
        OriginalUri(uri): OriginalUri,
        headers: HeaderMap,
        Json(_): Json<serde_json::Value>,
    ) -> axum::response::Response {
        keys.lock().unwrap().push((
            uri.path().to_owned(),
            headers[header::AUTHORIZATION].to_str().unwrap().to_owned(),
        ));
        if uri.path() == "/first/chat/completions" {
            return (
                axum::http::StatusCode::SERVICE_UNAVAILABLE,
                Json(json!({"error":{"message":"synthetic pre-acceptance rejection"}})),
            )
                .into_response();
        }
        Json(json!({
            "id":"synthetic-key-failover","object":"chat.completion",
            "choices":[{"index":0,"message":{"role":"assistant","content":"ok"},"finish_reason":"stop"}],
            "usage":{"prompt_tokens":2,"completion_tokens":3,"prompt_tokens_details":{"cached_tokens":0}}
        }))
        .into_response()
    }

    let f = fixture(pool, 10_000_000).await;
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let base = format!("http://{}", listener.local_addr().unwrap());
    let keys = Arc::new(std::sync::Mutex::new(Vec::new()));
    let app = Router::new()
        .route("/first/chat/completions", post(reply))
        .route("/second/chat/completions", post(reply))
        .with_state(keys.clone());
    let supplier = tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
    let g = failover_gateway(
        &f,
        &format!("{base}/first"),
        &format!("{base}/second"),
        true,
        false,
        false,
    );
    let primary = CredentialRef { id: 7, revision: 1 };
    let secondary = CredentialRef { id: 8, revision: 2 };
    g.bridge
        .credentials
        .append(vec![UpstreamCredential {
            reference: secondary,
            secret: "synthetic-secondary-provider-key".into(),
        }])
        .unwrap();
    g.bridge
        .routing
        .install_credential_pools(
            g.bridge.routing.version().unwrap(),
            &BTreeMap::from([(1, vec![primary, secondary]), (2, vec![primary])]),
        )
        .unwrap();
    // The large-model route only visits upstream 1. Advance its pool so the
    // failing primary uses index 1; the actual fallback uses a single key at 0.
    let _first = g
        .bridge
        .routing
        .route(routing::RouteRequest {
            model: "large-model",
            group: "allowed",
            speed: None,
            protocol: Protocol::OpenAiChat,
            replay: routing::ReplayPermission::Denied,
        })
        .unwrap();

    let (status, body, id) = call(&g, &f, "key-snapshot-two-dispatches", false, None).await;
    assert_eq!(status, 200, "{body}");
    assert_eq!(
        *keys.lock().unwrap(),
        [
            (
                "/first/chat/completions".to_owned(),
                "Bearer synthetic-secondary-provider-key".to_owned(),
            ),
            (
                "/second/chat/completions".to_owned(),
                "Bearer synthetic-selected-bound-provider-key".to_owned(),
            ),
        ],
        "both attempts must actually reach the local supplier with different credentials"
    );
    supplier.abort();

    let events = evidence(&f, &id).await;
    let contract = events
        .iter()
        .find(|event| event["kind"] == "model_reservation_contract")
        .unwrap();
    let first = &contract["binding"];
    let last = &contract["fallbacks"][0]["binding"];
    assert_eq!(first["credential_ref"], json!(secondary));
    assert_eq!(
        first["credential_selection"],
        json!({"is_multi_key":true,"multi_key_index":1})
    );
    assert_eq!(last["credential_ref"], json!(primary));
    assert_eq!(
        last["credential_selection"],
        json!({"is_multi_key":false,"multi_key_index":0})
    );
    let dispatches: Vec<_> = events
        .iter()
        .filter(|event| event["kind"] == "model_route_dispatch")
        .collect();
    assert_eq!(dispatches.len(), 2);
    assert_eq!(dispatches[0]["number"], 1);
    assert_eq!(dispatches[0]["inbound_protocol"], "chat");
    assert_eq!(dispatches[1]["inbound_protocol"], "chat");
    assert_eq!(dispatches[0]["route_id"], first["route_id"]);
    assert_eq!(dispatches[0]["wire_sha256"], contract["wire_sha256"]);
    assert_eq!(dispatches[1]["number"], 2);
    assert_eq!(dispatches[1]["route_id"], last["route_id"]);
    assert_eq!(
        dispatches[1]["wire_sha256"],
        contract["fallbacks"][0]["wire_sha256"]
    );
    let failure = events
        .iter()
        .find(|event| event["kind"] == "model_route_failure")
        .unwrap();
    assert_eq!(failure["number"], 1);
    assert_eq!(failure["route_id"], first["route_id"]);
    assert_eq!(failure["failure"]["http_status"], 503);
    assert_eq!(failure["failure"]["failure"]["delivery"], "rejected");

    let admin = IdentityStore::from_pool(f.pool.clone())
        .bootstrap_user(9, 5)
        .await
        .unwrap();
    let response = crate::usage::router(crate::usage::UsageStore::new(f.pool.clone()))
        .oneshot(
            axum::http::Request::get(format!("/core/v1/usage/admin/request?request_id={id}"))
                .header("authorization", format!("Bearer {}", admin.secret))
                .body(axum::body::Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), axum::http::StatusCode::OK);
    let bytes = axum::body::to_bytes(response.into_body(), 1 << 20)
        .await
        .unwrap();
    let response: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
    let other: serde_json::Value =
        serde_json::from_str(response["data"]["other"].as_str().unwrap()).unwrap();
    let summary = &other["admin_info"];
    assert_eq!(response["data"]["channel"], 2);
    assert_eq!(summary["key_snapshot_status"], "recorded");
    assert_eq!(summary["is_multi_key"], false);
    assert_eq!(summary["multi_key_index"], 0);
    assert_eq!(summary["route_attempts"].as_array().unwrap().len(), 2);
    for attempt in summary["route_attempts"].as_array().unwrap() {
        assert_eq!(attempt["inbound_protocol"], "chat");
        assert_eq!(attempt["protocol"], "chat");
        assert_eq!(attempt["protocol_path"], "same_protocol");
    }
    assert_eq!(summary["route_attempts"][0]["is_multi_key"], true);
    assert_eq!(summary["route_attempts"][0]["multi_key_index"], 1);
    assert_eq!(summary["route_attempts"][1]["is_multi_key"], false);
    assert_eq!(summary["route_attempts"][1]["multi_key_index"], 0);
    let encoded = response.to_string();
    for forbidden in [
        "credential_ref",
        "credential_selection",
        "synthetic-secondary-provider-key",
        "synthetic-selected-bound-provider-key",
    ] {
        assert!(!encoded.contains(forbidden), "{forbidden}");
    }
    let charge = receipt(&g, &id).await;
    assert_eq!((charge.state.as_str(), charge.settled), ("settled", 2));
    assert_eq!(requests(&f.pool).await, 1);
    assert_balance(&f, 9_999_998, 0).await;
}

#[sqlx::test(migrations = false)]
async fn route_failover_pg_unknown_and_partial_tool_results_never_replay(pool: PgPool) {
    let f = fixture(pool, 10_000_000).await;
    let good = CompatibilityProvider::start().await;
    for (index, (mode, reviewed, stream)) in [
        (FailureReply::Rejected, false, false),
        (FailureReply::ToolEvidence, true, false),
        (FailureReply::NestedToolEvidence, true, false),
        (FailureReply::Timeout, true, false),
        (FailureReply::Partial, true, true),
    ]
    .into_iter()
    .enumerate()
    {
        let bad = FailedSupplier::start(mode).await;
        let g = failover_gateway(&f, &bad.base, &good.endpoint, reviewed, false, false);
        let (_, body, id) = call(&g, &f, &format!("unknown-failover-{index}"), stream, None).await;
        let charge = g.bridge.charging.resolve(&id).await.unwrap().unwrap();
        assert_ne!(charge.state, "settled", "{body}");
        assert_eq!(charge.settled, 0);
        assert!(charge.reserved > 0);
        assert_eq!(bad.hits.load(Ordering::SeqCst), 1);
        assert!(good.bodies.lock().unwrap().is_empty());
        assert_eq!(g.reconcile_completed_streams(10).await.unwrap(), 0);
    }
}

async fn cross_group_case(pool: PgPool, flag: bool, granted: bool) {
    let f = fixture(pool, 10_000_000).await;
    // fixture() already grants developer access through bootstrap_user().
    if granted {
        sqlx::query("INSERT INTO core_identity.user_routing_groups(user_id,group_name,enabled,source_reference) VALUES(7,'backup',TRUE,'route-failover-test')").execute(&f.pool).await.unwrap();
    }
    sqlx::query("INSERT INTO core_identity.api_key_settings(credential_id,name,group_name,auto_groups,cross_group_retry) VALUES($1,'fallback-test','auto',ARRAY['allowed','backup'],$2)").bind(f.key_id).bind(flag).execute(&f.pool).await.unwrap();
    let good = CompatibilityProvider::start().await;
    let g = failover_gateway(
        &f,
        &refused_endpoint().await,
        &good.endpoint,
        false,
        true,
        false,
    );
    let (status, body, id) = call(&g, &f, "cross-group", false, None).await;
    if flag && granted {
        assert_eq!(status, 200, "{body}");
        let charge = g.bridge.charging.resolve(&id).await.unwrap().unwrap();
        assert_eq!(
            (
                charge.settled,
                charge.price.input_per_million,
                charge.price.group.as_str()
            ),
            (2, 1_000_000, "allowed")
        );
        assert!(
            evidence(&f, &id)
                .await
                .iter()
                .any(|e| e["kind"] == "model_route_dispatch" && e["actual_group"] == "backup")
        );
        assert_balance(&f, 9_999_998, 0).await;
    } else {
        assert_ne!(status, 200, "{body}");
        assert!(good.bodies.lock().unwrap().is_empty());
        let charge = g.bridge.charging.resolve(&id).await.unwrap().unwrap();
        assert_eq!((charge.state.as_str(), charge.settled), ("settled", 0));
        assert_balance(&f, 10_000_000, 0).await;
    }
}

#[sqlx::test(migrations = false)]
async fn route_failover_pg_ping_keeps_confirmed_rejection_free_without_replay(pool: PgPool) {
    let f = fixture(pool, 10_000_000).await;
    let good = CompatibilityProvider::start().await;
    let release = Arc::new(tokio::sync::Notify::new());
    let app = Router::new()
        .route(
            "/v1/chat/completions",
            post(
                |State(release): State<Arc<tokio::sync::Notify>>| async move {
                    release.notified().await;
                    (
                        axum::http::StatusCode::SERVICE_UNAVAILABLE,
                        Json(json!({"error":{"message":"pre-acceptance rejection"}})),
                    )
                },
            ),
        )
        .with_state(release.clone());
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let base = format!("http://{}/v1", listener.local_addr().unwrap());
    let supplier = tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
    let g = failover_gateway(&f, &base, &good.endpoint, true, false, false);
    let id = crate::hex_codec::encode(Sha256::digest("ping-before-rejection"));
    let (context, mut request) = admit(&g, &f, &id, "small-model", 16).await;
    request.stream = true;
    request.has_sse_policy = true;
    request.sse_ping = Some(Duration::from_millis(10));
    let relay::Session {
        head,
        body,
        completion,
        ..
    } = g.relay.start(request, context).unwrap();
    assert_eq!(head.await.unwrap().unwrap().status, 200);
    release.notify_one();
    let output = axum::body::to_bytes(axum::body::Body::from_stream(body), 1 << 20)
        .await
        .unwrap();
    assert!(std::str::from_utf8(&output).unwrap().contains(": PING"));
    let done = completion.await.unwrap();
    assert_eq!(done.report.delivery, routing::DeliveryState::Rejected);
    assert_eq!(done.settlement, relay::Settlement::Confirmed);
    assert_eq!(done.report.attempts, 1);
    assert!(good.bodies.lock().unwrap().is_empty());
    let charge = receipt(&g, &id).await;
    assert!(charge.reserved > 0);
    assert_eq!((charge.state.as_str(), charge.settled), ("settled", 0));
    assert!(
        evidence(&f, &id).await.iter().any(|e| {
            e["kind"] == "model_no_acceptance" && e["receipt"]["outcome"] == "rejected"
        })
    );
    assert_balance(&f, 10_000_000, 0).await;
    supplier.abort();
}

#[sqlx::test(migrations = false)]
async fn route_failover_pg_confirmed_rejection_zero_recovers_after_restart(pool: PgPool) {
    let f = fixture(pool, 10_000_000).await;
    let bad = FailedSupplier::start(FailureReply::Rejected).await;
    let good = CompatibilityProvider::start().await;
    // Native key has no cross-group permission, so the reviewed rejection is
    // final. The isolated DB fails settlement after the durable zero checkpoint.
    let initial = failover_gateway(&f, &bad.base, &good.endpoint, true, true, false);
    sqlx::raw_sql("CREATE FUNCTION public.block_route_settlement() RETURNS trigger LANGUAGE plpgsql AS $$ BEGIN IF NEW.state='settled' THEN RAISE EXCEPTION 'synthetic settlement failure'; END IF; RETURN NEW; END $$; CREATE TRIGGER block_route_settlement BEFORE UPDATE ON core_charging.requests FOR EACH ROW EXECUTE FUNCTION public.block_route_settlement()")
        .execute(&f.pool).await.unwrap();
    let (_, _, id) = call(&initial, &f, "rejection-restart", false, None).await;
    let pending = initial.bridge.charging.resolve(&id).await.unwrap().unwrap();
    assert_eq!(pending.state, "reconcile");
    assert!(
        evidence(&f, &id)
            .await
            .iter()
            .any(|e| e["kind"] == "model_no_acceptance" && e["receipt"]["outcome"] == "rejected")
    );
    drop(initial);
    sqlx::raw_sql("DROP TRIGGER block_route_settlement ON core_charging.requests; DROP FUNCTION public.block_route_settlement()")
        .execute(&f.pool).await.unwrap();
    let restarted = failover_gateway(&f, &bad.base, &good.endpoint, true, true, false);
    assert_eq!(restarted.reconcile_completed_streams(10).await.unwrap(), 1);
    assert_eq!(restarted.reconcile_completed_streams(10).await.unwrap(), 0);
    let charge = restarted
        .bridge
        .charging
        .resolve(&id)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(
        (
            charge.state.as_str(),
            charge.settled,
            charge.outcome.as_deref()
        ),
        ("settled", 0, Some("failed"))
    );
    assert_eq!(bad.hits.load(Ordering::SeqCst), 1);
    assert!(good.bodies.lock().unwrap().is_empty());
    assert_balance(&f, 10_000_000, 0).await;
}

#[sqlx::test(migrations = false)]
async fn route_failover_pg_cross_group_keeps_initial_sale(pool: PgPool) {
    cross_group_case(pool, true, true).await;
}
#[sqlx::test(migrations = false)]
async fn route_failover_pg_cross_group_requires_key_opt_in(pool: PgPool) {
    cross_group_case(pool, false, true).await;
}
#[sqlx::test(migrations = false)]
async fn route_failover_pg_cross_group_never_expands_grants(pool: PgPool) {
    cross_group_case(pool, true, false).await;
}

#[sqlx::test(migrations = false)]
async fn route_affinity_pg_explicit_session_is_stable_and_not_disclosed(pool: PgPool) {
    let f = fixture(pool, 10_000_000).await;
    let a = CompatibilityProvider::start().await;
    let b = CompatibilityProvider::start().await;
    let g = failover_gateway(&f, &a.endpoint, &b.endpoint, false, false, true);
    let session = "private-explicit-session-never-persist";
    let mut route = None;
    for index in 0..3 {
        let (status, body, id) =
            call(&g, &f, &format!("affinity-{index}"), false, Some(session)).await;
        assert_eq!(status, 200, "{body}");
        let events = evidence(&f, &id).await;
        let dispatched = events
            .iter()
            .find(|e| e["kind"] == "model_route_dispatch")
            .unwrap()["route_id"]
            .clone();
        if let Some(expected) = &route {
            assert_eq!(&dispatched, expected);
        } else {
            route = Some(dispatched);
        }
        assert!(!serde_json::to_string(&events).unwrap().contains(session));
    }
    assert!(a.bodies.lock().unwrap().is_empty() || b.bodies.lock().unwrap().is_empty());
    assert_eq!(
        a.bodies.lock().unwrap().len() + b.bodies.lock().unwrap().len(),
        3
    );
}

#[sqlx::test(migrations = false)]
async fn key_pool_pg_forwarding_and_reservations_use_the_same_revision(pool: PgPool) {
    async fn reply(
        State(keys): State<Arc<std::sync::Mutex<Vec<String>>>>,
        headers: HeaderMap,
        Json(_body): Json<serde_json::Value>,
    ) -> Json<serde_json::Value> {
        keys.lock()
            .unwrap()
            .push(headers[header::AUTHORIZATION].to_str().unwrap().to_owned());
        Json(json!({
            "id":"synthetic-key-pool","object":"chat.completion",
            "choices":[{"index":0,"message":{"role":"assistant","content":"pool response"},"finish_reason":"stop"}],
            "usage":{"prompt_tokens":2,"completion_tokens":3,"prompt_tokens_details":{"cached_tokens":0}}
        }))
    }
    let f = fixture(pool, 10_000_000).await;
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let base = format!("http://{}/v1", listener.local_addr().unwrap());
    let keys = Arc::new(std::sync::Mutex::new(Vec::new()));
    let app = Router::new()
        .route("/v1/chat/completions", post(reply))
        .with_state(keys.clone());
    let supplier = tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
    let g = gateway(&f, &base, vec![]).with_local_streaming();
    let primary = CredentialRef { id: 7, revision: 1 };
    let secondary = CredentialRef { id: 8, revision: 2 };
    g.bridge
        .credentials
        .append(vec![UpstreamCredential {
            reference: secondary,
            secret: "synthetic-pool-secondary".into(),
        }])
        .unwrap();
    g.bridge
        .routing
        .install_credential_pools(
            g.bridge.routing.version().unwrap(),
            &BTreeMap::from([(1, vec![primary, secondary])]),
        )
        .unwrap();

    for (index, reference) in [primary, secondary, primary].into_iter().enumerate() {
        let (status, body, id) = call(&g, &f, &format!("key-pool-{index}"), false, None).await;
        assert_eq!(status, 200, "{body}");
        let persisted: serde_json::Value = sqlx::query_scalar(
            "SELECT detail FROM core_charging.usage_events WHERE request_id=$1 AND event_id='reservation-contract:v1'",
        ).bind(&id).fetch_one(&f.pool).await.unwrap();
        assert_eq!(persisted["binding"]["credential_ref"], json!(reference));
        let charge = receipt(&g, &id).await;
        assert_eq!((charge.state.as_str(), charge.settled), ("settled", 2));
    }
    assert_eq!(
        *keys.lock().unwrap(),
        [
            "Bearer synthetic-selected-bound-provider-key",
            "Bearer synthetic-pool-secondary",
            "Bearer synthetic-selected-bound-provider-key",
        ]
    );
    assert_eq!(requests(&f.pool).await, 3);
    assert_balance(&f, 9_999_994, 0).await;
    supplier.abort();
}

#[path = "channel_affinity_tests.rs"]
mod channel_affinity_tests;

#[sqlx::test(migrations = false)]
async fn reliability_pg_retry_after_is_shared_and_preserves_the_manual_stop(pool: PgPool) {
    let f = fixture(pool, 10_000_000).await;
    let good = CompatibilityProvider::start().await;
    let bad = FailedSupplier::start(FailureReply::RetryAfter).await;
    let gateway = || {
        let g = failover_gateway(&f, &bad.base, &good.endpoint, true, false, false);
        let mut d = g.bridge.routing.catalog_document().unwrap();
        let expected = g.bridge.routing.version().unwrap();
        d.routing.version += 1;
        d.routing.routing_reliability = Some(serde_json::from_value(json!({
            "retry_times":1,"retry_status_codes":"429","auto_disable":true,
            "disable_status_codes":"429","disable_keywords":[],"disable_threshold_seconds":5,
            "probe_enabled":false,"probe_mode":"passive_recovery","probe_interval_seconds":30,"auto_recover":false
        })).unwrap());
        g.bridge
            .routing
            .publish(
                expected,
                d.routing,
                Arc::new(crate::routing::PriceSnapshot::compile(d.pricing).unwrap()),
            )
            .unwrap();
        g
    };
    let first = gateway();
    let (status, body, id) = call(&first, &f, "cooldown-first", false, None).await;
    assert_eq!(status, 200, "{body}");
    let events = evidence(&f, &id).await;
    assert_eq!(
        events
            .iter()
            .filter(|e| e["kind"] == "model_route_dispatch")
            .count(),
        2
    );
    assert_eq!(events.iter().filter(|e| e["kind"] == "final").count(), 1);
    assert_eq!(call(&first, &f, "cooldown-first", false, None).await.0, 409);
    let valid: bool = sqlx::query_scalar("SELECT disabled_scope IS NULL AND reason='http_status' AND status_code=429 AND next_probe_at > clock_timestamp() AND next_probe_at <= clock_timestamp()+interval '121 seconds' FROM core_channels.route_health WHERE channel_id=1 AND credential_id=7 AND credential_revision=1").fetch_one(&f.pool).await.unwrap();
    assert!(
        valid,
        "supplier Retry-After must survive as a soft deadline"
    );
    drop(first);
    let second = gateway();
    let (status, body, id) = call(&second, &f, "cooldown-other-gateway", false, None).await;
    assert_eq!(status, 200, "{body}");
    assert_eq!(bad.hits.load(Ordering::SeqCst), 1);
    assert_eq!(
        evidence(&f, &id)
            .await
            .iter()
            .filter(|e| e["kind"] == "model_route_dispatch")
            .count(),
        1
    );
    let expire = || {
        sqlx::query("UPDATE core_channels.route_health SET next_probe_at=clock_timestamp()-interval '1 second' WHERE channel_id=1 AND reason='http_status' AND disabled_scope IS NULL").execute(&f.pool)
    };
    assert_eq!(expire().await.unwrap().rows_affected(), 1);
    let (status, body, _) = call(&second, &f, "cooldown-expired", false, None).await;
    assert_eq!(status, 200, "{body}");
    assert_eq!(bad.hits.load(Ordering::SeqCst), 2);
    second
        .bridge
        .routing
        .set_operational_enabled(second.bridge.routing.version().unwrap(), 1, false)
        .unwrap();
    assert_eq!(expire().await.unwrap().rows_affected(), 1);
    let (status, body, _) = call(&second, &f, "cooldown-operator-stop", false, None).await;
    assert_eq!(status, 200, "{body}");
    assert_eq!(bad.hits.load(Ordering::SeqCst), 2);
    assert_eq!(good.bodies.lock().unwrap().len(), 4);
}

#[sqlx::test(migrations = false)]
async fn reliability_pg_401_quarantines_key_and_survives_restart(pool: PgPool) {
    let f = fixture(pool, 10_000_000).await;
    let good = CompatibilityProvider::start().await;
    let bad = FailedSupplier::start(FailureReply::Unauthorized).await;
    let gateway = || {
        let g = failover_gateway(&f, &bad.base, &good.endpoint, true, false, false);
        let mut d = g.bridge.routing.catalog_document().unwrap();
        let expected = g.bridge.routing.version().unwrap();
        d.routing.version += 1;
        d.routing.routing_reliability=Some(serde_json::from_value(json!({
           "retry_times":1,"retry_status_codes":"401","auto_disable":true,"disable_status_codes":"401",
           "disable_keywords":[],"disable_threshold_seconds":5,"probe_enabled":true,"probe_mode":"passive_recovery",
           "probe_interval_seconds":30,"auto_recover":true
        })).unwrap());
        g.bridge
            .routing
            .publish(
                expected,
                d.routing,
                Arc::new(crate::routing::PriceSnapshot::compile(d.pricing).unwrap()),
            )
            .unwrap();
        g
    };
    let first = gateway();
    let (status, body, id) = call(&first, &f, "quarantine-one", false, None).await;
    assert_eq!(status, 200, "{body}");
    let events = evidence(&f, &id).await;
    assert_eq!(
        events
            .iter()
            .filter(|e| e["kind"] == "model_route_dispatch")
            .count(),
        2
    );
    assert_eq!(events.iter().filter(|e| e["kind"] == "final").count(), 1);
    let state: (String, i16) = sqlx::query_as(
        "SELECT disabled_scope,status_code FROM core_channels.route_health WHERE channel_id=1",
    )
    .fetch_one(&f.pool)
    .await
    .unwrap();
    assert_eq!(state, ("credential".into(), 401));
    drop(first);
    let second = gateway();
    let (status, body, id) = call(&second, &f, "quarantine-two", false, None).await;
    assert_eq!(status, 200, "{body}");
    assert_eq!(bad.hits.load(Ordering::SeqCst), 1);
    assert_eq!(
        evidence(&f, &id)
            .await
            .iter()
            .filter(|e| e["kind"] == "model_route_dispatch")
            .count(),
        1
    );
}

#[sqlx::test(migrations = false)]
async fn reliability_pg_probe_single_owner_and_stale_result(pool: PgPool) {
    sqlx::raw_sql(
        r#"
        CREATE SCHEMA core_channels;
        CREATE TABLE public.routing_active_release(config_version bigint);
        INSERT INTO public.routing_active_release VALUES (23);
        CREATE FUNCTION public.routing_control_read() RETURNS jsonb
        LANGUAGE sql STABLE SECURITY DEFINER SET search_path=pg_catalog AS $$
            SELECT jsonb_build_object('routing',jsonb_build_object('version',config_version))
            FROM public.routing_active_release
        $$;
        REVOKE ALL ON FUNCTION public.routing_control_read() FROM PUBLIC;
    "#,
    )
    .execute(&pool)
    .await
    .unwrap();
    sqlx::raw_sql(include_str!("../../schema/route_health.sql"))
        .execute(&pool)
        .await
        .unwrap();
    let role = format!("lmm_health_{:016x}", rand::random::<u64>());
    sqlx::raw_sql(sqlx::AssertSqlSafe(format!(
        "CREATE ROLE {role}; GRANT {role} TO CURRENT_USER; GRANT USAGE ON SCHEMA core_channels TO {role};
         GRANT SELECT,INSERT,UPDATE ON core_channels.route_health TO {role};
         GRANT EXECUTE ON FUNCTION public.routing_control_read() TO {role}"
    ))).execute(&pool).await.unwrap();
    let limited = PgPool::connect_with(
        pool.connect_options()
            .as_ref()
            .clone()
            .options([("role", role.as_str())]),
    )
    .await
    .unwrap();
    assert!(
        !sqlx::query_scalar::<_, bool>(
            "SELECT has_table_privilege('public.routing_active_release','SELECT')"
        )
        .fetch_one(&limited)
        .await
        .unwrap()
    );
    async fn claim(pool: &PgPool, id: i64) -> Option<i64> {
        use sqlx::Row;
        sqlx::query(include_str!("channel_health_claim.sql"))
            .bind(23_i64)
            .bind(vec![id])
            .bind(vec![9_i64])
            .bind(vec![3_i64])
            .bind(true)
            .bind(5_f64)
            .fetch_optional(pool)
            .await
            .unwrap()
            .map(|r| r.get("generation"))
    }
    async fn complete(pool: &PgPool, generation: i64) -> u64 {
        sqlx::query(include_str!("channel_health_complete.sql"))
            .bind(23_i64)
            .bind(1_i64)
            .bind(9_i64)
            .bind(3_i64)
            .bind(generation)
            .bind(50_i64)
            .bind(30_f64)
            .bind(true)
            .bind(None::<String>)
            .bind(None::<String>)
            .bind(Some(200_i16))
            .execute(pool)
            .await
            .unwrap()
            .rows_affected()
    }
    let (a, b) = tokio::join!(claim(&limited, 1), claim(&limited, 1));
    assert_ne!(a.is_some(), b.is_some());
    let owner = a.or(b).unwrap();
    sqlx::query(
        "UPDATE core_channels.route_health SET disabled_scope='credential',generation=generation+1",
    )
    .execute(&pool)
    .await
    .unwrap();
    assert_eq!(complete(&limited, owner).await, 0);
    sqlx::query("UPDATE core_channels.route_health SET lease_until=NULL")
        .execute(&pool)
        .await
        .unwrap();
    let current = claim(&limited, 1).await.unwrap();
    sqlx::query("UPDATE public.routing_active_release SET config_version=24")
        .execute(&pool)
        .await
        .unwrap();
    assert_eq!(complete(&limited, current).await, 0);
    assert_eq!(claim(&limited, 1).await, None);
    let scope: String = sqlx::query_scalar("SELECT disabled_scope FROM core_channels.route_health")
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(scope, "credential");
    limited.close().await;
    sqlx::raw_sql(sqlx::AssertSqlSafe(format!(
        "DROP OWNED BY {role}; DROP ROLE {role}"
    )))
    .execute(&pool)
    .await
    .unwrap();
}

//! Real local suppliers and PostgreSQL: one logical bill across safe attempts.
use super::*;
use axum::{Json, Router, extract::State, response::IntoResponse, routing::post};
use std::sync::atomic::{AtomicUsize, Ordering};
use tokio::net::TcpListener;
use tower::ServiceExt;

#[derive(Clone, Copy)]
enum FailureReply {
    Rejected,
    ToolEvidence,
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
                FailureReply::Rejected => (axum::http::StatusCode::SERVICE_UNAVAILABLE, Json(json!({"error":{"message":"synthetic pre-acceptance rejection"}}))).into_response(),
                FailureReply::ToolEvidence => (axum::http::StatusCode::SERVICE_UNAVAILABLE, Json(json!({"error":{"message":"synthetic failure after work"},"usage":{"prompt_tokens":1},"tool_calls":[{"id":"already-called"}]}))).into_response(),
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
        context.pre_acceptance_rejection_statuses = vec![503];
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
    let response = g.router().oneshot(request.body(axum::body::Body::from(json!({"model":"small-model","messages":[{"role":"user","content":"hello"}],"max_tokens":16,"stream":stream}).to_string())).unwrap()).await.unwrap();
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
    for (index, (first, reviewed, stream)) in [
        (refused_endpoint().await, false, false),
        (rejection.base.clone(), true, true),
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
        assert!(events.iter().any(|e| e["failure"]["failure"]["delivery"]
            == if reviewed { "rejected" } else { "not_sent" }));
        assert_eq!(call(&g, &f, &key, stream, None).await.0, 409);
        assert_eq!(good.bodies.lock().unwrap().len(), index + 1);
    }
    assert_eq!(rejection.hits.load(Ordering::SeqCst), 1);
    assert_balance(&f, 9_999_996, 0).await;
}

#[sqlx::test(migrations = false)]
async fn route_failover_pg_unknown_and_partial_tool_results_never_replay(pool: PgPool) {
    let f = fixture(pool, 10_000_000).await;
    let good = CompatibilityProvider::start().await;
    for (index, (mode, reviewed, stream)) in [
        (FailureReply::Rejected, false, false),
        (FailureReply::ToolEvidence, true, false),
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

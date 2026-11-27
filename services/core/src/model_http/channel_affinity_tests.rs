//! Two independently constructed gateways share only PostgreSQL and loopback suppliers.
use super::*;
use crate::model_http::{ModelIdentity, session_affinity};
use axum::extract::OriginalUri;
use serde_json::Value;
use std::sync::{Mutex, atomic::AtomicBool};

fn policies(gateway: &ModelGateway, keep: bool, switch: bool, capacity: usize) {
    let router = &gateway.bridge.routing;
    let version = router.version().unwrap();
    let mut doc = router.catalog_document().unwrap();
    doc.routing.version += 1;
    let rule = |name: &str, skip: bool| {
        json!({
            "name":name,"model_regex":["^small-model$"],"key_sources":[{"type":"gjson","path":"prompt_cache_key"}],
            "value_regex":format!("^{name}-"),"skip_retry_on_failure":skip,
            "include_using_group":true,"include_rule_name":true,"include_model_name":false
        })
    };
    doc.routing.channel_affinity = Some(serde_json::from_value(json!({
        "enabled":true,"max_entries":capacity,"default_ttl_seconds":3600,"switch_on_success":switch,
        "keep_on_channel_disabled":keep,"rules":[rule("retry",false),rule("single",true)]
    })).unwrap());
    doc.routing.routing_reliability = Some(serde_json::from_value(json!({
        "retry_times":1,"retry_status_codes":"401","auto_disable":false,"disable_status_codes":"401",
        "disable_keywords":[],"disable_threshold_seconds":5,"probe_enabled":false,
        "probe_mode":"passive_recovery","probe_interval_seconds":30,"auto_recover":false
    })).unwrap());
    router
        .publish(
            version,
            doc.routing,
            Arc::new(PriceSnapshot::compile(doc.pricing).unwrap()),
        )
        .unwrap();
}

async fn model_call(gateway: &ModelGateway, f: &Fixture, key: &str, id: &str) -> (u16, String) {
    let body = json!({"model":"small-model","prompt_cache_key":key,"max_tokens":16,
        "messages":[{"role":"user","content":"hello"}]});
    let request = axum::http::Request::builder()
        .method("POST")
        .uri("/v1/chat/completions")
        .header("authorization", format!("Bearer {}", f.key))
        .header("idempotency-key", id)
        .header("content-type", "application/json")
        .body(axum::body::Body::from(body.to_string()))
        .unwrap();
    let response = gateway.router().oneshot(request).await.unwrap();
    let status = response.status().as_u16();
    let id = response.headers()["x-request-id"]
        .to_str()
        .unwrap()
        .to_owned();
    let _body = axum::body::to_bytes(response.into_body(), 1 << 20)
        .await
        .unwrap();
    tokio::time::timeout(Duration::from_secs(5), async {
        while gateway.bridge.active.lock().unwrap().contains_key(&id) {
            tokio::task::yield_now().await;
        }
    })
    .await
    .unwrap();
    (status, id)
}

#[sqlx::test(migrations = false)]
async fn rule_affinity_pg_401_switches_only_after_success_across_nodes(pool: PgPool) {
    type StateData = (Arc<AtomicBool>, Arc<Mutex<Vec<String>>>);
    async fn supplier(
        State((reject, hits)): State<StateData>,
        OriginalUri(uri): OriginalUri,
        Json(payload): Json<Value>,
    ) -> axum::response::Response {
        assert!(payload["prompt_cache_key"].is_string());
        hits.lock().unwrap().push(uri.path().to_owned());
        if uri.path().starts_with("/first/") && reject.load(Ordering::SeqCst) {
            return (
                axum::http::StatusCode::UNAUTHORIZED,
                Json(json!({"error":{"message":"synthetic invalid key"}})),
            )
                .into_response();
        }
        Json(json!({"id":"synthetic-affinity","object":"chat.completion",
            "choices":[{"index":0,"message":{"role":"assistant","content":"ok"},"finish_reason":"stop"}],
            "usage":{"prompt_tokens":2,"completion_tokens":3,"prompt_tokens_details":{"cached_tokens":0}}})).into_response()
    }
    let f = fixture(pool, 10_000_000).await;
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let base = format!("http://{}", listener.local_addr().unwrap());
    let reject = Arc::new(AtomicBool::new(false));
    let hits = Arc::new(Mutex::new(Vec::new()));
    let service = Router::new()
        .route("/first/chat/completions", post(supplier))
        .route("/second/chat/completions", post(supplier))
        .with_state((reject.clone(), hits.clone()));
    let server = tokio::spawn(async move {
        axum::serve(listener, service).await.unwrap();
    });
    let node = || {
        let gateway = failover_gateway(
            &f,
            &format!("{base}/first"),
            &format!("{base}/second"),
            true,
            false,
            false,
        );
        policies(&gateway, false, true, 10);
        gateway
    };
    let sender = node();
    let other = node();
    for key in ["retry-private", "single-private"] {
        assert_eq!(
            model_call(&sender, &f, key, &format!("seed-{key}")).await.0,
            200
        );
    }
    hits.lock().unwrap().clear();
    reject.store(true, Ordering::SeqCst);
    let (ok, id) = model_call(&sender, &f, "retry-private", "failover-once").await;
    assert_eq!(ok, 200);
    assert_eq!(
        hits.lock().unwrap().as_slice(),
        ["/first/chat/completions", "/second/chat/completions"]
    );
    let events = evidence(&f, &id).await;
    assert_eq!(events.iter().filter(|e| e["kind"] == "final").count(), 1);
    assert_eq!(
        events
            .iter()
            .filter(|e| e["kind"] == "model_route_dispatch")
            .count(),
        2
    );
    let charge = sender.bridge.charging.resolve(&id).await.unwrap().unwrap();
    assert_eq!((charge.state.as_str(), charge.settled), ("settled", 2));

    hits.lock().unwrap().clear();
    reject.store(false, Ordering::SeqCst);
    assert_eq!(
        model_call(&other, &f, "retry-private", "node-two").await.0,
        200
    );
    assert_eq!(
        hits.lock().unwrap().as_slice(),
        ["/second/chat/completions"]
    );
    hits.lock().unwrap().clear();
    reject.store(true, Ordering::SeqCst);
    let (status, single) = model_call(&other, &f, "single-private", "no-replay").await;
    assert_ne!(status, 200);
    assert_eq!(hits.lock().unwrap().as_slice(), ["/first/chat/completions"]);
    assert_eq!(
        evidence(&f, &single)
            .await
            .iter()
            .filter(|e| e["kind"] == "model_route_dispatch")
            .count(),
        1
    );
    let found: Vec<(String, i64)> = sqlx::query_as(
        "SELECT rule_name,channel_id FROM core_channels.affinity_cache ORDER BY rule_name",
    )
    .fetch_all(&f.pool)
    .await
    .unwrap();
    assert_eq!(found, vec![("retry".into(), 2), ("single".into(), 1)]);
    server.abort();
}

#[sqlx::test(migrations = false)]
async fn rule_affinity_pg_scopes_expiry_capacity_and_rule_eviction(pool: PgPool) {
    use crate::routing::affinity::{Config, Input, Runtime, store};
    let f = fixture(pool, 10_000_000).await;
    let rule = |name: &str, pattern: &str, source: Value| {
        json!({
            "name":name,"model_regex":[pattern],"key_sources":[source],
            "ttl_seconds":if name=="claude"{90}else{0},
            "skip_retry_on_failure":true,"include_using_group":true,
            "include_rule_name":true,"include_model_name":false
        })
    };
    let config: Config = serde_json::from_value(json!({
        "enabled":true,"max_entries":2,"default_ttl_seconds":3600,
        "switch_on_success":true,"keep_on_channel_disabled":false,"rules":[
            rule("gpt","^gpt-.*$",json!({"type":"gjson","path":"prompt_cache_key"})),
            rule("claude","^claude-.*$",json!({"type":"gjson","path":"metadata.user_id"})),
            rule("context","^context$",json!({"type":"context_int","key":"user_id"}))
        ]
    }))
    .unwrap();
    let policy = Runtime::compile(Some(config), None).unwrap();
    let headers = HeaderMap::new();
    let body = json!({"prompt_cache_key":"same-value","metadata":{"user_id":"same-value"}});
    let bind = |tenant, model, group| {
        policy
            .bind(
                &Input {
                    model,
                    path: "/v1/test",
                    tenant_id: tenant,
                    user_id: 11,
                    key_id: Some(15),
                    headers: &headers,
                    body: &body,
                },
                group,
            )
            .unwrap()
    };
    let a = bind(7, "gpt-test", "primary");
    let b = bind(7, "claude-test", "primary");
    assert_ne!(a.key, b.key);
    assert_ne!(a.key, bind(8, "gpt-test", "primary").key);
    assert_ne!(a.key, bind(7, "gpt-test", "backup").key);
    assert_ne!(a.key, bind(7, "context", "primary").key);
    assert_eq!(a.ttl, 3600);
    assert_eq!(b.ttl, 90);
    let revision = [3; 32];
    store::success(&f.pool, &a, 1, &revision).await.unwrap();
    store::success(&f.pool, &b, 2, &revision).await.unwrap();
    let c = bind(8, "gpt-test", "primary");
    store::success(&f.pool, &c, 3, &revision).await.unwrap();
    assert!(store::lookup(&f.pool, &a.key).await.unwrap().is_none());
    assert_eq!(
        store::lookup(&f.pool, &c.key)
            .await
            .unwrap()
            .unwrap()
            .channel,
        3
    );
    let mut tx = f.pool.begin().await.unwrap();
    assert_eq!(store::clear(&mut tx, Some("claude")).await.unwrap(), 1);
    tx.commit().await.unwrap();
    assert!(store::lookup(&f.pool, &b.key).await.unwrap().is_none());
    let stats =
        serde_json::to_value(store::stats(&f.pool, policy.config.as_ref()).await.unwrap()).unwrap();
    assert_eq!(stats["total"], 1);
    assert_eq!(stats["by_rule_name"]["gpt"], 1);
    assert!(!stats.to_string().contains("same-value"));
    sqlx::query(
        "UPDATE core_channels.affinity_cache SET expires_at=clock_timestamp()-interval '1 second'",
    )
    .execute(&f.pool)
    .await
    .unwrap();
    assert!(store::lookup(&f.pool, &c.key).await.unwrap().is_none());
    assert_eq!(
        serde_json::to_value(store::stats(&f.pool, policy.config.as_ref()).await.unwrap()).unwrap()
            ["total"],
        0
    );
}

#[sqlx::test(migrations = false)]
async fn rule_affinity_pg_unavailable_and_changed_channel_bindings(pool: PgPool) {
    let f = fixture(pool, 10_000_000).await;
    let first = CompatibilityProvider::start().await;
    let last = CompatibilityProvider::start().await;
    let rebuilt = CompatibilityProvider::start().await;
    let g = failover_gateway(&f, &first.endpoint, &last.endpoint, false, false, false);
    let router = &g.bridge.routing;
    let enabled = |channel, enabled| {
        router
            .set_operational_enabled(router.version().unwrap(), channel, enabled)
            .unwrap()
    };
    policies(&g, true, true, 10);
    enabled(1, false);
    assert_eq!(model_call(&g, &f, "retry-context", "seed").await.0, 200);
    enabled(1, true);
    enabled(2, false);
    assert_eq!(
        model_call(&g, &f, "retry-context", "skip-unavailable")
            .await
            .0,
        200
    );
    assert_eq!(first.bodies.lock().unwrap().len(), 1);
    enabled(2, true);
    assert_eq!(
        model_call(&g, &f, "retry-context", "reuse-recovered")
            .await
            .0,
        200
    );
    assert_eq!(last.bodies.lock().unwrap().len(), 2);
    policies(&g, false, true, 10);
    let version = router.version().unwrap();
    let mut doc = router.catalog_document().unwrap();
    doc.routing.version += 1;
    doc.routing
        .upstreams
        .iter_mut()
        .find(|u| u.id == 2)
        .unwrap()
        .endpoint = Endpoint::local_loopback(rebuilt.endpoint.clone()).unwrap();
    router
        .publish(
            version,
            doc.routing,
            Arc::new(PriceSnapshot::compile(doc.pricing).unwrap()),
        )
        .unwrap();
    assert_eq!(
        model_call(&g, &f, "retry-context", "new-endpoint").await.0,
        200
    );
    assert!(rebuilt.bodies.lock().unwrap().is_empty());
    assert_eq!(first.bodies.lock().unwrap().len(), 2);
    let row: i64 = sqlx::query_scalar("SELECT channel_id FROM core_channels.affinity_cache")
        .fetch_one(&f.pool)
        .await
        .unwrap();
    assert_eq!(row, 1);
}

#[sqlx::test(migrations = false)]
async fn prompt_cache_affinity_pg_reuses_the_supplier_without_creating_affinity_rows(pool: PgPool) {
    let f = fixture(pool, 10_000_000).await;
    let a = CompatibilityProvider::start().await;
    let b = CompatibilityProvider::start().await;
    let g = failover_gateway(&f, &a.endpoint, &b.endpoint, false, false, true);
    let key = "same-pi-conversation";
    for turn in 0..4 {
        let (status, id) = model_call(&g, &f, key, &format!("pi-cache-{turn}")).await;
        assert_eq!(status, 200);
        assert_eq!(
            g.bridge.charging.resolve(&id).await.unwrap().unwrap().state,
            "settled"
        );
        assert!(
            !serde_json::to_string(&evidence(&f, &id).await)
                .unwrap()
                .contains(key)
        );
    }
    {
        let left = a.bodies.lock().unwrap();
        let right = b.bodies.lock().unwrap();
        assert_eq!(left.len() + right.len(), 4);
        assert!(left.is_empty() || right.is_empty());
        for body in left.iter().chain(right.iter()) {
            assert_eq!(body["prompt_cache_key"], key);
        }
    }
    let rows: i64 = sqlx::query_scalar("SELECT count(*) FROM core_channels.affinity_cache")
        .fetch_one(&f.pool)
        .await
        .unwrap();
    assert_eq!(rows, 0);
}

#[test]
fn prompt_cache_affinity_separates_users_grants_hint_sources_and_explicit_sessions() {
    let principal = |user_id, grant: &str| {
        ModelIdentity::OAuth(crate::identity::grants::OAuthSource {
            grant_id: grant.to_owned(),
            access_id: "synthetic".into(),
            client_id: "pi".into(),
            issuer: "https://synthetic.invalid".into(),
            resource: "synthetic".into(),
            user_id,
            personal_account_id: user_id,
            user_version: 1,
            consent_version: 1,
            scopes: vec![],
            expires_at: 1,
            budget_credits: 1,
        })
    };
    let headers = HeaderMap::new();
    let seed = |headers: &HeaderMap, user, grant: &str, protocol, body: Value| {
        let request = relay::Request::parse(
            protocol,
            &serde_json::to_vec(&body).unwrap(),
            &Limits::default(),
        )
        .unwrap();
        session_affinity(
            headers,
            &principal(user, grant),
            request.prompt_cache_hint(),
        )
        .unwrap()
    };
    let model = "m";
    let responses = json!({"model":model,"input":"hi","prompt_cache_key":"private"});
    let chat = json!({"model":model,"messages":[{"role":"user","content":"hi"}],"prompt_cache_key":"private"});
    let claude = json!({"model":model, "max_tokens":32, "system":"private","messages":[{"role":"user","content":"first"}],
        "cache_control":{"type":"ephemeral"}});
    let base = seed(
        &headers,
        1,
        "a",
        ClientProtocol::Responses,
        responses.clone(),
    );
    assert!(base.is_some());
    assert_eq!(base, seed(&headers, 1, "a", ClientProtocol::Chat, chat));
    assert_ne!(
        base,
        seed(
            &headers,
            2,
            "a",
            ClientProtocol::Responses,
            responses.clone()
        )
    );
    assert_ne!(
        base,
        seed(
            &headers,
            1,
            "b",
            ClientProtocol::Responses,
            responses.clone()
        )
    );
    let cached = seed(&headers, 1, "a", ClientProtocol::Anthropic, claude.clone());
    assert!(cached.is_some());
    assert_ne!(base, cached);
    let mut next = claude.clone();
    next["messages"].as_array_mut().unwrap().extend([
        json!({"role":"assistant","content":"ok"}),
        json!({"role":"user","content":"next"}),
    ]);
    assert_eq!(
        cached,
        seed(&headers, 1, "a", ClientProtocol::Anthropic, next.clone())
    );
    next["metadata"] = json!({"user_id":"private"});
    assert_ne!(
        cached,
        seed(&headers, 1, "a", ClientProtocol::Anthropic, next.clone())
    );
    next.as_object_mut().unwrap().remove("cache_control");
    assert_eq!(
        seed(&headers, 1, "a", ClientProtocol::Anthropic, next),
        None
    );
    let mut explicit = HeaderMap::new();
    explicit.insert("x-lmm-session-id", HeaderValue::from_static("manual"));
    assert_eq!(
        seed(&explicit, 1, "a", ClientProtocol::Anthropic, claude),
        seed(&explicit, 1, "a", ClientProtocol::Responses, responses)
    );
    assert_eq!(
        seed(
            &headers,
            1,
            "a",
            ClientProtocol::Responses,
            json!({"model":model,"input":"hi"})
        ),
        None
    );
}

//! A real Anthropic HTTP response settles against the fixed platform wallet.
use super::*;
use axum::{Json, Router, http::HeaderMap, routing::post};
use serde_json::{Value, json};
use std::sync::Mutex;

fn anthropic_gateway(pool: &PgPool, endpoint: &str) -> ModelGateway {
    let mut routing: RoutingConfig = serde_json::from_value(json!({
        "version":1,"groups":[{"name":"allowed","enabled":true}],
        "upstreams":[{"id":1,"endpoint":"https://synthetic.invalid/v1",
            "protocol":"anthropic_messages","credential":{"id":7,"revision":1},"enabled":true}],
        "routes":[{"id":1,"model":"local-model","group":"allowed","enabled":true,"speeds":["standard"],
            "targets":[{"id":1,"upstream_id":1,"upstream_model":"claude-service-selected","enabled":true,
                "priority":0,"weight":1,"speeds":["standard"],"payload":"adapt","timeout_ms":5000}],
            "failover":{"max_attempts":1,"retry_on":[],"allow_replay_after_send":false}}],"aliases":[]
    })).unwrap();
    routing.upstreams[0].endpoint = Endpoint::local_loopback(endpoint.into()).unwrap();
    let prices: PriceConfig = serde_json::from_value(json!({"version":1,"rules":[{
        "id":1,"model":"local-model","group":"allowed","speed":"standard","billing_mode":"tiered_decimal",
        "rates":{"input":0,"output":0,"cache_read":0,"cache_write":0,"request":0},
        "group_multiplier_ppm":1000000,"speed_multiplier_ppm":1000000,
        "tiered":{"revision":2,"source_digest":"ab".repeat(32),"uses_cache_read":true,
            "uses_cache_write":true,"uses_cache_write_1h":true,
            "tiers":[{"up_to_input_tokens":null,"rates":{
                "input":{"numerator":2,"denominator":1},
                "output":{"numerator":4,"denominator":1},
                "cache_read":{"numerator":1,"denominator":2},
                "cache_write":{"numerator":3,"denominator":1},
                "cache_write_1h":{"numerator":6,"denominator":1}}}]}
    }]})).unwrap();
    let router =
        lmm_core::routing::Router::new(routing, Arc::new(PriceSnapshot::compile(prices).unwrap()))
            .unwrap();
    ModelGateway::new(
        pool.clone(),
        Arc::new(router),
        vec![UpstreamCredential {
            reference: CredentialRef { id: 7, revision: 1 },
            secret: "synthetic-service-anthropic-key".into(),
        }],
        BTreeMap::new(),
        Limits {
            total_timeout: Duration::from_secs(2),
            hook_timeout: Duration::from_secs(1),
            ..Limits::default()
        },
        ChargePolicy {
            input_token_ceiling: 2048,
            lease_seconds: 30,
        },
    )
    .unwrap()
}

#[sqlx::test]
async fn service_funding_anthropic_cache_usage_settles_one_platform_journal(pool: PgPool) {
    let f = setup(&pool).await;
    let captured = Arc::new(Mutex::new(Vec::<(HeaderMap, Value)>::new()));
    let requests = captured.clone();
    let app = Router::new().route("/service82/v1/messages", post(move |headers: HeaderMap, Json(input): Json<Value>| {
        requests.lock().unwrap().push((headers,input));
        async {
            Json(json!({"id":"msg_service82","type":"message","role":"assistant","model":"claude-service-selected",
                "content":[{"type":"text","text":"A synthetic platform-funded answer."}],
                "stop_reason":"end_turn","stop_sequence":null,
                "usage":{"input_tokens":11,"output_tokens":5,"cache_read_input_tokens":7,"cache_creation_input_tokens":3,
                    "service_tier":"standard","inference_geo":"global",
                    "server_tool_use":{"web_search_requests":0,"web_fetch_requests":0},
                    "cache_creation":{"ephemeral_5m_input_tokens":2,"ephemeral_1h_input_tokens":1}}}))
        }
    }));
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let endpoint = format!("http://{}/service82/v1", listener.local_addr().unwrap());
    let (stop, stopped) = tokio::sync::oneshot::channel::<()>();
    let server = tokio::spawn(async move {
        axum::serve(listener, app)
            .with_graceful_shutdown(async {
                let _ = stopped.await;
            })
            .await
            .unwrap();
    });
    let runtime = runtime_authority::RuntimeRole::create(&pool).await;
    runtime.optional("core_charging").await;
    runtime.optional("core_assistant").await;
    let s = AssistantModel::new(
        runtime.pool.clone(),
        anthropic_gateway(&runtime.pool, &endpoint),
        policy(f.payer_account),
    )
    .unwrap();
    let b = binding(&s, &f.customer.secret, "service-anthropic-cache", "hello").await;
    let op = s
        .reserve_text(&f.customer.secret, &b, "hello")
        .await
        .unwrap();
    assert_eq!(op.state, State::Settled);
    assert_eq!(
        op.answer_text.as_deref(),
        Some("A synthetic platform-funded answer.")
    );
    // ceil((11*2 + 7*0.5 + 2*3 + 1*6 + 5*4) * 500000 / 1000000).
    assert_eq!(op.settled_credits, Some(29));
    let row: (i64,i64,i64,i64,i64,i64,Value) = sqlx::query_as(
        "SELECT actor_user_id,owner_account_id,payer_account_id,input_tokens,cached_tokens,output_tokens,price FROM core_charging.requests WHERE id=$1")
        .bind(&op.request_id).fetch_one(&pool).await.unwrap();
    assert_eq!(
        (row.0, row.1, row.2, row.3, row.4, row.5),
        (7, f.customer_account, f.payer_account, 21, 7, 5)
    );
    assert_eq!(row.6["tiered"]["input_semantics"], "anthropic_text");
    assert_eq!(row.6["tiered"]["tariff"]["source_digest"], "ab".repeat(32));
    let final_usage: Value = sqlx::query_scalar("SELECT detail->'usage' FROM core_charging.usage_events WHERE request_id=$1 AND event_id='final'")
        .bind(&op.request_id).fetch_one(&pool).await.unwrap();
    assert_eq!(final_usage["input"], 21);
    assert_eq!(final_usage["cached"], 7);
    assert_eq!(final_usage["output"], 5);
    assert_eq!(final_usage["cache_creation"]["total"], 3);
    assert_eq!(final_usage["cache_creation"]["five_minute"], 2);
    assert_eq!(final_usage["cache_creation"]["one_hour"], 1);
    let journal: (i64,i64,i64,String,i64,bool) = sqlx::query_as(
        "SELECT j.actor_user_id,a.owner_account_id,j.amount_units,j.kind,h.captured,h.closed FROM core_charging.ledger_holds h JOIN core_billing.ledger_journals j ON j.id=h.capture_journal_id AND j.parent_journal_id=h.reservation_journal_id JOIN core_billing.ledger_accounts a ON a.id=j.payer_id WHERE h.request_id=$1")
        .bind(&op.request_id).fetch_one(&pool).await.unwrap();
    assert_eq!(
        journal,
        (7, f.payer_account, 29, "capture".into(), 29, true)
    );
    assert_eq!(
        f.ledger
            .balance(f.customer_account)
            .await
            .unwrap()
            .available_units,
        0
    );
    let platform = f.ledger.balance(f.payer_account).await.unwrap();
    assert_eq!(
        (platform.available_units, platform.reserved_units),
        (99971, 0)
    );
    assert_eq!(
        s.reserve_text(&f.customer.secret, &b, "hello")
            .await
            .unwrap()
            .request_id,
        op.request_id
    );
    let captures: i64 = sqlx::query_scalar("SELECT count(*) FROM core_charging.ledger_holds WHERE request_id=$1 AND capture_journal_id IS NOT NULL")
        .bind(&op.request_id).fetch_one(&pool).await.unwrap();
    assert_eq!(captures, 1);
    {
        let captured = captured.lock().unwrap();
        assert_eq!(captured.len(), 1);
        let (headers, request) = &captured[0];
        assert_eq!(headers["x-api-key"], "synthetic-service-anthropic-key");
        assert_eq!(headers["anthropic-version"], "2023-06-01");
        assert!(headers.get("authorization").is_none());
        assert_eq!(request["model"], "claude-service-selected");
        assert_eq!(request["max_tokens"], 32);
        assert_eq!(request["service_tier"], "standard_only");
        assert!(request.get("inference_geo").is_none());
        assert!(request.get("tools").is_none());
    }
    stop.send(()).unwrap();
    tokio::time::timeout(Duration::from_secs(2), server)
        .await
        .unwrap()
        .unwrap();
    drop(s);
    runtime.cleanup().await;
}

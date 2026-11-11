//! Recovery through real gateway persistence, frozen contracts and PostgreSQL ledger.
use super::*;

fn legacy_gateway(f: &Fixture) -> ModelGateway {
    let original = gateway(f, EXTERNAL_BASE, contexts(SMALL_INPUT));
    let mut document = original.bridge.routing.catalog_document().unwrap();
    document.routing.routes.truncate(1);
    document.pricing.rules.truncate(1);
    document.pricing.rules[0].rounding = Some(routing::TokenRounding::LegacyGoRatioV1);
    let source = serde_json::from_value(json!({
        "source_revision": charging::legacy_precharge::SOURCE_REVISION,
        "upstream_id": 1, "upstream_model": "small-provider", "model": "small-model",
        "preconsumed_tokens": 5000, "count_token": true, "get_media_token": true,
        "get_media_token_not_stream": false, "source_free_model": false,
        "operator_max_billable": 100000
    }))
    .unwrap();
    let g = ModelGateway::new_with_provider_policy(
        f.pool.clone(),
        Arc::new(
            routing::Router::new(
                document.routing,
                Arc::new(PriceSnapshot::compile(document.pricing).unwrap()),
            )
            .unwrap(),
        ),
        vec![UpstreamCredential {
            reference: CredentialRef { id: 7, revision: 1 },
            secret: "synthetic-legacy-precharge-key".into(),
        }],
        BTreeMap::from([(f.key_id, "allowed".into())]),
        original.limits.clone(),
        original.bridge.policy,
        ProviderPolicy::new(contexts(SMALL_INPUT), GLOBAL_INPUT).unwrap(),
    )
    .unwrap()
    .with_legacy_precharge(vec![source])
    .unwrap();
    g
}

#[sqlx::test(migrations = false)]
async fn legacy_sse_recovery_prioritizes_terminal_and_promotes_crash_window(pool: PgPool) {
    let f = fixture(pool, 40000).await;
    let g = legacy_gateway(&f);
    let mut ids = Vec::new();
    for mode in [
        "recorded",
        "crash-window",
        "mismatched-checkpoint",
        "wrong-route",
        "audit-only",
    ] {
        let id = format!("legacy-sse-{mode}");
        let (context, mut request) = admit(&g, &f, &id, "small-model", 1).await;
        request.stream = true;
        let route = g.bridge.select(&context, &request).await.unwrap();
        let reservation = g.bridge.reserve(&context, &route, &request).await.unwrap();
        g.bridge
            .dispatching(&context, &route, &request.upstream_body(&route).unwrap())
            .await
            .unwrap();
        let worker = g.bridge.active.lock().unwrap()[&id].worker;
        let observed = usage(7000);
        if mode == "recorded" {
            g.bridge
                .persist_output(
                    &reservation,
                    &OutputCheckpoint {
                        sequence: 1,
                        packet_sha256: [8; 32],
                        packet_bytes: 128,
                        usage: observed.clone(),
                        terminal_observed: true,
                    },
                )
                .await
                .unwrap();
        } else {
            g.bridge
                .persist_provider_observation(
                    &id,
                    &worker,
                    if mode == "wrong-route" {
                        "other-route"
                    } else {
                        &route.id
                    },
                    observed.clone(),
                    SMALL_INPUT,
                    1,
                )
                .await
                .unwrap();
            if mode != "audit-only" {
                let checkpoint = if mode == "mismatched-checkpoint" {
                    usage(7001)
                } else {
                    observed
                };
                let compact = serde_json::to_value(checkpoint.checkpoint_usage()).unwrap();
                let mut tx = f.pool.begin().await.unwrap();
                sqlx::query("SET TRANSACTION ISOLATION LEVEL SERIALIZABLE")
                    .execute(&mut *tx)
                    .await
                    .unwrap();
                sqlx::query("SELECT core_model.checkpoint_output($1,$2,1,$3,128,$4,true)")
                    .bind(&id)
                    .bind(worker.as_slice())
                    .bind([8_u8; 32].as_slice())
                    .bind(compact)
                    .execute(&mut *tx)
                    .await
                    .unwrap();
                tx.commit().await.unwrap();
            }
        }
        assert_eq!(receipt(&g, &id).await.reserved, 5001);
        g.bridge.charging.mark_unknown(&id, &worker).await.unwrap();
        ids.push(id);
    }
    // Restart with no active worker, changed context configuration and no source
    // precharge config. Only the original durable contract can authorize recovery.
    let restarted = gateway(&f, EXTERNAL_BASE, contexts(32768));
    assert_eq!(
        restarted
            .reconcile_completed_streams_page(20, None)
            .await
            .unwrap()
            .0,
        2
    );
    for (index, id) in ids.iter().enumerate() {
        let charge = receipt(&restarted, id).await;
        if index < 2 {
            assert_eq!(charge.state, "settled");
            assert_eq!(charge.settled, 7000);
            assert_eq!(charge.payer_account_id, f.account);
            let terminals:i64=sqlx::query_scalar("SELECT count(*) FROM core_charging.usage_events WHERE request_id=$1 AND event_id='model-provider-terminal:v1'")
                .bind(id).fetch_one(&f.pool).await.unwrap();
            assert_eq!(terminals, 1);
        } else {
            assert_eq!(charge.state, "reconcile");
            assert_eq!(charge.reserved, 5001);
        }
    }
    assert_balance(&f, 10997, 15003).await;
    assert_eq!(
        restarted
            .reconcile_completed_streams_page(20, None)
            .await
            .unwrap()
            .0,
        0
    );
    assert_balance(&f, 10997, 15003).await;
}

fn expression_gateway(f: &Fixture, base: &str) -> ModelGateway {
    let contracts = if base == EXTERNAL_BASE {
        contexts(SMALL_INPUT)
    } else {
        vec![]
    };
    let original = gateway(f, base, contracts.clone());
    let mut document = original.bridge.routing.catalog_document().unwrap();
    document.routing.routes.truncate(1);
    document.pricing.rules.truncate(1);
    let expression = r#"tier("base",p*2+c*10+cr*0.1+cc*2.5)"#;
    let rule = &mut document.pricing.rules[0];
    rule.billing_mode = BillingMode::TieredDecimal;
    rule.rates = TokenRates {
        input: 0,
        output: 0,
        cache_read: 0,
        cache_write: 0,
        request: 0,
    };
    rule.tiered = Some(
        serde_json::from_value(json!({
            "revision":2,"source_digest":charging::contract::digest(expression.as_bytes()),
            "uses_cache_read":true,"uses_cache_write":true,
            "tiers":[{"up_to_input_tokens":null,"rates":{
                "input":{"numerator":2,"denominator":1},
                "output":{"numerator":10,"denominator":1},
                "cache_read":{"numerator":1,"denominator":10},
                "cache_write":{"numerator":5,"denominator":2}
            }}]
        }))
        .unwrap(),
    );
    ModelGateway::new_with_provider_policy(
        f.pool.clone(),
        Arc::new(
            routing::Router::new(
                document.routing,
                Arc::new(PriceSnapshot::compile(document.pricing).unwrap()),
            )
            .unwrap(),
        ),
        vec![UpstreamCredential {
            reference: CredentialRef { id: 7, revision: 1 },
            secret: "synthetic-expression-provider-key".into(),
        }],
        BTreeMap::from([(f.key_id, "allowed".into())]),
        original.limits.clone(),
        original.bridge.policy,
        ProviderPolicy::new(contracts, GLOBAL_INPUT).unwrap(),
    )
    .unwrap()
    .with_legacy_precharge(vec![
        serde_json::from_value(json!({
            "source_revision":charging::legacy_precharge::SOURCE_REVISION,
            "upstream_id":1,"upstream_model":"small-provider","model":"small-model",
            "preconsumed_tokens":5000,"count_token":true,"get_media_token":true,
            "get_media_token_not_stream":false,"source_free_model":false,
            "operator_max_billable":100000,"source_expression":expression
        }))
        .unwrap(),
    ])
    .unwrap()
}

#[sqlx::test(migrations = false)]
async fn legacy_expression_gateway_and_sse_recovery_confirm_source_float_amount(pool: PgPool) {
    let f = fixture(pool, 1000).await;
    let g = expression_gateway(&f, EXTERNAL_BASE);
    let observed = relay::Usage {
        input_tokens: Some(2490),
        cached_input_tokens: Some(2490),
        output_tokens: Some(0),
        cache_creation_input_tokens: Some(0),
        ..Default::default()
    };
    let mut ids = Vec::new();
    for stream in [false, true] {
        let id = format!("legacy-expression-old-float-{stream}");
        let (context, mut request) = admit(&g, &f, &id, "small-model", 1).await;
        request.stream = stream;
        let route = g.bridge.select(&context, &request).await.unwrap();
        let reservation = g.bridge.reserve(&context, &route, &request).await.unwrap();
        g.bridge
            .dispatching(&context, &route, &request.upstream_body(&route).unwrap())
            .await
            .unwrap();
        let (charge, contract) = g.bridge.charging.reservation_contract(&id).await.unwrap();
        let normalized =
            super::super::billing::frozen_priced_usage(&contract, &charge, observed.clone())
                .unwrap();
        assert_eq!(charge.price.cost(normalized).unwrap(), 125);
        if stream {
            g.bridge
                .persist_output(
                    &reservation,
                    &OutputCheckpoint {
                        sequence: 1,
                        packet_sha256: [11; 32],
                        packet_bytes: 128,
                        usage: observed.clone(),
                        terminal_observed: true,
                    },
                )
                .await
                .unwrap();
            let worker = g.bridge.active.lock().unwrap()[&id].worker;
            g.bridge.charging.mark_unknown(&id, &worker).await.unwrap();
        } else {
            let report = terminal(&id, &route, observed.clone());
            g.bridge.finalize(&reservation, &report).await.unwrap();
            // A repeated terminal and settlement keeps the same source charge.
            g.bridge.finalize(&reservation, &report).await.unwrap();
        }
        ids.push(id);
    }
    let restarted = gateway(&f, EXTERNAL_BASE, contexts(32768));
    assert_eq!(
        restarted
            .reconcile_completed_streams_page(20, None)
            .await
            .unwrap()
            .0,
        1
    );
    for id in &ids {
        let charge = receipt(&restarted, id).await;
        assert_eq!(charge.state, "settled");
        assert_eq!(charge.settled, 124);
        assert_eq!(
            restarted
                .bridge
                .charging
                .terminal_amount(id, charge.usage)
                .await
                .unwrap(),
            124
        );
        assert_eq!(charge.price.cost(charge.usage).unwrap(), 125);
    }
    assert_balance(&f, 752, 0).await;
    assert_eq!(
        restarted
            .reconcile_completed_streams_page(20, None)
            .await
            .unwrap()
            .0,
        0
    );
    assert_balance(&f, 752, 0).await;
}

#[sqlx::test(migrations = false)]
async fn legacy_expression_http_charges_source_float_counterexample(pool: PgPool) {
    use axum::{Json, Router, routing::post};
    async fn supplier() -> Json<serde_json::Value> {
        Json(json!({
            "id":"synthetic-source-float","object":"chat.completion",
            "choices":[{"index":0,"message":{"role":"assistant","content":""},"finish_reason":"stop"}],
            "usage":{"prompt_tokens":2490,"completion_tokens":0,
                "prompt_tokens_details":{"cached_tokens":2490,"cache_write_tokens":0}}
        }))
    }
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let base = format!("http://{}/v1", listener.local_addr().unwrap());
    let server = tokio::spawn(async move {
        axum::serve(
            listener,
            Router::new().route("/v1/chat/completions", post(supplier)),
        )
        .await
        .unwrap();
    });
    let f = fixture(pool, 1000).await;
    let g = expression_gateway(&f, &base);
    let body = json!({"model":"small-model","messages":[{"role":"user","content":"hello"}],"max_tokens":1});
    let (status, response, id) =
        compatibility_call(&g, &f, true, "source-float-http", body.to_string()).await;
    assert_eq!(status, 200, "{response}");
    let charge = receipt(&g, &id).await;
    assert_eq!(charge.state, "settled");
    assert_eq!(charge.settled, 124);
    assert_eq!(charge.price.cost(charge.usage).unwrap(), 125);
    assert_balance(&f, 876, 0).await;
    server.abort();
}

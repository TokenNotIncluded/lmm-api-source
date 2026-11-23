//! Real native admission, durable charging and authorized log projections.
//! Usage is synthetic test evidence. No provider, payment or production DB is used.
use super::*;
use axum::{body::Body, http::StatusCode};
use serde_json::Value;
use tower::ServiceExt;

async fn read_log(pool: &PgPool, path: &str, secret: &str) -> (StatusCode, Value) {
    let response = crate::usage::router(crate::usage::UsageStore::new(pool.clone()))
        .oneshot(
            axum::http::Request::get(path)
                .header("authorization", format!("Bearer {secret}"))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    let status = response.status();
    let bytes = axum::body::to_bytes(response.into_body(), 1 << 20)
        .await
        .unwrap();
    (status, serde_json::from_slice(&bytes).unwrap())
}

async fn admin_snapshot(pool: &PgPool, id: &str, secret: &str) -> Value {
    let (status, response) = read_log(
        pool,
        &format!("/core/v1/usage/admin/request?request_id={id}"),
        secret,
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{response}");
    let encoded = response.to_string();
    for forbidden in [
        "credential_ref",
        "credential_selection",
        "generation_url",
        "synthetic-selected-bound-provider-key",
        "synthetic-secondary-provider-key",
    ] {
        assert!(!encoded.contains(forbidden), "{forbidden}");
    }
    let other: Value = serde_json::from_str(response["data"]["other"].as_str().unwrap()).unwrap();
    other["admin_info"].clone()
}

async fn persisted_contract(pool: &PgPool, id: &str) -> Value {
    sqlx::query_scalar(
        "SELECT detail FROM core_charging.usage_events WHERE request_id=$1 AND event_id='reservation-contract:v1'",
    ).bind(id).fetch_one(pool).await.unwrap()
}

fn publish_pool(g: &ModelGateway, references: Vec<CredentialRef>) {
    let mut release = g.bridge.routing.catalog_document().unwrap();
    release.routing.version += 1;
    release.routing.upstreams[0].credential = references[0];
    g.bridge
        .routing
        .publish_with_credential_pools(
            g.bridge.routing.version().unwrap(),
            release.routing,
            Arc::new(PriceSnapshot::compile(release.pricing).unwrap()),
            &BTreeMap::from([(1, references)]),
        )
        .unwrap();
}

async fn accept(g: &ModelGateway, f: &Fixture, id: &str) -> [u8; 32] {
    let (context, request) = admit(g, f, id, "small-model", 16).await;
    let route = g.bridge.select(&context, &request).await.unwrap();
    let body = request.upstream_body(&route).unwrap();
    g.bridge
        .reserve_prepared(&context, &route, &request, &body, None)
        .await
        .unwrap();
    let worker = g.bridge.active.lock().unwrap()[id].worker;
    g.bridge
        .charging
        .record_route_dispatch(id, &worker, 1, &route.id, "allowed", &digest(&body))
        .await
        .unwrap();
    worker
}

#[sqlx::test(migrations = false)]
async fn key_snapshot_survives_settlement_refund_and_is_admin_only(pool: PgPool) {
    let f = fixture(pool, 10_000_000).await;
    let g = gateway(&f, EXTERNAL_BASE, vec![]);
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
            &BTreeMap::from([(1, vec![primary, secondary])]),
        )
        .unwrap();
    // Advance the real pool once so admission selects ordinal 1.
    let _first = g
        .bridge
        .routing
        .route(routing::RouteRequest {
            model: "small-model",
            group: "allowed",
            speed: None,
            protocol: Protocol::OpenAiChat,
            replay: routing::ReplayPermission::Denied,
        })
        .unwrap();
    let id = "key-snapshot-multi";
    let worker = accept(&g, &f, id).await;
    let frozen = persisted_contract(&f.pool, id).await;
    assert_eq!(
        frozen["binding"]["credential_ref"],
        json!({"id":8,"revision":2})
    );
    assert_eq!(
        frozen["binding"]["credential_selection"],
        json!({"is_multi_key":true,"multi_key_index":1})
    );

    publish_pool(&g, vec![secondary, primary]);
    let actual = charging::Usage {
        input: 10,
        cached: 0,
        output: 1,
        ..Default::default()
    };
    let settled = g
        .bridge
        .charging
        .settle(id, &worker, actual, charging::Outcome::Completed)
        .await
        .unwrap();
    assert_eq!((settled.state.as_str(), settled.settled), ("settled", 10));
    assert!(
        settled.reserved > settled.settled,
        "the unused reservation difference was released"
    );
    let admin = IdentityStore::from_pool(f.pool.clone())
        .bootstrap_user(9, 5)
        .await
        .unwrap();
    let terminal = admin_snapshot(&f.pool, id, &admin.secret).await;
    assert_eq!(terminal["key_snapshot_status"], "recorded");
    assert_eq!(terminal["is_multi_key"], true);
    assert_eq!(terminal["multi_key_index"], 1);
    assert_eq!(terminal["route_attempts"][0]["multi_key_index"], 1);

    publish_pool(&g, vec![primary]);
    let refunded = g
        .bridge
        .charging
        .refund(
            id,
            "synthetic-refund",
            4,
            "isolated key snapshot regression",
        )
        .await
        .unwrap();
    assert_eq!(refunded.refunded, 4);
    assert_eq!(admin_snapshot(&f.pool, id, &admin.secret).await, terminal);
    assert_eq!(persisted_contract(&f.pool, id).await, frozen);
    assert_balance(&f, 10_000_000 - 6, 0).await;

    for path in [
        format!("/core/v1/usage/self/request?request_id={id}"),
        format!("/api/log/self?request_id={id}"),
    ] {
        let (status, response) = read_log(&f.pool, &path, &f.session).await;
        assert_eq!(status, StatusCode::OK, "{response}");
        let encoded = response.to_string();
        for forbidden in [
            "admin_info",
            "route_attempts",
            "credential_ref",
            "credential_selection",
            "is_multi_key",
            "multi_key_index",
            "key_snapshot_status",
        ] {
            assert!(!encoded.contains(forbidden), "{forbidden}");
        }
    }
    assert_eq!(
        read_log(
            &f.pool,
            &format!("/core/v1/usage/admin/request?request_id={id}"),
            &f.session
        )
        .await
        .0,
        StatusCode::FORBIDDEN,
    );

    let single_id = "key-snapshot-single";
    let single_worker = accept(&g, &f, single_id).await;
    g.bridge
        .charging
        .settle(
            single_id,
            &single_worker,
            actual,
            charging::Outcome::Completed,
        )
        .await
        .unwrap();
    let single = admin_snapshot(&f.pool, single_id, &admin.secret).await;
    assert_eq!(single["key_snapshot_status"], "recorded");
    assert_eq!(single["is_multi_key"], false);
    assert_eq!(single["multi_key_index"], 0);
}

#[sqlx::test(migrations = false)]
async fn key_snapshot_historical_contract_remains_unknown_after_refund(pool: PgPool) {
    let f = fixture(pool, 1000).await;
    let charging = charging(&f);
    let id = "key-snapshot-historical";
    let wire = b"synthetic-historical-wire";
    let (spec, basis, proof) = preparation(id, 24, 8, price(), wire);
    reserve(&charging, &f.key, &spec, basis, &proof)
        .await
        .unwrap();
    charging.start(id, &WORKER).await.unwrap();
    charging
        .record_route_dispatch(
            id,
            &WORKER,
            1,
            "synthetic-prepared-route",
            "allowed",
            &digest(wire),
        )
        .await
        .unwrap();
    charging
        .settle(
            id,
            &WORKER,
            charging::Usage {
                input: 10,
                cached: 0,
                output: 2,
                ..Default::default()
            },
            charging::Outcome::Completed,
        )
        .await
        .unwrap();
    let frozen = persisted_contract(&f.pool, id).await;
    assert!(frozen["binding"].get("credential_selection").is_none());
    let admin = IdentityStore::from_pool(f.pool.clone())
        .bootstrap_user(9, 5)
        .await
        .unwrap();
    let terminal = admin_snapshot(&f.pool, id, &admin.secret).await;
    assert_eq!(terminal["key_snapshot_status"], "unknown");
    assert!(terminal["is_multi_key"].is_null());
    assert!(terminal["multi_key_index"].is_null());
    assert_eq!(
        terminal["route_attempts"][0]["key_snapshot_status"],
        "unknown"
    );
    charging
        .refund(
            id,
            "synthetic-old-refund",
            4,
            "isolated historical regression",
        )
        .await
        .unwrap();
    assert_eq!(admin_snapshot(&f.pool, id, &admin.secret).await, terminal);
    assert_eq!(persisted_contract(&f.pool, id).await, frozen);
    assert_balance(&f, 992, 0).await;
}

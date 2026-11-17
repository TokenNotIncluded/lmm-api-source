//! Channel card and batch controls publish one durable routing revision.
use axum::http::StatusCode;
use lmm_core::channel_http::{ManagedChannels, managed_router};
use serde_json::{Value, json};
use sqlx::PgPool;
#[path = "common/channel_control.rs"]
mod support;
use support::*;

#[sqlx::test(migrations = false)]
async fn channel_batch_configuration_is_atomic_authorized_and_recoverable(pool: PgPool) {
    let (app, gateway, admin, ordinary) = vault_app(&pool).await;
    let routing = gateway.dashboard_routing();
    let initial = Version::from(routing.version().unwrap());
    let created = success(
        &app,
        "POST",
        "/core/v1/admin/routing/channels",
        &admin,
        channel_input(initial, 2, "synthetic-second-channel-only"),
    )
    .await;
    let initial: Version = serde_json::from_value(created["version"].clone()).unwrap();
    let before = active(&pool).await;
    let path = "/core/v1/admin/routing/channels/batch/configuration";
    let body = json!({"expected_version":initial,"ids":[1,2],
        "patch":{"enabled":false,"tag":"Batch team","priority":600,"weight":12,"timeout_ms":45000}});
    for (token, expected) in [
        (&ordinary, StatusCode::FORBIDDEN),
        (&String::new(), StatusCode::UNAUTHORIZED),
    ] {
        assert_eq!(
            request(app.clone(), "PUT", path, token, body.clone())
                .await
                .0,
            expected
        );
    }
    for (ids, patch, expected) in [
        (
            json!([1, 99]),
            json!({"enabled":false,"priority":600}),
            StatusCode::NOT_FOUND,
        ),
        (
            json!([1, 1]),
            json!({"enabled":false}),
            StatusCode::BAD_REQUEST,
        ),
        (json!([]), json!({"enabled":false}), StatusCode::BAD_REQUEST),
        (
            json!([0]),
            json!({"enabled":false}),
            StatusCode::BAD_REQUEST,
        ),
        (json!([1, 2]), json!({}), StatusCode::BAD_REQUEST),
        (
            json!([1, 2]),
            json!({"timeout_ms":0}),
            StatusCode::BAD_REQUEST,
        ),
        (
            json!([1, 2]),
            json!({"timeout_ms":3_600_001}),
            StatusCode::BAD_REQUEST,
        ),
        (
            json!([1, 2]),
            json!({"tag":"bad\nlabel"}),
            StatusCode::BAD_REQUEST,
        ),
    ] {
        let (status, result) = request(
            app.clone(),
            "PUT",
            path,
            &admin,
            json!({"expected_version":initial,"ids":ids,"patch":patch}),
        )
        .await;
        assert_eq!(status, expected, "{result}");
        assert_eq!(Version::from(routing.version().unwrap()), initial);
        assert_eq!(
            serde_json::to_value(active(&pool).await).unwrap(),
            serde_json::to_value(&before).unwrap()
        );
    }
    let (first, second) = tokio::join!(
        request(app.clone(), "PUT", path, &admin, body.clone()),
        request(app.clone(), "PUT", path, &admin, body),
    );
    let saved = match (first.0, second.0) {
        (StatusCode::OK, StatusCode::CONFLICT) => first.1,
        (StatusCode::CONFLICT, StatusCode::OK) => second.1,
        other => panic!("expected one publication and one conflict: {other:?}"),
    };
    assert_eq!(saved["data"]["updated"], 2);
    assert_eq!(saved["data"]["target_count"], 2);
    let changed: Version = serde_json::from_value(saved["data"]["version"].clone()).unwrap();
    assert_eq!(changed.routing, initial.routing + 1);
    assert_eq!(changed.pricing, initial.pricing);
    let durable = active(&pool).await;
    assert_eq!(Version::of(&durable), changed);
    assert!(
        durable
            .routing
            .upstreams
            .iter()
            .all(|upstream| !upstream.enabled)
    );
    for (before, after) in before
        .routing
        .routes
        .iter()
        .flat_map(|r| &r.targets)
        .zip(durable.routing.routes.iter().flat_map(|r| &r.targets))
    {
        assert_eq!(after.id, before.id);
        assert_eq!(after.client_protocols, before.client_protocols);
        assert_eq!(after.enabled, before.enabled);
        assert_eq!(after.upstream_model, before.upstream_model);
        assert_eq!(
            (after.priority, after.weight, after.timeout_ms),
            (600, 12, 45000)
        );
    }
    let restored_gateway = support::gateway(&pool)
        .with_channel_secret_key(&[83; 32])
        .unwrap();
    let restored = managed_router(
        pool.clone(),
        ManagedChannels::recover(pool.clone(), restored_gateway)
            .await
            .unwrap(),
    );
    for id in [1, 2] {
        let detail = success(
            &restored,
            "GET",
            &format!("/api/channel/{id}"),
            &admin,
            Value::Null,
        )
        .await;
        assert_eq!(detail["status"], 2);
        assert_eq!(detail["priority"], 600);
        assert_eq!(detail["tag"], "Batch team");
    }
    let audits: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM core_identity.audit WHERE action='channel.configuration_batch_set'",
    )
    .fetch_one(&pool)
    .await
    .unwrap();
    assert_eq!(audits, 1);
    let reset = success(&restored, "PUT", path, &admin,
        json!({"expected_version":changed,"ids":[1],"patch":{"enabled":true,"tag":"","priority":0,"weight":0}})).await;
    assert_eq!(reset["updated"], 1);
    let final_document = active(&pool).await;
    assert!(final_document.routing.upstreams[0].enabled);
    assert!(!final_document.routing.upstreams[1].enabled);
    assert_eq!(final_document.routing.routes[0].targets[0].priority, 0);
    assert_eq!(final_document.routing.routes[0].targets[0].weight, 0);
    assert_eq!(final_document.routing.routes[0].targets[1].priority, 600);
    assert_eq!(final_document.routing.routes[0].targets[1].weight, 12);
    let detail = success(&restored, "GET", "/api/channel/1", &admin, Value::Null).await;
    assert!(detail["tag"].is_null());
}

#[sqlx::test(migrations = false)]
async fn channel_batch_target_settings_require_configured_models(pool: PgPool) {
    let (app, gateway, admin, _) = vault_app(&pool).await;
    let initial = Version::from(gateway.dashboard_routing().version().unwrap());
    let mut input = channel_input(initial, 2, "synthetic-unconfigured-channel-only");
    input["enabled"] = json!(false);
    input["contexts"] = json!([]);
    input["attachments"] = json!([]);
    let created = success(
        &app,
        "POST",
        "/core/v1/admin/routing/channels",
        &admin,
        input,
    )
    .await;
    let version: Version = serde_json::from_value(created["version"].clone()).unwrap();
    let path = "/core/v1/admin/routing/channels/batch/configuration";
    let (status, _) = request(
        app.clone(),
        "PUT",
        path,
        &admin,
        json!({"expected_version":version,"ids":[1,2],"patch":{"priority":9}}),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    assert_eq!(
        Version::from(gateway.dashboard_routing().version().unwrap()),
        version
    );
    let saved = success(
        &app,
        "PUT",
        path,
        &admin,
        json!({"expected_version":version,"ids":[2],"patch":{"enabled":false,"tag":"Draft"}}),
    )
    .await;
    assert_eq!(saved["updated"], 1);
    assert_eq!(saved["target_count"], 0);
}

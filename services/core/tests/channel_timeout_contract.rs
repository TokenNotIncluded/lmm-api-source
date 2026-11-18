//! Real HTTP/PG contract for the existing global model timeout admission floor.
use axum::{Router, http::StatusCode};
use lmm_core::{
    channel_http::{ManagedChannels, managed_router},
    model_http::{ChargePolicy, ModelGateway, ProviderPolicy, UpstreamCredential},
    relay::Limits,
};
use serde_json::{Value, json};
use sqlx::PgPool;
use std::{collections::BTreeMap, time::Duration};

#[path = "common/channel_control.rs"]
#[allow(dead_code)]
mod support;
use support::*;

const CONFIGURATION: &str = "/core/v1/admin/routing/configuration";
const PREVIEW: &str = "/core/v1/admin/routing/configuration/preview";
const BATCH: &str = "/core/v1/admin/routing/channels/batch/configuration";

async fn app_with_timeout(pool: &PgPool, total_timeout_ms: u64, contexts: Value) -> Router {
    let gateway = ModelGateway::new_with_provider_policy(
        pool.clone(),
        std::sync::Arc::new(fixture().into_router().unwrap()),
        vec![UpstreamCredential {
            reference: fixture().routing.upstreams[0].credential,
            secret: "synthetic-channel-only".into(),
        }],
        BTreeMap::new(),
        Limits {
            total_timeout: Duration::from_millis(total_timeout_ms),
            max_output_tokens: 100,
            ..Limits::default()
        },
        ChargePolicy {
            input_token_ceiling: 1000,
            lease_seconds: 120,
        },
        ProviderPolicy::new(serde_json::from_value(contexts).unwrap(), 1000).unwrap(),
    )
    .unwrap();
    let management = ManagedChannels::recover(pool.clone(), gateway)
        .await
        .unwrap();
    managed_router(pool.clone(), management)
}

fn proposed(configuration: &Value, timeout_ms: u32) -> Value {
    let mut input = json!({
        "expected_version": configuration["version"],
        "routing": configuration["routing"],
        "prices": configuration["prices"],
        "bindings": configuration["bindings"],
    });
    input["routing"]["version"] = json!(configuration["version"]["routing"].as_u64().unwrap() + 1);
    input["routing"]["routes"][0]["targets"][0]["timeout_ms"] = json!(timeout_ms);
    input
}

fn assert_timeout_error(status: StatusCode, body: &Value, minimum: u64) {
    assert_eq!(status, StatusCode::CONFLICT, "{body}");
    assert_eq!(body["success"], false);
    assert_eq!(body["code"], "CHANNEL_TARGET_TIMEOUT_TOO_SHORT");
    assert_eq!(body["data"], json!({ "required_min_timeout_ms": minimum }));
    assert!(!body.to_string().contains("synthetic-channel-only"));
}

#[sqlx::test(migrations = false)]
async fn channel_timeout_contract_exposes_actual_limit_without_rewriting_existing_targets(
    pool: PgPool,
) {
    let (initial, _, admin, ordinary, _) = setup(&pool).await;
    let configuration = success(&initial, "GET", CONFIGURATION, &admin, Value::Null).await;
    let app = app_with_timeout(&pool, 45_000, configuration["bindings"]["contexts"].clone()).await;
    let configuration = success(&app, "GET", CONFIGURATION, &admin, Value::Null).await;
    assert_eq!(
        configuration["model_limits"],
        json!({ "total_timeout_ms": 45_000 })
    );
    assert_eq!(
        configuration["routing"]["routes"][0]["targets"][0]["timeout_ms"],
        30_000
    );
    assert!(!configuration.to_string().contains("synthetic-channel-only"));
    let (status, body) = request(
        app.clone(),
        "POST",
        PREVIEW,
        &admin,
        proposed(&configuration, 30_000),
    )
    .await;
    assert_timeout_error(status, &body, 45_000);
    let unchanged = success(&app, "GET", CONFIGURATION, &admin, Value::Null).await;
    assert_eq!(unchanged["version"], configuration["version"]);
    assert_eq!(unchanged["routing"], configuration["routing"]);
    let (status, body) = request(app, "GET", CONFIGURATION, &ordinary, Value::Null).await;
    assert_eq!(status, StatusCode::FORBIDDEN, "{body}");
    assert!(body.get("data").is_none_or(Value::is_null));
}

#[sqlx::test(migrations = false)]
async fn channel_timeout_contract_preview_publish_and_batch_share_the_required_minimum(
    pool: PgPool,
) {
    let (initial, _, admin, _, _) = setup(&pool).await;
    let configuration = success(&initial, "GET", CONFIGURATION, &admin, Value::Null).await;
    assert_eq!(
        configuration["model_limits"],
        json!({ "total_timeout_ms": 500 })
    );
    let mut input = proposed(&configuration, 500);
    let preview = success(&initial, "POST", PREVIEW, &admin, input.clone()).await;
    input["confirmation_digest"] = preview["confirmation_digest"].clone();

    // A real preview digest remains bound to the submitted document while the
    // serving configuration may acquire a stricter global limit before publish.
    let app = app_with_timeout(&pool, 750, configuration["bindings"]["contexts"].clone()).await;
    for (method, path) in [("POST", PREVIEW), ("PUT", CONFIGURATION)] {
        let (status, body) = request(app.clone(), method, path, &admin, input.clone()).await;
        assert_timeout_error(status, &body, 750);
    }
    let (status, body) = request(
        app.clone(), "PUT", BATCH, &admin,
        json!({ "expected_version": configuration["version"], "ids": [1], "patch": { "timeout_ms": 500 } }),
    ).await;
    assert_timeout_error(status, &body, 750);
    let unchanged = success(&app, "GET", CONFIGURATION, &admin, Value::Null).await;
    assert_eq!(unchanged["routing"], configuration["routing"]);
    assert_eq!(unchanged["version"], configuration["version"]);

    let mut input = proposed(&unchanged, 750);
    let preview = success(&app, "POST", PREVIEW, &admin, input.clone()).await;
    input["confirmation_digest"] = preview["confirmation_digest"].clone();
    success(&app, "PUT", CONFIGURATION, &admin, input).await;
    let saved = success(&app, "GET", CONFIGURATION, &admin, Value::Null).await;
    assert_eq!(
        saved["routing"]["routes"][0]["targets"][0]["timeout_ms"],
        750
    );
    assert_eq!(saved["version"]["routing"], 2);

    let mut missing_context = proposed(&saved, 750);
    missing_context["bindings"]["contexts"] = json!([]);
    let (status, body) = request(app, "POST", PREVIEW, &admin, missing_context).await;
    assert_eq!(status, StatusCode::CONFLICT, "{body}");
    assert_eq!(body["code"], "CHANNEL_BINDING_NOT_CONFIGURED");
}

#[sqlx::test(migrations = false)]
async fn channel_timeout_contract_preserves_disabled_upstream_drafts_and_checks_enable(
    pool: PgPool,
) {
    let (app, _, admin, _, _) = setup(&pool).await;
    let configuration = success(&app, "GET", CONFIGURATION, &admin, Value::Null).await;
    let mut input = proposed(&configuration, 1);
    input["routing"]["upstreams"][0]["enabled"] = json!(false);
    let preview = success(&app, "POST", PREVIEW, &admin, input.clone()).await;
    input["confirmation_digest"] = preview["confirmation_digest"].clone();
    success(&app, "PUT", CONFIGURATION, &admin, input).await;
    let saved = success(&app, "GET", CONFIGURATION, &admin, Value::Null).await;
    assert_eq!(saved["routing"]["routes"][0]["targets"][0]["timeout_ms"], 1);
    let (status, body) = request(
        app.clone(),
        "PUT",
        BATCH,
        &admin,
        json!({ "expected_version": saved["version"], "ids": [1], "patch": { "enabled": true } }),
    )
    .await;
    assert_timeout_error(status, &body, 500);
    let unchanged = success(&app, "GET", CONFIGURATION, &admin, Value::Null).await;
    assert_eq!(unchanged["routing"], saved["routing"]);
    assert_eq!(unchanged["version"], saved["version"]);
}

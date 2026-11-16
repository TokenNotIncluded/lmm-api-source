//! Shared fixtures for channel integration and crate-local transport tests.
use axum::{
    Router,
    body::{Body, to_bytes},
    http::{Request, StatusCode, header},
};
use lmm_core::{
    channel_http::{ManagedChannels, managed_router},
    identity::IdentityStore,
    model_http::{ChargePolicy, ModelGateway, ProviderPolicy, UpstreamCredential},
    relay::Limits,
    routing::{
        CredentialRef, PriceConfig, PriceSnapshot, ReleaseVersion, Router as ServingRouter,
        RoutingConfig, store::ReleaseDocument,
    },
};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use sqlx::PgPool;
use std::{collections::BTreeMap, sync::Arc, time::Duration};
use tower::ServiceExt;

#[derive(Clone, Copy, Debug, Deserialize, Serialize, PartialEq, Eq)]
pub(super) struct Version {
    pub(super) routing: u64,
    pub(super) pricing: u64,
}
impl From<ReleaseVersion> for Version {
    fn from(value: ReleaseVersion) -> Self {
        Self {
            routing: value.routing,
            pricing: value.pricing,
        }
    }
}
impl Version {
    pub(super) fn of(document: &ReleaseDocument) -> Self {
        Self {
            routing: document.routing.version,
            pricing: document.pricing.version,
        }
    }
}
pub(super) fn fixture() -> ReleaseDocument {
    let config:RoutingConfig=serde_json::from_value(json!({"version":1,"groups":[{"name":"default","enabled":true}],
      "upstreams":[{"id":1,"endpoint":"https://provider.example/v1/","protocol":"openai_chat","credential":{"id":771,"revision":1},"enabled":true}],
      "routes":[{"id":1,"model":"model-a","group":"default","enabled":true,"speeds":["standard"],
       "targets":[{"id":1,"upstream_id":1,"upstream_model":"model-a","enabled":true,"priority":0,"weight":1,"speeds":["standard"],"payload":"adapt","timeout_ms":30000}],
       "failover":{"max_attempts":1,"retry_on":[],"allow_replay_after_send":false}}],"aliases":[]})).unwrap();
    let prices:PriceConfig=serde_json::from_value(json!({"version":1,"rules":[{"id":1,"model":"model-a","group":"default","speed":"standard",
      "rates":{"input":2,"output":4,"cache_read":0,"cache_write":0,"request":0},"group_multiplier_ppm":1000000,"speed_multiplier_ppm":1000000}]})).unwrap();
    ReleaseDocument {
        contract_version: 1,
        routing: config,
        pricing: prices,
    }
}
pub(super) fn route_request() -> lmm_core::routing::RouteRequest<'static> {
    lmm_core::routing::RouteRequest {
        model: "model-a",
        group: "default",
        protocol: lmm_core::routing::Protocol::OpenAiChat,
        speed: None,
        replay: lmm_core::routing::ReplayPermission::Denied,
    }
}
pub(super) fn gateway(pool: &PgPool) -> ModelGateway {
    gateway_with_loopback(pool, None)
}
pub(super) fn gateway_with_loopback(pool: &PgPool, loopback: Option<&str>) -> ModelGateway {
    gateway_fixture(pool, loopback, None)
}
pub(super) fn gateway_fixture(
    pool: &PgPool,
    loopback: Option<&str>,
    missing_binding: Option<bool>,
) -> ModelGateway {
    let document = fixture();
    let mut config = document.routing;
    let prices = document.pricing;
    let mut contexts:Vec<lmm_core::model_http::ExternalModelContext>=serde_json::from_value(json!([{"upstream_id":1,"upstream_model":"model-a","protocol":"openai_chat","endpoint":"https://provider.example/v1/","max_input_tokens":1000,"max_output_tokens":100,"contract_revision":"synthetic-v1","missing_cached_tokens_is_zero":true,"missing_cache_creation_tokens_is_zero":true}])).unwrap();
    if let Some(endpoint) = loopback {
        config.upstreams[0].endpoint =
            lmm_core::routing::Endpoint::local_loopback(endpoint.into()).unwrap();
        contexts.clear();
    }
    if let Some(missing_secret) = missing_binding {
        config.upstreams[0].enabled = false;
        config.routes[0].enabled = false;
        if !missing_secret {
            contexts.clear();
        }
    }
    let credentials = if missing_binding == Some(true) {
        Vec::new()
    } else {
        vec![UpstreamCredential {
            reference: CredentialRef {
                id: 771,
                revision: 1,
            },
            secret: "synthetic-channel-only".into(),
        }]
    };
    let routing = Arc::new(
        ServingRouter::new(config, Arc::new(PriceSnapshot::compile(prices).unwrap())).unwrap(),
    );
    ModelGateway::new_with_provider_policy(
        pool.clone(),
        routing,
        credentials,
        BTreeMap::new(),
        Limits {
            total_timeout: Duration::from_millis(500),
            max_output_tokens: 100,
            ..Limits::default()
        },
        ChargePolicy {
            input_token_ceiling: 1000,
            lease_seconds: 60,
        },
        ProviderPolicy::new(contexts, 1000).unwrap(),
    )
    .unwrap()
}
pub(super) async fn request(
    app: Router,
    method: &str,
    path: &str,
    secret: &str,
    body: Value,
) -> (StatusCode, Value) {
    let response = app
        .oneshot(
            Request::builder()
                .method(method)
                .uri(path)
                .header(header::AUTHORIZATION, format!("Bearer {secret}"))
                .header(header::CONTENT_TYPE, "application/json")
                .body(Body::from(serde_json::to_vec(&body).unwrap()))
                .unwrap(),
        )
        .await
        .unwrap();
    let status = response.status();
    let bytes = to_bytes(response.into_body(), 1024 * 1024).await.unwrap();
    (
        status,
        serde_json::from_slice(&bytes)
            .unwrap_or_else(|_| json!({"raw":String::from_utf8_lossy(&bytes)})),
    )
}
pub(super) async fn setup(
    pool: &PgPool,
) -> (Router, Arc<ServingRouter>, String, String, ModelGateway) {
    lmm_core::core_schema::init(pool).await.unwrap();
    let identities = IdentityStore::from_pool(pool.clone());
    let admin = identities.bootstrap_user(8, 6).await.unwrap();
    let ordinary = identities.bootstrap_user(7, 1).await.unwrap();
    let gateway = gateway(pool);
    let routing = gateway.dashboard_routing();
    let management = ManagedChannels::recover(pool.clone(), gateway.clone())
        .await
        .unwrap();
    (
        managed_router(pool.clone(), management),
        routing,
        admin.secret,
        ordinary.secret,
        gateway,
    )
}

pub(super) fn channel_input(expected: Version, id: u64, secret: &str) -> Value {
    json!({"expected_version":expected,"id":id,"name":"Published provider","endpoint":"https://second.example/v1/",
      "protocol":"openai_chat","enabled":true,"secret":secret,
      "contexts":[{"upstream_id":id,"upstream_model":"model-a","protocol":"openai_chat","endpoint":"https://second.example/v1/",
        "max_input_tokens":1000,"max_output_tokens":100,"contract_revision":"synthetic-provider-v2",
        "missing_cached_tokens_is_zero":true,"missing_cache_creation_tokens_is_zero":true}],
      "attachments":[{"route_id":1,"upstream_model":"model-a","priority":0,"weight":1,"speeds":["standard"],"timeout_ms":30000,"payload":"adapt"}]})
}
pub(super) async fn active(pool: &PgPool) -> ReleaseDocument {
    let value: Value = sqlx::query_scalar("SELECT public.routing_control_read()")
        .fetch_one(pool)
        .await
        .unwrap();
    serde_json::from_value(value).unwrap()
}
pub(super) async fn success(
    app: &Router,
    method: &str,
    path: &str,
    admin: &str,
    body: Value,
) -> Value {
    let (status, value) = request(app.clone(), method, path, admin, body).await;
    assert_eq!(status, StatusCode::OK, "{path}: {value}");
    value["data"].clone()
}
pub(super) async fn vault_app(pool: &PgPool) -> (Router, ModelGateway, String, String) {
    let (_, _, admin, ordinary, _) = setup(pool).await;
    let gateway = gateway(pool).with_channel_secret_key(&[83; 32]).unwrap();
    let management = ManagedChannels::recover(pool.clone(), gateway.clone())
        .await
        .unwrap();
    (
        managed_router(pool.clone(), management),
        gateway,
        admin,
        ordinary,
    )
}

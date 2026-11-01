//! Durable native channel control through the public host and HTTP boundaries.
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
use sha2::{Digest, Sha256};
use sqlx::PgPool;
use std::{collections::BTreeMap, sync::Arc, time::Duration};
use tower::ServiceExt;

#[derive(Clone, Copy, Debug, Deserialize, Serialize, PartialEq, Eq)]
struct Version {
    routing: u64,
    pricing: u64,
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
    fn of(document: &ReleaseDocument) -> Self {
        Self {
            routing: document.routing.version,
            pricing: document.pricing.version,
        }
    }
}
fn fixture() -> ReleaseDocument {
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
fn route_request() -> lmm_core::routing::RouteRequest<'static> {
    lmm_core::routing::RouteRequest {
        model: "model-a",
        group: "default",
        protocol: lmm_core::routing::Protocol::OpenAiChat,
        speed: None,
        replay: lmm_core::routing::ReplayPermission::Denied,
    }
}
fn gateway(pool: &PgPool) -> ModelGateway {
    gateway_with_loopback(pool, None)
}
fn gateway_with_loopback(pool: &PgPool, loopback: Option<&str>) -> ModelGateway {
    gateway_fixture(pool, loopback, None)
}
fn gateway_fixture(
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
fn gateway_for_restore(pool: &PgPool) -> ModelGateway {
    gateway(pool)
}
async fn request(
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
async fn setup(pool: &PgPool) -> (Router, Arc<ServingRouter>, String, String, ModelGateway) {
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
#[sqlx::test(migrations = false)]
async fn channel_control_persists_status_and_delete_with_cas_and_audit(pool: PgPool) {
    let (app, routing, admin, ordinary, gateway) = setup(&pool).await;
    let initial = Version::from(routing.version().unwrap());
    let body = json!({"expected_version":initial,"enabled":false});
    assert_eq!(
        request(
            app.clone(),
            "PUT",
            "/api/channel/1/status",
            &ordinary,
            body.clone()
        )
        .await
        .0,
        StatusCode::FORBIDDEN
    );
    let (status, result) = request(
        app.clone(),
        "PUT",
        "/api/channel/1/status",
        &admin,
        body.clone(),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{result}");
    let changed: Version = serde_json::from_value(result["data"]["version"].clone()).unwrap();
    assert_eq!(changed.routing, initial.routing + 1);
    let (status, detail) = request(app.clone(), "GET", "/api/channel/1", &admin, Value::Null).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(detail["data"]["status"], 2);
    assert!(routing.route(route_request()).is_err());
    assert!(
        gateway
            .oauth_catalog_surface()
            .project(&["default".into()], "https://resource.example", 1)
            .unwrap()
            .models
            .is_empty()
    );
    assert_eq!(
        request(app.clone(), "PUT", "/api/channel/1/status", &admin, body)
            .await
            .0,
        StatusCode::CONFLICT
    );
    let durable = lmm_core::routing::store::PostgresRoutingStore::new(pool.clone())
        .load_active()
        .await
        .unwrap();
    assert_eq!(Version::of(&durable), changed);
    let restored_gateway = gateway_for_restore(&pool);
    let restored = restored_gateway.dashboard_routing();
    // Reproduce a process whose durable snapshot applied but metadata refresh
    // did not finish. Recovery must repair metadata even for an equal document.
    durable.clone().apply(&restored).unwrap();
    assert!(
        restored_gateway
            .oauth_catalog_surface()
            .project(&["default".into()], "https://resource.example", 1)
            .is_err()
    );
    let _recovered = ManagedChannels::recover(pool.clone(), restored_gateway.clone())
        .await
        .unwrap();
    assert_eq!(
        Version::from(restored.version().unwrap()),
        Version::of(&durable)
    );
    assert!(restored.route(route_request()).is_err());
    assert!(
        restored_gateway
            .oauth_catalog_surface()
            .project(&["default".into()], "https://resource.example", 1)
            .unwrap()
            .models
            .is_empty()
    );
    let (status, result) = request(
        app,
        "DELETE",
        "/api/channel/1",
        &admin,
        json!({"expected_version":changed}),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{result}");
    let after = lmm_core::routing::store::PostgresRoutingStore::new(pool.clone())
        .load_active()
        .await
        .unwrap();
    assert_eq!(
        Version::from(routing.version().unwrap()),
        Version::of(&after)
    );
    assert!(after.routing.upstreams.iter().all(|u| u.id != 1));
    assert!(
        after
            .routing
            .routes
            .iter()
            .flat_map(|r| &r.targets)
            .all(|t| t.upstream_id != 1)
    );
    let audits:i64=sqlx::query_scalar("SELECT count(*) FROM core_identity.audit WHERE action IN ('channel.status','channel.delete')").fetch_one(&pool).await.unwrap();
    assert_eq!(audits, 2);
}
#[sqlx::test(migrations = false)]
async fn price_preview_requires_exact_confirmation_and_preserves_frozen_requests(pool: PgPool) {
    let (app, routing, admin, _, gateway) = setup(&pool).await;
    let old = fixture();
    let route_request = lmm_core::routing::RouteRequest {
        model: "model-a",
        group: "default",
        protocol: lmm_core::routing::Protocol::OpenAiChat,
        speed: None,
        replay: lmm_core::routing::ReplayPermission::Denied,
    };
    let frozen = routing.route(route_request).unwrap().quote();
    let version = Version::of(&old);
    let mut prices = old.pricing.clone();
    prices.version += 1;
    prices.rules[0].group_multiplier_ppm *= 2;
    let body = json!({"expected_version":version,"prices":prices});
    let (status, preview) = request(
        app.clone(),
        "POST",
        "/core/v1/admin/routing/prices/preview",
        &admin,
        body,
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{preview}");
    assert_eq!(Version::from(routing.version().unwrap()), Version::of(&old));
    assert_eq!(
        preview["data"]["before"],
        serde_json::to_value(&old.pricing).unwrap()
    );
    let mut publish =
        json!({"expected_version":version,"prices":prices,"confirmation_digest":"wrong"});
    assert_eq!(
        request(
            app.clone(),
            "PUT",
            "/core/v1/admin/routing/prices",
            &admin,
            publish.clone()
        )
        .await
        .0,
        StatusCode::BAD_REQUEST
    );
    publish["confirmation_digest"] = preview["data"]["confirmation_digest"].clone();
    let (status, result) =
        request(app, "PUT", "/core/v1/admin/routing/prices", &admin, publish).await;
    assert_eq!(status, StatusCode::OK, "{result}");
    let durable = lmm_core::routing::store::PostgresRoutingStore::new(pool.clone())
        .load_active()
        .await
        .unwrap();
    assert_eq!(durable.pricing, prices);
    assert_eq!(
        Version::from(routing.version().unwrap()),
        Version::of(&durable)
    );
    let catalog = gateway
        .oauth_catalog_surface()
        .project(&["default".into()], "https://resource.example", 1)
        .unwrap();
    assert_eq!(catalog.pricing_version, prices.version);
    assert_eq!(catalog.models.len(), 1);
    assert_eq!(
        frozen.multipliers_ppm().0,
        old.pricing.rules[0].group_multiplier_ppm
    );
    let now = routing
        .route(lmm_core::routing::RouteRequest {
            model: "model-a",
            group: "default",
            protocol: lmm_core::routing::Protocol::OpenAiChat,
            speed: None,
            replay: lmm_core::routing::ReplayPermission::Denied,
        })
        .unwrap()
        .quote();
    assert_eq!(
        now.multipliers_ppm().0,
        prices.rules[0].group_multiplier_ppm
    );
    assert_eq!(
        old.pricing.rules[0].group_multiplier_ppm * 2,
        prices.rules[0].group_multiplier_ppm
    );
}
#[sqlx::test(migrations = false)]
async fn audit_failure_preserves_publication(pool: PgPool) {
    let (app, routing, admin, _, _gateway) = setup(&pool).await;
    let before = fixture();
    sqlx::raw_sql("CREATE FUNCTION public.reject_channel_audit() RETURNS trigger LANGUAGE plpgsql AS $$ BEGIN IF NEW.action='channel.status' THEN RAISE EXCEPTION 'synthetic audit failure'; END IF; RETURN NEW; END $$; CREATE TRIGGER reject_channel_audit BEFORE INSERT ON core_identity.audit FOR EACH ROW EXECUTE FUNCTION public.reject_channel_audit();").execute(&pool).await.unwrap();
    let (status, result) = request(
        app,
        "PUT",
        "/api/channel/1/status",
        &admin,
        json!({"expected_version":Version::of(&before),"enabled":false}),
    )
    .await;
    assert_eq!(status, StatusCode::SERVICE_UNAVAILABLE, "{result}");
    assert_eq!(
        Version::from(routing.version().unwrap()),
        Version::of(&before)
    );
    assert!(routing.route(route_request()).is_ok());
    let durable: Option<Value> = sqlx::query_scalar("SELECT public.routing_control_read()")
        .fetch_one(&pool)
        .await
        .unwrap();
    assert!(durable.is_none());
    let versions: i64 = sqlx::query_scalar("SELECT count(*) FROM routing_config_versions")
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(versions, 0);
}

#[sqlx::test(migrations = false)]
async fn unrepresentable_ledger_prices_are_rejected_before_preview_or_publication(pool: PgPool) {
    let (app, routing, admin, _, _) = setup(&pool).await;
    let original = fixture();
    let expected = Version::of(&original);
    {
        let mut prices = original.pricing.clone();
        prices.version += 1;
        prices.rules[0].rates.input = 1;
        // This is a valid routing document but converts to half a ledger unit per
        // million tokens. The control plane must apply the gateway's exact check.
        assert!(PriceSnapshot::compile(prices.clone()).is_ok());
        let (status, body) = request(
            app.clone(),
            "POST",
            "/core/v1/admin/routing/prices/preview",
            &admin,
            json!({"expected_version":expected,"prices":prices}),
        )
        .await;
        assert_eq!(status, StatusCode::BAD_REQUEST, "{body}");
        let digest = format!(
            "{:x}",
            Sha256::digest(
                serde_json::to_vec(&("lmm-price-publication-v1", expected, &prices)).unwrap()
            )
        );
        let (status, body) = request(
            app.clone(),
            "PUT",
            "/core/v1/admin/routing/prices",
            &admin,
            json!({"expected_version":expected,"prices":prices,"confirmation_digest":digest}),
        )
        .await;
        assert_eq!(status, StatusCode::BAD_REQUEST, "{body}");
        assert_eq!(
            Version::from(routing.version().unwrap()),
            Version::of(&original)
        );
        assert_eq!(
            routing
                .route(route_request())
                .unwrap()
                .quote()
                .multipliers_ppm()
                .0,
            original.pricing.rules[0].group_multiplier_ppm
        );
    }
    let persisted: Option<Value> = sqlx::query_scalar("SELECT public.routing_control_read()")
        .fetch_one(&pool)
        .await
        .unwrap();
    assert!(persisted.is_none());
}

#[sqlx::test(migrations = false)]
async fn channel_catalogue_probe_authenticates_and_exposes_only_model_identifiers(pool: PgPool) {
    use std::sync::atomic::{AtomicUsize, Ordering};
    let (_app, _, admin, ordinary, _) = setup(&pool).await;
    let calls = Arc::new(AtomicUsize::new(0));
    let observed = calls.clone();
    let upstream=Router::new().route("/v1/models",axum::routing::get(move |headers:axum::http::HeaderMap|{
        let observed=observed.clone();
        async move {
            assert_eq!(headers[header::AUTHORIZATION],"Bearer synthetic-channel-only");
            observed.fetch_add(1,Ordering::SeqCst);
            axum::Json(json!({"data":[{"id":"model-b","owner":"synthetic-channel-only"},{"id":"model-a"},{"id":"model-b"}]}))
        }
    }));
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let origin = format!("http://{}/v1", listener.local_addr().unwrap());
    let server = tokio::spawn(async move { axum::serve(listener, upstream).await.unwrap() });
    let gateway = gateway_with_loopback(&pool, Some(&origin));
    let management = ManagedChannels::recover(pool.clone(), gateway)
        .await
        .unwrap();
    let app = managed_router(pool, management);
    let (status, _) = request(
        app.clone(),
        "GET",
        "/api/channel/test/1",
        &ordinary,
        Value::Null,
    )
    .await;
    assert_eq!(status, StatusCode::FORBIDDEN);
    assert_eq!(calls.load(Ordering::SeqCst), 0);
    let (status, body) = request(
        app.clone(),
        "GET",
        "/api/channel/test/1",
        &admin,
        Value::Null,
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(body["data"]["models"], json!(["model-a", "model-b"]));
    assert_eq!(body["data"]["validation_scope"], "model_catalog");
    assert_eq!(body["data"]["inference_tested"], false);
    assert!(!body.to_string().contains("synthetic-channel-only"));
    let (status, body) = request(
        app,
        "GET",
        "/api/channel/fetch_models/1",
        &admin,
        Value::Null,
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(body["data"], json!(["model-a", "model-b"]));
    assert_eq!(calls.load(Ordering::SeqCst), 2);
    server.abort();
    let _ = server.await;
}

#[sqlx::test(migrations = false)]
async fn enabling_a_channel_requires_its_real_loaded_key_and_provider_contract(pool: PgPool) {
    let (_, _, admin, _, _) = setup(&pool).await;
    for missing_secret in [true, false] {
        let gateway = gateway_fixture(&pool, None, Some(missing_secret));
        let routing = gateway.dashboard_routing();
        let management = ManagedChannels::recover(pool.clone(), gateway)
            .await
            .unwrap();
        let app = managed_router(pool.clone(), management);
        let version = Version::from(routing.version().unwrap());
        let (status, body) = request(
            app.clone(),
            "PUT",
            "/api/channel/1/status",
            &admin,
            json!({"expected_version":version,"enabled":true}),
        )
        .await;
        assert_eq!(status, StatusCode::CONFLICT, "{body}");
        assert_eq!(body["code"], "CHANNEL_BINDING_NOT_CONFIGURED");
        let (status, detail) = request(app, "GET", "/api/channel/1", &admin, Value::Null).await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(detail["data"]["status"], 2);
        assert_eq!(Version::from(routing.version().unwrap()), version);
        let durable: Option<Value> = sqlx::query_scalar("SELECT public.routing_control_read()")
            .fetch_one(&pool)
            .await
            .unwrap();
        assert!(durable.is_none());
    }
}

#[path = "common/runtime.rs"]
mod runtime;

#[sqlx::test(migrations = false)]
async fn restricted_runtime_publishes_without_routing_dml_and_rolls_back_with_audit(pool: PgPool) {
    let (_, _, admin, _, _) = setup(&pool).await;
    let role = runtime::RuntimeRole::create(&pool).await;
    role.optional("core_charging").await;
    role.optional("core_model").await;
    // These are the production model controls and Web routing read grants.
    // Runtime receives no DML on any routing artifact or credential reference.
    sqlx::raw_sql(&format!(
        "GRANT USAGE ON SCHEMA public TO {0}; GRANT SELECT ON public.routing_active_release,public.routing_groups,public.routing_routes TO {0}; GRANT EXECUTE ON FUNCTION public.routing_control_read(),public.routing_control_publish(jsonb,jsonb,bigint,bigint),public.routing_control_publish(jsonb,jsonb,jsonb,bigint,bigint),public.routing_control_bindings(bigint),public.routing_control_secret_read(bigint,bigint),public.routing_control_secret_create(bigint,bigint,bigint,text,bytea),public.routing_control_secret_list(bigint),public.routing_control_secret_remove(bigint,bigint,bigint) TO {0}",
        role.name
    )).execute(&pool).await.unwrap();
    lmm_core::runtime_database::check(
        &role.pool,
        lmm_core::runtime_database::RequiredCapabilities {
            models: true,
            ..Default::default()
        },
    )
    .await
    .unwrap();
    let writes:bool=sqlx::query_scalar("SELECT coalesce(bool_or(has_table_privilege(c.oid,'INSERT,UPDATE,DELETE,TRUNCATE') OR has_any_column_privilege(c.oid,'UPDATE')),false) FROM pg_class c JOIN pg_namespace n ON n.oid=c.relnamespace WHERE n.nspname='public' AND c.relname LIKE 'routing_%' AND c.relkind='r'").fetch_one(&role.pool).await.unwrap();
    assert!(!writes);
    let direct =
        sqlx::query("INSERT INTO public.routing_credential_refs(id,revision) VALUES(999999,1)")
            .execute(&role.pool)
            .await
            .unwrap_err();
    assert_eq!(
        direct.as_database_error().and_then(|e| e.code()).as_deref(),
        Some("42501")
    );
    let material = sqlx::query("SELECT document FROM public.routing_config_versions")
        .fetch_all(&role.pool)
        .await
        .unwrap_err();
    assert_eq!(
        material
            .as_database_error()
            .and_then(|e| e.code())
            .as_deref(),
        Some("42501")
    );
    let gateway = gateway(&role.pool);
    let routing = gateway.dashboard_routing();
    let management = ManagedChannels::recover(role.pool.clone(), gateway)
        .await
        .unwrap();
    let app = managed_router(role.pool.clone(), management);
    let (status, list) = request(app.clone(), "GET", "/api/channel", &admin, Value::Null).await;
    assert_eq!(status, StatusCode::OK, "{list}");
    assert_eq!(list["data"]["items"][0]["id"], 1);
    let expected = Version::from(routing.version().unwrap());
    let (status, body) = request(
        app.clone(),
        "PUT",
        "/api/channel/1/status",
        &admin,
        json!({"expected_version":expected,"enabled":false}),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    let version: Version = serde_json::from_value(body["data"]["version"].clone()).unwrap();
    let durable: Value = sqlx::query_scalar("SELECT public.routing_control_read()")
        .fetch_one(&role.pool)
        .await
        .unwrap();
    assert_eq!(durable["routing"]["version"], version.routing);
    assert_eq!(durable["routing"]["upstreams"][0]["enabled"], false);
    sqlx::raw_sql("CREATE FUNCTION public.reject_runtime_channel_audit() RETURNS trigger LANGUAGE plpgsql AS $$ BEGIN IF NEW.action='channel.prices.publish' THEN RAISE EXCEPTION 'synthetic runtime audit failure'; END IF; RETURN NEW; END $$; CREATE TRIGGER reject_runtime_channel_audit BEFORE INSERT ON core_identity.audit FOR EACH ROW EXECUTE FUNCTION public.reject_runtime_channel_audit();").execute(&pool).await.unwrap();
    let mut prices = fixture().pricing;
    prices.version += 1;
    prices.rules[0].group_multiplier_ppm *= 2;
    let digest = format!(
        "{:x}",
        Sha256::digest(
            serde_json::to_vec(&("lmm-price-publication-v1", version, &prices)).unwrap()
        )
    );
    let (status, body) = request(
        app,
        "PUT",
        "/core/v1/admin/routing/prices",
        &admin,
        json!({"expected_version":version,"prices":prices,"confirmation_digest":digest}),
    )
    .await;
    assert_eq!(status, StatusCode::SERVICE_UNAVAILABLE, "{body}");
    let after: Value = sqlx::query_scalar("SELECT public.routing_control_read()")
        .fetch_one(&role.pool)
        .await
        .unwrap();
    assert_eq!(after, durable);
    assert_eq!(Version::from(routing.version().unwrap()), version);
    let counts:(i64,i64,i64)=sqlx::query_as("SELECT (SELECT count(*) FROM public.routing_config_versions),(SELECT count(*) FROM public.routing_price_versions),(SELECT count(*) FROM core_identity.audit WHERE action IN ('channel.status','channel.prices.publish'))").fetch_one(&pool).await.unwrap();
    assert_eq!(counts, (1, 1, 1));
    role.cleanup().await;
}

fn channel_input(expected: Version, id: u64, secret: &str) -> Value {
    json!({"expected_version":expected,"id":id,"name":"Published provider","endpoint":"https://second.example/v1/",
      "protocol":"openai_chat","enabled":true,"secret":secret,
      "contexts":[{"upstream_id":id,"upstream_model":"model-a","protocol":"openai_chat","endpoint":"https://second.example/v1/",
        "max_input_tokens":1000,"max_output_tokens":100,"contract_revision":"synthetic-provider-v2",
        "missing_cached_tokens_is_zero":true,"missing_cache_creation_tokens_is_zero":true}],
      "attachments":[{"route_id":1,"upstream_model":"model-a","priority":0,"weight":1,"speeds":["standard"],"timeout_ms":30000,"payload":"adapt"}]})
}
async fn active(pool: &PgPool) -> ReleaseDocument {
    let value: Value = sqlx::query_scalar("SELECT public.routing_control_read()")
        .fetch_one(pool)
        .await
        .unwrap();
    serde_json::from_value(value).unwrap()
}
async fn success(app: &Router, method: &str, path: &str, admin: &str, body: Value) -> Value {
    let (status, value) = request(app.clone(), method, path, admin, body).await;
    assert_eq!(status, StatusCode::OK, "{path}: {value}");
    value["data"].clone()
}
async fn vault_app(pool: &PgPool) -> (Router, ModelGateway, String, String) {
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

#[sqlx::test(migrations = false)]
async fn vault_keys_persist_encrypt_rotate_retire_and_recover_into_real_transport(pool: PgPool) {
    use std::sync::atomic::{AtomicUsize, Ordering};
    let (_, _, admin, ordinary) = vault_app(&pool).await;
    let role = runtime::RuntimeRole::create(&pool).await;
    role.optional("core_charging").await;
    role.optional("core_model").await;
    sqlx::raw_sql(&format!("GRANT USAGE ON SCHEMA public TO {0}; GRANT SELECT ON public.routing_active_release,public.routing_groups,public.routing_routes TO {0}", role.name)).execute(&pool).await.unwrap();
    lmm_core::runtime_database::check(
        &role.pool,
        lmm_core::runtime_database::RequiredCapabilities {
            models: true,
            ..Default::default()
        },
    )
    .await
    .unwrap();
    for query in [
        "SELECT ciphertext FROM public.channel_secret_revisions",
        "SELECT document FROM public.channel_release_bindings",
    ] {
        let denied = sqlx::query(query).fetch_all(&role.pool).await.unwrap_err();
        assert_eq!(
            denied
                .as_database_error()
                .and_then(|error| error.code())
                .as_deref(),
            Some("42501")
        );
    }
    let live_gateway = gateway(&role.pool)
        .with_channel_secret_key(&[83; 32])
        .unwrap();
    let management = ManagedChannels::recover(role.pool.clone(), live_gateway.clone())
        .await
        .unwrap();
    let app = managed_router(role.pool.clone(), management);
    let expected = Version::of(&fixture());
    let input = channel_input(expected, 2, "synthetic-vault-primary-key");
    assert_eq!(
        request(
            app.clone(),
            "POST",
            "/core/v1/admin/routing/channels",
            &ordinary,
            input.clone()
        )
        .await
        .0,
        StatusCode::FORBIDDEN
    );
    let mut missing = input.clone();
    missing["contexts"] = json!([]);
    assert_eq!(
        request(
            app.clone(),
            "POST",
            "/core/v1/admin/routing/channels",
            &admin,
            missing
        )
        .await
        .0,
        StatusCode::CONFLICT
    );
    let empty: i64 = sqlx::query_scalar("SELECT count(*) FROM public.channel_secret_revisions")
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(
        empty, 0,
        "a failed binding must roll back its private revision"
    );
    success(
        &app,
        "POST",
        "/core/v1/admin/routing/channels",
        &admin,
        input,
    )
    .await;
    let mut document = active(&pool).await;
    let primary = document
        .routing
        .upstreams
        .iter()
        .find(|u| u.id == 2)
        .unwrap()
        .credential;
    let ciphertext: Vec<u8> = sqlx::query_scalar(
        "SELECT ciphertext FROM public.channel_secret_revisions WHERE id=$1 AND revision=$2",
    )
    .bind(primary.id as i64)
    .bind(primary.revision as i64)
    .fetch_one(&pool)
    .await
    .unwrap();
    assert!(
        !ciphertext
            .windows(27)
            .any(|window| window == b"synthetic-vault-primary-key")
    );
    let (_, list) = request(
        app.clone(),
        "GET",
        "/api/channel/search?keyword=Published",
        &admin,
        Value::Null,
    )
    .await;
    assert_eq!(list["data"]["total"], 1);
    assert_eq!(list["data"]["items"][0]["name"], "Published provider");
    assert!(!list.to_string().contains("second.example"));
    success(
        &app,
        "PUT",
        "/api/channel/1/status",
        &admin,
        json!({"expected_version":Version::of(&document),"enabled":false}),
    )
    .await;
    document = active(&pool).await;
    let added=success(&app,"POST","/core/v1/admin/routing/channels/2/keys",&admin,
        json!({"operation":"add","expected_version":Version::of(&document),"label":"Rotation","secret":"synthetic-vault-secondary-key","activate":false})).await;
    let secondary: CredentialRef = serde_json::from_value(added["reference"].clone()).unwrap();
    document = active(&pool).await;
    assert_eq!(
        document
            .routing
            .upstreams
            .iter()
            .find(|u| u.id == 2)
            .unwrap()
            .credential,
        primary
    );
    let listed = success(
        &app,
        "GET",
        "/core/v1/admin/routing/channels/2/keys",
        &admin,
        Value::Null,
    )
    .await;
    assert_eq!(listed["keys"].as_array().unwrap().len(), 2);
    assert!(!listed.to_string().contains("synthetic-vault"));
    success(&app,"POST","/core/v1/admin/routing/channels/2/keys",&admin,
        json!({"operation":"activate","expected_version":Version::of(&document),"reference":secondary})).await;
    document = active(&pool).await;
    assert_eq!(
        document
            .routing
            .upstreams
            .iter()
            .find(|u| u.id == 2)
            .unwrap()
            .credential,
        secondary
    );
    assert_eq!(request(app.clone(),"POST","/core/v1/admin/routing/channels/2/keys",&admin,
        json!({"operation":"remove","expected_version":Version::of(&document),"reference":secondary})).await.0,StatusCode::CONFLICT);
    success(
        &app,
        "POST",
        "/core/v1/admin/routing/channels/2/keys",
        &admin,
        json!({"operation":"remove","expected_version":Version::of(&document),"reference":primary}),
    )
    .await;
    document = active(&pool).await;
    assert_eq!(request(app.clone(),"POST","/core/v1/admin/routing/channels/2/keys",&admin,
        json!({"operation":"activate","expected_version":Version::of(&document),"reference":primary})).await.0,StatusCode::NOT_FOUND);
    let raw_audit: String = sqlx::query_scalar(
        "SELECT coalesce(string_agg(target::text,''),'') FROM core_identity.audit",
    )
    .fetch_one(&pool)
    .await
    .unwrap();
    assert!(!raw_audit.contains("synthetic-vault"));
    assert!(
        ManagedChannels::recover(
            role.pool.clone(),
            gateway(&role.pool)
                .with_channel_secret_key(&[84; 32])
                .unwrap()
        )
        .await
        .is_err()
    );
    drop(app);
    let restored_gateway = gateway(&role.pool)
        .with_channel_secret_key(&[83; 32])
        .unwrap();
    let restored = ManagedChannels::recover(role.pool.clone(), restored_gateway.clone())
        .await
        .unwrap();
    assert_eq!(
        restored_gateway
            .oauth_catalog_surface()
            .project(&["default".into()], "https://resource.example", 1)
            .unwrap()
            .models
            .len(),
        1
    );
    // Only this test uses the existing host-owned loopback constructor. The
    // persisted administrative endpoint remains public HTTPS; production DNS,
    // TLS and private-address policy receive no bypass or test option.
    let hits = Arc::new(AtomicUsize::new(0));
    let seen = hits.clone();
    let provider = axum::Router::new().route(
        "/v1/models",
        axum::routing::get(move |headers: axum::http::HeaderMap| {
            let seen = seen.clone();
            async move {
                assert_eq!(
                    headers.get(header::AUTHORIZATION).unwrap(),
                    "Bearer synthetic-vault-secondary-key"
                );
                seen.fetch_add(1, Ordering::SeqCst);
                axum::Json(json!({"data":[{"id":"model-a"}]}))
            }
        }),
    );
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let server = tokio::spawn(async move {
        axum::serve(listener, provider).await.unwrap();
    });
    document.routing.version += 1;
    document
        .routing
        .upstreams
        .iter_mut()
        .find(|u| u.id == 2)
        .unwrap()
        .endpoint =
        lmm_core::routing::Endpoint::local_loopback(format!("http://{address}/v1/")).unwrap();
    document
        .apply(&restored_gateway.dashboard_routing())
        .unwrap();
    let restored_app = managed_router(role.pool.clone(), restored);
    let probe = success(
        &restored_app,
        "GET",
        "/api/channel/test/2",
        &admin,
        Value::Null,
    )
    .await;
    assert_eq!(probe["models"], json!(["model-a"]));
    assert_eq!(probe["inference_tested"], false);
    assert_eq!(hits.load(Ordering::SeqCst), 1);
    server.abort();
    drop(restored_app);
    drop(restored_gateway);
    drop(live_gateway);
    role.cleanup().await;
}

#[sqlx::test(migrations = false)]
async fn vault_rejects_private_destinations_and_rolls_back_key_with_audit_failure(pool: PgPool) {
    let (app, gateway, admin, _) = vault_app(&pool).await;
    let expected = Version::of(&fixture());
    for endpoint in [
        "https://127.0.0.1/v1/",
        "https://10.0.0.1/v1/",
        "https://user:password@provider.example/v1/",
        "https://provider.example/v1/?key=secret",
    ] {
        let mut input = channel_input(expected, 2, "synthetic-vault-never-saved");
        input["endpoint"] = json!(endpoint);
        input["contexts"][0]["endpoint"] = json!(endpoint);
        let (status, body) = request(
            app.clone(),
            "POST",
            "/core/v1/admin/routing/channels",
            &admin,
            input,
        )
        .await;
        assert!(
            status == StatusCode::BAD_REQUEST || status == StatusCode::UNPROCESSABLE_ENTITY,
            "{body}"
        );
    }
    let configuration = success(
        &app,
        "GET",
        "/core/v1/admin/routing/configuration",
        &admin,
        Value::Null,
    )
    .await;
    for discovery in [
        json!({"models":["model-a"],"observed_at":0}),
        json!({"models":["model-a"],"observed_at":253402300800_i64}),
        json!({"models":["model\ninvalid"],"observed_at":1}),
    ] {
        let mut proposed = json!({"expected_version":configuration["version"],"routing":configuration["routing"],"prices":configuration["prices"],"bindings":configuration["bindings"]});
        proposed["routing"]["version"] = json!(2);
        proposed["bindings"]["discoveries"] = json!({"1":discovery});
        assert_eq!(
            request(
                app.clone(),
                "POST",
                "/core/v1/admin/routing/configuration/preview",
                &admin,
                proposed
            )
            .await
            .0,
            StatusCode::BAD_REQUEST
        );
    }
    sqlx::raw_sql("CREATE FUNCTION public.reject_vault_audit() RETURNS trigger LANGUAGE plpgsql AS $$ BEGIN IF NEW.action='channel.create' THEN RAISE EXCEPTION 'synthetic audit failure'; END IF; RETURN NEW; END $$; CREATE TRIGGER reject_vault_audit BEFORE INSERT ON core_identity.audit FOR EACH ROW EXECUTE FUNCTION public.reject_vault_audit();").execute(&pool).await.unwrap();
    let (status, body) = request(
        app,
        "POST",
        "/core/v1/admin/routing/channels",
        &admin,
        channel_input(expected, 2, "synthetic-vault-never-saved"),
    )
    .await;
    assert_eq!(status, StatusCode::SERVICE_UNAVAILABLE, "{body}");
    for table in [
        "channel_secret_revisions",
        "channel_release_bindings",
        "routing_config_versions",
        "routing_credential_refs",
    ] {
        let count: i64 = sqlx::query_scalar(&format!("SELECT count(*) FROM public.{table}"))
            .fetch_one(&pool)
            .await
            .unwrap();
        assert_eq!(count, 0, "{table}");
    }
    assert_eq!(
        Version::from(gateway.dashboard_routing().version().unwrap()),
        expected
    );
}

#[sqlx::test(migrations = false)]
async fn executable_models_and_vendors_require_confirmed_prices_and_preserve_dependencies(
    pool: PgPool,
) {
    let (app, gateway, admin, ordinary) = vault_app(&pool).await;
    let initial = Version::of(&fixture());
    let vendor = json!({"expected_version":initial,"vendor":{"id":9,"name":"Reviewed provider","description":"Synthetic contract"}});
    assert_eq!(
        request(
            app.clone(),
            "POST",
            "/core/v1/admin/routing/vendors",
            &ordinary,
            vendor.clone()
        )
        .await
        .0,
        StatusCode::FORBIDDEN
    );
    success(
        &app,
        "POST",
        "/core/v1/admin/routing/vendors",
        &admin,
        vendor,
    )
    .await;
    let expected = Version::of(&active(&pool).await);
    let mut route = serde_json::to_value(&fixture().routing.routes[0]).unwrap();
    route["id"] = json!(2);
    route["model"] = json!("model-b");
    route["targets"][0]["id"] = json!(2);
    route["targets"][0]["upstream_model"] = json!("model-b");
    let mut price = serde_json::to_value(&fixture().pricing.rules[0]).unwrap();
    price["id"] = json!(2);
    price["model"] = json!("model-b");
    let mut model = json!({"expected_version":expected,"route":route,
        "metadata":{"display_name":"Model B","description":"Reviewed synthetic model","vendor_id":9},"prices":[price],
        "contexts":[{"upstream_id":1,"upstream_model":"model-b","protocol":"openai_chat","endpoint":"https://provider.example/v1/",
            "max_input_tokens":1000,"max_output_tokens":100,"contract_revision":"synthetic-model-b-v1","missing_cached_tokens_is_zero":true,"missing_cache_creation_tokens_is_zero":true}]});
    assert_eq!(
        request(
            app.clone(),
            "POST",
            "/core/v1/admin/routing/models",
            &admin,
            model.clone()
        )
        .await
        .0,
        StatusCode::BAD_REQUEST
    );
    let preview = success(
        &app,
        "POST",
        "/core/v1/admin/routing/models/preview",
        &admin,
        json!({"editing":false,"model":model}),
    )
    .await;
    assert_eq!(preview["saved"], false);
    assert_eq!(Version::of(&active(&pool).await), expected);
    model["confirmation_digest"] = preview["confirmation_digest"].clone();
    success(
        &app,
        "POST",
        "/core/v1/admin/routing/models",
        &admin,
        model.clone(),
    )
    .await;
    let document = active(&pool).await;
    assert_eq!(document.routing.routes.len(), 2);
    assert_eq!(document.pricing.version, 2);
    let catalogue = gateway
        .oauth_catalog_surface()
        .project(&["default".into()], "https://resource.example", 1)
        .unwrap();
    // OAuth V1 intentionally advertises text models only when the public
    // model equals its provider wire model. Renamed/mapped routes remain
    // executable through the native gateway and its separate inventory.
    assert_eq!(catalogue.models.len(), 2);
    assert!(catalogue.models.iter().any(|model| model.name == "model-b"));
    assert_eq!(
        request(
            app.clone(),
            "DELETE",
            "/core/v1/admin/routing/vendors/9",
            &admin,
            json!({"expected_version":Version::of(&document)})
        )
        .await
        .0,
        StatusCode::CONFLICT
    );
    let expected = Version::of(&document);
    model["expected_version"] = json!(expected);
    model["route"]["model"] = json!("model-b-renamed");
    model["prices"][0]["model"] = json!("model-b-renamed");
    model["metadata"]["display_name"] = json!("Renamed B");
    model["confirmation_digest"] = Value::Null;
    let preview = success(
        &app,
        "POST",
        "/core/v1/admin/routing/models/preview",
        &admin,
        json!({"editing":true,"model":model}),
    )
    .await;
    model["confirmation_digest"] = preview["confirmation_digest"].clone();
    success(
        &app,
        "PUT",
        "/core/v1/admin/routing/models/2",
        &admin,
        model.clone(),
    )
    .await;
    assert_eq!(
        request(
            app.clone(),
            "PUT",
            "/core/v1/admin/routing/models/2",
            &admin,
            model
        )
        .await
        .0,
        StatusCode::CONFLICT
    );
    let document = active(&pool).await;
    assert!(
        document
            .routing
            .routes
            .iter()
            .any(|route| route.model == "model-b-renamed")
    );
    let native_models = success(
        &app,
        "GET",
        "/core/v1/admin/routing/models",
        &admin,
        Value::Null,
    )
    .await;
    assert_eq!(native_models["items"].as_array().unwrap().len(), 2);
    assert!(
        native_models["items"]
            .as_array()
            .unwrap()
            .iter()
            .any(|item| {
                item["route"]["model"] == "model-b-renamed"
                    && item["route"]["targets"][0]["upstream_model"] == "model-b"
            })
    );
    let mut plan = gateway
        .dashboard_routing()
        .route(lmm_core::routing::RouteRequest {
            model: "model-b-renamed",
            group: "default",
            protocol: lmm_core::routing::Protocol::OpenAiChat,
            speed: None,
            replay: lmm_core::routing::ReplayPermission::Denied,
        })
        .unwrap();
    let attempt = plan.start().unwrap();
    assert_eq!(attempt.upstream_model(), "model-b");
    assert_eq!(attempt.version().pricing, document.pricing.version);
    success(
        &app,
        "DELETE",
        "/core/v1/admin/routing/models/2",
        &admin,
        json!({"expected_version":Version::of(&document)}),
    )
    .await;
    let document = active(&pool).await;
    assert_eq!(document.routing.routes.len(), 1);
    assert_eq!(document.pricing.rules.len(), 1);
    success(
        &app,
        "DELETE",
        "/core/v1/admin/routing/vendors/9",
        &admin,
        json!({"expected_version":Version::of(&document)}),
    )
    .await;
    let result = success(
        &app,
        "GET",
        "/core/v1/admin/routing/vendors",
        &admin,
        Value::Null,
    )
    .await;
    assert_eq!(result["items"], json!([]));
}

#[sqlx::test(migrations = false)]
async fn price_drafts_survive_restart_and_publish_zero_multiplier_without_changing_frozen_quotes(
    pool: PgPool,
) {
    let (app, gateway, admin, _) = vault_app(&pool).await;
    let frozen = gateway
        .dashboard_routing()
        .route(route_request())
        .unwrap()
        .quote();
    let expected = Version::of(&fixture());
    let mut proposed = fixture().pricing;
    proposed.version = 2;
    proposed.rules[0].group_multiplier_ppm = 0;
    success(
        &app,
        "PUT",
        "/core/v1/admin/routing/prices/drafts/15",
        &admin,
        json!({"expected_version":expected,"name":"Free promotional group","prices":proposed}),
    )
    .await;
    let saved = active(&pool).await;
    assert_eq!(saved.pricing.version, 1);
    assert_eq!(saved.pricing.rules[0].group_multiplier_ppm, 1_000_000);
    let restored_gateway = gateway_for_restore(&pool)
        .with_channel_secret_key(&[83; 32])
        .unwrap();
    let restored = ManagedChannels::recover(pool.clone(), restored_gateway.clone())
        .await
        .unwrap();
    let restored_app = managed_router(pool.clone(), restored);
    let drafts = success(
        &restored_app,
        "GET",
        "/core/v1/admin/routing/prices/drafts",
        &admin,
        Value::Null,
    )
    .await;
    assert_eq!(drafts["items"][0]["draft"]["id"], 15);
    assert_eq!(drafts["items"][0]["stale"], false);
    let version = Version::of(&saved);
    let preview = success(
        &restored_app,
        "POST",
        "/core/v1/admin/routing/prices/drafts/15/preview",
        &admin,
        json!({"expected_version":version}),
    )
    .await;
    assert_eq!(preview["saved"], false);
    assert_eq!(
        request(
            restored_app.clone(),
            "POST",
            "/core/v1/admin/routing/prices/drafts/15/publish",
            &admin,
            json!({"expected_version":version,"confirmation_digest":"wrong"})
        )
        .await
        .0,
        StatusCode::BAD_REQUEST
    );
    success(
        &restored_app,
        "POST",
        "/core/v1/admin/routing/prices/drafts/15/publish",
        &admin,
        json!({"expected_version":version,"confirmation_digest":preview["confirmation_digest"]}),
    )
    .await;
    let published = active(&pool).await;
    assert_eq!(published.pricing, proposed);
    assert_eq!(frozen.multipliers_ppm().0, 1_000_000);
    assert_eq!(
        restored_gateway
            .dashboard_routing()
            .route(route_request())
            .unwrap()
            .quote()
            .multipliers_ppm()
            .0,
        0
    );
    let drafts = success(
        &restored_app,
        "GET",
        "/core/v1/admin/routing/prices/drafts",
        &admin,
        Value::Null,
    )
    .await;
    assert_eq!(drafts["items"], json!([]));
    let audits:i64=sqlx::query_scalar("SELECT count(*) FROM core_identity.audit WHERE action IN ('channel.prices.draft.save','channel.prices.draft.publish')").fetch_one(&pool).await.unwrap();
    assert_eq!(audits, 2);
}

#[sqlx::test(migrations = false)]
async fn channel_edits_bind_actual_context_and_stale_drafts_never_replace_new_prices(pool: PgPool) {
    let (app, _, admin, _) = vault_app(&pool).await;
    let initial = Version::of(&fixture());
    let mut input = channel_input(initial, 2, "synthetic-edit-key");
    success(
        &app,
        "POST",
        "/core/v1/admin/routing/channels",
        &admin,
        input.clone(),
    )
    .await;
    let current = active(&pool).await;
    input.as_object_mut().unwrap().remove("secret");
    input["expected_version"] = json!(Version::of(&current));
    input["name"] = json!("Edited provider");
    input["endpoint"] = json!("https://third.example/v1/");
    assert_eq!(
        request(
            app.clone(),
            "PUT",
            "/core/v1/admin/routing/channels/2",
            &admin,
            input.clone()
        )
        .await
        .0,
        StatusCode::BAD_REQUEST
    );
    assert_eq!(active(&pool).await, current);
    input["contexts"][0]["endpoint"] = input["endpoint"].clone();
    success(
        &app,
        "PUT",
        "/core/v1/admin/routing/channels/2",
        &admin,
        input,
    )
    .await;
    let current = active(&pool).await;
    let mut proposed = current.pricing.clone();
    proposed.version += 1;
    proposed.rules[0].group_multiplier_ppm = 2_000_000;
    success(
        &app,
        "PUT",
        "/core/v1/admin/routing/prices/drafts/10",
        &admin,
        json!({"expected_version":Version::of(&current),"name":"Stale draft","prices":proposed}),
    )
    .await;
    let version = Version::of(&active(&pool).await);
    proposed.rules[0].group_multiplier_ppm = 3_000_000;
    let preview = success(
        &app,
        "POST",
        "/core/v1/admin/routing/prices/preview",
        &admin,
        json!({"expected_version":version,"prices":proposed}),
    )
    .await;
    success(&app,"PUT","/core/v1/admin/routing/prices",&admin,json!({"expected_version":version,"prices":proposed,"confirmation_digest":preview["confirmation_digest"]})).await;
    let current = active(&pool).await;
    let drafts = success(
        &app,
        "GET",
        "/core/v1/admin/routing/prices/drafts",
        &admin,
        Value::Null,
    )
    .await;
    assert_eq!(drafts["items"][0]["stale"], true);
    assert_eq!(
        request(
            app.clone(),
            "POST",
            "/core/v1/admin/routing/prices/drafts/10/preview",
            &admin,
            json!({"expected_version":Version::of(&current)})
        )
        .await
        .0,
        StatusCode::CONFLICT
    );
    assert_eq!(active(&pool).await, current);
    success(
        &app,
        "DELETE",
        "/core/v1/admin/routing/prices/drafts/10",
        &admin,
        json!({"expected_version":Version::of(&current)}),
    )
    .await;
    let details = success(&app, "GET", "/api/channel/2", &admin, Value::Null).await;
    assert_eq!(details["name"], "Edited provider");
}

#[sqlx::test(migrations = false)]
async fn channel_copy_publishes_independent_keys_and_executable_bindings(pool: PgPool) {
    use std::sync::atomic::{AtomicUsize, Ordering};
    let (_, _, admin, ordinary) = vault_app(&pool).await;
    let role = runtime::RuntimeRole::create(&pool).await;
    role.optional("core_charging").await;
    role.optional("core_model").await;
    let live = gateway(&role.pool)
        .with_channel_secret_key(&[83; 32])
        .unwrap();
    let manager = ManagedChannels::recover(role.pool.clone(), live.clone())
        .await
        .unwrap();
    let app = managed_router(role.pool.clone(), manager);
    let expected = Version::from(live.dashboard_routing().version().unwrap());
    let input = json!({"expected_version":expected,"suffix":"_copy"});
    assert_eq!(
        request(
            app.clone(),
            "POST",
            "/core/v1/admin/routing/channels/1/copy",
            &ordinary,
            input.clone()
        )
        .await
        .0,
        StatusCode::FORBIDDEN
    );
    let (first, second) = tokio::join!(
        request(
            app.clone(),
            "POST",
            "/core/v1/admin/routing/channels/1/copy",
            &admin,
            input.clone()
        ),
        request(
            app.clone(),
            "POST",
            "/core/v1/admin/routing/channels/1/copy",
            &admin,
            input
        ),
    );
    let (status, reply) = if first.0 == StatusCode::OK {
        assert_eq!(second.0, StatusCode::CONFLICT, "{second:?}");
        first
    } else {
        assert_eq!(first.0, StatusCode::CONFLICT, "{first:?}");
        second
    };
    assert_eq!(status, StatusCode::OK, "{reply}");
    let id = reply["data"]["id"].as_u64().unwrap();
    assert_ne!(id, 1);
    assert_eq!(
        reply["data"]["version"],
        json!({"routing":expected.routing+1,"pricing":expected.pricing})
    );
    assert!(!reply.to_string().contains("synthetic-channel-only"));
    let document = active(&pool).await;
    let source = &document.routing.upstreams[0];
    let copied = document
        .routing
        .upstreams
        .iter()
        .find(|u| u.id == id)
        .unwrap();
    assert_eq!(source.id, 1);
    assert_eq!(copied.endpoint, source.endpoint);
    assert_eq!(copied.protocol, source.protocol);
    assert_eq!(copied.enabled, source.enabled);
    assert_ne!(copied.credential, source.credential);
    assert_eq!(document.pricing, fixture().pricing);
    assert_eq!(document.routing.routes[0].targets.len(), 2);
    let original_target = &document.routing.routes[0].targets[0];
    let copied_target = &document.routing.routes[0].targets[1];
    assert_eq!(copied_target.upstream_id, id);
    assert_ne!(copied_target.id, original_target.id);
    assert_eq!(copied_target.upstream_model, original_target.upstream_model);
    assert_eq!(copied_target.weight, original_target.weight);
    assert_eq!(copied_target.priority, original_target.priority);
    let detail = success(
        &app,
        "GET",
        &format!("/api/channel/{id}"),
        &admin,
        Value::Null,
    )
    .await;
    assert_eq!(detail["name"], "Channel 1_copy");
    let keys: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM public.channel_secret_revisions WHERE upstream_id=$1",
    )
    .bind(id as i64)
    .fetch_one(&pool)
    .await
    .unwrap();
    assert_eq!(keys, 1, "copy must own its independent encrypted key");
    let ciphertext: Vec<u8> = sqlx::query_scalar(
        "SELECT ciphertext FROM public.channel_secret_revisions WHERE upstream_id=$1",
    )
    .bind(id as i64)
    .fetch_one(&pool)
    .await
    .unwrap();
    assert!(
        !ciphertext
            .windows(b"synthetic-channel-only".len())
            .any(|s| s == b"synthetic-channel-only")
    );
    let denied = sqlx::query("SELECT ciphertext FROM public.channel_secret_revisions")
        .fetch_all(&role.pool)
        .await
        .unwrap_err();
    assert_eq!(
        denied.as_database_error().and_then(|e| e.code()).as_deref(),
        Some("42501")
    );
    let audit: Value =
        sqlx::query_scalar("SELECT target FROM core_identity.audit WHERE action='channel.copy'")
            .fetch_one(&pool)
            .await
            .unwrap();
    assert_eq!(
        audit["change"],
        json!({"source_id":1,"id":id,"reset_balance":true})
    );
    success(&app, "POST", "/core/v1/admin/routing/channels/1/keys", &admin,
        json!({"operation":"add","expected_version":Version::of(&document),"label":"Source rotation","secret":"synthetic-source-rotated","activate":true})).await;
    let after_rotation = active(&pool).await;
    assert_eq!(
        after_rotation
            .routing
            .upstreams
            .iter()
            .find(|u| u.id == id)
            .unwrap()
            .credential,
        copied.credential
    );
    success(
        &app,
        "PUT",
        "/api/channel/1/status",
        &admin,
        json!({"expected_version":Version::of(&after_rotation),"enabled":false}),
    )
    .await;
    drop(app);
    drop(live);
    let restored = gateway(&role.pool)
        .with_channel_secret_key(&[83; 32])
        .unwrap();
    let manager = ManagedChannels::recover(role.pool.clone(), restored.clone())
        .await
        .unwrap();
    assert!(
        restored.dashboard_routing().route(route_request()).is_ok(),
        "copy must remain routable after disabling its source"
    );
    let hits = Arc::new(AtomicUsize::new(0));
    let seen = hits.clone();
    let provider = Router::new().route(
        "/v1/models",
        axum::routing::get(move |headers: axum::http::HeaderMap| {
            let seen = seen.clone();
            async move {
                assert_eq!(
                    headers.get(header::AUTHORIZATION).unwrap(),
                    "Bearer synthetic-channel-only"
                );
                seen.fetch_add(1, Ordering::SeqCst);
                axum::Json(json!({"data":[{"id":"model-a"}]}))
            }
        }),
    );
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let server = tokio::spawn(async move {
        axum::serve(listener, provider).await.unwrap();
    });
    let mut transport_document = active(&pool).await;
    transport_document.routing.version += 1;
    transport_document
        .routing
        .upstreams
        .iter_mut()
        .find(|u| u.id == id)
        .unwrap()
        .endpoint =
        lmm_core::routing::Endpoint::local_loopback(format!("http://{address}/v1/")).unwrap();
    transport_document
        .apply(&restored.dashboard_routing())
        .unwrap();
    let app = managed_router(role.pool.clone(), manager);
    let probe = success(
        &app,
        "GET",
        &format!("/api/channel/test/{id}"),
        &admin,
        Value::Null,
    )
    .await;
    assert_eq!(probe["models"], json!(["model-a"]));
    assert_eq!(hits.load(Ordering::SeqCst), 1);
    server.abort();
    drop(app);
    drop(restored);
    role.cleanup().await;
}

#[sqlx::test(migrations = false)]
async fn channel_copy_legacy_names_and_failures_are_atomic(pool: PgPool) {
    let (app, live, admin, ordinary) = vault_app(&pool).await;
    for (query, name) in [
        ("", "Channel 1_复制"),
        ("?suffix=", "Channel 1"),
        ("?reset_balance=false", "Channel 1_复制"),
        (
            "?suffix=%E5%89%AF%E6%9C%AC&reset_balance=true",
            "Channel 1副本",
        ),
    ] {
        let copied = success(
            &app,
            "POST",
            &format!("/api/channel/copy/1{query}"),
            &admin,
            Value::Null,
        )
        .await;
        let detail = success(
            &app,
            "GET",
            &format!("/api/channel/{}", copied["id"]),
            &admin,
            Value::Null,
        )
        .await;
        assert_eq!(detail["name"], name);
    }
    let before = active(&pool).await;
    assert_eq!(
        request(
            app.clone(),
            "POST",
            "/api/channel/copy/1?reset_balance=false",
            &ordinary,
            Value::Null
        )
        .await
        .0,
        StatusCode::FORBIDDEN
    );
    let input = json!({"expected_version":Version::of(&before),"suffix":"_copy"});
    assert_eq!(
        request(
            app.clone(),
            "POST",
            "/core/v1/admin/routing/channels/999/copy",
            &admin,
            input.clone()
        )
        .await
        .0,
        StatusCode::NOT_FOUND
    );
    let mut invalid = input.clone();
    invalid["suffix"] = json!("x".repeat(129));
    assert_eq!(
        request(
            app.clone(),
            "POST",
            "/core/v1/admin/routing/channels/1/copy",
            &admin,
            invalid
        )
        .await
        .0,
        StatusCode::BAD_REQUEST
    );
    let keys_before: i64 =
        sqlx::query_scalar("SELECT count(*) FROM public.channel_secret_revisions")
            .fetch_one(&pool)
            .await
            .unwrap();
    sqlx::raw_sql("CREATE FUNCTION public.reject_copy_audit() RETURNS trigger LANGUAGE plpgsql AS $$ BEGIN IF NEW.action='channel.copy' THEN RAISE EXCEPTION 'synthetic copy audit failure'; END IF; RETURN NEW; END $$; CREATE TRIGGER reject_copy_audit BEFORE INSERT ON core_identity.audit FOR EACH ROW EXECUTE FUNCTION public.reject_copy_audit();").execute(&pool).await.unwrap();
    assert_eq!(
        request(
            app,
            "POST",
            "/core/v1/admin/routing/channels/1/copy",
            &admin,
            input
        )
        .await
        .0,
        StatusCode::SERVICE_UNAVAILABLE
    );
    assert_eq!(active(&pool).await, before);
    assert_eq!(
        Version::from(live.dashboard_routing().version().unwrap()),
        Version::of(&before)
    );
    let keys_after: i64 =
        sqlx::query_scalar("SELECT count(*) FROM public.channel_secret_revisions")
            .fetch_one(&pool)
            .await
            .unwrap();
    assert_eq!(
        keys_before, keys_after,
        "failed copy must roll back encrypted key"
    );
}

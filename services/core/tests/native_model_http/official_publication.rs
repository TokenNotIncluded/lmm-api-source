//! The admin API accepts only reviewed HTTPS destinations. Its durable release supplies
//! all prices and routes to the serving fixture; only supplier network addresses change.
use super::*;
use lmm_core::{
    channel_http::{ManagedChannels, managed_router},
    model_http::ExternalModelContext,
};
use reqwest::Method;
use store::ReleaseDocument;

pub(super) struct Publisher {
    pool: PgPool,
    client: reqwest::Client,
    admin: String,
    origin: String,
    server: JoinHandle<()>,
}

impl Drop for Publisher {
    fn drop(&mut self) {
        self.server.abort();
    }
}

impl Publisher {
    pub(super) async fn new(pool: &PgPool, admin: String, bootstrap: &ReleaseDocument) -> Self {
        let mut bootstrap = bootstrap.clone();
        let contexts: Vec<ExternalModelContext> = bootstrap
            .routing
            .upstreams
            .iter_mut()
            .map(|upstream| {
                let endpoint = format!("https://supplier-{}.example/v1/", upstream.id);
                upstream.endpoint = Endpoint::try_from(endpoint.clone()).unwrap();
                serde_json::from_value(json!({
                "upstream_id": upstream.id, "upstream_model": "provider-model",
                "protocol": upstream.protocol, "endpoint": endpoint,
                "max_input_tokens": 100, "max_output_tokens": 20,
                "contract_revision": "synthetic-four-protocol-v1",
                "missing_cached_tokens_is_zero": true, "missing_cache_creation_tokens_is_zero": true
            })).unwrap()
            })
            .collect();
        let gateway = four_protocol_gateway(pool, bootstrap, contexts);
        let management = ManagedChannels::recover(pool.clone(), gateway)
            .await
            .unwrap();
        let app = managed_router(pool.clone(), management);
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let origin = format!("http://{}", listener.local_addr().unwrap());
        let server = tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
        Self {
            pool: pool.clone(),
            client: reqwest::Client::builder()
                .no_proxy()
                .timeout(Duration::from_secs(5))
                .build()
                .unwrap(),
            admin,
            origin,
            server,
        }
    }

    async fn configuration(&self, method: Method, suffix: &str, body: Option<&Value>) -> Value {
        let url = format!(
            "{}/core/v1/admin/routing/configuration{suffix}",
            self.origin
        );
        let mut req = self.client.request(method, url).bearer_auth(&self.admin);
        if let Some(body) = body {
            req = req.json(body);
        }
        let response = req.send().await.unwrap();
        let status = response.status();
        let result: Value = response.json().await.unwrap();
        assert_eq!(status, StatusCode::OK, "{suffix}: {result}");
        assert_eq!(result["success"], true);
        result["data"].clone()
    }

    pub(super) async fn publish(&self, output_rate: u64) -> ReleaseDocument {
        let before = self.configuration(Method::GET, "", None).await;
        let mut proposal = json!({
            "expected_version": before["version"], "routing": before["routing"],
            "prices": before["prices"], "bindings": before["bindings"]
        });
        proposal["routing"]["version"] = json!(before["version"]["routing"].as_u64().unwrap() + 1);
        proposal["prices"]["version"] = json!(before["version"]["pricing"].as_u64().unwrap() + 1);
        proposal["bindings"]["official_prices"] = json!({
            "models": {"native-model": {
                "rates": {"input":2_000_000, "output":output_rate, "cache_read":500_000, "cache_write":0, "request":0},
                "billing_mode":"tokens", "source":"fixture", "source_unit":"micro-USD per million tokens"
            }},
            "group_multipliers_ppm":{"allowed":1_000_000},
            "speed_multipliers_ppm":{"fast":1_000_000,"ultrafast":1_000_000}
        });
        let preview = self
            .configuration(Method::POST, "/preview", Some(&proposal))
            .await;
        assert_eq!(preview["saved"], false);
        assert_eq!(self.configuration(Method::GET, "", None).await, before);
        let digest = preview["confirmation_digest"].as_str().unwrap();
        assert_eq!(digest.len(), 64);
        proposal["confirmation_digest"] = json!(digest);
        let published = self.configuration(Method::PUT, "", Some(&proposal)).await;
        let current = self.configuration(Method::GET, "", None).await;
        assert_eq!(current["version"], published["version"]);
        assert_eq!(
            current["bindings"]["official_prices"],
            proposal["bindings"]["official_prices"]
        );
        assert_eq!(current["prices"], preview["prices"]);
        let rules = current["prices"]["rules"].as_array().unwrap();
        assert_eq!(rules.len(), 1);
        assert_eq!(rules[0]["billing_mode"], "official_flat");
        assert_eq!(rules[0]["model"], "native-model");
        assert_eq!(rules[0]["group"], "allowed");
        assert_eq!(rules[0]["speed"], "standard");
        let canonical = &rules[0]["tiered"];
        assert_eq!(canonical["revision"], 2);
        assert_eq!(canonical["uses_cache_read"], true);
        assert_eq!(canonical["uses_cache_write"], true);
        assert_eq!(canonical["uses_cache_write_1h"], true);
        let bands = canonical["tiers"].as_array().unwrap();
        assert_eq!(bands.len(), 1);
        assert_eq!(
            bands[0]["rates"]["cache_write_1h"],
            bands[0]["rates"]["cache_write"]
        );

        let saved: Value = sqlx::query_scalar("SELECT public.routing_control_read()")
            .fetch_one(&self.pool)
            .await
            .unwrap();
        assert_eq!(saved["routing"], current["routing"]);
        assert_eq!(saved["pricing"], current["prices"]);
        serde_json::from_value(saved).unwrap()
    }
}

pub(super) fn use_local_suppliers(release: &mut ReleaseDocument, suppliers: &[(Protocol, Server)]) {
    let mut expected = serde_json::to_value(&*release).unwrap();
    assert_eq!(release.routing.upstreams.len(), suppliers.len());
    for (index, upstream) in release.routing.upstreams.iter_mut().enumerate() {
        let (protocol, supplier) = &suppliers[usize::try_from(upstream.id - 1).unwrap()];
        assert_eq!(upstream.protocol, *protocol);
        assert_eq!(
            upstream.endpoint.as_str(),
            format!("https://supplier-{}.example/v1/", upstream.id)
        );
        upstream.endpoint = Endpoint::local_loopback(supplier.endpoint.clone()).unwrap();
        expected["routing"]["upstreams"][index]["endpoint"] = json!(supplier.endpoint);
    }
    assert_eq!(serde_json::to_value(release).unwrap(), expected);
}

pub(super) async fn frozen(pool: &PgPool, id: &str) -> Value {
    sqlx::query_scalar("SELECT price FROM core_charging.requests WHERE id=$1")
        .bind(id)
        .fetch_one(pool)
        .await
        .unwrap()
}

pub(super) async fn check_attempt(pool: &PgPool, id: &str, supplier: usize, protocol: Protocol) {
    let (cached, binding): (i64, Value) = sqlx::query_as(
        "SELECT r.cached_tokens, e.detail->'binding' FROM core_charging.requests r
         JOIN core_charging.usage_events e ON e.request_id=r.id AND e.event_id='reservation-contract:v1'
         WHERE r.id=$1"
    ).bind(id).fetch_one(pool).await.unwrap();
    let route_id = format!("r1-t{supplier}-u{supplier}");
    let protocol = match protocol {
        Protocol::OpenAiChat => "chat",
        Protocol::OpenAiResponses => "responses",
        Protocol::AnthropicMessages => "anthropic",
        Protocol::Gemini => "gemini",
        _ => unreachable!(),
    };
    assert_eq!(cached, 2);
    assert_eq!(binding["route_id"], route_id);
    assert_eq!(binding["target_id"], supplier);
    assert_eq!(binding["upstream_id"], supplier);
    assert_eq!(binding["protocol"], protocol);
    let dispatches: Vec<String> = sqlx::query_scalar(
        "SELECT detail->>'route_id' FROM core_charging.usage_events WHERE request_id=$1 AND event_id LIKE 'route-dispatch:%' ORDER BY event_id"
    ).bind(id).fetch_all(pool).await.unwrap();
    assert_eq!(dispatches, [route_id]);
}

pub(super) fn check_frozen(price: &Value, protocol: Protocol) {
    let frozen = &price["tiered"];
    let tariff = &frozen["tariff"];
    let (revision, basis) = match protocol {
        Protocol::OpenAiChat | Protocol::OpenAiResponses => (4, "inclusive"),
        Protocol::AnthropicMessages => (2, "anthropic_text"),
        Protocol::Gemini => (1, "inclusive"),
        _ => unreachable!(),
    };
    assert_eq!(price["model"], "native-model");
    assert_eq!(price["group"], "allowed");
    assert_eq!(tariff["revision"], revision);
    assert_eq!(tariff["source_digest"].as_str().unwrap().len(), 64);
    assert_eq!(frozen["input_semantics"], basis);
    assert_eq!(frozen["credits_per_usd"], 500_000);
    assert_eq!(frozen["group_multiplier_ppm"], 1_000_000);
    assert_eq!(frozen["speed_multiplier_ppm"], 1_000_000);
    assert_eq!(tariff["uses_cache_read"], true);
    assert_eq!(tariff["uses_cache_write"], revision != 1);
    assert_eq!(
        tariff["uses_cache_write_1h"].as_bool().unwrap_or(false),
        revision == 2
    );
    assert_eq!(
        tariff["uses_cache_write_12h"].as_bool().unwrap_or(false),
        revision == 4
    );
    assert_eq!(tariff["tiers"].as_array().unwrap().len(), 1);
    let rates = &tariff["tiers"][0]["rates"];
    for (meter, numerator) in [
        ("input", 2_000_000),
        ("output", 4_000_000),
        ("cache_read", 500_000),
        ("cache_write", 0),
    ] {
        let rate = &rates[meter];
        // Equivalent reduced and source-denominator fractions carry the same fee.
        let n = rate["numerator"].as_u64().unwrap() as u128;
        let d = rate["denominator"].as_u64().unwrap() as u128;
        assert!(d > 0);
        assert_eq!(n * 1_000_000, d * numerator);
    }
}

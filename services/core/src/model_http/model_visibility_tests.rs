use super::*;
use axum::{body::Body, http::Request};
use serde_json::Value;
use tower::ServiceExt;

async fn call(g: &ModelGateway, key: &str, path: &str, body: Value) -> (u16, Value) {
    let response = g
        .router()
        .oneshot(
            Request::post(path)
                .header("content-type", "application/json")
                .header("authorization", format!("Bearer {key}"))
                .body(Body::from(body.to_string()))
                .unwrap(),
        )
        .await
        .unwrap();
    let status = response.status().as_u16();
    let body = axum::body::to_bytes(response.into_body(), 16384)
        .await
        .unwrap();
    (status, serde_json::from_slice(&body).unwrap())
}

async fn chat(g: &ModelGateway, key: &str, model: &str) -> (u16, Value) {
    call(
        g,
        key,
        "/v1/chat/completions",
        json!({
            "model":model,"messages":[{"role":"user","content":"test"}]
        }),
    )
    .await
}

#[sqlx::test(migrations = false)]
async fn model_http_scopes_missing_forbidden_and_unavailable_models(pool: PgPool) {
    let f = fixture(pool, 10_000_000).await;
    let supplier = CompatibilityProvider::start().await;
    let g = gateway(&f, &supplier.endpoint, vec![]);
    let router = &g.bridge.routing;
    let mut doc = router.catalog_document().unwrap();
    doc.routing.version += 1;
    doc.pricing.version += 1;
    let mut private = doc.routing.routes[0].clone();
    private.id = 3;
    private.targets[0].id = 3;
    private.group = "private".into();
    private.model = "private-model".into();
    let mut price = doc.pricing.rules[0].clone();
    price.id = 3;
    price.group = private.group.clone();
    price.model = private.model.clone();
    doc.pricing.rules.push(price);
    doc.routing.groups.push(routing::ModelGroup {
        name: private.group.clone(),
        enabled: true,
    });
    doc.routing.routes.push(private);
    doc.routing.aliases.push(routing::ModelAlias {
        group: "allowed".into(),
        alias: "permitted-alias".into(),
        model: "large-model".into(),
    });
    let release = router
        .publish(
            router.version().unwrap(),
            doc.routing,
            Arc::new(PriceSnapshot::compile(doc.pricing).unwrap()),
        )
        .unwrap();

    assert_eq!(chat(&g, &f.key, "invalid model").await.0, 400);
    let unknown = chat(&g, &f.key, "model-typo").await;
    assert_eq!(unknown.0, 404);
    assert_eq!(unknown.1["error"]["code"], "model_not_found");
    assert_eq!(chat(&g, &f.key, "private-model").await, unknown);
    assert_eq!(chat(&g, "invalid", "small-model").await.0, 401);
    let responses = call(
        &g,
        &f.key,
        "/v1/responses",
        json!({"model":"typo","input":"test"}),
    )
    .await;
    assert_eq!(responses, unknown);
    for (path, body) in [
        ("/v1/embeddings", json!({"model":"typo","input":"test"})),
        (
            "/v1/rerank",
            json!({"model":"typo","query":"test","documents":["test"]}),
        ),
    ] {
        assert_eq!(call(&g, &f.key, path, body).await, unknown);
    }
    let anthropic = call(
        &g,
        &f.key,
        "/v1/messages",
        json!({
            "model":"typo","max_tokens":8,"messages":[{"role":"user","content":"test"}]
        }),
    )
    .await;
    assert_eq!(anthropic.0, 404);
    assert_eq!(anthropic.1["error"]["type"], "not_found_error");
    let gemini = call(
        &g,
        &f.key,
        "/v1beta/models/typo:generateContent",
        json!({
            "contents":[{"role":"user","parts":[{"text":"test"}]}]
        }),
    )
    .await;
    assert_eq!(gemini.0, 404);
    assert_eq!(gemini.1["error"]["status"], "NOT_FOUND");

    sqlx::query("INSERT INTO core_identity.api_key_settings(credential_id,name,group_name,model_limits_enabled,model_limits) VALUES($1,'visibility','allowed',TRUE,ARRAY['small-model','permitted-alias','permitted-typo'])")
        .bind(f.key_id).execute(&f.pool).await.unwrap();
    let denied = chat(&g, &f.key, "large-model").await;
    assert_eq!(denied.0, 403);
    assert_eq!(chat(&g, &f.key, "model-typo").await, denied);
    assert_eq!(chat(&g, &f.key, "permitted-alias").await, unknown);
    assert_eq!(chat(&g, &f.key, "permitted-typo").await, unknown);

    router.set_operational_enabled(release, 1, false).unwrap();
    let unavailable = chat(&g, &f.key, "small-model").await;
    assert_eq!(unavailable.0, 503);
    assert_eq!(unavailable.1["error"]["code"], "route_unavailable");
    assert_eq!(chat(&g, &f.key, "large-model").await, denied);
    assert_eq!(chat(&g, &f.key, "permitted-alias").await, unknown);
    assert_eq!(requests(&f.pool).await, 0);
    assert!(supplier.bodies.lock().unwrap().is_empty());
    assert_balance(&f, 10_000_000, 0).await;
}

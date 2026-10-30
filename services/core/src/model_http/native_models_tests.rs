use super::*;
use crate::{routing::Protocol, test_runtime::RuntimeRole};
use axum::{body::to_bytes, http::Request as HttpRequest};
use tower::ServiceExt;

fn models() -> Vec<Model> {
    ["a", "b", "c"]
        .into_iter()
        .map(|id| Model {
            id: id.into(),
            input: 2000,
            output: 1000,
        })
        .collect()
}
#[test]
fn native_model_pages_are_bounded_and_directional() {
    let models = models();
    let first = AnthropicPage::parse(Some("limit=1"))
        .unwrap()
        .render(&models)
        .unwrap();
    assert_eq!(first["first_id"], "a");
    assert_eq!(first["has_more"], true);
    let after = AnthropicPage::parse(Some("limit=1&after_id=a"))
        .unwrap()
        .render(&models)
        .unwrap();
    assert_eq!(after["last_id"], "b");
    let before = AnthropicPage::parse(Some("limit=1&before_id=c"))
        .unwrap()
        .render(&models)
        .unwrap();
    assert_eq!(before["first_id"], "b");
    assert_eq!(before["has_more"], true);
    assert!(AnthropicPage::parse(Some("limit=0")).is_err());
    assert!(AnthropicPage::parse(Some("limit=1&limit=2")).is_err());
    assert!(AnthropicPage::parse(Some("before_id=b&after_id=a")).is_err());
    assert!(
        AnthropicPage::parse(Some("after_id=hidden"))
            .unwrap()
            .render(&models)
            .is_err()
    );
    let empty = AnthropicPage::parse(None).unwrap().render(&[]).unwrap();
    assert_eq!(empty["first_id"], Value::Null);
    assert_eq!(empty["has_more"], false);
}
#[test]
fn gemini_page_token_binds_visible_view_and_requested_page_size() {
    let mut view = View {
        models: models(),
        digest: [1; 32],
    };
    let first = GeminiPage::parse(Some("pageSize=2"))
        .unwrap()
        .render(&view)
        .unwrap();
    let token = first["nextPageToken"].as_str().unwrap();
    let page = GeminiPage::parse(Some(&format!("pageSize=2&pageToken={token}"))).unwrap();
    let next = page.render(&view).unwrap();
    assert_eq!(next["models"][0]["name"], "models/c");
    assert!(next.get("nextPageToken").is_none());
    assert!(
        GeminiPage::parse(Some(&format!("pageSize=1&pageToken={token}")))
            .unwrap()
            .render(&view)
            .is_err()
    );
    view.digest = [2; 32];
    assert!(page.render(&view).is_err());
    assert_eq!(
        GeminiPage::parse(Some("pageSize=1001"))
            .unwrap()
            .render(&view)
            .unwrap()["models"]
            .as_array()
            .unwrap()
            .len(),
        3
    );
    assert!(GeminiPage::parse(Some("pageSize=-1")).is_err());
    assert!(GeminiPage::parse(Some("key=secret")).is_err());
}
async fn get(
    gateway: &ModelGateway,
    key: &str,
    path: &str,
    anthropic: bool,
) -> (StatusCode, Value) {
    let mut request = HttpRequest::builder().uri(path).header(
        if anthropic {
            "x-api-key"
        } else {
            "x-goog-api-key"
        },
        key,
    );
    if anthropic {
        request = request.header("anthropic-version", "2023-06-01");
    }
    let response = gateway
        .router()
        .oneshot(request.body(Body::empty()).unwrap())
        .await
        .unwrap();
    let status = response.status();
    assert_eq!(response.headers()[header::CACHE_CONTROL], "no-store");
    (
        status,
        serde_json::from_slice(&to_bytes(response.into_body(), 1 << 20).await.unwrap()).unwrap(),
    )
}
#[sqlx::test(migrations = false)]
async fn native_models_pg_authorized_gemini_pagination_and_single_model(pool: PgPool) {
    let f = native_http::test_support::fixture(pool, 10000, &["alpha", "vendor:model"]).await;
    let role = RuntimeRole::create(&f.pool).await;
    role.optional("core_charging").await;
    role.optional("core_model").await;
    let g = super::super::text_request_tests::gateway(
        &role.pool,
        "http://127.0.0.1:4999/v1beta",
        Protocol::Gemini,
        &["alpha", "vendor:model", "hidden"],
        1,
        false,
    );
    let (status, first) = get(&g, &f.key, "/v1beta/models?pageSize=1", false).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(first["models"][0]["name"], "models/alpha");
    let token = first["nextPageToken"].as_str().unwrap();
    let path = format!("/v1beta/models?pageSize=1&pageToken={token}");
    let (_, next) = get(&g, &f.key, &path, false).await;
    assert_eq!(next["models"][0]["name"], "models/vendor:model");
    assert!(next.get("nextPageToken").is_none());
    let (status, model) = get(&g, &f.key, "/v1beta/models/vendor%3Amodel", false).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(
        model["supportedGenerationMethods"],
        json!(["generateContent"])
    );
    assert_eq!(model["inputTokenLimit"], 2000);
    for id in ["hidden", "missing"] {
        let (status, error) = get(&g, &f.key, &format!("/v1beta/models/{id}"), false).await;
        assert_eq!(status, StatusCode::NOT_FOUND);
        assert_eq!(error["error"]["message"], "Model not found");
    }
    assert_eq!(
        get(&g, &f.session, "/v1beta/models", false).await.0,
        StatusCode::FORBIDDEN
    );
    sqlx::query("UPDATE core_identity.api_key_settings SET model_limits=ARRAY['vendor:model'] WHERE credential_id=$1").bind(f.key_id).execute(&f.pool).await.unwrap();
    assert_eq!(
        get(&g, &f.key, &path, false).await.0,
        StatusCode::BAD_REQUEST
    );
    assert_eq!(
        get(&g, &f.key, "/v1beta/models/alpha", false).await.0,
        StatusCode::NOT_FOUND
    );
    assert_eq!(
        sqlx::query_scalar::<_, i64>("SELECT count(*) FROM core_charging.requests")
            .fetch_one(&f.pool)
            .await
            .unwrap(),
        0
    );
    drop(g);
    role.cleanup().await;
}
#[sqlx::test(migrations = false)]
async fn native_models_pg_anthropic_header_dispatch_preserves_openai_and_checks_protocol(
    pool: PgPool,
) {
    let f = native_http::test_support::fixture(pool, 10000, &["a", "b", "c"]).await;
    let g = super::super::text_request_tests::gateway(
        &f.pool,
        "http://127.0.0.1:4999/v1",
        Protocol::AnthropicMessages,
        &["a", "b", "c"],
        1,
        false,
    );
    let (status, first) = get(&g, &f.key, "/v1/models?limit=1", true).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(first["data"][0]["type"], "model");
    assert_eq!(first["first_id"], "a");
    let (_, next) = get(&g, &f.key, "/v1/models?limit=1&after_id=a", true).await;
    assert_eq!(next["last_id"], "b");
    let response = g
        .router()
        .oneshot(
            HttpRequest::builder()
                .uri("/v1/models")
                .header(header::AUTHORIZATION, format!("Bearer {}", f.key))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let openai: Value =
        serde_json::from_slice(&to_bytes(response.into_body(), 1 << 20).await.unwrap()).unwrap();
    assert_eq!(openai["object"], "list");
    assert_eq!(openai["data"][0]["object"], "model");
    let chat = super::super::text_request_tests::gateway(
        &f.pool,
        "http://127.0.0.1:4999/v1",
        Protocol::OpenAiChat,
        &["a", "b", "c"],
        1,
        false,
    );
    let (_, native) = get(&chat, &f.key, "/v1/models", true).await;
    assert_eq!(native["data"], json!([]));
    assert_eq!(
        sqlx::query_scalar::<_, i64>("SELECT count(*) FROM core_charging.requests")
            .fetch_one(&f.pool)
            .await
            .unwrap(),
        0
    );
}

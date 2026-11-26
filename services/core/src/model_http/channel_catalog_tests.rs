// Copyright (C) 2026 LIghtJUNction. SPDX-License-Identifier: AGPL-3.0-or-later.
use super::{ChannelProbe, ModelGateway, parse_models};
use crate::{
    model_http::{ChargePolicy, UpstreamCredential},
    relay::Limits,
    routing::{CredentialRef, PriceConfig, PriceSnapshot, Protocol, RoutingConfig, UpstreamConfig},
};
use axum::{
    Json, Router,
    body::Body,
    extract::State,
    http::{Request, StatusCode},
};
use serde_json::{Value, json};
use std::{
    collections::BTreeMap,
    sync::{Arc, Mutex},
    time::Duration,
};
use tokio::{net::TcpListener, task::JoinHandle};

const SECRET: &str = "synthetic-catalogue-only";

#[derive(Clone, Debug)]
struct ObservedRequest {
    path: String,
    query: BTreeMap<String, String>,
    key: Option<String>,
    authorization: Option<String>,
    version: Option<String>,
}

#[derive(Clone)]
struct Supplier {
    pages: Arc<Vec<(StatusCode, Value)>>,
    requests: Arc<Mutex<Vec<ObservedRequest>>>,
    protocol: Protocol,
}

async fn respond(
    State(supplier): State<Supplier>,
    request: Request<Body>,
) -> (StatusCode, Json<Value>) {
    let url = reqwest::Url::parse(&format!("http://localhost{}", request.uri())).unwrap();
    let key_header = if supplier.protocol == Protocol::Gemini {
        "x-goog-api-key"
    } else {
        "x-api-key"
    };
    let mut requests = supplier.requests.lock().unwrap();
    let index = requests.len();
    requests.push(ObservedRequest {
        path: request.uri().path().to_owned(),
        query: url.query_pairs().into_owned().collect(),
        authorization: request
            .headers()
            .get("authorization")
            .map(|v| v.to_str().unwrap().to_owned()),
        key: request
            .headers()
            .get(key_header)
            .map(|value| value.to_str().unwrap().to_owned()),
        version: request
            .headers()
            .get("anthropic-version")
            .map(|value| value.to_str().unwrap().to_owned()),
    });
    let (status, body) = supplier
        .pages
        .get(index)
        .cloned()
        .unwrap_or((StatusCode::INTERNAL_SERVER_ERROR, json!({})));
    (status, Json(body))
}

struct Fixture {
    gateway: ModelGateway,
    upstream: UpstreamConfig,
    requests: Arc<Mutex<Vec<ObservedRequest>>>,
    task: JoinHandle<()>,
}
impl Drop for Fixture {
    fn drop(&mut self) {
        self.task.abort();
    }
}
impl Fixture {
    async fn new(protocol: Protocol, pages: Vec<(StatusCode, Value)>) -> Self {
        Self::with_base(protocol, pages, "https://catalogue.example/v1/").await
    }
    async fn with_base(protocol: Protocol, pages: Vec<(StatusCode, Value)>, base: &str) -> Self {
        let requests = Arc::new(Mutex::new(Vec::new()));
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let endpoint = format!("http://{}/", listener.local_addr().unwrap());
        let app = Router::new().fallback(respond).with_state(Supplier {
            pages: Arc::new(pages),
            requests: requests.clone(),
            protocol,
        });
        let task = tokio::spawn(async move {
            axum::serve(listener, app).await.unwrap();
        });
        let config: RoutingConfig = serde_json::from_value(json!({
            "version":1, "groups":[{"name":"default","enabled":true}],
            "upstreams":[{"id":1,"endpoint":base,"protocol":protocol,
            "credential":{"id":1,"revision":1},"enabled":true}], "routes":[], "aliases":[]
        }))
        .unwrap();
        let upstream = config.upstreams[0].clone();
        let pricing: PriceConfig = serde_json::from_value(json!({"version":1,"rules":[]})).unwrap();
        // Catalogue GETs do not use the database or create billable requests.
        let pool = sqlx::postgres::PgPoolOptions::new()
            .connect_lazy("postgresql://catalogue:synthetic-only@127.0.0.1:1/unused")
            .unwrap();
        let gateway = ModelGateway::new(
            pool,
            Arc::new(
                crate::routing::Router::new(
                    config,
                    Arc::new(PriceSnapshot::compile(pricing).unwrap()),
                )
                .unwrap(),
            ),
            vec![UpstreamCredential {
                reference: CredentialRef { id: 1, revision: 1 },
                secret: SECRET.to_owned(),
            }],
            BTreeMap::new(),
            Limits {
                total_timeout: Duration::from_secs(5),
                ..Limits::default()
            },
            ChargePolicy {
                input_token_ceiling: 1000,
                lease_seconds: 60,
            },
        )
        .unwrap()
        .with_local_streaming()
        .with_balance_probe_loopback(
            &reqwest::Url::parse(base)
                .unwrap()
                .origin()
                .ascii_serialization(),
            &endpoint,
        )
        .unwrap();
        Self {
            gateway,
            upstream,
            requests,
            task,
        }
    }
    async fn probe(&self) -> Result<ChannelProbe, crate::relay::RelayError> {
        self.gateway.probe_channel_snapshot(&self.upstream).await
    }
}

#[tokio::test]
async fn protocol_presets_fetch_the_vendor_catalogue_with_only_its_bearer_key() {
    for (base, protocol, expected) in [
        (
            "https://api.stepfun.com/v1",
            Protocol::AnthropicMessages,
            "/v1/models",
        ),
        (
            "https://api.stepfun.com/v1",
            Protocol::OpenAiResponses,
            "/v1/models",
        ),
        (
            "https://api.modelarts-maas.com/anthropic/v1",
            Protocol::AnthropicMessages,
            "/v2/models",
        ),
        (
            "https://api.modelarts-maas.com/v1",
            Protocol::OpenAiEmbeddings,
            "/v2/models",
        ),
        (
            "https://api.modelarts-maas.com/v1",
            Protocol::JinaRerank,
            "/v2/models",
        ),
    ] {
        let fixture = Fixture::with_base(
            protocol,
            vec![(
                StatusCode::OK,
                json!({"data":[{"id":"synthetic-returned"}]}),
            )],
            base,
        )
        .await;
        assert_eq!(
            fixture.probe().await.unwrap().models,
            ["synthetic-returned"]
        );
        let calls = fixture.requests.lock().unwrap();
        assert_eq!(calls.len(), 1);
        assert_eq!(calls[0].path, expected);
        assert_eq!(
            calls[0].authorization.as_deref(),
            Some("Bearer synthetic-catalogue-only")
        );
        assert_eq!(calls[0].key, None);
        assert_eq!(calls[0].version, None);
        assert!(calls[0].query.is_empty());
    }
}

#[tokio::test]
async fn gemini_catalogue_follows_pages_and_encodes_the_cursor() {
    let cursor = "next+/=&?page";
    let fixture = Fixture::new(
        Protocol::Gemini,
        vec![
            (
                StatusCode::OK,
                json!({"models":[{"name":"models/model-z"}],"nextPageToken":cursor}),
            ),
            (
                StatusCode::OK,
                json!({"models":[{"name":"models/model-a"},{"name":"models/model-z"}]}),
            ),
        ],
    )
    .await;
    let result = fixture.probe().await.unwrap();
    assert_eq!(result.models, ["model-a", "model-z"]);
    assert_eq!(result.validation_scope, "model_catalog");
    assert!(!result.inference_tested);
    let requests = fixture.requests.lock().unwrap();
    assert_eq!(requests.len(), 2);
    assert!(requests[0].query.is_empty());
    assert_eq!(
        requests[1].query,
        BTreeMap::from([("pageToken".to_owned(), cursor.to_owned())])
    );
    for request in requests.iter() {
        assert_eq!(request.path, "/v1/models");
        assert_eq!(request.key.as_deref(), Some(SECRET));
        assert!(request.version.is_none());
    }
}

#[tokio::test]
async fn anthropic_catalogue_uses_last_id_until_has_more_is_false() {
    let fixture = Fixture::new(
        Protocol::AnthropicMessages,
        vec![
            (
                StatusCode::OK,
                json!({"data":[{"id":"model-z"}],"has_more":true,"last_id":"model-z"}),
            ),
            (
                StatusCode::OK,
                json!({"data":[{"id":"model-a"}],"has_more":false,"last_id":"model-a"}),
            ),
        ],
    )
    .await;
    assert_eq!(
        fixture.probe().await.unwrap().models,
        ["model-a", "model-z"]
    );
    let requests = fixture.requests.lock().unwrap();
    assert_eq!(requests.len(), 2);
    assert!(requests[0].query.is_empty());
    assert_eq!(
        requests[1].query,
        BTreeMap::from([("after_id".to_owned(), "model-z".to_owned())])
    );
    for request in requests.iter() {
        assert_eq!(request.path, "/v1/models");
        assert_eq!(request.key.as_deref(), Some(SECRET));
        assert_eq!(request.version.as_deref(), Some("2023-06-01"));
    }
}

#[tokio::test]
async fn catalogue_rejects_later_page_failure_and_cursor_cycles_without_partial_success() {
    let first = json!({"models":[{"name":"models/model-a"}],"nextPageToken":"next"});
    for second in [
        (StatusCode::BAD_GATEWAY, json!({"error":"unavailable"})),
        (
            StatusCode::OK,
            json!({"models":[{"name":"models/model-b"}],"nextPageToken":"next"}),
        ),
    ] {
        let fixture = Fixture::new(
            Protocol::Gemini,
            vec![(StatusCode::OK, first.clone()), second],
        )
        .await;
        assert!(fixture.probe().await.is_err());
        assert_eq!(fixture.requests.lock().unwrap().len(), 2);
    }
}

#[tokio::test]
async fn catalogue_enforces_the_model_limit_across_pages() {
    let first: Vec<_> = (0..5001)
        .map(|i| json!({"id": format!("model-{i}")}))
        .collect();
    let second: Vec<_> = (5001..10001)
        .map(|i| json!({"id": format!("model-{i}")}))
        .collect();
    let fixture = Fixture::new(
        Protocol::AnthropicMessages,
        vec![
            (
                StatusCode::OK,
                json!({"data":first,"has_more":true,"last_id":"model-5000"}),
            ),
            (
                StatusCode::OK,
                json!({"data":second,"has_more":false,"last_id":"model-10000"}),
            ),
        ],
    )
    .await;
    assert!(fixture.probe().await.is_err());
    assert_eq!(fixture.requests.lock().unwrap().len(), 2);
}

#[test]
fn catalogue_rejects_invalid_continuations_and_empty_normalized_model_names() {
    for value in [
        json!({"models":[{"name":"models/model-a"}],"nextPageToken":42}),
        json!({"models":[{"name":"models/model-a"}],"nextPageToken":"bad\ncursor"}),
        json!({"models":[{"name":"models/"}]}),
    ] {
        assert!(
            parse_models(
                &serde_json::to_vec(&value).unwrap(),
                Protocol::Gemini,
                SECRET
            )
            .is_err()
        );
    }
    for value in [
        json!({"data":[{"id":"model-a"}],"has_more":true}),
        json!({"data":[{"id":"model-a"}],"has_more":true,"last_id":""}),
        json!({"data":[{"id":"model-a"}],"has_more":"true","last_id":"model-a"}),
    ] {
        assert!(
            parse_models(
                &serde_json::to_vec(&value).unwrap(),
                Protocol::AnthropicMessages,
                SECRET
            )
            .is_err()
        );
    }
}

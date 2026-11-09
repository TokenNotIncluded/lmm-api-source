use super::*;
use axum::{
    Router,
    body::{Body, to_bytes},
    http::{Request, header},
    response::IntoResponse,
    routing::get,
};
use std::sync::{Arc, Mutex};
use tokio::{net::TcpListener, task::JoinHandle};
use tower::ServiceExt;

#[derive(Clone)]
struct Fixture {
    body: Arc<Mutex<String>>,
    status: Arc<Mutex<StatusCode>>,
    queries: Arc<Mutex<Vec<String>>>,
    slow: Arc<Mutex<bool>>,
    chunked: Arc<Mutex<bool>>,
}

async fn provider(State(state): State<Fixture>, RawQuery(query): RawQuery) -> Response {
    state
        .queries
        .lock()
        .unwrap()
        .push(query.unwrap_or_default());
    let slow = *state.slow.lock().unwrap();
    if slow {
        tokio::time::sleep(Duration::from_millis(250)).await;
    }
    let body = state.body.lock().unwrap().clone();
    let status = *state.status.lock().unwrap();
    let body = if *state.chunked.lock().unwrap() {
        Body::from_stream(tokio_stream::iter(vec![
            Ok::<_, std::io::Error>(body[..body.len() / 2].to_owned()),
            Ok(body[body.len() / 2..].to_owned()),
        ]))
    } else {
        Body::from(body)
    };
    (status, [(header::LOCATION, "/unexpected")], body).into_response()
}

async fn fixture() -> (Reader, Fixture, JoinHandle<()>) {
    let state = Fixture {
        body: Arc::new(Mutex::new(
            json!({"amount":1,"base":"USD","date":"2026-01-02","rates":{"CNY":7.25}}).to_string(),
        )),
        status: Arc::new(Mutex::new(StatusCode::OK)),
        queries: Arc::new(Mutex::new(vec![])),
        slow: Arc::new(Mutex::new(false)),
        chunked: Arc::new(Mutex::new(false)),
    };
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let endpoint = Url::parse(&format!("http://{}/quote", listener.local_addr().unwrap())).unwrap();
    let app = Router::new()
        .route("/quote", get(provider))
        .route("/unexpected", get(provider))
        .with_state(state.clone());
    let task = tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
    // The production client policy is unchanged apart from the local HTTP
    // fixture transport and a shorter deadline for the timeout case.
    let reader = Reader {
        client: Some(
            http_client()
                .https_only(false)
                .timeout(Duration::from_millis(100))
                .build()
                .unwrap(),
        ),
        endpoint,
    };
    (reader, state, task)
}

#[tokio::test]
async fn fixed_provider_quotes_validate_currency_date_rate_and_transport_bounds() {
    for query in [
        None,
        Some("currency=CN"),
        Some("currency=https://127.0.0.1"),
        Some("currency=CNY&currency=USD"),
        Some("currency=CNY&url=https://other.test"),
    ] {
        assert!(currency(query).is_err());
    }
    assert_eq!(currency(Some("currency=%20cny%20")).unwrap(), "CNY");
    assert_eq!(Reader::new().endpoint.as_str(), PROVIDER_URL);
    let (reader, state, task) = fixture().await;
    assert_eq!(reader.fetch("USD").await.unwrap().rate, 1.0);
    assert!(state.queries.lock().unwrap().is_empty());
    let quote = reader.fetch("CNY").await.unwrap();
    assert_eq!(quote.rate, 7.25);
    assert_eq!(quote.rate_date, "2026-01-02");
    assert_eq!(quote.provider, "frankfurter.dev");
    assert!(chrono::DateTime::parse_from_rfc3339(&quote.fetched_at).is_ok());
    assert_eq!(
        state.queries.lock().unwrap().as_slice(),
        ["base=USD&symbols=CNY"]
    );

    for bad in [
        json!({"amount":1,"base":"EUR","date":"2026-01-02","rates":{"CNY":7.25}}),
        json!({"amount":2,"base":"USD","date":"2026-01-02","rates":{"CNY":7.25}}),
        json!({"amount":1,"base":"USD","date":"2026-01-02","rates":{"EUR":1.0}}),
        json!({"amount":1,"base":"USD","date":"not-a-date","rates":{"CNY":7.25}}),
        json!({"amount":1,"base":"USD","date":"2999-01-01","rates":{"CNY":7.25}}),
        json!({"amount":1,"base":"USD","date":"2026-01-02","rates":{"CNY":0}}),
        json!({"amount":1,"base":"USD","date":"2026-01-02","rates":{"CNY":1e10}}),
        json!({"amount":1,"base":"USD","date":"2026-01-02","rates":{"CNY":"7.25"}}),
    ] {
        *state.body.lock().unwrap() = bad.to_string();
        assert!(reader.fetch("CNY").await.is_err(), "{bad}");
    }
    let before_redirect = state.queries.lock().unwrap().len();
    *state.status.lock().unwrap() = StatusCode::FOUND;
    assert!(reader.fetch("CNY").await.is_err());
    assert_eq!(state.queries.lock().unwrap().len(), before_redirect + 1);
    *state.status.lock().unwrap() = StatusCode::INTERNAL_SERVER_ERROR;
    assert!(reader.fetch("CNY").await.is_err());
    *state.status.lock().unwrap() = StatusCode::OK;
    *state.body.lock().unwrap() = "x".repeat(MAX_RESPONSE_BYTES + 1);
    for chunked in [false, true] {
        *state.chunked.lock().unwrap() = chunked;
        assert!(reader.fetch("CNY").await.is_err());
    }
    *state.slow.lock().unwrap() = true;
    assert!(reader.fetch("CNY").await.is_err());
    task.abort();
}

#[sqlx::test(migrations = false)]
async fn quote_route_requires_a_current_root_session_and_never_saves(pool: sqlx::PgPool) {
    use super::super::{RuntimeOptions, router};
    use crate::{funding::FundingPolicy, identity::IdentityStore};

    crate::core_schema::init(&pool).await.unwrap();
    let identity = IdentityStore::from_pool(pool.clone());
    let root = identity.bootstrap_user(1, 6).await.unwrap();
    let admin = identity.bootstrap_user(2, 5).await.unwrap();
    let actor = identity.authorize(&root.secret).await.unwrap();
    let api_key = identity
        .issue_key(
            &root.secret,
            actor.owner,
            FundingPolicy::new_personal_key(),
            3600,
        )
        .await
        .unwrap();
    let (reader, fixture, task) = fixture().await;
    let mut state = OptionsHttp::new(
        pool.clone(),
        identity,
        RuntimeOptions {
            server_address: "https://console.example.test".into(),
            registration_enabled: false,
            password_register_enabled: false,
            oauth_register_enabled: false,
            registration_disabled_methods: vec![],
        },
    );
    state.exchange_rates = reader;
    let app = router(state);
    for (secret, expected) in [
        (None, StatusCode::UNAUTHORIZED),
        (Some(admin.secret.as_str()), StatusCode::FORBIDDEN),
        (Some(api_key.secret.as_str()), StatusCode::FORBIDDEN),
        (Some(root.secret.as_str()), StatusCode::OK),
    ] {
        let mut request = Request::builder().uri("/api/option/exchange-rate?currency=cny");
        if let Some(secret) = secret {
            request = request.header(header::AUTHORIZATION, format!("Bearer {secret}"));
        }
        let result = app
            .clone()
            .oneshot(request.body(Body::empty()).unwrap())
            .await
            .unwrap();
        assert_eq!(result.status(), expected);
        assert_eq!(result.headers()[header::CACHE_CONTROL], "no-store");
        if expected == StatusCode::OK {
            let body: serde_json::Value =
                serde_json::from_slice(&to_bytes(result.into_body(), 65536).await.unwrap())
                    .unwrap();
            assert_eq!(body["data"]["rate"], 7.25);
        }
    }
    assert_eq!(fixture.queries.lock().unwrap().len(), 1);
    for (path, expected) in [
        (
            "/api/option/exchange-rate?currency=CNY&url=http://localhost",
            StatusCode::BAD_REQUEST,
        ),
        (
            "/api/option/exchange-rate?currency=CNY",
            StatusCode::BAD_GATEWAY,
        ),
    ] {
        *fixture.status.lock().unwrap() = StatusCode::BAD_GATEWAY;
        let result = app
            .clone()
            .oneshot(
                Request::builder()
                    .uri(path)
                    .header(header::AUTHORIZATION, format!("Bearer {}", root.secret))
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(result.status(), expected);
    }
    let saved: i64 =
        sqlx::query_scalar("SELECT count(*) FROM core_console.options WHERE key='USDExchangeRate'")
            .fetch_one(&pool)
            .await
            .unwrap();
    assert_eq!(saved, 0);
    sqlx::query("UPDATE core_identity.credentials SET revoked_at=clock_timestamp() WHERE id=$1")
        .bind(root.id)
        .execute(&pool)
        .await
        .unwrap();
    let result = app
        .oneshot(
            Request::builder()
                .uri("/api/option/exchange-rate?currency=USD")
                .header(header::AUTHORIZATION, format!("Bearer {}", root.secret))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(result.status(), StatusCode::UNAUTHORIZED);
    task.abort();
}

//! Native HTTP settings against a disposable PostgreSQL database and restricted
//! runtime role. No live exchange-rate service or payment provider is contacted.
use axum::{
    Router,
    body::{Body, to_bytes},
    http::{Request, StatusCode, header},
};
use lmm_core::{
    billing::{
        Credits,
        ledger::{Action, Ledger, Request as LedgerRequest},
    },
    core_schema,
    http::{self, Capabilities},
    identity::IdentityStore,
    options_http::{self, OptionsHttp, RuntimeOptions},
    runtime_database::{self, RequiredCapabilities},
};
use serde_json::{Value, json};
use sqlx::PgPool;
use tower::ServiceExt;

#[path = "common/runtime.rs"]
mod runtime;

fn settings(pool: &PgPool) -> Router {
    options_http::router(OptionsHttp::new(
        pool.clone(),
        IdentityStore::from_pool(pool.clone()),
        RuntimeOptions {
            server_address: "https://console.example.test".into(),
            registration_enabled: false,
            password_register_enabled: false,
            oauth_register_enabled: false,
            registration_disabled_methods: vec![],
        },
    ))
}

async fn call(
    app: &Router,
    method: &str,
    path: &str,
    secret: &str,
    body: Value,
) -> (StatusCode, Value) {
    let result = app
        .clone()
        .oneshot(
            Request::builder()
                .method(method)
                .uri(path)
                .header(header::AUTHORIZATION, format!("Bearer {secret}"))
                .header(header::CONTENT_TYPE, "application/json")
                .body(Body::from(body.to_string()))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(result.headers()[header::CACHE_CONTROL], "no-store");
    let status = result.status();
    let body = to_bytes(result.into_body(), 1 << 20).await.unwrap();
    (status, serde_json::from_slice(&body).unwrap())
}

async fn stored(pool: &PgPool) -> Option<(String, i64)> {
    sqlx::query_as("SELECT value,revision FROM core_console.options WHERE key='USDExchangeRate'")
        .fetch_optional(pool)
        .await
        .unwrap()
}

#[sqlx::test(migrations = false)]
async fn display_rate_persists_with_root_authority_atomic_audit_and_unchanged_ledger(
    owner: PgPool,
) {
    core_schema::init(&owner).await.unwrap();
    let identity = IdentityStore::from_pool(owner.clone());
    let root = identity.bootstrap_user(1, 6).await.unwrap();
    let admin = identity.bootstrap_user(2, 5).await.unwrap();
    let account = identity
        .authorize(&root.secret)
        .await
        .unwrap()
        .owner_account_id;
    let ledger = Ledger::from_pool(owner.clone());
    ledger.open_wallet(account).await.unwrap();
    ledger
        .execute(&LedgerRequest {
            scope: "display-exchange-rate-test".into(),
            operation_key: "synthetic-opening".into(),
            actor_user_id: Some(1),
            reason: "Synthetic historical balance in a disposable database".into(),
            action: Action::Credit {
                account_id: account,
                amount_units: Credits::try_from(1250000).unwrap(),
                payment_reference: "synthetic:display-exchange-rate".into(),
            },
        })
        .await
        .unwrap();
    let balance_before = ledger.balance(account).await.unwrap();
    let journals_before: Vec<String> = sqlx::query_scalar(
        "SELECT row_to_json(j)::text FROM core_billing.ledger_journals j ORDER BY id",
    )
    .fetch_all(&owner)
    .await
    .unwrap();
    let runtime = runtime::RuntimeRole::create(&owner).await;
    runtime.optional("core_console").await;
    runtime_database::check(
        &runtime.pool,
        RequiredCapabilities {
            console: true,
            ..Default::default()
        },
    )
    .await
    .unwrap();
    let app = settings(&runtime.pool);

    let (_, initial) = call(&app, "GET", "/api/option/", &root.secret, json!(null)).await;
    assert!(
        initial["data"]
            .as_array()
            .unwrap()
            .iter()
            .any(|option| option["key"] == "USDExchangeRate" && option["value"] == "")
    );
    assert!(
        initial["capabilities"]["writable_keys"]
            .as_array()
            .unwrap()
            .contains(&json!("USDExchangeRate"))
    );
    assert_eq!(
        call(
            &app,
            "PUT",
            "/api/option/",
            &admin.secret,
            json!({"key":"USDExchangeRate","value":"7.2"})
        )
        .await
        .0,
        StatusCode::FORBIDDEN
    );
    for invalid in [
        "", "0", "-1", "NaN", "Infinity", "1e309", "1e10", "0.00009", " 7", "+7", "07",
    ] {
        assert_eq!(
            call(
                &app,
                "POST",
                "/api/option/bulk",
                &root.secret,
                json!({"values":{"USDExchangeRate":invalid,"Notice":"must roll back"}})
            )
            .await
            .0,
            StatusCode::BAD_REQUEST,
            "{invalid}"
        );
    }
    assert_eq!(
        call(
            &app,
            "POST",
            "/api/option/bulk",
            &root.secret,
            json!({"values":{"USDExchangeRate":"7.2","QuotaPerUnit":"1"}})
        )
        .await
        .0,
        StatusCode::CONFLICT
    );
    assert!(stored(&owner).await.is_none());
    assert_eq!(
        call(
            &app,
            "POST",
            "/api/option/validate",
            &root.secret,
            json!({"values":{"USDExchangeRate":"7.2"},"expected_values":{"USDExchangeRate":""}})
        )
        .await
        .0,
        StatusCode::OK
    );
    assert!(stored(&owner).await.is_none());
    let first = call(
        &app,
        "POST",
        "/api/option/bulk",
        &root.secret,
        json!({"values":{"USDExchangeRate":"7.2"},"expected_values":{"USDExchangeRate":""}}),
    );
    let second = call(
        &app,
        "POST",
        "/api/option/bulk",
        &root.secret,
        json!({"values":{"USDExchangeRate":"7.3"},"expected_values":{"USDExchangeRate":""}}),
    );
    let (first, second) = tokio::join!(first, second);
    let mut statuses = [first.0.as_u16(), second.0.as_u16()];
    statuses.sort();
    assert_eq!(statuses, [200, 409]);
    let saved = stored(&owner).await.unwrap();
    assert_eq!(saved.1, 1);
    assert_eq!(
        call(
            &app,
            "PUT",
            "/api/option/",
            &root.secret,
            json!({"key":"USDExchangeRate","value":saved.0})
        )
        .await
        .0,
        StatusCode::OK
    );
    assert_eq!(stored(&owner).await.as_ref(), Some(&saved));

    let reloaded = settings(&runtime.pool);
    let (_, options) = call(&reloaded, "GET", "/api/option/", &root.secret, json!(null)).await;
    assert!(
        options["data"]
            .as_array()
            .unwrap()
            .iter()
            .any(|option| option["key"] == "USDExchangeRate" && option["value"] == saved.0)
    );
    let public = http::configured_router(
        runtime.pool.clone(),
        Capabilities {
            console: true,
            ..Default::default()
        },
    );
    let (status, status_body) = call(&public, "GET", "/api/status", "", json!(null)).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(
        status_body["data"]["usd_exchange_rate"],
        saved.0.parse::<f64>().unwrap()
    );
    assert_eq!(status_body["data"]["cny_per_usd"], saved.0);
    let audit: Vec<Value> = sqlx::query_scalar(
        "SELECT target FROM core_identity.audit WHERE action='console.options.update'",
    )
    .fetch_all(&owner)
    .await
    .unwrap();
    assert_eq!(audit.len(), 1);
    assert_eq!(audit[0]["changes"][0]["key"], "USDExchangeRate");
    assert_eq!(audit[0]["changes"][0]["after"], json!([saved.0, 1]));

    for statement in [
        "UPDATE core_console.options SET value='8' WHERE key='USDExchangeRate'",
        "INSERT INTO core_console.options(key,value) VALUES ('OAuthSecret','denied')",
        "DELETE FROM core_console.options WHERE key='USDExchangeRate'",
        "CREATE TABLE core_console.denied(x int)",
    ] {
        let error = sqlx::query(statement)
            .execute(&runtime.pool)
            .await
            .unwrap_err();
        assert_eq!(
            error.as_database_error().unwrap().code().as_deref(),
            Some("42501")
        );
    }
    for statement in [
        "SELECT core_console.write_display_exchange_rate('0')",
        "SELECT core_console.write_display_exchange_rate('NaN')",
        "SELECT core_console.write_public_option('USDExchangeRate','8')",
    ] {
        let error = sqlx::query(statement)
            .execute(&runtime.pool)
            .await
            .unwrap_err();
        assert_eq!(
            error.as_database_error().unwrap().code().as_deref(),
            Some("22023")
        );
    }
    sqlx::query("ALTER TABLE core_identity.audit ADD CONSTRAINT reject_rate_audit CHECK (action <> 'console.options.update') NOT VALID")
        .execute(&owner).await.unwrap();
    assert_eq!(
        call(
            &app,
            "PUT",
            "/api/option/",
            &root.secret,
            json!({"key":"USDExchangeRate","value":"8"})
        )
        .await
        .0,
        StatusCode::SERVICE_UNAVAILABLE
    );
    assert_eq!(stored(&owner).await.as_ref(), Some(&saved));
    assert_eq!(ledger.balance(account).await.unwrap(), balance_before);
    let journals_after: Vec<String> = sqlx::query_scalar(
        "SELECT row_to_json(j)::text FROM core_billing.ledger_journals j ORDER BY id",
    )
    .fetch_all(&owner)
    .await
    .unwrap();
    assert_eq!(journals_after, journals_before);
    assert_eq!(lmm_core::billing::CREDITS_PER_USD, 500000);

    sqlx::query("UPDATE core_console.options SET value='NaN' WHERE key='USDExchangeRate'")
        .execute(&owner)
        .await
        .unwrap();
    assert_eq!(
        call(&public, "GET", "/api/status", "", json!(null)).await.0,
        StatusCode::SERVICE_UNAVAILABLE
    );
    assert_eq!(
        call(&app, "GET", "/api/option/", &root.secret, json!(null))
            .await
            .0,
        StatusCode::SERVICE_UNAVAILABLE
    );
    runtime.cleanup().await;
}

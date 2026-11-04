use axum::{
    body::{Body, to_bytes},
    http::{Request, StatusCode},
};
use lmm_core::{
    billing::{
        Credits,
        ledger::{Action, Ledger, Request as LedgerRequest},
    },
    core_schema,
    directory_ads::{self, Error, Store},
    identity::{BrowserAuth, BrowserRegistration, BrowserSessionMetadata, IdentityStore},
    internal_rpc::pb,
};
use sqlx::PgPool;
use tower::ServiceExt;

async fn advertising_runtime(owner: &PgPool) -> PgPool {
    use lmm_core::runtime_database::{self, RequiredCapabilities, RuntimeDatabaseError};
    use sqlx::postgres::PgPoolOptions;
    let role = format!("ad_runtime_{}", std::process::id());
    let options = owner.connect_options();
    let database = options.get_database().unwrap();
    let source = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..");
    let apply = |script: &str, flag: &str| {
        let output = std::process::Command::new("psql")
            .args([
                "-X",
                "-q",
                "-v",
                "ON_ERROR_STOP=1",
                "-v",
                &format!("runtime_role={role}"),
                "-v",
                flag,
                "-f",
            ])
            .arg(source.join("deployment/postgres").join(script))
            .env("PGDATABASE", database)
            .output()
            .unwrap();
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
    };
    apply("grant-core-runtime.sql", "enable_directory_ads=true");
    let runtime = PgPoolOptions::new()
        .max_connections(3)
        .connect_with(options.as_ref().clone().username(&role))
        .await
        .unwrap();
    let required = || RequiredCapabilities {
        directory_ads: true,
        ..Default::default()
    };
    runtime_database::check(&runtime, required()).await.unwrap();
    for (table, columns) in [
        (
            "quotes",
            &[
                "confirmation_digest",
                "confirmation_sid",
                "confirmed_sid",
                "confirmed_at",
            ][..],
        ),
        ("refunds", &["receipt", "journal_id"][..]),
    ] {
        for column in columns {
            sqlx::query(&format!(
                "REVOKE UPDATE({column}) ON core_directory_ads.{table} FROM {role}"
            ))
            .execute(owner)
            .await
            .unwrap();
            assert_eq!(
                runtime_database::check(&runtime, required()).await,
                Err(RuntimeDatabaseError::MissingPrivileges)
            );
            sqlx::query(&format!(
                "GRANT UPDATE({column}) ON core_directory_ads.{table} TO {role}"
            ))
            .execute(owner)
            .await
            .unwrap();
        }
    }
    for privilege in ["DELETE", "UPDATE"] {
        sqlx::query(&format!(
            "GRANT {privilege} ON core_directory_ads.payments TO {role}"
        ))
        .execute(owner)
        .await
        .unwrap();
        assert_eq!(
            runtime_database::check(&runtime, required()).await,
            Err(RuntimeDatabaseError::UnsafePrivileges)
        );
        sqlx::query(&format!(
            "REVOKE {privilege} ON core_directory_ads.payments FROM {role}"
        ))
        .execute(owner)
        .await
        .unwrap();
    }
    apply("revoke-core-runtime.sql", "confirm_revoke=true");
    assert_eq!(
        runtime_database::check(&runtime, required()).await,
        Err(RuntimeDatabaseError::MissingPrivileges)
    );
    let remaining: bool = sqlx::query_scalar("SELECT has_any_column_privilege($1,'core_directory_ads.quotes','UPDATE') OR has_any_column_privilege($1,'core_directory_ads.refunds','UPDATE')")
        .bind(&role).fetch_one(owner).await.unwrap();
    assert!(!remaining, "revoke left advertising column grants");
    apply("grant-core-runtime.sql", "enable_directory_ads=true");
    runtime_database::check(&runtime, required()).await.unwrap();
    runtime
}

#[sqlx::test(migrations = false)]
async fn browser_confirmed_charge_refund_and_replays(pool: PgPool) {
    let runtime_role = format!("ad_runtime_{}", std::process::id());
    sqlx::query(&format!("CREATE ROLE {runtime_role} LOGIN NOSUPERUSER NOCREATEDB NOCREATEROLE NOREPLICATION NOBYPASSRLS"))
        .execute(&pool).await.unwrap();
    let owner_role = "lmm_core_owner";
    let database = pool.connect_options().get_database().unwrap().to_owned();
    sqlx::query("DO $$ BEGIN IF NOT EXISTS (SELECT FROM pg_roles WHERE rolname='lmm_core_owner') THEN CREATE ROLE lmm_core_owner NOLOGIN; END IF; END $$")
        .execute(&pool).await.unwrap();
    sqlx::query(&format!(
        "ALTER DATABASE \"{database}\" OWNER TO {owner_role}"
    ))
    .execute(&pool)
    .await
    .unwrap();
    sqlx::query(&format!("ALTER SCHEMA public OWNER TO {owner_role}"))
        .execute(&pool)
        .await
        .unwrap();
    let pool = sqlx::postgres::PgPoolOptions::new()
        .max_connections(3)
        .after_connect(move |connection, _| {
            let statement = format!("SET ROLE {owner_role}");
            Box::pin(async move {
                sqlx::query(&statement).execute(connection).await?;
                Ok(())
            })
        })
        .connect_with(pool.connect_options().as_ref().clone())
        .await
        .unwrap();
    core_schema::init(&pool).await.unwrap();
    let identity = IdentityStore::from_pool(pool.clone()).with_registration_enabled(true);
    let browser = BrowserAuth::new(identity.clone(), &[43; 32]).unwrap();
    browser
        .register(&BrowserRegistration {
            username: "adbuyer",
            password: "synthetic passphrase",
            email: "",
        })
        .await
        .unwrap();
    let session = browser
        .login(
            "adbuyer",
            "synthetic passphrase",
            &BrowserSessionMetadata::default(),
        )
        .await
        .unwrap();
    let token = session.data["access_token"].as_str().unwrap();
    let actor = identity.authorize(token).await.unwrap();
    let ledger = Ledger::from_pool(pool.clone());
    ledger.open_wallet(actor.owner_account_id).await.unwrap();
    ledger
        .execute(&LedgerRequest {
            scope: "ad-test".into(),
            operation_key: "fund".into(),
            actor_user_id: Some(actor.user_id),
            reason: "Synthetic local test credits".into(),
            action: Action::Credit {
                account_id: actor.owner_account_id,
                amount_units: Credits::try_from(1_000_000).unwrap(),
                payment_reference: "synthetic-ad-funding".into(),
            },
        })
        .await
        .unwrap();
    let runtime = advertising_runtime(&pool).await;
    let store = Store::new(runtime);
    let input = pb::PrepareDirectoryAdRequest {
        protocol_major: 1,
        request_id: "synthetic-ad-0001".into(),
        bid_cents: 100,
        content_version: 1,
        name: "Example".into(),
        url: "https://example.com/".into(),
        summary: "Summary".into(),
        description: "Description".into(),
        content_sha256: directory_ads::content_digest(
            "Example",
            "https://example.com/",
            "Summary",
            "Description",
        ),
    };
    let quote = store
        .prepare("community-directory-ads", token, &input)
        .await
        .unwrap();
    let pay = pb::PayDirectoryAdRequest {
        protocol_major: 1,
        request_id: input.request_id.clone(),
        quote_id: quote.quote_id.clone(),
        expected_quota: quote.quota,
        content_sha256: input.content_sha256.clone(),
    };
    assert_eq!(
        store
            .pay("community-directory-ads", token, &pay)
            .await
            .unwrap_err(),
        Error::ConfirmationRequired
    );
    let app =
        directory_ads::http::router(store.clone(), "https://app.example".into(), false).unwrap();
    let cookie = format!("new_api_refresh={}", session.refresh_token);
    let response = app
        .clone()
        .oneshot(
            Request::builder()
                .uri(&quote.confirmation_path)
                .header("cookie", &cookie)
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let body = to_bytes(response.into_body(), 16384).await.unwrap();
    let html = std::str::from_utf8(&body).unwrap();
    let csrf = html
        .split("name=\"csrf\" value=\"")
        .nth(1)
        .unwrap()
        .split('"')
        .next()
        .unwrap();
    let confirm = |origin: &str| {
        Request::builder()
            .method("POST")
            .uri(&quote.confirmation_path)
            .header("cookie", &cookie)
            .header("origin", origin)
            .header("content-type", "application/x-www-form-urlencoded")
            .body(Body::from(format!("csrf={csrf}")))
            .unwrap()
    };
    assert_eq!(
        app.clone()
            .oneshot(confirm("https://wrong.example"))
            .await
            .unwrap()
            .status(),
        StatusCode::FORBIDDEN
    );
    assert_eq!(
        app.oneshot(confirm("https://app.example"))
            .await
            .unwrap()
            .status(),
        StatusCode::OK
    );
    let paid = store
        .pay("community-directory-ads", token, &pay)
        .await
        .unwrap();
    assert_eq!(paid.outcome, "succeeded");
    assert_eq!(paid.charged_quota, 500_000);
    assert_eq!(
        store
            .pay("community-directory-ads", token, &pay)
            .await
            .unwrap(),
        paid
    );
    let refund = pb::RefundDirectoryAdRequest {
        protocol_major: 1,
        request_id: "synthetic-refund-0001".into(),
        payment_id: paid.payment_id.clone(),
    };
    assert_eq!(
        store
            .refund("community-directory-ads", token, &refund)
            .await
            .unwrap_err(),
        Error::Forbidden
    );
    // Owner fixture grants the existing Root role before authorizing a new session.
    sqlx::query("UPDATE core_identity.users SET platform_role='superadmin' WHERE id=$1")
        .bind(actor.user_id)
        .execute(&pool)
        .await
        .unwrap();
    let root = browser
        .login(
            "adbuyer",
            "synthetic passphrase",
            &BrowserSessionMetadata::default(),
        )
        .await
        .unwrap();
    let root_token = root.data["access_token"].as_str().unwrap();
    let refunded = store
        .refund("community-directory-ads", root_token, &refund)
        .await
        .unwrap();
    assert_eq!(refunded.outcome, "succeeded");
    assert_eq!(refunded.refunded_quota, 500_000);
    assert_eq!(
        store
            .refund("community-directory-ads", root_token, &refund)
            .await
            .unwrap(),
        refunded
    );
    let journals: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM core_billing.ledger_journals WHERE scope='core-directory-ads'",
    )
    .fetch_one(&pool)
    .await
    .unwrap();
    assert_eq!(journals, 2);
    let unbalanced:i64=sqlx::query_scalar("SELECT count(*) FROM (SELECT journal_id FROM core_billing.ledger_entries GROUP BY journal_id HAVING sum(delta_units)<>0) s").fetch_one(&pool).await.unwrap();
    assert_eq!(unbalanced, 0);
    let remaining:i64=sqlx::query_scalar("SELECT sum(amount_credits-refunded)::bigint FROM core_charging.external_consumption WHERE domain='directory_ad'").fetch_one(&pool).await.unwrap();
    assert_eq!(remaining, 0);

    let socket = std::env::temp_dir().join(format!("lmm-ad-{}.sock", std::process::id()));
    let listener = tokio::net::UnixListener::bind(&socket).unwrap();
    let service_token = "synthetic-ad-service-credential-000001";
    let rpc = directory_ads::rpc::DirectoryAdsRpc::new(
        store,
        service_token.as_bytes(),
        "community-directory-ads".into(),
        directory_ads::rpc::Permissions {
            pay: true,
            refund: true,
            read: true,
            reconcile: true,
        },
    )
    .unwrap();
    let server = tokio::spawn(async move {
        tonic::transport::Server::builder()
            .add_service(pb::core_directory_ads_server::CoreDirectoryAdsServer::new(
                rpc,
            ))
            .serve_with_incoming(tokio_stream::wrappers::UnixListenerStream::new(listener))
            .await
            .unwrap();
    });
    let socket_child = socket.clone();
    let user = root_token.to_owned();
    let output = tokio::task::spawn_blocking(move || {
        std::process::Command::new("go")
            .args(["test", "./internal/coreclient", "-run", "^TestDirectoryAdsLive$", "-count=1", "-v"])
            .current_dir(std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../extensions"))
            .env("LMM_AD_TEST_SOCKET", socket_child)
            .env("LMM_AD_TEST_SERVICE", service_token)
            .env("LMM_AD_TEST_USER", user)
            .env("LMM_AD_TEST_PAYMENT", serde_json::json!({"protocol_major":1,"request_id":pay.request_id,"quote_id":pay.quote_id,"expected_quota":pay.expected_quota,"content_sha256":pay.content_sha256}).to_string())
            .env("LMM_AD_TEST_REFUND", serde_json::json!({"protocol_major":1,"request_id":refund.request_id,"payment_id":refund.payment_id}).to_string())
            .output().unwrap()
    }).await.unwrap();
    server.abort();
    let _ = server.await;
    std::fs::remove_file(socket).unwrap();
    assert!(
        output.status.success()
            && String::from_utf8_lossy(&output.stdout).contains("--- PASS: TestDirectoryAdsLive"),
        "Go/Core advertising RPC: {}{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
}

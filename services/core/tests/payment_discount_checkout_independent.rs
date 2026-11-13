//! Independent synthetic HTTP -> Go -> Tonic -> real PostgreSQL acceptance.
#![cfg(unix)]
use lmm_core::{
    billing::ledger,
    identity::IdentityStore,
    internal_rpc::RpcServer,
    payments::{
        PaymentsStore,
        discounts::{self, Definition},
        rpc::{PaymentsRpc, Permissions},
    },
};
use serde_json::json;
use sqlx::PgPool;
use std::{fs, os::unix::fs::PermissionsExt, path::PathBuf, process::Command};
use tokio::sync::watch;
#[sqlx::test(migrations = false)]
#[ignore = "requires owned PG18.6 and frozen Go binary"]
async fn real_discount_checkout_http(pool: PgPool) {
    let out = PathBuf::from(std::env::var("DISCOUNT_JOINT_OUTPUT").unwrap());
    let identity = IdentityStore::from_pool(pool.clone());
    identity.init_database().await.unwrap();
    for schema in [
        lmm_core::abuse::SCHEMA,
        ledger::SCHEMA,
        lmm_core::payments::SCHEMA,
        discounts::SCHEMA,
        discounts::MANAGEMENT_SCHEMA,
    ] {
        sqlx::raw_sql(schema).execute(&pool).await.unwrap();
    }
    let owner = identity.bootstrap_user(7191, 6).await.unwrap();
    lmm_core::config::initialize(None, true).unwrap();
    let store = PaymentsStore::new(
        pool.clone(),
        lmm_core::config::configured_payment_channels().unwrap(),
    )
    .unwrap();
    for (code, end) in [(" save20 ", 0), ("OTHER20", 0), ("EXPIRED20", 1)] {
        store
            .save_discount(
                &owner.secret,
                None,
                Definition {
                    code: code.into(),
                    name: "Independent synthetic".into(),
                    discount_percent: 20,
                    min_credit_units: 500000,
                    max_uses: 10,
                    enabled: true,
                    starts_at: 0,
                    ends_at: end,
                },
            )
            .await
            .unwrap();
    }
    let socket = out.join("core.sock");
    let it = "independent_identity_service_credential_0001";
    let pt = "independent_payment_service_credential_00001";
    let rpc = PaymentsRpc::new(
        store,
        pt.as_bytes(),
        Permissions {
            prepare: true,
            read_user: true,
            ..Default::default()
        },
    )
    .unwrap();
    let server = RpcServer::bind(&socket, it.as_bytes(), Some(identity))
        .unwrap()
        .with_payments(rpc);
    let (shutdown, rx) = watch::channel(false);
    let task = tokio::spawn(server.serve(rx));
    let fixture = out.join("fixture.private.json");
    fs::write(&fixture,serde_json::to_vec(&json!({"socket":socket,"identity_token":it,"payment_token":pt,"owner":owner.secret,"module_url_file":std::env::var("DISCOUNT_MODULE_URL_FILE").unwrap(),"module_db":std::env::var("DISCOUNT_MODULE_DB").unwrap()})).unwrap()).unwrap();
    fs::set_permissions(&fixture, fs::Permissions::from_mode(0o600)).unwrap();
    let result = tokio::task::spawn_blocking(move || {
        Command::new(std::env::var("DISCOUNT_GO_BINARY").unwrap())
            .args([
                "-test.run=^TestDiscountCheckoutRealRust$",
                "-test.v",
                "-test.count=1",
                "-test.timeout=60s",
            ])
            .env("DISCOUNT_JOINT_FIXTURE", fixture)
            .output()
            .unwrap()
    })
    .await
    .unwrap();
    shutdown.send(true).unwrap();
    task.await.unwrap().unwrap();
    fs::write(
        out.join("go.private.log"),
        [result.stdout.clone(), result.stderr].concat(),
    )
    .unwrap();
    assert!(
        result.status.success(),
        "Go HTTP acceptance failed; inspect owned private log"
    );
    assert!(
        String::from_utf8_lossy(&result.stdout).contains("--- PASS: TestDiscountCheckoutRealRust")
    );
    let counts:(i64,i64,i64,i64)=sqlx::query_as("SELECT (SELECT count(*) FROM core_payments.intents),(SELECT count(*) FROM core_payments.discount_reservations),(SELECT count(*) FROM core_payments.discount_reservations WHERE consumed),(SELECT count(*) FROM core_billing.ledger_journals)").fetch_one(&pool).await.unwrap();
    assert_eq!(
        counts,
        (1, 1, 0, 0),
        "rejections/retries must not create intent, reserve or money"
    );
    let amounts: (i64, i64) =
        sqlx::query_as("SELECT amount_minor,credit_units FROM core_payments.intents")
            .fetch_one(&pool)
            .await
            .unwrap();
    assert_eq!(amounts, (1109, 1000000));
}

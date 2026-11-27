//! Native Zed, its packaged provider bridge, Core public OAuth and one PostgreSQL wallet.
use serde_json::{Value, json};
use sqlx::PgPool;
use std::{
    io::Write,
    path::Path,
    process::{Command, Stdio},
    sync::atomic::Ordering,
};

#[allow(dead_code)]
#[path = "common/model_fixture.rs"]
mod model_fixture;
#[allow(dead_code)]
#[path = "common/pi_oauth_fixture.rs"]
mod pi;
#[allow(dead_code)]
#[path = "common/runtime.rs"]
mod runtime;

#[sqlx::test(migrations = false)]
#[ignore = "requires the official Linux Zed window and the packaged bridge on a hosted runner"]
async fn zed_inline_assistant_settles_one_ordinary_oauth_request(pool: PgPool) {
    pi::with_cli_host_fixture(pool, pi::CliHost::Zed, |f| async move {
        let before = f.ledger.balance(f.account).await.unwrap();
        let profile = json!({
            "coreBase": f.base, "issuer": f.issuer.profile().issuer(),
            "refresh": f.refresh, "cap": before.available_units,
        });
        let output = tokio::task::spawn_blocking(move || {
            let script =
                Path::new(env!("CARGO_MANIFEST_DIR")).join("../../scripts/zed-core-host/host.mjs");
            let mut child = Command::new("node")
                .arg(script)
                .stdin(Stdio::piped())
                .stdout(Stdio::piped())
                .spawn()
                .unwrap();
            child
                .stdin
                .take()
                .unwrap()
                .write_all(&serde_json::to_vec(&profile).unwrap())
                .unwrap();
            child.wait_with_output().unwrap()
        })
        .await
        .unwrap();
        assert!(output.status.success(), "native Zed driver failed");
        let mut proof: Value = serde_json::from_slice(&output.stdout).unwrap();
        let request = proof["request_id"].as_str().unwrap();
        assert_eq!(f.provider.hits.load(Ordering::SeqCst), 1);
        let row: (String, i64, i64, i64, i64, i64) = sqlx::query_as(
            "SELECT id, input_tokens, cached_tokens, output_tokens, settled, refunded
             FROM core_charging.requests WHERE oauth_client_id='lmm-zed' AND payer_account_id=$1
             AND key_id IS NULL AND state='settled'",
        )
        .bind(f.account)
        .fetch_one(&f.owner)
        .await
        .unwrap();
        assert_eq!(row, (request.to_owned(), 10, 2, 20, 49, 0));
        let captures: (i64, i64) = sqlx::query_as(
            "SELECT count(*), coalesce(sum(captured),0)::bigint FROM core_charging.ledger_holds
             WHERE request_id=$1 AND closed AND capture_journal_id IS NOT NULL",
        )
        .bind(request)
        .fetch_one(&f.owner)
        .await
        .unwrap();
        assert_eq!(captures, (1, 49));
        let after = f.ledger.balance(f.account).await.unwrap();
        assert_eq!(after.available_units, before.available_units - 49);
        assert_eq!(after.reserved_units, 0);
        proof["database"] = json!({
            "input_tokens":10,"cached_tokens":2,"output_tokens":20,"captured":49,
            "capture_count":1,"reserved_after":0,"wallet_before":before.available_units,
            "wallet_after":after.available_units,
        });
        std::fs::write(
            std::env::var("ZED_HOST_RECEIPT").unwrap(),
            serde_json::to_vec_pretty(&proof).unwrap(),
        )
        .unwrap();
    })
    .await;
}

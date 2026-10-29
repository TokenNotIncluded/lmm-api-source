//! Actual two-cluster history tests, driven by scripts/test-upgrade-history.py.
use lmm_core::upgrade_history::{Binding, Command, History, HistoryError, Phase};
use serde_json::json;
use sqlx::{Connection, PgConnection};
use std::{env, fs, process::Command as Process};

fn url(role: &str) -> String {
    format!(
        "postgresql://{role}@localhost/authority?host={}&port=55831",
        env::var("HISTORY_SOCKET").unwrap()
    )
}
fn binding() -> Binding {
    Binding {
        site_id: "isolated-site-0001".into(),
        operation_id: "original-upgrade-0001".into(),
        request_sha256: "a".repeat(64),
        candidate_sha256: "b".repeat(64),
        target_system_identifier: env::var("HISTORY_TARGET_SYSTEM_ID").unwrap(),
        target_database: "postgres".into(),
        target_database_oid: 5,
    }
}
async fn history() -> History {
    History::connect(
        &url("hist_writer"),
        &"1".repeat(64),
        &binding().target_system_identifier,
    )
    .await
    .unwrap()
}

#[tokio::test]
#[ignore = "requires two isolated PG18 clusters and actual Core binary"]
async fn external_history_survives_target_restore_and_consumes_once() {
    let core = env::var("HISTORY_CORE_BIN").unwrap();
    let private = std::path::PathBuf::from(env::var("HISTORY_PRIVATE").unwrap());
    let owner_config = private.join("owner.toml");
    let url_file = private.join("authority-url");
    fs::write(&url_file, url("hist_installer")).unwrap();
    use std::os::unix::fs::PermissionsExt;
    fs::set_permissions(&url_file, fs::Permissions::from_mode(0o600)).unwrap();
    fs::write(
        &owner_config,
        format!(
            "version=1\n[database]\nurl_file = {:?}\n",
            url_file.to_str().unwrap()
        ),
    )
    .unwrap();
    let request = json!({"action":"install","authority_id":"1".repeat(64),"owner":"hist_owner","writer":"hist_writer"});
    let mut child = Process::new(&core)
        .args([
            "--config",
            owner_config.to_str().unwrap(),
            "upgrade-history",
        ])
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .spawn()
        .unwrap();
    use std::io::Write;
    child
        .stdin
        .take()
        .unwrap()
        .write_all(request.to_string().as_bytes())
        .unwrap();
    let output = child.wait_with_output().unwrap();
    assert!(
        output.status.success(),
        "actual Core authority install: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    let report: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(report["result"]["status"], "installed");
    assert_eq!(report["site_open_allowed"], false);
    assert_eq!(report["retry_allowed"], false);
    let mut h = history().await;
    let b = binding();
    assert_eq!(h.snapshot(&b).await.unwrap(), None);
    let closing = h.reserve(&b).await.unwrap();
    assert_eq!(closing.sequence, 1);
    assert_eq!(closing.phase, Phase::Closing);
    assert_eq!(h.reserve(&b).await.unwrap(), closing);
    let mut other = b.clone();
    other.operation_id = "different-upgrade-0002".into();
    assert!(matches!(
        h.reserve(&other).await,
        Err(HistoryError::Conflict)
    ));
    let held = h.held(&b, &"c".repeat(64)).await.unwrap();
    assert_eq!(held.sequence, 2);
    assert_eq!(held.phase, Phase::Held);
    assert!(matches!(
        h.held(&b, &"c".repeat(64)).await,
        Err(HistoryError::Conflict)
    ));
    let mut h1 = history().await;
    let mut h2 = history().await;
    let (a, z) = tokio::join!(h1.start(&b), h2.start(&b));
    assert_eq!([&a, &z].iter().filter(|r| r.is_ok()).count(), 1);
    assert_eq!(
        [&a, &z]
            .iter()
            .filter(|r| matches!(r, Err(HistoryError::Conflict)))
            .count(),
        1
    );
    drop(h1);
    drop(h2);
    assert_eq!(h.snapshot(&b).await.unwrap().unwrap().phase, Phase::Started);
    // A real target commit is deliberately lost by restoring the pre-operation
    // physical cluster. Authority is a different running postmaster/PGDATA.
    let socket = env::var("HISTORY_TARGET_SOCKET").unwrap();
    let target_url = format!("postgresql://postgres@localhost/postgres?host={socket}&port=55832");
    let mut target = PgConnection::connect(&target_url).await.unwrap();
    let target_id: String =
        sqlx::query_scalar("SELECT system_identifier::text FROM pg_control_system()")
            .fetch_one(&mut target)
            .await
            .unwrap();
    assert_eq!(target_id, b.target_system_identifier);
    sqlx::query("INSERT INTO target_receipt VALUES(1)")
        .execute(&mut target)
        .await
        .unwrap();
    target.close().await.unwrap();
    let restored = Process::new("python3")
        .arg(env::var("HISTORY_RESTORE_SCRIPT").unwrap())
        .arg("restore-target")
        .env("HISTORY_PRIVATE", &private)
        .output()
        .unwrap();
    assert!(
        restored.status.success(),
        "physical restore: {}",
        String::from_utf8_lossy(&restored.stderr)
    );
    let mut target = PgConnection::connect(&target_url).await.unwrap();
    let restored_id: String =
        sqlx::query_scalar("SELECT system_identifier::text FROM pg_control_system()")
            .fetch_one(&mut target)
            .await
            .unwrap();
    assert_eq!(target_id, restored_id);
    let receipt_count: i64 = sqlx::query_scalar("SELECT count(*) FROM target_receipt")
        .fetch_one(&mut target)
        .await
        .unwrap();
    assert_eq!(receipt_count, 0);
    target.close().await.unwrap();
    assert_eq!(h.reserve(&b).await.unwrap().phase, Phase::Started);
    assert!(matches!(h.start(&b).await, Err(HistoryError::Conflict)));
    let committed = h.committed(&b, &"d".repeat(64)).await.unwrap();
    assert_eq!(committed.sequence, 4);
    assert_eq!(committed.phase, Phase::Committed);
    assert_eq!(committed.receipt_sha256, Some("d".repeat(64)));
    assert!(matches!(
        h.committed(&b, &"d".repeat(64)).await,
        Err(HistoryError::Conflict)
    ));
    drop(h);
    let mut h = history().await;
    assert_eq!(h.snapshot(&b).await.unwrap(), Some(committed.clone()));
    let authority_id = env::var("HISTORY_AUTHORITY_SYSTEM_ID").unwrap();
    assert!(matches!(
        History::connect(&url("hist_writer"), &"1".repeat(64), &authority_id).await,
        Err(HistoryError::Identity)
    ));
    assert!(matches!(
        History::connect(
            &url("hist_writer"),
            &"2".repeat(64),
            &b.target_system_identifier
        )
        .await,
        Err(HistoryError::Identity)
    ));
    other = b.clone();
    other.target_system_identifier = authority_id;
    assert!(matches!(
        h.reserve(&other).await,
        Err(HistoryError::Identity)
    ));
    let mut runtime = PgConnection::connect(&url("hist_runtime")).await.unwrap();
    for sql in [
        "SELECT lmm_upgrade_history.identity()",
        "UPDATE lmm_upgrade_history.operations SET sequence=1",
        "DELETE FROM lmm_upgrade_history.operations",
        "TRUNCATE lmm_upgrade_history.operations",
    ] {
        let error = sqlx::query(sql).execute(&mut runtime).await.unwrap_err();
        assert_eq!(
            error.as_database_error().unwrap().code().as_deref(),
            Some("42501")
        );
    }
    let mut writer = PgConnection::connect(&url("hist_writer")).await.unwrap();
    for sql in [
        "SELECT * FROM lmm_upgrade_history.operations",
        "UPDATE lmm_upgrade_history.operations SET sequence=1",
        "TRUNCATE lmm_upgrade_history.operations",
        "SET ROLE hist_owner",
    ] {
        let error = sqlx::query(sql).execute(&mut writer).await.unwrap_err();
        assert_eq!(
            error.as_database_error().unwrap().code().as_deref(),
            Some("42501")
        );
    }
    let inspect = Command::Inspect {
        authority_id: "1".repeat(64),
        binding: b.clone(),
    }
    .run(&url("hist_writer"))
    .await
    .unwrap();
    assert_eq!(inspect["result"]["operation"]["phase"], "committed");
    assert_eq!(inspect["site_open_allowed"], false);
    assert_eq!(inspect["retry_allowed"], false);
    let mut lost = b.clone();
    lost.site_id = "lost-ack-site-0001".into();
    lost.operation_id = "lost-ack-operation-0001".into();
    h.reserve(&lost).await.unwrap();
    h.held(&lost, &"e".repeat(64)).await.unwrap();
    let proxy_url = format!(
        "postgresql://hist_writer@127.0.0.1:{}/authority?sslmode=disable",
        env::var("HISTORY_PROXY_PORT").unwrap()
    );
    let mut faulted = History::connect(&proxy_url, &"1".repeat(64), &b.target_system_identifier)
        .await
        .unwrap();
    assert!(matches!(
        faulted.start(&lost).await,
        Err(HistoryError::CommitUnknown)
    ));
    drop(faulted);
    let observed = h.snapshot(&lost).await.unwrap().unwrap();
    assert_eq!(observed.sequence, 3);
    assert_eq!(observed.phase, Phase::Started);
    assert!(matches!(h.start(&lost).await, Err(HistoryError::Conflict)));
    let bad = Command::Install {
        authority_id: "1".repeat(64),
        owner: "hist_owner".into(),
        writer: "hist_writer".into(),
    }
    .run(&url("hist_installer"))
    .await;
    assert!(bad.is_err());
    assert_eq!(h.snapshot(&b).await.unwrap(), Some(committed));
}

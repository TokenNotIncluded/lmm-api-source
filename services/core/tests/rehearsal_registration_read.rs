//! Opt-in read-only acceptance against an explicitly restored local rehearsal.
use lmm_core::community_read::{CommunityRead, UserPageRequest, UsersInfoRequest};
use sqlx::{
    Row,
    postgres::{PgConnectOptions, PgPoolOptions},
};
use std::{
    collections::BTreeMap,
    path::{Path, PathBuf},
};

async fn rehearsal_pool() -> sqlx::PgPool {
    let dsn = std::env::var("LMM_CORE_REHEARSAL_READ_DATABASE_URL")
        .expect("explicit rehearsal DSN required");
    let options: PgConnectOptions = dsn.parse().expect("invalid rehearsal DSN");
    let socket = options
        .get_socket()
        .map_or_else(|| Path::new(options.get_host()), PathBuf::as_path);
    let backups = PathBuf::from(std::env::var_os("HOME").expect("home required"))
        .join("Backups")
        .canonicalize()
        .unwrap();
    assert!(
        socket.canonicalize().unwrap().starts_with(backups),
        "only a local Backups socket is allowed"
    );
    let pool = PgPoolOptions::new()
        .max_connections(1)
        .after_connect(|c, _| {
            Box::pin(async move {
                sqlx::query("SET default_transaction_read_only=on")
                    .execute(c)
                    .await?;
                Ok(())
            })
        })
        .connect_with(options)
        .await
        .unwrap();
    let local: bool = sqlx::query_scalar("SELECT inet_server_addr() IS NULL AND current_database() LIKE 'lmm_full_audit_%' AND current_setting('listen_addresses')='' AND current_setting('default_transaction_read_only')='on'")
        .fetch_one(&pool).await.unwrap();
    assert!(
        local,
        "requires an isolated read-only local rehearsal connection"
    );
    pool
}

#[tokio::test]
#[ignore = "requires an explicitly selected local restored rehearsal database"]
async fn restored_registration_times_survive_all_import_receipts() {
    let pool = rehearsal_pool().await;
    let rows = sqlx::query("SELECT e.key::bigint AS id,e.value::bigint AS registered FROM core_migration.import_runs i CROSS JOIN LATERAL jsonb_each_text(i.manifest->'registration_times') e WHERE i.manifest->>'scope'='legacy-cutover-identity.v1'")
        .fetch_all(&pool).await.unwrap();
    let expected: BTreeMap<i64, Option<i64>> = rows
        .into_iter()
        .map(|r| {
            let time: i64 = r.get("registered");
            (r.get("id"), (time > 0).then_some(time))
        })
        .collect();
    assert!(
        !expected.is_empty(),
        "source-verified registration receipt required"
    );
    let read = CommunityRead::new(pool.clone()).await.unwrap();
    let mut actual = BTreeMap::new();
    let mut cursor = 0;
    let mut batch_mismatches = 0;
    loop {
        let page = read
            .user_page(&UserPageRequest {
                after_id: cursor,
                limit: 100,
            })
            .await
            .unwrap();
        let ids: Vec<i64> = page.users.iter().map(|u| u.user_id).collect();
        let requested = ids.len();
        let details = read
            .users(&UsersInfoRequest { user_ids: ids })
            .await
            .unwrap();
        assert_eq!(
            details.len(),
            requested,
            "all requested user details must be returned"
        );
        batch_mismatches += details
            .iter()
            .filter(|u| expected.get(&u.user_id) != Some(&u.registered_at))
            .count();
        for user in page.users {
            actual.insert(user.user_id, user.registered_at);
        }
        if !page.has_more {
            break;
        }
        assert!(page.next_id > cursor, "pagination must advance");
        cursor = page.next_id;
    }
    assert_eq!(
        actual.len(),
        expected.len(),
        "all imported users must be enumerated"
    );
    let mismatches = expected
        .iter()
        .filter(|(id, time)| actual.get(id) != Some(time))
        .count();
    assert_eq!(
        mismatches, 0,
        "paginated registration times must match the sealed source cohort"
    );
    assert_eq!(
        batch_mismatches, 0,
        "batch user details must match the same source cohort"
    );
    pool.close().await;
}

#[tokio::test]
#[ignore = "requires original-source private expected cash facts and local rehearsal"]
async fn restored_cash_history_matches_original_classifications() {
    let path = PathBuf::from(
        std::env::var_os("LMM_CORE_REHEARSAL_CASH_EXPECTED")
            .expect("explicit expected facts required"),
    )
    .canonicalize()
    .unwrap();
    let backups = PathBuf::from(std::env::var_os("HOME").unwrap())
        .join("Backups")
        .canonicalize()
        .unwrap();
    assert!(
        path.starts_with(backups),
        "expected cash facts must stay in private Backups"
    );
    let expected: Vec<lmm_core::community_read::FirstCashPayment> =
        serde_json::from_slice(&std::fs::read(path).unwrap()).unwrap();
    assert!(!expected.is_empty());
    let pool = rehearsal_pool().await;
    let read = CommunityRead::new(pool.clone()).await.unwrap();
    let mut mismatches = 0;
    for batch in expected.chunks(100) {
        let result = read
            .first_cash_payments(&UsersInfoRequest {
                user_ids: batch.iter().map(|u| u.user_id).collect(),
            })
            .await
            .unwrap();
        assert_eq!(result.users.len(), batch.len());
        for (actual, expected) in result.users.iter().zip(batch) {
            mismatches += usize::from(
                actual.user_id != expected.user_id
                    || actual.first_paid_at != expected.first_paid_at
                    || actual.second_paid_at != expected.second_paid_at
                    || actual.history_complete != expected.history_complete
                    || actual.unclassified_orders != expected.unclassified_orders,
            );
        }
    }
    assert_eq!(
        mismatches, 0,
        "source-derived dates, unknowns and coverage must match for every imported user"
    );
    pool.close().await;
}

#[tokio::test]
#[ignore = "requires explicitly restored Core, acquisition and community databases"]
async fn restored_funnel_uses_real_core_and_extension_databases() {
    use std::os::unix::fs::DirBuilderExt;
    let pool = rehearsal_pool().await;
    let read = CommunityRead::new(pool.clone()).await.unwrap();
    let dir = std::env::temp_dir().join(format!("lmm-restored-funnel-{}", std::process::id()));
    std::fs::DirBuilder::new().mode(0o700).create(&dir).unwrap();
    let socket = dir.join("core.sock");
    let listener = tokio::net::UnixListener::bind(&socket).unwrap();
    let router =
        lmm_core::community_read::http::router(read, b"synthetic-community-purpose-token-20261104")
            .unwrap();
    let server = tokio::spawn(async move {
        axum::serve(listener, router).await.unwrap();
    });
    let output = tokio::task::spawn_blocking(move || {
        std::process::Command::new("go")
            .args([
                "test",
                "./internal/modules/acquisition",
                "-run",
                "^TestRestoredFunnelLive$",
                "-count=1",
                "-v",
            ])
            .current_dir(concat!(env!("CARGO_MANIFEST_DIR"), "/../extensions"))
            .env("LMM_TEST_CORE_FUNNEL_SOCKET", socket)
            .output()
            .unwrap()
    })
    .await
    .unwrap();
    server.abort();
    let _ = server.await;
    std::fs::remove_dir_all(dir).unwrap();
    pool.close().await;
    assert!(
        output.status.success()
            && String::from_utf8_lossy(&output.stdout).contains("--- PASS: TestRestoredFunnelLive"),
        "restored Core/Go funnel: {} {}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
}

#[tokio::test]
#[ignore = "requires private source amount expectations and an explicitly restored Core database"]
async fn restored_cash_amounts_match_original_orders() {
    #[derive(serde::Deserialize)]
    struct Expected {
        start: i64,
        end: i64,
        users: Vec<lmm_core::community_read::CashPaymentUser>,
    }
    let path = PathBuf::from(
        std::env::var_os("LMM_CORE_REHEARSAL_CASH_TOTALS_EXPECTED")
            .expect("expected cash amounts required"),
    )
    .canonicalize()
    .unwrap();
    let backups = PathBuf::from(std::env::var_os("HOME").unwrap())
        .join("Backups")
        .canonicalize()
        .unwrap();
    assert!(path.starts_with(backups));
    let expected: Vec<Expected> = serde_json::from_slice(&std::fs::read(path).unwrap()).unwrap();
    assert!(!expected.is_empty());
    let pool = rehearsal_pool().await;
    let read = CommunityRead::new(pool.clone()).await.unwrap();
    let mut compared = 0;
    for period in expected {
        for users in period.users.chunks(100) {
            let actual = read
                .cash_payment_totals(&lmm_core::community_read::CashPeriodRequest {
                    user_ids: users.iter().map(|u| u.user_id).collect(),
                    start: period.start,
                    end: period.end,
                })
                .await
                .unwrap();
            assert_eq!(
                serde_json::to_value(&actual.users).unwrap(),
                serde_json::to_value(users).unwrap(),
                "source cash amounts, refunds, order counts and unknowns must match per account and currency"
            );
            compared += users.len();
        }
    }
    assert!(compared > 0);
    pool.close().await;
}

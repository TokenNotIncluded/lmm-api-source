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

#[tokio::test]
#[ignore = "requires an explicitly selected local restored rehearsal database"]
async fn restored_registration_times_survive_all_import_receipts() {
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

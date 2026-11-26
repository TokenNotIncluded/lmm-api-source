//! Authenticated native read-model regressions against SQLx-owned PostgreSQL.
//! Source only until leased author/independent execution; no production data.
use super::*;
use crate::billing::{
    Credits,
    ledger::{Action, Ledger, Request as LedgerRequest},
};
use crate::identity::{BrowserAuth, BrowserRegistration, BrowserSessionMetadata};
use crate::test_runtime as runtime;
use axum::{
    body::{Body, to_bytes},
    http::{Request, StatusCode},
};
use totp_rs::{Algorithm, Builder, Secret};
use tower::ServiceExt;
#[path = "access_groups_tests.rs"]
mod access_groups_tests;
#[path = "usage_tests.rs"]
mod usage_tests;
const PASSWORD: &str = "synthetic passphrase";
struct Fixture {
    identity: IdentityStore,
    store: AdminUsersStore,
    admin: String,
    root: String,
    role: runtime::RuntimeRole,
}
async fn fresh(pool: &PgPool) -> Fixture {
    crate::core_schema::init(pool).await.unwrap();
    let identity = IdentityStore::from_pool(pool.clone())
        .with_registration_enabled(true)
        .with_mfa_keys("test".into(), vec![("test".into(), vec![41; 32])])
        .unwrap();
    let admin = identity.bootstrap_user(900, 5).await.unwrap().secret;
    let root = identity.bootstrap_user(901, 6).await.unwrap().secret;
    let role = runtime::RuntimeRole::create(pool).await;
    let (session, current): (String, String) =
        sqlx::query_as("SELECT session_user::text,current_user::text")
            .fetch_one(&role.pool)
            .await
            .unwrap();
    let url = reqwest::Url::parse(&role.url).unwrap();
    assert!(
        session == role.name && current == role.name && url.username() == role.name,
        "fixture must use its actual restricted runtime identity"
    );
    let runtime_identity = IdentityStore::from_pool(role.pool.clone())
        .with_mfa_keys("test".into(), vec![("test".into(), vec![41; 32])])
        .unwrap();
    Fixture {
        identity,
        store: AdminUsersStore::new(runtime_identity),
        admin,
        root,
        role,
    }
}
async fn person(f: &Fixture, pool: &PgPool, id: i64, level: i16, name: &str) -> IssuedCredential {
    let value = f.identity.bootstrap_user(id, level).await.unwrap();
    sqlx::query("UPDATE core_identity.user_profiles SET username=$2,display_name=$2,email=$3 WHERE user_id=$1").bind(id).bind(name).bind(format!("{name}@example.test")).execute(pool).await.unwrap();
    value
}
async fn page(f: &Fixture, secret: &str, q: UsersQuery) -> Value {
    serde_json::to_value(f.store.users(secret, &q).await.unwrap()).unwrap()
}
fn search(keyword: &str) -> UsersQuery {
    UsersQuery {
        keyword: keyword.into(),
        ..UsersQuery::default()
    }
}
async fn state(pool: &PgPool) -> Vec<(String, String)> {
    // Hash only; assertion failure never prints credentials, verifiers or mail.
    let names:Vec<(String,String)>=sqlx::query_as("SELECT n.nspname,c.relname FROM pg_class c JOIN pg_namespace n ON n.oid=c.relnamespace WHERE c.relkind='r' AND (n.nspname LIKE 'core_%' OR n.nspname='public') ORDER BY n.nspname,c.relname").fetch_all(pool).await.unwrap();
    let mut values = Vec::new();
    for (schema, table) in names {
        assert!(
            schema
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || b == b'_')
                && table
                    .bytes()
                    .all(|b| b.is_ascii_alphanumeric() || b == b'_')
        );
        let hash:String=sqlx::query_scalar(sqlx::AssertSqlSafe(format!("SELECT encode(sha256(convert_to(COALESCE(jsonb_agg(to_jsonb(t) ORDER BY to_jsonb(t)::text),'[]'::jsonb)::text,'UTF8')),'hex') FROM \"{schema}\".\"{table}\" t"))).fetch_one(pool).await.unwrap();
        values.push((format!("{schema}.{table}"), hash));
    }
    let seq:Vec<(String,String)>=sqlx::query_as("SELECT schemaname||'.'||sequencename,COALESCE(last_value::text,'null') FROM pg_sequences WHERE schemaname LIKE 'core_%' OR schemaname='public' ORDER BY schemaname,sequencename").fetch_all(pool).await.unwrap();
    values.extend(seq);
    values
}
async fn ledger_action(
    ledger: &Ledger,
    key: &str,
    user: i64,
    action: Action,
) -> crate::billing::ledger::Outcome {
    let outcome = ledger
        .execute(&LedgerRequest {
            scope: "admin-users-read-fixture".into(),
            operation_key: key.into(),
            actor_user_id: Some(user),
            reason: "Synthetic native wallet read".into(),
            action,
        })
        .await
        .unwrap();
    assert!(
        matches!(outcome, crate::billing::ledger::Outcome::Posted { .. }),
        "synthetic financial setup must have a real posted receipt"
    );
    outcome
}
async fn fund(f: &Fixture, pool: &PgPool, user: i64) -> (Ledger, i64) {
    let account: i64 =
        sqlx::query_scalar("SELECT personal_account_id FROM core_identity.users WHERE id=$1")
            .bind(user)
            .fetch_one(pool)
            .await
            .unwrap();
    let ledger = Ledger::from_pool(pool.clone());
    ledger.open_wallet(account).await.unwrap();
    ledger_action(
        &ledger,
        "credit",
        user,
        Action::Credit {
            account_id: account,
            amount_units: Credits::try_from(1000).unwrap(),
            payment_reference: "synthetic-admin-users".into(),
        },
    )
    .await;
    assert!(f.identity.authorize(&f.admin).await.is_ok());
    (ledger, account)
}
#[sqlx::test(migrations = false)]
async fn real_admin_and_root_read_but_service_tier_client_role_and_keys_do_not(pool: PgPool) {
    let f = fresh(&pool).await;
    let ordinary = person(&f, &pool, 10, 4, "ordinary").await;
    let a = page(&f, &f.admin, UsersQuery::default()).await;
    let b = page(&f, &f.root, UsersQuery::default()).await;
    assert_eq!(a["total"], 3);
    assert_eq!(a, b);
    assert_eq!(a["items"][0]["role"], 100);
    assert_eq!(a["items"][1]["role"], 10);
    assert!(matches!(
        f.store
            .users(
                &ordinary.secret,
                &UsersQuery {
                    role: Some("superadmin".into()),
                    ..UsersQuery::default()
                }
            )
            .await,
        Err(AdminUsersError::Forbidden)
    ));
    let key = f
        .identity
        .issue_key(
            &f.admin,
            Account {
                kind: AccountKind::Personal,
                id: 900,
            },
            FundingPolicy::new_personal_key(),
            3600,
        )
        .await
        .unwrap();
    assert!(matches!(
        f.store.users(&key.secret, &UsersQuery::default()).await,
        Err(AdminUsersError::Forbidden)
    ));
    let team = f.identity.create_team(&f.admin).await.unwrap();
    let team_key = f
        .identity
        .issue_key(
            &f.admin,
            Account {
                kind: AccountKind::Team,
                id: team,
            },
            FundingPolicy {
                version: 1,
                account_order: Some(vec![Account {
                    kind: AccountKind::Team,
                    id: team,
                }]),
                personal_billing_preference: None,
            },
            3600,
        )
        .await
        .unwrap();
    assert!(matches!(
        f.store
            .users(&team_key.secret, &UsersQuery::default())
            .await,
        Err(AdminUsersError::Forbidden)
    ));
    for token in [
        format!("lmm_at_{}", "a".repeat(43)),
        format!("human_grant_{}", "b".repeat(64)),
    ] {
        assert!(
            matches!(
                f.store.users(&token, &UsersQuery::default()).await,
                Err(AdminUsersError::Unauthorized)
            ),
            "a foreign-store credential cannot become a Core Session"
        );
    }
    f.role.cleanup().await;
}
#[sqlx::test(migrations = false)]
async fn same_read_only_transaction_enforces_proof_and_changes_no_rows_or_sequences(pool: PgPool) {
    let f = fresh(&pool).await;
    person(&f, &pool, 10, 0, "snapshot").await;
    let before = state(&pool).await;
    page(&f, &f.admin, UsersQuery::default()).await;
    f.store
        .groups(&f.admin, &GroupsQuery::default())
        .await
        .unwrap();
    let after = state(&pool).await;
    assert_eq!(before, after);
    let mut tx = f.store.begin().await.unwrap();
    let read_only: String = sqlx::query_scalar("SHOW transaction_read_only")
        .fetch_one(&mut *tx)
        .await
        .unwrap();
    let isolation: String = sqlx::query_scalar("SHOW transaction_isolation")
        .fetch_one(&mut *tx)
        .await
        .unwrap();
    assert_eq!(read_only, "on");
    assert_eq!(isolation, "repeatable read");
    AdminUsersStore::authorize(&mut tx, &f.admin).await.unwrap();
    let error=sqlx::query("INSERT INTO core_identity.audit(actor_user_id,action,target) VALUES(900,'denied-read-write','{}')").execute(&mut *tx).await.unwrap_err();
    assert_eq!(
        error.as_database_error().and_then(|e| e.code()).as_deref(),
        Some("25006")
    );
    tx.rollback().await.unwrap();
    assert_eq!(before, state(&pool).await);
    f.role.cleanup().await;
}
#[sqlx::test(migrations = false)]
async fn revoked_disabled_and_missing_persistent_proof_cannot_read(pool: PgPool) {
    let f = fresh(&pool).await;
    let bare = new_secret("lmms_").unwrap();
    sqlx::query("INSERT INTO core_identity.credentials(digest,kind,user_id,user_version,owner_account_id,expires_at) SELECT $1,'session',id,auth_version,personal_account_id,clock_timestamp()+interval '1 hour' FROM core_identity.users WHERE id=900").bind(digest(&bare).unwrap()).execute(&pool).await.unwrap();
    assert!(
        f.store.users(&bare, &UsersQuery::default()).await.is_err(),
        "naked Session must not acquire assurance from its role"
    );
    sqlx::query("UPDATE core_identity.users SET auth_version=auth_version+1 WHERE id=900")
        .execute(&pool)
        .await
        .unwrap();
    assert!(matches!(
        f.store.users(&f.admin, &UsersQuery::default()).await,
        Err(AdminUsersError::Unauthorized)
    ));
    sqlx::query("UPDATE core_identity.users SET active=FALSE WHERE id=901")
        .execute(&pool)
        .await
        .unwrap();
    assert!(matches!(
        f.store.users(&f.root, &UsersQuery::default()).await,
        Err(AdminUsersError::Unauthorized)
    ));
    f.role.cleanup().await;
}
#[sqlx::test(migrations = false)]
async fn real_revocation_expiry_and_actor_account_disable_cannot_read(pool: PgPool) {
    let f = fresh(&pool).await;
    for (id, field) in [
        (910, "revoked"),
        (911, "expired"),
        (912, "inactive_account"),
    ] {
        let credential = f.identity.bootstrap_user(id, 5).await.unwrap();
        assert!(
            f.store
                .users(&credential.secret, &UsersQuery::default())
                .await
                .is_ok()
        );
        match field {
            "revoked" => {
                sqlx::query("UPDATE core_identity.credentials SET revoked_at=clock_timestamp() WHERE digest=$1").bind(digest(&credential.secret).unwrap()).execute(&pool).await.unwrap();
            }
            "expired" => {
                sqlx::query("UPDATE core_identity.credentials SET expires_at=clock_timestamp()-interval '1 second' WHERE digest=$1").bind(digest(&credential.secret).unwrap()).execute(&pool).await.unwrap();
            }
            "inactive_account" => {
                sqlx::query("UPDATE core_identity.accounts SET active=FALSE WHERE id=(SELECT personal_account_id FROM core_identity.users WHERE id=$1)").bind(id).execute(&pool).await.unwrap();
            }
            _ => unreachable!(),
        }
        let before = state(&pool).await;
        assert!(
            matches!(
                f.store
                    .users(&credential.secret, &UsersQuery::default())
                    .await,
                Err(AdminUsersError::Unauthorized)
            ),
            "current credential and personal account must remain live"
        );
        assert!(
            matches!(
                f.store
                    .groups(&credential.secret, &GroupsQuery::default())
                    .await,
                Err(AdminUsersError::Unauthorized)
            ),
            "group reads require the same current authority"
        );
        assert_eq!(before, state(&pool).await);
    }
    f.role.cleanup().await;
}
#[sqlx::test(migrations = false)]
async fn literal_and_numeric_search_pagination_and_ties_are_stable(pool: PgPool) {
    let f = fresh(&pool).await;
    person(&f, &pool, 10, 0, "Read%Literal").await;
    person(&f, &pool, 11, 1, "Read_Literal").await;
    person(&f, &pool, 12, 2, "ReadOther").await;
    for id in [10, 11, 12] {
        sqlx::query(
            "UPDATE core_identity.user_profiles SET display_name='Same display' WHERE user_id=$1",
        )
        .bind(id)
        .execute(&pool)
        .await
        .unwrap();
    }
    let q = UsersQuery {
        keyword: "read".into(),
        sort_by: Some("display_name".into()),
        sort_order: Some("asc".into()),
        page_size: Some(2),
        ..UsersQuery::default()
    };
    let first = page(&f, &f.admin, q.clone()).await;
    let second = page(&f, &f.admin, UsersQuery { page: Some(2), ..q }).await;
    assert_eq!(first["total"], 3);
    assert_eq!(first["items"][0]["id"], "10");
    assert_eq!(first["items"][1]["id"], "11");
    assert_eq!(second["items"][0]["id"], "12");
    for (needle, id) in [("%", "10"), ("_Literal", "11"), ("12", "12")] {
        let result = page(&f, &f.admin, search(needle)).await;
        assert_eq!(result["total"], 1);
        assert_eq!(result["items"][0]["id"], id);
    }
    let empty = page(&f, &f.admin, search("' OR 1=1 --")).await;
    assert_eq!(empty["total"], 0);
    assert_eq!(empty["items"], json!([]));
    f.role.cleanup().await;
}
#[sqlx::test(migrations = false)]
async fn real_group_membership_count_pagination_and_array_sort_match(pool: PgPool) {
    let f = fresh(&pool).await;
    for id in [10, 11, 12] {
        person(&f, &pool, id, 0, &format!("group-{id}")).await;
    }
    for (user, name, enabled) in [
        (10, "Alpha", true),
        (10, "Beta", true),
        (11, "Alpha", true),
        (11, "disabled", false),
        (12, "alpha", true),
    ] {
        sqlx::query("INSERT INTO core_identity.user_routing_groups(user_id,group_name,enabled,source_reference) VALUES($1,$2,$3,'synthetic-admin-users')").bind(user).bind(name).bind(enabled).execute(&pool).await.unwrap();
    }
    let selected = page(
        &f,
        &f.admin,
        UsersQuery {
            group: Some("Alpha".into()),
            keyword: "group-".into(),
            sort_by: Some("groups".into()),
            sort_order: Some("asc".into()),
            ..UsersQuery::default()
        },
    )
    .await;
    assert_eq!(selected["total"], 2);
    assert_eq!(selected["items"][0]["id"], "11");
    assert_eq!(selected["items"][1]["groups"], json!(["Alpha", "Beta"]));
    assert_eq!(
        page(
            &f,
            &f.admin,
            UsersQuery {
                group: Some("alpha".into()),
                ..UsersQuery::default()
            }
        )
        .await["total"],
        1
    );
    assert_eq!(
        page(
            &f,
            &f.admin,
            UsersQuery {
                group: Some("disabled".into()),
                ..UsersQuery::default()
            }
        )
        .await["total"],
        0
    );
    let a = serde_json::to_value(
        f.store
            .groups(
                &f.admin,
                &GroupsQuery {
                    page: Some(1),
                    page_size: Some(2),
                },
            )
            .await
            .unwrap(),
    )
    .unwrap();
    let b = serde_json::to_value(
        f.store
            .groups(
                &f.admin,
                &GroupsQuery {
                    page: Some(2),
                    page_size: Some(2),
                },
            )
            .await
            .unwrap(),
    )
    .unwrap();
    assert_eq!(a["total"], 3);
    assert_eq!(a["items"], json!(["Alpha", "Beta"]));
    assert_eq!(b["items"], json!(["alpha"]));
    f.role.cleanup().await;
}
#[sqlx::test(migrations = false)]
async fn role_status_and_inactive_account_are_distinct_filters(pool: PgPool) {
    let f = fresh(&pool).await;
    person(&f, &pool, 10, 0, "filter-zero").await;
    person(&f, &pool, 11, 4, "filter-four").await;
    sqlx::query("UPDATE core_identity.users SET active=FALSE WHERE id=11")
        .execute(&pool)
        .await
        .unwrap();
    let zero = page(
        &f,
        &f.admin,
        UsersQuery {
            role: Some("user".into()),
            status: Some("active".into()),
            ..UsersQuery::default()
        },
    )
    .await;
    assert_eq!(zero["total"], 1);
    assert_eq!(zero["items"][0]["id"], "10");
    let disabled = page(
        &f,
        &f.admin,
        UsersQuery {
            status: Some("disabled".into()),
            ..UsersQuery::default()
        },
    )
    .await;
    assert_eq!(disabled["items"][0]["status"], 2);
    assert_eq!(disabled["items"][0]["wallet"]["status"], "unavailable");
    sqlx::query("UPDATE core_identity.accounts SET active=FALSE WHERE id=(SELECT personal_account_id FROM core_identity.users WHERE id=10)").execute(&pool).await.unwrap();
    let row = page(&f, &f.admin, search("filter-zero")).await;
    assert_eq!(row["items"][0]["active"], true);
    assert_eq!(row["items"][0]["status"], 1);
    assert_eq!(row["items"][0]["account_active"], false);
    assert_eq!(row["items"][0]["wallet"]["status"], "unavailable");
    f.role.cleanup().await;
}
#[sqlx::test(migrations = false)]
async fn missing_profile_and_large_identity_are_explicit_and_exact(pool: PgPool) {
    let f = fresh(&pool).await;
    person(&f, &pool, 10, 0, "missing-profile").await;
    sqlx::query("DELETE FROM core_identity.user_profiles WHERE user_id=10")
        .execute(&pool)
        .await
        .unwrap();
    let row = page(&f, &f.admin, search("10")).await;
    assert_eq!(row["items"][0]["profile_available"], false);
    assert_eq!(row["items"][0]["username"], Value::Null);
    assert_eq!(row["items"][0]["email"], Value::Null);
    assert_eq!(row["items"][0]["registered_at_unix_ms"], Value::Null);
    f.identity
        .bootstrap_user(9_007_199_254_740_993, 0)
        .await
        .unwrap();
    let large = page(&f, &f.admin, search("9007199254740993")).await;
    assert_eq!(large["total"], 1);
    assert_eq!(large["items"][0]["id"], "9007199254740993");
    assert_eq!(large["items"][0]["auth_version"], "1");
    f.role.cleanup().await;
}
#[sqlx::test(migrations = false)]
async fn native_audited_zero_is_not_unknown_legacy_missing_wallet(pool: PgPool) {
    let f = fresh(&pool).await;
    person(&f, &pool, 10, 0, "zero-native").await;
    let mut tx = pool.begin().await.unwrap();
    let account = super::super::new_account(&mut tx, "personal")
        .await
        .unwrap();
    let user: i64 = sqlx::query_scalar(
        "INSERT INTO core_identity.users(personal_account_id) VALUES($1) RETURNING id",
    )
    .bind(account)
    .fetch_one(&mut *tx)
    .await
    .unwrap();
    sqlx::query(
        "INSERT INTO core_identity.user_profiles(user_id,username) VALUES($1,'unknown-legacy')",
    )
    .bind(user)
    .execute(&mut *tx)
    .await
    .unwrap();
    tx.commit().await.unwrap();
    let native = page(&f, &f.admin, search("zero-native")).await;
    assert_eq!(native["items"][0]["wallet"]["status"], "available");
    assert_eq!(native["items"][0]["wallet"]["available_units"], "0");
    let unknown = page(&f, &f.admin, search("unknown-legacy")).await;
    assert_eq!(unknown["items"][0]["wallet"]["status"], "unavailable");
    assert!(
        unknown["items"][0]["wallet"]
            .get("available_units")
            .is_none()
    );
    let buckets:i64=sqlx::query_scalar("SELECT count(*) FROM core_billing.ledger_accounts WHERE owner_account_id IN ($1,(SELECT personal_account_id FROM core_identity.users WHERE id=10))").bind(account).fetch_one(&pool).await.unwrap();
    assert_eq!(buckets, 0);
    f.role.cleanup().await;
}
#[sqlx::test(migrations = false)]
async fn real_reserved_and_debt_wallet_reuses_existing_reader_without_spend(pool: PgPool) {
    let f = fresh(&pool).await;
    person(&f, &pool, 10, 0, "money-target").await;
    let (ledger, account) = fund(&f, &pool, 10).await;
    ledger_action(
        &ledger,
        "reserve",
        10,
        Action::Reserve {
            account_id: account,
            amount_units: Credits::try_from(200).unwrap(),
        },
    )
    .await;
    let before = state(&pool).await;
    let row = page(&f, &f.admin, search("money-target")).await;
    assert_eq!(row["items"][0]["wallet"]["available_units"], "800");
    assert_eq!(row["items"][0]["wallet"]["reserved_units"], "200");
    assert_eq!(before, state(&pool).await);
    let credit:i64=sqlx::query_scalar("SELECT id FROM core_billing.ledger_journals WHERE scope='admin-users-read-fixture' AND operation_key='credit'").fetch_one(&pool).await.unwrap();
    ledger_action(
        &ledger,
        "refund",
        10,
        Action::RefundTopup {
            journal_id: credit,
            amount_units: Credits::try_from(900).unwrap(),
            refund_reference: "synthetic-read-refund".into(),
        },
    )
    .await;
    let original = crate::financial_profile::read_wallet(&mut pool.acquire().await.unwrap(), 10)
        .await
        .unwrap();
    let before = state(&pool).await;
    let row = page(&f, &f.admin, search("money-target")).await;
    let wallet = &row["items"][0]["wallet"];
    assert_eq!(wallet["available_units"], original.quota.to_string());
    assert_eq!(
        wallet["wallet_units"],
        original.wallet_spendable_quota.to_string()
    );
    assert_eq!(wallet["debt_units"], "100");
    assert_eq!(wallet["reserved_units"], "200");
    assert_eq!(wallet["money_basis"], "unknown");
    assert_eq!(wallet["money_available"], false);
    assert_eq!(wallet["currency"], Value::Null);
    assert_eq!(wallet["provenance"]["user_id"], "10");
    assert_eq!(
        wallet["provenance"]["personal_account_id"],
        account.to_string()
    );
    assert_eq!(before, state(&pool).await);
    sqlx::query("UPDATE core_identity.users SET active=FALSE WHERE id=10")
        .execute(&pool)
        .await
        .unwrap();
    let held = state(&pool).await;
    let disabled = page(&f, &f.admin, search("money-target")).await;
    assert_eq!(disabled["items"][0]["wallet"]["status"], "unavailable");
    assert_eq!(held, state(&pool).await);
    f.role.cleanup().await;
}
#[sqlx::test(migrations = false)]
async fn original_reader_precision_and_partial_bucket_fail_closed(pool: PgPool) {
    let f = fresh(&pool).await;
    person(&f, &pool, 10, 0, "precision-target").await;
    let account: i64 =
        sqlx::query_scalar("SELECT personal_account_id FROM core_identity.users WHERE id=10")
            .fetch_one(&pool)
            .await
            .unwrap();
    let ledger = Ledger::from_pool(pool.clone());
    ledger.open_wallet(account).await.unwrap();
    ledger_action(
        &ledger,
        "huge",
        10,
        Action::Credit {
            account_id: account,
            amount_units: Credits::try_from(9_007_199_254_740_992).unwrap(),
            payment_reference: "synthetic-read-huge".into(),
        },
    )
    .await;
    let before = state(&pool).await;
    let result = page(&f, &f.admin, search("precision-target")).await;
    assert_eq!(result["items"][0]["wallet"]["status"], "unavailable");
    assert_eq!(before, state(&pool).await);
    person(&f, &pool, 11, 0, "partial-target").await;
    sqlx::query("INSERT INTO core_billing.ledger_accounts(owner_account_id,bucket) SELECT personal_account_id,'wallet' FROM core_identity.users WHERE id=11").execute(&pool).await.unwrap();
    let before = state(&pool).await;
    assert!(matches!(
        f.store.users(&f.admin, &search("partial-target")).await,
        Err(AdminUsersError::Unavailable)
    ));
    assert_eq!(before, state(&pool).await);
    f.role.cleanup().await;
}
#[sqlx::test(migrations = false)]
async fn unsupported_filters_sorts_and_invalid_bounds_never_mutate(pool: PgPool) {
    let f = fresh(&pool).await;
    let before = state(&pool).await;
    for key in ["service_level", "trust_level"] {
        assert!(serde_json::from_value::<UsersQuery>(json!({key:0})).is_err());
    }
    for q in [
        UsersQuery {
            risk_min: Some("0.8".into()),
            ..UsersQuery::default()
        },
        UsersQuery {
            status: Some("deleted".into()),
            ..UsersQuery::default()
        },
    ] {
        assert!(matches!(
            f.store.users(&f.admin, &q).await,
            Err(AdminUsersError::UnsupportedFilter)
        ));
    }
    for sort in ["quota", "used_quota", "group", "risk_score"] {
        assert!(matches!(
            f.store
                .users(
                    &f.admin,
                    &UsersQuery {
                        sort_by: Some(sort.into()),
                        ..UsersQuery::default()
                    }
                )
                .await,
            Err(AdminUsersError::UnsupportedSort)
        ));
    }
    for q in [
        UsersQuery {
            page: Some(0),
            ..UsersQuery::default()
        },
        UsersQuery {
            page_size: Some(101),
            ..UsersQuery::default()
        },
        UsersQuery {
            sort_by: Some("id; DELETE FROM core_identity.users".into()),
            ..UsersQuery::default()
        },
        UsersQuery {
            role: Some("100".into()),
            ..UsersQuery::default()
        },
    ] {
        assert!(matches!(
            f.store.users(&f.admin, &q).await,
            Err(AdminUsersError::Invalid)
        ));
    }
    assert_eq!(before, state(&pool).await);
    let result = page(&f, &f.admin, UsersQuery::default()).await;
    assert_eq!(
        result["capabilities"]["implemented_write_actions"],
        json!(["create"])
    );
    assert!(
        result["capabilities"]["unavailable_projections"]
            .as_array()
            .unwrap()
            .contains(&json!("used_quota"))
    );
    f.role.cleanup().await;
}
#[sqlx::test(migrations = false)]
async fn actual_http_requires_bearer_and_rejects_unknown_duplicate_and_unsupported_queries(
    pool: PgPool,
) {
    let f = fresh(&pool).await;
    // Protocol support is independent of assembly availability; absent/false
    // availability cannot justify a legacy call or users-list authorization.
    for enabled in [false, true] {
        let status_router = crate::http::configured_router(
            f.role.pool.clone(),
            crate::http::Capabilities {
                admin_users_read: enabled,
                ..Default::default()
            },
        );
        let response = status_router
            .oneshot(
                Request::builder()
                    .uri("/api/status")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::OK);
        let bytes = to_bytes(response.into_body(), 16 * 1024).await.unwrap();
        let status: Value = serde_json::from_slice(&bytes).unwrap();
        assert_eq!(status["data"]["admin_users_protocol"], PROTOCOL);
        assert_eq!(status["data"]["capabilities"]["admin_users_read"], enabled);
    }
    let app = router(f.store.clone());
    let request = Request::builder()
        .uri("/core/v1/admin/users")
        .header("cookie", format!("session={}", f.admin))
        .header("x-user-token", &f.admin)
        .header("role", "100")
        .body(Body::empty())
        .unwrap();
    let response = app.clone().oneshot(request).await.unwrap();
    assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
    for (query, status) in [
        ("page=1&page=2", StatusCode::BAD_REQUEST),
        ("target_user_id=900", StatusCode::BAD_REQUEST),
        ("risk_min=0.8", StatusCode::UNPROCESSABLE_ENTITY),
        ("sort_by=quota", StatusCode::UNPROCESSABLE_ENTITY),
        ("page=0", StatusCode::BAD_REQUEST),
    ] {
        let response = app
            .clone()
            .oneshot(
                Request::builder()
                    .uri(format!("/core/v1/admin/users?{query}"))
                    .header("authorization", format!("Bearer {}", f.admin))
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), status);
        assert_eq!(response.headers()["cache-control"], "private, no-store");
        assert_eq!(response.headers()["vary"], "Authorization");
        let body = to_bytes(response.into_body(), 16 * 1024).await.unwrap();
        assert!(
            !String::from_utf8_lossy(&body).contains(&f.admin),
            "errors must not expose credentials"
        );
    }
    let response = app
        .oneshot(
            Request::builder()
                .uri("/core/v1/admin/users")
                .header("authorization", format!("Bearer {}", f.admin))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let body = to_bytes(response.into_body(), 1024 * 1024).await.unwrap();
    assert!(
        !String::from_utf8_lossy(&body).contains(&f.admin),
        "responses must not expose credentials"
    );
    f.role.cleanup().await;
}
#[sqlx::test(migrations = false)]
async fn history_origin_prefers_no_new_privileges_for_admin_reads(pool: PgPool) {
    async fn request(
        app: &axum::Router,
        session: &str,
        uri: &str,
        prefer: bool,
        status: StatusCode,
    ) -> Value {
        let mut req = Request::builder()
            .uri(uri)
            .header("authorization", format!("Bearer {session}"));
        if prefer {
            req = req.header("prefer", r#"include="identity-origin""#);
        }
        let response = app
            .clone()
            .oneshot(req.body(Body::empty()).unwrap())
            .await
            .unwrap();
        assert_eq!(response.status(), status);
        assert_eq!(response.headers()["cache-control"], "private, no-store");
        assert_eq!(
            response.headers()["vary"],
            if prefer {
                "Authorization, Prefer"
            } else {
                "Authorization"
            }
        );
        serde_json::from_slice(&to_bytes(response.into_body(), 64 * 1024).await.unwrap()).unwrap()
    }

    let f = fresh(&pool).await;
    let visitor = person(&f, &pool, 11, 0, "visitor").await;
    f.role.optional("web_identity").await;
    // Owner installs the synthetic supplement and its independent history coverage.
    // The HTTP router always connects as the restricted runtime role.
    sqlx::raw_sql(r#"
        WITH fixture AS (
            SELECT '[{"source_user_id":73,"target_personal_account_id":10074,"target_user_id":10073}]' AS mapping,
                   '{"id":73}' AS source_facts
        ), receipt AS (
            SELECT mapping::jsonb, encode(sha256(convert_to(mapping,'UTF8')),'hex') AS mapping_hash,
                   source_facts::jsonb, encode(sha256(convert_to(source_facts,'UTF8')),'hex') AS facts_hash
            FROM fixture
        )
        INSERT INTO core_migration.import_runs(migration_id,source_archive_sha256,mapping_sha256,manifest)
        SELECT 'synthetic-history-only',repeat('a',64),mapping_hash,jsonb_build_object(
            'format_version',1,'scope','history-only-identity-supplement.v1',
            'mapping',mapping,'mapping_sha256',mapping_hash,
            'source_snapshot',jsonb_build_object('sha256',repeat('a',64),'metadata',
                jsonb_build_object('schema','legacy_source','observed_at',1700000040)),
            'source_user_facts',jsonb_build_array(jsonb_build_object('facts',source_facts,'source_record_sha256',facts_hash)),
            'target',jsonb_build_object(
                'instance',jsonb_build_object('database',current_database(),'database_oid',
                    (SELECT oid::bigint FROM pg_database WHERE datname=current_database())),
                'before_identity_state',jsonb_build_object(
                    'users',jsonb_build_object('max_id',(SELECT max(id) FROM core_identity.users)),
                    'accounts',jsonb_build_object('max_id',(SELECT max(id) FROM core_identity.accounts)))),
            'target_policy','{"accounts_active":false,"users_active":false,"platform_role":"user","auth_version":1,
                              "credentials_imported":false,"wallet_opening_imported":false}'::jsonb)
        FROM receipt;
        INSERT INTO core_identity.accounts(id,kind,active) OVERRIDING SYSTEM VALUE VALUES(10074,'personal',false);
        INSERT INTO core_identity.users(id,personal_account_id,active) VALUES(10073,10074,false);
        INSERT INTO core_identity.user_profiles(user_id,username,password_login_allowed) VALUES(10073,'history_fixture',false);
        WITH source AS (
            INSERT INTO core_history.sources(contract_version,source_archive_sha256,mapping_sha256,source_namespace,observed_at,complete)
            VALUES('core.history-source.v1',repeat('a',64),repeat('b',64),'legacy.logs:legacy_source',1700000040,true)
            RETURNING source_id,source_archive_sha256,mapping_sha256
        )
        INSERT INTO core_history.scope_receipts(source_id,user_id,owner_account_id,source_archive_sha256,mapping_sha256,source_scope_sha256,complete)
        SELECT source_id,10073,10074,source_archive_sha256,mapping_sha256,repeat('c',64),true FROM source;
    "#).execute(&pool).await.unwrap();

    let app = router(f.store.clone());
    let routes = [
        ("/core/v1/admin/users?keyword=10073", "/items/0"),
        ("/core/v1/admin/users/10073", "/user"),
    ];
    for (uri, pointer) in routes {
        let marked = request(&app, &f.admin, uri, true, StatusCode::OK).await;
        let row = marked.pointer(pointer).unwrap();
        assert_eq!(row["id"], "10073");
        assert_eq!(row["identity_origin"], "history_only");
        assert_eq!(row["active"], false);
        assert_eq!(row["account_active"], false);
        assert_eq!(row["auth_version"], "1");
        let baseline = request(&app, &f.admin, uri, false, StatusCode::OK).await;
        let mut expected = row.clone();
        expected.as_object_mut().unwrap().remove("identity_origin");
        assert_eq!(baseline.pointer(pointer), Some(&expected));
        let denied = request(&app, &visitor.secret, uri, true, StatusCode::FORBIDDEN).await;
        assert!(denied.pointer(pointer).is_none());
    }

    // This is the same attested identity. Losing the optional schema grant must
    // leave each normal admin route readable, with the original inactive flags.
    let revoke = format!("REVOKE USAGE ON SCHEMA core_migration FROM {}", f.role.name);
    sqlx::query(sqlx::AssertSqlSafe(revoke))
        .execute(&pool)
        .await
        .unwrap();
    for (uri, pointer) in routes {
        let fallback = request(&app, &f.admin, uri, true, StatusCode::OK).await;
        let row = fallback.pointer(pointer).unwrap();
        assert_eq!(row["id"], "10073");
        assert_eq!(row["active"], false);
        assert_eq!(row["account_active"], false);
        assert_eq!(row["auth_version"], "1");
        assert!(row.get("identity_origin").is_none());
    }
    f.role.cleanup().await;
}

#[sqlx::test(migrations = false)]
async fn one_snapshot_keeps_total_and_rows_consistent_across_committed_insert(pool: PgPool) {
    let f = fresh(&pool).await;
    let mut tx = f.store.begin().await.unwrap();
    AdminUsersStore::authorize(&mut tx, &f.admin).await.unwrap();
    let count: i64 = sqlx::query_scalar("SELECT count(*) FROM core_identity.users")
        .fetch_one(&mut *tx)
        .await
        .unwrap();
    f.identity.bootstrap_user(10, 0).await.unwrap();
    let ids: Vec<i64> = sqlx::query_scalar("SELECT id FROM core_identity.users ORDER BY id")
        .fetch_all(&mut *tx)
        .await
        .unwrap();
    assert_eq!(count, 2);
    assert_eq!(ids, vec![900, 901]);
    tx.commit().await.unwrap();
    assert_eq!(page(&f, &f.admin, UsersQuery::default()).await["total"], 3);
    f.role.cleanup().await;
}

// Reused exact helper bytes from StageA tests/identity_mfa.rs; no new factor protocol.
fn code_at(secret: &str, epoch: u64) -> String {
    let secret = Secret::try_from_base32(secret).unwrap();
    Builder::new()
        .with_algorithm(Algorithm::SHA1)
        .with_digits(6)
        .with_step_duration(30)
        .with_skew(1)
        .with_secret(secret.as_bytes())
        .build()
        .unwrap()
        .generate(epoch)
        .to_string()
}

#[sqlx::test(migrations = false)]
async fn true_browser_mfa_admin_session_reads_while_prior_generation_cannot(pool: PgPool) {
    let f = fresh(&pool).await;
    let browser = BrowserAuth::new(f.identity.clone(), &[43; 32]).unwrap();
    for (name, role, expected_role) in [("mfa-admin", "admin", 10), ("mfa-root", "superadmin", 100)]
    {
        browser
            .register(&BrowserRegistration {
                username: name,
                password: PASSWORD,
                email: "",
            })
            .await
            .unwrap();
        let id: i64 =
            sqlx::query_scalar("SELECT user_id FROM core_identity.user_profiles WHERE username=$1")
                .bind(name)
                .fetch_one(&pool)
                .await
                .unwrap();
        // Synthetic installer grants real platform role; fresh password login
        // creates the family/proof. No web_sessions/proof INSERT/verified flag.
        sqlx::query("UPDATE core_identity.users SET platform_role=$2 WHERE id=$1")
            .bind(id)
            .bind(role)
            .execute(&pool)
            .await
            .unwrap();
        let original = browser
            .login(name, PASSWORD, &BrowserSessionMetadata::default())
            .await
            .unwrap();
        let access = original.data["access_token"].as_str().unwrap();
        let setup = browser.mfa_setup(access).await.unwrap();
        let now: i64 =
            sqlx::query_scalar("SELECT floor(extract(epoch FROM clock_timestamp()))::bigint")
                .fetch_one(&pool)
                .await
                .unwrap();
        let enabled = browser
            .mfa_enable(access, &code_at(&setup.secret, u64::try_from(now).unwrap()))
            .await
            .unwrap();
        assert!(f.store.users(access, &UsersQuery::default()).await.is_err());
        let fresh = enabled.data["access_token"].as_str().unwrap();
        let result = page(&f, fresh, search(name)).await;
        assert_eq!(result["total"], 1);
        assert_eq!(result["items"][0]["role"], expected_role);
        let principal = f.identity.authorize(fresh).await.unwrap();
        let level: i16 = sqlx::query_scalar(
            "SELECT factor_level FROM core_identity.session_authentication WHERE credential_id=$1",
        )
        .bind(principal.credential_id)
        .fetch_one(&pool)
        .await
        .unwrap();
        assert_eq!(level, 2);
    }
    f.role.cleanup().await;
}
#[test]
fn json_number_bounds_and_defaults_fail_closed() {
    assert_eq!(safe_total(9_007_199_254_740_991), Ok(9_007_199_254_740_991));
    assert!(safe_total(-1).is_err());
    assert!(safe_total(9_007_199_254_740_992).is_err());
    assert_eq!(safe_time(0), Ok(0));
    assert_eq!(safe_time(-1), Ok(-1));
    assert!(safe_time(9_007_199_254_740_992).is_err());
    let q = UsersQuery::default().validate().unwrap();
    assert_eq!((q.page, q.size, q.offset), (1, 20, 0));
}

#[sqlx::test(migrations = false)]
async fn exact_user_detail_preserves_the_list_wallet_projection_and_is_read_only(pool: PgPool) {
    let f = fresh(&pool).await;
    person(&f, &pool, 10, 0, "selected-user").await;
    person(&f, &pool, 11, 0, "contains-10").await;
    person(&f, &pool, 9_007_199_254_740_993, 0, "large-user").await;
    let (ledger, account) = fund(&f, &pool, 10).await;
    ledger_action(
        &ledger,
        "detail-reserve",
        10,
        Action::Reserve {
            account_id: account,
            amount_units: Credits::try_from(200).unwrap(),
        },
    )
    .await;
    let before = state(&pool).await;
    let list = page(&f, &f.admin, search("10")).await;
    assert_eq!(
        list["total"], 2,
        "numeric keyword search also matches profile text"
    );
    let detail = serde_json::to_value(f.store.user(&f.admin, 10).await.unwrap()).unwrap();
    assert_eq!(detail["version"], 1);
    assert_eq!(detail["user"]["id"], "10");
    assert_eq!(
        &detail["user"],
        list["items"]
            .as_array()
            .unwrap()
            .iter()
            .find(|row| row["id"] == "10")
            .unwrap()
    );
    assert_eq!(detail["user"]["wallet"]["available_units"], "800");
    assert_eq!(detail["user"]["wallet"]["reserved_units"], "200");
    assert_eq!(detail["user"]["wallet"]["money_available"], false);
    assert_eq!(detail["statistics"]["user_id"], "10");
    assert_eq!(
        detail["statistics"]["personal_account_id"],
        account.to_string()
    );
    assert_eq!(detail["statistics"]["usage"]["requests"], "0");
    assert_eq!(detail["statistics"]["usage"]["tokens"], "0");
    assert_eq!(detail["statistics"]["usage"]["credits"], "0");
    assert_eq!(
        detail["statistics"]["historical_counters"]["status"],
        "not_imported"
    );
    assert!(detail["statistics"]["referrals"]["invited_users"].is_null());
    let large =
        serde_json::to_value(f.store.user(&f.root, 9_007_199_254_740_993).await.unwrap()).unwrap();
    assert_eq!(large["user"]["id"], "9007199254740993");
    assert_eq!(large["statistics"]["user_id"], "9007199254740993");
    assert!(matches!(
        f.store.user(&f.admin, 999).await,
        Err(AdminUsersError::NotFound)
    ));
    assert_eq!(before, state(&pool).await);
    sqlx::query("UPDATE core_identity.users SET active=FALSE WHERE id=10")
        .execute(&pool)
        .await
        .unwrap();
    let before = state(&pool).await;
    let disabled = serde_json::to_value(f.store.user(&f.admin, 10).await.unwrap()).unwrap();
    assert_eq!(disabled["user"]["wallet"]["status"], "unavailable");
    assert!(disabled["user"]["wallet"].get("wallet_units").is_none());
    assert!(disabled["statistics"]["referrals"]["affiliate_balance_units"].is_null());
    assert_eq!(before, state(&pool).await);
    f.role.cleanup().await;
}

#[sqlx::test(migrations = false)]
async fn exact_user_detail_http_keeps_session_authority_and_private_cache_policy(pool: PgPool) {
    let f = fresh(&pool).await;
    let ordinary = person(&f, &pool, 10, 4, "detail-target").await;
    let key = f
        .identity
        .issue_key(
            &f.admin,
            Account {
                kind: AccountKind::Personal,
                id: 900,
            },
            FundingPolicy::new_personal_key(),
            3600,
        )
        .await
        .unwrap();
    let team = f.identity.create_team(&f.admin).await.unwrap();
    let team_key = f
        .identity
        .issue_key(
            &f.admin,
            Account {
                kind: AccountKind::Team,
                id: team,
            },
            FundingPolicy {
                version: 1,
                account_order: Some(vec![Account {
                    kind: AccountKind::Team,
                    id: team,
                }]),
                personal_billing_preference: None,
            },
            3600,
        )
        .await
        .unwrap();
    let before = state(&pool).await;
    let app = router(f.store.clone());
    for (path, credential, status) in [
        ("10", Some(f.admin.as_str()), StatusCode::OK),
        ("10", Some(f.root.as_str()), StatusCode::OK),
        ("999", Some(f.admin.as_str()), StatusCode::NOT_FOUND),
        ("10", Some(ordinary.secret.as_str()), StatusCode::FORBIDDEN),
        ("999", Some(ordinary.secret.as_str()), StatusCode::FORBIDDEN),
        ("10", Some(key.secret.as_str()), StatusCode::FORBIDDEN),
        ("10", Some(team_key.secret.as_str()), StatusCode::FORBIDDEN),
        ("10", None, StatusCode::UNAUTHORIZED),
        ("0", Some(f.admin.as_str()), StatusCode::BAD_REQUEST),
        ("not-an-id", Some(f.admin.as_str()), StatusCode::BAD_REQUEST),
    ] {
        let mut request = Request::builder().uri(format!("/core/v1/admin/users/{path}"));
        if let Some(secret) = credential {
            request = request.header("authorization", format!("Bearer {secret}"));
        }
        let response = app
            .clone()
            .oneshot(request.body(Body::empty()).unwrap())
            .await
            .unwrap();
        assert_eq!(response.status(), status, "path {path}");
        assert_eq!(response.headers()["cache-control"], "private, no-store");
        assert_eq!(response.headers()["vary"], "Authorization");
        let bytes = to_bytes(response.into_body(), 64 * 1024).await.unwrap();
        let body: Value = serde_json::from_slice(&bytes).unwrap();
        if status == StatusCode::OK {
            assert_eq!(body["user"]["id"], "10");
        } else {
            assert!(body.get("user").is_none());
            assert!(body.get("usage").is_none());
            assert!(body.get("statistics").is_none());
        }
        assert!(!String::from_utf8_lossy(&bytes).contains(&f.admin));
    }
    assert_eq!(before, state(&pool).await);
    f.role.cleanup().await;
}

#[sqlx::test(migrations = false)]
async fn exact_user_detail_statistics_preserve_import_units_and_distinct_invitees(pool: PgPool) {
    let f = fresh(&pool).await;
    crate::runtime_database::check(&f.role.pool, Default::default())
        .await
        .unwrap();
    person(&f, &pool, 10, 0, "history-owner").await;
    let ordinary = person(&f, &pool, 11, 0, "recorded-both").await;
    person(&f, &pool, 12, 0, "recorded-native").await;
    person(&f, &pool, 13, 0, "recorded-imported").await;
    let account: i64 =
        sqlx::query_scalar("SELECT personal_account_id FROM core_identity.users WHERE id=10")
            .fetch_one(&pool)
            .await
            .unwrap();
    let mut tx = pool.begin().await.unwrap();
    sqlx::query("SET TRANSACTION ISOLATION LEVEL SERIALIZABLE")
        .execute(&mut *tx)
        .await
        .unwrap();
    sqlx::query("SET LOCAL synchronous_commit=on")
        .execute(&mut *tx)
        .await
        .unwrap();
    let opening = serde_json::json!({
        "migration_id":"detail-history", "source_entity_key":"user:10",
        "source_archive_sha256":"a".repeat(64), "mapping_sha256":"b".repeat(64),
        "source_row_sha256":"c".repeat(64), "observed_at":1700000000,
        "account_id":account, "available_units":5000, "reserved_units":0,
        "affiliate_units":-250
    });
    let receipt: Value = sqlx::query_scalar("SELECT core_billing.import_opening_balance($1)")
        .bind(&opening)
        .fetch_one(&mut *tx)
        .await
        .unwrap();
    assert_eq!(receipt["status"], "posted");
    let counters = serde_json::json!({
        "migration_id":"detail-history", "source_entity_key":"user:10",
        "source_archive_sha256":"a".repeat(64), "mapping_sha256":"b".repeat(64),
        "source_row_sha256":"c".repeat(64), "observed_at":1700000000,
        "account_id":account, "user_id":10, "used_quota":-17,
        "request_count":9007199254740991_i64, "aff_history":450000
    });
    let receipt: Value =
        sqlx::query_scalar("SELECT core_billing.import_account_opening_counters($1)")
            .bind(&counters)
            .fetch_one(&mut *tx)
            .await
            .unwrap();
    assert_eq!(receipt["status"], "imported");
    sqlx::query(
        "INSERT INTO core_identity.affiliate_codes(user_id,code) VALUES(10,repeat('a',32))",
    )
    .execute(&mut *tx)
    .await
    .unwrap();
    for invitee in [11, 12] {
        crate::affiliate::bind_registration(&mut tx, invitee, &"a".repeat(32))
            .await
            .unwrap();
    }
    sqlx::query("INSERT INTO core_referrals.source(archive_sha256,migration_id,document,review) VALUES(repeat('a',64),'detail-history','{}','{}')")
        .execute(&mut *tx).await.unwrap();
    sqlx::query("INSERT INTO core_referrals.bindings(user_id,inviter_id,legacy_first_top_up,stripe_customer,prior_cash,archive_sha256,source_user_id) VALUES(11,10,0,'',false,repeat('a',64),11),(13,10,0,'',false,repeat('a',64),13)")
        .execute(&mut *tx).await.unwrap();
    // The importer initializes the separately reviewed normalized counter. It
    // is not the original raw counter and must never be added to that counter.
    sqlx::query("INSERT INTO core_referrals.history(inviter,granted) VALUES(10,700000)")
        .execute(&mut *tx)
        .await
        .unwrap();
    tx.commit().await.unwrap();

    let before = state(&pool).await;
    let detail = serde_json::to_value(f.store.user(&f.admin, 10).await.unwrap()).unwrap();
    let statistics = &detail["statistics"];
    // Imported counters remain separate from the existing native receipt reader.
    assert_eq!(detail["usage"]["status"], "available");
    assert_eq!(detail["usage"]["scope"], "actor_personal_usage");
    assert_eq!(detail["usage"]["historical_imports_included"], false);
    assert_eq!(detail["usage"]["settled_requests"], "0");
    assert_eq!(detail["usage"]["gross_settled_units"], "0");
    assert_eq!(detail["usage"]["net_consumption_units"], "0");
    assert_eq!(statistics["usage"]["requests"], "0");
    assert_eq!(statistics["usage"]["credits"], "0");
    assert_eq!(
        statistics["historical_counters"],
        serde_json::json!({
            "status":"available", "unit":"legacy_raw_units", "used_quota":"-17",
            "request_count":"9007199254740991", "affiliate_rewards":"450000",
            "observed_at_unix_ms":1700000000000_i64
        })
    );
    assert_eq!(statistics["referrals"]["affiliate_balance_units"], "-250");
    assert!(statistics["referrals"]["invited_users"].is_null());
    assert!(statistics["referrals"]["reviewed_rewards"].is_null());
    assert_eq!(before, state(&pool).await);

    // Apply the exact new production grant fragment, then only the existing
    // native binding's two read columns. No referral write privilege is needed.
    let grants = include_str!("../../../../../deployment/postgres/grant-core-runtime.sql")
        .split_once("-- BEGIN admin-user-statistics runtime grants\n")
        .unwrap()
        .1
        .split_once("-- END admin-user-statistics runtime grants")
        .unwrap()
        .0
        .replace(":\"runtime_role\"", &f.role.name);
    sqlx::raw_sql(sqlx::AssertSqlSafe::<&str>(&grants))
        .execute(&pool)
        .await
        .unwrap();
    sqlx::raw_sql(sqlx::AssertSqlSafe(format!(
        "GRANT SELECT(user_id,inviter_id) ON core_identity.affiliate_registrations TO {}",
        f.role.name
    )))
    .execute(&pool)
    .await
    .unwrap();
    let narrow: bool = sqlx::query_scalar(
        "SELECT has_column_privilege('core_referrals.bindings','user_id','SELECT')
         AND has_column_privilege('core_referrals.bindings','inviter_id','SELECT')
         AND NOT has_column_privilege('core_referrals.bindings','stripe_customer','SELECT')
         AND NOT has_table_privilege('core_referrals.bindings','SELECT')
         AND NOT has_table_privilege('core_referrals.bindings','INSERT,UPDATE,DELETE')
         AND NOT has_any_column_privilege('core_referrals.bindings','INSERT,UPDATE,REFERENCES')
         AND has_column_privilege('core_referrals.history','inviter','SELECT')
         AND has_column_privilege('core_referrals.history','granted','SELECT')
         AND NOT has_table_privilege('core_referrals.history','SELECT')
         AND NOT has_table_privilege('core_referrals.history','INSERT,UPDATE,DELETE')
         AND NOT has_any_column_privilege('core_referrals.history','INSERT,UPDATE,REFERENCES')
         AND NOT has_table_privilege('core_referrals.source','SELECT')
         AND NOT has_any_column_privilege('core_referrals.source','SELECT')",
    )
    .fetch_one(&f.role.pool)
    .await
    .unwrap();
    assert!(narrow);
    crate::runtime_database::check(&f.role.pool, Default::default())
        .await
        .unwrap();
    let detail = serde_json::to_value(f.store.user(&f.admin, 10).await.unwrap()).unwrap();
    assert_eq!(detail["statistics"]["referrals"]["invited_users"], "3");
    assert_eq!(
        detail["statistics"]["referrals"]["invitation_scope"],
        "recorded_native_and_imported_bindings"
    );
    assert_eq!(
        detail["statistics"]["referrals"]["reviewed_rewards"],
        serde_json::json!({
            "scope":"reviewed_history_and_later_settlements", "granted_credits":"700000",
            "unit":"credit_500k_usd", "credits_per_usd":500000
        })
    );
    let missing = serde_json::to_value(f.store.user(&f.admin, 12).await.unwrap()).unwrap();
    assert!(missing["statistics"]["referrals"]["reviewed_rewards"].is_null());
    assert_eq!(before, state(&pool).await);
    assert!(matches!(
        f.store.user(&ordinary.secret, 10).await,
        Err(AdminUsersError::Forbidden)
    ));

    // An inaccessible optional schema/table remains unknown without aborting
    // the authorized profile's read-only transaction or inventing a zero.
    sqlx::raw_sql(sqlx::AssertSqlSafe(format!(
        "REVOKE USAGE ON SCHEMA core_referrals FROM {}; REVOKE SELECT ON core_history.log_records FROM {}",
        f.role.name, f.role.name
    ))).execute(&pool).await.unwrap();
    let detail = serde_json::to_value(f.store.user(&f.admin, 10).await.unwrap()).unwrap();
    assert_eq!(detail["user"]["id"], "10");
    assert_eq!(detail["usage"]["status"], "available");
    assert_eq!(detail["usage"]["net_consumption_units"], "0");
    assert_eq!(
        detail["statistics"]["usage"],
        serde_json::json!({"status":"unavailable"})
    );
    assert!(detail["statistics"]["referrals"]["invited_users"].is_null());
    assert!(detail["statistics"]["referrals"]["reviewed_rewards"].is_null());
    assert_eq!(
        detail["statistics"]["historical_counters"]["request_count"],
        "9007199254740991"
    );
    assert_eq!(before, state(&pool).await);
    f.role.cleanup().await;
}

#[sqlx::test(migrations = false)]
async fn exact_user_detail_retains_full_bigint_source_counters_without_import_changes(
    pool: PgPool,
) {
    let f = fresh(&pool).await;
    let cases = [
        (20_i64, i64::MIN, 9_007_199_254_740_993_i64, i64::MAX),
        (21_i64, i64::MAX, i64::MIN, -9_007_199_254_740_993_i64),
    ];
    for (user, used, requests, rewards) in cases {
        person(&f, &pool, user, 0, &format!("raw-counter-{user}")).await;
        let account: i64 =
            sqlx::query_scalar("SELECT personal_account_id FROM core_identity.users WHERE id=$1")
                .bind(user)
                .fetch_one(&pool)
                .await
                .unwrap();
        let entity = format!("user:{user}");
        let row_hash = format!("{user:064x}");
        let mut tx = pool.begin().await.unwrap();
        sqlx::query("SET TRANSACTION ISOLATION LEVEL SERIALIZABLE")
            .execute(&mut *tx)
            .await
            .unwrap();
        sqlx::query("SET LOCAL synchronous_commit=on")
            .execute(&mut *tx)
            .await
            .unwrap();
        let opening = serde_json::json!({
            "migration_id":"detail-wide-history", "source_entity_key":entity,
            "source_archive_sha256":"a".repeat(64), "mapping_sha256":"b".repeat(64),
            "source_row_sha256":row_hash, "observed_at":1700000000,
            "account_id":account, "available_units":0, "reserved_units":0, "affiliate_units":0
        });
        let receipt: Value = sqlx::query_scalar("SELECT core_billing.import_opening_balance($1)")
            .bind(&opening)
            .fetch_one(&mut *tx)
            .await
            .unwrap();
        assert_eq!(receipt["status"], "posted");
        let counters = serde_json::json!({
            "migration_id":"detail-wide-history", "source_entity_key":entity,
            "source_archive_sha256":"a".repeat(64), "mapping_sha256":"b".repeat(64),
            "source_row_sha256":row_hash, "observed_at":1700000000,
            "account_id":account, "user_id":user,
            "used_quota":used, "request_count":requests, "aff_history":rewards
        });
        // The current import API still rejects these amounts. This is a read
        // regression for valid BIGINT storage, not new importer admissibility.
        sqlx::query("SAVEPOINT unchanged_import_boundary")
            .execute(&mut *tx)
            .await
            .unwrap();
        let rejected = sqlx::query_scalar::<_, Value>(
            "SELECT core_billing.import_account_opening_counters($1)",
        )
        .bind(&counters)
        .fetch_one(&mut *tx)
        .await
        .unwrap_err();
        assert_eq!(
            rejected
                .as_database_error()
                .and_then(|error| error.code())
                .as_deref(),
            Some("P1000")
        );
        sqlx::query("ROLLBACK TO SAVEPOINT unchanged_import_boundary")
            .execute(&mut *tx)
            .await
            .unwrap();
        sqlx::query("RELEASE SAVEPOINT unchanged_import_boundary")
            .execute(&mut *tx)
            .await
            .unwrap();
        // Seed pre-existing raw facts as the offline owner, keeping every
        // account/user/source/opening receipt binding intact. No money changes.
        sqlx::query("INSERT INTO core_billing.account_opening_counters(account_id,user_id,migration_id,source_entity_key,source_archive_sha256,mapping_sha256,source_row_sha256,observed_at,used_quota,request_count,aff_history,request) VALUES($1,$2,'detail-wide-history',$3,repeat('a',64),repeat('b',64),$4,1700000000,$5,$6,$7,$8)")
            .bind(account).bind(user).bind(&entity).bind(&row_hash)
            .bind(used).bind(requests).bind(rewards).bind(&counters)
            .execute(&mut *tx).await.unwrap();
        tx.commit().await.unwrap();
    }
    let before = state(&pool).await;
    for (user, used, requests, rewards) in cases {
        let detail = serde_json::to_value(f.store.user(&f.admin, user).await.unwrap()).unwrap();
        let counters = &detail["statistics"]["historical_counters"];
        assert_eq!(counters["status"], "available");
        assert_eq!(counters["unit"], "legacy_raw_units");
        assert_eq!(counters["used_quota"], used.to_string());
        assert_eq!(counters["request_count"], requests.to_string());
        assert_eq!(counters["affiliate_rewards"], rewards.to_string());
        assert_eq!(detail["statistics"]["usage"]["requests"], "0");
        assert_eq!(detail["user"]["wallet"]["wallet_units"], "0");
    }
    assert_eq!(before, state(&pool).await);
    f.role.cleanup().await;
}

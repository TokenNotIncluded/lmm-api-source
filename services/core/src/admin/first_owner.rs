//! One offline production owner for an untouched fresh installation.
use lmm_core::{
    core_schema,
    identity::{
        IdentityStore,
        grants::{GrantProfile, GrantStore, IssuerMode},
    },
};
use serde::{Deserialize, Serialize};
use sqlx::{PgConnection, PgPool, Row};
use std::io::{self, Read};
use zeroize::Zeroizing;

const MAX_INPUT_BYTES: u64 = 4096;

/// Contains the initial password. Never Debug or Serialize.
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Request {
    username: String,
    display_name: String,
    password: String,
}

#[derive(Serialize)]
pub(super) struct Receipt {
    protocol: &'static str,
    user_id: i64,
    username: String,
    platform_role: &'static str,
    setup_completed: bool,
}

fn invalid() -> io::Error {
    io::Error::new(io::ErrorKind::InvalidInput, "invalid first-owner input")
}

pub(super) fn read(input: impl Read) -> io::Result<Request> {
    let mut bytes = Zeroizing::new(Vec::new());
    input
        .take(MAX_INPUT_BYTES + 1)
        .read_to_end(&mut bytes)
        .map_err(|_| invalid())?;
    if bytes.len() as u64 > MAX_INPUT_BYTES {
        return Err(invalid());
    }
    let request: Request = serde_json::from_slice(&bytes).map_err(|_| invalid())?;
    // A stable canonical login is also the browser username. Display names may
    // contain Unicode. Reject controls and silent whitespace normalization.
    if !(3..=20).contains(&request.username.len())
        || !request.username.as_bytes()[0].is_ascii_alphanumeric()
        || !request
            .username
            .bytes()
            .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b"_.-".contains(&b))
        || request.display_name.is_empty()
        || request.display_name.len() > 256
        || request.display_name.chars().count() > 80
        || request.display_name.trim() != request.display_name
        || request.display_name.chars().any(char::is_control)
        || request.password.len() > 1024
        || request.password.chars().count() < 12
    {
        return Err(invalid());
    }
    Ok(request)
}

fn refused(message: &'static str) -> io::Error {
    io::Error::other(message)
}

fn storage(_: sqlx::Error) -> io::Error {
    refused("first-owner database operation failed")
}

// Only tables populated by init-db are exempt from the empty-data test. The
// mutable seed values are checked below; core_schema::check and GrantStore::open
// verify the schema receipts. The optional builtin catalog comes later.
const SEEDED: &[(&str, &str)] = &[
    ("core_meta", "schema_contract"),
    ("core_meta", "component_contract"),
    ("core_meta", "runtime_contract"),
    ("core_oauth", "contract"),
    ("core_abuse", "settings"),
    ("core_humangrant", "human_grant_gate"),
    ("core_events", "capacity"),
    ("core_request_limits", "policy"),
    ("core_request_limits", "auto_policy"),
    ("core_promotions", "secrets"),
    ("core_channels", "projection_state"),
    ("core_gift", "risk_key"),
    ("core_gift", "policy"),
    ("core_tool_market", "credit_policy"),
    ("core_trust_pricing", "policies"),
    ("core_billing", "ledger_accounts"),
    ("core_billing", "balance_state"),
    ("public", "routing_active_release"),
];

async fn lock_fresh_database(connection: &mut PgConnection) -> io::Result<()> {
    sqlx::raw_sql("SET TRANSACTION ISOLATION LEVEL READ COMMITTED; SET LOCAL synchronous_commit=on; SET LOCAL lock_timeout='3s'; SET LOCAL statement_timeout='5s'; SET LOCAL idle_in_transaction_session_timeout='10s'")
        .execute(&mut *connection).await.map_err(storage)?;
    // The existing installer, registration and setup paths use these locks.
    // READ COMMITTED statements after waiting observe the winning transaction.
    for lock in [741_803_120_i64, 741_803_121, 741_803_133] {
        sqlx::query("SELECT pg_advisory_xact_lock($1)")
            .bind(lock)
            .execute(&mut *connection)
            .await
            .map_err(storage)?;
    }
    let owner: bool = sqlx::query_scalar(
        "SELECT core_identity.is_identity_relation_owner() AND \
         (current_user=session_user OR EXISTS(SELECT FROM pg_roles r \
          WHERE r.rolname=current_user AND NOT r.rolcanlogin \
          AND pg_has_role(session_user,r.rolname,'MEMBER'))) AND \
         NOT EXISTS(SELECT FROM pg_class c JOIN pg_namespace n ON n.oid=c.relnamespace \
          WHERE n.nspname NOT IN ('pg_catalog','information_schema') \
          AND left(n.nspname,3)<>'pg_' AND c.relkind IN ('r','p') \
          AND pg_get_userbyid(c.relowner)<>current_user)",
    )
    .fetch_one(&mut *connection)
    .await
    .map_err(storage)?;
    if !owner {
        return Err(refused(
            "first-owner requires the offline installation owner",
        ));
    }
    let tables = sqlx::query(
        "SELECT n.nspname,c.relname,format('%I.%I',n.nspname,c.relname) AS qualified \
         FROM pg_class c JOIN pg_namespace n ON n.oid=c.relnamespace \
         WHERE n.nspname NOT IN ('pg_catalog','information_schema') \
         AND left(n.nspname,3)<>'pg_' AND c.relkind IN ('r','p') \
         ORDER BY n.nspname COLLATE \"C\",c.relname COLLATE \"C\"",
    )
    .fetch_all(&mut *connection)
    .await
    .map_err(storage)?;
    // Real table locks also exclude writers which do not cooperate with the
    // advisory locks. All identifiers come quoted by pg_catalog, never stdin.
    let names = tables
        .iter()
        .map(|row| row.try_get::<String, _>("qualified"))
        .collect::<Result<Vec<_>, _>>()
        .map_err(storage)?;
    sqlx::query(&format!(
        "LOCK TABLE {} IN SHARE ROW EXCLUSIVE MODE",
        names.join(",")
    ))
    .execute(&mut *connection)
    .await
    .map_err(storage)?;
    for row in tables {
        let schema: String = row.try_get("nspname").map_err(storage)?;
        let table: String = row.try_get("relname").map_err(storage)?;
        if SEEDED.contains(&(schema.as_str(), table.as_str())) {
            continue;
        }
        let qualified: String = row.try_get("qualified").map_err(storage)?;
        let occupied: bool = sqlx::query_scalar(&format!("SELECT EXISTS(SELECT FROM {qualified})"))
            .fetch_one(&mut *connection)
            .await
            .map_err(storage)?;
        if occupied {
            return Err(refused("first-owner requires an untouched fresh database"));
        }
    }
    let seeds: bool = sqlx::query_scalar(
        r#"SELECT
        (SELECT count(*)=1 AND bool_and(singleton AND revision=0 AND document='{"version":1,"policies":[]}'::jsonb) FROM core_abuse.settings)
        AND (SELECT count(*)=1 AND bool_and(id=1 AND revision=1) FROM core_humangrant.human_grant_gate)
        AND (SELECT count(*)=1 AND bool_and(singleton AND max_events=100000 AND max_bytes=268435456 AND pending_events=0 AND pending_bytes=0) FROM core_events.capacity)
        AND (SELECT count(*)=1 AND bool_and(singleton AND revision=1 AND NOT enabled
            AND window_seconds=60 AND user_limit IS NULL AND group_limits='{}'::jsonb
            AND updated_at>to_timestamp(0)
            AND updated_at<=(SELECT installed_at FROM core_meta.schema_contract WHERE singleton)) FROM core_request_limits.policy)
        AND (SELECT count(*)=1 AND bool_and(singleton AND revision=1 AND groups='{}'::text[]
            AND updated_at>to_timestamp(0)
            AND updated_at<=(SELECT installed_at FROM core_meta.schema_contract WHERE singleton)) FROM core_request_limits.auto_policy)
        -- promotions.sql concatenates two gen_random_uuid() UUIDv4 values.
        -- Their random bytes have no fixed expected value; validate the exact
        -- seeded representation, not a claim that the key was never replaced.
        AND (SELECT count(*)=1 AND bool_and(singleton AND octet_length(code_key)=32
            AND (get_byte(code_key,6) & 240)=64 AND (get_byte(code_key,8) & 192)=128
            AND (get_byte(code_key,22) & 240)=64 AND (get_byte(code_key,24) & 192)=128) FROM core_promotions.secrets)
        -- Fresh init has no charging requests; all projection data/dirty rows
        -- remain subject to the non-seeded empty-table guard above.
        AND (SELECT count(*)=1 AND bool_and(singleton AND ready) FROM core_channels.projection_state)
        AND (SELECT count(*)=1 AND bool_and(singleton AND octet_length(secret)=32) FROM core_gift.risk_key)
        AND (SELECT count(*)=1 AND bool_and(singleton AND NOT enabled AND exclusive_cap=0) FROM core_gift.policy)
        AND (SELECT count(*)=1 AND bool_and(singleton AND version=1 AND platform_account_id IS NULL AND platform_user_id IS NULL AND NOT enabled AND updated_by IS NULL AND updated_at=0 AND fee_bps=0) FROM core_tool_market.credit_policy)
        AND (SELECT count(*)=1 AND bool_and(singleton AND revision=1
            AND spec='{"version":1,"paid_activation_enabled":true,"decay_period_days":90,"tiers":[{"level":0,"min_paid_credits":0,"discount":{"numerator":1,"denominator":1}},{"level":1,"min_paid_credits":500000,"discount":{"numerator":1,"denominator":1}},{"level":2,"min_paid_credits":5000000,"discount":{"numerator":97,"denominator":100}},{"level":3,"min_paid_credits":25000000,"discount":{"numerator":94,"denominator":100}},{"level":4,"min_paid_credits":100000000,"discount":{"numerator":9,"denominator":10}}],"admin_discount":{"numerator":9,"denominator":10},"superadmin_discount":{"numerator":9,"denominator":10}}'::jsonb
            AND source_digest=sha256(convert_to('core-native-trust-policy-v1','UTF8'))
            AND source_cutoff>0 AND native_user_floor=1
            AND native_since>=to_timestamp(source_cutoff)
            AND native_since<=statement_timestamp()) FROM core_trust_pricing.policies)
        AND (SELECT count(*)=1 AND bool_and(singleton AND config_version IS NULL) FROM public.routing_active_release)
        AND (SELECT count(*)=2 AND count(DISTINCT bucket)=2 AND bool_and(owner_account_id IS NULL AND bucket IN ('clearing','revenue') AND unit='credit_500k_usd') FROM core_billing.ledger_accounts)
        AND (SELECT count(*)=2 AND bool_and(balance_units=0 AND revision=0) FROM core_billing.balance_state)"#,
    )
    .fetch_one(connection)
    .await
    .map_err(storage)?;
    if !seeds {
        return Err(refused("first-owner requires an untouched fresh database"));
    }
    Ok(())
}

pub(super) async fn run(pool: &PgPool, request: Request) -> io::Result<Receipt> {
    core_schema::check(pool)
        .await
        .map_err(|_| refused("first-owner requires the current installed schema"))?;
    // Reuse the existing read-only OAuth contract verification. The profile is
    // only a value required by that verifier; it makes no network requests and
    // is discarded without creating an issuer, client, token or permission.
    let profile = GrantProfile::new("https://installer.invalid", IssuerMode::Production)
        .map_err(|_| refused("first-owner schema verification failed"))?;
    GrantStore::open(IdentityStore::from_pool(pool.clone()), profile)
        .await
        .map_err(|_| refused("first-owner requires the current installed schema"))?;
    let Request {
        username,
        display_name,
        password,
    } = request;
    let hash = IdentityStore::from_pool(pool.clone())
        .hash_installer_password(password)
        .await
        .map_err(|_| refused("first-owner password preparation failed"))?;
    let mut tx = pool.begin().await.map_err(storage)?;
    lock_fresh_database(&mut tx).await?;
    let account: i64 = sqlx::query_scalar(
        "INSERT INTO core_identity.accounts(kind,service_level) VALUES ('personal',0) RETURNING id",
    )
    .fetch_one(&mut *tx)
    .await
    .map_err(storage)?;
    sqlx::query("INSERT INTO core_billing.account_policies(account_id) VALUES ($1)")
        .bind(account)
        .execute(&mut *tx)
        .await
        .map_err(storage)?;
    let user: i64 = sqlx::query_scalar(
        "INSERT INTO core_identity.users(personal_account_id,platform_role) VALUES ($1,'superadmin') RETURNING id",
    )
    .bind(account).fetch_one(&mut *tx).await.map_err(storage)?;
    sqlx::query("INSERT INTO core_identity.user_profiles(user_id,username,display_name,password_login_allowed) VALUES ($1,$2,$3,TRUE)")
        .bind(user).bind(&username).bind(display_name).execute(&mut *tx).await.map_err(storage)?;
    sqlx::query("INSERT INTO core_identity.password_logins(login_name,user_id,password_hash) VALUES ($1,$2,$3)")
        .bind(&username).bind(user).bind(hash).execute(&mut *tx).await.map_err(storage)?;
    sqlx::query("INSERT INTO core_identity.audit(actor_user_id,action,target) VALUES ($1,'installation.first-owner',jsonb_build_object('user_id',$1::bigint))")
        .bind(user).execute(&mut *tx).await.map_err(storage)?;
    sqlx::query("INSERT INTO core_console.initialization(version) VALUES ($1)")
        .bind(env!("CARGO_PKG_VERSION"))
        .execute(&mut *tx)
        .await
        .map_err(storage)?;
    tx.commit().await.map_err(storage)?;
    Ok(Receipt {
        protocol: "core.first-owner.v1",
        user_id: user,
        username,
        platform_role: "superadmin",
        setup_completed: true,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    const VALID: &str = r#"{"username":"site-owner","display_name":"Site Owner","password":"synthetic installer password"}"#;

    #[test]
    fn accepts_only_bounded_closed_validated_stdin() {
        assert!(read(VALID.as_bytes()).is_ok());
        for value in [
            "{}".to_owned(),
            VALID.replace("site-owner", "UPPER"),
            VALID.replace("site-owner", "a"),
            VALID.replace("site-owner", "x\\u0000name"),
            VALID.replace("Site Owner", " Site Owner"),
            VALID.replace("Site Owner", "Owner\\nName"),
            VALID.replace("synthetic installer password", "short"),
            VALID.replace('}', ",\"role\":\"admin\"}"),
            VALID.replace('}', ",\"password\":\"duplicate-value\"}"),
            format!("{VALID} trailing"),
            "x".repeat(4097),
        ] {
            let error = read(value.as_bytes()).err().expect("must reject");
            assert_eq!(error.to_string(), "invalid first-owner input");
            assert!(!error.to_string().contains("synthetic"));
        }
    }

    #[sqlx::test(migrations = false)]
    async fn an_insert_failure_rolls_back_all_identity_and_setup_rows(pool: PgPool) {
        core_schema::init(&pool).await.unwrap();
        // Exercise the transaction failure after the account/user INSERTs.
        // Production stdin validation rejects this invalid display name first.
        let request = Request {
            username: "site-owner".into(),
            display_name: "x".repeat(257),
            password: "synthetic installer password".into(),
        };
        assert!(run(&pool, request).await.is_err());
        let empty: bool = sqlx::query_scalar(
            "SELECT NOT EXISTS(SELECT FROM core_identity.users) \
             AND NOT EXISTS(SELECT FROM core_identity.accounts) \
             AND NOT EXISTS(SELECT FROM core_identity.password_logins) \
             AND NOT EXISTS(SELECT FROM core_identity.user_profiles) \
             AND NOT EXISTS(SELECT FROM core_identity.audit) \
             AND NOT EXISTS(SELECT FROM core_billing.account_policies) \
             AND NOT EXISTS(SELECT FROM core_console.initialization)",
        )
        .fetch_one(&pool)
        .await
        .unwrap();
        assert!(empty);
    }

    async fn assert_no_owner_or_money(pool: &PgPool) {
        let untouched: bool=sqlx::query_scalar(
            "SELECT NOT EXISTS(SELECT FROM core_identity.users)
              AND NOT EXISTS(SELECT FROM core_identity.accounts)
              AND NOT EXISTS(SELECT FROM core_identity.credentials)
              AND NOT EXISTS(SELECT FROM core_identity.password_logins)
              AND NOT EXISTS(SELECT FROM core_identity.user_profiles)
              AND NOT EXISTS(SELECT FROM core_identity.audit)
              AND NOT EXISTS(SELECT FROM core_billing.account_policies)
              AND NOT EXISTS(SELECT FROM core_console.initialization)
              AND NOT EXISTS(SELECT FROM core_billing.ledger_journals)
              AND NOT EXISTS(SELECT FROM core_billing.ledger_entries)
              AND (SELECT count(*)=2 AND bool_and(balance_units=0 AND revision=0) FROM core_billing.balance_state)"
        ).fetch_one(pool).await.unwrap();
        assert!(untouched);
    }

    #[sqlx::test(migrations = false)]
    async fn current_fresh_seeds_allow_exactly_one_owner_without_session_or_funds(pool: PgPool) {
        core_schema::init(&pool).await.unwrap();
        let receipt = run(&pool, read(VALID.as_bytes()).unwrap()).await.unwrap();
        assert_eq!(receipt.platform_role, "superadmin");
        assert!(receipt.setup_completed);
        let mut tx = pool.begin().await.unwrap();
        sqlx::query("SET TRANSACTION ISOLATION LEVEL REPEATABLE READ READ ONLY")
            .execute(&mut *tx)
            .await
            .unwrap();
        let wallet = lmm_core::financial_profile::read_wallet(&mut tx, receipt.user_id)
            .await
            .unwrap();
        assert!(wallet.financial_projection_available);
        assert_eq!(wallet.wallet_unit, "core_credit");
        assert_eq!(
            (
                wallet.quota,
                wallet.wallet_reserved_quota,
                wallet.wallet_debt
            ),
            (0, 0, 0)
        );
        assert!(!wallet.wallet_money_available);
        let profile = lmm_core::financial_profile::read(&mut tx, receipt.user_id)
            .await
            .unwrap();
        assert!(profile.financial_projection_available && profile.usage_projection_available);
        assert_eq!(
            (
                profile.quota,
                profile.used_quota,
                profile.request_count,
                profile.aff_quota
            ),
            (0, 0, 0, 0)
        );
        assert_eq!(profile.normalized_used_quota, Some(0));
        tx.commit().await.unwrap();
        let correct: bool=sqlx::query_scalar(
            "SELECT (SELECT count(*)=1 AND bool_and(platform_role='superadmin') FROM core_identity.users)
              AND (SELECT count(*)=1 FROM core_identity.password_logins)
              AND (SELECT count(*)=1 FROM core_identity.accounts)
              AND (SELECT count(*)=1 FROM core_console.initialization)
              AND NOT EXISTS(SELECT FROM core_identity.credentials)
              AND NOT EXISTS(SELECT FROM core_billing.ledger_journals)
              AND NOT EXISTS(SELECT FROM core_billing.ledger_entries)
              AND (SELECT count(*)=2 AND bool_and(balance_units=0 AND revision=0) FROM core_billing.balance_state)
              AND NOT EXISTS(SELECT FROM core_trust_pricing.policy_edit_signing_key)
              AND NOT EXISTS(SELECT FROM core_trust_pricing.policy_editors)
              AND NOT EXISTS(SELECT FROM core_trust_pricing.policy_edit_audit)
              AND NOT EXISTS(SELECT FROM core_meta.trust_policy_upgrades)
              AND NOT EXISTS(SELECT FROM core_channels.catalog_observations)
              AND NOT EXISTS(SELECT FROM core_meta.channel_observations_upgrades)"
        ).fetch_one(&pool).await.unwrap();
        assert!(correct);
        let error = run(&pool, read(VALID.as_bytes()).unwrap())
            .await
            .err()
            .unwrap();
        assert_eq!(
            error.to_string(),
            "first-owner requires an untouched fresh database"
        );
        let count: i64 = sqlx::query_scalar("SELECT count(*) FROM core_identity.users")
            .fetch_one(&pool)
            .await
            .unwrap();
        assert_eq!(count, 1);
    }

    #[sqlx::test(migrations = false)]
    async fn changed_or_missing_component44_seeds_are_not_fresh(pool: PgPool) {
        core_schema::init(&pool).await.unwrap();
        // Each mutation is committed before the guard opens its own transaction.
        // Matching the refusal text prevents a transaction setup failure from
        // masquerading as a successful seed rejection.
        for (table, changes) in [
            (
                "core_request_limits.policy",
                vec![
                    "revision=2",
                    "enabled=true",
                    "window_seconds=61",
                    "user_limit=1",
                    "group_limits='{\"test\":1}'::jsonb",
                    "updated_at='infinity'::timestamptz",
                    "updated_at=to_timestamp(0)",
                    "updated_at=(SELECT installed_at + interval '1 microsecond' FROM core_meta.schema_contract WHERE singleton)",
                ],
            ),
            (
                "core_request_limits.auto_policy",
                vec![
                    "revision=2",
                    "groups=ARRAY['test']",
                    "updated_at='infinity'::timestamptz",
                    "updated_at=to_timestamp(0)",
                    "updated_at=(SELECT installed_at + interval '1 microsecond' FROM core_meta.schema_contract WHERE singleton)",
                ],
            ),
            (
                "core_promotions.secrets",
                vec![
                    "code_key=decode(repeat('00',32),'hex')",
                    "code_key=set_byte(code_key,6,0)",
                    "code_key=set_byte(code_key,8,0)",
                    "code_key=set_byte(code_key,22,0)",
                    "code_key=set_byte(code_key,24,0)",
                ],
            ),
            ("core_channels.projection_state", vec!["ready=false"]),
        ] {
            let original: serde_json::Value =
                sqlx::query_scalar(&format!("SELECT to_jsonb(t) FROM {table} t"))
                    .fetch_one(&pool)
                    .await
                    .unwrap();
            for change in changes.into_iter().map(Some).chain(std::iter::once(None)) {
                let mutation = match change {
                    Some(change) => format!("UPDATE {table} SET {change}"),
                    None => format!("DELETE FROM {table}"),
                };
                sqlx::query(&mutation).execute(&pool).await.unwrap();
                let mut tx = pool.begin().await.unwrap();
                let error = lock_fresh_database(&mut tx)
                    .await
                    .err()
                    .expect("polluted seed accepted");
                assert_eq!(
                    error.to_string(),
                    "first-owner requires an untouched fresh database",
                    "{mutation}"
                );
                tx.rollback().await.unwrap();
                assert_no_owner_or_money(&pool).await;
                let mut restore = pool.begin().await.unwrap();
                sqlx::query(&format!("DELETE FROM {table}"))
                    .execute(&mut *restore)
                    .await
                    .unwrap();
                sqlx::query(&format!(
                    "INSERT INTO {table} SELECT * FROM jsonb_populate_record(NULL::{table},$1)"
                ))
                .bind(sqlx::types::Json(&original))
                .execute(&mut *restore)
                .await
                .unwrap();
                restore.commit().await.unwrap();
            }
        }
        // All four original seed rows restored: the same guard must now pass.
        let mut tx = pool.begin().await.unwrap();
        lock_fresh_database(&mut tx).await.unwrap();
        tx.rollback().await.unwrap();
    }

    #[sqlx::test(migrations = false)]
    async fn preprovisioned_trust_key_or_editor_blocks_first_owner(pool: PgPool) {
        core_schema::init(&pool).await.unwrap();
        for (table, insert) in [
            (
                "core_trust_pricing.policy_edit_signing_key",
                "INSERT INTO core_trust_pricing.policy_edit_signing_key(secret) VALUES(decode(repeat('0b',32),'hex'))",
            ),
            (
                "core_trust_pricing.policy_editors",
                "INSERT INTO core_trust_pricing.policy_editors(editor_id,actor_user_id,version,credential_id,passkey,rp_id,origin) VALUES('preprovisioned',1,1,decode('01','hex'),'{\"cred\":{}}'::jsonb,'localhost','https://localhost')",
            ),
        ] {
            sqlx::query(insert).execute(&pool).await.unwrap();
            let error = run(&pool, read(VALID.as_bytes()).unwrap())
                .await
                .err()
                .expect("preprovisioned trust authority accepted");
            assert_eq!(
                error.to_string(),
                "first-owner requires an untouched fresh database"
            );
            assert_no_owner_or_money(&pool).await;
            let count: i64 = sqlx::query_scalar(&format!("SELECT count(*) FROM {table}"))
                .fetch_one(&pool)
                .await
                .unwrap();
            assert_eq!(count, 1, "refusal must preserve preprovisioned authority");
            sqlx::query(&format!("DELETE FROM {table}"))
                .execute(&pool)
                .await
                .unwrap();
        }
    }
}

#[cfg(test)]
mod component46_tests {
    use super::*;
    #[sqlx::test(migrations = false)]
    async fn populated_observation_receipt_is_not_fresh(pool: PgPool) {
        core_schema::init(&pool).await.unwrap();
        sqlx::query("INSERT INTO core_meta.channel_observations_upgrades(upgrade_id,request_sha256,request,preserved_data_sha256,preserved_permissions_sha256,successor_catalog_sha256) VALUES('not-a-fresh-install',decode(repeat('ab',32),'hex'),'{}',decode(repeat('ab',32),'hex'),decode(repeat('ab',32),'hex'),decode(repeat('ab',32),'hex'))").execute(&pool).await.unwrap();
        let mut tx = pool.begin().await.unwrap();
        let error = lock_fresh_database(&mut tx)
            .await
            .err()
            .expect("component46 receipt is not a first-owner seed");
        assert_eq!(
            error.to_string(),
            "first-owner requires an untouched fresh database"
        );
        tx.rollback().await.unwrap();
    }
}

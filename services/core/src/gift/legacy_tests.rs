//! Real PostgreSQL behavior at the old/new authority boundary. All identities,
//! source records and the HMAC key here are synthetic; no production material.
use super::*;
use crate::{gift::GiftStore, identity::IdentityStore, test_runtime::RuntimeRole};
use sha2::{Digest, Sha256};
use sqlx::PgPool;

const USER: i64 = 91_001;
const OTHER: i64 = 91_002;
const NAMESPACE: &str = "synthetic-cc52-gift-source-001";
const SECRET: &[u8] = b"source-secret-is-a-string-not-a-hex-decoding";

struct Fixture {
    session: String,
    other_session: String,
    account: i64,
    time: i64,
}

async fn fixture(pool: &PgPool, source_status: &str, amount: i64) -> Fixture {
    crate::core_schema::init(pool).await.unwrap();
    let ids = IdentityStore::from_pool(pool.clone());
    let session = ids.bootstrap_user(USER, 0).await.unwrap().secret;
    let other_session = ids.bootstrap_user(OTHER, 0).await.unwrap().secret;
    let account = ids.authorize(&session).await.unwrap().owner_account_id;
    assert_ne!(
        account, USER,
        "the fixture must exercise actual account mapping"
    );
    sqlx::query("UPDATE core_identity.user_profiles SET email=CASE user_id WHEN $1 THEN 'A.B+offer@Googlemail.com' ELSE 'other@example.test' END")
        .bind(USER).execute(pool).await.unwrap();
    sqlx::query("UPDATE core_gift.policy SET enabled=TRUE,exclusive_cap=1001")
        .execute(pool)
        .await
        .unwrap();
    let time: i64 =
        sqlx::query_scalar("SELECT floor(extract(epoch FROM clock_timestamp()))::bigint")
            .fetch_one(pool)
            .await
            .unwrap();
    sqlx::query("INSERT INTO core_gift.legacy_installations(namespace,source_revision,source_archive_sha256,source_export_sha256,source_export_proof_sha256,identity_scope,identity_validator_sha256,identity_material_sha256,identity_receipt_sha256,identity_mapping_sha256,source_observed_at,source_enabled,inclusive_cap,risk_secret,input_sha256) VALUES($1,'cc52aa195c6f2a01d703a1a407be73c31b89867d',repeat('1',64),repeat('2',64),repeat('3',64),'synthetic-current-identity',repeat('4',64),repeat('5',64),repeat('6',64),repeat('7',64),$2,true,1000,$3,repeat('8',64))")
        .bind(NAMESPACE).bind(time).bind(SECRET).execute(pool).await.unwrap();
    sqlx::query("INSERT INTO core_gift.legacy_bindings(namespace,source_user_id,user_id,account_id,source_present) VALUES($1,41,$2,$3,TRUE)")
        .bind(NAMESPACE).bind(USER).bind(account).execute(pool).await.unwrap();
    let gift_id = crate::hex_codec::encode(Sha256::digest(
        [
            b"legacy-assistant-gift-v1\0".as_slice(),
            NAMESPACE.as_bytes(),
            b"\0",
            b"1",
        ]
        .concat(),
    ));
    sqlx::query("INSERT INTO core_gift.legacy_decisions(namespace,source_id,source_user_id,gift_id,source_status,source_quota,amount_credits,created_at,claimed_at,source_row_sha256,amount_basis_sha256) VALUES($1,1,41,$2,$3,10000000,$4,$5,$6,repeat('a',64),repeat('b',64))")
        .bind(NAMESPACE).bind(gift_id).bind(source_status).bind(amount).bind(time-2_000_000)
        .bind(if source_status=="claimed" {time-1_000_000} else {0})
        .execute(pool).await.unwrap();
    sqlx::query("INSERT INTO core_gift.legacy_imports(namespace,scope,operation_id,input_sha256,material_sha256,counts,installed_by,database_oid) VALUES($1,'current-source-gift-continuation-v1','synthetic-legacy-gift-install-001',repeat('8',64),repeat('9',64),'{\"bindings\":1,\"decisions\":1,\"risk_memories\":0,\"profiles\":0,\"fingerprints\":0,\"claims\":0}',current_user,(SELECT oid FROM pg_database WHERE datname=current_database()))")
        .bind(NAMESPACE).execute(pool).await.unwrap();
    Fixture {
        session,
        other_session,
        account,
        time,
    }
}

async fn source_state(pool: &PgPool) -> serde_json::Value {
    sqlx::query_scalar("SELECT jsonb_build_array((SELECT jsonb_agg(to_jsonb(t) ORDER BY namespace) FROM core_gift.legacy_installations t),(SELECT jsonb_agg(to_jsonb(t) ORDER BY namespace,source_user_id) FROM core_gift.legacy_bindings t),(SELECT jsonb_agg(to_jsonb(t) ORDER BY namespace,source_id) FROM core_gift.legacy_decisions t),(SELECT jsonb_agg(to_jsonb(t) ORDER BY namespace,key_hash) FROM core_gift.legacy_risk_memories t),(SELECT jsonb_agg(to_jsonb(t) ORDER BY namespace) FROM core_gift.legacy_imports t))")
        .fetch_one(pool).await.unwrap()
}

async fn native_counts(pool: &PgPool) -> (i64, i64, i64, i64) {
    sqlx::query_as("SELECT (SELECT count(*) FROM core_gift.decisions),(SELECT count(*) FROM core_gift.qualifications),(SELECT count(*) FROM core_charging.requests),(SELECT count(*) FROM core_billing.ledger_journals WHERE kind='gift')")
        .fetch_one(pool).await.unwrap()
}

#[sqlx::test]
async fn old_offer_claim_requires_trusted_peer_and_posts_one_original_amount(pool: PgPool) {
    let f = fixture(&pool, "offered", 1000).await;
    let before = source_state(&pool).await;
    let role = RuntimeRole::create(&pool).await;
    role.optional("core_gift").await;
    let store = GiftStore::new(role.pool.clone());
    let blocked = store.get(&f.session).await.unwrap().unwrap();
    assert!(!blocked.claim_available);
    assert_eq!(store.claim_stored(&f.session).await, Err(Error::Forbidden));
    assert!(
        store
            .get_with_peer(&f.other_session, Some("203.0.113.7".parse().unwrap()))
            .await
            .unwrap()
            .is_none()
    );
    assert_eq!(
        store
            .claim_stored_with_peer("not-a-session", Some("203.0.113.7".parse().unwrap()))
            .await,
        Err(Error::Unauthorized)
    );
    let peer = Some("::ffff:203.0.113.7".parse().unwrap());
    let available = store
        .get_with_peer(&f.session, peer)
        .await
        .unwrap()
        .unwrap();
    assert!(
        available.claim_available,
        "source inclusive cap remains inclusive"
    );
    assert_eq!(available.credit_amount, 1000);
    assert_eq!(available.max_credit_amount, 1001);
    assert_eq!(native_counts(&pool).await, (0, 0, 0, 0));
    let (a, b) = tokio::join!(
        store.claim_stored_with_peer(&f.session, peer),
        store.claim_stored_with_peer(&f.session, peer)
    );
    let a = a.unwrap();
    let b = b.unwrap();
    assert_eq!(a.0, b.0);
    assert_ne!(a.1, b.1);
    let replay = store.claim_stored(&f.session).await.unwrap();
    assert_eq!(replay.0, a.0);
    assert!(replay.1);
    let journal = a.0.ledger_transaction_id.unwrap();
    let actual:(i64,i64,i64,String)=sqlx::query_as("SELECT j.actor_user_id,a.owner_account_id,j.amount_units,j.scope FROM core_billing.ledger_journals j JOIN core_billing.ledger_accounts a ON a.id=j.payee_id WHERE j.id=$1")
        .bind(journal).fetch_one(&pool).await.unwrap();
    assert_eq!(
        actual,
        (USER, f.account, 1000, "core-assistant-legacy-gift".into())
    );
    assert_eq!(native_counts(&pool).await, (0, 0, 0, 1));
    assert_eq!(source_state(&pool).await, before);
    for table in [
        "legacy_installations",
        "legacy_bindings",
        "legacy_decisions",
        "legacy_risk_memories",
        "legacy_imports",
    ] {
        let writable:bool=sqlx::query_scalar("SELECT has_table_privilege('core_gift.'||$1,'INSERT,UPDATE,DELETE,TRUNCATE') OR has_any_column_privilege('core_gift.'||$1,'INSERT,UPDATE')")
            .bind(table).fetch_one(&role.pool).await.unwrap();
        assert!(!writable, "source authority {table}");
    }
    role.cleanup().await;
}

#[sqlx::test]
async fn old_claimed_receipt_replays_without_a_new_ledger_or_google_fact(pool: PgPool) {
    let f = fixture(&pool, "claimed", 750).await;
    sqlx::query("UPDATE core_gift.policy SET enabled=false,exclusive_cap=0")
        .execute(&pool)
        .await
        .unwrap();
    let before = source_state(&pool).await;
    let store = GiftStore::new(pool.clone());
    let (receipt, already) = store.claim_stored(&f.session).await.unwrap();
    assert!(already);
    assert_eq!(receipt.claimed_at_unix, Some(f.time - 1_000_000));
    assert_eq!(receipt.credit_amount, 750);
    assert_eq!(receipt.ledger_transaction_id, None);
    assert_eq!(receipt.claim_blocked_code, "already_claimed_legacy");
    assert_eq!(native_counts(&pool).await, (0, 0, 0, 0));
    assert_eq!(source_state(&pool).await, before);
}

#[sqlx::test]
async fn declined_and_orphan_decisions_cannot_create_native_gifts(pool: PgPool) {
    let f = fixture(&pool, "declined", 0).await;
    let store = GiftStore::new(pool.clone());
    assert!(
        store
            .get_with_peer(&f.session, Some("203.0.113.7".parse().unwrap()))
            .await
            .unwrap()
            .is_none()
    );
    let mut c = pool.acquire().await.unwrap();
    assert_eq!(
        native_admission(&mut c, USER).await.unwrap().0,
        "reward_used"
    );
    assert_eq!(store.claim_stored(&f.session).await, Err(Error::NotFound));
    drop(c);
    sqlx::query("INSERT INTO core_gift.legacy_bindings(namespace,source_user_id,user_id,account_id,source_present) VALUES($1,99,NULL,NULL,FALSE)")
        .bind(NAMESPACE).execute(&pool).await.unwrap();
    assert!(
        sqlx::query(
            "UPDATE core_gift.legacy_bindings SET user_id=$1,account_id=$2 WHERE source_user_id=99"
        )
        .bind(OTHER)
        .bind(f.account)
        .execute(&pool)
        .await
        .is_err()
    );
    assert!(sqlx::query("INSERT INTO core_gift.legacy_bindings(namespace,source_user_id,user_id,account_id,source_present) VALUES($1,98,$2,$3,FALSE)")
        .bind(NAMESPACE).bind(OTHER).bind(f.account).execute(&pool).await.is_err());
    assert_eq!(native_counts(&pool).await, (0, 0, 0, 0));
}

#[sqlx::test]
async fn legacy_freshness_identity_and_campaign_boundaries_are_preserved(pool: PgPool) {
    let f = fixture(&pool, "offered", 100).await;
    let mut c = pool.acquire().await.unwrap();
    let i = installation(&mut c).await.unwrap().unwrap();
    observe(&mut c, USER, "203.0.113.7".parse().unwrap())
        .await
        .unwrap();
    let stamp: i64 = sqlx::query_scalar(
        "SELECT observed_at FROM core_gift.legacy_profiles WHERE subject='source:41'",
    )
    .fetch_one(&mut *c)
    .await
    .unwrap();
    assert!(
        profile(&mut c, &i, USER, "source:41", None, stamp + 900)
            .await
            .unwrap()
            .is_some()
    );
    assert!(
        profile(&mut c, &i, USER, "source:41", None, stamp + 901)
            .await
            .unwrap()
            .is_none()
    );
    assert!(
        profile(&mut c, &i, USER, "source:41", None, stamp - 60)
            .await
            .unwrap()
            .is_some()
    );
    assert!(
        profile(&mut c, &i, USER, "source:41", None, stamp - 61)
            .await
            .unwrap()
            .is_none()
    );
    sqlx::query(
        "UPDATE core_identity.user_profiles SET email='changed@example.test' WHERE user_id=$1",
    )
    .bind(USER)
    .execute(&mut *c)
    .await
    .unwrap();
    assert!(
        profile(&mut c, &i, USER, "source:41", None, stamp)
            .await
            .unwrap()
            .is_none()
    );
    sqlx::query("UPDATE core_identity.user_profiles SET email='ab@gmail.com' WHERE user_id=$1")
        .bind(USER)
        .execute(&mut *c)
        .await
        .unwrap();
    let net = network(&i, "203.0.113.7".parse().unwrap()).unwrap();
    // Four template peers are required, only three of them need this network.
    for subject in [
        "source:41",
        "source:101",
        "source:102",
        "source:103",
        "source:104",
    ] {
        for (message, token) in [(1u8, 11u8), (1, 12), (2, 13), (2, 14)] {
            sqlx::query("INSERT INTO core_gift.legacy_fingerprints(namespace,subject,message_digest,token,created_at) VALUES($1,$2,$3,$4,$5)")
                .bind(NAMESPACE).bind(subject).bind(vec![message;32]).bind(vec![token;32]).bind(stamp-604800).execute(&mut *c).await.unwrap();
        }
        if subject != "source:41" && subject != "source:104" {
            sqlx::query("INSERT INTO core_gift.legacy_profiles(namespace,subject,identity_hash,network_hash,observed_at) VALUES($1,$2,$3,$4,$5)")
                .bind(NAMESPACE).bind(subject).bind(vec![1u8;32]).bind(&net).bind(stamp-604800).execute(&mut *c).await.unwrap();
        }
    }
    assert!(
        campaign_hold(&mut c, &i, "source:41", &net, stamp)
            .await
            .unwrap()
    );
    assert!(
        !campaign_hold(&mut c, &i, "source:41", &net, stamp + 1)
            .await
            .unwrap()
    );
    sqlx::query("DELETE FROM core_gift.legacy_fingerprints WHERE subject='source:104'")
        .execute(&mut *c)
        .await
        .unwrap();
    assert!(
        !campaign_hold(&mut c, &i, "source:41", &net, stamp)
            .await
            .unwrap()
    );
    assert_eq!(native_counts(&pool).await, (0, 0, 0, 0));
    assert_eq!(
        f.account,
        IdentityStore::from_pool(pool.clone())
            .authorize(&f.session)
            .await
            .unwrap()
            .owner_account_id
    );
}

#[sqlx::test]
async fn native_admission_retains_identity_and_thirty_day_network_memory(pool: PgPool) {
    let f = fixture(&pool, "offered", 100).await;
    let mut c = pool.acquire().await.unwrap();
    observe(&mut c, OTHER, "203.0.113.7".parse().unwrap())
        .await
        .unwrap();
    let i = installation(&mut c).await.unwrap().unwrap();
    let net = network(&i, "203.0.113.7".parse().unwrap()).unwrap();
    sqlx::query("INSERT INTO core_gift.legacy_risk_memories(namespace,key_hash,kind,decision_count,window_started_at,updated_at,source_row_sha256) VALUES($1,$2,'network',3,$3,$3,repeat('c',64))")
        .bind(NAMESPACE).bind(&net).bind(f.time).execute(&mut *c).await.unwrap();
    assert_eq!(
        native_admission(&mut c, OTHER).await.unwrap(),
        (String::new(), 3)
    );
    assert_eq!(
        network_count(&mut c, &i, &net, f.time + 2_592_000 - 1)
            .await
            .unwrap(),
        3
    );
    assert_eq!(
        network_count(&mut c, &i, &net, f.time + 2_592_000)
            .await
            .unwrap(),
        0
    );
    let identity = identity(&mut c, &i, OTHER).await.unwrap().unwrap();
    sqlx::query("INSERT INTO core_gift.legacy_risk_memories(namespace,key_hash,kind,decision_count,window_started_at,updated_at,source_row_sha256) VALUES($1,$2,'identity',1,$3,$3,repeat('d',64))")
        .bind(NAMESPACE).bind(identity).bind(f.time).execute(&mut *c).await.unwrap();
    assert_eq!(
        native_admission(&mut c, OTHER).await.unwrap().0,
        "identity_used"
    );
    // An existing source offer skips identity-used admission, as cc52 did.
    drop(c);
    assert!(
        GiftStore::new(pool.clone())
            .get_with_peer(&f.session, Some("203.0.113.7".parse().unwrap()))
            .await
            .unwrap()
            .unwrap()
            .claim_available
    );
    assert_eq!(native_counts(&pool).await, (0, 0, 0, 0));
}

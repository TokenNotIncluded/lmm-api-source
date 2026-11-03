//! Actual Python cutover plan against an isolated database, followed by native
//! Rust claim/cancel operations. No production users or provider calls.
use lmm_core::{
    core_schema,
    identity::{BrowserAuth, BrowserRegistration, BrowserSessionMetadata, IdentityStore},
    wallet::Store,
};
use serde_json::{Value, json};
use sqlx::PgPool;
use std::{
    io::Write,
    process::{Command, Stdio},
};

#[path = "common/runtime.rs"]
mod runtime;

fn render(data: &Value, phase: &str, archive: &str) -> String {
    let mut child = Command::new("python3")
        .arg("-B").arg("-c")
        .arg("import json,sys; import legacy_cutover_import as i; import legacy_cutover_money as m; d=json.load(sys.stdin); f=i.plan_opening if sys.argv[1]=='opening' else m.plan_wallet; print('\\n'.join(f(d,sys.argv[2],'wallet-cutover-test',2000000000)[0]))")
        .arg(phase).arg(archive)
        .current_dir(concat!(env!("CARGO_MANIFEST_DIR"), "/../../scripts"))
        .stdin(Stdio::piped()).stdout(Stdio::piped()).stderr(Stdio::piped()).spawn().unwrap();
    child
        .stdin
        .take()
        .unwrap()
        .write_all(serde_json::to_string(data).unwrap().as_bytes())
        .unwrap();
    let out = child.wait_with_output().unwrap();
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    String::from_utf8(out.stdout).unwrap()
}
async fn apply(pool: &PgPool, sql: &str) -> Result<(), sqlx::Error> {
    let mut tx = pool.begin().await?;
    sqlx::raw_sql("SET TRANSACTION ISOLATION LEVEL SERIALIZABLE; SET LOCAL synchronous_commit=on;")
        .execute(&mut *tx)
        .await?;
    sqlx::raw_sql(sql).execute(&mut *tx).await?;
    tx.commit().await
}
async fn balance(pool: &PgPool, account: i64, bucket: &str) -> i64 {
    sqlx::query_scalar("SELECT b.balance_units FROM core_billing.balance_state b JOIN core_billing.ledger_accounts a ON a.id=b.ledger_account_id WHERE a.owner_account_id=$1 AND a.bucket=$2")
        .bind(account).bind(bucket).fetch_one(pool).await.unwrap()
}

#[sqlx::test(migrations = false)]
async fn imported_pending_transfers_preserve_balance_and_native_lifecycle(pool: PgPool) {
    core_schema::init(&pool).await.unwrap();
    core_schema::check(&pool).await.unwrap();
    let identity = IdentityStore::from_pool(pool.clone()).with_registration_enabled(true);
    let browser = BrowserAuth::new(identity.clone(), &[43; 32]).unwrap();
    browser
        .register(&BrowserRegistration {
            username: "legacy_sender",
            password: "synthetic password",
            email: "",
        })
        .await
        .unwrap();
    let bundle = browser
        .login(
            "legacy_sender",
            "synthetic password",
            &BrowserSessionMetadata::default(),
        )
        .await
        .unwrap();
    let sender = identity
        .authorize(bundle.data["access_token"].as_str().unwrap())
        .await
        .unwrap();
    let recipient_session = identity.bootstrap_user(2, 0).await.unwrap();
    let recipient = identity.authorize(&recipient_session.secret).await.unwrap();
    // The cutover contract preserves user IDs as personal account IDs.
    assert_eq!(sender.user_id, sender.owner_account_id);
    assert_eq!(recipient.user_id, recipient.owner_account_id);
    let users: Vec<Value> = [(sender.user_id,1000), (recipient.user_id,0)].into_iter().map(|(id,quota)| json!({"id":id,"quota":quota,"used_quota":0,"request_count":0,"aff_quota":0,"aff_history":0})).collect();
    let transfers: Vec<Value> = [(1,300,"a"),(2,200,"b"),(3,100,"c")].into_iter().map(|(id,quota,token)| json!({"id":id,"sender_id":sender.user_id,"quota":quota,"status":"pending","token":token.repeat(64),"request_key":format!("legacy-transfer-{id:04}"),"created_at":1900000000,"recipient_id":0,"claimed_at":0,"cancelled_at":0})).collect();
    let mut data = json!({"users":users,"wallet_transfers":transfers,"top_ups_pending":[]});
    data["wallet_transfers"]
        .as_array_mut()
        .unwrap()
        .push(json!({
            "id":4,"sender_id":sender.user_id,"quota":50,"status":"cancelled",
            "created_at":1800000000,"cancelled_at":1800000010,"recipient_id":0,"claimed_at":0
        }));
    let archive = "a".repeat(64);
    apply(&pool, &render(&data, "opening", &archive))
        .await
        .unwrap();
    let plan = render(&data, "wallet", &archive);
    // An archive mismatch must not create either a transfer or money.
    assert!(
        apply(&pool, &render(&data, "wallet", &"b".repeat(64)))
            .await
            .is_err()
    );
    assert_eq!(
        balance(&pool, sender.owner_account_id, "wallet").await,
        1000
    );
    assert_eq!(balance(&pool, sender.owner_account_id, "reserved").await, 0);
    // Fail after one liability was posted: the entire batch must roll back.
    let mut duplicate = data.clone();
    duplicate["wallet_transfers"][1]["id"] = json!(1);
    assert!(
        apply(&pool, &render(&duplicate, "wallet", &archive))
            .await
            .is_err()
    );
    assert_eq!(
        balance(&pool, sender.owner_account_id, "wallet").await,
        1000
    );
    assert_eq!(balance(&pool, sender.owner_account_id, "reserved").await, 0);
    assert_eq!(
        sqlx::query_scalar::<_, i64>("SELECT count(*) FROM core_wallet.transfers")
            .fetch_one(&pool)
            .await
            .unwrap(),
        0
    );
    apply(&pool, &plan).await.unwrap();
    assert_eq!(
        balance(&pool, sender.owner_account_id, "wallet").await,
        1000
    );
    assert_eq!(
        balance(&pool, sender.owner_account_id, "reserved").await,
        600
    );
    // Reserve metadata must name the actual debit and credit legs.
    let valid: bool = sqlx::query_scalar("SELECT bool_and((SELECT delta_units FROM core_billing.ledger_entries WHERE journal_id=j.id AND ledger_account_id=j.payer_id) IS NOT DISTINCT FROM -j.amount_units AND (SELECT delta_units FROM core_billing.ledger_entries WHERE journal_id=j.id AND ledger_account_id=j.payee_id) IS NOT DISTINCT FROM j.amount_units) FROM core_billing.ledger_journals j WHERE j.scope='core-wallet-transfer' AND j.kind='reserve'").fetch_one(&pool).await.unwrap();
    assert!(valid);
    let store = Store::new(pool.clone());
    assert!(
        sqlx::query("UPDATE core_wallet.legacy_transfer_history SET amount=999 WHERE id=4")
            .execute(&pool)
            .await
            .is_err()
    );
    let runtime = runtime::RuntimeRole::create(&pool).await;
    let grants = include_str!("../../../deployment/postgres/grant-core-runtime.sql")
        .split_once("\\if :enable_wallet\n")
        .unwrap()
        .1
        .split_once("\\endif")
        .unwrap()
        .0
        .replace(":\"runtime_role\"", &runtime.name);
    sqlx::raw_sql(&grants).execute(&pool).await.unwrap();
    let rights: (bool,bool) = sqlx::query_as("SELECT has_table_privilege('core_wallet.legacy_transfer_history','SELECT'),has_table_privilege('core_wallet.legacy_transfer_history','INSERT,UPDATE,DELETE,TRUNCATE')")
        .fetch_one(&runtime.pool).await.unwrap();
    assert_eq!(rights, (true, false));
    let restricted = Store::new(runtime.pool.clone());
    let page = restricted
        .list(bundle.data["access_token"].as_str().unwrap(), 4)
        .await
        .unwrap();
    assert!(page.iter().all(|t| t.id < 4));
    assert_eq!(
        restricted
            .list(bundle.data["access_token"].as_str().unwrap(), 0)
            .await
            .unwrap()
            .len(),
        4
    );
    assert!(
        restricted
            .list(&recipient_session.secret, 0)
            .await
            .unwrap()
            .is_empty()
    );
    let history = store
        .list(bundle.data["access_token"].as_str().unwrap(), 0)
        .await
        .unwrap();
    assert!(
        history
            .iter()
            .any(|t| t.id == 4 && t.state == "cancelled" && t.amount_credits == 50)
    );
    assert!(
        store
            .list(&recipient_session.secret, 0)
            .await
            .unwrap()
            .is_empty()
    );

    assert!(
        store
            .claim(
                bundle.data["access_token"].as_str().unwrap(),
                &"a".repeat(64)
            )
            .await
            .is_err()
    );
    store
        .claim(&recipient_session.secret, &"a".repeat(64))
        .await
        .unwrap();
    store
        .claim(&recipient_session.secret, &"a".repeat(64))
        .await
        .unwrap();
    assert_eq!(
        balance(&pool, recipient.owner_account_id, "wallet").await,
        300
    );
    let (_, csrf) = store
        .confirmation_page(&bundle.refresh_token, 2, "cancel")
        .await
        .unwrap();
    store
        .confirm(&bundle.refresh_token, 2, &csrf, "cancel")
        .await
        .unwrap();
    store
        .confirm(&bundle.refresh_token, 2, &csrf, "cancel")
        .await
        .unwrap();
    assert_eq!(
        balance(&pool, sender.owner_account_id, "wallet").await,
        1200
    );
    assert!(
        store
            .claim(&recipient_session.secret, &"b".repeat(64))
            .await
            .is_err()
    );
    let (_, csrf) = store
        .confirmation_page(&bundle.refresh_token, 3, "cancel")
        .await
        .unwrap();
    let token = "c".repeat(64);
    let (claim, cancel) = tokio::join!(
        store.claim(&recipient_session.secret, &token),
        store.confirm(&bundle.refresh_token, 3, &csrf, "cancel")
    );
    assert_ne!(claim.is_ok(), cancel.is_ok());
    assert_eq!(balance(&pool, sender.owner_account_id, "reserved").await, 0);
    assert_eq!(
        balance(&pool, sender.owner_account_id, "wallet").await
            + balance(&pool, recipient.owner_account_id, "wallet").await,
        1600
    );
    let journals: i64 = sqlx::query_scalar("SELECT count(*) FROM core_billing.ledger_journals")
        .fetch_one(&pool)
        .await
        .unwrap();
    assert!(apply(&pool, &plan).await.is_err());
    assert_eq!(
        sqlx::query_scalar::<_, i64>("SELECT count(*) FROM core_billing.ledger_journals")
            .fetch_one(&pool)
            .await
            .unwrap(),
        journals
    );
    let conserved: bool =
        sqlx::query_scalar("SELECT sum(balance_units)=0 FROM core_billing.balance_state")
            .fetch_one(&pool)
            .await
            .unwrap();
    assert!(conserved);
    runtime.cleanup().await;
}

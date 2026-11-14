//! Synthetic legacy codes through the real Python importer and Core ledger.
use lmm_core::{
    billing::ledger::Ledger,
    core_schema,
    identity::IdentityStore,
    promotions::{PromotionStore, RedeemRequest},
};
use serde_json::{Value, json};
use sqlx::PgPool;
use std::{
    io::Write,
    process::{Command, Stdio},
};
fn render(data: &Value, phase: &str, archive: &str) -> String {
    let mut child = Command::new("python3")
        .arg("-B").arg("-c")
        .arg("import json,sys; import legacy_cutover_import as i; import legacy_cutover_money as m; d=json.load(sys.stdin); f=m.plan_promotions; print('\\n'.join(f(d,sys.argv[2],'wallet-cutover-test',2000000000)[0]))")
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
    sqlx::raw_sql(sqlx::AssertSqlSafe::<&str>(sql))
        .execute(&mut *tx)
        .await?;
    tx.commit().await
}

#[sqlx::test(migrations = false)]
async fn imported_codes_redeem_once_and_used_codes_cannot_replay(pool: PgPool) {
    core_schema::init(&pool).await.unwrap();
    let identity = IdentityStore::from_pool(pool.clone());
    identity.bootstrap_user(1, 6).await.unwrap();
    let user = identity.bootstrap_user(2, 0).await.unwrap().secret;
    let other = identity.bootstrap_user(3, 0).await.unwrap().secret;
    let account = identity.authorize(&user).await.unwrap().owner_account_id;
    let ledger = Ledger::from_pool(pool.clone());
    ledger.open_wallet(account).await.unwrap();
    let code = "a".repeat(32);
    let used = "b".repeat(32);
    let mut data = json!({"users":[{"id":1,"role":100,"deleted_at":null}],
        "options":{}, "redemptions":[], "checkin_recent":[{"n":0,"total":0}],
        "checkin_total":[{"n":0}],"discount_codes":[],"discount_code_reservations":[],
        "red_packets":[],"red_packet_items":[],"red_packet_claims":[{"n":0}],
        "referral_rewards":[],"other_counts":[]});
    data["redemptions"] = json!([
        {"id":1,"user_id":1,"key":code,"status":1,"name":"Synthetic","quota":100,
            "created_time":1,"expired_time":0,"deleted_at":null,"reward_type":"quota"},
        {"id":2,"user_id":1,"key":used,"status":3,"name":"Synthetic","quota":100,
            "created_time":1,"expired_time":0,"deleted_at":null,"reward_type":"quota"}]);
    apply(&pool, &render(&data, "promotions", &"a".repeat(64)))
        .await
        .unwrap();
    assert_eq!(ledger.balance(account).await.unwrap().available_units, 0);
    let store = PromotionStore::new(pool.clone());
    let request = || RedeemRequest {
        code: code.clone(),
        operation_key: "legacy_redeem_once".into(),
    };
    let receipt = store.redeem(&user, request()).await.unwrap();
    assert_eq!(receipt.amount_units, "100");
    assert_eq!(store.redeem(&user, request()).await.unwrap(), receipt);
    assert!(
        store
            .redeem(
                &other,
                RedeemRequest {
                    code: code.clone(),
                    operation_key: "other_redeem_once".into()
                }
            )
            .await
            .is_err()
    );
    assert!(
        store
            .redeem(
                &user,
                RedeemRequest {
                    code: used,
                    operation_key: "used_redeem_once_".into()
                }
            )
            .await
            .is_err()
    );
    assert_eq!(ledger.balance(account).await.unwrap().available_units, 100);
    let conserved: bool =
        sqlx::query_scalar("SELECT sum(balance_units)=0 FROM core_billing.balance_state")
            .fetch_one(&pool)
            .await
            .unwrap();
    assert!(conserved);
}

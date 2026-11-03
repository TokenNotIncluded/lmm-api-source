//! First-payment history must be source-bound and must preserve unknown facts.
use lmm_core::{
    community_read::{CommunityRead, UsersInfoRequest},
    core_schema,
    identity::IdentityStore,
};
use serde_json::json;
use sqlx::PgPool;

#[sqlx::test(migrations = false)]
async fn cash_history_uses_classification_mapping_and_complete_coverage(pool: PgPool) {
    core_schema::init(&pool).await.unwrap();
    let identity = IdentityStore::from_pool(pool.clone());
    let mut owners = Vec::new();
    for uid in [71, 72] {
        let key = identity.bootstrap_user(uid, 0).await.unwrap();
        let account = identity
            .authorize(&key.secret)
            .await
            .unwrap()
            .owner_account_id;
        let base = json!({"migration_id":"cash-history-opening","source_entity_key":format!("user:{uid}"),
    "source_archive_sha256":"a".repeat(64),"mapping_sha256":"b".repeat(64),"source_row_sha256":"c".repeat(64),
    "observed_at":1793674367_i64,"account_id":account});
        let mut opening = base.clone();
        let obj = opening.as_object_mut().unwrap();
        obj.insert("available_units".into(), json!(0));
        obj.insert("reserved_units".into(), json!(0));
        obj.insert("affiliate_units".into(), json!(0));
        let mut counters = base;
        let obj = counters.as_object_mut().unwrap();
        obj.insert("user_id".into(), json!(uid));
        obj.insert("used_quota".into(), json!(0));
        obj.insert("request_count".into(), json!(0));
        obj.insert("aff_history".into(), json!(0));
        let mut tx = pool.begin().await.unwrap();
        sqlx::query("SET TRANSACTION ISOLATION LEVEL SERIALIZABLE")
            .execute(&mut *tx)
            .await
            .unwrap();
        sqlx::query("SET LOCAL synchronous_commit=on")
            .execute(&mut *tx)
            .await
            .unwrap();
        sqlx::query("SELECT core_billing.import_opening_balance($1)")
            .bind(opening)
            .execute(&mut *tx)
            .await
            .unwrap();
        sqlx::query("SELECT core_billing.import_account_opening_counters($1)")
            .bind(counters)
            .execute(&mut *tx)
            .await
            .unwrap();
        tx.commit().await.unwrap();
        owners.push(account);
    }
    let read = CommunityRead::new(pool.clone()).await.unwrap();
    let request = UsersInfoRequest {
        user_ids: vec![71, 72],
    };
    assert!(
        read.first_cash_payments(&request)
            .await
            .unwrap()
            .users
            .iter()
            .all(|u| !u.history_complete && u.first_paid_at.is_none())
    );
    sqlx::query("INSERT INTO core_history.finance_sources(source_key,contract,source_archive_sha256,mapping_sha256,tables_sha256,normalization_sha256,rows_sha256,observed_at,user_count,topup_count,credits_per_usd,complete,native_request_ids) VALUES(repeat('d',64),'core.site-finance-source.v1',repeat('a',64),repeat('b',64),repeat('c',64),repeat('e',64),repeat('f',64),1793674367,2,4,500000,true,'{}')").execute(&pool).await.unwrap();
    for (uid, account) in [(71, owners[0]), (72, owners[1])] {
        sqlx::query("INSERT INTO core_history.finance_users(user_id,account_id,source_key,source_row_sha256,raw_used_quota,normalized_used_credits) VALUES($1,$2,repeat('d',64),repeat('c',64),0,0)").bind(uid).bind(account).execute(&pool).await.unwrap();
    }
    sqlx::query("INSERT INTO core_history.finance_topups(source_key,source_order_id,source_row_sha256,classification,currency,gross_amount_micros,refunded_amount_micros) VALUES(repeat('d',64),1,repeat('c',64),'cash','USD',1000000,0),(repeat('d',64),2,repeat('c',64),'virtual','LDC',1000000,0),(repeat('d',64),3,repeat('c',64),'unconfirmed',NULL,NULL,NULL),(repeat('d',64),4,repeat('c',64),'excluded',NULL,NULL,NULL)").execute(&pool).await.unwrap();
    let rows:Vec<_>=[(1,71,owners[0],1700000000_i64),(2,71,owners[0],1600000000),(3,72,owners[1],1500000000),(4,72,owners[1],1400000000)].into_iter().map(|(id,user,account,at)|json!({"target_user_id":user,"target_personal_account_id":account,"record":{"id":id,"user_id":user,"complete_time":at,"status":"success"}})).collect();
    let mut manifest = json!({"scope":"wallet-topup-history.v1","source_archive_sha256":"a".repeat(64),"complete":true,"source_row_count":4,"rows":rows,
 "identity_map":[{"source_user_id":171,"target_user_id":71,"target_personal_account_id":owners[0]},{"source_user_id":172,"target_user_id":72,"target_personal_account_id":owners[1]}],
 "per_source_user_counts":{"171":2,"172":2}});
    // Wrong archive metadata cannot make an imported cohort appear complete.
    manifest["source_archive_sha256"] = json!("b".repeat(64));
    sqlx::query("INSERT INTO core_migration.import_runs(migration_id,source_archive_sha256,mapping_sha256,manifest) VALUES('wrong-cash-history',repeat('b',64),repeat('b',64),$1)").bind(&manifest).execute(&pool).await.unwrap();
    assert!(
        read.first_cash_payments(&request)
            .await
            .unwrap()
            .users
            .iter()
            .all(|u| !u.history_complete && u.first_paid_at.is_none())
    );
    manifest["source_archive_sha256"] = json!("a".repeat(64));
    sqlx::query("INSERT INTO core_migration.import_runs(migration_id,source_archive_sha256,mapping_sha256,manifest) VALUES('cash-history',repeat('a',64),repeat('b',64),$1)").bind(&manifest).execute(&pool).await.unwrap();
    let result = read.first_cash_payments(&request).await.unwrap();
    assert!(result.users[0].history_complete);
    assert_eq!(
        result.users[0].first_paid_at,
        Some(1700000000),
        "earlier virtual credits do not count as cash"
    );
    assert_eq!(result.users[0].unclassified_orders, 0);
    assert!(!result.users[1].history_complete);
    assert_eq!(result.users[1].unclassified_orders, 1);
    assert!(
        result.users[1].first_paid_at.is_none(),
        "unconfirmed old money is not zero or a first cash date"
    );
}

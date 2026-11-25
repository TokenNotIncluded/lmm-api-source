//! Native console gift contract with production identity/ledger functions on a disposable database.
use axum::{
    body::{Body, to_bytes},
    http::Request,
};
use lmm_core::{
    billing::{
        Credits,
        ledger::{Action, Ledger, Outcome, Request as Entry},
    },
    identity::IdentityStore,
    promotions,
};
use serde_json::{Value, json};
use sqlx::PgPool;
use tower::ServiceExt;
#[path = "common/runtime.rs"]
mod runtime;

async fn http(pool: &PgPool, method: &str, path: &str, secret: Option<&str>) -> (u16, Value) {
    let mut req = Request::builder().method(method).uri(path);
    if let Some(s) = secret {
        req = req.header("authorization", format!("Bearer {s}"));
    }
    let res = promotions::router(pool.clone())
        .oneshot(req.body(Body::empty()).unwrap())
        .await
        .unwrap();
    let code = res.status().as_u16();
    let bytes = to_bytes(res.into_body(), 65536).await.unwrap();
    (code, serde_json::from_slice(&bytes).unwrap())
}

#[sqlx::test]
async fn compensation_gifts_require_real_funding_and_return_the_same_receipt(pool: PgPool) {
    lmm_core::core_schema::init(&pool).await.unwrap();
    let identity = IdentityStore::from_pool(pool.clone());
    let root = identity.bootstrap_user(8400, 6).await.unwrap().secret;
    let one = identity.bootstrap_user(8401, 0).await.unwrap().secret;
    let two = identity.bootstrap_user(8402, 0).await.unwrap().secret;
    let treasury = identity.authorize(&root).await.unwrap().owner_account_id;
    let recipient = identity.authorize(&one).await.unwrap().owner_account_id;
    let ledger = Ledger::from_pool(pool.clone());
    ledger.open_wallet(treasury).await.unwrap();
    let runtime = runtime::RuntimeRole::create(&pool).await;
    runtime.optional("core_console").await;
    runtime.optional("core_charging").await;
    let web = &runtime.pool;
    assert_eq!(http(web, "GET", "/api/user/gift", None).await.0, 401);
    assert_eq!(
        http(web, "GET", "/api/user/gift", Some(&one)).await,
        (200, json!({"success":true,"data":[]}))
    );
    let t: i64 = sqlx::query_scalar("SELECT floor(extract(epoch FROM clock_timestamp()))::bigint")
        .fetch_one(&pool)
        .await
        .unwrap();
    let active = json!({"id":31,"title":"Maintenance credit","description":"","quota":10,"budget_quota":10,"max_claims":1,
        "start_at":t-600,"end_at":t+600,"min_used_quota":0,"min_account_age_days":0,"enabled":true,"created_at":t-1200});
    let mut age = active.clone();
    age["id"] = json!(32);
    age["min_account_age_days"] = json!(7);
    let mut expired = active.clone();
    expired["id"] = json!(33);
    expired["end_at"] = json!(t - 5);
    let mut future = active.clone();
    future["id"] = json!(34);
    future["start_at"] = json!(t + 100);
    let mut inactive = active.clone();
    inactive["id"] = json!(35);
    inactive["enabled"] = json!(false);
    let mut spend = active.clone();
    spend["id"] = json!(36);
    spend["min_used_quota"] = json!(2);
    sqlx::query("INSERT INTO core_console.options(key,value) VALUES ('compensation_gifts.v1',$1)")
        .bind(json!({"treasury_account_id":treasury,"campaigns":[active,age,expired,future,inactive,spend]}).to_string()).execute(&pool).await.unwrap();
    let path = "/api/user/gift/31/claim";
    for id in [32, 33, 34, 36] {
        assert_eq!(
            http(
                web,
                "POST",
                &format!("/api/user/gift/{id}/claim"),
                Some(&one)
            )
            .await
            .0,
            403
        );
    }
    assert_eq!(
        http(web, "POST", "/api/user/gift/35/claim", Some(&one))
            .await
            .0,
        404
    );
    assert_eq!(http(web, "POST", path, Some(&one)).await.0, 409);
    // Synthetic fixture funds the operator wallet. The endpoint must debit this
    // wallet by transfer; it cannot mint another credit to cover a shortfall.
    let funding = ledger
        .execute(&Entry {
            scope: "synthetic-test".into(),
            operation_key: "seed".into(),
            actor_user_id: Some(8400),
            reason: "Synthetic treasury funding".into(),
            action: Action::Gift {
                account_id: treasury,
                amount_units: Credits::try_from(15).unwrap(),
            },
        })
        .await
        .unwrap();
    assert!(matches!(funding, Outcome::Posted { .. }));
    let (a, b) = tokio::join!(
        http(web, "POST", path, Some(&one)),
        http(web, "POST", path, Some(&one))
    );
    assert_eq!((a.0, b.0), (200, 200), "{a:?} {b:?}");
    assert_ne!(
        a.1["data"]["already_claimed"],
        b.1["data"]["already_claimed"]
    );
    assert_eq!(a.1["data"]["claim"], b.1["data"]["claim"]);
    assert_eq!(a.1["data"]["claim"]["user_id"], 8401);
    assert_eq!(a.1["data"]["claim"]["gift_id"], 31);
    assert_eq!(a.1["data"]["claim"]["quota"], 10);
    assert_eq!(ledger.balance(recipient).await.unwrap().available_units, 10);
    assert_eq!(ledger.balance(treasury).await.unwrap().available_units, 5);
    assert_eq!(http(web, "POST", path, Some(&two)).await.0, 409);
    let (code, list) = http(web, "GET", "/api/user/gift", Some(&one)).await;
    assert_eq!(code, 200, "{list}");
    let rows = list["data"].as_array().unwrap();
    assert_eq!(rows.len(), 5);
    assert!(
        rows.iter().find(|g| g["id"] == 31).unwrap()["claimed"]
            .as_bool()
            .unwrap()
    );
    let effects:i64=sqlx::query_scalar("SELECT count(*) FROM core_billing.ledger_journals WHERE scope='core-compensation-gift' AND kind='transfer'").fetch_one(&pool).await.unwrap();
    assert_eq!(effects, 1);
    sqlx::query(
        "UPDATE core_console.options SET value='malformed' WHERE key='compensation_gifts.v1'",
    )
    .execute(&pool)
    .await
    .unwrap();
    assert_eq!(http(web, "GET", "/api/user/gift", Some(&one)).await.0, 503);
    assert_eq!(
        http(web, "POST", path, Some(&one)).await.1["data"]["claim"],
        a.1["data"]["claim"]
    );
    runtime.cleanup().await;
}

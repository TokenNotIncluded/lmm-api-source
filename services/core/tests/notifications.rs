//! Real PostgreSQL notification publication, account isolation and wallet facts.
use axum::{
    Router,
    body::{Body, to_bytes},
    http::{Request, StatusCode},
};
use lmm_core::{
    console_compat::{self, ConsoleCompat},
    identity::{
        IdentityStore,
        admin_users::{AdjustmentRequest, AdminUsersStore, SecurityAction, SecurityRequest},
    },
};
use serde_json::{Value, json};
use sqlx::PgPool;
use tower::ServiceExt;
#[path = "common/runtime.rs"]
mod runtime;

async fn call(
    app: &Router,
    method: &str,
    path: &str,
    secret: &str,
    body: Value,
) -> (StatusCode, Value) {
    let response = app
        .clone()
        .oneshot(
            Request::builder()
                .method(method)
                .uri(path)
                .header("authorization", format!("Bearer {secret}"))
                .header("content-type", "application/json")
                .body(Body::from(body.to_string()))
                .unwrap(),
        )
        .await
        .unwrap();
    let status = response.status();
    let bytes = to_bytes(response.into_body(), 1 << 20).await.unwrap();
    (
        status,
        serde_json::from_slice(&bytes).unwrap_or(Value::Null),
    )
}
async fn get(app: &Router, path: &str, secret: &str) -> Value {
    let (status, body) = call(app, "GET", path, secret, Value::Null).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    body
}
fn prefs(revision: i64, enabled: bool, threshold: &str) -> Value {
    json!({"revision":revision,"enabled":enabled,"low_balance_units":threshold})
}

#[sqlx::test(migrations = false)]
async fn low_balance_is_persistent_scoped_and_rearmed_by_committed_wallet(pool: PgPool) {
    lmm_core::core_schema::init(&pool).await.unwrap();
    let owner = IdentityStore::from_pool(pool.clone());
    let admin = owner.bootstrap_user(900, 5).await.unwrap();
    let alice = owner.bootstrap_user(10, 0).await.unwrap();
    let bob = owner.bootstrap_user(11, 0).await.unwrap();
    let runtime = runtime::RuntimeRole::create(&pool).await;
    runtime.optional("core_console").await;
    let state = ConsoleCompat::new(
        runtime.pool.clone(),
        IdentityStore::from_pool(runtime.pool.clone()),
    );
    let app = console_compat::router(state.clone());
    let preferences = "/core/v1/notifications/preferences";
    let todos = "/api/todos?category=all&p=1&page_size=50";
    assert_eq!(
        get(&app, preferences, &alice.secret).await["data"]["revision"],
        0
    );
    assert_eq!(
        call(
            &app,
            "PUT",
            preferences,
            &alice.secret,
            prefs(0, true, "100")
        )
        .await
        .0,
        StatusCode::OK
    );
    assert_eq!(
        call(
            &app,
            "PUT",
            preferences,
            &alice.secret,
            prefs(0, true, "200")
        )
        .await
        .0,
        StatusCode::CONFLICT
    );
    assert_eq!(
        call(
            &app,
            "PUT",
            preferences,
            &alice.secret,
            prefs(1, true, "1e3")
        )
        .await
        .0,
        StatusCode::BAD_REQUEST
    );
    state.notification_sweep(0).await.unwrap();
    let first = get(&app, todos, &alice.secret).await;
    assert_eq!(first["data"]["total"], 1);
    assert_eq!(first["data"]["items"][0]["type"], "low_balance");
    assert_eq!(first["data"]["items"][0]["details"]["balance_units"], "0");
    let id = first["data"]["items"][0]["source_id"].as_i64().unwrap();
    assert_eq!(get(&app, todos, &alice.secret).await["data"]["total"], 1);
    assert_eq!(get(&app, todos, &bob.secret).await["data"]["total"], 0);
    let read = json!({"category":"all","ids":[id],"all":false});
    assert_eq!(
        call(&app, "POST", "/api/todos/read", &bob.secret, read.clone())
            .await
            .1["data"]["marked"],
        0
    );
    assert_eq!(
        call(&app, "POST", "/api/todos/read", &alice.secret, read)
            .await
            .1["data"]["marked"],
        1
    );
    assert_eq!(
        get(&app, todos, &alice.secret).await["data"]["total_unread_count"],
        0
    );
    // Saving the same threshold does not manufacture a new low-balance episode.
    assert_eq!(
        call(
            &app,
            "PUT",
            preferences,
            &alice.secret,
            prefs(1, true, "100")
        )
        .await
        .0,
        StatusCode::OK
    );
    assert_eq!(get(&app, todos, &alice.secret).await["data"]["total"], 1);
    let manager = AdminUsersStore::new(IdentityStore::from_pool(runtime.pool.clone()));
    for (index, delta) in [(1, "200"), (2, "-150")] {
        manager
            .adjust_user_balance(
                &admin.secret,
                10,
                &AdjustmentRequest {
                    operation_key: format!("synthetic-notification-adjustment-{index:04}"),
                    delta_units: delta.into(),
                    reason: "Synthetic notification balance transition".into(),
                },
            )
            .await
            .unwrap();
        state.notification_sweep(0).await.unwrap();
    }
    let restarted = console_compat::router(ConsoleCompat::new(
        runtime.pool.clone(),
        IdentityStore::from_pool(runtime.pool.clone()),
    ));
    let second = get(&restarted, todos, &alice.secret).await;
    assert_eq!(second["data"]["total"], 2);
    assert_eq!(second["data"]["total_unread_count"], 1);
    assert_eq!(second["data"]["items"][0]["details"]["balance_units"], "50");
    let journals: i64 = sqlx::query_scalar("SELECT count(*) FROM core_billing.ledger_journals")
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(
        journals, 2,
        "reading and delivering reminders must not move money"
    );
    assert!(
        sqlx::query("DELETE FROM core_notifications.inbox")
            .execute(&runtime.pool)
            .await
            .is_err()
    );
    assert!(
        sqlx::query("UPDATE core_notifications.inbox SET title='changed'")
            .execute(&runtime.pool)
            .await
            .is_err()
    );
    runtime.cleanup().await;
}

fn document(version: i64, input: i64) -> (Value, Value) {
    let rules:Vec<Value>=[("default","public-model"),("private-group","private-model")].into_iter().enumerate().map(|(id,(group,model))| json!({
        "id":id+1,"model":model,"group":group,"speed":"standard",
        "rates":{"input":if id==0 {input}else{12},"output":4,"cache_read":0,"cache_write":0,"request":0},
        "group_multiplier_ppm":1000000,"speed_multiplier_ppm":1000000
    })).collect();
    (
        json!({"version":version,"groups":[],"upstreams":[],"routes":[],"aliases":[]}),
        json!({"version":version,"rules":rules}),
    )
}
async fn publish(c: &mut sqlx::PgConnection, version: i64, input: i64, previous: Option<i64>) {
    let (routing, prices) = document(version, input);
    sqlx::query("SELECT public.routing_control_publish($1,$2,$3,$3)")
        .bind(routing)
        .bind(prices)
        .bind(previous)
        .execute(c)
        .await
        .unwrap();
}

#[sqlx::test(migrations = false)]
async fn price_events_come_from_committed_publications_and_current_group_scope(pool: PgPool) {
    lmm_core::core_schema::init(&pool).await.unwrap();
    let owner = IdentityStore::from_pool(pool.clone());
    let alice = owner.bootstrap_user(10, 0).await.unwrap();
    sqlx::query("INSERT INTO core_identity.user_routing_groups(user_id,group_name,enabled,source_reference) VALUES(10,'default',true,'synthetic-price-notification-fixture') ON CONFLICT(user_id,group_name) DO UPDATE SET enabled=true").execute(&pool).await.unwrap();
    let runtime = runtime::RuntimeRole::create(&pool).await;
    runtime.optional("core_console").await;
    let app = console_compat::router(ConsoleCompat::new(
        runtime.pool.clone(),
        IdentityStore::from_pool(runtime.pool.clone()),
    ));
    assert_eq!(
        get(&app, "/api/ratio-notifications", &alice.secret).await["data"],
        json!([])
    );
    publish(&mut pool.acquire().await.unwrap(), 1, 2, None).await;
    let first = get(&app, "/api/ratio-notifications", &alice.secret).await;
    assert_eq!(first["data"].as_array().unwrap().len(), 1);
    assert_eq!(first["data"][0]["changes"][0]["model"], "public-model");
    assert_eq!(first["data"][0]["changes"][0]["new"]["rates"]["input"], "2");
    let mut aborted = pool.begin().await.unwrap();
    publish(&mut aborted, 2, 6, Some(1)).await;
    aborted.rollback().await.unwrap();
    assert_eq!(
        get(&app, "/api/ratio-notifications", &alice.secret).await["data"]
            .as_array()
            .unwrap()
            .len(),
        1
    );
    publish(&mut pool.acquire().await.unwrap(), 2, 8, Some(1)).await;
    let events = get(&app, "/api/ratio-notifications", &alice.secret).await;
    assert_eq!(events["data"].as_array().unwrap().len(), 2);
    assert_eq!(
        events["data"][0]["changes"][0]["old"]["rates"]["input"],
        "2"
    );
    assert_eq!(
        events["data"][0]["changes"][0]["new"]["rates"]["input"],
        "8"
    );
    assert!(!events.to_string().contains("private-model"));
    // Decimal/tiered prices keep scalar rates at zero. A change to the actual
    // tier must still be published, with exact decimal strings in the Web DTO.
    use lmm_core::routing::{
        DecimalRate, PriceConfig, PriceSnapshot, TieredRates, TieredTariff, TokenTier,
    };
    for (version, numerator) in [(3, 5), (4, 7)] {
        let (routing, mut prices) = document(version, 0);
        prices["rules"][0]["billing_mode"] = json!("tiered_decimal");
        prices["rules"][0]["rates"] =
            json!({"input":0,"output":0,"cache_read":0,"cache_write":0,"request":0});
        prices["rules"][0]["tiered"] = serde_json::to_value(TieredTariff {
            revision: 1,
            source_digest: "23".repeat(32),
            uses_cache_read: false,
            uses_cache_write: false,
            uses_cache_write_1h: false,
            uses_cache_write_12h: false,
            tiers: vec![TokenTier {
                up_to_input_tokens: None,
                rates: TieredRates {
                    input: DecimalRate {
                        numerator,
                        denominator: 4,
                    },
                    output: DecimalRate {
                        numerator: 10,
                        denominator: 1,
                    },
                    cache_read: DecimalRate {
                        numerator: 0,
                        denominator: 1,
                    },
                    cache_write: DecimalRate {
                        numerator: 0,
                        denominator: 1,
                    },
                    cache_write_1h: None,
                    cache_write_12h: None,
                },
            }],
        })
        .unwrap();
        PriceSnapshot::compile(serde_json::from_value::<PriceConfig>(prices.clone()).unwrap())
            .unwrap();
        sqlx::query("SELECT public.routing_control_publish($1,$2,$3,$3)")
            .bind(routing)
            .bind(prices)
            .bind(version - 1)
            .execute(&pool)
            .await
            .unwrap();
    }
    let tiered = get(&app, "/api/ratio-notifications", &alice.secret).await;
    assert_eq!(tiered["data"].as_array().unwrap().len(), 4);
    assert_eq!(
        tiered["data"][0]["changes"][0]["old"]["tiered"]["tiers"][0]["rates"]["input"]["numerator"],
        "5"
    );
    assert_eq!(
        tiered["data"][0]["changes"][0]["new"]["tiered"]["tiers"][0]["rates"]["input"]["numerator"],
        "7"
    );
    assert!(
        sqlx::query("UPDATE core_notifications.price_events SET model='changed'")
            .execute(&runtime.pool)
            .await
            .is_err()
    );
    assert_eq!(
        call(
            &app,
            "GET",
            "/api/ratio-notifications?before=-1",
            &alice.secret,
            Value::Null
        )
        .await
        .0,
        StatusCode::BAD_REQUEST
    );
    runtime.cleanup().await;
}

#[sqlx::test(migrations = false)]
async fn one_failed_account_does_not_starve_later_notifications(pool: PgPool) {
    lmm_core::core_schema::init(&pool).await.unwrap();
    let owner = IdentityStore::from_pool(pool.clone());
    owner.bootstrap_user(10, 0).await.unwrap();
    owner.bootstrap_user(11, 0).await.unwrap();
    sqlx::query("INSERT INTO core_notifications.preferences(user_id,low_balance_units) VALUES(10,100),(11,100)").execute(&pool).await.unwrap();
    sqlx::raw_sql("CREATE FUNCTION public.synthetic_notification_failure() RETURNS trigger LANGUAGE plpgsql AS $$ BEGIN IF NEW.user_id=10 THEN RAISE EXCEPTION 'synthetic account failure'; END IF; RETURN NEW; END $$; CREATE TRIGGER synthetic_notification_failure BEFORE INSERT ON core_notifications.inbox FOR EACH ROW EXECUTE FUNCTION public.synthetic_notification_failure();").execute(&pool).await.unwrap();
    let runtime = runtime::RuntimeRole::create(&pool).await;
    runtime.optional("core_console").await;
    let state = ConsoleCompat::new(
        runtime.pool.clone(),
        IdentityStore::from_pool(runtime.pool.clone()),
    );
    let pass = state.notification_sweep(0).await.unwrap();
    assert_eq!(pass.failed_accounts, 1);
    assert_eq!(pass.next_user, None);
    let users: Vec<i64> =
        sqlx::query_scalar("SELECT user_id FROM core_notifications.inbox ORDER BY user_id")
            .fetch_all(&pool)
            .await
            .unwrap();
    assert_eq!(users, vec![11]);
    sqlx::query("DROP TRIGGER synthetic_notification_failure ON core_notifications.inbox")
        .execute(&pool)
        .await
        .unwrap();
    assert_eq!(
        state.notification_sweep(0).await.unwrap().failed_accounts,
        0
    );
    let users: Vec<i64> =
        sqlx::query_scalar("SELECT user_id FROM core_notifications.inbox ORDER BY user_id")
            .fetch_all(&pool)
            .await
            .unwrap();
    assert_eq!(users, vec![10, 11]);
    runtime.cleanup().await;
}

#[sqlx::test(migrations = false)]
async fn account_security_notifications_share_the_audit_transaction(pool: PgPool) {
    lmm_core::core_schema::init(&pool).await.unwrap();
    let owner = IdentityStore::from_pool(pool.clone());
    let admin = owner.bootstrap_user(900, 5).await.unwrap();
    owner.bootstrap_user(10, 0).await.unwrap();
    let runtime = runtime::RuntimeRole::create(&pool).await;
    runtime.optional("core_console").await;
    let manager = AdminUsersStore::new(IdentityStore::from_pool(runtime.pool.clone()));
    let before = manager.user_security(&admin.secret, 10).await.unwrap();
    manager
        .update_user_security(
            &admin.secret,
            10,
            &SecurityRequest {
                expected_auth_version: before.auth_version,
                action: SecurityAction::RevokeCredentials,
                reason: "Synthetic authorized security reset".into(),
            },
        )
        .await
        .unwrap();
    let notification: (String, String) =
        sqlx::query_as("SELECT category,kind FROM core_notifications.inbox WHERE user_id=10")
            .fetch_one(&pool)
            .await
            .unwrap();
    assert_eq!(
        notification,
        (
            "security_incident".into(),
            "admin.user.credentials.revoke".into()
        )
    );
    let leaked:i64=sqlx::query_scalar("SELECT count(*) FROM core_notifications.inbox WHERE details::text LIKE '%Synthetic authorized%' OR summary LIKE '%Synthetic authorized%'").fetch_one(&pool).await.unwrap();
    assert_eq!(leaked, 0);
    runtime.cleanup().await;
}

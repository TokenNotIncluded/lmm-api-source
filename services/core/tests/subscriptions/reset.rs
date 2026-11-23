//! Synthetic owners, real PostgreSQL and Charging + TransactionalLedger. These
//! cases exercise reset rights; they are not approval to import source funds.
use super::*;
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use subscriptions::{
    ResetCodeRequest, ResetExecuteRequest, ResetFilter, ResetPreviewRequest, ResetTarget,
};

fn digest(code: &str) -> Vec<u8> {
    let mut h = Sha256::new();
    h.update(b"core.subscription-reset-code.v1\0");
    h.update(code.as_bytes());
    h.finalize().to_vec()
}
fn code_request(code: &str, key: &str) -> ResetCodeRequest {
    ResetCodeRequest {
        code: code.into(),
        operation_key: key.into(),
    }
}
fn preview(plan: i64, mode: &str, expiry: Option<i64>) -> ResetPreviewRequest {
    ResetPreviewRequest {
        mode: mode.into(),
        targets: vec![ResetTarget {
            user_id: 8302,
            plan_id: plan,
        }],
        all_matching: false,
        filter: None,
        voucher_expires_at: expiry,
    }
}
fn execute(token: &str, id: &str) -> ResetExecuteRequest {
    ResetExecuteRequest {
        operation_id: id.into(),
        preview_token: token.into(),
    }
}
async fn time(pool: &PgPool) -> i64 {
    sqlx::query_scalar("SELECT floor(extract(epoch FROM clock_timestamp()))::bigint")
        .fetch_one(pool)
        .await
        .unwrap()
}
async fn seed_code(pool: &PgPool, plan: i64, code: &str, expiry: i64, voucher_expiry: i64) {
    sqlx::query("INSERT INTO core_subscriptions.reset_codes(digest,plan_id,issuer_user_id,source_reference,expires_at,voucher_expires_at,enabled) VALUES($1,$2,8301,$3,$4,$5,true)")
        .bind(digest(code)).bind(plan).bind(format!("synthetic-code-{}",&code[..8])).bind(expiry).bind(voucher_expiry).execute(pool).await.unwrap();
}
async fn money(pool: &PgPool) -> String {
    sqlx::query_scalar(r#"SELECT encode(sha256(convert_to(jsonb_build_object(
        'journals',(SELECT coalesce(jsonb_agg(to_jsonb(t) ORDER BY id),'[]') FROM core_billing.ledger_journals t),
        'entries',(SELECT coalesce(jsonb_agg(to_jsonb(t) ORDER BY journal_id,ledger_account_id),'[]') FROM core_billing.ledger_entries t),
        'balances',(SELECT coalesce(jsonb_agg(to_jsonb(t) ORDER BY ledger_account_id),'[]') FROM core_billing.balance_state t))::text,'UTF8')),'hex')"#).fetch_one(pool).await.unwrap()
}
async fn consumed(pool: &PgPool, id: i64) -> (i64, i64, Option<i64>) {
    sqlx::query_as("SELECT c.used_credits::bigint,c.reserved_credits::bigint,c.reset_event_id FROM core_charging.subscription_window($1,floor(extract(epoch FROM clock_timestamp()))::bigint) w CROSS JOIN LATERAL core_charging.subscription_consumption($1,w.start_at,NULL) c")
        .bind(id).fetch_one(pool).await.unwrap()
}
fn request(id: &str, reserve: i64) -> Request {
    Request {
        id: id.into(),
        fingerprint: [9; 32],
        reserve,
        lease_seconds: 300,
        maximum_seconds: 3600,
        price: Price {
            rounding: None,
            text_request: None,
            tool: None,
            audio: None,
            grounding: None,
            user_trust: None,
            image: None,
            upstream_cost_group_ppm: None,
            tiered: None,
            version: "synthetic-reset-v1".into(),
            model: "test-model".into(),
            group: "default".into(),
            input_per_million: 1_000_000,
            cached_per_million: 500_000,
            output_per_million: 1_000_000,
        },
    }
}
async fn settle(charging: &Charging, id: &str, n: i64) {
    assert!(charging.start(id, &[11; 32]).await.unwrap());
    charging
        .settle(
            id,
            &[11; 32],
            Usage {
                output: n,
                ..Usage::default()
            },
            charging::Outcome::Completed,
        )
        .await
        .unwrap();
}

#[sqlx::test(migrations = false)]
async fn reset_voucher_generation_replay_inflight_and_late_refund_preserve_money(pool: PgPool) {
    let mut f = fixture(&pool, true).await;
    let role = runtime::RuntimeRole::create(&pool).await;
    role.optional("core_charging").await;
    role.optional("core_subscriptions").await;
    f.store = SubscriptionStore::new(role.pool.clone());
    let plan = f
        .store
        .save_plan(&f.admin, None, create("reset_plan_original"))
        .await
        .unwrap();
    let mut ids = Vec::new();
    for key in ["reset_purchase_first", "reset_purchase_second"] {
        let q = f
            .store
            .quote(&f.user, quote(plan.id, key, None))
            .await
            .unwrap();
        ids.push(
            f.store
                .confirm(&f.user, &q.id)
                .await
                .unwrap()
                .subscription_id
                .unwrap(),
        );
    }
    let original_grants: Value = sqlx::query_scalar(
        "SELECT jsonb_agg(to_jsonb(s) ORDER BY id) FROM core_charging.subscriptions s",
    )
    .fetch_one(&pool)
    .await
    .unwrap();
    let key = f
        .identity
        .issue_key(
            &f.user,
            Account {
                kind: AccountKind::Personal,
                id: 8302,
            },
            FundingPolicy {
                version: 1,
                account_order: None,
                personal_billing_preference: Some(BillingPreference::SubscriptionOnly),
            },
            3600,
        )
        .await
        .unwrap();
    sqlx::query("INSERT INTO core_identity.user_routing_groups(user_id,group_name,enabled,source_reference) VALUES(8302,'default',true,'synthetic-reset')").execute(&pool).await.unwrap();
    let before_money = money(&pool).await;
    let t = time(&pool).await;
    let code = "11111111111111111111111111111111";
    seed_code(&pool, plan.id, code, 0, t + 3600).await;
    let claimed = f
        .store
        .try_redeem_reset_code(&f.user, code_request(code, "reset_code_claim_first"))
        .await
        .unwrap()
        .unwrap();
    assert!(claimed.get("amount_units").is_none() && claimed.get("journal_id").is_none());
    let voucher = claimed["reset_voucher_id"].as_i64().unwrap();
    assert_eq!(
        f.store
            .try_redeem_reset_code(&f.user, code_request(code, "reset_code_claim_first"))
            .await
            .unwrap(),
        Some(claimed)
    );
    assert_eq!(
        f.store
            .try_redeem_reset_code(&f.other, code_request(code, "reset_code_claim_other"))
            .await,
        Err(Error::Conflict)
    );
    assert_eq!(
        f.store.redeem_reset_voucher(&f.other, voucher).await,
        Err(Error::NotFound)
    );
    assert_eq!(
        f.store
            .preview_reset(&f.user, preview(plan.id, "hard", None))
            .await,
        Err(Error::Forbidden)
    );
    let charging = Charging::new(role.pool.clone(), Arc::new(TransactionalLedger));
    let first = request("reset-before-generation", 40);
    let reserved = charging
        .reserve(&key.secret, &first, &[11; 32])
        .await
        .unwrap();
    let active_id = match reserved.source {
        Source::Subscription { id, .. } => id,
        _ => panic!("subscription source required"),
    };
    assert_eq!(
        f.store.redeem_reset_voucher(&f.user, voucher).await,
        Err(Error::Conflict)
    );
    assert_eq!(
        sqlx::query_scalar::<_, i64>(
            "SELECT count(*) FROM core_subscriptions.reset_voucher_redemptions"
        )
        .fetch_one(&pool)
        .await
        .unwrap(),
        0
    );
    settle(&charging, &first.id, 5).await;
    let old_request: Value =
        sqlx::query_scalar("SELECT to_jsonb(r) FROM core_charging.requests r WHERE id=$1")
            .bind(&first.id)
            .fetch_one(&pool)
            .await
            .unwrap();
    let receipt = f
        .store
        .redeem_reset_voucher(&f.user, voucher)
        .await
        .unwrap();
    assert_eq!(receipt["reset_count"], 2);
    assert_eq!(receipt["restored_quota"], 5);
    assert_eq!(
        f.store
            .redeem_reset_voucher(&f.user, voucher)
            .await
            .unwrap(),
        receipt
    );
    assert_eq!(
        sqlx::query_scalar::<_, i64>("SELECT count(*) FROM core_subscriptions.reset_events")
            .fetch_one(&pool)
            .await
            .unwrap(),
        2
    );
    assert_eq!(
        sqlx::query_scalar::<_, Value>(
            "SELECT to_jsonb(r) FROM core_charging.requests r WHERE id=$1"
        )
        .bind(&first.id)
        .fetch_one(&pool)
        .await
        .unwrap(),
        old_request
    );
    assert_eq!(consumed(&pool, active_id).await.0, 0);
    // Displayed current usage is zero after a reset, but refund authority still
    // consults all immutable historical consumption across generations.
    let purchase_id: i64 =
        sqlx::query_scalar("SELECT id FROM core_subscriptions.purchases WHERE subscription_id=$1")
            .bind(active_id)
            .fetch_one(&pool)
            .await
            .unwrap();
    assert_eq!(
        f.store.preview_refund(&f.user, purchase_id).await,
        Err(Error::Used)
    );
    let second = request("reset-after-generation", 40);
    charging
        .reserve(&key.secret, &second, &[11; 32])
        .await
        .unwrap();
    settle(&charging, &second.id, 6).await;
    charging
        .refund(
            &first.id,
            "late-old-reset-refund",
            5,
            "synthetic provider refund after reset",
        )
        .await
        .unwrap();
    assert_eq!(
        consumed(&pool, active_id).await.0,
        6,
        "old-generation refund changed new quota"
    );
    assert_eq!(
        f.store
            .purchases(&f.user)
            .await
            .unwrap()
            .iter()
            .map(|p| p.used_units.parse::<i64>().unwrap())
            .sum::<i64>(),
        6
    );
    let summary = f
        .store
        .reset_targets(&f.admin, ResetFilter::default(), 1, 20)
        .await
        .unwrap();
    assert_eq!(summary["total"], 1);
    assert_eq!(summary["items"][0]["amount_used"], 6);
    let soft = f
        .store
        .preview_reset(&f.admin, preview(plan.id, "soft", Some(t + 3600)))
        .await
        .unwrap();
    let soft_result = f
        .store
        .execute_reset(
            &f.admin,
            execute(soft["token"].as_str().unwrap(), "root-soft-once"),
        )
        .await
        .unwrap();
    assert_eq!(soft_result["vouchers_issued"], 1);
    assert_eq!(soft_result["restored_quota"], 0);
    assert_eq!(consumed(&pool, active_id).await.0, 6);
    let hard = f
        .store
        .preview_reset(&f.admin, preview(plan.id, "hard", None))
        .await
        .unwrap();
    let hard_token = hard["token"].as_str().unwrap();
    let hard_result = f
        .store
        .execute_reset(&f.admin, execute(hard_token, "root-hard-once"))
        .await
        .unwrap();
    assert_eq!(hard_result["restored_quota"], 6);
    assert_eq!(
        f.store
            .execute_reset(&f.admin, execute(hard_token, "root-hard-once"))
            .await
            .unwrap(),
        hard_result
    );
    assert_eq!(
        f.store
            .execute_reset(&f.admin, execute(hard_token, "root-other-operation"))
            .await,
        Err(Error::Conflict)
    );
    let stale = f
        .store
        .preview_reset(&f.admin, preview(plan.id, "hard", None))
        .await
        .unwrap();
    let third = request("reset-after-preview", 40);
    charging
        .reserve(&key.secret, &third, &[11; 32])
        .await
        .unwrap();
    settle(&charging, &third.id, 7).await;
    assert_eq!(
        f.store
            .execute_reset(
                &f.admin,
                execute(stale["token"].as_str().unwrap(), "root-stale-must-refuse")
            )
            .await,
        Err(Error::Conflict)
    );
    assert_eq!(consumed(&pool, active_id).await.0, 7);
    assert_eq!(
        sqlx::query_scalar::<_, Value>(
            "SELECT jsonb_agg(to_jsonb(s) ORDER BY id) FROM core_charging.subscriptions s"
        )
        .fetch_one(&pool)
        .await
        .unwrap(),
        original_grants
    );
    assert_eq!(money(&pool).await, before_money);
    for sql in [
        "DELETE FROM core_subscriptions.reset_events",
        "UPDATE core_subscriptions.reset_vouchers SET expires_at=expires_at+1",
        "INSERT INTO core_subscriptions.reset_codes SELECT * FROM core_subscriptions.reset_codes",
        "UPDATE core_charging.subscription_request_resets SET reset_event_id=reset_event_id",
    ] {
        assert!(sqlx::raw_sql(sql).execute(&role.pool).await.is_err());
    }
    balanced(&pool).await;
    role.cleanup().await;
}

#[sqlx::test(migrations = false)]
async fn reset_bearer_future_rights_explicit_basis_and_expiry_are_exact(pool: PgPool) {
    let mut f = fixture(&pool, true).await;
    let role = runtime::RuntimeRole::create(&pool).await;
    role.optional("core_charging").await;
    role.optional("core_subscriptions").await;
    f.store = SubscriptionStore::new(role.pool.clone());
    let plan = f
        .store
        .save_plan(&f.admin, None, create("reset_explicit_plan"))
        .await
        .unwrap();
    let t = time(&pool).await;
    let before_money = money(&pool).await;
    let code = "22222222222222222222222222222222";
    seed_code(&pool, plan.id, code, 0, 253402300799).await;
    let claim = f
        .store
        .try_redeem_reset_code(&f.user, code_request(code, "reset_future_right_claim"))
        .await
        .unwrap()
        .unwrap();
    let id = claim["reset_voucher_id"].as_i64().unwrap();
    assert_eq!(claim["reset_voucher_expires_at"], 253402300799_i64);
    assert_eq!(
        f.store.redeem_reset_voucher(&f.user, id).await,
        Err(Error::Conflict)
    );
    assert_eq!(
        f.store.reset_vouchers(&f.user).await.unwrap()[0]["status"],
        "available"
    );
    // These owner-attested imported grants are synthetic. The reset basis is
    // deliberately lower than the opening basis, as after a partial refund.
    let mut created = Vec::new();
    for (suffix, reset_limit, unlimited) in [("rebased", 200_i64, false), ("unlimited", 0, true)] {
        let mut tx = pool.begin().await.unwrap();
        sqlx::query("SET TRANSACTION ISOLATION LEVEL SERIALIZABLE")
            .execute(&mut *tx)
            .await
            .unwrap();
        let sub:i64=sqlx::query_scalar("INSERT INTO core_charging.subscriptions(account_id,grant_reference,period,anchor,seconds,ends_at,limit_credits,reset_basis,reset_limit_credits,reset_unlimited,models,groups) VALUES($1,$2,'custom',$3,120,$4,400,'explicit',$5,$6,ARRAY['*'],ARRAY['*']) RETURNING id")
            .bind(f.account).bind(format!("synthetic-reset-{suffix}")).bind(t-60).bind(t+600).bind(reset_limit).bind(unlimited).fetch_one(&mut *tx).await.unwrap();
        sqlx::query("INSERT INTO core_charging.subscription_opening_consumption(subscription_id,start_at,end_at,consumed_credits,import_reference) VALUES($1,$2,$3,300,$4)")
            .bind(sub).bind(t-60).bind(t+60).bind(format!("synthetic-opening-{suffix}")).execute(&mut *tx).await.unwrap();
        sqlx::query("INSERT INTO core_subscriptions.reset_imported_grant_plans(subscription_id,user_id,plan_id,source_reference) VALUES($1,8302,$2,$3)")
            .bind(sub).bind(plan.id).bind(format!("synthetic-plan-{suffix}")).execute(&mut *tx).await.unwrap();
        tx.commit().await.unwrap();
        created.push((sub, reset_limit, unlimited));
    }
    let receipt = f.store.redeem_reset_voucher(&f.user, id).await.unwrap();
    assert_eq!(receipt["reset_count"], 2);
    assert_eq!(receipt["restored_quota"], 100);
    for (sub, limit, unlimited) in created {
        assert_eq!(consumed(&pool, sub).await.0, 0);
        let window: (i64, i64, i64, bool) =
            sqlx::query_as("SELECT * FROM core_charging.subscription_window($1,$2)")
                .bind(sub)
                .bind(t)
                .fetch_one(&pool)
                .await
                .unwrap();
        assert_eq!(window, (t - 60, t + 60, limit, unlimited));
        let next: (i64, i64, i64, bool) =
            sqlx::query_as("SELECT * FROM core_charging.subscription_window($1,$2)")
                .bind(sub)
                .bind(t + 60)
                .fetch_one(&pool)
                .await
                .unwrap();
        assert_eq!(next, (t + 60, t + 180, limit, unlimited));
    }
    let expired = "33333333333333333333333333333333";
    seed_code(&pool, plan.id, expired, t - 1, t + 600).await;
    assert_eq!(
        f.store
            .try_redeem_reset_code(&f.user, code_request(expired, "reset_expired_code_reject"))
            .await,
        Err(Error::Conflict)
    );
    let expired_voucher:i64=sqlx::query_scalar("INSERT INTO core_subscriptions.reset_vouchers(user_id,plan_id,operation_id,expires_at,created_by,created_at) VALUES(8302,$1,'synthetic-expired-voucher',$2,8301,$3) RETURNING id")
        .bind(plan.id).bind(t-1).bind(t-60).fetch_one(&pool).await.unwrap();
    assert_eq!(
        f.store.redeem_reset_voucher(&f.user, expired_voucher).await,
        Err(Error::Conflict)
    );
    assert_eq!(money(&pool).await, before_money);
    role.cleanup().await;
}

#[sqlx::test(migrations = false)]
async fn reset_code_http_one_owner_original_response_and_saved_retry(pool: PgPool) {
    let f = fixture(&pool, true).await;
    let role = runtime::RuntimeRole::create(&pool).await;
    role.optional("core_charging").await;
    role.optional("core_subscriptions").await;
    role.optional("core_promotions").await;
    let store = SubscriptionStore::new(role.pool.clone());
    let plan = store
        .save_plan(&f.admin, None, create("reset_http_plan_original"))
        .await
        .unwrap();
    let code = "44444444444444444444444444444444";
    seed_code(&pool, plan.id, code, 0, time(&pool).await + 3600).await;
    let app = subscriptions::router(role.pool.clone())
        .merge(lmm_core::promotions::router(role.pool.clone()));
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let origin = format!("http://{}", listener.local_addr().unwrap());
    let (stop, done) = tokio::sync::oneshot::channel();
    let server = tokio::spawn(async move {
        axum::serve(listener, app)
            .with_graceful_shutdown(async {
                let _ = done.await;
            })
            .await
            .unwrap()
    });
    let client = reqwest::Client::new();
    let body = json!({"operation_key":"reset_http_claim_request","code":code});
    let (one, two) = tokio::join!(
        client
            .post(format!("{origin}/core/v1/promotions/redemptions"))
            .bearer_auth(&f.user)
            .json(&body)
            .send(),
        client
            .post(format!("{origin}/core/v1/promotions/redemptions"))
            .bearer_auth(&f.other)
            .json(&body)
            .send()
    );
    let one = one.unwrap();
    let two = two.unwrap();
    assert_ne!(one.status().is_success(), two.status().is_success());
    let (winner, loser, response) = if one.status().is_success() {
        (&f.user, &f.other, one)
    } else {
        (&f.other, &f.user, two)
    };
    let result: Value = response.json().await.unwrap();
    assert_eq!(result.as_object().unwrap().len(), 4);
    assert_eq!(result["reward_type"], "reset_voucher");
    let retried: Value = client
        .post(format!("{origin}/core/v1/promotions/redemptions"))
        .bearer_auth(winner)
        .json(&body)
        .send()
        .await
        .unwrap()
        .error_for_status()
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(retried, result);
    let losing = client
        .post(format!("{origin}/core/v1/promotions/redemptions"))
        .bearer_auth(loser)
        .json(&body)
        .send()
        .await
        .unwrap();
    assert_eq!(losing.status(), reqwest::StatusCode::CONFLICT);
    let list: Value = client
        .get(format!("{origin}/api/subscription/self/reset-vouchers"))
        .bearer_auth(winner)
        .send()
        .await
        .unwrap()
        .error_for_status()
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(list["success"], true);
    assert_eq!(list["data"].as_array().unwrap().len(), 1);
    assert_eq!(list["data"][0]["status"], "available");
    let root = client
        .post(format!("{origin}/api/subscription/root/reset/preview"))
        .bearer_auth(winner)
        .json(&json!({"mode":"hard","targets":[]}))
        .send()
        .await
        .unwrap();
    assert_eq!(root.status(), reqwest::StatusCode::FORBIDDEN);
    let no_active = client
        .post(format!(
            "{origin}/api/subscription/self/reset-vouchers/{}/redeem",
            result["reset_voucher_id"].as_i64().unwrap()
        ))
        .bearer_auth(winner)
        .send()
        .await
        .unwrap();
    assert_eq!(no_active.status(), reqwest::StatusCode::CONFLICT);
    assert_eq!(
        sqlx::query_scalar::<_, i64>("SELECT count(*) FROM core_subscriptions.reset_vouchers")
            .fetch_one(&pool)
            .await
            .unwrap(),
        1
    );
    assert_eq!(
        sqlx::query_scalar::<_, i64>(
            "SELECT count(*) FROM core_subscriptions.reset_voucher_redemptions"
        )
        .fetch_one(&pool)
        .await
        .unwrap(),
        0
    );
    stop.send(()).unwrap();
    server.await.unwrap();
    role.cleanup().await;
}

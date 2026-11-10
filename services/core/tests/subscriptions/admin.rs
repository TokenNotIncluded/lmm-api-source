//! Process-level acceptance for administration of the real subscription grants.
//! The SQL-only runner is separate evidence and does not execute these tests.
use super::*;
use subscriptions::{AdminGrantInvalidation, AdminGrantRecordsQuery};

fn invalidation(key: &str) -> AdminGrantInvalidation {
    AdminGrantInvalidation {
        operation_key: key.into(),
        user_id: 8302,
        reason: "Synthetic entitlement revocation".into(),
    }
}

#[sqlx::test]
async fn admin_reads_and_root_invalidation_recheck_authority_before_receipt_recovery(pool: PgPool) {
    let mut f = fixture(&pool, true).await;
    let reader = f.identity.bootstrap_user(8304, 5).await.unwrap().secret;
    let runtime = runtime::RuntimeRole::create(&pool).await;
    runtime.optional("core_charging").await;
    runtime.optional("core_subscriptions").await;
    f.store = SubscriptionStore::new(runtime.pool.clone());
    let plan = f
        .store
        .save_plan(&f.admin, None, create("admin_records_plan"))
        .await
        .unwrap();
    let quote = f
        .store
        .quote(&f.user, quote(plan.id, "admin_records_quote", None))
        .await
        .unwrap();
    let purchase = f.store.confirm(&f.user, &quote.id).await.unwrap();
    let id = purchase.subscription_id.unwrap();
    let records = f
        .store
        .admin_records(&reader, AdminGrantRecordsQuery::default())
        .await
        .unwrap();
    assert_eq!(records.total, 1);
    assert_eq!(records.items[0].user_id, 8302);
    assert_eq!(records.items[0].plan, Some(plan));
    assert_eq!(records.items[0].used_units, "0");
    assert_eq!(records.items[0].reserved_units, "0");
    assert_eq!(
        f.store
            .admin_records(&f.user, AdminGrantRecordsQuery::default())
            .await,
        Err(Error::Forbidden)
    );
    let request = invalidation("admin_invalidation_original");
    assert_eq!(
        f.store.invalidate_grant(&reader, id, request.clone()).await,
        Err(Error::Forbidden)
    );
    let mut wrong_user = request.clone();
    wrong_user.user_id = 8303;
    assert_eq!(
        f.store.invalidate_grant(&f.admin, id, wrong_user).await,
        Err(Error::Conflict)
    );
    let receipt = f
        .store
        .invalidate_grant(&f.admin, id, request.clone())
        .await
        .unwrap();
    assert_eq!(receipt.subscription_id, id);
    assert_eq!(receipt.user_id, 8302);
    assert!(receipt.renewal_stopped);
    assert_eq!(
        f.store
            .invalidate_grant(&f.admin, id, request.clone())
            .await
            .unwrap(),
        receipt
    );
    let mut changed = request.clone();
    changed.reason = "Different original request".into();
    assert_eq!(
        f.store.invalidate_grant(&f.admin, id, changed).await,
        Err(Error::Conflict)
    );
    let rows = f
        .store
        .admin_records(
            &reader,
            AdminGrantRecordsQuery {
                status: Some("invalidated".into()),
                user_id: Some(8302),
                ..Default::default()
            },
        )
        .await
        .unwrap();
    assert_eq!(rows.total, 1);
    assert!(!rows.items[0].active);
    assert_eq!(rows.items[0].renewal_canceled, Some(true));
    assert_eq!(
        f.ledger.balance(f.account).await.unwrap().available_units,
        400_000
    );
    let audits: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM core_identity.audit WHERE action='subscription.grant.invalidate'",
    )
    .fetch_one(&pool)
    .await
    .unwrap();
    assert_eq!(audits, 1);
    sqlx::query("UPDATE core_identity.users SET active=false WHERE id=8301")
        .execute(&pool)
        .await
        .unwrap();
    assert!(matches!(
        f.store.invalidate_grant(&f.admin, id, request).await,
        Err(Error::Unauthorized | Error::Forbidden)
    ));
    balanced(&pool).await;
    runtime.cleanup().await;
}

#[sqlx::test]
async fn invalidation_stops_new_holds_and_renewal_but_existing_requests_can_finish(pool: PgPool) {
    let mut f = fixture(&pool, true).await;
    let runtime = runtime::RuntimeRole::create(&pool).await;
    runtime.optional("core_charging").await;
    runtime.optional("core_subscriptions").await;
    f.store = SubscriptionStore::new(runtime.pool.clone());
    let plan = f
        .store
        .save_plan(&f.admin, None, create("invalidate_usage_plan"))
        .await
        .unwrap();
    let quote = f
        .store
        .quote(&f.user, quote(plan.id, "invalidate_usage_quote", None))
        .await
        .unwrap();
    let purchase = f.store.confirm(&f.user, &quote.id).await.unwrap();
    let id = purchase.subscription_id.unwrap();
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
    sqlx::query("INSERT INTO core_identity.user_routing_groups(user_id,group_name,enabled,source_reference) VALUES(8302,'default',true,'synthetic-revocation-usage')")
        .execute(&pool).await.unwrap();
    let charging = Charging::new(runtime.pool.clone(), Arc::new(TransactionalLedger));
    let mut request = Request {
        id: "before-invalidation-settle".into(),
        fingerprint: [31; 32],
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
            version: "synthetic-v1".into(),
            model: "test-model".into(),
            group: "default".into(),
            input_per_million: 1_000_000,
            cached_per_million: 500_000,
            output_per_million: 1_000_000,
        },
        reserve: 40,
        lease_seconds: 300,
        maximum_seconds: 3600,
    };
    let worker = [32; 32];
    let settle_id = request.id.clone();
    let charge = charging
        .reserve(&key.secret, &request, &worker)
        .await
        .unwrap();
    assert!(matches!(charge.source, Source::Subscription { id: source, .. } if source == id));
    assert!(charging.start(&settle_id, &worker).await.unwrap());
    request.id = "before-invalidation-release".into();
    request.fingerprint = [33; 32];
    let release_id = request.id.clone();
    charging
        .reserve(&key.secret, &request, &worker)
        .await
        .unwrap();
    f.store
        .invalidate_grant(&f.admin, id, invalidation("invalidate_inflight_grant"))
        .await
        .unwrap();
    let held = f
        .store
        .admin_records(&f.admin, AdminGrantRecordsQuery::default())
        .await
        .unwrap();
    assert_eq!(held.items[0].reserved_units, "80");
    assert_eq!(
        charging
            .increase(&key.secret, &settle_id, &worker, 50)
            .await,
        Err(charging::Error::NoFunds)
    );
    request.id = "after-invalidation-denied".into();
    request.fingerprint = [34; 32];
    assert_eq!(
        charging.reserve(&key.secret, &request, &worker).await,
        Err(charging::Error::NoFunds)
    );
    assert_eq!(
        f.store
            .quote(
                &f.user,
                super::quote(plan.id, "invalidated_renewal_denied", purchase.purchase_id)
            )
            .await,
        Err(Error::Conflict)
    );
    let settled = charging
        .settle(
            &settle_id,
            &worker,
            Usage {
                output: 5,
                ..Usage::default()
            },
            charging::Outcome::Completed,
        )
        .await
        .unwrap();
    assert_eq!(settled.state, "settled");
    let released = charging.release(&release_id, &worker).await.unwrap();
    assert_eq!(released.state, "released");
    let after = f
        .store
        .admin_records(&f.admin, AdminGrantRecordsQuery::default())
        .await
        .unwrap();
    assert_eq!(after.items[0].used_units, "5");
    assert_eq!(after.items[0].reserved_units, "0");
    assert!(!after.items[0].active);
    assert_eq!(after.items[0].purchase_id, purchase.purchase_id);
    assert_eq!(
        f.ledger.balance(f.account).await.unwrap().available_units,
        400_000
    );
    balanced(&pool).await;
    runtime.cleanup().await;
}

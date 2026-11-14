//! Management behavior tests share the real provider/ledger fixture in payments.rs.
use super::*;
use axum::{
    body::{Body, to_bytes},
    http::{Request, StatusCode},
};
use lmm_core::payments::discounts::{self, Batch, CodesQuery, Definition};
use tower::ServiceExt;

async fn management(pool: &PgPool) -> Fixture {
    let mut f = fresh_with_level(pool, 6).await;
    for ddl in [discounts::SCHEMA, discounts::MANAGEMENT_SCHEMA] {
        sqlx::raw_sql(ddl).execute(pool).await.unwrap();
    }
    f.store = IngressStore(
        PaymentsStore::new(
            pool.clone(),
            vec![
                quantity_channel(1, "USD", quantity_policy(1, 1)),
                Channel::stripe_test(2, MERCHANT.into(), SECRET.into(), 100_000),
            ],
        )
        .unwrap(),
    );
    f
}
fn definition(code: &str, max: i64) -> Definition {
    Definition {
        code: code.into(),
        name: "Synthetic management".into(),
        discount_percent: 20,
        min_credit_units: 0,
        max_uses: max,
        enabled: true,
        starts_at: 0,
        ends_at: 0,
    }
}
fn batch(key: &str, count: i32) -> Batch {
    Batch {
        request_key: key.into(),
        count,
        name: "Synthetic batch".into(),
        discount_percent: 20,
        min_credit_units: 9007199254740993,
        max_uses: 0,
        enabled: true,
        starts_at: 0,
        ends_at: i64::MAX,
    }
}
async fn request(
    router: axum::Router,
    credential: &str,
    method: &str,
    path: &str,
    body: Value,
) -> (StatusCode, Value) {
    let response = router
        .oneshot(
            Request::builder()
                .method(method)
                .uri(path)
                .header("authorization", format!("Bearer {credential}"))
                .header("content-type", "application/json")
                .body(Body::from(serde_json::to_vec(&body).unwrap()))
                .unwrap(),
        )
        .await
        .unwrap();
    let status = response.status();
    let bytes = to_bytes(response.into_body(), 81920).await.unwrap();
    let value = serde_json::from_slice(&bytes).unwrap_or(Value::Null);
    (status, value)
}
#[sqlx::test]
async fn management_http_exact_quantities_paging_atomic_batch_replay_and_roles(pool: PgPool) {
    let f = management(&pool).await;
    let admin = f.identity.bootstrap_user(202, 5).await.unwrap();
    let user = f.identity.bootstrap_user(203, 0).await.unwrap();
    let router = discounts::http::router(f.store.0.clone());
    let path = "/core/v1/payment-discounts/admin/codes";
    let mut input = serde_json::to_value(definition("EXACT", 0)).unwrap();
    input["name"] = json!("Summer Sale");
    input["min_credit_units"] = json!("9007199254740993");
    input["max_uses"] = json!("9223372036854775807");
    input["starts_at"] = json!("0");
    input["ends_at"] = json!("9223372036854775807");
    let (status, response) =
        request(router.clone(), &admin.secret, "POST", path, input.clone()).await;
    assert_eq!(status, StatusCode::OK);
    for field in [
        "id",
        "min_credit_units",
        "max_uses",
        "starts_at",
        "ends_at",
        "used_count",
        "reserved_count",
    ] {
        assert!(response["data"][field].is_string(), "{field}");
    }
    assert_eq!(response["data"]["min_credit_units"], "9007199254740993");
    let id = response["data"]["id"].as_str().unwrap();
    input["code"] = json!("RENAMED");
    input["enabled"] = json!(false);
    let (status, response) = request(
        router.clone(),
        &admin.secret,
        "PUT",
        &format!("{path}/{id}"),
        input.clone(),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(response["data"]["id"], id);
    assert_eq!(response["data"]["code"], "RENAMED");
    let (_, status_view) = request(
        router.clone(),
        &admin.secret,
        "PUT",
        &format!("{path}/{id}/status"),
        json!({"enabled":true}),
    )
    .await;
    assert_eq!(status_view["data"]["enabled"], true);
    assert_eq!(
        status_view["data"]["min_credit_units"],
        input["min_credit_units"]
    );
    assert_eq!(status_view["data"]["code"], "RENAMED");
    assert_eq!(
        request(
            router.clone(),
            &admin.secret,
            "PUT",
            &format!("{path}/{id}/status"),
            json!({"enabled":false,"discount_percent":5})
        )
        .await
        .0,
        StatusCode::UNPROCESSABLE_ENTITY
    );
    request(
        router.clone(),
        &admin.secret,
        "PUT",
        &format!("{path}/{id}/status"),
        json!({"enabled":false}),
    )
    .await;
    let (_, response) = request(
        router.clone(),
        &admin.secret,
        "GET",
        &format!("{path}?keyword=rename&enabled=false&limit=1&after_id=0"),
        Value::Null,
    )
    .await;
    assert_eq!(response["data"]["items"].as_array().unwrap().len(), 1);
    assert!(response["data"]["next_after"].is_null());
    let (status, name_only) = request(
        router.clone(),
        &admin.secret,
        "GET",
        &format!("{path}?keyword=suMmEr&enabled=false&limit=1&after_id=0"),
        Value::Null,
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    let matches = name_only["data"]["items"].as_array().unwrap();
    assert_eq!(matches.len(), 1);
    assert_eq!(matches[0]["id"], id);
    assert_eq!(matches[0]["name"], "Summer Sale");
    assert_eq!(matches[0]["code"], "RENAMED");
    assert!(!matches[0]["code"].as_str().unwrap().starts_with("SUMMER"));
    assert!(name_only["data"]["next_after"].is_null());
    let mut invalid = input.clone();
    invalid["max_uses"] = json!(1.5);
    assert_eq!(
        request(router.clone(), &admin.secret, "POST", path, invalid)
            .await
            .0,
        StatusCode::UNPROCESSABLE_ENTITY
    );
    for invalid in ["9223372036854775808", "1.0", "1e3"] {
        let mut v = input.clone();
        v["max_uses"] = json!(invalid);
        assert_eq!(
            request(router.clone(), &admin.secret, "POST", path, v)
                .await
                .0,
            StatusCode::UNPROCESSABLE_ENTITY
        );
    }
    for query in ["limit=101", "limit=0", "after_id=-1"] {
        assert_eq!(
            request(
                router.clone(),
                &admin.secret,
                "GET",
                &format!("{path}?{query}"),
                Value::Null
            )
            .await
            .0,
            StatusCode::BAD_REQUEST
        );
    }
    assert_eq!(
        request(router.clone(), &user.secret, "GET", path, Value::Null)
            .await
            .0,
        StatusCode::FORBIDDEN
    );
    let b = batch("71111111-2222-4333-8444-555555555555", 100);
    let value = serde_json::to_value(&b).unwrap();
    let (status, first) = request(
        router.clone(),
        &admin.secret,
        "POST",
        &format!("{path}/batch"),
        value.clone(),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(first["data"]["items"].as_array().unwrap().len(), 100);
    let (_, replay) = request(
        router.clone(),
        &admin.secret,
        "POST",
        &format!("{path}/batch"),
        value.clone(),
    )
    .await;
    assert_eq!(first, replay, "lost ACK must return original batch");
    let mut different = value.clone();
    different["name"] = json!("different");
    assert_eq!(
        request(
            router.clone(),
            &admin.secret,
            "POST",
            &format!("{path}/batch"),
            different
        )
        .await
        .0,
        StatusCode::CONFLICT
    );
    let other = f
        .store
        .generate_discount_codes(&f.secret, b.clone())
        .await
        .unwrap();
    assert_ne!(other[0].id, first["data"]["items"][0]["id"]);
    for count in [0, 101] {
        let mut b = b.clone();
        b.count = count;
        b.request_key = "81111111-2222-4333-8444-555555555555".into();
        assert!(matches!(
            f.store.generate_discount_codes(&admin.secret, b).await,
            Err(Error::Invalid)
        ));
    }
    let mut after = 0;
    let mut ids = Vec::new();
    loop {
        let page = f
            .store
            .list_discount_codes(
                &admin.secret,
                &CodesQuery {
                    after_id: after,
                    limit: 20,
                    ..Default::default()
                },
            )
            .await
            .unwrap();
        ids.extend(page.items.iter().map(|c| c.id.clone()));
        if let Some(next) = page.next_after {
            after = next.parse().unwrap();
        } else {
            break;
        }
    }
    assert_eq!(ids.len(), 201);
    assert!(
        ids.windows(2)
            .all(|pair| pair[0].parse::<i64>().unwrap() > pair[1].parse::<i64>().unwrap()),
        "pages must preserve legacy newest-first order"
    );
    let distinct: std::collections::BTreeSet<_> = ids.iter().collect();
    assert_eq!(distinct.len(), 201);
    let duplicate = definition("RENAMED", 0);
    assert!(matches!(
        f.store.save_discount(&admin.secret, None, duplicate).await,
        Err(Error::Conflict)
    ));
    let other = f
        .store
        .save_discount(&admin.secret, None, definition("OTHER", 0))
        .await
        .unwrap();
    assert!(matches!(
        f.store
            .save_discount(&admin.secret, Some(other.id), definition("RENAMED", 0))
            .await,
        Err(Error::Conflict)
    ));
    let (status, deleted) = request(
        router.clone(),
        &admin.secret,
        "DELETE",
        &format!("{path}/{id}"),
        Value::Null,
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(deleted, json!({"success":true,"data":null}));
    assert_eq!(
        request(
            router.clone(),
            &admin.secret,
            "DELETE",
            &format!("{path}/{id}"),
            Value::Null
        )
        .await
        .0,
        StatusCode::OK
    );
    // Replay returns the original saved views even after a batch code is deleted.
    let batch_id = first["data"]["items"][0]["id"]
        .as_str()
        .unwrap()
        .parse()
        .unwrap();
    f.store
        .delete_discount_code(&admin.secret, batch_id)
        .await
        .unwrap();
    assert_eq!(
        request(
            router.clone(),
            &admin.secret,
            "POST",
            &format!("{path}/batch"),
            value.clone()
        )
        .await
        .1,
        first
    );
    let key = f
        .identity
        .issue_key(
            &f.secret,
            lmm_core::accounts::Account {
                kind: lmm_core::accounts::AccountKind::Personal,
                id: 101,
            },
            lmm_core::funding::FundingPolicy::new_personal_key(),
            3600,
        )
        .await
        .unwrap();
    assert_eq!(
        request(router.clone(), &key.secret, "GET", path, Value::Null)
            .await
            .0,
        StatusCode::FORBIDDEN
    );
    sqlx::query("UPDATE core_identity.users SET platform_role='user' WHERE id=202")
        .execute(&pool)
        .await
        .unwrap();
    assert_eq!(
        request(
            router,
            &admin.secret,
            "POST",
            &format!("{path}/batch"),
            value
        )
        .await
        .0,
        StatusCode::FORBIDDEN,
        "replay must reauthorize"
    );
    let rows: i64 =
        sqlx::query_scalar("SELECT count(*) FROM core_payments.discount_management_batches")
            .fetch_one(&pool)
            .await
            .unwrap();
    assert_eq!(rows, 2);
}
#[sqlx::test]
async fn management_tombstones_keep_pending_frozen_replay_and_consumption_cleanup(pool: PgPool) {
    let f = management(&pool).await;
    let row = f
        .store
        .save_discount(&f.secret, None, definition("SAVE20", 1))
        .await
        .unwrap();
    let legacy = PaymentsStore::new(
        pool.clone(),
        vec![Channel::stripe_test(
            2,
            MERCHANT.into(),
            SECRET.into(),
            100_000,
        )],
    )
    .unwrap();
    let mut old_req = prepare_request(&f, "old_no_coupon_pending_001", 100);
    old_req.payment_channel_id = 2;
    let old = legacy.prepare(&f.secret, old_req.clone()).await.unwrap();
    let mut req = prepare_request(&f, "discount_frozen_pending_001", 0);
    req.requested_credit_units = 1_000_000;
    let intent = f
        .store
        .prepare_with_discount(&f.secret, req.clone(), "SAVE20")
        .await
        .unwrap();
    assert_eq!(intent.amount_minor, 158); // 200 * .99 * .8, rounded half-up
    let view = f.store.discount_code_view(&f.secret, row.id).await.unwrap();
    assert_eq!(
        (view.used_count.as_str(), view.reserved_count.as_str()),
        ("0", "1")
    );
    assert_eq!(
        f.store
            .delete_exhausted_discount_codes(&f.secret)
            .await
            .unwrap(),
        0
    );
    let mut updated = definition("RENAMED20", 1);
    updated.enabled = false;
    updated.discount_percent = 30;
    f.store
        .save_discount(&f.secret, Some(row.id), updated)
        .await
        .unwrap();
    assert_eq!(
        f.store
            .prepare_with_discount(&f.secret, req.clone(), "SAVE20")
            .await
            .unwrap(),
        intent
    );
    f.store
        .delete_discount_code(&f.secret, row.id)
        .await
        .unwrap();
    assert_eq!(
        f.store
            .prepare_with_discount(&f.secret, req, "SAVE20")
            .await
            .unwrap(),
        intent
    );
    assert!(matches!(
        f.store
            .quote_discount(&f.secret, 1, 1_000_000, "RENAMED20")
            .await,
        Err(Error::NotFound)
    ));
    assert!(matches!(
        f.store
            .save_discount(&f.secret, Some(row.id), definition("RESTORE20", 1))
            .await,
        Err(Error::NotFound)
    ));
    assert!(
        f.store
            .discount_codes(&f.secret, 0)
            .await
            .unwrap()
            .is_empty()
    );
    f.store
        .credit(credit_request(
            &intent,
            "discount_credit_after_delete_001",
            quantity_paid_proof(&intent),
        ))
        .await
        .unwrap();
    assert_eq!(f.store.prepare(&f.secret, old_req).await.unwrap(), old);
    credit(&f, &old, "old_credit_after_management_001").await;
    let counts:(i64,i64,i64)=sqlx::query_as("SELECT (SELECT count(*) FROM core_payments.discount_codes),(SELECT count(*) FROM core_payments.discount_reservations),(SELECT count(*) FROM core_payments.discount_reservations WHERE consumed)").fetch_one(&pool).await.unwrap();
    assert_eq!(counts, (1, 1, 1));
    let row = f
        .store
        .save_discount(&f.secret, None, definition("CLEAN20", 1))
        .await
        .unwrap();
    let mut req = prepare_request(&f, "discount_cleanup_pending_001", 0);
    req.requested_credit_units = 1_000_000;
    let consumed = f
        .store
        .prepare_with_discount(&f.secret, req, "CLEAN20")
        .await
        .unwrap();
    f.store
        .credit(credit_request(
            &consumed,
            "discount_cleanup_credit_001",
            quantity_paid_proof(&consumed),
        ))
        .await
        .unwrap();
    let view = f.store.discount_code_view(&f.secret, row.id).await.unwrap();
    assert_eq!(
        (view.used_count.as_str(), view.reserved_count.as_str()),
        ("1", "0")
    );
    let router = discounts::http::router(f.store.0.clone());
    assert_eq!(
        request(
            router,
            &f.secret,
            "DELETE",
            "/core/v1/payment-discounts/admin/codes/exhausted",
            Value::Null
        )
        .await
        .1,
        json!({"success":true,"data":{"count":"1"}})
    );
    assert_eq!(
        f.store
            .delete_exhausted_discount_codes(&f.secret)
            .await
            .unwrap(),
        0
    );
    assert_eq!(f.store.discount_codes(&f.secret, 0).await.unwrap().len(), 0);
    assert_eq!(journals(&pool).await, 3);
}

async fn mark_imported(pool: &PgPool, user: i64) {
    // Synthetic facts exercise runtime policy admission only, not legacy import approval.
    sqlx::query("WITH h AS (SELECT decode(repeat('ab',32),'hex') AS b) INSERT INTO core_identity.privileged_import_facts(user_id,archive_sha256,a2_semantic_sha256,a2_file_sha256,a2_sql_sha256,migration_id,system_id,database_oid,instance_nonce,import_run,source_enabled,source_deleted,password_verifier_recognized,password_login_enabled,password_hash_sha256,login_name_sha256,source_role,source_second_factor_required,source_auth_version) SELECT $1,b,b,b,b,'management-policy-fixture',b,1,b,b,TRUE,FALSE,TRUE,TRUE,b,b,'admin',FALSE,1 FROM h")
        .bind(user).execute(pool).await.unwrap();
}
#[sqlx::test]
async fn management_imported_policy_is_complete_and_static_auth_does_not_invent_catalog_allow(
    pool: PgPool,
) {
    use lmm_core::identity as permissions;
    let f = management(&pool).await;
    let admin = f.identity.bootstrap_user(204, 5).await.unwrap();
    mark_imported(&pool, 204).await;
    assert!(
        matches!(
            f.store.discount_codes(&admin.secret, 0).await,
            Err(Error::Denied)
        ),
        "missing imported policy must fail closed"
    );
    let entries = permissions::Action::CATALOG
        .iter()
        .map(|a| (a.catalog_name().unwrap().to_owned(), "deny".to_owned()))
        .collect::<Vec<_>>();
    let snapshot = permissions::complete_policy_snapshot(204, 1, &entries).unwrap();
    let payload = permissions::canonical_policy_payload(&snapshot).unwrap();
    let sha = permissions::policy_sha256(&snapshot).unwrap();
    sqlx::query("INSERT INTO core_identity.permission_policies(user_id,revision,contract,sha256,canonical_payload,verified_complete) VALUES(204,1,'core.permissions.v1',$1,$2,TRUE)")
        .bind(sha.as_slice()).bind(payload).execute(&pool).await.unwrap();
    assert!(
        matches!(
            f.store.discount_codes(&admin.secret, 0).await,
            Err(Error::Denied)
        ),
        "hash alone cannot claim complete policy"
    );
    for (action, effect) in entries {
        sqlx::query("INSERT INTO core_identity.permission_policy_rows(user_id,action,effect) VALUES(204,$1,$2)").bind(action).bind(effect).execute(&pool).await.unwrap();
    }
    let code = f
        .store
        .save_discount(&admin.secret, None, definition("STATIC20", 0))
        .await
        .unwrap();
    assert_eq!(
        code.code, "STATIC20",
        "old static AdminAuth is independent from unrelated catalog denies"
    );
    let loaded = permissions::load_complete_policy(&mut pool.acquire().await.unwrap(), 204)
        .await
        .unwrap();
    assert_eq!(loaded.sha256, sha);
    let denies:i64=sqlx::query_scalar("SELECT count(*) FROM core_identity.permission_policy_rows WHERE user_id=204 AND effect='deny'").fetch_one(&pool).await.unwrap();
    assert_eq!(denies, 9, "management must preserve explicit denies");
    let other = f.identity.bootstrap_user(205, 5).await.unwrap();
    mark_imported(&pool, 205).await;
    sqlx::query("INSERT INTO core_identity.permission_policies(user_id,revision,contract,sha256,canonical_payload,verified_complete) VALUES(205,1,'core.permissions.v1',decode(repeat('ab',32),'hex'),'tampered'::bytea,TRUE)").execute(&pool).await.unwrap();
    assert!(matches!(
        f.store.discount_codes(&other.secret, 0).await,
        Err(Error::Denied)
    ));
}
#[sqlx::test]
async fn management_failed_batch_rolls_back_codes_audit_and_receipt(pool: PgPool) {
    let f = management(&pool).await;
    sqlx::raw_sql("CREATE FUNCTION core_payments.reject_second_test_code() RETURNS trigger LANGUAGE plpgsql AS $$ BEGIN IF NEW.name='Reject batch' AND EXISTS(SELECT FROM core_payments.discount_codes WHERE name=NEW.name) THEN RAISE EXCEPTION 'synthetic second-item rejection' USING ERRCODE='P1001'; END IF;RETURN NEW;END $$;CREATE TRIGGER reject_second_test_code BEFORE INSERT ON core_payments.discount_codes FOR EACH ROW EXECUTE FUNCTION core_payments.reject_second_test_code();").execute(&pool).await.unwrap();
    let mut input = batch("61111111-2222-4333-8444-555555555555", 3);
    input.name = "Reject batch".into();
    assert!(matches!(
        f.store
            .generate_discount_codes(&f.secret, input.clone())
            .await,
        Err(Error::Conflict)
    ));
    let counts:(i64,i64)=sqlx::query_as("SELECT (SELECT count(*) FROM core_payments.discount_codes),(SELECT count(*) FROM core_payments.discount_management_batches)").fetch_one(&pool).await.unwrap();
    assert_eq!(counts, (0, 0));
    sqlx::query("DROP TRIGGER reject_second_test_code ON core_payments.discount_codes")
        .execute(&pool)
        .await
        .unwrap();
    let items = f
        .store
        .generate_discount_codes(&f.secret, input)
        .await
        .unwrap();
    assert_eq!(items.len(), 3);
}

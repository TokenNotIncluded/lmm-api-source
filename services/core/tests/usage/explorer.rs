use super::*;

async fn observe(pool: &PgPool, id: &str, elapsed: u64, upstream: &str, success: bool) {
    let mut tx = pool.begin().await.unwrap();
    sqlx::query("SET TRANSACTION ISOLATION LEVEL SERIALIZABLE")
        .execute(&mut *tx)
        .await
        .unwrap();
    sqlx::query("INSERT INTO core_charging.usage_events(request_id,event_id,detail) VALUES($1,'performance-v1',$2)")
        .bind(id).bind(json!({"schema":"core.performance.v1","finished_at":1700000000,"elapsed_us":elapsed,
            "first_output_us":120500,"output_tokens":20,"success":success,"is_stream":true,
            "upstream_request_id":upstream,"status_code":if success {200}else{503}}))
        .execute(&mut *tx).await.unwrap();
    tx.commit().await.unwrap();
}

async fn csv(app: &Router, path: &str, secret: &str) -> Vec<u8> {
    let response = app
        .clone()
        .oneshot(
            Request::builder()
                .uri(path)
                .header("authorization", format!("Bearer {secret}"))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    assert_eq!(
        response.headers()["content-type"],
        "text/csv; charset=utf-8"
    );
    to_bytes(response.into_body(), 1 << 20)
        .await
        .unwrap()
        .to_vec()
}

#[sqlx::test(migrations = false)]
async fn explorer_uses_identical_filters_for_list_summary_export_and_scoped_detail(pool: PgPool) {
    let f = fixture(&pool).await;
    settled(&f, &f.key.secret, "a-success", Outcome::Completed).await;
    settled(&f, &f.key.secret, "b-failed", Outcome::Failed).await;
    f.charging
        .reserve(&f.key.secret, &spec("c-pending"), &WORKER)
        .await
        .unwrap();
    f.charging
        .refund("a-success", "explorer-refund", 20, "private test refund")
        .await
        .unwrap();
    observe(&pool, "a-success", 1_250_250, "upstream-good", true).await;
    observe(&pool, "b-failed", 4_000_000, "upstream-failed", false).await;

    let all = ok(&f, "/api/log/self/stat").await;
    assert_eq!(all["total_count"], 3);
    assert_eq!(all["success_count"], 1);
    assert_eq!(all["error_count"], 1);
    assert_eq!(all["pending_count"], 1);
    assert_eq!(all["refunded_count"], 1);
    assert_eq!(all["total_tokens"], 60);
    assert_eq!(all["input_tokens"], 20);
    assert_eq!(all["output_tokens"], 40);
    assert_eq!(all["duration_sample_count"], 2);
    assert_eq!(all["average_duration_ms"], 2625.125);
    assert_eq!(all["p95_duration_ms"], 4000.0);
    assert_eq!(all["duration_percentile_scope"], "all_samples");
    assert_eq!(all["rate_basis"], "selected_window");
    assert_eq!(all["rate_window_seconds"], 60);
    let seconds = all["window_end"].as_f64().unwrap() - all["window_start"].as_f64().unwrap() + 1.0;
    assert_eq!(
        all["requests_per_minute"].as_f64().unwrap(),
        180.0 / seconds
    );

    let filters = "status=error&min_duration_ms=2000&max_duration_ms=5000&upstream_request_id=upstream-failed";
    let page = ok(&f, &format!("/api/log/self?{filters}")).await;
    assert_eq!(page["total"], 1);
    let item = &page["items"][0];
    assert_eq!(item["request_id"], "b-failed");
    assert_eq!(item["duration_ms"], 4000.0);
    assert_eq!(item["first_token_ms"], 120.5);
    assert_eq!(item["is_stream"], true);
    assert_eq!(item["upstream_request_id"], "upstream-failed");
    assert_eq!(item["status_code"], 503);
    assert!(item.get("channel").is_none());
    assert!(item.get("route_attempts").is_none());
    let other: Value = serde_json::from_str(item["other"].as_str().unwrap()).unwrap();
    assert!(other.get("admin_info").is_none());
    let summary = ok(&f, &format!("/api/log/self/stat?{filters}")).await;
    assert_eq!(summary["total_count"], 1);
    assert_eq!(summary["quota"], item["quota"]);
    assert_eq!(summary["error_count"], 1);
    assert_eq!(summary["duration_sample_count"], 1);
    let exported = csv(
        &f.app,
        &format!("/core/v1/usage/self/export.csv?{filters}"),
        &f.session.secret,
    )
    .await;
    let mut reader = csv::Reader::from_reader(exported.as_slice());
    let headers = reader.headers().unwrap().clone();
    assert!(
        !headers
            .iter()
            .any(|header| matches!(header, "channel" | "route_attempts" | "username"))
    );
    let values: Vec<_> = reader
        .deserialize::<std::collections::HashMap<String, String>>()
        .map(Result::unwrap)
        .collect();
    assert_eq!(values.len(), 1);
    assert_eq!(values[0]["request_id"], "b-failed");
    assert_eq!(values[0]["duration_ms"], "4000");
    assert_eq!(values[0]["upstream_request_id"], "upstream-failed");
    assert_eq!(values[0]["status_code"], "503");

    let detail = ok(&f, "/core/v1/usage/self/request?request_id=b-failed").await;
    assert_eq!(&detail, item);
    assert_eq!(
        ok(&f, "/api/log/self?status=refunded").await["items"][0]["request_id"],
        "a-success"
    );
    let pending = ok(&f, "/api/log/self?status=pending").await;
    assert_eq!(pending["total"], 1);
    assert_eq!(pending["items"][0]["request_id"], "c-pending");
    assert!(pending["items"][0]["duration_ms"].is_null());
    assert_eq!(
        ok(&f, "/api/log/self?status=pending&min_duration_ms=0").await["total"],
        0
    );
    assert_eq!(
        ok(&f, "/api/log/self?upstream_request_id=absent").await["total"],
        0
    );
    let ascending = ok(&f, "/api/log/self?sort_order=asc&page_size=1").await;
    assert_eq!(ascending["items"][0]["request_id"], "a-success");
    let descending = ok(&f, "/api/log/self?sort_order=desc&page_size=1").await;
    assert_eq!(descending["items"][0]["request_id"], "c-pending");

    let other_user = f.identity.bootstrap_user(8, 0).await.unwrap();
    let forbidden = get(
        &f.app,
        "/core/v1/usage/self/request?request_id=b-failed",
        Some(&other_user.secret),
    )
    .await;
    assert_eq!(forbidden.0, StatusCode::NOT_FOUND);
    assert_eq!(forbidden.1["code"], "USAGE_NOT_FOUND");
    assert!(forbidden.1.get("data").is_none());
    assert_eq!(
        get(
            &f.app,
            "/core/v1/usage/self/request?request_id=missing",
            Some(&f.session.secret)
        )
        .await
        .0,
        StatusCode::NOT_FOUND
    );
    let admin = f.identity.bootstrap_user(9, 5).await.unwrap();
    assert_eq!(
        get(
            &f.app,
            "/core/v1/usage/admin/request?request_id=b-failed",
            Some(&f.session.secret)
        )
        .await
        .0,
        StatusCode::FORBIDDEN
    );
    let admin_detail = get(
        &f.app,
        "/core/v1/usage/admin/request?request_id=b-failed",
        Some(&admin.secret),
    )
    .await;
    assert_eq!(admin_detail.0, StatusCode::OK);
    assert!(admin_detail.1["data"]["username"].is_string());
    assert!(admin_detail.1["data"]["channel"].is_null());
    assert_eq!(
        get(
            &f.app,
            "/core/v1/usage/admin/logs?channel=123",
            Some(&admin.secret)
        )
        .await
        .1["data"]["total"],
        0
    );
}

#[sqlx::test(migrations = false)]
async fn old_performance_receipts_keep_unknown_stream_and_provider_identifiers(pool: PgPool) {
    let f = fixture(&pool).await;
    settled(&f, &f.key.secret, "legacy-telemetry", Outcome::Completed).await;
    let mut tx = pool.begin().await.unwrap();
    sqlx::query("SET TRANSACTION ISOLATION LEVEL SERIALIZABLE")
        .execute(&mut *tx)
        .await
        .unwrap();
    sqlx::query("INSERT INTO core_charging.usage_events(request_id,event_id,detail) VALUES('legacy-telemetry','performance-v1',$1)")
        .bind(json!({"schema":"core.performance.v1","finished_at":1700000000,"elapsed_us":1234567,"first_output_us":null,"output_tokens":20,"success":true}))
        .execute(&mut *tx).await.unwrap();
    tx.commit().await.unwrap();
    let detail = ok(
        &f,
        "/core/v1/usage/self/request?request_id=legacy-telemetry",
    )
    .await;
    assert_eq!(detail["duration_ms"], 1234.567);
    for field in [
        "first_token_ms",
        "is_stream",
        "upstream_request_id",
        "status_code",
    ] {
        assert!(detail[field].is_null(), "{field}: {detail}");
    }
}

#[sqlx::test(migrations = false)]
async fn invalid_optional_telemetry_does_not_hide_valid_financial_receipts(pool: PgPool) {
    let f = fixture(&pool).await;
    settled(&f, &f.key.secret, "invalid-telemetry", Outcome::Completed).await;
    let mut tx = pool.begin().await.unwrap();
    sqlx::query("SET TRANSACTION ISOLATION LEVEL SERIALIZABLE")
        .execute(&mut *tx)
        .await
        .unwrap();
    sqlx::query("INSERT INTO core_charging.usage_events(request_id,event_id,detail) VALUES('invalid-telemetry','performance-v1',$1)")
        .bind(json!({"schema":"core.performance.v1","finished_at":1700000000,"elapsed_us":1000,"first_output_us":2000,"output_tokens":20,"success":true}))
        .execute(&mut *tx).await.unwrap();
    tx.commit().await.unwrap();
    let detail = ok(
        &f,
        "/core/v1/usage/self/request?request_id=invalid-telemetry",
    )
    .await;
    assert_eq!(detail["quota"], 49);
    assert_eq!(detail["diagnostics_available"], false);
    assert!(detail["duration_ms"].is_null());
    assert!(detail["first_token_ms"].is_null());
    let summary = ok(&f, "/api/log/self/stat?request_id=invalid-telemetry").await;
    assert_eq!(summary["quota"], 49);
    assert_eq!(summary["total_count"], 1);
    assert_eq!(summary["duration_sample_count"], 0);
    assert!(summary["average_duration_ms"].is_null());
    let exported = csv(
        &f.app,
        "/core/v1/usage/self/export.csv?request_id=invalid-telemetry",
        &f.session.secret,
    )
    .await;
    let records: Vec<_> = csv::Reader::from_reader(exported.as_slice())
        .deserialize::<std::collections::HashMap<String, String>>()
        .map(Result::unwrap)
        .collect();
    assert_eq!(records.len(), 1);
    assert_eq!(records[0]["net_consumption_credits"], "49");
    assert_eq!(records[0]["duration_ms"], "");
}

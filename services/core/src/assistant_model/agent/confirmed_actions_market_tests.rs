//! Synthetic reviewed Market service, real paid Agent preparation and the
//! original HttpOnly browser. No MCP dispatch, external provider or production data.
use super::*;
use crate::tool_market::{
    assistant_runtime::AssistantMarketRuntime,
    credits::{Price, Quote},
};

pub(super) const SERVICE: &str = "confirmation-market-service";
const TOOL: &str = "confirmation-market-tool";
const VERSION: &str = "confirmation-market-version";

pub(super) fn arguments() -> Value {
    json!({"service_id":SERVICE,"tool_id":TOOL,"version_id":VERSION})
}
pub(super) fn limits() -> Value {
    json!({"max_price_quota":7,"max_total_quota":21,"max_calls":3,"lifetime_seconds":600})
}
pub(super) async fn seed(f: &ConfirmFixture, pool: &PgPool) {
    let password = "synthetic-seller123";
    f.browser
        .register(&BrowserRegistration {
            username: "confirmation-seller",
            password,
            email: "",
        })
        .await
        .unwrap();
    let seller = f
        .browser
        .login(
            "confirmation-seller",
            password,
            &BrowserSessionMetadata::default(),
        )
        .await
        .unwrap();
    let seller_user = seller.data["user"]["id"].as_i64().unwrap();
    let principal = f
        .identity
        .authorize(seller.data["access_token"].as_str().unwrap())
        .await
        .unwrap();
    let seller_account = principal.owner_account_id;
    assert_ne!(seller_user, f.user);
    assert_ne!(seller_account, f.account);
    crate::billing::ledger::Ledger::from_pool(pool.clone())
        .open_wallet(seller_account)
        .await
        .unwrap();
    let price = Price::Fixed { price_quota: 7 };
    let remote_digest =
        crate::hex_codec::encode(Sha256::digest(b"synthetic-reviewed-market-schema-v1"));
    let pricing_digest =
        crate::hex_codec::encode(Sha256::digest(serde_json::to_vec(&price).unwrap()));
    let quote_id = crate::hex_codec::encode(Sha256::digest(
        serde_json::to_vec(&[
            "legacy-credit-quote-v2",
            VERSION,
            TOOL,
            &remote_digest,
            &pricing_digest,
        ])
        .unwrap(),
    ));
    let quote = Quote {
        id: quote_id,
        service_id: SERVICE.into(),
        tool_id: TOOL.into(),
        version_id: VERSION.into(),
        remote_digest,
        pricing_digest,
        price,
        expires: chrono::Utc::now().timestamp() + 3600,
    };
    sqlx::query("INSERT INTO core_tool_market.publishers(user_id,active) VALUES($1,TRUE)")
        .bind(seller_user)
        .execute(pool)
        .await
        .unwrap();
    sqlx::query("INSERT INTO core_tool_market.credit_services(service_id,owner_user_id,active_version_id,updated_by,updated_at) VALUES($1,$2,$3,$2,$4)")
        .bind(SERVICE).bind(seller_user).bind(VERSION).bind(chrono::Utc::now().timestamp())
        .execute(pool).await.unwrap();
    sqlx::query("INSERT INTO core_tool_market.credit_quotes(id,seller_user_id,seller_account_id,fee_bps,policy_version,snapshot,approved,ceiling_credits) VALUES($1,$2,$3,0,1,$4,TRUE,7)")
        .bind(&quote.id).bind(seller_user).bind(seller_account).bind(sqlx::types::Json(&quote))
        .execute(pool).await.unwrap();
    assert_eq!(
        count(
            pool,
            "SELECT count(*) FROM core_tool_market.credit_consents"
        )
        .await,
        0
    );
    assert_eq!(
        count(
            pool,
            "SELECT count(*) FROM core_tool_market.credit_access_operations"
        )
        .await,
        0
    );
}
async fn count(pool: &PgPool, sql: &'static str) -> i64 {
    sqlx::query_scalar(sql).fetch_one(pool).await.unwrap()
}
async fn ledger_snapshot(pool: &PgPool) -> Value {
    sqlx::query_scalar("SELECT jsonb_build_object(
      'accounts',(SELECT COALESCE(jsonb_agg(to_jsonb(a) ORDER BY to_jsonb(a)::text),'[]'::jsonb) FROM core_billing.ledger_accounts a),
      'balances',(SELECT COALESCE(jsonb_agg(to_jsonb(b) ORDER BY to_jsonb(b)::text),'[]'::jsonb) FROM core_billing.balance_state b),
      'journals',(SELECT COALESCE(jsonb_agg(to_jsonb(j) ORDER BY to_jsonb(j)::text),'[]'::jsonb) FROM core_billing.ledger_journals j))")
        .fetch_one(pool).await.unwrap()
}
async fn inspect(f: &ConfirmFixture, id: &str) -> (String, String) {
    let (status, page, nonce) = f.inspect(id).await;
    assert_eq!(status, reqwest::StatusCode::OK, "{page}");
    (page["csrf"].as_str().unwrap().to_owned(), nonce.unwrap())
}
pub(super) async fn approve(f: &ConfirmFixture) -> Value {
    let id = f
        .prepare(
            "connect_market_tool",
            arguments(),
            "Connect this reviewed service with my browser-selected limits.",
            false,
        )
        .await;
    let (csrf, nonce) = inspect(f, &id).await;
    let (status, value) = f
        .post(
            &id,
            &csrf,
            &nonce,
            json!({"decision":"approve","market_limits":limits()}),
        )
        .await;
    assert_eq!(status, reqwest::StatusCode::OK, "{value}");
    assert_eq!(value["executed"], true);
    value
}
async fn replace_runtime(f: &mut ConfirmFixture, policy: AgentPolicy) {
    f.task.abort();
    let _ = (&mut f.task).await;
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let origin = format!("http://{}", listener.local_addr().unwrap());
    let runtime = AssistantMarketRuntime::from_policy(Arc::new(policy)).unwrap();
    let app = confirmation::router(
        BrowserConfirmation::new(f.agent.base.pool.clone(), &origin, true)
            .unwrap()
            .with_identity_store(f.identity.clone())
            .with_market_runtime(runtime),
    );
    f.origin = origin;
    f.task = tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
}

#[sqlx::test(migrations = false)]
async fn confirmed_market_connection_uses_browser_limits_once_without_wallet_mutation(
    pool: PgPool,
) {
    let f = fixture_market(&pool, 60).await;
    let id = f
        .prepare(
            "connect_market_tool",
            arguments(),
            "Connect the reviewed service.",
            false,
        )
        .await;
    let (csrf, nonce) = inspect(&f, &id).await;
    let before = ledger_snapshot(&pool).await;
    let hits = f.provider.hits.load(Ordering::SeqCst);
    let (a, b) = tokio::join!(
        f.post(
            &id,
            &csrf,
            &nonce,
            json!({"decision":"approve","market_limits":limits()})
        ),
        f.post(
            &id,
            &csrf,
            &nonce,
            json!({"decision":"approve","market_limits":limits()})
        ),
    );
    assert_eq!(a.0, reqwest::StatusCode::OK, "{}", a.1);
    assert_eq!(b.0, reqwest::StatusCode::OK, "{}", b.1);
    assert_eq!(a.1["receipt"], b.1["receipt"]);
    let receipt = &a.1["receipt"]["execution_receipt"];
    let grant = receipt["resource_id"].as_str().unwrap();
    assert_eq!(receipt["resource_kind"], "market_consent");
    assert_eq!(receipt["after"]["browser_limits"], limits());
    assert_eq!(receipt["after"]["consent"]["frozen_price_quota"], 7);
    assert_eq!(receipt["after"]["consent"]["max_total_quota"], 21);
    assert_eq!(receipt["after"]["consent"]["max_calls"], 3);
    assert_eq!(receipt["after"]["consent"]["reserved_quota"], 0);
    assert_eq!(receipt["after"]["consent"]["spent_quota"], 0);
    let row = sqlx::query("SELECT * FROM core_tool_market.credit_consents WHERE id=$1")
        .bind(grant)
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(row.get::<i64, _>("actor_user_id"), f.user);
    assert_eq!(row.get::<i64, _>("payer_account_id"), f.account);
    let request: Value = row.get("request");
    assert_eq!(request["client_id"], "builtin-assistant");
    assert_eq!(request["operation_id"], a.1["receipt"]["operation_id"]);
    assert_eq!(
        request["expires_at"].as_i64().unwrap(),
        row.get::<i64, _>("created_at") + 600
    );
    assert_eq!(
        count(
            &pool,
            "SELECT count(*) FROM core_tool_market.credit_consents"
        )
        .await,
        1
    );
    assert_eq!(
        count(
            &pool,
            "SELECT count(*) FROM core_tool_market.credit_access_operations"
        )
        .await,
        1
    );
    assert_eq!(
        count(&pool, "SELECT count(*) FROM core_tool_market.credit_calls").await,
        0
    );
    assert_eq!(audit_count(&pool).await, 1);
    assert!(
        sqlx::query_scalar::<_, bool>("SELECT core_assistant_tools.market_confirmation_bound($1)")
            .bind(&id)
            .fetch_one(&pool)
            .await
            .unwrap()
    );
    let raw: Vec<u8> = sqlx::query_scalar("SELECT o.result_json FROM core_assistant.agent_tool_operations o JOIN core_assistant_tools.confirmations c ON c.operation_id=o.id WHERE c.id=$1")
        .bind(&id).fetch_one(&pool).await.unwrap();
    let model_receipt: Value = serde_json::from_slice(&raw).unwrap();
    assert_eq!(model_receipt["status"], "prepared");
    assert_eq!(model_receipt["executed"], false);
    assert_eq!(ledger_snapshot(&pool).await, before);
    assert_eq!(f.provider.hits.load(Ordering::SeqCst), hits);

    // A real, valid, unused approval cannot authorize another INSERT by
    // lending its confirmation ID. These rows reuse the exact original details
    // and request hash; an absent/fake confirmation is not the attack fixture.
    let canonical = sqlx::query("SELECT * FROM core_tool_market.credit_access_operations WHERE actor_user_id=$1 AND operation_id=$2")
        .bind(f.user).bind(a.1["receipt"]["operation_id"].as_str().unwrap())
        .fetch_one(&pool).await.unwrap();
    let original_operation: String = canonical.get("operation_id");
    let other_actor: i64 =
        sqlx::query_scalar("SELECT seller_user_id FROM core_tool_market.credit_quotes WHERE id=$1")
            .bind(row.get::<String, _>("quote_id"))
            .fetch_one(&pool)
            .await
            .unwrap();
    assert_ne!(other_actor, f.user);
    for (actor, operation, target) in [
        (
            f.user,
            crate::assistant_model::random("borrowed-confirmation-operation"),
            grant.to_owned(),
        ),
        (other_actor, original_operation.clone(), grant.to_owned()),
        // The different operation avoids the existing (actor,operation) PK;
        // the new target must not borrow the old operation's valid receipt.
        (
            f.user,
            crate::assistant_model::random("borrowed-confirmation-target"),
            "f".repeat(64),
        ),
    ] {
        assert!(
            sqlx::query_scalar::<_, bool>(
                "SELECT core_assistant_tools.market_confirmation_bound($1)"
            )
            .bind(&id)
            .fetch_one(&pool)
            .await
            .unwrap()
        );
        let mut tx = pool.begin().await.unwrap();
        sqlx::query("INSERT INTO core_tool_market.credit_access_operations(actor_user_id,operation_id,kind,request_hash,target_id,created_at,details) VALUES($1,$2,$3,$4,$5,$6,$7)")
            .bind(actor).bind(operation).bind(canonical.get::<String,_>("kind"))
            .bind(canonical.get::<String,_>("request_hash")).bind(target)
            .bind(canonical.get::<i64,_>("created_at")).bind(canonical.get::<Value,_>("details"))
            .execute(&mut *tx).await.unwrap();
        let error = tx.commit().await.unwrap_err();
        assert_eq!(
            error.as_database_error().unwrap().code().as_deref(),
            Some("23514")
        );
        assert_eq!(
            count(
                &pool,
                "SELECT count(*) FROM core_tool_market.credit_access_operations"
            )
            .await,
            1
        );
    }

    // Neither reverse table may acquire an assistant-tagged row without the
    // real approved confirmation. Both errors occur at deferred commit.
    let mut tx = pool.begin().await.unwrap();
    let mut orphan_request = request.clone();
    orphan_request["operation_id"] = json!(crate::assistant_model::random("orphan-consent-op"));
    sqlx::query("INSERT INTO core_tool_market.credit_consents(id,actor_user_id,payer_account_id,quote_id,request,created_at) VALUES($1,$2,$3,$4,$5,$6)")
        .bind(crate::hex_codec::encode(Sha256::digest(b"orphan-consent")))
        .bind(f.user).bind(f.account).bind(row.get::<String,_>("quote_id"))
        .bind(orphan_request).bind(row.get::<i64,_>("created_at")).execute(&mut *tx).await.unwrap();
    let error = tx.commit().await.unwrap_err();
    assert_eq!(
        error.as_database_error().unwrap().code().as_deref(),
        Some("23514")
    );
    let mut tx = pool.begin().await.unwrap();
    sqlx::query("INSERT INTO core_tool_market.credit_access_operations(actor_user_id,operation_id,kind,request_hash,target_id,created_at,details) VALUES($1,$2,'consent-create',$3,$4,$5,$6)")
        .bind(f.user).bind(crate::assistant_model::random("orphan-access-op"))
        .bind("b".repeat(64)).bind("c".repeat(64)).bind(row.get::<i64,_>("created_at"))
        .bind(json!({"assistant_confirmation_id":"d".repeat(64),"assistant_tool_operation_id":"e".repeat(64),"policy_revision":"confirmation-market-policy-v1"}))
        .execute(&mut *tx).await.unwrap();
    let error = tx.commit().await.unwrap_err();
    assert_eq!(
        error.as_database_error().unwrap().code().as_deref(),
        Some("23514")
    );
    assert_eq!(
        count(
            &pool,
            "SELECT count(*) FROM core_tool_market.credit_consents"
        )
        .await,
        1
    );
    assert_eq!(
        count(
            &pool,
            "SELECT count(*) FROM core_tool_market.credit_access_operations"
        )
        .await,
        1
    );
    assert_eq!(ledger_snapshot(&pool).await, before);
    f.close().await;
}

#[sqlx::test(migrations = false)]
async fn confirmed_market_limits_are_browser_only_and_replay_cannot_expand_them(pool: PgPool) {
    let f = fixture_market(&pool, 60).await;
    let id = f
        .prepare(
            "connect_market_tool",
            arguments(),
            "Connect only within the limits I select.",
            false,
        )
        .await;
    let (csrf, nonce) = inspect(&f, &id).await;
    let mut too_small = limits();
    too_small["max_price_quota"] = json!(6);
    let mut no_calls = limits();
    no_calls["max_calls"] = json!(0);
    let mut too_long = limits();
    too_long["lifetime_seconds"] = json!(3601);
    for (input, expected) in [
        (
            json!({"decision":"approve"}),
            reqwest::StatusCode::BAD_REQUEST,
        ),
        (
            json!({"decision":"approve","market_limits":too_small}),
            reqwest::StatusCode::FORBIDDEN,
        ),
        (
            json!({"decision":"approve","market_limits":no_calls}),
            reqwest::StatusCode::BAD_REQUEST,
        ),
        (
            json!({"decision":"approve","market_limits":too_long}),
            reqwest::StatusCode::FORBIDDEN,
        ),
    ] {
        let (status, body) = f.post(&id, &csrf, &nonce, input).await;
        assert_eq!(status, expected, "{body}");
        assert_eq!(
            count(
                &pool,
                "SELECT count(*) FROM core_tool_market.credit_consents"
            )
            .await,
            0
        );
        assert_eq!(audit_count(&pool).await, 0);
    }
    let approved = f
        .post(
            &id,
            &csrf,
            &nonce,
            json!({"decision":"approve","market_limits":limits()}),
        )
        .await;
    assert_eq!(approved.0, reqwest::StatusCode::OK, "{}", approved.1);
    let mut expanded = limits();
    expanded["max_total_quota"] = json!(210);
    assert_eq!(
        f.post(
            &id,
            &csrf,
            &nonce,
            json!({"decision":"approve","market_limits":expanded})
        )
        .await
        .0,
        reqwest::StatusCode::CONFLICT
    );
    let replay = f
        .post(
            &id,
            &csrf,
            &nonce,
            json!({"decision":"approve","market_limits":limits()}),
        )
        .await;
    assert_eq!(replay.0, reqwest::StatusCode::OK);
    assert_eq!(replay.1["receipt"], approved.1["receipt"]);
    assert_eq!(
        count(
            &pool,
            "SELECT count(*) FROM core_tool_market.credit_consents"
        )
        .await,
        1
    );
    assert_eq!(audit_count(&pool).await, 1);
    f.close().await;
}

#[sqlx::test(migrations = false)]
async fn confirmed_market_rechecks_live_policy_and_service_revision(pool: PgPool) {
    let mut f = fixture_market(&pool, 60).await;
    let id = f
        .prepare(
            "connect_market_tool",
            arguments(),
            "Prepare this exact current service.",
            false,
        )
        .await;
    let original = (*f.agent.policy).clone();
    let mut changed = original.clone();
    changed.market.as_mut().unwrap().revision = "confirmation-market-policy-v2".into();
    replace_runtime(&mut f, changed).await;
    let (csrf, nonce) = inspect(&f, &id).await;
    assert_eq!(
        f.post(
            &id,
            &csrf,
            &nonce,
            json!({"decision":"approve","market_limits":limits()})
        )
        .await
        .0,
        reqwest::StatusCode::FORBIDDEN
    );
    let mut removed = original.clone();
    removed.allowed_tools.remove("connect_market_tool");
    removed.initial_tools.remove("connect_market_tool");
    replace_runtime(&mut f, removed).await;
    let (csrf, nonce) = inspect(&f, &id).await;
    assert_eq!(
        f.post(
            &id,
            &csrf,
            &nonce,
            json!({"decision":"approve","market_limits":limits()})
        )
        .await
        .0,
        reqwest::StatusCode::FORBIDDEN
    );
    replace_runtime(&mut f, original).await;
    sqlx::query("UPDATE core_tool_market.credit_services SET revision=revision+1,updated_at=extract(epoch FROM clock_timestamp())::bigint WHERE service_id=$1")
        .bind(SERVICE).execute(&pool).await.unwrap();
    let (csrf, nonce) = inspect(&f, &id).await;
    assert_eq!(
        f.post(
            &id,
            &csrf,
            &nonce,
            json!({"decision":"approve","market_limits":limits()})
        )
        .await
        .0,
        reqwest::StatusCode::CONFLICT
    );
    assert_eq!(
        count(
            &pool,
            "SELECT count(*) FROM core_tool_market.credit_consents"
        )
        .await,
        0
    );
    assert_eq!(
        count(
            &pool,
            "SELECT count(*) FROM core_tool_market.credit_access_operations"
        )
        .await,
        0
    );
    assert_eq!(audit_count(&pool).await, 0);
    assert_eq!(
        sqlx::query_scalar::<_, String>(
            "SELECT state FROM core_assistant_tools.confirmations WHERE id=$1"
        )
        .bind(&id)
        .fetch_one(&pool)
        .await
        .unwrap(),
        "pending"
    );
    // A new preparation of the actual revised service is valid; the old
    // immutable preparation remains pending and never gains that authority.
    let fresh = approve(&f).await;
    assert_eq!(
        fresh["receipt"]["execution_receipt"]["resource_kind"],
        "market_consent"
    );
    assert_eq!(audit_count(&pool).await, 1);
    assert_eq!(
        sqlx::query_scalar::<_, String>(
            "SELECT state FROM core_assistant_tools.confirmations WHERE id=$1"
        )
        .bind(&id)
        .fetch_one(&pool)
        .await
        .unwrap(),
        "pending"
    );
    f.close().await;
}

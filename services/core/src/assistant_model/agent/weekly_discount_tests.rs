//! Paid native Agent rounds and actual HttpOnly browser claims.
//! Provider and payment configuration are disposable local fixtures; no external call is made.
use super::*;
use crate::{
    assistant_tools_core::weekly::agent as weekly,
    internal_rpc::pb,
    payments::{Channel, CreditQuotePolicy, Error as PaymentError, PaymentsStore},
};
use rand::{SeedableRng, rngs::StdRng};
use rsa::{PrivateKey, pkcs8::EncodePublicKey};
use std::{collections::BTreeMap, sync::OnceLock};

const APPLY: &str = "Please evaluate my weekly discount application after this conversation.";
const REASON: &str = "The original conversation supports this weekly decision.";

async fn fixture(pool: &PgPool) -> ConfirmFixture {
    let f = fixture_extended(pool, 300, false, false, true).await;
    f.runtime_role.optional("core_payments").await;
    let initial: (bool, i16) =
        sqlx::query_as("SELECT enabled,maximum_percent FROM core_weekly.weekly_discount_policy")
            .fetch_one(pool)
            .await
            .unwrap();
    assert_eq!(initial, (false, 10));
    sqlx::query("UPDATE core_weekly.weekly_discount_policy SET enabled=TRUE")
        .execute(pool)
        .await
        .unwrap();
    f
}
async fn conversation(f: &ConfirmFixture, label: &str) -> String {
    f.agent
        .base
        .conversation(f.access(), label, true)
        .await
        .unwrap()
        .core_conversation_id
}
async fn run_turn(
    f: &ConfirmFixture,
    cv: &str,
    turn: &str,
    text: &str,
    tool: Option<(&str, Value)>,
) -> Option<Value> {
    {
        let mut queue = f.queue.lock().unwrap();
        if let Some((name, arguments)) = &tool {
            queue.push_back(Some((
                (*name).to_string(),
                serde_json::to_string(arguments).unwrap(),
            )));
        }
        queue.push_back(None);
    }
    f.agent.start(f.access(), cv, turn, text).await.unwrap();
    let mut completed = false;
    for _ in 0..600 {
        match f.agent.get(f.access(), cv, turn).await {
            Ok(s) if s.phase == "completed" => {
                assert!(s.final_text.is_some());
                assert!(s.settled_credits > 0);
                assert_eq!(s.held_credits, 0);
                completed = true;
                break;
            }
            Ok(s) => assert!(
                !matches!(s.phase.as_str(), "unknown" | "rejected" | "expired"),
                "{}",
                s.phase
            ),
            Err(Error::Conflict) => {}
            Err(error) => panic!("unexpected weekly Agent error: {error:?}"),
        }
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    assert!(completed, "weekly Agent turn did not complete");
    if let Some((name, _)) = tool {
        let bytes: Vec<u8> = sqlx::query_scalar(
            "SELECT o.result_json FROM core_assistant.agent_tool_operations o
             JOIN core_assistant.agent_turns t ON t.id=o.turn_id
             WHERE t.conversation_id=$1 AND t.client_turn_id=$2 AND o.tool_name=$3 AND o.state='completed'"
        ).bind(cv).bind(turn).bind(name).fetch_one(&f.agent.base.pool).await.unwrap();
        Some(serde_json::from_slice(&bytes).unwrap())
    } else {
        None
    }
}
async fn prepare(f: &ConfirmFixture, cv: &str, turn: &str, text: &str, percent: i16) -> Value {
    run_turn(
        f,
        cv,
        turn,
        text,
        Some((
            weekly::TOOL_NAME,
            json!({"discount_percent":percent,"reason":REASON}),
        )),
    )
    .await
    .unwrap()
}
async fn decision_count(pool: &PgPool) -> i64 {
    sqlx::query_scalar("SELECT count(*) FROM core_weekly.weekly_discounts")
        .fetch_one(pool)
        .await
        .unwrap()
}
async fn offered(f: &ConfirmFixture, percent: i16) -> (String, Value) {
    let cv = conversation(f, "weekly-original-conversation").await;
    run_turn(
        f,
        &cv,
        "warm",
        "I am using this service for a course project.",
        None,
    )
    .await;
    let result = prepare(f, &cv, "apply", APPLY, percent).await;
    assert_eq!(result["created"], true, "{result}");
    assert_eq!(result["decision"]["discount_percent"], percent);
    assert!(result["decision"].get("code").is_none());
    (
        result["decision"]["id"].as_str().unwrap().to_string(),
        result,
    )
}
async fn inspect(
    f: &ConfirmFixture,
    id: &str,
    cookie: &str,
) -> (reqwest::StatusCode, Value, Option<String>) {
    let response = f
        .http
        .get(format!(
            "{}/api/user/auth/assistant/weekly-discounts/{id}",
            f.origin
        ))
        .header("cookie", cookie)
        .send()
        .await
        .unwrap();
    let status = response.status();
    let nonce = response
        .headers()
        .get("set-cookie")
        .map(|v| v.to_str().unwrap().split(';').next().unwrap().to_owned());
    (status, response.json().await.unwrap(), nonce)
}
async fn claim(
    f: &ConfirmFixture,
    id: &str,
    cookie: &str,
    csrf: &str,
    nonce: &str,
    origin: &str,
) -> (reqwest::StatusCode, Value) {
    let response = f
        .http
        .post(format!(
            "{}/api/user/auth/assistant/weekly-discounts/{id}",
            f.origin
        ))
        .header("cookie", format!("{cookie}; {nonce}"))
        .header("origin", origin)
        .header("x-lmm-assistant-csrf", csrf)
        .json(&json!({"claim":true}))
        .send()
        .await
        .unwrap();
    (response.status(), response.json().await.unwrap())
}
async fn history(f: &ConfirmFixture) -> Value {
    let response = f
        .http
        .get(format!(
            "{}/api/user/auth/assistant/weekly-discounts",
            f.origin
        ))
        .header("cookie", f.cookie())
        .send()
        .await
        .unwrap();
    assert_eq!(response.status(), reqwest::StatusCode::OK);
    response.json().await.unwrap()
}
async fn finance(pool: &PgPool) -> Value {
    sqlx::query_scalar(
        "SELECT jsonb_build_object(
          'journals',(SELECT coalesce(jsonb_agg(to_jsonb(j) ORDER BY j.id),'[]'::jsonb) FROM core_billing.ledger_journals j),
          'balances',(SELECT coalesce(jsonb_agg(to_jsonb(b) ORDER BY b.ledger_account_id),'[]'::jsonb) FROM core_billing.balance_state b),
          'intents',(SELECT coalesce(jsonb_agg(to_jsonb(i) ORDER BY i.id),'[]'::jsonb) FROM core_payments.intents i))"
    ).fetch_one(pool).await.unwrap()
}
fn payments(pool: &PgPool) -> PaymentsStore {
    static KEY: OnceLock<PrivateKey> = OnceLock::new();
    let key = KEY.get_or_init(|| PrivateKey::new(&mut StdRng::seed_from_u64(88), 2048).unwrap());
    let mut channel = Channel::pancake_isolated_test(
        88001,
        "merchant_weekly_fixture".into(),
        key.to_public_key()
            .to_public_key_pem(Default::default())
            .unwrap(),
        "STO_weekly".into(),
        "PROD_weekly".into(),
        "USD".into(),
        100_000,
    )
    .unwrap();
    channel.credits_per_minor = 0;
    channel.credit_quote = Some(CreditQuotePolicy {
        settlement_per_usd_numerator: 1,
        settlement_per_usd_denominator: 1,
        discounts: BTreeMap::new(),
    });
    PaymentsStore::new(pool.clone(), vec![channel]).unwrap()
}
fn request(f: &ConfirmFixture, key: &str) -> pb::PrepareTopupRequest {
    pb::PrepareTopupRequest {
        mutation: Some(pb::MutationContext {
            idempotency_key: key.into(),
            expected_version: None,
        }),
        account_id: f.account,
        payment_channel_id: 88001,
        currency: "USD".into(),
        amount_minor: 0,
        requested_credit_units: 1_000_000,
    }
}

#[sqlx::test(migrations = false)]
async fn weekly_native_prepare_requires_two_paid_original_turns_and_preserves_status_questions(
    pool: PgPool,
) {
    let f = fixture(&pool).await;
    let cv = conversation(&f, "weekly-proof-original").await;
    let first = prepare(&f, &cv, "first", APPLY, 5).await;
    assert_eq!(first["created"], false, "{first}");
    assert_eq!(decision_count(&pool).await, 0);
    // The first turn already has two paid model rounds. It still counts as one user turn.
    let paid: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM core_assistant.agent_rounds r JOIN core_assistant.agent_turns t ON t.id=r.turn_id
         JOIN core_charging.requests q ON q.id=r.model_request_id
         WHERE t.conversation_id=$1 AND t.client_turn_id='first' AND r.state='settled' AND q.state='settled'"
    ).bind(&cv).fetch_one(&pool).await.unwrap();
    assert_eq!(paid, 2);
    let read = run_turn(
        &f,
        &cv,
        "read",
        "What is my weekly discount status?",
        Some(("get_weekly_discount_status", json!({}))),
    )
    .await
    .unwrap();
    assert_eq!(read["read_only"], true);
    assert_eq!(read["decision_used"], false);
    assert_eq!(decision_count(&pool).await, 0);
    let status_only = prepare(&f, &cv, "status", "Did you evaluate my weekly discount?", 5).await;
    assert_eq!(status_only["read_only"], true, "{status_only}");
    assert_eq!(status_only["error"], "weekly_status_read_required");
    assert_eq!(decision_count(&pool).await, 0);
    // Paid turns in another conversation do not satisfy the original conversation.
    let other = conversation(&f, "weekly-proof-other").await;
    assert_eq!(
        prepare(&f, &other, "first", APPLY, 5).await["created"],
        false
    );
    assert_eq!(decision_count(&pool).await, 0);
    let accepted = prepare(&f, &cv, "apply", APPLY, 10).await;
    assert_eq!(accepted["created"], true, "{accepted}");
    assert_eq!(accepted["decision"]["status"], "offered");
    assert_eq!(decision_count(&pool).await, 1);
    let binding:(String,String,String,bool,bool) = sqlx::query_as(
        "SELECT d.conversation_id,o.state,o.domain_receipt_id,
          o.result_sha256=sha256(o.result_json),r.state='settled' AND q.state='settled'
         FROM core_weekly.weekly_discounts d JOIN core_assistant.agent_tool_operations o ON o.id=d.operation_id
         JOIN core_assistant.agent_rounds r ON r.id=o.round_id JOIN core_charging.requests q ON q.id=r.model_request_id
         WHERE d.user_id=$1"
    ).bind(f.user).fetch_one(&pool).await.unwrap();
    assert_eq!(binding.0, cv);
    assert_eq!(binding.1, "completed");
    assert_eq!(
        binding.2,
        format!(
            "core-weekly-{}",
            accepted["decision"]["id"].as_str().unwrap()
        )
    );
    assert!(binding.3 && binding.4);
    f.close().await;
}

#[sqlx::test(migrations = false)]
async fn weekly_zero_decision_consumes_the_week_and_is_readable_with_policy_disabled(pool: PgPool) {
    let f = fixture(&pool).await;
    let (id, original) = offered(&f, 0).await;
    assert_eq!(original["decision"]["status"], "declined");
    sqlx::query("UPDATE core_weekly.weekly_discount_policy SET enabled=FALSE")
        .execute(&pool)
        .await
        .unwrap();
    let cv = conversation(&f, "weekly-zero-history").await;
    let read = run_turn(
        &f,
        &cv,
        "read",
        "Please read my weekly discount status.",
        Some(("get_weekly_discount_status", json!({}))),
    )
    .await
    .unwrap();
    assert_eq!(read["status"], "declined");
    assert_eq!(read["decision_used"], true);
    let repeated = prepare(&f, &cv, "apply", APPLY, 10).await;
    assert_eq!(repeated["created"], false, "{repeated}");
    assert_eq!(repeated["decision"], original["decision"]);
    assert_eq!(decision_count(&pool).await, 1);
    let history = history(&f).await;
    assert_eq!(history["decisions"].as_array().unwrap().len(), 1);
    assert_eq!(history["decisions"][0], original["decision"]);
    let (status, view, nonce) = inspect(&f, &id, &f.cookie()).await;
    assert_eq!(status, reqwest::StatusCode::OK);
    assert_eq!(view["claimable"], false);
    assert!(nonce.is_none());
    assert!(view["decision"].get("code").is_none());
    assert_eq!(
        sqlx::query_scalar::<_, i64>("SELECT count(*) FROM core_payments.discount_codes")
            .fetch_one(&pool)
            .await
            .unwrap(),
        0
    );
    f.close().await;
}

#[sqlx::test(migrations = false)]
async fn weekly_browser_claim_is_private_exactly_once_and_reserves_one_owner_coupon(pool: PgPool) {
    let f = fixture(&pool).await;
    let (id, original) = offered(&f, 5).await;
    let model_hits = f.provider.hits.load(Ordering::SeqCst);
    let before = finance(&pool).await;
    let other_family = f
        .browser
        .login(
            "confirm-user",
            "test-password123",
            &BrowserSessionMetadata::default(),
        )
        .await
        .unwrap();
    let other_cookie = format!("new_api_refresh={}", other_family.refresh_token);
    let (status, other_view, other_nonce) = inspect(&f, &id, &other_cookie).await;
    assert_eq!(status, reqwest::StatusCode::OK);
    assert_eq!(other_view["claimable"], false);
    assert!(other_nonce.is_none());
    assert!(other_view["decision"].get("code").is_none());

    let (status, view, nonce) = inspect(&f, &id, &f.cookie()).await;
    assert_eq!(status, reqwest::StatusCode::OK, "{view}");
    assert_eq!(view["claimable"], true);
    let nonce = nonce.unwrap();
    let csrf = view["csrf"].as_str().unwrap();
    assert_eq!(
        claim(&f, &id, &f.cookie(), &"0".repeat(64), &nonce, &f.origin)
            .await
            .0,
        reqwest::StatusCode::FORBIDDEN
    );
    assert_eq!(
        claim(
            &f,
            &id,
            &f.cookie(),
            csrf,
            &nonce,
            "https://different.example.invalid"
        )
        .await
        .0,
        reqwest::StatusCode::FORBIDDEN
    );
    assert_eq!(
        claim(&f, &id, &other_cookie, csrf, &nonce, &f.origin)
            .await
            .0,
        reqwest::StatusCode::FORBIDDEN
    );
    assert_eq!(finance(&pool).await, before);

    let cookie = f.cookie();
    let (one, two) = tokio::join!(
        claim(&f, &id, &cookie, csrf, &nonce, &f.origin),
        claim(&f, &id, &cookie, csrf, &nonce, &f.origin)
    );
    assert_eq!(one.0, reqwest::StatusCode::OK, "{}", one.1);
    assert_eq!(two.0, reqwest::StatusCode::OK, "{}", two.1);
    assert_eq!(one.1, two.1);
    assert_eq!(one.1["decision"]["status"], "claimed");
    let code = one.1["decision"]["code"].as_str().unwrap();
    assert!(code.starts_with("AIWEEK-"));
    assert_eq!(code.len(), 39);
    let terms:(i64,i32,i64,i64,i64,i64) = sqlx::query_as(
        "SELECT c.id,c.discount_percent,c.max_uses,c.min_credit_units,c.ends_at,w.owner_user_id
         FROM core_payments.discount_codes c JOIN core_payments.weekly_discount_owners w ON w.code_id=c.id WHERE c.code=$1"
    ).bind(code).fetch_one(&pool).await.unwrap();
    assert_eq!((terms.1, terms.2, terms.3), (5, 1, 0));
    assert_eq!(
        terms.4,
        original["decision"]["expires_at"].as_i64().unwrap()
    );
    assert_eq!(terms.5, f.user);
    assert_eq!(
        sqlx::query_scalar::<_, i64>("SELECT count(*) FROM core_payments.discount_codes")
            .fetch_one(&pool)
            .await
            .unwrap(),
        1
    );
    assert_eq!(finance(&pool).await, before);
    assert_eq!(f.provider.hits.load(Ordering::SeqCst), model_hits);
    let immutable:Value = sqlx::query_scalar(
        "SELECT convert_from(o.result_json,'UTF8')::jsonb FROM core_assistant.agent_tool_operations o
         JOIN core_weekly.weekly_discounts d ON d.operation_id=o.id WHERE d.id=$1"
    ).bind(&id).fetch_one(&pool).await.unwrap();
    assert_eq!(immutable, original);

    let store = payments(&f.runtime_role.pool);
    let quote = store
        .quote_discount(f.access(), 88001, 1_000_000, code)
        .await
        .unwrap();
    assert_eq!(
        (
            quote.amount_minor,
            quote.credit_units,
            quote.discount_percent
        ),
        (190, 1_000_000, 5)
    );
    let outsider = f.identity.bootstrap_user(88002, 0).await.unwrap().secret;
    assert_eq!(
        store
            .quote_discount(&outsider, 88001, 1_000_000, code)
            .await
            .unwrap_err(),
        PaymentError::NotFound
    );
    let first = request(&f, "weekly_coupon_reservation_01");
    let intent = store
        .prepare_with_discount(f.access(), first.clone(), code)
        .await
        .unwrap();
    assert_eq!(
        store
            .prepare_with_discount(f.access(), first, code)
            .await
            .unwrap(),
        intent
    );
    assert_eq!(
        store
            .prepare_with_discount(
                f.access(),
                request(&f, "weekly_coupon_reservation_02"),
                code
            )
            .await
            .unwrap_err(),
        PaymentError::Conflict
    );
    let usage: (i64, i64) = sqlx::query_as(
        "SELECT used_count,reserved_count FROM core_payments.discount_code_usage WHERE code_id=$1",
    )
    .bind(terms.0)
    .fetch_one(&pool)
    .await
    .unwrap();
    assert_eq!(usage, (0, 1));
    // Reservation itself is not evidence of external payment or ledger credit.
    let journals:Value = sqlx::query_scalar(
        "SELECT coalesce(jsonb_agg(to_jsonb(j) ORDER BY j.id),'[]'::jsonb) FROM core_billing.ledger_journals j"
    ).fetch_one(&pool).await.unwrap();
    assert_eq!(journals, before["journals"]);
    f.close().await;
}

#[sqlx::test(migrations = false)]
async fn weekly_claim_rechecks_policy_and_coupon_terms_cannot_be_rewritten(pool: PgPool) {
    let f = fixture(&pool).await;
    let (id, _) = offered(&f, 5).await;
    let (_, view, nonce) = inspect(&f, &id, &f.cookie()).await;
    let csrf = view["csrf"].as_str().unwrap();
    let nonce = nonce.unwrap();
    for (enabled, maximum) in [(false, 10_i16), (true, 4_i16)] {
        sqlx::query("UPDATE core_weekly.weekly_discount_policy SET enabled=$1,maximum_percent=$2")
            .bind(enabled)
            .bind(maximum)
            .execute(&pool)
            .await
            .unwrap();
        let denied = claim(&f, &id, &f.cookie(), csrf, &nonce, &f.origin).await;
        assert_eq!(denied.0, reqwest::StatusCode::FORBIDDEN, "{}", denied.1);
        assert_eq!(
            sqlx::query_scalar::<_, i64>("SELECT count(*) FROM core_payments.discount_codes")
                .fetch_one(&pool)
                .await
                .unwrap(),
            0
        );
        assert_eq!(history(&f).await["decisions"].as_array().unwrap().len(), 1);
    }
    sqlx::query("UPDATE core_weekly.weekly_discount_policy SET enabled=TRUE,maximum_percent=10")
        .execute(&pool)
        .await
        .unwrap();
    let accepted = claim(&f, &id, &f.cookie(), csrf, &nonce, &f.origin).await;
    assert_eq!(accepted.0, reqwest::StatusCode::OK, "{}", accepted.1);
    let code_id: i64 =
        sqlx::query_scalar("SELECT code_id FROM core_weekly.weekly_discounts WHERE id=$1")
            .bind(&id)
            .fetch_one(&pool)
            .await
            .unwrap();
    let failed =
        sqlx::query("UPDATE core_payments.discount_codes SET discount_percent=10 WHERE id=$1")
            .bind(code_id)
            .execute(&pool)
            .await
            .unwrap_err();
    assert!(failed.as_database_error().is_some());
    let failed = sqlx::query("DELETE FROM core_payments.discount_codes WHERE id=$1")
        .bind(code_id)
        .execute(&pool)
        .await
        .unwrap_err();
    assert!(failed.as_database_error().is_some());
    sqlx::query("UPDATE core_payments.discount_codes SET enabled=FALSE WHERE id=$1")
        .bind(code_id)
        .execute(&pool)
        .await
        .unwrap();
    assert_eq!(
        payments(&f.runtime_role.pool)
            .quote_discount(
                f.access(),
                88001,
                1_000_000,
                accepted.1["decision"]["code"].as_str().unwrap()
            )
            .await
            .unwrap_err(),
        PaymentError::Denied
    );
    let unrelated:i64 = sqlx::query_scalar(
        "INSERT INTO core_payments.discount_codes(code,name,discount_percent,min_credit_units,max_uses,
         enabled,starts_at,ends_at,created_by)
         VALUES('ORDINARY-TEST','Unrelated ordinary code',10,0,1,TRUE,0,0,$1) RETURNING id"
    ).bind(f.user).fetch_one(&pool).await.unwrap();
    assert_eq!(
        sqlx::query("DELETE FROM core_payments.discount_codes WHERE id=$1")
            .bind(unrelated)
            .execute(&pool)
            .await
            .unwrap()
            .rows_affected(),
        1,
        "the weekly coupon trigger must not suppress unrelated deletes"
    );

    let actor = f.identity.authorize(f.access()).await.unwrap();
    sqlx::query("UPDATE core_identity.credentials SET revoked_at=clock_timestamp() WHERE id=$1")
        .bind(actor.credential_id)
        .execute(&pool)
        .await
        .unwrap();
    let (status, _, _) = inspect(&f, &id, &f.cookie()).await;
    assert!(matches!(
        status,
        reqwest::StatusCode::UNAUTHORIZED | reqwest::StatusCode::FORBIDDEN
    ));
    f.close().await;
}

#[sqlx::test(migrations = false)]
async fn weekly_policy_enforces_the_current_ordinary_role_and_compatible_ten_percent_ceiling(
    pool: PgPool,
) {
    let mut f = fixture(&pool).await;
    let cv = conversation(&f, "weekly-current-role").await;
    run_turn(
        &f,
        &cv,
        "warm",
        "I would like to discuss this week's course project.",
        None,
    )
    .await;
    let excessive = prepare(&f, &cv, "excessive", APPLY, 11).await;
    assert_eq!(excessive["created"], false, "{excessive}");
    assert_eq!(decision_count(&pool).await, 0);
    sqlx::query("UPDATE core_identity.users SET platform_role='admin' WHERE id=$1")
        .bind(f.user)
        .execute(&pool)
        .await
        .unwrap();
    f.bundle = f
        .browser
        .login(
            "confirm-user",
            "test-password123",
            &BrowserSessionMetadata::default(),
        )
        .await
        .unwrap();
    let administrator = prepare(&f, &cv, "admin", APPLY, 0).await;
    assert_eq!(administrator["created"], false, "{administrator}");
    assert_eq!(decision_count(&pool).await, 0);
    sqlx::query("UPDATE core_identity.users SET platform_role='user' WHERE id=$1")
        .bind(f.user)
        .execute(&pool)
        .await
        .unwrap();
    f.bundle = f
        .browser
        .login(
            "confirm-user",
            "test-password123",
            &BrowserSessionMetadata::default(),
        )
        .await
        .unwrap();
    let ordinary = prepare(&f, &cv, "ordinary", APPLY, 10).await;
    assert_eq!(ordinary["created"], true, "{ordinary}");
    assert_eq!(ordinary["decision"]["discount_percent"], 10);
    assert_eq!(decision_count(&pool).await, 1);
    f.close().await;
}

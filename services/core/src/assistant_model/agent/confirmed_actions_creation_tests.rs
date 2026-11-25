//! Real original browser, paid model preparation, immutable one-time key card.
use super::*;

async fn cards(f: &ConfirmFixture, id: &str) -> (reqwest::StatusCode, Value, Option<String>) {
    let r = f
        .http
        .get(format!(
            "{}/api/user/auth/assistant/confirmations/{id}/key",
            f.origin
        ))
        .header("cookie", f.cookie())
        .header("accept", "application/json")
        .send()
        .await
        .unwrap();
    let status = r.status();
    let cookie = r
        .headers()
        .get("set-cookie")
        .map(|v| v.to_str().unwrap().split(';').next().unwrap().to_owned());
    (status, r.json().await.unwrap(), cookie)
}
async fn reveal(
    f: &ConfirmFixture,
    id: &str,
    csrf: &str,
    nonce: &str,
) -> (reqwest::StatusCode, Value) {
    let r = f
        .http
        .post(format!(
            "{}/api/user/auth/assistant/confirmations/{id}/key",
            f.origin
        ))
        .header("origin", &f.origin)
        .header("cookie", format!("{}; {nonce}", f.cookie()))
        .header("x-lmm-assistant-csrf", csrf)
        .json(&json!({"reveal":true}))
        .send()
        .await
        .unwrap();
    (r.status(), r.json().await.unwrap())
}
async fn key_count(pool: &PgPool) -> i64 {
    sqlx::query_scalar("SELECT count(*) FROM core_identity.credentials WHERE kind='api_key'")
        .fetch_one(pool)
        .await
        .unwrap()
}
async fn prepare_creation(f: &ConfirmFixture, name: &str) -> String {
    f.prepare(
        actions::CREATE_TOOL,
        json!({"name":name,"group":"allowed"}),
        "Create one API Key in the allowed group and show its exact browser preview.",
        false,
    )
    .await
}
#[sqlx::test(migrations = false)]
async fn confirmed_key_creation_issues_once_and_original_browser_consumes_secret_once(
    pool: PgPool,
) {
    let f = fixture(&pool, 300).await;
    // Retired L1/developer access is not creation authority.
    assert!(
        !sqlx::query_scalar::<_, bool>(
            "SELECT EXISTS(SELECT FROM core_identity.developer_access WHERE user_id=$1)"
        )
        .bind(f.user)
        .fetch_one(&pool)
        .await
        .unwrap()
    );
    let id = prepare_creation(&f, "one-private-key").await;
    let (_, view, nonce) = f.inspect(&id).await;
    let nonce = nonce.unwrap();
    let csrf = view["csrf"].as_str().unwrap();
    assert_eq!(key_count(&pool).await, 0);
    let model_calls = f.provider.hits.load(Ordering::SeqCst);
    let journals: i64 = sqlx::query_scalar("SELECT count(*) FROM core_billing.ledger_journals")
        .fetch_one(&pool)
        .await
        .unwrap();
    // Make both real browser transactions wait at the versioned gate before
    // either can approve. The second rebuilds only its empty gate transaction,
    // then reads and returns the exact same already committed receipt.
    let mut gate = pool.begin().await.unwrap();
    sqlx::query("SELECT id FROM core_humangrant.human_grant_gate WHERE id = 1 FOR UPDATE")
        .fetch_one(&mut *gate)
        .await
        .unwrap();
    let gate_pid: i32 = sqlx::query_scalar("SELECT pg_backend_pid()")
        .fetch_one(&mut *gate)
        .await
        .unwrap();
    let posts = async {
        tokio::join!(
            f.post(&id, csrf, &nonce, json!({"decision":"approve"})),
            f.post(&id, csrf, &nonce, json!({"decision":"approve"}))
        )
    };
    let observe = pool.clone();
    let release = async move {
        loop {
            let waiting: i64 = sqlx::query_scalar(
                "SELECT count(*) FROM pg_stat_activity
                 WHERE datname=current_database() AND pid<>$1
                   AND state='active' AND wait_event_type='Lock'
                   AND query='UPDATE core_humangrant.human_grant_gate SET revision = revision WHERE id = 1 RETURNING id'",
            )
            .bind(gate_pid)
            .fetch_one(&observe)
            .await
            .unwrap();
            if waiting == 2 {
                break;
            }
            tokio::task::yield_now().await;
        }
        gate.rollback().await.unwrap();
    };
    // These are owned futures, so a failure drops the held transaction too.
    let ((a, b), ()) = tokio::time::timeout(std::time::Duration::from_secs(5), async {
        tokio::join!(posts, release)
    })
    .await
    .expect("both browser approvals must wait at the gate and finish after release");
    assert_eq!(a.0, reqwest::StatusCode::OK);
    assert_eq!(b.0, reqwest::StatusCode::OK);
    assert_eq!(a.1["receipt"], b.1["receipt"]);
    assert_eq!(a.1["executed"], true);
    assert_eq!(key_count(&pool).await, 1);
    assert_eq!(audit_count(&pool).await, 1);
    assert!(!a.1.to_string().contains("lmmk_"));
    assert_eq!(f.provider.hits.load(Ordering::SeqCst), model_calls);
    assert_eq!(
        sqlx::query_scalar::<_, i64>("SELECT count(*) FROM core_billing.ledger_journals")
            .fetch_one(&pool)
            .await
            .unwrap(),
        journals
    );
    let credential = a.1["receipt"]["execution_receipt"]["resource_id"]
        .as_i64()
        .unwrap();
    let (status, card, card_cookie) = cards(&f, &id).await;
    assert_eq!(status, reqwest::StatusCode::OK);
    assert_eq!(card["key_card"]["credential_id"], credential);
    assert_eq!(card["key_card"]["state"], "ready");
    assert!(card.get("api_key").is_none());
    assert!(!card.to_string().contains("ciphertext"));
    let saved:(bool,bool,i64)=sqlx::query_as(
        "SELECT p.ciphertext IS NOT NULL,s.encrypted_secret IS NULL,p.expires_at-p.created_at
         FROM core_assistant_tools.key_creation_cards p JOIN core_identity.api_key_settings s ON s.credential_id=p.credential_id
         WHERE p.confirmation_id=$1").bind(&id).fetch_one(&pool).await.unwrap();
    assert_eq!(saved, (true, true, 600));
    let unauthorized = f
        .http
        .post(format!(
            "{}/api/user/auth/assistant/confirmations/{id}/key",
            f.origin
        ))
        .bearer_auth(f.access())
        .header("origin", &f.origin)
        .header("cookie", card_cookie.as_deref().unwrap())
        .header("x-lmm-assistant-csrf", card["csrf"].as_str().unwrap())
        .json(&json!({"reveal":true}))
        .send()
        .await
        .unwrap();
    assert!(!unauthorized.status().is_success());
    let card_nonce = card_cookie.unwrap();
    let card_csrf = card["csrf"].as_str().unwrap();
    let (first, second) = tokio::join!(
        reveal(&f, &id, card_csrf, &card_nonce),
        reveal(&f, &id, card_csrf, &card_nonce)
    );
    assert_eq!(first.0, reqwest::StatusCode::OK);
    assert_eq!(second.0, reqwest::StatusCode::OK);
    assert_eq!(
        usize::from(first.1.get("api_key").is_some())
            + usize::from(second.1.get("api_key").is_some()),
        1
    );
    let secret = first
        .1
        .get("api_key")
        .or_else(|| second.1.get("api_key"))
        .unwrap()
        .as_str()
        .unwrap();
    let actor = f.identity.authorize(secret).await.unwrap();
    assert_eq!(actor.user_id, f.user);
    assert_eq!(actor.owner_account_id, f.account);
    assert_eq!(actor.credential_id, credential);
    let erased:(bool,bool)=sqlx::query_as(
        "SELECT ciphertext IS NULL,consumed_at IS NOT NULL FROM core_assistant_tools.key_creation_cards WHERE confirmation_id=$1")
        .bind(&id).fetch_one(&pool).await.unwrap();
    assert_eq!(erased, (true, true));
    let history:Vec<String>=sqlx::query_scalar(
        "SELECT convert_from(result_json,'UTF8') FROM core_assistant.agent_tool_operations WHERE result_json IS NOT NULL")
        .fetch_all(&pool).await.unwrap();
    assert!(history.iter().all(|value| !value.contains(secret)));
    let previews:Vec<String>=sqlx::query_scalar(
        "SELECT preview::text||COALESCE(execution_receipt::text,'') FROM core_assistant_tools.confirmations")
        .fetch_all(&pool).await.unwrap();
    assert!(previews.iter().all(|value| !value.contains(secret)));
    assert!(
        f.provider
            .requests
            .lock()
            .unwrap()
            .iter()
            .all(|value| !value.to_string().contains(secret))
    );
    let read = cards(&f, &id).await;
    assert_eq!(read.1["key_card"]["state"], "consumed");
    assert!(read.1.get("api_key").is_none());
    let replay = f
        .post(&id, csrf, &nonce, json!({"decision":"approve"}))
        .await;
    assert_eq!(replay.1["receipt"], a.1["receipt"]);
    assert_eq!(key_count(&pool).await, 1);
    assert!(replay.1.get("api_key").is_none());
    f.close().await;
}
#[sqlx::test(migrations = false)]
async fn confirmed_key_creation_mfa_failure_keeps_flow_and_all_issuance_rows_untouched(
    pool: PgPool,
) {
    let mut f = fixture(&pool, 300).await;
    let setup = f.browser.mfa_setup(f.access()).await.unwrap();
    let enrollment = code_at(&setup.secret, epoch(&pool).await);
    f.bundle = f.browser.mfa_enable(f.access(), &enrollment).await.unwrap();
    let id = prepare_creation(&f, "factor-private-key").await;
    let (_, view, nonce) = f.inspect(&id).await;
    let nonce = nonce.unwrap();
    assert_eq!(view["confirmation"]["requires_two_factor"], true);
    let csrf = view["csrf"].as_str().unwrap();
    let bad = f
        .post(
            &id,
            csrf,
            &nonce,
            json!({"decision":"approve","two_factor_code":"xxxxxx"}),
        )
        .await;
    assert_eq!(bad.1["error"], "two_factor_code_invalid");
    assert_eq!(key_count(&pool).await, 0);
    assert_eq!(audit_count(&pool).await, 0);
    assert_eq!(
        sqlx::query_scalar::<_, i64>(
            "SELECT count(*) FROM core_assistant_tools.key_creation_cards"
        )
        .fetch_one(&pool)
        .await
        .unwrap(),
        0
    );
    assert_eq!(
        sqlx::query_scalar::<_, i32>(
            "SELECT failed_attempts FROM core_identity.mfa_factors WHERE user_id=$1"
        )
        .bind(f.user)
        .fetch_one(&pool)
        .await
        .unwrap(),
        1
    );
    let current = code_at(&setup.secret, epoch(&pool).await + 30);
    let good = f
        .post(
            &id,
            csrf,
            &nonce,
            json!({"decision":"approve","two_factor_code":current}),
        )
        .await;
    assert_eq!(good.0, reqwest::StatusCode::OK);
    assert_eq!(good.1["executed"], true);
    assert_eq!(key_count(&pool).await, 1);
    assert_eq!(audit_count(&pool).await, 1);
    assert!(good.1.get("api_key").is_none());
    assert!(!good.1.to_string().contains(&setup.secret));
    f.close().await;
}
#[sqlx::test(migrations = false)]
async fn confirmed_key_creation_current_group_cas_and_private_card_binding_fail_closed(
    pool: PgPool,
) {
    let f = fixture(&pool, 300).await;
    let original_credential = f
        .identity
        .authorize(f.access())
        .await
        .unwrap()
        .credential_id;
    let id = prepare_creation(&f, "stale-group-key").await;
    let (_, view, nonce) = f.inspect(&id).await;
    let nonce = nonce.unwrap();
    sqlx::query("UPDATE core_identity.user_routing_groups SET enabled=FALSE WHERE user_id=$1 AND group_name='allowed'")
        .bind(f.user).execute(&pool).await.unwrap();
    let result = f
        .post(
            &id,
            view["csrf"].as_str().unwrap(),
            &nonce,
            json!({"decision":"approve"}),
        )
        .await;
    assert!(!result.0.is_success());
    assert_eq!(key_count(&pool).await, 0);
    assert_eq!(audit_count(&pool).await, 0);
    let mut tx = pool.begin().await.unwrap();
    let forged=sqlx::query(
        "INSERT INTO core_assistant_tools.key_creation_cards(confirmation_id,credential_id,ciphertext,created_at,expires_at)
         VALUES($1,$2,$3,1,601)")
        .bind(&id).bind(original_credential)
        .bind(vec![7u8;64]).execute(&mut *tx).await;
    forged.unwrap();
    let rejected = tx.commit().await.unwrap_err();
    assert_eq!(
        rejected
            .as_database_error()
            .and_then(|e| e.code())
            .as_deref(),
        Some("23514")
    );
    assert_eq!(
        sqlx::query_scalar::<_, i64>(
            "SELECT count(*) FROM core_assistant_tools.key_creation_cards"
        )
        .fetch_one(&pool)
        .await
        .unwrap(),
        0
    );
    f.close().await;
}

//! The invitation domain suspends while Core keeps the original paid operation claimed.
//! This covers Core's browser authorization contract; SMTP delivery remains Go-owned.
use super::*;
use crate::assistant_tools_core::{Error as ToolError, confirmation::ConfirmationStore};

async fn financial_state(pool: &PgPool) -> Value {
    sqlx::query_scalar(
        "SELECT jsonb_build_object(
         'journals',(SELECT coalesce(jsonb_agg(to_jsonb(j) ORDER BY id),'[]'::jsonb)
                     FROM core_billing.ledger_journals j),
         'balances',(SELECT coalesce(jsonb_agg(to_jsonb(b) ORDER BY b.ledger_account_id),'[]'::jsonb)
                     FROM core_billing.balance_state b),
         'codes',(SELECT coalesce(jsonb_agg(to_jsonb(c) ORDER BY code),'[]'::jsonb)
                  FROM core_identity.affiliate_codes c),
         'registrations',(SELECT count(*) FROM core_identity.affiliate_registrations))",
    )
    .fetch_one(pool)
    .await
    .unwrap()
}

#[sqlx::test(migrations = false)]
async fn invitation_confirmation_requires_original_pending_paid_operation_and_browser_decision(
    pool: PgPool,
) {
    let f = fixture_options(&pool, 300, false, true).await;
    let code = "a".repeat(32);
    sqlx::query("INSERT INTO core_identity.affiliate_codes(code,user_id) VALUES($1,$2)")
        .bind(&code)
        .bind(f.user)
        .execute(&pool)
        .await
        .unwrap();
    sqlx::query(
        "UPDATE core_identity.user_profiles SET email='sender@example.invalid' WHERE user_id=$1",
    )
    .bind(f.user)
    .execute(&pool)
    .await
    .unwrap();
    let arguments = json!({"email":"recipient+tag@example.invalid"});
    f.queue.lock().unwrap().push_back(Some((
        "send_invitation".into(),
        serde_json::to_string(&arguments).unwrap(),
    )));
    let conversation = f
        .agent
        .base
        .conversation(f.access(), "invitation-browser-authorization", true)
        .await
        .unwrap()
        .core_conversation_id;
    f.agent
        .start(
            f.access(),
            &conversation,
            "turn1",
            "Prepare one invitation for recipient+tag@example.invalid",
        )
        .await
        .unwrap();
    let pending = wait(&f.agent, f.access(), &conversation, "tools_pending").await;
    assert!(pending.settled_credits > 0);
    assert_eq!(pending.held_credits, 0);
    assert_eq!(pending.pending_calls.len(), 1);
    let call = &pending.pending_calls[0];
    assert_eq!(call.scope.name, "send_invitation");
    assert_eq!(call.scope.domain, "extension");
    assert_eq!(
        serde_json::from_slice::<Value>(&call.arguments_json).unwrap(),
        arguments
    );
    let paid: (String, String, bool) = sqlx::query_as(
        "SELECT r.state,p.state,p.assistant_grant_id IS NOT NULL
         FROM core_assistant.agent_tool_operations o
         JOIN core_assistant.agent_rounds r ON r.id=o.round_id
         JOIN core_charging.requests p ON p.id=r.model_request_id
         WHERE o.id=$1 AND p.actor_user_id=$2 AND p.owner_account_id=$3",
    )
    .bind(&call.id)
    .bind(f.user)
    .bind(f.account)
    .fetch_one(&pool)
    .await
    .unwrap();
    assert_eq!(paid, ("settled".to_string(), "settled".to_string(), true));
    assert!(
        f.agent
            .claim(f.access(), &call.id, "invitation-domain-claim")
            .await
            .unwrap()
            .may_execute
    );
    let draft = json!({
        "kind":"invitation_email_v1",
        "email":"recipient+tag@example.invalid",
        "invitation_code":code,
    });
    let store = ConfirmationStore::new(pool.clone());
    let mut changed = draft.clone();
    changed["email"] = json!("different@example.invalid");
    assert!(matches!(
        store.prepare(f.access(), &call.id, &changed).await,
        Err(ToolError::Conflict)
    ));
    changed = draft.clone();
    changed["invitation_code"] = json!("b".repeat(32));
    assert!(matches!(
        store.prepare(f.access(), &call.id, &changed).await,
        Err(ToolError::Conflict)
    ));
    let prepared = store.prepare(f.access(), &call.id, &draft).await.unwrap();
    assert_eq!(prepared.state, "pending");
    assert_eq!(prepared.tool_name, "send_invitation");
    assert_eq!(prepared.preview, draft);
    assert!(!prepared.executed);
    assert!(prepared.execution_receipt.is_none());
    assert_eq!(
        prepared.browser_path,
        format!(
            "/api/user/auth/assistant/confirmations/{}",
            prepared.challenge_id
        )
    );
    let digest = Sha256::digest(serde_json::to_vec(&draft).unwrap());
    assert_eq!(prepared.draft_sha256, crate::hex_codec::encode(digest));
    assert!(matches!(
        store.receipt(f.access(), &call.id, digest.as_slice()).await,
        Err(ToolError::Forbidden)
    ));

    // Reproduce the worker's lock order with two real transactions. The runtime
    // prepare must wait on this exact conversation before taking operation rows.
    // Reserve its connection first so the two-connection runtime pool cannot be
    // exhausted by a background wakeup waiting behind the gate.
    let operation_before: Value = sqlx::query_scalar(
        "SELECT to_jsonb(o) FROM core_assistant.agent_tool_operations o WHERE id=$1",
    )
    .bind(&call.id)
    .fetch_one(&pool)
    .await
    .unwrap();
    let finances_before = financial_state(&pool).await;
    let mut preparing = f.agent.base.pool.begin().await.unwrap();
    let preparing_pid: i32 = sqlx::query_scalar("SELECT pg_backend_pid()")
        .fetch_one(&mut *preparing)
        .await
        .unwrap();
    let context = crate::assistant_tools_core::context::from_session(&mut preparing, f.access())
        .await
        .unwrap();
    let mut gate = pool.begin().await.unwrap();
    crate::identity::authorize_in_transaction(&mut gate, f.access())
        .await
        .unwrap();
    crate::assistant_model::lock_conversation_id(&mut gate, &conversation)
        .await
        .unwrap();
    sqlx::query("SELECT id FROM core_assistant.agent_turns WHERE conversation_id=$1 AND client_turn_id='turn1' FOR UPDATE")
        .bind(&conversation)
        .fetch_one(&mut *gate)
        .await
        .unwrap();
    let gate_pid: i32 = sqlx::query_scalar("SELECT pg_backend_pid()")
        .fetch_one(&mut *gate)
        .await
        .unwrap();
    let prepare_id = call.id.clone();
    let prepare_draft = draft.clone();
    let prepare = async move {
        let challenge =
            crate::assistant_tools_core::confirmation::prepare_from_operation_in_transaction(
                &mut preparing,
                &context,
                &prepare_id,
                &prepare_draft,
            )
            .await
            .unwrap();
        preparing.commit().await.unwrap();
        challenge
    };
    let observe_pool = pool.clone();
    let gate_operation = call.id.clone();
    let release_gate = async move {
        loop {
            let waiting: bool = sqlx::query_scalar(
                "SELECT EXISTS(
                 SELECT FROM pg_locks waiting JOIN pg_locks held
                   USING (locktype,database,classid,objid,objsubid)
                 WHERE waiting.pid=$1 AND held.pid=$2
                   AND waiting.locktype='advisory' AND NOT waiting.granted AND held.granted)",
            )
            .bind(preparing_pid)
            .bind(gate_pid)
            .fetch_one(&observe_pool)
            .await
            .unwrap();
            if waiting {
                break;
            }
            tokio::task::yield_now().await;
        }
        // While prepare waits, a worker holding the turn must still be able to
        // lock its operation immediately: there is no operation -> turn cycle.
        let locked: String = sqlx::query_scalar(
            "SELECT id FROM core_assistant.agent_tool_operations WHERE id=$1 FOR UPDATE NOWAIT",
        )
        .bind(&gate_operation)
        .fetch_one(&mut *gate)
        .await
        .unwrap();
        assert_eq!(locked, gate_operation);
        gate.rollback().await.unwrap();
    };
    // No detached task survives timeout/panic: dropping these futures drops
    // both SQLx transactions and rolls back their locks.
    let (replayed, ()) = tokio::time::timeout(Duration::from_secs(5), async {
        tokio::join!(prepare, release_gate)
    })
    .await
    .expect("prepare must wait on the conversation gate and complete after its release");
    assert_eq!(replayed.challenge_id, prepared.challenge_id);
    assert_eq!(replayed.state, "pending");
    assert_eq!(replayed.preview, draft);
    let operation_after: Value = sqlx::query_scalar(
        "SELECT to_jsonb(o) FROM core_assistant.agent_tool_operations o WHERE id=$1",
    )
    .bind(&call.id)
    .fetch_one(&pool)
    .await
    .unwrap();
    assert_eq!(operation_after, operation_before);
    assert_eq!(financial_state(&pool).await, finances_before);

    // Match the real Go Pending behavior: it saves awaiting_confirmation in its
    // own journal and deliberately does NOT call Core CompleteAgentTool. Polling,
    // duplicate claims and a repeated prepare cannot advance the paid turn.
    for _ in 0..2 {
        let replay = f
            .agent
            .claim(f.access(), &call.id, "invitation-domain-claim")
            .await
            .unwrap();
        assert!(!replay.may_execute);
        assert!(replay.accepted.is_none());
        let snapshot = f
            .agent
            .get(f.access(), &conversation, "turn1")
            .await
            .unwrap();
        assert_eq!(snapshot.phase, "tools_pending");
        assert!(snapshot.final_text.is_none());
        assert_eq!(snapshot.round, pending.round);
        assert_eq!(snapshot.pending_calls[0].id, call.id);
        assert_eq!(
            store
                .prepare(f.access(), &call.id, &draft)
                .await
                .unwrap()
                .challenge_id,
            prepared.challenge_id
        );
    }
    let operation: (String, bool, i64) = sqlx::query_as(
        "SELECT state,result_json IS NULL,
         (SELECT count(*) FROM core_assistant.agent_tool_observations WHERE operation_id=o.id)
         FROM core_assistant.agent_tool_operations o WHERE id=$1",
    )
    .bind(&call.id)
    .fetch_one(&pool)
    .await
    .unwrap();
    assert_eq!(operation, ("claimed".to_string(), true, 0));
    assert_eq!(f.provider.hits.load(Ordering::SeqCst), 1);

    // Another browser login belongs to the same user but is not the original
    // family/credential bound to this provider operation and its confirmation.
    let other = f
        .browser
        .login(
            "confirm-user",
            "test-password123",
            &BrowserSessionMetadata::default(),
        )
        .await
        .unwrap();
    let other_access = other.data["access_token"].as_str().unwrap();
    assert!(matches!(
        store.prepare(other_access, &call.id, &draft).await,
        Err(ToolError::Forbidden)
    ));
    let response = f
        .http
        .get(format!("{}{}", f.origin, prepared.browser_path))
        .header("cookie", format!("new_api_refresh={}", other.refresh_token))
        .send()
        .await
        .unwrap();
    assert_eq!(response.status(), reqwest::StatusCode::FORBIDDEN);
    let before = financial_state(&pool).await;
    let (status, view, nonce) = f.inspect(&prepared.challenge_id).await;
    assert_eq!(status, reqwest::StatusCode::OK, "{view}");
    assert_eq!(view["confirmation"]["preview"], draft);
    assert_eq!(view["confirmation"]["state"], "pending");
    let nonce = nonce.unwrap();
    let csrf = view["csrf"].as_str().unwrap();
    let denied = f
        .post(
            &prepared.challenge_id,
            &"0".repeat(64),
            &nonce,
            json!({"decision":"approve"}),
        )
        .await;
    assert_eq!(denied.0, reqwest::StatusCode::FORBIDDEN);
    let approved = f
        .post(
            &prepared.challenge_id,
            csrf,
            &nonce,
            json!({"decision":"approve"}),
        )
        .await;
    assert_eq!(approved.0, reqwest::StatusCode::OK, "{}", approved.1);
    assert_eq!(approved.1["receipt"]["state"], "approved");
    assert_eq!(approved.1["receipt"]["operation_id"], call.id);
    assert_eq!(approved.1["receipt"]["draft_sha256"], prepared.draft_sha256);
    assert_eq!(approved.1["executed"], false);
    assert!(approved.1["receipt"].get("execution_receipt").is_none());

    let receipt = store
        .receipt(f.access(), &call.id, digest.as_slice())
        .await
        .unwrap();
    assert_eq!(receipt.challenge_id, prepared.challenge_id);
    assert_eq!(receipt.state, "approved");
    assert!(!receipt.executed); // Authorizes Go's same operation; Core sends no email.
    assert!(matches!(
        store.receipt(f.access(), &call.id, &[0; 32]).await,
        Err(ToolError::Conflict)
    ));
    let replay = f
        .post(
            &prepared.challenge_id,
            csrf,
            &nonce,
            json!({"decision":"approve"}),
        )
        .await;
    assert_eq!(replay.0, reqwest::StatusCode::OK, "{}", replay.1);
    assert_eq!(replay.1["receipt"], approved.1["receipt"]);
    assert_eq!(financial_state(&pool).await, before);
    assert_eq!(
        sqlx::query_scalar::<_, i64>(
            "SELECT count(*) FROM core_assistant_tools.confirmations WHERE operation_id=$1"
        )
        .bind(&call.id)
        .fetch_one(&pool)
        .await
        .unwrap(),
        1
    );
    assert_eq!(f.provider.hits.load(Ordering::SeqCst), 1);

    let actor = f.identity.authorize(f.access()).await.unwrap();
    sqlx::query("UPDATE core_identity.credentials SET revoked_at=clock_timestamp() WHERE id=$1")
        .bind(actor.credential_id)
        .execute(&pool)
        .await
        .unwrap();
    assert!(matches!(
        store.receipt(f.access(), &call.id, digest.as_slice()).await,
        Err(ToolError::Unauthorized | ToolError::Forbidden)
    ));
    assert_eq!(financial_state(&pool).await, before);
    f.close().await;
}

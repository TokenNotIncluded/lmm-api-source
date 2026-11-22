//! Explicit cancellation of one reviewed read never erases an uncertain effect.
//! All model requests, identity decisions and durable rows use the real Core.
use super::*;
use crate::identity::{BrowserAuth, BrowserRegistration, BrowserSessionMetadata};
use std::collections::BTreeMap;

const MUTABLE: [&str; 3] = [
    "core_assistant.agent_turns",
    "core_assistant.agent_tool_operations",
    "core_assistant.grants",
];

#[derive(Clone, Debug, PartialEq)]
struct CoreState {
    unchanged: BTreeMap<String, String>,
    mutable: BTreeMap<String, Vec<Value>>,
}

async fn core_state(owner: &PgPool) -> CoreState {
    let relations: Vec<(String, String, String)> = sqlx::query_as(
        "SELECT n.nspname,c.relname,c.relkind::text
         FROM pg_catalog.pg_class c JOIN pg_catalog.pg_namespace n ON n.oid=c.relnamespace
         WHERE c.relkind IN ('r','p','S')
           AND (left(n.nspname,5)='core_' OR n.nspname='public')
         ORDER BY n.nspname,c.relname",
    )
    .fetch_all(owner)
    .await
    .unwrap();
    let mut state = CoreState {
        unchanged: BTreeMap::new(),
        mutable: BTreeMap::new(),
    };
    for (schema, relation, kind) in relations {
        for name in [&schema, &relation] {
            assert!(
                !name.is_empty() && name.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'_')
            );
        }
        let name = format!("{schema}.{relation}");
        if MUTABLE.contains(&name.as_str()) {
            let rows: Value = sqlx::query_scalar(sqlx::AssertSqlSafe(format!(
                "SELECT COALESCE(jsonb_agg(to_jsonb(t) ORDER BY t.id),'[]'::jsonb)
                 FROM \"{schema}\".\"{relation}\" t"
            )))
            .fetch_one(owner)
            .await
            .unwrap();
            state.mutable.insert(name, rows.as_array().unwrap().clone());
        } else {
            let hash: String = sqlx::query_scalar(sqlx::AssertSqlSafe(format!(
                "SELECT encode(sha256(convert_to(
                   COALESCE(jsonb_agg(to_jsonb(t) ORDER BY to_jsonb(t)::text),'[]'::jsonb)::text,
                   'UTF8')),'hex') FROM \"{schema}\".\"{relation}\" t"
            )))
            .fetch_one(owner)
            .await
            .unwrap();
            state.unchanged.insert(format!("{kind}:{name}"), hash);
        }
    }
    assert_eq!(state.mutable.len(), MUTABLE.len());
    assert!(
        state
            .unchanged
            .keys()
            .any(|name| name.ends_with(":core_charging.requests"))
    );
    state
}

async fn original_grants(owner: &PgPool, turn: &str) -> Vec<String> {
    let grants = sqlx::query_scalar(
        "SELECT q.assistant_grant_id FROM core_assistant.agent_rounds r
         JOIN core_charging.requests q ON q.id=r.model_request_id
         WHERE r.turn_id=$1 ORDER BY r.round",
    )
    .bind(turn)
    .fetch_all(owner)
    .await
    .unwrap();
    assert!(!grants.is_empty());
    grants
}

fn after_cancel(mut before: CoreState, turn: &str, grants: &[String], expired: bool) -> CoreState {
    let mut turns = 0;
    for row in before.mutable.get_mut(MUTABLE[0]).unwrap() {
        if row["id"] == turn {
            turns += 1;
            if expired {
                row["phase"] = json!("expired");
                row["revision"] = json!(row["revision"].as_i64().unwrap() + 1);
            }
        }
    }
    assert_eq!(turns, 1);
    for row in before.mutable.get_mut(MUTABLE[1]).unwrap() {
        if row["turn_id"] == turn && row["state"] == "pending" {
            row["state"] = json!("cancelled");
        }
    }
    let mut revoked = 0;
    for row in before.mutable.get_mut(MUTABLE[2]).unwrap() {
        if grants.iter().any(|grant| row["id"] == *grant) {
            row["revoked"] = json!(true);
            revoked += 1;
        }
    }
    assert_eq!(revoked, grants.len());
    before
}

async fn original_receipts(owner: &PgPool, turn: &str) -> Value {
    sqlx::query_scalar(
        "SELECT jsonb_build_object(
          'models',(SELECT coalesce(jsonb_agg(jsonb_build_object('round',to_jsonb(r),
             'charge',to_jsonb(q),'operation',to_jsonb(o)) ORDER BY r.round),'[]'::jsonb)
            FROM core_assistant.agent_rounds r JOIN core_charging.requests q ON q.id=r.model_request_id
            JOIN core_assistant.operations o ON o.request_id=q.id WHERE r.turn_id=$1),
          'tools',(SELECT coalesce(jsonb_agg(to_jsonb(o) ORDER BY o.id),'[]'::jsonb)
            FROM core_assistant.agent_tool_operations o WHERE o.turn_id=$1),
          'observations',(SELECT coalesce(jsonb_agg(to_jsonb(b) ORDER BY b.position),'[]'::jsonb)
            FROM core_assistant.agent_tool_observations b JOIN core_assistant.agent_tool_operations o
              ON o.id=b.operation_id WHERE o.turn_id=$1),
          'grants',(SELECT coalesce(jsonb_agg(to_jsonb(g) ORDER BY g.id),'[]'::jsonb)
            FROM core_assistant.grants g JOIN core_charging.requests q ON q.assistant_grant_id=g.id
            JOIN core_assistant.agent_rounds r ON r.model_request_id=q.id WHERE r.turn_id=$1))",
    ).bind(turn).fetch_one(owner).await.unwrap()
}

async fn wait_turn(
    f: &Fixture,
    session: &str,
    conversation: &str,
    client: &str,
    phase: &str,
) -> TurnSnapshot {
    for _ in 0..600 {
        match f.agent.get(session, conversation, client).await {
            Ok(snapshot) if snapshot.phase == phase => return snapshot,
            Ok(snapshot) => assert!(!matches!(
                snapshot.phase.as_str(),
                "unknown" | "rejected" | "expired"
            )),
            Err(Error::Conflict) => {}
            Err(error) => panic!("unexpected agent error: {error:?}"),
        }
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    panic!("turn {client} did not reach {phase}");
}

async fn prepare(
    f: &Fixture,
    session: &str,
    label: &str,
    calls: Value,
) -> (String, String, Vec<ToolCall>) {
    {
        let mut queue = f.provider.queue.lock().unwrap();
        queue.clear();
        queue.push_back(json!({"role":"assistant","content":null,"tool_calls":calls}));
        queue.push_back(
            json!({"role":"assistant","content":"Fresh turn without cancelled private data."}),
        );
    }
    let cv = f
        .agent
        .base
        .conversation(session, label, true)
        .await
        .unwrap()
        .core_conversation_id;
    f.agent
        .start(
            session,
            &cv,
            "turn1",
            "Read the requested administrator skills.",
        )
        .await
        .unwrap();
    let ready = wait_turn(f, session, &cv, "turn1", "tools_pending").await;
    assert!(ready.settled_credits > 0);
    assert_eq!(ready.held_credits, 0);
    assert!(f.agent.base.drain(Duration::from_secs(5)).await);
    let turn = sqlx::query_scalar("SELECT id FROM core_assistant.agent_turns WHERE conversation_id=$1 AND client_turn_id='turn1'")
        .bind(&cv).fetch_one(&f.agent.base.pool).await.unwrap();
    (cv, turn, ready.pending_calls)
}

fn private_call(id: &str) -> Value {
    json!({"id":id,"type":"function","function":{"name":"get_admin_user_skills","arguments":format!("{{\"target_user_id\":{TARGET}}}")}})
}

async fn claim(f: &Fixture, session: &str, call: &ToolCall, key: &str) {
    assert!(
        f.agent
            .claim(session, &call.id, key)
            .await
            .unwrap()
            .may_execute
    );
}

async fn finish(
    f: &Fixture,
    session: &str,
    call: &ToolCall,
    key: &str,
    state: &str,
    result: &[u8],
) -> Result<(), Error> {
    let receipt = if state == "completed" {
        format!("service:{}", call.id)
    } else {
        String::new()
    };
    f.agent
        .complete(
            session,
            Completion {
                operation_id: &call.id,
                claim_key: key,
                state,
                result_json: result,
                result_sha256: &Sha256::digest(result),
                domain_receipt_id: &receipt,
            },
        )
        .await
        .map(|_| ())
}

async fn new_turn_without_old_private_data(f: &Fixture, session: &str, cv: &str) {
    let hits = f.provider.hits.load(Ordering::SeqCst);
    f.agent
        .start(
            session,
            cv,
            "turn2",
            "Answer this fresh turn without private skills.",
        )
        .await
        .unwrap();
    assert_eq!(
        wait_turn(f, session, cv, "turn2", "completed")
            .await
            .final_text
            .as_deref(),
        Some("Fresh turn without cancelled private data.")
    );
    assert!(f.agent.base.drain(Duration::from_secs(5)).await);
    assert_eq!(f.provider.hits.load(Ordering::SeqCst), hits + 1);
    let request = f.provider.requests.lock().unwrap().last().unwrap().clone();
    assert!(!request["messages"].to_string().contains(MARKER));
}

async fn stable_expired_snapshot(f: &Fixture, session: &str, cv: &str, stopped: &TurnSnapshot) {
    assert_eq!(stopped.phase, "expired");
    assert!(stopped.pending_calls.is_empty());
    let again = f.agent.get(session, cv, "turn1").await.unwrap();
    assert!(again.pending_calls.is_empty());
    assert_eq!(
        (
            again.phase,
            again.revision,
            again.round,
            again.held_credits,
            again.settled_credits,
            again.final_text
        ),
        (
            stopped.phase.clone(),
            stopped.revision,
            stopped.round,
            stopped.held_credits,
            stopped.settled_credits,
            stopped.final_text.clone()
        )
    );
}

#[sqlx::test(migrations = false)]
async fn cancelled_claimed_private_read_preserves_money_and_allows_a_new_turn(pool: PgPool) {
    let f = fixture(&pool).await;
    let math = json!({"id":"unclaimed_math","type":"function","function":{"name":"calculate_math","arguments":"{\"expression\":\"1+1\"}"}});
    let (cv, turn, calls) = prepare(
        &f,
        &f.session.secret,
        "cancel-claimed-read",
        json!([private_call("read"), math]),
    )
    .await;
    assert_eq!(calls.len(), 2);
    let private = &calls[0];
    claim(&f, &f.session.secret, private, "original-private-claim").await;
    let result = body(&f).await;
    sqlx::query("UPDATE core_identity.users SET auth_version=auth_version+1 WHERE id=901")
        .execute(&pool)
        .await
        .unwrap();
    assert!(matches!(
        accept(&f, private, &result).await,
        Err(Error::Forbidden)
    ));
    let before = core_state(&pool).await;
    let grants = original_grants(&pool, &turn).await;
    let stopped = f
        .agent
        .cancel(&f.session.secret, &cv, "turn1")
        .await
        .unwrap();
    stable_expired_snapshot(&f, &f.session.secret, &cv, &stopped).await;
    assert_eq!(stopped.held_credits, 0);
    assert!(matches!(
        f.agent
            .claim(&f.session.secret, &calls[1].id, "late-math")
            .await,
        Err(Error::Conflict)
    ));
    assert!(matches!(
        accept(&f, private, &result).await,
        Err(Error::Conflict)
    ));
    assert!(matches!(
        finish(
            &f,
            &f.session.secret,
            private,
            "original-private-claim",
            "unknown",
            br#"{"status":"unknown"}"#
        )
        .await,
        Err(Error::Conflict)
    ));
    assert_eq!(
        core_state(&pool).await,
        after_cancel(before, &turn, &grants, true)
    );
    let receipt = original_receipts(&pool, &turn).await;
    assert_eq!(receipt["models"].as_array().unwrap().len(), 1);
    new_turn_without_old_private_data(&f, &f.session.secret, &cv).await;
    assert_eq!(original_receipts(&pool, &turn).await, receipt);
    f.close().await;
}

#[sqlx::test(migrations = false)]
async fn cancelled_unknown_read_does_not_replay_old_private_results_into_the_new_turn(
    pool: PgPool,
) {
    let f = fixture(&pool).await;
    let (cv, turn, calls) = prepare(
        &f,
        &f.session.secret,
        "cancel-unknown-read",
        json!([private_call("accepted_read"), private_call("unknown_read")]),
    )
    .await;
    assert_eq!(calls.len(), 2);
    let result = body(&f).await;
    claim(&f, &f.session.secret, &calls[0], "accepted-private").await;
    finish(
        &f,
        &f.session.secret,
        &calls[0],
        "accepted-private",
        "completed",
        &result,
    )
    .await
    .unwrap();
    assert!(f.agent.base.drain(Duration::from_secs(5)).await);
    claim(&f, &f.session.secret, &calls[1], "unknown-private").await;
    let unknown =
        serde_json::to_vec(&json!({"status":"unknown","unaccepted_private_data":MARKER})).unwrap();
    finish(
        &f,
        &f.session.secret,
        &calls[1],
        "unknown-private",
        "unknown",
        &unknown,
    )
    .await
    .unwrap();
    sqlx::query("UPDATE core_identity.users SET platform_role='admin' WHERE id=901")
        .execute(&pool)
        .await
        .unwrap();
    let before = core_state(&pool).await;
    let grants = original_grants(&pool, &turn).await;
    let stopped = f
        .agent
        .cancel(&f.session.secret, &cv, "turn1")
        .await
        .unwrap();
    stable_expired_snapshot(&f, &f.session.secret, &cv, &stopped).await;
    // A previously accepted UNKNOWN receipt is still queryable and repeatable;
    // it does not authorize a late successful result or another model request.
    finish(
        &f,
        &f.session.secret,
        &calls[1],
        "unknown-private",
        "unknown",
        &unknown,
    )
    .await
    .unwrap();
    let recovery = f
        .agent
        .claim(&f.session.secret, &calls[1].id, "unknown-private")
        .await
        .unwrap();
    assert!(!recovery.may_execute);
    let accepted = recovery.accepted.unwrap();
    assert_eq!(accepted.state, "unknown");
    assert_eq!(
        accepted.result_sha256.as_slice(),
        Sha256::digest(&unknown).as_slice()
    );
    assert!(accepted.domain_receipt_id.is_empty());
    assert!(matches!(
        f.agent
            .claim(&f.session.secret, &calls[1].id, "different-claim")
            .await,
        Err(Error::Conflict)
    ));
    assert!(matches!(
        finish(
            &f,
            &f.session.secret,
            &calls[1],
            "unknown-private",
            "completed",
            &result
        )
        .await,
        Err(Error::Conflict)
    ));
    assert!(matches!(
        finish(
            &f,
            &f.session.secret,
            &calls[0],
            "accepted-private",
            "completed",
            &result
        )
        .await,
        Err(Error::Forbidden)
    ));
    assert_eq!(
        core_state(&pool).await,
        after_cancel(before, &turn, &grants, true)
    );
    let receipt = original_receipts(&pool, &turn).await;
    new_turn_without_old_private_data(&f, &f.session.secret, &cv).await;
    assert_eq!(original_receipts(&pool, &turn).await, receipt);
    f.close().await;
}

#[sqlx::test(migrations = false)]
async fn private_read_with_an_unknown_invitation_still_blocks_the_next_turn(pool: PgPool) {
    let f = fixture_with_tools(&pool, &["send_invitation"]).await;
    let invite = json!({"id":"invite","type":"function","function":{"name":"send_invitation","arguments":"{\"email\":\"synthetic@example.invalid\"}"}});
    let (cv, turn, calls) = prepare(
        &f,
        &f.session.secret,
        "mixed-unknown-effect",
        json!([private_call("read"), invite]),
    )
    .await;
    assert_eq!(calls.len(), 2);
    claim(&f, &f.session.secret, &calls[0], "original-private-claim").await;
    accept(&f, &calls[0], &body(&f).await).await.unwrap();
    assert!(f.agent.base.drain(Duration::from_secs(5)).await);
    claim(&f, &f.session.secret, &calls[1], "invitation-claim").await;
    finish(
        &f,
        &f.session.secret,
        &calls[1],
        "invitation-claim",
        "unknown",
        br#"{"status":"unknown"}"#,
    )
    .await
    .unwrap();
    let before = core_state(&pool).await;
    let grants = original_grants(&pool, &turn).await;
    assert_eq!(
        f.agent
            .cancel(&f.session.secret, &cv, "turn1")
            .await
            .unwrap()
            .phase,
        "tools_pending"
    );
    assert!(matches!(
        f.agent
            .start(
                &f.session.secret,
                &cv,
                "turn2",
                "A new turn must remain blocked."
            )
            .await,
        Err(Error::Conflict)
    ));
    assert_eq!(
        core_state(&pool).await,
        after_cancel(before, &turn, &grants, false)
    );
    assert_eq!(f.provider.hits.load(Ordering::SeqCst), 1);
    // No SMTP call is made: this tests Core's real durable UNKNOWN boundary.
    f.close().await;
}

#[sqlx::test(migrations = false)]
async fn read_cancellation_requires_the_original_browser_session(pool: PgPool) {
    let f = fixture(&pool).await;
    let identity = IdentityStore::from_pool(f.runtime_role.pool.clone())
        .with_registration_enabled(true)
        .with_mfa_keys("test".into(), vec![("test".into(), vec![41; 32])])
        .unwrap();
    let browser = BrowserAuth::new(identity.clone(), &[43; 32]).unwrap();
    let username = "cancel-browser-admin";
    let password = "synthetic passphrase";
    browser
        .register(&BrowserRegistration {
            username,
            password,
            email: "",
        })
        .await
        .unwrap();
    let user: i64 =
        sqlx::query_scalar("SELECT user_id FROM core_identity.user_profiles WHERE username=$1")
            .bind(username)
            .fetch_one(&pool)
            .await
            .unwrap();
    sqlx::query("UPDATE core_identity.users SET platform_role='admin' WHERE id=$1")
        .bind(user)
        .execute(&pool)
        .await
        .unwrap();
    let first = browser
        .login(username, password, &BrowserSessionMetadata::default())
        .await
        .unwrap();
    let second = browser
        .login(username, password, &BrowserSessionMetadata::default())
        .await
        .unwrap();
    let session = first.data["access_token"].as_str().unwrap();
    let replacement = second.data["access_token"].as_str().unwrap();
    let actor = identity.authorize(session).await.unwrap();
    let other = identity.authorize(replacement).await.unwrap();
    assert_eq!(
        (actor.user_id, actor.credential_generation()),
        (other.user_id, other.credential_generation())
    );
    assert_ne!(actor.credential_id, other.credential_id);
    sqlx::query("INSERT INTO core_identity.user_routing_groups(user_id,group_name,enabled,source_reference) VALUES($1,'allowed',TRUE,'synthetic-cancel-browser')")
        .bind(user).execute(&pool).await.unwrap();
    let ledger = Ledger::from_pool(pool.clone());
    ledger.open_wallet(actor.owner_account_id).await.unwrap();
    ledger
        .execute(&Request {
            scope: "admin-cancel-test".into(),
            operation_key: "browser-funds".into(),
            actor_user_id: Some(user),
            reason: "Disposable paid browser cancellation fixture".into(),
            action: Action::Credit {
                account_id: actor.owner_account_id,
                amount_units: Credits::try_from(400000).unwrap(),
                payment_reference: "synthetic:cancel-browser".into(),
            },
        })
        .await
        .unwrap();
    let (cv, turn, calls) = prepare(
        &f,
        session,
        "original-browser-session",
        json!([private_call("read")]),
    )
    .await;
    claim(&f, session, &calls[0], "browser-private").await;
    let before = core_state(&pool).await;
    assert!(matches!(
        f.agent.cancel(replacement, &cv, "turn1").await,
        Err(Error::Forbidden)
    ));
    assert_eq!(core_state(&pool).await, before);
    let grants = original_grants(&pool, &turn).await;
    assert_eq!(
        f.agent.cancel(session, &cv, "turn1").await.unwrap().phase,
        "expired"
    );
    assert!(matches!(
        finish(
            &f,
            replacement,
            &calls[0],
            "browser-private",
            "completed",
            br#"{"ok":true}"#
        )
        .await,
        Err(Error::Forbidden)
    ));
    assert_eq!(
        core_state(&pool).await,
        after_cancel(before, &turn, &grants, true)
    );
    let receipt = original_receipts(&pool, &turn).await;
    new_turn_without_old_private_data(&f, session, &cv).await;
    assert_eq!(original_receipts(&pool, &turn).await, receipt);
    drop(browser);
    drop(identity);
    f.close().await;
}

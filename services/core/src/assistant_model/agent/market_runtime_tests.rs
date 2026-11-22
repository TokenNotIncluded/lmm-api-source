//! Real paid model claim and native browser consent, then the actual Core
//! reserve/dispatch boundary. No external MCP or settlement is simulated.
use super::*;
use crate::tool_market::{
    Owner,
    assistant_runtime::{
        AssistantCatalogueRequest, AssistantMarketRuntime, AssistantReserveRequest,
    },
    credit_access::{CreditStore, IdRequest},
    credit_execution::ReserveRequest,
};

fn owner(f: &ConfirmFixture) -> Owner {
    Owner {
        user: f.user.to_string(),
        account: f.user.to_string(),
    }
}
fn credits(f: &ConfirmFixture) -> CreditStore {
    CreditStore::from_pool(f.agent.base.pool.clone()).with_assistant_runtime(
        AssistantMarketRuntime::from_policy(f.agent.policy.clone()).unwrap(),
    )
}
async fn reviewed_fixed_policy(f: &ConfirmFixture) {
    let pool = &f.owner_pool;
    f.browser
        .register(&BrowserRegistration {
            username: "market-platform",
            password: "test-password123",
            email: "",
        })
        .await
        .unwrap();
    let bundle = f
        .browser
        .login(
            "market-platform",
            "test-password123",
            &BrowserSessionMetadata::default(),
        )
        .await
        .unwrap();
    let actor = f
        .identity
        .authorize(bundle.data["access_token"].as_str().unwrap())
        .await
        .unwrap();
    assert_ne!(actor.user_id, f.user);
    sqlx::query("UPDATE core_identity.users SET platform_role='superadmin' WHERE id=$1")
        .bind(actor.user_id)
        .execute(pool)
        .await
        .unwrap();
    crate::billing::ledger::Ledger::from_pool(pool.clone())
        .open_wallet(actor.owner_account_id)
        .await
        .unwrap();
    sqlx::query("UPDATE core_tool_market.credit_policy SET enabled=TRUE,platform_user_id=$1,platform_account_id=$2,updated_by=$1,updated_at=extract(epoch FROM clock_timestamp())::bigint WHERE singleton")
        .bind(actor.user_id).bind(actor.owner_account_id).execute(pool).await.unwrap();
    let quote_id: String = sqlx::query_scalar(
        "SELECT id FROM core_tool_market.credit_quotes WHERE snapshot->>'service_id'=$1",
    )
    .bind(market_confirmation::SERVICE)
    .fetch_one(pool)
    .await
    .unwrap();
    // Owner-installed immutable fixed-success review is a prerequisite for
    // a positive-price hold. This is synthetic policy, never a result witness.
    let schema = json!({"type":"object","properties":{"ok":{"type":"boolean"}},"additionalProperties":false});
    let digest = crate::hex_codec::encode(Sha256::digest(serde_json::to_vec(&schema).unwrap()));
    sqlx::query("INSERT INTO core_tool_market.credit_result_contracts(quote_id,proof_mode,witness_role,output_schema,output_schema_digest) VALUES($1,'core_host_fixed_success','synthetic-market-witness',$2,$3)")
        .bind(quote_id).bind(sqlx::types::Json(schema)).bind(digest).execute(pool).await.unwrap();
}
async fn paid_call(f: &ConfirmFixture, grant: &str) -> (String, AssistantReserveRequest) {
    let mut args = market_confirmation::arguments();
    let inner = json!({"query":"exact <tag> & \u{2028} input","limit":2});
    args["grant_id"] = json!(grant);
    args["arguments"] = inner.clone();
    f.queue.lock().unwrap().push_back(Some((
        "call_market_tool".into(),
        serde_json::to_string(&args).unwrap(),
    )));
    let conversation = f
        .agent
        .base
        .conversation(
            f.access(),
            &crate::assistant_model::random("market-paid-call"),
            true,
        )
        .await
        .unwrap()
        .core_conversation_id;
    f.agent
        .start(
            f.access(),
            &conversation,
            "turn1",
            "Call this exact reviewed tool within my approved limit.",
        )
        .await
        .unwrap();
    let pending = wait(&f.agent, f.access(), &conversation, "tools_pending").await;
    assert_eq!(pending.pending_calls.len(), 1);
    assert!(pending.settled_credits > 0);
    let operation = pending.pending_calls[0].id.clone();
    let quote_id: String =
        sqlx::query_scalar("SELECT quote_id FROM core_tool_market.credit_consents WHERE id=$1")
            .bind(grant)
            .fetch_one(&f.agent.base.pool)
            .await
            .unwrap();
    let resolve_by: i64 =
        sqlx::query_scalar("SELECT floor(extract(epoch FROM transaction_timestamp()))::bigint+120")
            .fetch_one(&f.agent.base.pool)
            .await
            .unwrap();
    let request = ReserveRequest {
        owner: owner(f),
        client_id: "builtin-assistant".into(),
        request_key: operation.clone(),
        quote_id,
        grant_id: grant.into(),
        tool_id: args["tool_id"].as_str().unwrap().into(),
        version_id: args["version_id"].as_str().unwrap().into(),
        input_digest: crate::builtins::input_digest(&inner).unwrap(),
        resolve_by,
    };
    (
        operation.clone(),
        AssistantReserveRequest {
            request,
            assistant_tool_operation_id: operation,
        },
    )
}
async fn approved_call(f: &ConfirmFixture) -> (String, String, AssistantReserveRequest) {
    reviewed_fixed_policy(f).await;
    let approved = market_confirmation::approve(f).await;
    let grant = approved["receipt"]["execution_receipt"]["resource_id"]
        .as_str()
        .unwrap()
        .to_owned();
    let confirmation = sqlx::query_scalar::<_, String>(
        "SELECT id FROM core_assistant_tools.confirmations WHERE operation_id=$1",
    )
    .bind(approved["receipt"]["operation_id"].as_str().unwrap())
    .fetch_one(&f.agent.base.pool)
    .await
    .unwrap();
    let (_, request) = paid_call(f, &grant).await;
    (confirmation, grant, request)
}
async fn claim(f: &ConfirmFixture, request: &AssistantReserveRequest) {
    assert!(
        f.agent
            .claim(
                f.access(),
                &request.assistant_tool_operation_id,
                "market-host-claim"
            )
            .await
            .unwrap()
            .may_execute
    );
}
async fn assert_historical_binding(f: &ConfirmFixture, id: &str) {
    assert!(
        sqlx::query_scalar::<_, bool>("SELECT core_assistant_tools.market_confirmation_bound($1)")
            .bind(id)
            .fetch_one(&f.agent.base.pool)
            .await
            .unwrap()
    );
}
async fn calls(f: &ConfirmFixture) -> i64 {
    sqlx::query_scalar("SELECT count(*) FROM core_tool_market.credit_calls")
        .fetch_one(&f.agent.base.pool)
        .await
        .unwrap()
}

#[sqlx::test(migrations = false)]
async fn assistant_market_reserve_requires_paid_claim_exact_inputs_and_single_dispatch(
    pool: PgPool,
) {
    let f = fixture_market(&pool, 60).await;
    let (confirmation, grant, request) = approved_call(&f).await;
    let store = credits(&f);
    assert!(
        store.reserve_assistant(f.access(), &request).await.is_err(),
        "unclaimed model output cannot reserve"
    );
    assert!(
        store.reserve(f.access(), &request.request).await.is_err(),
        "ordinary endpoint cannot mint assistant authority"
    );
    claim(&f, &request).await;
    assert!(
        CreditStore::from_pool(f.agent.base.pool.clone())
            .reserve_assistant(f.access(), &request)
            .await
            .is_err()
    );
    let mut mutations = Vec::new();
    let mut changed = request.clone();
    changed.request.input_digest = "f".repeat(64);
    mutations.push(changed);
    let mut changed = request.clone();
    changed.request.grant_id = "e".repeat(64);
    mutations.push(changed);
    let mut changed = request.clone();
    changed.request.version_id = "different-version".into();
    mutations.push(changed);
    let mut changed = request.clone();
    changed.request.owner.user = (f.user + 1000).to_string();
    mutations.push(changed);
    let mut changed = request.clone();
    changed.assistant_tool_operation_id = "d".repeat(64);
    changed.request.request_key = changed.assistant_tool_operation_id.clone();
    mutations.push(changed);
    for changed in mutations {
        assert!(store.reserve_assistant(f.access(), &changed).await.is_err());
        assert_eq!(calls(&f).await, 0);
    }
    let other_session = f
        .browser
        .login(
            "confirm-user",
            "test-password123",
            &BrowserSessionMetadata::default(),
        )
        .await
        .unwrap();
    assert!(
        store
            .reserve_assistant(
                other_session.data["access_token"].as_str().unwrap(),
                &request
            )
            .await
            .is_err(),
        "a second real login cannot redeem the original paid operation"
    );
    let before = crate::billing::ledger::Ledger::from_pool(pool.clone())
        .balance(f.account)
        .await
        .unwrap();
    let reserved = store.reserve_assistant(f.access(), &request).await.unwrap();
    assert_eq!(reserved.held_quota, 7);
    assert_eq!(reserved.state, "reserved");
    let after = crate::billing::ledger::Ledger::from_pool(pool.clone())
        .balance(f.account)
        .await
        .unwrap();
    assert_eq!(after.available_units, before.available_units - 7);
    assert_eq!(after.reserved_units, before.reserved_units + 7);
    let replay = store.reserve_assistant(f.access(), &request).await.unwrap();
    assert_eq!(replay, reserved);
    assert_eq!(calls(&f).await, 1);
    let counters: (i64, i64) = sqlx::query_as(
        "SELECT reserved_calls,reserved_credits FROM core_tool_market.credit_consents WHERE id=$1",
    )
    .bind(&grant)
    .fetch_one(&pool)
    .await
    .unwrap();
    assert_eq!(counters, (1, 7));
    assert_historical_binding(&f, &confirmation).await;

    let mut policy = (*f.agent.policy).clone();
    policy.market.as_mut().unwrap().revision = "withdrawn-policy-revision".into();
    let changed_runtime = CreditStore::from_pool(f.agent.base.pool.clone())
        .with_assistant_runtime(AssistantMarketRuntime::from_policy(Arc::new(policy)).unwrap());
    assert!(
        changed_runtime
            .dispatch(f.access(), &reserved.call_id)
            .await
            .is_err()
    );
    let hits = f.provider.hits.load(Ordering::SeqCst);
    let dispatched = store.dispatch(f.access(), &reserved.call_id).await.unwrap();
    assert_eq!(dispatched.state, "running");
    assert!(dispatched.dispatch_won);
    let replay = store.dispatch(f.access(), &reserved.call_id).await.unwrap();
    assert_eq!(replay.dispatch_id, dispatched.dispatch_id);
    assert!(!replay.dispatch_won);
    assert_eq!(f.provider.hits.load(Ordering::SeqCst), hits);
    assert_eq!(calls(&f).await, 1);
    assert_historical_binding(&f, &confirmation).await;
    let snapshot: Value =
        sqlx::query_scalar("SELECT snapshot FROM core_tool_market.credit_calls WHERE id=$1")
            .bind(&reserved.call_id)
            .fetch_one(&pool)
            .await
            .unwrap();
    assert_eq!(
        snapshot["assistant_tool_operation_id"],
        request.assistant_tool_operation_id
    );
    f.close().await;
}

#[sqlx::test(migrations = false)]
async fn assistant_market_revocation_after_hold_blocks_dispatch_without_rewriting_history(
    pool: PgPool,
) {
    let f = fixture_market(&pool, 60).await;
    let (confirmation, grant, request) = approved_call(&f).await;
    claim(&f, &request).await;
    let store = credits(&f);
    let reserved = store.reserve_assistant(f.access(), &request).await.unwrap();
    store
        .revoke(
            f.access(),
            &IdRequest {
                owner: owner(&f),
                operation_id: "browser-revoke-original-market-consent".into(),
                id: grant,
            },
            false,
        )
        .await
        .unwrap();
    assert!(store.dispatch(f.access(), &reserved.call_id).await.is_err());
    assert_eq!(
        store
            .query(f.access(), &reserved.call_id)
            .await
            .unwrap()
            .unwrap()
            .state,
        "reserved"
    );
    assert_eq!(
        store.reserve_assistant(f.access(), &request).await.unwrap(),
        reserved,
        "read-only replay observes the original hold and cannot dispatch"
    );
    assert_eq!(calls(&f).await, 1);
    assert_historical_binding(&f, &confirmation).await;
    f.close().await;
}

#[sqlx::test(migrations = false)]
async fn assistant_market_changed_auth_generation_cannot_redeem_or_dispatch_original_hold(
    pool: PgPool,
) {
    let f = fixture_market(&pool, 60).await;
    let (confirmation, _, request) = approved_call(&f).await;
    claim(&f, &request).await;
    let store = credits(&f);
    let reserved = store.reserve_assistant(f.access(), &request).await.unwrap();
    sqlx::query("UPDATE core_identity.users SET auth_version=auth_version+1 WHERE id=$1")
        .bind(f.user)
        .execute(&pool)
        .await
        .unwrap();
    assert!(store.reserve_assistant(f.access(), &request).await.is_err());
    assert!(store.dispatch(f.access(), &reserved.call_id).await.is_err());
    assert_eq!(calls(&f).await, 1);
    assert_historical_binding(&f, &confirmation).await;
    f.close().await;
}

#[sqlx::test(migrations = false)]
async fn assistant_market_catalogue_reads_only_live_owned_exact_version_consents(pool: PgPool) {
    let f = fixture_market(&pool, 60).await;
    let approved = market_confirmation::approve(&f).await;
    let grant = approved["receipt"]["execution_receipt"]["resource_id"]
        .as_str()
        .unwrap()
        .to_owned();
    let args = market_confirmation::arguments();
    let request = AssistantCatalogueRequest {
        owner: owner(&f),
        service_id: market_confirmation::SERVICE.into(),
        version_id: args["version_id"].as_str().unwrap().into(),
        tool_ids: vec![args["tool_id"].as_str().unwrap().into()],
        grant_id: String::new(),
    };
    let store = credits(&f);
    let first = store
        .assistant_catalogue(f.access(), &request)
        .await
        .unwrap();
    assert_eq!(first.consents.len(), 1);
    assert_eq!(first.consents[0].id, grant);
    assert_eq!(first.consents[0].reserved_quota, 0);
    assert_eq!(first.policy_revision, "confirmation-market-policy-v1");
    assert_eq!(calls(&f).await, 0);
    let mut exact = request.clone();
    exact.grant_id = grant.clone();
    assert_eq!(
        store
            .assistant_catalogue(f.access(), &exact)
            .await
            .unwrap()
            .consents
            .len(),
        1
    );
    let mut wrong = request.clone();
    wrong.owner.account = "0".into();
    assert!(store.assistant_catalogue(f.access(), &wrong).await.is_err());
    let mut wrong = request.clone();
    wrong.service_id = "unapproved-service".into();
    assert!(store.assistant_catalogue(f.access(), &wrong).await.is_err());
    let mut wrong = request.clone();
    wrong.tool_ids = vec!["a".into(), "b".into(), "c".into()];
    assert!(store.assistant_catalogue(f.access(), &wrong).await.is_err());
    let mut policy = (*f.agent.policy).clone();
    policy.allowed_tools.remove("get_connected_market_tools");
    let disabled = CreditStore::from_pool(f.agent.base.pool.clone())
        .with_assistant_runtime(AssistantMarketRuntime::from_policy(Arc::new(policy)).unwrap());
    assert!(
        disabled
            .assistant_catalogue(f.access(), &request)
            .await
            .is_err()
    );
    store
        .revoke(
            f.access(),
            &IdRequest {
                owner: owner(&f),
                operation_id: "browser-revoke-catalogue-grant".into(),
                id: grant,
            },
            false,
        )
        .await
        .unwrap();
    assert!(
        store
            .assistant_catalogue(f.access(), &request)
            .await
            .unwrap()
            .consents
            .is_empty()
    );
    assert!(
        store
            .assistant_catalogue(f.access(), &exact)
            .await
            .unwrap()
            .consents
            .is_empty()
    );
    assert_eq!(calls(&f).await, 0);
    f.close().await;
}

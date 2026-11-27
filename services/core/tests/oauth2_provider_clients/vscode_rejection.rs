//! Actual VS Code HTTP request identifiers correlate provider observations and wallet holds.
use super::pi;
use serde::Serialize;
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use sqlx::PgPool;
use std::{path::Path, sync::atomic::Ordering};

pub(super) async fn interrupt_zero_settlement(owner: &PgPool) {
    // Fail the final transaction only. Core commits its own transport evidence first.
    sqlx::raw_sql(
        "CREATE FUNCTION public.block_vscode_zero() RETURNS trigger LANGUAGE plpgsql AS $$
         BEGIN RAISE EXCEPTION 'synthetic zero settlement interruption'; END $$;
         CREATE TRIGGER block_vscode_zero BEFORE UPDATE ON core_charging.requests
         FOR EACH ROW WHEN (NEW.state='settled' AND NEW.settled=0)
         EXECUTE FUNCTION public.block_vscode_zero()",
    )
    .execute(owner)
    .await
    .unwrap();
}

#[derive(Debug, PartialEq, Serialize, sqlx::FromRow)]
struct Receipt {
    request_id: String,
    state: String,
    outcome: Option<String>,
    input: i64,
    cached: i64,
    output: i64,
    settled_credits: i64,
    refunded_credits: i64,
    reserved: i64,
    closed: bool,
    captured: i64,
    capture_journal_id: Option<i64>,
    final_events: i64,
}

async fn receipt(f: &pi::Fixture, client: &str, id: &str) -> Receipt {
    let mut rows: Vec<Receipt> = sqlx::query_as(
        "SELECT r.id AS request_id,r.state,r.outcome,r.input_tokens AS input,r.cached_tokens AS cached,
         r.output_tokens AS output,r.settled AS settled_credits,r.refunded AS refunded_credits,
         h.reserved,h.closed,h.captured,h.capture_journal_id,
         (SELECT count(*) FROM core_charging.usage_events e WHERE e.request_id=r.id AND e.event_id='final') AS final_events
         FROM core_charging.requests r JOIN core_charging.ledger_holds h ON h.request_id=r.id
         JOIN core_oauth.families family ON family.id=r.oauth_family_id AND family.client_id=r.oauth_client_id
         WHERE r.id=$1 AND r.oauth_client_id=$2 AND r.payer_account_id=$3 AND r.actor_user_id=$4 AND r.key_id IS NULL
         AND family.user_id=r.actor_user_id AND family.owner_account_id=r.payer_account_id
         AND family.issuance_kind='browser' AND family.purpose='normal_model'",
    ).bind(id).bind(client).bind(f.account).bind(f.user).fetch_all(&f.owner).await.unwrap();
    assert_eq!(rows.len(), 1, "one linked wallet hold per HTTP request");
    rows.pop().unwrap()
}

fn assert_pending(row: &Receipt) {
    assert_eq!(row.state, "reconcile");
    assert!(row.reserved > 0 && !row.closed);
    assert_eq!(
        (
            row.captured,
            row.settled_credits,
            row.refunded_credits,
            row.final_events
        ),
        (0, 0, 0, 0)
    );
    assert_eq!(row.capture_journal_id, None);
}

pub(super) async fn recover(f: &pi::Fixture, client: &str, wire_path: &Path) -> Value {
    let lines = std::fs::read_to_string(wire_path).unwrap();
    let events: Vec<(u16, String)> = lines
        .lines()
        .map(|l| serde_json::from_str(l).unwrap())
        .collect();
    let [success_wire, unknown_wire, reviewed_wire] = events.as_slice() else {
        panic!("expected three responses from actual VS Code model POSTs");
    };
    assert_eq!(
        (success_wire.0, unknown_wire.0, reviewed_wire.0),
        (200, 502, 502)
    );
    assert_eq!(
        events
            .iter()
            .map(|e| &e.1)
            .collect::<std::collections::BTreeSet<_>>()
            .len(),
        3
    );
    let success = receipt(f, client, &success_wire.1).await;
    let unknown = receipt(f, client, &unknown_wire.1).await;
    let pending = receipt(f, client, &reviewed_wire.1).await;
    assert_eq!(
        (
            success.state.as_str(),
            success.input,
            success.cached,
            success.output,
            success.settled_credits,
            success.refunded_credits
        ),
        ("settled", 10, 2, 20, 49, 0)
    );
    assert!(
        success.closed
            && success.capture_journal_id.is_some()
            && success.final_events == 1
            && success.captured == 49
    );
    assert_pending(&unknown);
    assert_pending(&pending);
    let observations: Vec<(String, Value)> = sqlx::query_as(
        "SELECT request_id,detail FROM core_charging.usage_events WHERE request_id IN ($1,$2)
         AND event_id='model-no-acceptance:v1'",
    )
    .bind(&unknown.request_id)
    .bind(&pending.request_id)
    .fetch_all(&f.owner)
    .await
    .unwrap();
    let [(id, proof)] = observations.as_slice() else {
        panic!("only the structured refusal has no-acceptance evidence");
    };
    assert_eq!(id, &pending.request_id);
    assert_eq!(proof["kind"], "model_no_acceptance");
    assert_eq!(proof["receipt"]["outcome"], "rejected");
    assert_eq!(proof["receipt"]["http_status"], 429);
    let raw = serde_json::to_vec(&json!({"error":{"type":"rate_limit_error","code":"rate_limit_exceeded","message":"request rejected before generation"}})).unwrap();
    let digest: [u8; 32] = Sha256::digest(raw).into();
    assert_eq!(proof["receipt"]["receipt_sha256"], json!(digest));
    let before = f.ledger.balance(f.account).await.unwrap();
    assert_eq!(before.reserved_units, unknown.reserved + pending.reserved);
    assert_eq!(
        before.available_units + before.reserved_units,
        pi::WALLET_CREDITS - 49
    );

    sqlx::raw_sql("DROP TRIGGER block_vscode_zero ON core_charging.requests; DROP FUNCTION public.block_vscode_zero()")
        .execute(&f.owner).await.unwrap();
    let recovered = f.gateway.reconcile_completed_streams(64).await.unwrap();
    let replay = f.gateway.reconcile_completed_streams(64).await.unwrap();
    assert_eq!((recovered, replay), (1, 0));
    let closed = receipt(f, client, &reviewed_wire.1).await;
    assert_eq!(
        (
            closed.state.as_str(),
            closed.outcome.as_deref(),
            closed.captured,
            closed.settled_credits,
            closed.refunded_credits,
            closed.final_events
        ),
        ("settled", Some("failed"), 0, 0, 0, 1)
    );
    assert!(closed.closed && closed.capture_journal_id.is_some());
    assert_eq!(closed.reserved, pending.reserved);
    assert_eq!(receipt(f, client, &unknown_wire.1).await, unknown);
    assert_eq!(receipt(f, client, &success_wire.1).await, success);
    let after = f.ledger.balance(f.account).await.unwrap();
    assert_eq!(after.reserved_units, unknown.reserved);
    assert_eq!(
        after.available_units,
        before.available_units + pending.reserved
    );
    assert_eq!(
        after.available_units + after.reserved_units,
        pi::WALLET_CREDITS - 49
    );
    assert_eq!(f.provider.hits.load(Ordering::SeqCst), 3);
    json!({"requests":[success,unknown,closed],"recovery":{
        "unknown_request_id":unknown_wire.1,"confirmed_request_id":reviewed_wire.1,
        "confirmed_before":pending,"provider_receipt_sha256": digest.iter().map(|b| format!("{b:02x}")).collect::<String>(),
        "held_before":before.reserved_units,"held_after":after.reserved_units,
        "released_credits":after.available_units-before.available_units,"first_scan":recovered,"repeat_scan":replay
    }})
}

//! Real relay/PG recovery after a committed terminal callback loses confirmation.
//! This is a contract fault injection, not a PostgreSQL COMMIT-ACK network fault
//! or a claim about the cause of the historical Pi timeout. Native-key admission
//! exercises the same ordinary token charging path, without testing Pi OAuth.
use super::*;
use sqlx::Connection as _;
use tokio::sync::{Mutex, Notify};

struct CommittedTerminalDelay {
    inner: Arc<super::super::Bridge>,
    blocker: Arc<Mutex<Option<sqlx::PgConnection>>>,
    committed: Arc<Notify>,
}

impl Billing for CommittedTerminalDelay {
    fn reserve<'a>(
        &'a self,
        context: &'a RequestContext,
        route: &'a Route,
        request: &'a relay::Request,
    ) -> relay::RelayFuture<'a, relay::Reservation> {
        self.inner.reserve(context, route, request)
    }

    fn reserve_prepared<'a>(
        &'a self,
        context: &'a RequestContext,
        route: &'a Route,
        request: &'a relay::Request,
        body: &'a [u8],
        observation: Option<&'a relay::CountObservation>,
    ) -> relay::RelayFuture<'a, relay::Reservation> {
        self.inner
            .reserve_prepared(context, route, request, body, observation)
    }

    fn checkpoint<'a>(
        &'a self,
        reservation: &'a relay::Reservation,
        checkpoint: &'a OutputCheckpoint,
    ) -> relay::RelayFuture<'a, ()> {
        Box::pin(async move {
            self.inner.checkpoint(reservation, checkpoint).await?;
            if checkpoint.terminal_observed {
                // The real checkpoint has committed. Keep confirmation pending
                // until the unchanged relay hook deadline cancels this future.
                // The independently owned session survives that cancellation.
                let mut connection = self.blocker.lock().await;
                sqlx::query("SELECT pg_advisory_lock(hashtextextended('core_charging:' || $1,0))")
                    .bind(&reservation.id)
                    .execute(connection.as_mut().unwrap())
                    .await
                    .unwrap();
                drop(connection);
                self.committed.notify_one();
                std::future::pending::<()>().await;
            }
            Ok(())
        })
    }

    fn finalize<'a>(
        &'a self,
        reservation: &'a relay::Reservation,
        report: &'a relay::Report,
    ) -> relay::RelayFuture<'a, ()> {
        // Forward unchanged: the real finalizer first waits for the request
        // lock when reading its frozen contract, before provider-final/settle.
        self.inner.finalize(reservation, report)
    }
}

fn stream_gateway(f: &Fixture, base: &str) -> ModelGateway {
    // Match tests/common/pi_oauth_fixture.rs: at 500,000 credits/USD, usage
    // 10/20/cache2 costs 48.5 rounded up to 49; bounds 100/20 reserve 140.
    let original = gateway_with_output_rate(f, base, vec![], 4_000_000);
    let mut document = original.bridge.routing.catalog_document().unwrap();
    document.routing.routes.truncate(1);
    document.pricing.rules.truncate(1);
    document.pricing.rules[0].rates.cache_read = 500_000;
    ModelGateway::new(
        f.pool.clone(),
        Arc::new(
            routing::Router::new(
                document.routing,
                Arc::new(PriceSnapshot::compile(document.pricing).unwrap()),
            )
            .unwrap(),
        ),
        vec![UpstreamCredential {
            reference: CredentialRef { id: 7, revision: 1 },
            secret: "synthetic-checkpoint-recovery-key".into(),
        }],
        BTreeMap::from([(f.key_id, "allowed".into())]),
        original.limits.clone(),
        ChargePolicy {
            input_token_ceiling: 100,
            lease_seconds: 30,
        },
    )
    .unwrap()
    .with_local_streaming()
}

#[sqlx::test(migrations = false)]
async fn committed_terminal_hook_timeout_recovers_original_charge(pool: PgPool) {
    const ID: &str = "committed-terminal-hook-timeout";
    let f = fixture(pool, 1000).await;
    let provider = CompatibilityProvider::start_with_usage(10, 20, 2).await;
    let mut running = stream_gateway(&f, &provider.endpoint);
    assert_eq!(running.limits.hook_timeout, Duration::from_secs(2));
    // A dedicated connection is never returned to the pool with a session lock.
    let blocker = Arc::new(Mutex::new(Some(
        sqlx::PgConnection::connect_with(f.pool.connect_options().as_ref())
            .await
            .unwrap(),
    )));
    let committed = Arc::new(Notify::new());
    running.relay = relay::Relay::new(
        running.bridge.clone(),
        Arc::new(CommittedTerminalDelay {
            inner: running.bridge.clone(),
            blocker: blocker.clone(),
            committed: committed.clone(),
        }),
        running.limits.clone(),
    )
    .unwrap();
    let (context, mut request) = admit(&running, &f, ID, "small-model", 20).await;
    request.stream = true;
    let relay::Session {
        head,
        mut body,
        completion,
        ..
    } = running.relay.start(request, context).unwrap();
    let drain = tokio::spawn(async move {
        assert_eq!(head.await.unwrap().unwrap().status, 200);
        let mut wire = Vec::new();
        while let Some(chunk) = body.next_chunk().await {
            wire.extend_from_slice(&chunk.unwrap());
        }
        (String::from_utf8(wire).unwrap(), completion.await.unwrap())
    });
    tokio::time::timeout(Duration::from_secs(10), committed.notified())
        .await
        .expect("real terminal checkpoint did not reach the commit barrier");
    let (wire, completion) = tokio::time::timeout(Duration::from_secs(10), drain)
        .await
        .expect("relay did not terminate after its hook deadlines")
        .unwrap();
    // Do not use Charging::resolve while the fault owns its advisory lock.
    let pending: (String, i64, i64) =
        sqlx::query_as("SELECT state,reserved,settled FROM core_charging.requests WHERE id=$1")
            .bind(ID)
            .fetch_one(&f.pool)
            .await
            .unwrap();
    let terminal: (i64, serde_json::Value) = sqlx::query_as(
        "SELECT sequence,provider_usage FROM core_model.output_checkpoints WHERE request_id=$1 AND terminal_observed ORDER BY sequence DESC LIMIT 1",
    )
    .bind(ID)
    .fetch_one(&f.pool)
    .await
    .unwrap();
    let final_events: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM core_charging.usage_events WHERE request_id=$1 AND event_id IN ('usage:provider-final','final')",
    )
    .bind(ID)
    .fetch_one(&f.pool)
    .await
    .unwrap();
    // Only release after completion: the real finalizer must also remain unable
    // to settle. Taking/closing the connection makes cleanup ownership explicit.
    let mut connection = blocker.lock().await.take().unwrap();
    let unlocked: bool =
        sqlx::query_scalar("SELECT pg_advisory_unlock(hashtextextended('core_charging:' || $1,0))")
            .bind(ID)
            .fetch_one(&mut connection)
            .await
            .unwrap();
    connection.close().await.unwrap();
    assert!(unlocked);
    assert_eq!(completion.error, Some(RelayError::SettlementPending));
    assert_eq!(completion.settlement, relay::Settlement::Pending);
    assert!(completion.report.provider_complete);
    assert!(wire.contains("synthetic answer"));
    assert!(wire.contains("settlement_pending"));
    assert!(!wire.contains("[DONE]"));
    assert_eq!(pending, ("streaming".into(), 140, 0));
    assert_eq!(final_events, 0);
    assert!(terminal.0 > 0);
    assert_eq!(terminal.1["input_tokens"], 10);
    assert_eq!(terminal.1["cached_input_tokens"], 2);
    assert_eq!(terminal.1["output_tokens"], 20);
    assert_balance(&f, 860, 140).await;
    let (original, contract) = running
        .bridge
        .charging
        .reservation_contract(ID)
        .await
        .unwrap();
    assert!(contract.legacy_precharge().is_none());
    assert!(original.price.upstream_cost_group_ppm.is_none());
    drop(running);

    let restarted = stream_gateway(&f, &provider.endpoint);
    // Only the disposable fixture owner advances lease expiry. Do not seed
    // usage, a final event, a recovery state, or any money movement.
    let mut tx = f.pool.begin().await.unwrap();
    sqlx::query("SET TRANSACTION ISOLATION LEVEL SERIALIZABLE")
        .execute(&mut *tx)
        .await
        .unwrap();
    let expired = sqlx::query(
        "UPDATE core_charging.requests SET lease_until=0,revision=revision+1 WHERE id=$1 AND state='streaming'",
    )
    .bind(ID)
    .execute(&mut *tx)
    .await
    .unwrap();
    assert_eq!(expired.rows_affected(), 1);
    tx.commit().await.unwrap();
    assert_eq!(restarted.recover_expired(10).await.unwrap(), 1);
    let held = receipt(&restarted, ID).await;
    assert_eq!(
        (held.state.as_str(), held.reserved, held.settled),
        ("reconcile", 140, 0)
    );
    assert_balance(&f, 860, 140).await;
    assert_eq!(restarted.reconcile_completed_streams(10).await.unwrap(), 1);
    assert_eq!(restarted.reconcile_completed_streams(10).await.unwrap(), 0);
    let settled = receipt(&restarted, ID).await;
    assert_eq!((settled.state.as_str(), settled.settled), ("settled", 49));
    assert_eq!(settled.id, original.id);
    assert_eq!(settled.payer_account_id, original.payer_account_id);
    assert_eq!(settled.payer_account_id, f.account);
    assert_eq!(settled.source, original.source);
    assert_eq!(settled.price, original.price);
    assert_eq!(settled.outcome.as_deref(), Some("completed"));
    assert_eq!(
        serde_json::to_value(settled.usage).unwrap(),
        json!({"input":10,"cached":2,"output":20})
    );

    let (context, mut request) = admit(&restarted, &f, ID, "small-model", 20).await;
    request.stream = true;
    let duplicate = restarted.relay.start(request, context).unwrap();
    tokio::time::timeout(Duration::from_secs(10), async {
        assert_eq!(
            duplicate.head.await.unwrap().unwrap_err(),
            RelayError::Conflict
        );
        assert!(
            !duplicate
                .completion
                .await
                .unwrap()
                .report
                .upstream_attempted
        );
    })
    .await
    .unwrap();
    drop(duplicate.body);
    assert_eq!(provider.bodies.lock().unwrap().len(), 1);
    assert_eq!(requests(&f.pool).await, 1);
    let finals: Vec<serde_json::Value> = sqlx::query_scalar(
        "SELECT detail FROM core_charging.usage_events WHERE request_id=$1 AND event_id='final'",
    )
    .bind(ID)
    .fetch_all(&f.pool)
    .await
    .unwrap();
    assert_eq!(finals.len(), 1);
    assert_eq!(
        finals[0]["usage"],
        json!({"input":10,"cached":2,"output":20})
    );
    assert_eq!(
        finals[0]["evidence"],
        format!("model-stream:{ID}:{}", terminal.0)
    );
    assert_balance(&f, 951, 0).await;
}

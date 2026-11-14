//! Native cache pricing through the existing real-HTTP/PG continuity fixture.
use super::*;

const FUNDS: i64 = 100_000;
const HOLD: i64 = 5_050;
// Compact omits max_output_tokens and retains the loopback installation bound of 100.
const COMPACT_HOLD: i64 = 5_300;
const COST: i64 = 39;

pub(super) fn price(mode: &str) -> PriceConfig {
    let multiplier = if mode == "rev4_repriced" { 2 } else { 1 };
    let revision = if mode == "rev4_revision1" {
        1
    } else if mode == "rev4_foreign" {
        2
    } else {
        4
    };
    let mut rates = json!({
        "input":{"numerator":6*multiplier,"denominator":1},
        "output":{"numerator":10*multiplier,"denominator":1},
        "cache_read":{"numerator":2*multiplier,"denominator":1},
        "cache_write":{"numerator":if revision==1 {0} else {8*multiplier},"denominator":1}
    });
    if revision == 4 {
        rates["cache_write_12h"] = json!({"numerator":12*multiplier,"denominator":1});
    }
    if mode == "rev4_fractional" {
        for (field, numerator) in [
            ("input", 7),
            ("output", 11),
            ("cache_read", 1),
            ("cache_write", 5),
            ("cache_write_12h", 13),
        ] {
            rates[field] = json!({"numerator":numerator,"denominator":3});
        }
    }
    let tiers = if mode == "rev4_tiers" {
        json!([{"up_to_input_tokens":8,"rates":rates},
            {"up_to_input_tokens":null,"rates":{
                "input":{"numerator":3,"denominator":1},"output":{"numerator":6,"denominator":1},
                "cache_read":{"numerator":1,"denominator":1},"cache_write":{"numerator":4,"denominator":1},
                "cache_write_12h":{"numerator":8,"denominator":1}}}])
    } else {
        json!([{"up_to_input_tokens":null,"rates":rates}])
    };
    serde_json::from_value(json!({"version":multiplier,"rules":[{
        "id":1,"model":"continuity-public","group":"allowed","speed":"standard",
        "billing_mode":"tiered_decimal",
        "rates":{"input":0,"output":0,"cache_read":0,"cache_write":0,"request":0},
        "group_multiplier_ppm":1000000,"speed_multiplier_ppm":1000000,
        "tiered":{"revision":revision,"source_digest":"a4".repeat(32),
            "uses_cache_read":true,"uses_cache_write":revision!=1,
            "uses_cache_write_1h":false,"uses_cache_write_12h":revision==4,
            "tiers":tiers}
    }]}))
    .unwrap()
}

fn usage() -> Value {
    json!({"input_tokens":7,"output_tokens":3,"total_tokens":10,
        "input_tokens_details":{"cached_tokens":2,"cache_write_tokens":3,
            "cache_write_tokens_details":{"ttl_30m_tokens":1,"ttl_12h_tokens":2}}})
}

fn input(streaming: bool, store: Option<bool>) -> Value {
    let mut value = json!({"model":"continuity-public","input":"priced context",
        "stream":streaming,"max_output_tokens":50,"temperature":0.375,"top_p":0.625,
        "parallel_tool_calls":false,"prompt_cache_options":{"mode":"implicit","ttl":"12h"}});
    if let Some(store) = store {
        value["store"] = store.into();
    }
    value
}

fn lookup(pool: &PgPool, a: &Supplier, b: &Supplier, mode: &str) -> ModelGateway {
    gateway(
        pool,
        &WorkerConfig {
            database: String::new(),
            suppliers: vec![a.base.clone(), b.base.clone()],
            mode: mode.into(),
            ready: String::new(),
        },
    )
}

async fn assert_charge(g: &ModelGateway, id: &str, held: i64, cost: i64) {
    assert_charge_input(g, id, held, cost, 7).await;
}
async fn assert_charge_input(g: &ModelGateway, id: &str, held: i64, cost: i64, input: i64) {
    let charge = g.bridge.charging.resolve(id).await.unwrap().unwrap();
    assert_eq!(
        (charge.state.as_str(), charge.reserved, charge.settled),
        ("settled", held, cost)
    );
    assert_eq!(
        (charge.usage.input, charge.usage.cached, charge.usage.output),
        (input, 2, 3)
    );
    let cache = charge.usage.cache_creation.unwrap();
    assert_eq!((cache.total, cache.twelve_hour), (3, Some(2)));
    assert_eq!(charge.price.tiered.as_ref().unwrap().tariff.revision, 4);
    let count:i64=sqlx::query_scalar("SELECT count(*) FROM core_charging.ledger_commands WHERE request_id=$1 AND command->'change'->>'operation'='settle'")
        .bind(id).fetch_one(&g.bridge.pool).await.unwrap();
    assert_eq!(
        count, 1,
        "one actual financial settlement per original request"
    );
}

async fn reply(pool: &PgPool, response: reqwest::Response, streaming: bool) -> (String, Value) {
    let status = response.status();
    let id = response.headers()["x-request-id"]
        .to_str()
        .unwrap()
        .to_owned();
    let bytes = response.bytes().await.unwrap();
    assert_eq!(
        status,
        StatusCode::OK,
        "{}",
        String::from_utf8_lossy(&bytes)
    );
    let value = if streaming {
        let text = std::str::from_utf8(&bytes).unwrap();
        let packets: Vec<_> = text.split_inclusive("\n\n").map(str::as_bytes).collect();
        let stored:Vec<(Vec<u8>,i64)>=sqlx::query_as("SELECT packet_sha256,packet_bytes FROM core_model.output_checkpoints WHERE request_id=$1 ORDER BY sequence")
            .bind(&id).fetch_all(pool).await.unwrap();
        assert_eq!(stored.len(), packets.len());
        for ((sha, len), packet) in stored.iter().zip(&packets) {
            assert_eq!(*sha, Sha256::digest(packet).to_vec());
            assert_eq!(*len, packet.len() as i64);
        }
        let event: Value = serde_json::from_str(
            packets
                .last()
                .unwrap()
                .split(|b| *b == b'\n')
                .find_map(|line| line.strip_prefix(b"data: "))
                .map(std::str::from_utf8)
                .unwrap()
                .unwrap(),
        )
        .unwrap();
        assert_eq!(event["type"], "response.completed");
        event["response"].clone()
    } else {
        packet(pool, &id, &bytes).await;
        serde_json::from_slice(&bytes).unwrap()
    };
    assert_eq!(
        value["usage"]["input_tokens_details"]["cache_write_tokens_details"]["ttl_12h_tokens"],
        2
    );
    (id, value)
}

#[sqlx::test(migrations = false)]
async fn native_rev4_json_sse_previous_compact_nonzero_cache_settle_once(pool: PgPool) {
    let f = native_http::test_support::fixture(pool, FUNDS, &["continuity-public"]).await;
    let rt = runtime(&f.pool).await;
    let (a, ta) = supplier().await;
    let (b, tb) = supplier().await;
    *a.cache_usage.lock().unwrap() = Some(usage());
    let core = Core::start(&rt.url, &a, &b, "rev4", None).await;
    let g = lookup(&rt.pool, &a, &b, "rev4");
    let mut root = None;
    for stream in [false, true] {
        for store in [None, Some(false), Some(true)] {
            let request_body = input(stream, store);
            let response = request(
                &core,
                &f.key,
                &format!("cache-{stream}-{store:?}"),
                false,
                request_body.clone(),
            )
            .await
            .unwrap();
            let (id, value) = reply(&f.pool, response, stream).await;
            assert_charge(&g, &id, HOLD, COST).await;
            let wire = a.seen.lock().unwrap().last().unwrap().clone();
            for field in [
                "input",
                "temperature",
                "top_p",
                "parallel_tool_calls",
                "prompt_cache_options",
                "max_output_tokens",
            ] {
                assert_eq!(
                    wire.get(field),
                    request_body.get(field),
                    "native wire {field}"
                );
            }
            assert_eq!(wire.get("store"), request_body.get("store"));
            let active:i64=sqlx::query_scalar("SELECT count(*) FROM core_model.responses_continuations WHERE request_id=$1 AND active")
            .bind(&id).fetch_one(&f.pool).await.unwrap();
            assert_eq!(active, i64::from(store.unwrap_or(true)));
            if root.is_none() {
                root = Some((id, value["id"].as_str().unwrap().to_owned()));
            }
        }
    }
    drop(core);
    // A later turn uses its current quote; the parent's exact provider binding
    // and its previously frozen price/usage remain untouched across restart.
    let core = Core::start(&rt.url, &a, &b, "rev4_repriced", None).await;
    let new = lookup(&rt.pool, &a, &b, "rev4_repriced");
    let (root_id, handle) = root.unwrap();
    let mut follow = input(false, Some(true));
    follow["previous_response_id"] = handle.clone().into();
    let response = request(&core, &f.key, "cache-current-price", false, follow)
        .await
        .unwrap();
    let (id, _) = reply(&f.pool, response, false).await;
    assert_charge(&new, &id, HOLD * 2, COST * 2).await;
    assert_charge(&new, &root_id, HOLD, COST).await;
    assert_eq!(
        a.seen.lock().unwrap().last().unwrap()["previous_response_id"],
        "resp_provider_1"
    );
    let response = request(
        &core,
        &f.key,
        "cache-compact",
        true,
        json!({"model":"continuity-public",
        "previous_response_id":handle,"input":"compact context"}),
    )
    .await
    .unwrap();
    let (compact_id, compact) = reply(&f.pool, response, false).await;
    assert_eq!(compact["object"], "response.compaction");
    let (_, compact_contract) = new
        .bridge
        .charging
        .reservation_contract(&compact_id)
        .await
        .unwrap();
    assert_eq!(compact_contract.bounds(), (800, 100));
    assert_charge(&new, &compact_id, COMPACT_HOLD * 2, COST * 2).await;
    let mut replay = input(true, Some(true));
    replay["input"] = compact["output"].clone();
    let response = request(&core, &f.key, "cache-opaque-roundtrip", false, replay)
        .await
        .unwrap();
    let (id, _) = reply(&f.pool, response, true).await;
    assert_charge(&new, &id, HOLD * 2, COST * 2).await;
    let balance = f.ledger.balance(f.account).await.unwrap();
    assert_eq!(
        (balance.available_units, balance.reserved_units),
        (FUNDS - 6 * COST - 3 * COST * 2, 0)
    );
    assert_eq!(a.calls.load(Ordering::SeqCst), 9);
    assert_eq!(b.calls.load(Ordering::SeqCst), 0);
    assert_eq!(
        a.counts.load(Ordering::SeqCst) + b.counts.load(Ordering::SeqCst),
        0
    );
    drop(core);
    ta.abort();
    tb.abort();
    rt.cleanup().await;
}

#[sqlx::test(migrations = false)]
async fn native_rev4_missing_and_contradictory_cache_receipts_hold_original_funds(pool: PgPool) {
    let f = native_http::test_support::fixture(pool, FUNDS, &["continuity-public"]).await;
    let rt = runtime(&f.pool).await;
    let (a, ta) = supplier().await;
    let (b, tb) = supplier().await;
    let core = Core::start(&rt.url, &a, &b, "rev4", None).await;
    let mut missing_total = usage();
    missing_total["input_tokens_details"]
        .as_object_mut()
        .unwrap()
        .remove("cache_write_tokens");
    let mut missing_lifetime = usage();
    missing_lifetime["input_tokens_details"]
        .as_object_mut()
        .unwrap()
        .remove("cache_write_tokens_details");
    let mut contradictory = usage();
    contradictory["input_tokens_details"]["cache_write_tokens_details"]["ttl_12h_tokens"] =
        4.into();
    let no_writes = json!({"input_tokens":7,"output_tokens":3,"total_tokens":10,"input_tokens_details":{"cached_tokens":2}});
    let mut count = 0;
    for (index, receipt) in [missing_total, missing_lifetime, contradictory, no_writes]
        .into_iter()
        .enumerate()
    {
        *a.cache_usage.lock().unwrap() = Some(receipt);
        for stream in [false, true] {
            let response = request(
                &core,
                &f.key,
                &format!("cache-unknown-{index}-{stream}"),
                false,
                input(stream, Some(true)),
            )
            .await
            .unwrap();
            let id = response.headers()["x-request-id"]
                .to_str()
                .unwrap()
                .to_owned();
            if !stream {
                assert!(!response.status().is_success());
            }
            if let Ok(bytes) = response.bytes().await {
                assert!(
                    !String::from_utf8_lossy(&bytes).contains("\"type\":\"response.completed\"")
                );
            }
            let state: (String, i64, i64) = sqlx::query_as(
                "SELECT state,reserved,settled FROM core_charging.requests WHERE id=$1",
            )
            .bind(&id)
            .fetch_one(&f.pool)
            .await
            .unwrap();
            assert_eq!(state, ("reconcile".into(), HOLD, 0));
            count += 1;
        }
    }
    drop(core);
    let core = Core::start(&rt.url, &a, &b, "rev4_repriced", None).await;
    let balance = f.ledger.balance(f.account).await.unwrap();
    assert_eq!(
        (balance.available_units, balance.reserved_units),
        (FUNDS - count * HOLD, count * HOLD)
    );
    let maps: i64 = sqlx::query_scalar("SELECT count(*) FROM core_model.responses_continuations")
        .fetch_one(&f.pool)
        .await
        .unwrap();
    assert_eq!(maps, 0);
    assert_eq!(a.calls.load(Ordering::SeqCst), count as usize);
    assert_eq!(a.counts.load(Ordering::SeqCst), 0);
    drop(core);
    ta.abort();
    tb.abort();
    rt.cleanup().await;
}

#[sqlx::test(migrations = false)]
async fn native_rev4_terminal_followed_by_bytes_keeps_hold(pool: PgPool) {
    let f = native_http::test_support::fixture(pool, FUNDS, &["continuity-public"]).await;
    let rt = runtime(&f.pool).await;
    let (a, ta) = supplier().await;
    let (b, tb) = supplier().await;
    *a.cache_usage.lock().unwrap() = Some(usage());
    a.trailing_stream_bytes.store(true, Ordering::SeqCst);
    let core = Core::start(&rt.url, &a, &b, "rev4", None).await;
    let response = request(
        &core,
        &f.key,
        "cache-invalid-tail",
        false,
        input(true, Some(true)),
    )
    .await
    .unwrap();
    let id = response.headers()["x-request-id"]
        .to_str()
        .unwrap()
        .to_owned();
    if let Ok(bytes) = response.bytes().await {
        assert!(!String::from_utf8_lossy(&bytes).contains("\"type\":\"response.completed\""));
    }
    let state: (String, i64, i64) =
        sqlx::query_as("SELECT state,reserved,settled FROM core_charging.requests WHERE id=$1")
            .bind(&id)
            .fetch_one(&f.pool)
            .await
            .unwrap();
    assert_eq!(state, ("reconcile".into(), HOLD, 0));
    let terminal:i64=sqlx::query_scalar("SELECT count(*) FROM core_model.output_checkpoints WHERE request_id=$1 AND terminal_observed")
        .bind(&id).fetch_one(&f.pool).await.unwrap();
    assert_eq!(terminal, 0);
    let balance = f.ledger.balance(f.account).await.unwrap();
    assert_eq!(
        (balance.available_units, balance.reserved_units),
        (FUNDS - HOLD, HOLD)
    );
    drop(core);
    ta.abort();
    tb.abort();
    rt.cleanup().await;
}

#[sqlx::test(migrations = false)]
async fn native_tiered_revision1_prices_reads_and_foreign_cache_dialect_never_dispatches(
    pool: PgPool,
) {
    let f = native_http::test_support::fixture(pool, FUNDS, &["continuity-public"]).await;
    let rt = runtime(&f.pool).await;
    let (a, ta) = supplier().await;
    let (b, tb) = supplier().await;
    *a.cache_usage.lock().unwrap() = Some(
        json!({"input_tokens":7,"output_tokens":3,"total_tokens":10,"input_tokens_details":{"cached_tokens":2}}),
    );
    let core = Core::start(&rt.url, &a, &b, "rev4_revision1", None).await;
    let mut body = input(false, Some(true));
    body.as_object_mut().unwrap().remove("prompt_cache_options");
    let response = request(&core, &f.key, "native-tiered-read", false, body.clone())
        .await
        .unwrap();
    let status = response.status();
    let id = response.headers()["x-request-id"]
        .to_str()
        .unwrap()
        .to_owned();
    let bytes = response.bytes().await.unwrap();
    assert_eq!(
        status,
        StatusCode::OK,
        "{}",
        String::from_utf8_lossy(&bytes)
    );
    packet(&f.pool, &id, &bytes).await;
    let state: (String, i64, i64) =
        sqlx::query_as("SELECT state,reserved,settled FROM core_charging.requests WHERE id=$1")
            .bind(&id)
            .fetch_one(&f.pool)
            .await
            .unwrap();
    assert_eq!(state, ("settled".into(), 2650, 32));
    drop(core);
    let core = Core::start(&rt.url, &a, &b, "rev4_foreign", None).await;
    let response = request(&core, &f.key, "foreign-native-tariff", false, body)
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::BAD_REQUEST);
    let _ = response.bytes().await;
    let requests: i64 = sqlx::query_scalar("SELECT count(*) FROM core_charging.requests")
        .fetch_one(&f.pool)
        .await
        .unwrap();
    assert_eq!(requests, 1);
    assert_eq!(a.calls.load(Ordering::SeqCst), 1);
    assert_eq!(a.counts.load(Ordering::SeqCst), 0);
    let balance = f.ledger.balance(f.account).await.unwrap();
    assert_eq!(
        (balance.available_units, balance.reserved_units),
        (FUNDS - 32, 0)
    );
    drop(core);
    ta.abort();
    tb.abort();
    rt.cleanup().await;
}

#[sqlx::test(migrations = false)]
async fn native_rev4_crash_and_ack_recovery_preserve_original_cache_price_once(pool: PgPool) {
    let f = native_http::test_support::fixture(pool, FUNDS, &["continuity-public"]).await;
    let rt = runtime(&f.pool).await;
    let (a, ta) = supplier().await;
    let (b, tb) = supplier().await;
    *a.cache_usage.lock().unwrap() = Some(usage());
    let mut settled = 0;
    for stage in [
        "after_terminal",
        "after_candidate",
        "after_settlement",
        "after_activation",
        "ack:candidate",
        "ack:settlement",
        "ack:activation",
    ] {
        let old: Vec<String> = sqlx::query_scalar("SELECT id FROM core_charging.requests")
            .fetch_all(&f.pool)
            .await
            .unwrap();
        let mut core = Core::start(&rt.url, &a, &b, "rev4", Some(stage)).await;
        let result = request(
            &core,
            &f.key,
            &format!("cache-crash-{stage}"),
            false,
            input(false, Some(true)),
        )
        .await;
        if stage.starts_with("ack:") {
            let response = result.unwrap();
            assert_eq!(response.status(), StatusCode::OK);
            let _ = response.bytes().await.unwrap();
        } else {
            assert!(result.is_err());
            assert_eq!(core.child.wait().unwrap().code(), Some(97));
        }
        drop(core);
        let id: String =
            sqlx::query_scalar("SELECT id FROM core_charging.requests WHERE NOT(id=ANY($1))")
                .bind(old)
                .fetch_one(&f.pool)
                .await
                .unwrap();
        if matches!(stage, "after_terminal" | "after_candidate") {
            tokio::time::sleep(Duration::from_secs(9)).await;
        }
        let calls = a.calls.load(Ordering::SeqCst);
        // Current prices are doubled, but recovery must use the original
        // durable terminal receipt and its original39-credit frozen price.
        let core = Core::start(&rt.url, &a, &b, "rev4_repriced", None).await;
        let g = lookup(&rt.pool, &a, &b, "rev4_repriced");
        if stage == "after_terminal" {
            let charge = g.bridge.charging.resolve(&id).await.unwrap().unwrap();
            assert_eq!(
                (charge.state.as_str(), charge.reserved, charge.settled),
                ("reconcile", HOLD, 0)
            );
            let terminals:i64=sqlx::query_scalar("SELECT count(*) FROM core_model.output_checkpoints WHERE request_id=$1 AND terminal_observed")
                .bind(&id).fetch_one(&f.pool).await.unwrap();
            assert_eq!(terminals, 0);
            let mappings: i64 = sqlx::query_scalar(
                "SELECT count(*) FROM core_model.responses_continuations WHERE request_id=$1",
            )
            .bind(&id)
            .fetch_one(&f.pool)
            .await
            .unwrap();
            assert_eq!(mappings, 0);
        } else {
            assert_charge(&g, &id, HOLD, COST).await;
            settled += 1;
            let active: bool = sqlx::query_scalar(
                "SELECT active FROM core_model.responses_continuations WHERE request_id=$1",
            )
            .bind(&id)
            .fetch_one(&f.pool)
            .await
            .unwrap();
            assert!(active);
        }
        for _ in 0..3 {
            assert_eq!(g.reconcile_completed_streams(1).await.unwrap(), 0);
        }
        assert_eq!(
            a.calls.load(Ordering::SeqCst),
            calls,
            "recovery cannot redispatch supplier"
        );
        drop(core);
    }
    let balance = f.ledger.balance(f.account).await.unwrap();
    assert_eq!(
        (balance.available_units, balance.reserved_units),
        (FUNDS - HOLD - settled * COST, HOLD)
    );
    assert_eq!(settled, 6);
    assert_eq!(a.calls.load(Ordering::SeqCst), 7);
    assert_eq!(
        a.counts.load(Ordering::SeqCst) + b.counts.load(Ordering::SeqCst),
        0
    );
    ta.abort();
    tb.abort();
    rt.cleanup().await;
}

#[sqlx::test(migrations = false)]
async fn native_rev4_tiers_and_fractional_prices_use_inclusive_input_and_one_rounding(
    pool: PgPool,
) {
    let f = native_http::test_support::fixture(pool, FUNDS, &["continuity-public"]).await;
    let rt = runtime(&f.pool).await;
    let (a, ta) = supplier().await;
    let (b, tb) = supplier().await;
    let mut spent = 0;
    // Independent exact arithmetic: tier1 <=8: cost39. Tier2 input9 has
    // uncached4*1.5 + read2*.5 + short1*2 + long2*4 + output3*3 =26.
    // Fractions: (2*7+2*1+1*5+2*13+3*11)/6=40/3 ->13 (one final
    // half-up rounding). Max hold (800*13+50*11)/6=1825, no per-term ceil.
    for (mode, tokens, held, cost) in [
        ("rev4_tiers", 7, 3350, 39),
        ("rev4_tiers", 9, 3350, 26),
        ("rev4_fractional", 7, 1825, 13),
    ] {
        let mut receipt = usage();
        receipt["input_tokens"] = tokens.into();
        receipt["total_tokens"] = (tokens + 3).into();
        *a.cache_usage.lock().unwrap() = Some(receipt);
        let core = Core::start(&rt.url, &a, &b, mode, None).await;
        let response = request(
            &core,
            &f.key,
            &format!("exact-{mode}-{tokens}"),
            false,
            input(false, Some(true)),
        )
        .await
        .unwrap();
        let (id, _) = reply(&f.pool, response, false).await;
        let g = lookup(&rt.pool, &a, &b, mode);
        assert_charge_input(&g, &id, held, cost, tokens).await;
        spent += cost;
        drop(core);
    }
    let balance = f.ledger.balance(f.account).await.unwrap();
    assert_eq!(
        (balance.available_units, balance.reserved_units),
        (FUNDS - spent, 0)
    );
    assert_eq!(a.calls.load(Ordering::SeqCst), 3);
    assert_eq!(a.counts.load(Ordering::SeqCst), 0);
    ta.abort();
    tb.abort();
    rt.cleanup().await;
}

//! Persisted legacy evidence fixture: no current packet writer is used to seed rows.
use super::*;
#[sqlx::test(migrations = false)]
async fn audio_pg_legacy_v3_journal_read_only_recovery(pool: PgPool) {
    legacy_fixture(pool, false).await;
}
#[sqlx::test(migrations = false)]
async fn audio_pg_legacy_v3_journal_corrupt_usage_keeps_hold(pool: PgPool) {
    legacy_fixture(pool, true).await;
}
async fn legacy_fixture(pool: PgPool, corrupt: bool) {
    // Directly exercise the checkpoint/finalize gap. This is object recreation,
    // not an OS-crash acceptance runner, and does not call an external supplier.
    let f = native_http::test_support::fixture(pool, 10000, &["audio-public"]).await;
    let runtime = RuntimeRole::create(&f.pool).await;
    runtime.optional("core_charging").await;
    runtime.optional("core_model").await;
    let supplier = Supplier::start(false).await;
    let old = gateway(&runtime.pool, &supplier);
    let mut audio = rule(&supplier.endpoint).price;
    audio.usage_format = AudioUsageFormat::OpenAiChatV1;
    let price = charging::Price {
        audio: Some(audio.clone()),
        text_request: None,
        tool: None,
        grounding: None,
        user_trust: None,
        image: None,
        upstream_cost_group_ppm: None,
        tiered: None,
        version: audio.price_revision.clone(),
        model: audio.model.clone(),
        group: audio.group.clone(),
        input_per_million: 0,
        cached_per_million: 0,
        output_per_million: 0,
    };
    let worker = [9; 32];
    let fingerprint = [8; 32];
    let wire_hash = "07".repeat(32);
    let prepared = charging::contract::PreparedReservation::audio_chat(
        charging::contract::hex_digest(&fingerprint),
        charging::contract::ReservationBinding {
            route_id: "r1-t1-u1".into(),
            target_id: 1,
            upstream_id: 1,
            protocol: "audio_chat".into(),
            model_snapshot: "synthetic-audio-upstream".into(),
            generation_url: format!("{}/chat/completions", supplier.endpoint),
            credential_ref: CredentialRef { id: 7, revision: 1 },
            payload_policy_sha256: "06".repeat(32),
            provider_contract_revision: audio.provider_contract_revision.clone(),
            projection_revision: "audio-chat-stream-raw-v1".into(),
            quote_version: 1,
        },
        wire_hash.clone(),
        audio.bounds,
    )
    .unwrap();
    let spec = charging::Request {
        id: "synthetic-audio-recovery".into(),
        fingerprint,
        reserve: price
            .reservation_ceiling(charging::ReservationBasis::Audio)
            .unwrap(),
        price,
        lease_seconds: 30,
        maximum_seconds: 30,
    };
    old.bridge
        .charging
        .reserve_prepared_model_with_context(
            &f.key,
            &spec,
            &worker,
            None,
            None,
            charging::ReservationBasis::Audio,
            &prepared,
        )
        .await
        .unwrap();
    assert!(old.bridge.charging.start(&spec.id, &worker).await.unwrap());
    old.bridge
        .charging
        .record_route_dispatch(&spec.id, &worker, 1, "r1-t1-u1", "allowed", &wire_hash)
        .await
        .unwrap();

    let fixture: Value =
        serde_json::from_str(include_str!("fixtures/chat-journal-v3.json")).unwrap();
    let raw = fixture["raw_usage"].as_str().unwrap();
    let mut bytes = Vec::new();
    for packet in fixture["frames"].as_array().unwrap() {
        bytes.extend_from_slice(packet.as_str().unwrap().as_bytes());
    }
    let stream_sha: [u8; 32] = Sha256::digest(&bytes).into();
    let stored = old
        .bridge
        .charging
        .resolve(&spec.id)
        .await
        .unwrap()
        .unwrap();
    let price_sha = charging::contract::typed_digest(&stored.price).unwrap();
    let mut chain = "00".repeat(32);
    let mut tx = f.pool.begin().await.unwrap();
    sqlx::query("SET TRANSACTION ISOLATION LEVEL SERIALIZABLE")
        .execute(&mut *tx)
        .await
        .unwrap();
    for (i, row) in fixture["rows"].as_array().unwrap().iter().enumerate() {
        let mut row = row.clone();
        assert_eq!(row["journal_version"], 3);
        assert!(row.get("raw_usage").is_none());
        if corrupt && i == 1 {
            // Keep the actual frame hash: a different raw usage packet cannot borrow its authority.
            row["usage_packet"] = json!(
                row["usage_packet"]
                    .as_str()
                    .unwrap()
                    .replace("\"text_tokens\":1", "\"text_tokens\":2")
                    .replace("\"prompt_tokens\":4", "\"prompt_tokens\":5")
                    .replace("\"total_tokens\":10", "\"total_tokens\":11")
            );
        }
        row["price_sha256"] = json!(price_sha);
        row["previous_chain"] = json!(chain);
        chain = charging::contract::typed_digest(&row).unwrap();
        row["chain_sha256"] = json!(chain);
        sqlx::query(
            "INSERT INTO core_charging.usage_events(request_id,event_id,detail) VALUES($1,$2,$3)",
        )
        .bind(&spec.id)
        .bind(format!("audio-packet:v1:{:04}", i + 1))
        .bind(row)
        .execute(&mut *tx)
        .await
        .unwrap();
    }
    tx.commit().await.unwrap();
    let before:Vec<Value>=sqlx::query_scalar("SELECT detail FROM core_charging.usage_events WHERE request_id=$1 AND event_id LIKE 'audio-packet:v1:%' ORDER BY event_id")
        .bind(&spec.id).fetch_all(&f.pool).await.unwrap();
    let checkpoint = old
        .bridge
        .charging
        .checkpoint_audio(&spec.id, &worker, raw, stream_sha)
        .await;
    if corrupt {
        assert!(checkpoint.is_err());
        old.bridge
            .charging
            .mark_unknown(&spec.id, &worker)
            .await
            .unwrap();
        drop(old);
        let new = gateway(&runtime.pool, &supplier);
        new.recover_expired(10).await.unwrap();
        let charge = new
            .bridge
            .charging
            .resolve(&spec.id)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(charge.settled, 0);
        assert_eq!(charge.reserved, spec.reserve);
        let receipts:i64=sqlx::query_scalar("SELECT count(*) FROM core_charging.usage_events WHERE request_id=$1 AND event_id='audio-terminal:v1'")
            .bind(&spec.id).fetch_one(&f.pool).await.unwrap();
        assert_eq!(receipts, 0);
        let after:Vec<Value>=sqlx::query_scalar("SELECT detail FROM core_charging.usage_events WHERE request_id=$1 AND event_id LIKE 'audio-packet:v1:%' ORDER BY event_id")
            .bind(&spec.id).fetch_all(&f.pool).await.unwrap();
        assert_eq!(before, after);
        assert_eq!(supplier.state.hits.load(Ordering::SeqCst), 0);
        drop(new);
        runtime.cleanup().await;
        return;
    }
    checkpoint.unwrap();
    old.bridge
        .charging
        .mark_unknown(&spec.id, &worker)
        .await
        .unwrap();
    drop(old);
    let mut new_rule = rule(&supplier.endpoint);
    new_rule.price.price_revision = "audio-v2".into();
    new_rule.price.tariff = AudioTariff::Fixed {
        usd_per_request: AudioRatio {
            numerator: 1,
            denominator: 100,
        },
    };
    let new = gateway(&runtime.pool, &supplier)
        .with_audio_rules(vec![new_rule])
        .unwrap();
    new.recover_expired(10).await.unwrap();
    new.recover_expired(10).await.unwrap();
    let charge = new
        .bridge
        .charging
        .resolve(&spec.id)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(
        (
            charge.state.as_str(),
            charge.settled,
            charge.price.version.as_str()
        ),
        ("settled", 62, "audio-v1")
    );
    let balance = f.ledger.balance(f.account).await.unwrap();
    assert_eq!((balance.available_units, balance.reserved_units), (9938, 0));
    assert_eq!(supplier.state.hits.load(Ordering::SeqCst), 0);

    let after:Vec<Value>=sqlx::query_scalar("SELECT detail FROM core_charging.usage_events WHERE request_id=$1 AND event_id LIKE 'audio-packet:v1:%' ORDER BY event_id")
        .bind(&spec.id).fetch_all(&f.pool).await.unwrap();
    assert_eq!(before, after, "legacy rows must never be rewritten to v4");
    drop(new);
    runtime.cleanup().await;
}

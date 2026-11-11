//! Real ledger tests for the frozen legacy wallet transaction. Terminal facts
//! are owner-seeded here, while gateway tests cover their supplier provenance.
use super::*;
use crate::billing::{
    Credits,
    charging::contract::{
        self, LegacyOrdinaryEstimate, LegacyPrechargeAuthorization, PreparedReservation,
        ReservationBinding, ReservationBounds, ReservationSemantics,
    },
    ledger::{Action, Ledger as WalletLedger, Outcome as WalletOutcome, Request as WalletRequest},
    transactional_ledger::TransactionalLedger,
};

fn prepared_legacy(id: &str, max: i64) -> (Request, ReservationBasis, PreparedReservation) {
    let mut spec = request(id, 120);
    spec.price.rounding = Some(crate::routing::TokenRounding::LegacyGoRatioV1);
    let estimate = LegacyOrdinaryEstimate {
        options: super::super::legacy_precharge::SourceOptions {
            preconsumed_tokens: 5,
            count_token: true,
            get_media_token: true,
            get_media_token_not_stream: false,
        },
        prompt_tokens: 5,
        explicit_max_tokens: None,
        input_rate: 2_000_000,
        group_ppm: 1_000_000,
        model_ratio_bits: 1.0_f64.to_bits(),
        group_ratio_bits: 1.0_f64.to_bits(),
        source_free_model: false,
        trust: None,
        source_tariff: None,
    };
    let proof = PreparedReservation::context_max(
        contract::hex_digest(&spec.fingerprint),
        ReservationBinding {
            route_id: "legacy-route".into(),
            target_id: 1,
            upstream_id: 1,
            protocol: "responses".into(),
            model_snapshot: spec.price.model.clone(),
            generation_url: "https://example.test/v1/responses".into(),
            credential_ref: crate::routing::CredentialRef { id: 1, revision: 1 },
            payload_policy_sha256: "02".repeat(32),
            provider_contract_revision: "responses-v1".into(),
            projection_revision: "responses-v1".into(),
            quote_version: 1,
        },
        "03".repeat(32),
        ReservationBounds {
            model_input_max: 100,
            input: 100,
            requested_output: 20,
            output: 20,
        },
        ReservationSemantics {
            input: "inclusive".into(),
            output: "includes_reasoning".into(),
            missing_cached_tokens_is_zero: true,
            missing_cache_creation_tokens_is_zero: false,
        },
    )
    .unwrap()
    .with_legacy_precharge(LegacyPrechargeAuthorization::Cc52OrdinaryWalletV1 {
        source_contract_sha256: "04".repeat(32),
        estimate_sha256: contract::typed_digest(&estimate).unwrap(),
        estimated_hold: 5,
        operator_max_billable: max,
        estimate,
    })
    .unwrap();
    (
        spec,
        ReservationBasis::Tokens {
            input: 100,
            output: 20,
        },
        proof,
    )
}

async fn credit_wallet(wallet: &WalletLedger, key: &str, amount: i64) {
    let posted = wallet
        .execute(&WalletRequest {
            scope: "legacy-wallet-test".into(),
            operation_key: key.into(),
            actor_user_id: Some(2),
            reason: "Isolated legacy wallet test".into(),
            action: Action::Credit {
                account_id: 102,
                amount_units: Credits::try_from(amount).unwrap(),
                payment_reference: format!("synthetic:legacy:{key}"),
            },
        })
        .await
        .unwrap();
    assert!(matches!(posted, WalletOutcome::Posted { .. }));
}

async fn terminal(pool: &PgPool, spec: &Request, usage: Usage) {
    if spec.price.tiered.is_some() {
        let observed = crate::relay::Usage {
            input_tokens: Some(usage.input.try_into().unwrap()),
            cached_input_tokens: Some(usage.cached.try_into().unwrap()),
            output_tokens: Some(usage.output.try_into().unwrap()),
            ..Default::default()
        };
        owner_seed_query(pool, sqlx::query("INSERT INTO core_charging.usage_events(request_id,event_id,detail) VALUES($1,'provider-observation:terminal',$2)")
            .bind(&spec.id).bind(serde_json::json!({"kind":"provider_observation","provider_complete":true,"route_id":"legacy-route","raw_usage":observed}))).await.unwrap();
    }
    owner_seed_query(pool, sqlx::query(
        "INSERT INTO core_charging.usage_events(request_id,event_id,detail) VALUES ($1,'model-provider-terminal:v1',$2)"
    ).bind(&spec.id).bind(serde_json::json!({
        "kind": "model_provider_terminal", "normalized_usage": usage,
        "price_sha256": contract::typed_digest(&spec.price).unwrap(),
    }))).await.unwrap();
}

#[sqlx::test(migrations = false)]
async fn legacy_wallet_debt_replay_refund_and_topup(pool: PgPool) {
    fixture(&pool).await;
    let wallet = WalletLedger::from_pool(pool.clone());
    wallet.open_wallet(102).await.unwrap();
    credit_wallet(&wallet, "initial", 8).await;
    let engine = Charging::new(pool.clone(), Arc::new(TransactionalLedger));
    let (spec, basis, proof) = prepared_legacy("legacy-debt", 20);
    let reserve = engine
        .reserve_prepared_model_with_context(PERSONAL, &spec, &WORKER, None, None, basis, &proof)
        .await
        .unwrap();
    assert_eq!(reserve.reserved, 5); // context ceiling is 120, wallet has only 8
    assert_eq!(reserve.source, Source::Wallet);
    engine.start(&spec.id, &WORKER).await.unwrap();
    let usage = Usage {
        input: 10,
        output: 5,
        ..Usage::default()
    };
    assert_eq!(
        engine
            .settle_legacy_wallet_terminal(&spec.id, &WORKER, usage, Outcome::Completed)
            .await,
        Err(Error::Conflict)
    );
    terminal(&pool, &spec, usage).await;
    // Terminal authority spends the original payer despite key revocation.
    owner_seed_query(
        &pool,
        sqlx::query(
            "UPDATE core_identity.credentials SET revoked_at=clock_timestamp() WHERE id=11",
        ),
    )
    .await
    .unwrap();
    let settled = engine
        .settle_legacy_wallet_terminal(&spec.id, &WORKER, usage, Outcome::Completed)
        .await
        .unwrap();
    assert_eq!(settled.settled, 15);
    let balance = wallet.balance(102).await.unwrap();
    assert_eq!((balance.spendable_units, balance.debt_units), (0, -7));
    assert_eq!(
        engine
            .settle_legacy_wallet_terminal(&spec.id, &WORKER, usage, Outcome::Completed)
            .await
            .unwrap(),
        settled
    );
    assert_eq!(wallet.balance(102).await.unwrap(), balance);
    engine
        .refund(&spec.id, "first", 4, "synthetic compensation")
        .await
        .unwrap();
    let balance = wallet.balance(102).await.unwrap();
    assert_eq!((balance.spendable_units, balance.debt_units), (0, -3));
    credit_wallet(&wallet, "repayment", 5).await;
    let balance = wallet.balance(102).await.unwrap();
    assert_eq!((balance.spendable_units, balance.debt_units), (2, 0));
    engine
        .refund(&spec.id, "rest", 11, "synthetic remaining compensation")
        .await
        .unwrap();
    let balance = wallet.balance(102).await.unwrap();
    assert_eq!((balance.spendable_units, balance.debt_units), (13, 0));
}

#[sqlx::test(migrations = false)]
async fn legacy_wallet_operator_cap_and_budget_failure_keep_hold(pool: PgPool) {
    let (engine, recorder) = fixture(&pool).await;
    let (spec, basis, proof) = prepared_legacy("legacy-cap", 10);
    engine
        .reserve_prepared_model_with_context(PERSONAL, &spec, &WORKER, None, None, basis, &proof)
        .await
        .unwrap();
    engine.start(&spec.id, &WORKER).await.unwrap();
    let usage = Usage {
        input: 10,
        output: 5,
        ..Usage::default()
    };
    terminal(&pool, &spec, usage).await;
    assert_eq!(
        engine
            .settle_legacy_wallet_terminal(&spec.id, &WORKER, usage, Outcome::Completed)
            .await,
        Err(Error::Invalid)
    );
    assert_eq!(engine.resolve(&spec.id).await.unwrap().unwrap().reserved, 5);

    let (spec, basis, proof) = prepared_legacy("legacy-budget", 20);
    engine
        .reserve_prepared_model_with_context(PERSONAL, &spec, &WORKER, None, None, basis, &proof)
        .await
        .unwrap();
    engine.start(&spec.id, &WORKER).await.unwrap();
    terminal(&pool, &spec, usage).await;
    engine
        .set_budget(
            MEMBER_SESSION,
            &Budget {
                scope: "self".into(),
                account_id: None,
                user_id: Some(2),
                key_id: None,
                period: "day".into(),
                anchor: 0,
                seconds: 0,
                limit: 12,
            },
        )
        .await
        .unwrap();
    assert_eq!(
        engine
            .settle_legacy_wallet_terminal(&spec.id, &WORKER, usage, Outcome::Completed)
            .await,
        Err(Error::BudgetExceeded)
    );
    assert_eq!(engine.resolve(&spec.id).await.unwrap().unwrap().reserved, 5);
    let supplementary: i64 = sqlx::query_scalar("SELECT count(*) FROM core_charging.usage_events WHERE request_id=$1 AND event_id='legacy-wallet-supplement:v1'").bind(&spec.id).fetch_one(&pool).await.unwrap();
    assert_eq!(supplementary, 0);
    // Fail after recording reserve to exercise rollback of budget/hold/receipt.
    engine
        .set_budget(
            MEMBER_SESSION,
            &Budget {
                scope: "self".into(),
                account_id: None,
                user_id: Some(2),
                key_id: None,
                period: "day".into(),
                anchor: 0,
                seconds: 0,
                limit: 100,
            },
        )
        .await
        .unwrap();
    recorder.mode.store(3, Ordering::SeqCst);
    assert!(
        engine
            .settle_legacy_wallet_terminal(&spec.id, &WORKER, usage, Outcome::Completed)
            .await
            .is_err()
    );
    assert_eq!(engine.resolve(&spec.id).await.unwrap().unwrap().reserved, 5);
}

#[sqlx::test(migrations = false)]
async fn legacy_wallet_action_cannot_borrow_from_scope_alone(pool: PgPool) {
    fixture(&pool).await;
    let wallet = WalletLedger::from_pool(pool.clone());
    wallet.open_wallet(102).await.unwrap();
    let mut tx = owner_seed(&pool).await;
    let request = WalletRequest {
        scope: "core-charging".into(),
        operation_key: "unbound".into(),
        actor_user_id: Some(2),
        reason: "Probe missing frozen terminal authority".into(),
        action: Action::LegacyWalletReserve {
            request_id: "missing".into(),
            account_id: 102,
            amount_units: Credits::try_from(5_i64).unwrap(),
        },
    };
    let sqlx::types::Json(result): sqlx::types::Json<WalletOutcome> =
        sqlx::query_scalar("SELECT core_billing.post_ledger($1)")
            .bind(sqlx::types::Json(request))
            .fetch_one(&mut *tx)
            .await
            .unwrap();
    tx.commit().await.unwrap();
    assert!(matches!(result, WalletOutcome::Rejected { .. }));
    assert_eq!(wallet.balance(102).await.unwrap().debt_units, 0);
}

#[sqlx::test(migrations = false)]
async fn legacy_intent_preserves_subscription_order_and_ceiling(pool: PgPool) {
    let (engine, _) = fixture(&pool).await;
    let sub = subscription(&pool, 102, 120).await;
    owner_seed_sql(&pool, "UPDATE core_billing.account_policies SET preference='subscription_first' WHERE account_id=102").await.unwrap();
    let (spec, basis, proof) = prepared_legacy("legacy-subscription-first", 20);
    let charge = engine
        .reserve_prepared_model_with_context(PERSONAL, &spec, &WORKER, None, None, basis, &proof)
        .await
        .unwrap();
    assert!(matches!(charge.source, Source::Subscription { id, .. } if id == sub));
    assert_eq!(charge.reserved, 120);
    let (spec, basis, proof) = prepared_legacy("legacy-subscription-overflow", 20);
    let charge = engine
        .reserve_prepared_model_with_context(PERSONAL, &spec, &WORKER, None, None, basis, &proof)
        .await
        .unwrap();
    assert_eq!(charge.source, Source::Wallet);
    assert_eq!(charge.reserved, 5);
    owner_seed_sql(&pool, "UPDATE core_billing.account_policies SET preference='subscription_only' WHERE account_id=102").await.unwrap();
    let (spec, basis, proof) = prepared_legacy("legacy-subscription-only", 20);
    assert_eq!(
        engine
            .reserve_prepared_model_with_context(
                PERSONAL, &spec, &WORKER, None, None, basis, &proof
            )
            .await
            .unwrap_err(),
        Error::NoFunds
    );
}

#[sqlx::test(migrations = false)]
async fn legacy_terminal_reconcile_uses_original_authorization(pool: PgPool) {
    let (engine, _) = fixture(&pool).await;
    let (spec, basis, proof) = prepared_legacy("legacy-reconcile", 20);
    engine
        .reserve_prepared_model_with_context(PERSONAL, &spec, &WORKER, None, None, basis, &proof)
        .await
        .unwrap();
    engine.start(&spec.id, &WORKER).await.unwrap();
    let usage = Usage {
        input: 10,
        output: 5,
        ..Usage::default()
    };
    terminal(&pool, &spec, usage).await;
    engine.mark_unknown(&spec.id, &WORKER).await.unwrap();
    let original = engine.resolve(&spec.id).await.unwrap().unwrap();
    let settled = engine
        .reconcile(
            &spec.id,
            original.revision,
            usage,
            Outcome::Completed,
            "model-provider-terminal:v1",
        )
        .await
        .unwrap();
    assert_eq!(settled.settled, 15);
    assert_eq!(settled.payer_account_id, original.payer_account_id);
    assert_eq!(
        engine
            .reconcile(
                &spec.id,
                original.revision,
                usage,
                Outcome::Completed,
                "model-provider-terminal:v1"
            )
            .await
            .unwrap(),
        settled
    );
}

fn prepared_expression(id: &str) -> (Request, ReservationBasis, PreparedReservation) {
    let (mut spec, basis, proof) = prepared_legacy(id, 125);
    let expression = r#"tier("standard",p*2+c*4)"#;
    spec.price.rounding = None;
    spec.price.input_per_million = 0;
    spec.price.cached_per_million = 0;
    spec.price.output_per_million = 0;
    spec.price.tiered = Some(
        serde_json::from_value(serde_json::json!({
            "tariff": {
                "revision": 1, "source_digest": contract::digest(expression.as_bytes()),
                "uses_cache_read": false, "uses_cache_write": false,
                "tiers": [{"up_to_input_tokens": null, "rates": {
                    "input": {"numerator": 2, "denominator": 1},
                    "output": {"numerator": 4, "denominator": 1},
                    "cache_read": {"numerator": 0, "denominator": 1},
                    "cache_write": {"numerator": 0, "denominator": 1}
                }}]
            },
            "input_semantics": "inclusive", "group_multiplier_ppm": 1000000,
            "speed_multiplier_ppm": 1000000, "credits_per_usd": 500000
        }))
        .unwrap(),
    );
    spec.reserve = spec.price.reservation_ceiling(basis).unwrap();
    let estimate = contract::LegacyTieredEstimate {
        source_expression: expression.into(),
        options: super::super::legacy_precharge::SourceOptions {
            preconsumed_tokens: 5000,
            count_token: true,
            get_media_token: true,
            get_media_token_not_stream: false,
        },
        prompt_tokens: 5,
        explicit_max_tokens: Some(20),
        group_ppm: 1_000_000,
        group_ratio_bits: 1.0_f64.to_bits(),
        source_free_model: false,
        trust: None,
    };
    assert_eq!(estimate.hold(), Ok(45));
    estimate.verify_price(&spec.price).unwrap();
    let proof = proof
        .with_legacy_precharge(LegacyPrechargeAuthorization::Cc52TieredWalletV1 {
            source_contract_sha256: "05".repeat(32),
            estimate_sha256: contract::typed_digest(&estimate).unwrap(),
            estimated_hold: 45,
            operator_max_billable: 125,
            estimate,
        })
        .unwrap();
    (spec, basis, proof)
}

#[sqlx::test(migrations = false)]
async fn legacy_expression_wallet_borrows_only_with_original_hash_and_terminal(pool: PgPool) {
    fixture(&pool).await;
    let wallet = WalletLedger::from_pool(pool.clone());
    wallet.open_wallet(102).await.unwrap();
    credit_wallet(&wallet, "expression-initial", 50).await;
    let engine = Charging::new(pool.clone(), Arc::new(TransactionalLedger));
    let (spec, basis, proof) = prepared_expression("legacy-expression-debt");
    let mut changed = spec.clone();
    changed.price.tiered.as_mut().unwrap().tariff.source_digest = "fe".repeat(32);
    assert_eq!(
        engine
            .reserve_prepared_model_with_context(
                PERSONAL, &changed, &WORKER, None, None, basis, &proof
            )
            .await
            .unwrap_err(),
        Error::Conflict
    );
    let charge = engine
        .reserve_prepared_model_with_context(PERSONAL, &spec, &WORKER, None, None, basis, &proof)
        .await
        .unwrap();
    assert_eq!(charge.reserved, 45);
    assert_eq!(spec.reserve, 140);
    engine.start(&spec.id, &WORKER).await.unwrap();
    let usage = Usage {
        input: 80,
        output: 20,
        ..Usage::default()
    };
    terminal(&pool, &spec, usage).await;
    let settled = engine
        .settle_legacy_wallet_terminal(&spec.id, &WORKER, usage, Outcome::Completed)
        .await
        .unwrap();
    assert_eq!(settled.settled, 120);
    let balance = wallet.balance(102).await.unwrap();
    assert_eq!((balance.spendable_units, balance.debt_units), (0, -70));
    assert_eq!(
        engine
            .settle_legacy_wallet_terminal(&spec.id, &WORKER, usage, Outcome::Completed)
            .await
            .unwrap(),
        settled
    );
    assert_eq!(wallet.balance(102).await.unwrap(), balance);
    engine
        .refund(
            &spec.id,
            "expression-refund",
            120,
            "synthetic expression refund",
        )
        .await
        .unwrap();
    let balance = wallet.balance(102).await.unwrap();
    assert_eq!((balance.spendable_units, balance.debt_units), (50, 0));
}

#[sqlx::test(migrations = false)]
async fn legacy_expression_terminal_budget_failure_is_atomic(pool: PgPool) {
    let (engine, _) = fixture(&pool).await;
    let (spec, basis, proof) = prepared_expression("legacy-expression-budget");
    engine
        .set_budget(
            MEMBER_SESSION,
            &Budget {
                scope: "self".into(),
                account_id: None,
                user_id: Some(2),
                key_id: None,
                period: "day".into(),
                anchor: 0,
                seconds: 0,
                limit: 80,
            },
        )
        .await
        .unwrap();
    engine
        .reserve_prepared_model_with_context(PERSONAL, &spec, &WORKER, None, None, basis, &proof)
        .await
        .unwrap();
    engine.start(&spec.id, &WORKER).await.unwrap();
    let usage = Usage {
        input: 80,
        output: 20,
        ..Usage::default()
    };
    terminal(&pool, &spec, usage).await;
    assert_eq!(
        engine
            .settle_legacy_wallet_terminal(&spec.id, &WORKER, usage, Outcome::Completed)
            .await
            .unwrap_err(),
        Error::BudgetExceeded
    );
    let retained = engine.resolve(&spec.id).await.unwrap().unwrap();
    assert_eq!(
        (retained.state.as_str(), retained.reserved),
        ("streaming", 45)
    );
    let receipts: i64 = sqlx::query_scalar("SELECT count(*) FROM core_charging.usage_events WHERE request_id=$1 AND event_id='legacy-wallet-supplement:v1'").bind(&spec.id).fetch_one(&pool).await.unwrap();
    assert_eq!(receipts, 0);
}

#[sqlx::test(migrations = false)]
async fn legacy_observation_recovery_requires_revision_route_and_full_audit(pool: PgPool) {
    let (engine, _) = fixture(&pool).await;
    let (spec, basis, proof) = prepared_legacy("legacy-observation-recovery", 20);
    engine
        .reserve_prepared_model_with_context(PERSONAL, &spec, &WORKER, None, None, basis, &proof)
        .await
        .unwrap();
    engine.start(&spec.id, &WORKER).await.unwrap();
    engine
        .record_route_dispatch(
            &spec.id,
            &WORKER,
            1,
            "legacy-route",
            "default",
            &"03".repeat(32),
        )
        .await
        .unwrap();
    engine.mark_unknown(&spec.id, &WORKER).await.unwrap();
    let frozen = engine.resolve(&spec.id).await.unwrap().unwrap();
    let usage = Usage {
        input: 10,
        output: 5,
        ..Usage::default()
    };
    assert_eq!(
        engine
            .reconcile_legacy_wallet_observation(
                &spec.id,
                frozen.revision,
                "legacy-route",
                usage,
                Outcome::Completed,
                "synthetic-full-audit"
            )
            .await
            .unwrap_err(),
        Error::Conflict
    );
    engine
        .record_provider_observation(
            &spec.id,
            &WORKER,
            "legacy-route",
            &serde_json::to_value(&crate::relay::Usage {
                input_tokens: Some(10),
                output_tokens: Some(5),
                ..Default::default()
            })
            .unwrap(),
            100,
            20,
        )
        .await
        .unwrap();
    assert_eq!(
        engine
            .reconcile_legacy_wallet_observation(
                &spec.id,
                frozen.revision,
                "foreign-route",
                usage,
                Outcome::Completed,
                "synthetic-full-audit"
            )
            .await
            .unwrap_err(),
        Error::Conflict
    );
    // Complete audit alone has no recovery authority without its committed terminal checkpoint.
    assert_eq!(
        engine
            .reconcile_legacy_wallet_observation(
                &spec.id,
                frozen.revision,
                "legacy-route",
                usage,
                Outcome::Completed,
                "synthetic-full-audit"
            )
            .await
            .unwrap_err(),
        Error::Conflict
    );
    owner_seed_query(&pool, sqlx::query(
        "INSERT INTO core_model.output_checkpoints(request_id,sequence,packet_sha256,packet_bytes,prepared_bytes,provider_usage,terminal_observed) VALUES ($1,1,$2,1,1,$3,true)"
    ).bind(&spec.id).bind(vec![1_u8;32]).bind(serde_json::to_value(&crate::relay::Usage { input_tokens: Some(10), output_tokens: Some(5), ..Default::default() }).unwrap())).await.unwrap();
    // Isolate revision fencing after the required immutable evidence exists.
    assert_eq!(
        engine
            .reconcile_legacy_wallet_observation(
                &spec.id,
                frozen.revision - 1,
                "legacy-route",
                usage,
                Outcome::Completed,
                "synthetic-full-audit"
            )
            .await
            .unwrap_err(),
        Error::WrongState
    );
    engine
        .set_budget(
            MEMBER_SESSION,
            &Budget {
                scope: "self".into(),
                account_id: None,
                user_id: Some(2),
                key_id: None,
                period: "day".into(),
                anchor: 0,
                seconds: 0,
                limit: 10,
            },
        )
        .await
        .unwrap();
    assert_eq!(
        engine
            .reconcile_legacy_wallet_observation(
                &spec.id,
                frozen.revision,
                "legacy-route",
                usage,
                Outcome::Completed,
                "synthetic-full-audit"
            )
            .await
            .unwrap_err(),
        Error::BudgetExceeded
    );
    let terminal: bool = sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM core_charging.usage_events WHERE request_id=$1 AND event_id='model-provider-terminal:v1')").bind(&spec.id).fetch_one(&pool).await.unwrap();
    assert!(!terminal); // Reconstructed authority and funding roll back together.
    engine
        .set_budget(
            MEMBER_SESSION,
            &Budget {
                scope: "self".into(),
                account_id: None,
                user_id: Some(2),
                key_id: None,
                period: "day".into(),
                anchor: 0,
                seconds: 0,
                limit: 20,
            },
        )
        .await
        .unwrap();
    let settled = engine
        .reconcile_legacy_wallet_observation(
            &spec.id,
            frozen.revision,
            "legacy-route",
            usage,
            Outcome::Completed,
            "synthetic-full-audit",
        )
        .await
        .unwrap();
    assert_eq!(settled.settled, 15);
    assert_eq!(
        engine
            .reconcile_legacy_wallet_observation(
                &spec.id,
                frozen.revision,
                "legacy-route",
                usage,
                Outcome::Completed,
                "synthetic-full-audit"
            )
            .await
            .unwrap(),
        settled
    );
}

#[sqlx::test(migrations = false)]
async fn legacy_expression_terminal_float_differs_from_native_exact(pool: PgPool) {
    fixture(&pool).await;
    let wallet = WalletLedger::from_pool(pool.clone());
    wallet.open_wallet(102).await.unwrap();
    credit_wallet(&wallet, "float-difference", 50).await;
    let engine = Charging::new(pool.clone(), Arc::new(TransactionalLedger));
    let (mut spec, basis, proof) = prepared_expression("legacy-float-terminal");
    let expression = r#"tier("standard",p*2+c*4+cr*2.3)"#;
    let mut frozen = serde_json::to_value(spec.price.tiered.as_ref().unwrap()).unwrap();
    frozen["tariff"]["source_digest"] = contract::digest(expression.as_bytes()).into();
    frozen["tariff"]["uses_cache_read"] = true.into();
    frozen["tariff"]["tiers"][0]["rates"]["cache_read"] =
        serde_json::json!({"numerator":23,"denominator":10});
    spec.price.tiered = Some(serde_json::from_value(frozen).unwrap());
    spec.reserve = spec.price.reservation_ceiling(basis).unwrap();
    let LegacyPrechargeAuthorization::Cc52TieredWalletV1 { mut estimate, .. } =
        proof.legacy_precharge().unwrap().clone()
    else {
        panic!("expression proof");
    };
    estimate.source_expression = expression.into();
    let proof = proof
        .with_legacy_precharge(LegacyPrechargeAuthorization::Cc52TieredWalletV1 {
            source_contract_sha256: "05".repeat(32),
            estimate_sha256: contract::typed_digest(&estimate).unwrap(),
            estimated_hold: estimate.hold().unwrap(),
            operator_max_billable: 125,
            estimate,
        })
        .unwrap();
    assert_eq!(
        engine
            .reserve_prepared_model_with_context(
                PERSONAL, &spec, &WORKER, None, None, basis, &proof
            )
            .await
            .unwrap()
            .reserved,
        45
    );
    engine.start(&spec.id, &WORKER).await.unwrap();
    let usage = Usage {
        input: 50,
        cached: 50,
        ..Usage::default()
    };
    assert_eq!(spec.price.cost(usage), Ok(58));
    terminal(&pool, &spec, usage).await;
    assert_eq!(engine.terminal_amount(&spec.id, usage).await, Ok(57));
    let settled = engine
        .settle_legacy_wallet_terminal(&spec.id, &WORKER, usage, Outcome::Completed)
        .await
        .unwrap();
    assert_eq!(settled.settled, 57);
    assert_eq!(
        engine
            .settle_legacy_wallet_terminal(&spec.id, &WORKER, usage, Outcome::Completed)
            .await
            .unwrap(),
        settled
    );
    let balance = wallet.balance(102).await.unwrap();
    assert_eq!((balance.spendable_units, balance.debt_units), (0, -7));
    engine
        .refund(&spec.id, "float-refund", 57, "synthetic float refund")
        .await
        .unwrap();
    let balance = wallet.balance(102).await.unwrap();
    assert_eq!((balance.spendable_units, balance.debt_units), (50, 0));
}

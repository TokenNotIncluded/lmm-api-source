//! Synthetic signed Pancake callbacks through real Core proof and ledger paths.
use super::*;

fn refund_proof(
    intent: &pb::PaymentIntent,
    refund: &str,
    delivery: &str,
    amount: i64,
    status: &str,
) -> pb::ProviderEvidence {
    let mut evidence = quantity_paid_proof(intent);
    let mut value: Value = serde_json::from_slice(&evidence.signed_payload).unwrap();
    value["id"] = json!(delivery);
    value["eventId"] = json!(refund);
    value["eventType"] = json!(format!("refund.{status}"));
    value["data"]["refundStatus"] = json!(status);
    value["data"]["originalChargedAmount"] = value["data"]["chargedAmount"].clone();
    value["data"]["refundedAmount"] = json!(format!("{}.{:02}", amount / 100, amount % 100));
    // A ticket reference resembling a hold must remain external evidence.
    value["data"]["refundTicketMerchantExternalId"] = json!("not_local_hold_authority");
    evidence.signed_payload = serde_json::to_vec(&value).unwrap();
    let timestamp = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_millis();
    let mut signed = format!("{timestamp}.").into_bytes();
    signed.extend_from_slice(&evidence.signed_payload);
    let signature = quantity_fixture_rsa()
        .sign(Pkcs1v15Sign::new::<Sha256>(), &Sha256::digest(&signed))
        .unwrap();
    evidence.signature = format!("t={timestamp},v1={}", STANDARD.encode(signature)).into_bytes();
    evidence.provider_event_id = delivery.into();
    evidence
}

#[sqlx::test]
async fn pancake_signed_external_refunds_use_original_payment_and_cumulative_cap(pool: PgPool) {
    let mut f = fresh(&pool).await;
    f.store = quantity_store(&pool, quantity_policy(1, 1));
    let intent = f
        .store
        .prepare(
            &f.secret,
            quantity_request(&f, "pancake_refund_prepare", 500_000),
        )
        .await
        .unwrap();
    assert_eq!(intent.amount_minor, 100);
    let mut paid = Some(quantity_paid_proof(&intent));
    f.store.receive(intent.intent_id, &mut paid).await.unwrap();
    assert!(!paid.as_ref().unwrap().accepted_proof_id.is_empty());
    let credited = f
        .store
        .credit(credit_request(
            &intent,
            "pancake_refund_credit",
            paid.unwrap(),
        ))
        .await
        .unwrap();
    balance(&f, intent.credit_units, 0).await;
    let before = journals(&pool).await;

    let failed = refund_proof(&intent, "REF_failure", "delivery_failure", 20, "failed");
    let mut admitted = Some(failed.clone());
    f.store
        .receive(intent.intent_id, &mut admitted)
        .await
        .unwrap();
    let failure = pb::ExternalRefundRequest {
        mutation: mutation("pancake_failed_no_money"),
        intent_id: intent.intent_id,
        evidence: admitted,
    };
    assert_eq!(
        f.store.external_refund(failure).await.unwrap_err(),
        Error::Invalid
    );
    balance(&f, intent.credit_units, 0).await;
    assert_eq!(journals(&pool).await, before);

    let evidence = refund_proof(&intent, "REF_partial", "delivery_partial", 40, "succeeded");
    let request = pb::ExternalRefundRequest {
        mutation: mutation("pancake_partial_refund"),
        intent_id: intent.intent_id,
        evidence: Some(evidence),
    };
    let applied = f.store.external_refund(request.clone()).await.unwrap();
    assert_ne!(applied.journal_id, credited.journal_id);
    assert!(applied.journal_id > 0);
    assert_eq!(f.store.external_refund(request).await.unwrap(), applied);
    balance(&f, intent.credit_units * 60 / 100, 0).await;

    // A second delivery of one business refund is not a second refund.
    let retry = pb::ExternalRefundRequest {
        mutation: mutation("pancake_new_delivery"),
        intent_id: intent.intent_id,
        evidence: Some(refund_proof(
            &intent,
            "REF_partial",
            "delivery_partial_again",
            40,
            "succeeded",
        )),
    };
    assert_eq!(
        f.store.external_refund(retry).await.unwrap_err(),
        Error::Conflict
    );
    assert_eq!(journals(&pool).await, before + 1);
    balance(&f, intent.credit_units * 60 / 100, 0).await;

    // Each callback is below the original charge; cumulative capacity still
    // belongs to Core, not to the provider parser.
    let over = pb::ExternalRefundRequest {
        mutation: mutation("pancake_cumulative_overflow"),
        intent_id: intent.intent_id,
        evidence: Some(refund_proof(
            &intent,
            "REF_over",
            "delivery_over",
            70,
            "succeeded",
        )),
    };
    assert_eq!(
        f.store.external_refund(over).await.unwrap_err(),
        Error::Conflict
    );
    balance(&f, intent.credit_units * 60 / 100, 0).await;
    assert_eq!(journals(&pool).await, before + 1);
    let rest = pb::ExternalRefundRequest {
        mutation: mutation("pancake_refund_remainder"),
        intent_id: intent.intent_id,
        evidence: Some(refund_proof(
            &intent,
            "REF_rest",
            "delivery_rest",
            60,
            "succeeded",
        )),
    };
    f.store.external_refund(rest).await.unwrap();
    balance(&f, 0, 0).await;
    let totals: (i64, i64) = sqlx::query_as("SELECT refunded_minor,(SELECT count(*) FROM core_payments.provider_refunds WHERE intent_id=$1) FROM core_payments.intents WHERE id=$1")
        .bind(intent.intent_id).fetch_one(&pool).await.unwrap();
    assert_eq!(totals, (100, 2));
    assert_eq!(journals(&pool).await, before + 2);
    let unbalanced: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM (SELECT journal_id FROM core_billing.ledger_entries GROUP BY journal_id HAVING sum(delta_units) <> 0) q",
    ).fetch_one(&pool).await.unwrap();
    assert_eq!(unbalanced, 0, "every payment/refund journal must balance");
    let refund_delta: i64 = sqlx::query_scalar(
        "SELECT coalesce(sum(e.delta_units),0)::bigint FROM core_billing.ledger_entries e JOIN core_billing.ledger_accounts a ON a.id=e.ledger_account_id JOIN core_billing.ledger_journals j ON j.id=e.journal_id WHERE j.kind='refund_topup' AND a.owner_account_id=$1",
    ).bind(f.account).fetch_one(&pool).await.unwrap();
    assert_eq!(
        refund_delta, -intent.credit_units,
        "refund journals must reverse exactly the credited units"
    );
}

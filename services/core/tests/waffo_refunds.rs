//! Real Core payment and ledger integration against an explicitly synthetic
//! loopback Waffo provider. No external gateway or real funds are used.
use axum::{
    body::Bytes,
    extract::{OriginalUri, State},
    http::{HeaderMap, StatusCode},
    response::{IntoResponse, Response},
    routing::post,
};
use base64::{Engine, engine::general_purpose::STANDARD};
use lmm_core::{
    billing::ledger::{self, Ledger},
    identity::IdentityStore,
    internal_rpc::pb,
    payments::{Channel, Error, PaymentsStore},
};
use rand::{SeedableRng, rngs::StdRng};
use rsa::{
    Pkcs1v15Sign, RsaPrivateKey,
    pkcs8::{EncodePrivateKey, EncodePublicKey},
};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use std::sync::{
    Arc, Mutex,
    atomic::{AtomicUsize, Ordering},
};

fn provider_key(parts: &[&str]) -> String {
    let mut digest = Sha256::new();
    for part in parts {
        digest.update(format!("{}:", part.len()));
        digest.update(part.as_bytes());
    }
    let bytes = digest.finalize();
    format!(
        "pay_{}",
        bytes.iter().map(|b| format!("{b:02x}")).collect::<String>()
    )
}
fn mutation(key: &str) -> Option<pb::MutationContext> {
    Some(pb::MutationContext {
        idempotency_key: key.into(),
        expected_version: None,
    })
}
struct Provider {
    key: RsaPrivateKey,
    payment: Mutex<Value>,
    refund: Mutex<Value>,
    calls: AtomicUsize,
}
struct Fixture {
    state: Arc<Provider>,
    origin: String,
    task: tokio::task::JoinHandle<()>,
}
impl Drop for Fixture {
    fn drop(&mut self) {
        self.task.abort();
    }
}
impl Fixture {
    async fn start() -> Self {
        let state = Arc::new(Provider {
            key: RsaPrivateKey::new(&mut StdRng::seed_from_u64(8716), 2048).unwrap(),
            payment: Mutex::new(json!({"code":"unavailable"})),
            refund: Mutex::new(json!({"code":"unavailable"})),
            calls: AtomicUsize::new(0),
        });
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let origin = format!("http://{}", listener.local_addr().unwrap());
        let router = axum::Router::new()
            .route("/api/v1/order/inquiry", post(inquiry))
            .route("/api/v1/refund/inquiry", post(inquiry))
            .with_state(state.clone());
        let task = tokio::spawn(async move {
            axum::serve(listener, router).await.unwrap();
        });
        Self {
            state,
            origin,
            task,
        }
    }
    fn channel(&self) -> Channel {
        Channel::waffo_isolated_test(
            7,
            "merchant_fixture".into(),
            self.state
                .key
                .to_public_key()
                .to_public_key_pem(Default::default())
                .unwrap(),
            self.state
                .key
                .to_pkcs8_pem(Default::default())
                .unwrap()
                .to_string(),
            &self.origin,
            10000,
        )
        .unwrap()
    }
    fn payment(&self, order: &str) {
        *self.state.payment.lock().unwrap() = json!({"code":"0","data":{
            "merchantOrderId":order,"paymentRequestId":order,"acquiringOrderId":"acq_refund_pg",
            "orderStatus":"PAY_SUCCESS","orderCurrency":"USD","orderAmount":"1.00",
            "merchantInfo":{"merchantId":"merchant_fixture"}
        }});
    }
    fn refund(&self, value: &Value) {
        *self.state.refund.lock().unwrap() = json!({"code":"0","data":value});
    }
    fn evidence(&self, result: Value, refund: bool) -> pb::ProviderEvidence {
        let event_type = if refund {
            "REFUND_NOTIFICATION"
        } else {
            "PAYMENT_NOTIFICATION"
        };
        let event_id = if refund {
            provider_key(&[
                result["acquiringRefundOrderId"].as_str().unwrap(),
                event_type,
                result["refundStatus"].as_str().unwrap(),
            ])
        } else {
            provider_key(&["acq_refund_pg", event_type, "PAY_SUCCESS"])
        };
        let raw = serde_json::to_vec(&json!({"eventType":event_type,"result":result})).unwrap();
        let signature = STANDARD.encode(
            self.state
                .key
                .sign(Pkcs1v15Sign::new::<Sha256>(), &Sha256::digest(&raw))
                .unwrap(),
        );
        pb::ProviderEvidence {
            payment: Some(pb::ExternalPaymentReference {
                provider: "waffo".into(),
                merchant: "merchant_fixture".into(),
                environment: "test".into(),
                transaction: "acq_refund_pg".into(),
            }),
            provider_event_id: event_id,
            signed_payload: raw,
            signature: signature.into_bytes(),
            accepted_proof_id: String::new(),
        }
    }
}
async fn inquiry(
    State(state): State<Arc<Provider>>,
    OriginalUri(uri): OriginalUri,
    headers: HeaderMap,
    body: Bytes,
) -> Response {
    state.calls.fetch_add(1, Ordering::SeqCst);
    let signature = headers
        .get("X-SIGNATURE")
        .and_then(|value| STANDARD.decode(value.as_bytes()).ok());
    let verified = signature.is_some_and(|signature| {
        state
            .key
            .to_public_key()
            .verify(
                Pkcs1v15Sign::new::<Sha256>(),
                &Sha256::digest(&body),
                &signature,
            )
            .is_ok()
    });
    if !verified
        || headers
            .get("X-API-KEY")
            .and_then(|value| value.to_str().ok())
            != Some("isolated_waffo_synthetic_key")
        || headers
            .get("X-API-VERSION")
            .and_then(|value| value.to_str().ok())
            != Some("1.0.0")
    {
        return StatusCode::UNAUTHORIZED.into_response();
    }
    let query: Value = serde_json::from_slice(&body).unwrap();
    let is_refund = uri.path() == "/api/v1/refund/inquiry";
    if is_refund {
        if query.as_object().unwrap().len() != 2
            || query["refundRequestId"].as_str().is_none()
            || query["acquiringRefundOrderId"].as_str().is_none()
        {
            return StatusCode::BAD_REQUEST.into_response();
        }
    } else if query.as_object().unwrap().len() != 1
        || !query["paymentRequestId"]
            .as_str()
            .is_some_and(|value| value.starts_with("top_"))
    {
        return StatusCode::BAD_REQUEST.into_response();
    }
    let response = if is_refund {
        state.refund.lock().unwrap().clone()
    } else {
        state.payment.lock().unwrap().clone()
    };
    let bytes = serde_json::to_vec(&response).unwrap();
    let signature = STANDARD.encode(
        state
            .key
            .sign(Pkcs1v15Sign::new::<Sha256>(), &Sha256::digest(&bytes))
            .unwrap(),
    );
    ([("X-SIGNATURE", signature)], bytes).into_response()
}

#[tokio::test]
async fn isolated_waffo_constructor_rejects_external_origin_and_live_relabel() {
    let fixture = Fixture::start().await;
    let public = fixture
        .state
        .key
        .to_public_key()
        .to_public_key_pem(Default::default())
        .unwrap();
    let private = fixture
        .state
        .key
        .to_pkcs8_pem(Default::default())
        .unwrap()
        .to_string();
    for origin in [
        "https://api.waffo.com",
        "http://example.com:1234",
        "http://127.0.0.1",
        "http://127.0.0.1:0",
        "http://user@127.0.0.1:1234",
        "http://127.0.0.1:1234/path",
        "http://127.0.0.1:1234/?q=x",
    ] {
        assert!(
            Channel::waffo_isolated_test(
                7,
                "merchant_fixture".into(),
                public.clone(),
                private.clone(),
                origin,
                10000
            )
            .is_err(),
            "{origin}"
        );
    }
    let mut channel = fixture.channel();
    channel.environment = "live".into();
    let pool = sqlx::postgres::PgPoolOptions::new()
        .connect_lazy("postgresql://synthetic@127.0.0.1:1/synthetic")
        .unwrap();
    assert!(PaymentsStore::new(pool, vec![channel]).is_err());
    assert_eq!(fixture.state.calls.load(Ordering::SeqCst), 0);
}

#[sqlx::test]
async fn waffo_refunds_bind_original_dispatch_and_post_only_verified_facts(pool: sqlx::PgPool) {
    let identity = IdentityStore::from_pool(pool.clone());
    identity.init_database().await.unwrap();
    sqlx::raw_sql(lmm_core::abuse::SCHEMA)
        .execute(&pool)
        .await
        .unwrap();
    sqlx::raw_sql(ledger::SCHEMA).execute(&pool).await.unwrap();
    sqlx::raw_sql(lmm_core::payments::SCHEMA)
        .execute(&pool)
        .await
        .unwrap();
    let session = identity.bootstrap_user(7411, 0).await.unwrap();
    let account = identity
        .authorize(&session.secret)
        .await
        .unwrap()
        .owner_account_id;
    let ledger = Ledger::from_pool(pool.clone());
    let fixture = Fixture::start().await;
    let store = PaymentsStore::new(pool.clone(), vec![fixture.channel()]).unwrap();
    let intent = store
        .prepare_personal(
            &session.secret,
            pb::PreparePersonalTopupRequest {
                mutation: mutation("waffo_refund_quote"),
                payment_channel_id: 7,
                currency: "USD".into(),
                amount_minor: 100,
            },
        )
        .await
        .unwrap();
    let order = format!("top_{}", intent.intent_id);
    fixture.payment(&order);
    let payment = fixture.state.payment.lock().unwrap()["data"].clone();
    let mut paid = fixture.evidence(payment, false);
    paid.accepted_proof_id = store
        .accept_proof(pb::AcceptProviderProofRequest {
            intent_id: intent.intent_id,
            evidence: Some(paid.clone()),
        })
        .await
        .unwrap()
        .proof_id;
    store
        .credit(pb::CreditTopupRequest {
            mutation: mutation("waffo_refund_credit"),
            intent_id: intent.intent_id,
            evidence: Some(paid),
        })
        .await
        .unwrap();
    let hold = store
        .hold(
            &session.secret,
            pb::HoldRefundRequest {
                mutation: mutation("waffo_partial_hold"),
                intent_id: intent.intent_id,
                refund_amount_minor: 30,
            },
        )
        .await
        .unwrap();
    let balance = ledger.balance(account).await.unwrap();
    assert_eq!(
        (balance.available_units, balance.reserved_units),
        (350_000, 150_000)
    );
    assert!(
        store
            .resolve(
                pb::ResolveRefundRequest {
                    mutation: mutation("waffo_unknown_release"),
                    intent_id: intent.intent_id,
                    hold_receipt_id: hold.receipt_id.clone(),
                    evidence: None
                },
                false
            )
            .await
            .is_err()
    );
    let local = "pay_synthetic_partial_refund";
    let dispatch = provider_key(&["submit", local]);
    store
        .begin_refund_dispatch(pb::ResolveRefundRequest {
            mutation: mutation(&dispatch),
            intent_id: intent.intent_id,
            hold_receipt_id: hold.receipt_id.clone(),
            evidence: None,
        })
        .await
        .unwrap();
    let mut result = json!({
        "refundRequestId":dispatch,"merchantRefundOrderId":local,"acquiringOrderId":"acq_refund_pg",
        "acquiringRefundOrderId":"refund_partial_pg","origPaymentRequestId":order,"refundAmount":"0.30",
        "refundStatus":"REFUND_IN_PROGRESS","userCurrency":"USD",
        "extendInfo":json!({"lmm_hold_receipt_id":hold.receipt_id,"lmm_refund_id":local}).to_string()
    });
    fixture.refund(&result);
    store
        .accept_proof(pb::AcceptProviderProofRequest {
            intent_id: intent.intent_id,
            evidence: Some(fixture.evidence(result.clone(), true)),
        })
        .await
        .unwrap();
    result["refundStatus"] = json!("ORDER_PARTIALLY_REFUNDED");
    let mut completed = fixture.evidence(result.clone(), true);
    let mut wrong_query = result.clone();
    wrong_query["refundAmount"] = json!("0.31");
    fixture.refund(&wrong_query);
    assert_eq!(
        store
            .accept_proof(pb::AcceptProviderProofRequest {
                intent_id: intent.intent_id,
                evidence: Some(completed.clone())
            })
            .await,
        Err(Error::Conflict)
    );
    let balance = ledger.balance(account).await.unwrap();
    assert_eq!(
        (balance.available_units, balance.reserved_units),
        (350_000, 150_000)
    );
    fixture.refund(&result);
    let mut forged = completed.clone();
    forged.signature = b"not-provider-signed".to_vec();
    assert!(
        store
            .accept_proof(pb::AcceptProviderProofRequest {
                intent_id: intent.intent_id,
                evidence: Some(forged)
            })
            .await
            .is_err()
    );
    completed.accepted_proof_id = store
        .accept_proof(pb::AcceptProviderProofRequest {
            intent_id: intent.intent_id,
            evidence: Some(completed.clone()),
        })
        .await
        .unwrap()
        .proof_id;
    let request = pb::ResolveRefundRequest {
        mutation: mutation("waffo_partial_commit"),
        intent_id: intent.intent_id,
        hold_receipt_id: hold.receipt_id.clone(),
        evidence: Some(completed),
    };
    let receipt = store.resolve(request.clone(), true).await.unwrap();
    assert_eq!(receipt.state, 3);
    assert_eq!(store.resolve(request, true).await.unwrap(), receipt);
    let balance = ledger.balance(account).await.unwrap();
    assert_eq!(
        (balance.available_units, balance.reserved_units),
        (350_000, 0)
    );
    let hold = store
        .hold(
            &session.secret,
            pb::HoldRefundRequest {
                mutation: mutation("waffo_failed_hold"),
                intent_id: intent.intent_id,
                refund_amount_minor: 20,
            },
        )
        .await
        .unwrap();
    let local = "pay_synthetic_failed_refund";
    let dispatch = provider_key(&["submit", local]);
    store
        .begin_refund_dispatch(pb::ResolveRefundRequest {
            mutation: mutation(&dispatch),
            intent_id: intent.intent_id,
            hold_receipt_id: hold.receipt_id.clone(),
            evidence: None,
        })
        .await
        .unwrap();
    let failed_result = json!({
        "refundRequestId":dispatch,"merchantRefundOrderId":local,"acquiringOrderId":"acq_refund_pg",
        "acquiringRefundOrderId":"refund_failed_pg","origPaymentRequestId":order,"refundAmount":"0.20",
        "refundStatus":"ORDER_REFUND_FAILED","userCurrency":"USD",
        "extendInfo":json!({"lmm_hold_receipt_id":hold.receipt_id,"lmm_refund_id":local}).to_string()
    });
    fixture.refund(&failed_result);
    let mut failed = fixture.evidence(failed_result, true);
    let accept = pb::AcceptProviderProofRequest {
        intent_id: intent.intent_id,
        evidence: Some(failed.clone()),
    };
    let accepted = store.accept_proof(accept.clone()).await.unwrap();
    *fixture.state.payment.lock().unwrap() = json!({"code":"provider_unavailable"});
    let queries = fixture.state.calls.load(Ordering::SeqCst);
    assert_eq!(store.accept_proof(accept).await.unwrap(), accepted);
    assert_eq!(
        fixture.state.calls.load(Ordering::SeqCst),
        queries,
        "durable proof replay queried provider again"
    );
    failed.accepted_proof_id = accepted.proof_id;
    let request = pb::ResolveRefundRequest {
        mutation: mutation("waffo_failed_release"),
        intent_id: intent.intent_id,
        hold_receipt_id: hold.receipt_id.clone(),
        evidence: Some(failed),
    };
    let released = store.resolve(request.clone(), false).await.unwrap();
    assert_eq!(released.state, 4);
    assert_eq!(store.resolve(request, false).await.unwrap(), released);
    let balance = ledger.balance(account).await.unwrap();
    assert_eq!(
        (balance.available_units, balance.reserved_units),
        (350_000, 0)
    );
    assert_eq!(
        store
            .hold(
                &session.secret,
                pb::HoldRefundRequest {
                    mutation: mutation("waffo_cumulative_limit"),
                    intent_id: intent.intent_id,
                    refund_amount_minor: 71
                }
            )
            .await,
        Err(Error::Conflict)
    );
    let refunded: i64 =
        sqlx::query_scalar("SELECT refunded_minor FROM core_payments.intents WHERE id=$1")
            .bind(intent.intent_id)
            .fetch_one(&pool)
            .await
            .unwrap();
    assert_eq!(refunded, 30);
    let bindings: i64 =
        sqlx::query_scalar("SELECT count(*) FROM core_payments.refund_dispatch_bindings")
            .fetch_one(&pool)
            .await
            .unwrap();
    assert_eq!(bindings, 2);
    let net: i64 =
        sqlx::query_scalar("SELECT sum(delta_units)::bigint FROM core_billing.ledger_entries")
            .fetch_one(&pool)
            .await
            .unwrap();
    assert_eq!(net, 0);
}

//! Waffo Global's verification key is shared by provider accounts. Core also
//! reads the payment through its own authenticated, fixed-endpoint inquiry.
//! The wire contract follows waffo-go v2.1.0's order/inquiry and HTTP client.
use std::time::Duration;

use base64::{Engine, engine::general_purpose::STANDARD};
use reqwest::{Client, header::HeaderValue, redirect::Policy};
use rsa::{Pkcs1v15Sign, RsaPrivateKey, pkcs8::DecodePrivateKey, traits::PublicKeyParts};
use serde_json::json;
use sha2::{Digest, Sha256};
use zeroize::Zeroizing;

use super::{Channel, Error, Result, providers, stripe};

const MAX_RESPONSE: usize = 32 * 1024;
const TEST_ENDPOINT: &str = "https://api-sandbox.waffo.com/api/v1/order/inquiry";
const LIVE_ENDPOINT: &str = "https://api.waffo.com/api/v1/order/inquiry";

/// These account credentials stay inside Core. No caller selects the URL or
/// request body, and no inquiry credentials appear in RPC messages or logs.
pub(crate) struct Inquiry {
    client: Client,
    api_key: HeaderValue,
    private_key: RsaPrivateKey,
    #[cfg(test)]
    test_endpoint: Option<String>,
}

fn client() -> reqwest::ClientBuilder {
    Client::builder()
        .no_proxy()
        .redirect(Policy::none())
        .connect_timeout(Duration::from_secs(2))
        .timeout(Duration::from_secs(2))
}

impl Inquiry {
    pub(crate) fn new(api_key: String, private_key: String) -> Result<Self> {
        let api_key = Zeroizing::new(api_key);
        let private_key = Zeroizing::new(private_key);
        if api_key.is_empty()
            || api_key.len() > 4096
            || !api_key.bytes().all(|b| b.is_ascii_graphic())
        {
            return Err(Error::Invalid);
        }
        let mut api_key = HeaderValue::from_str(&api_key).map_err(|_| Error::Invalid)?;
        api_key.set_sensitive(true);
        let private_key = private_key.trim();
        let key = if private_key.starts_with("-----BEGIN PRIVATE KEY-----") {
            RsaPrivateKey::from_pkcs8_pem(private_key).map_err(|_| Error::Invalid)?
        } else {
            let der = Zeroizing::new(STANDARD.decode(private_key).map_err(|_| Error::Invalid)?);
            RsaPrivateKey::from_pkcs8_der(&der).map_err(|_| Error::Invalid)?
        };
        if !(2048..=8192).contains(&key.n().bits()) || key.validate().is_err() {
            return Err(Error::Invalid);
        }
        Ok(Self {
            client: client()
                .https_only(true)
                .build()
                .map_err(|_| Error::Unavailable)?,
            api_key,
            private_key: key,
            #[cfg(test)]
            test_endpoint: None,
        })
    }

    fn endpoint(&self, environment: &str) -> Result<&str> {
        let endpoint = match environment {
            "test" => TEST_ENDPOINT,
            "live" => LIVE_ENDPOINT,
            _ => return Err(Error::Invalid),
        };
        #[cfg(test)]
        if let Some(endpoint) = &self.test_endpoint {
            return Ok(endpoint);
        }
        Ok(endpoint)
    }

    pub(super) async fn confirm(
        &self,
        channel: &Channel,
        intent_id: i64,
        amount_minor: i64,
        transaction: &str,
    ) -> Result<()> {
        if channel.provider != "waffo" || intent_id <= 0 {
            return Err(Error::Invalid);
        }
        let order = format!("top_{intent_id}");
        let body =
            serde_json::to_vec(&json!({"paymentRequestId":order})).map_err(|_| Error::Invalid)?;
        let signature = self
            .private_key
            .sign_with_rng(
                &mut rand::rngs::OsRng,
                Pkcs1v15Sign::new::<Sha256>(),
                &Sha256::digest(&body),
            )
            .map_err(|_| Error::Unavailable)?;
        let mut response = self
            .client
            .post(self.endpoint(&channel.environment)?)
            .header("Content-Type", "application/json")
            .header("X-API-KEY", self.api_key.clone())
            .header("X-API-VERSION", "1.0.0")
            .header("X-SIGNATURE", STANDARD.encode(signature))
            .body(body)
            .send()
            .await
            .map_err(|_| Error::Unavailable)?;
        if !response.status().is_success()
            || response
                .content_length()
                .is_some_and(|size| size > MAX_RESPONSE as u64)
            || response.headers().get_all("X-SIGNATURE").iter().count() > 1
        {
            return Err(Error::Unavailable);
        }
        let signature = response.headers().get("X-SIGNATURE").cloned();
        let mut body = Vec::new();
        while let Some(chunk) = response.chunk().await.map_err(|_| Error::Unavailable)? {
            if chunk.len() > MAX_RESPONSE.saturating_sub(body.len()) {
                return Err(Error::Unavailable);
            }
            body.extend_from_slice(&chunk);
        }
        // The SDK permits unsigned inquiry replies. Their source is still the
        // fixed HTTPS endpoint queried with Core's own account credentials.
        if let Some(signature) = signature {
            providers::rsa_verify(&channel.webhook_secrets, &body, signature.as_bytes())?;
        }
        let response = stripe::unique_json(&body)?;
        if response.get("code").and_then(|v| v.as_str()) != Some("0") {
            return Err(Error::Unavailable);
        }
        let data = response.get("data").ok_or(Error::Invalid)?;
        if data.get("merchantOrderId").and_then(|v| v.as_str()) != Some(&order)
            || data.get("paymentRequestId").and_then(|v| v.as_str()) != Some(&order)
            || data.get("acquiringOrderId").and_then(|v| v.as_str()) != Some(transaction)
            || data.get("orderStatus").and_then(|v| v.as_str()) != Some("PAY_SUCCESS")
            || data.get("orderCurrency").and_then(|v| v.as_str()) != Some(&channel.currency)
            || data
                .get("merchantInfo")
                .and_then(|v| v.get("merchantId"))
                .and_then(|v| v.as_str())
                != Some(&channel.merchant)
        {
            return Err(Error::Conflict);
        }
        let amount = data
            .get("orderAmount")
            .and_then(|v| v.as_str())
            .ok_or(Error::Invalid)?;
        if providers::decimal_minor(amount)? != amount_minor {
            return Err(Error::Conflict);
        }
        Ok(())
    }
}

#[cfg(test)]
pub(super) mod tests {
    use super::*;
    use axum::{
        body::Bytes,
        extract::State,
        http::{HeaderMap, StatusCode},
        response::{IntoResponse, Response},
        routing::post,
    };
    use rsa::pkcs8::{EncodePrivateKey, EncodePublicKey};
    use serde_json::Value;
    use std::sync::{
        Arc, Mutex,
        atomic::{AtomicUsize, Ordering},
    };

    const API_KEY: &str = "core_only_synthetic_inquiry_key";
    struct Reply {
        status: StatusCode,
        body: Vec<u8>,
        signature: Option<String>,
    }
    struct ServerState {
        key: rsa::RsaPublicKey,
        reply: Mutex<Reply>,
        calls: AtomicUsize,
    }
    pub(crate) struct Fixture {
        endpoint: String,
        key: RsaPrivateKey,
        state: Arc<ServerState>,
        task: tokio::task::JoinHandle<()>,
    }
    impl Drop for Fixture {
        fn drop(&mut self) {
            self.task.abort();
        }
    }
    impl Fixture {
        pub(crate) async fn start(key: RsaPrivateKey) -> Self {
            let state = Arc::new(ServerState {
                key: key.to_public_key(),
                reply: Mutex::new(Reply {
                    status: StatusCode::SERVICE_UNAVAILABLE,
                    body: vec![],
                    signature: None,
                }),
                calls: AtomicUsize::new(0),
            });
            let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
            let endpoint = format!(
                "http://{}/api/v1/order/inquiry",
                listener.local_addr().unwrap()
            );
            let router = axum::Router::new()
                .route("/api/v1/order/inquiry", post(handle))
                .with_state(state.clone());
            let task = tokio::spawn(async move {
                axum::serve(listener, router).await.unwrap();
            });
            Self {
                endpoint,
                key,
                state,
                task,
            }
        }
        pub(crate) fn inquiry(&self) -> Inquiry {
            let mut inquiry = Inquiry::new(
                API_KEY.into(),
                self.key
                    .to_pkcs8_pem(Default::default())
                    .unwrap()
                    .to_string(),
            )
            .unwrap();
            // There is deliberately no corresponding production configuration.
            inquiry.client = client().build().unwrap();
            inquiry.test_endpoint = Some(self.endpoint.clone());
            inquiry
        }
        pub(crate) fn reply(&self, response: Value) {
            let body = serde_json::to_vec(&response).unwrap();
            let signature = STANDARD.encode(
                self.key
                    .sign(Pkcs1v15Sign::new::<Sha256>(), &Sha256::digest(&body))
                    .unwrap(),
            );
            self.raw_reply(StatusCode::OK, body, Some(signature));
        }
        fn raw_reply(&self, status: StatusCode, body: Vec<u8>, signature: Option<String>) {
            *self.state.reply.lock().unwrap() = Reply {
                status,
                body,
                signature,
            };
        }
        pub(crate) fn calls(&self) -> usize {
            self.state.calls.load(Ordering::SeqCst)
        }
    }
    async fn handle(
        State(state): State<Arc<ServerState>>,
        headers: HeaderMap,
        body: Bytes,
    ) -> Response {
        state.calls.fetch_add(1, Ordering::SeqCst);
        let valid_body = serde_json::from_slice::<Value>(&body).is_ok_and(|value| {
            value.as_object().is_some_and(|v| v.len() == 1)
                && value
                    .get("paymentRequestId")
                    .and_then(Value::as_str)
                    .and_then(|s| s.strip_prefix("top_"))
                    .and_then(|s| s.parse::<i64>().ok())
                    .is_some_and(|id| id > 0)
        });
        let valid_signature = headers
            .get("X-SIGNATURE")
            .and_then(|value| STANDARD.decode(value.as_bytes()).ok())
            .is_some_and(|signature| {
                state
                    .key
                    .verify(
                        Pkcs1v15Sign::new::<Sha256>(),
                        &Sha256::digest(&body),
                        &signature,
                    )
                    .is_ok()
            });
        if headers.get("X-API-KEY").and_then(|v| v.to_str().ok()) != Some(API_KEY)
            || headers.get("X-API-VERSION").and_then(|v| v.to_str().ok()) != Some("1.0.0")
            || !valid_body
            || !valid_signature
        {
            return StatusCode::UNAUTHORIZED.into_response();
        }
        let reply = state.reply.lock().unwrap();
        let mut response = (reply.status, reply.body.clone()).into_response();
        if let Some(signature) = &reply.signature {
            response
                .headers_mut()
                .insert("X-SIGNATURE", HeaderValue::from_str(signature).unwrap());
        }
        response
    }
    pub(crate) fn payment_response(order: &str, transaction: &str, merchant: &str) -> Value {
        json!({"code":"0", "data":{
            "merchantOrderId":order, "paymentRequestId":order, "acquiringOrderId":transaction,
            "orderStatus":"PAY_SUCCESS", "orderCurrency":"USD", "orderAmount":"1.00",
            "merchantInfo":{"merchantId":merchant}
        }})
    }
    #[tokio::test]
    async fn inquiry_checks_account_payment_and_bounded_provider_response() {
        use rand::{SeedableRng, rngs::StdRng};
        let key = RsaPrivateKey::new(&mut StdRng::seed_from_u64(924), 2048).unwrap();
        let fixture = Fixture::start(key.clone()).await;
        let inquiry = fixture.inquiry();
        let mut channel = Channel::stripe_test(1, "merchant_fixture".into(), "unused".into(), 100);
        channel.provider = "waffo".into();
        channel.webhook_secrets = vec![
            key.to_public_key()
                .to_public_key_pem(Default::default())
                .unwrap(),
        ];
        let response = payment_response("top_42", "acq_fixture", &channel.merchant);
        fixture.reply(response.clone());
        assert_eq!(
            inquiry.confirm(&channel, 42, 100, "acq_fixture").await,
            Ok(())
        );
        for (field, value) in [
            ("merchantOrderId", json!("top_43")),
            ("paymentRequestId", json!("top_43")),
            ("acquiringOrderId", json!("other_payment")),
            ("orderCurrency", json!("CNY")),
            ("orderAmount", json!("0.99")),
            ("orderStatus", json!("PENDING")),
            ("merchantInfo", json!({"merchantId":"another_merchant"})),
        ] {
            let mut changed = response.clone();
            changed["data"][field] = value;
            fixture.reply(changed);
            assert_eq!(
                inquiry.confirm(&channel, 42, 100, "acq_fixture").await,
                Err(Error::Conflict),
                "{field}"
            );
        }
        fixture.raw_reply(
            StatusCode::OK,
            serde_json::to_vec(&response).unwrap(),
            Some("invalid".into()),
        );
        assert_eq!(
            inquiry.confirm(&channel, 42, 100, "acq_fixture").await,
            Err(Error::Denied)
        );
        fixture.raw_reply(StatusCode::OK, serde_json::to_vec(&response).unwrap(), None);
        assert_eq!(
            inquiry.confirm(&channel, 42, 100, "acq_fixture").await,
            Ok(())
        );
        fixture.raw_reply(StatusCode::OK, vec![b' '; MAX_RESPONSE + 1], None);
        assert_eq!(
            inquiry.confirm(&channel, 42, 100, "acq_fixture").await,
            Err(Error::Unavailable)
        );
        fixture.raw_reply(StatusCode::TEMPORARY_REDIRECT, vec![], None);
        assert_eq!(
            inquiry.confirm(&channel, 42, 100, "acq_fixture").await,
            Err(Error::Unavailable)
        );
        fixture.reply(json!({"code":"E0001"}));
        assert_eq!(
            inquiry.confirm(&channel, 42, 100, "acq_fixture").await,
            Err(Error::Unavailable)
        );
        assert_eq!(fixture.calls(), 13);
        assert_eq!(
            Inquiry::new(
                API_KEY.into(),
                key.to_pkcs8_pem(Default::default()).unwrap().to_string()
            )
            .unwrap()
            .endpoint("live")
            .unwrap(),
            LIVE_ENDPOINT
        );
        assert_eq!(
            Inquiry::new(
                API_KEY.into(),
                key.to_pkcs8_pem(Default::default()).unwrap().to_string()
            )
            .unwrap()
            .endpoint("test")
            .unwrap(),
            TEST_ENDPOINT
        );
    }
}

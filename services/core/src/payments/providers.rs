//! Provider bytes are verified inside Core before durable proof admission.
//! Wire contracts follow the former monolith and the pinned official Waffo SDKs.
use std::collections::BTreeMap;

use base64::{Engine, engine::general_purpose::STANDARD};
use hmac::{Hmac, Mac};
use md5::Md5;
use rsa::{Pkcs1v15Sign, RsaPublicKey, pkcs8::DecodePublicKey, traits::PublicKeyParts};
use serde_json::Value;
use sha2::{Digest, Sha256};
use subtle::ConstantTimeEq;

use super::{
    Channel, Error, Result,
    stripe::{self, EventKind, Verified},
    valid_id,
};

fn public_key(value: &str) -> Result<RsaPublicKey> {
    let key = if value.starts_with("-----BEGIN PUBLIC KEY-----") {
        RsaPublicKey::from_public_key_pem(value).map_err(|_| Error::Invalid)?
    } else {
        let der = STANDARD.decode(value).map_err(|_| Error::Invalid)?;
        RsaPublicKey::from_public_key_der(&der).map_err(|_| Error::Invalid)?
    };
    if !(2048..=8192).contains(&key.n().bits()) {
        return Err(Error::Invalid);
    }
    Ok(key)
}

fn https_url(value: &str) -> bool {
    reqwest::Url::parse(value).is_ok_and(|u| {
        u.scheme() == "https"
            && u.host_str().is_some()
            && u.username().is_empty()
            && u.password().is_none()
            && u.fragment().is_none()
    })
}

pub(super) fn valid_channel(c: &Channel) -> bool {
    let v = &c.verification;
    let shared_secret = || {
        c.webhook_secrets.iter().all(|key| {
            (16..=4096).contains(&key.len()) && key.bytes().all(|b| b.is_ascii_graphic())
        })
    };
    let no_checkout = v.payment_type.is_empty()
        && v.endpoint.is_empty()
        && v.notify_url.is_empty()
        && v.return_url.is_empty();
    if c.provider != "waffo" && v.waffo_inquiry.is_some() {
        return false;
    }
    match c.provider.as_str() {
        "stripe" => {
            c.merchant.starts_with("acct_")
                && c.currency == "USD"
                && c.credits_per_minor == 5000
                && no_checkout
                && v.store_id.is_empty()
                && v.product_id.is_empty()
                && c.webhook_secrets.iter().all(|key| {
                    key.starts_with("whsec_")
                        && (16..=256).contains(&key.len())
                        && key.bytes().all(|b| b.is_ascii_graphic())
                })
        }
        "epay" => {
            matches!(c.currency.as_str(), "CNY" | "LDC")
                && valid_id(&v.payment_type)
                && !v.payment_type.eq_ignore_ascii_case("fastpay")
                && v.store_id.is_empty()
                && v.product_id.is_empty()
                && shared_secret()
                && https_url(&v.endpoint)
                && reqwest::Url::parse(&v.endpoint).is_ok_and(|u| u.query().is_none())
                && if c.currency == "LDC" {
                    v.endpoint.trim_end_matches('/') == "https://credit.linux.do/epay"
                        && v.payment_type == "epay"
                        && v.notify_url.len() <= 100
                        && v.return_url.len() <= 100
                } else {
                    reqwest::Url::parse(&v.endpoint)
                        .is_ok_and(|u| u.host_str() != Some("credit.linux.do"))
                }
                && https_url(&v.notify_url)
                && https_url(&v.return_url)
        }
        "creem" => {
            no_checkout
                && v.store_id.is_empty()
                && valid_id(&v.product_id)
                && matches!(c.currency.as_str(), "USD" | "CNY" | "EUR")
                && shared_secret()
        }
        "coinbase" => {
            no_checkout
                && v.store_id.is_empty()
                && v.product_id.is_empty()
                && c.currency == "USD"
                && c.credits_per_minor == 5000
                && c.max_amount_minor <= 10_000_000_000
                && shared_secret()
        }
        "waffo" => {
            no_checkout
                && v.store_id.is_empty()
                && v.product_id.is_empty()
                && c.currency == "USD"
                // A provider-wide public key alone does not establish which
                // account received funds. Both environments require Core's
                // independent inquiry using private account credentials.
                && v.waffo_inquiry.is_some()
                && c.webhook_secrets.iter().all(|key| public_key(key).is_ok())
        }
        "waffo_pancake" => {
            no_checkout
                && valid_id(&v.store_id)
                && valid_id(&v.product_id)
                && matches!(c.currency.as_str(), "USD" | "CNY")
                && c.webhook_secrets.iter().all(|key| public_key(key).is_ok())
        }
        _ => false,
    }
}

fn hex(bytes: &[u8]) -> String {
    const DIGITS: &[u8; 16] = b"0123456789abcdef";
    let mut output = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        output.push(DIGITS[usize::from(byte >> 4)] as char);
        output.push(DIGITS[usize::from(byte & 15)] as char);
    }
    output
}

fn unhex(value: &[u8]) -> Result<Vec<u8>> {
    if !value.len().is_multiple_of(2) {
        return Err(Error::Invalid);
    }
    value
        .chunks_exact(2)
        .map(|pair| {
            let hi = char::from(pair[0]).to_digit(16).ok_or(Error::Invalid)?;
            let lo = char::from(pair[1]).to_digit(16).ok_or(Error::Invalid)?;
            Ok((hi * 16 + lo) as u8)
        })
        .collect()
}

fn stable_key(parts: &[&str]) -> String {
    let mut hash = Sha256::new();
    for part in parts {
        hash.update(format!("{}:", part.len()));
        hash.update(part.as_bytes());
    }
    format!("pay_{}", hex(&hash.finalize()))
}

fn text<'a>(object: &'a Value, field: &str) -> Result<&'a str> {
    object
        .get(field)
        .and_then(Value::as_str)
        .ok_or(Error::Invalid)
}
fn id<'a>(object: &'a Value, field: &str) -> Result<&'a str> {
    let value = text(object, field)?;
    if !valid_id(value) {
        return Err(Error::Invalid);
    }
    Ok(value)
}

fn reference_id(value: &Value) -> Result<&str> {
    let value = value
        .as_str()
        .or_else(|| value.get("id").and_then(Value::as_str))
        .ok_or(Error::Invalid)?;
    if !valid_id(value) {
        return Err(Error::Invalid);
    }
    Ok(value)
}

/// Decimal protocol amounts are parsed as integers, with no float rounding.
pub(super) fn decimal_minor(value: &str) -> Result<i64> {
    if value.is_empty() || value.len() > 20 {
        return Err(Error::Invalid);
    }
    let (whole, fraction) = value.split_once('.').map_or((value, ""), |parts| parts);
    if whole.is_empty()
        || !whole.bytes().all(|b| b.is_ascii_digit())
        || fraction.len() > 2
        || !fraction.bytes().all(|b| b.is_ascii_digit())
        || (value.contains('.') && fraction.is_empty())
    {
        return Err(Error::Invalid);
    }
    let whole: i64 = whole.parse().map_err(|_| Error::Invalid)?;
    let fraction = match fraction.len() {
        0 => 0,
        1 => i64::from(fraction.as_bytes()[0] - b'0') * 10,
        _ => {
            i64::from(fraction.as_bytes()[0] - b'0') * 10 + i64::from(fraction.as_bytes()[1] - b'0')
        }
    };
    let amount = whole
        .checked_mul(100)
        .and_then(|n| n.checked_add(fraction))
        .ok_or(Error::Invalid)?;
    if amount <= 0 {
        return Err(Error::Invalid);
    }
    Ok(amount)
}

fn paid(event_id: String, transaction: String, amount_minor: i64) -> Verified {
    Verified {
        event_id,
        transaction,
        kind: EventKind::Credit,
        amount_minor,
        refund_id: None,
        hold_receipt_id: None,
        dispatch_key: None,
    }
}

pub(super) fn rsa_verify(keys: &[String], bytes: &[u8], signature: &[u8]) -> Result<()> {
    let signature = STANDARD.decode(signature).map_err(|_| Error::Denied)?;
    let digest = Sha256::digest(bytes);
    if keys.iter().any(|key| {
        public_key(key).is_ok_and(|key| {
            key.verify(Pkcs1v15Sign::new::<Sha256>(), &digest, &signature)
                .is_ok()
        })
    }) {
        Ok(())
    } else {
        Err(Error::Denied)
    }
}

pub(super) fn verify(
    payload: &[u8],
    signature: &[u8],
    channel: &Channel,
    intent_id: i64,
    currency: &str,
    amount: i64,
    now: i64,
) -> Result<Verified> {
    if channel.provider == "stripe" {
        return stripe::verify(
            payload, signature, channel, intent_id, currency, amount, now,
        );
    }
    if payload.is_empty()
        || payload.len() > 32 * 1024
        || signature.is_empty()
        || signature.len() > 2048
        || intent_id <= 0
        || amount <= 0
    {
        return Err(Error::Invalid);
    }
    if !channel.validate() {
        return Err(Error::Unavailable);
    }
    if currency != channel.currency || amount > channel.max_amount_minor {
        return Err(Error::Conflict);
    }
    let order = format!("top_{intent_id}");
    match channel.provider.as_str() {
        "epay" => verify_epay(payload, signature, channel, &order, amount),
        "creem" => verify_creem(payload, signature, channel, &order, amount),
        "coinbase" => verify_coinbase(payload, signature, channel, &order, amount, now),
        "waffo" => verify_waffo(payload, signature, channel, &order, amount),
        "waffo_pancake" => verify_pancake(payload, signature, channel, &order, amount, now),
        _ => Err(Error::Unavailable),
    }
}

fn epay_signature(values: &BTreeMap<String, String>, secret: &str) -> String {
    let input = values
        .iter()
        .filter(|(key, value)| {
            key.as_str() != "sign" && key.as_str() != "sign_type" && !value.is_empty()
        })
        .map(|(key, value)| format!("{key}={value}"))
        .collect::<Vec<_>>()
        .join("&")
        + secret;
    hex(&Md5::digest(input.as_bytes()))
}

pub(super) fn epay_checkout(channel: &Channel, order: &str, amount: i64) -> Result<String> {
    if channel.provider != "epay"
        || !channel.validate()
        || !valid_id(order)
        || amount <= 0
        || amount > channel.max_amount_minor
    {
        return Err(Error::Invalid);
    }
    let v = &channel.verification;
    let mut fields = BTreeMap::from([
        ("pid".to_owned(), channel.merchant.clone()),
        ("type".to_owned(), v.payment_type.clone()),
        ("out_trade_no".to_owned(), order.to_owned()),
        ("notify_url".to_owned(), v.notify_url.clone()),
        ("return_url".to_owned(), v.return_url.clone()),
        ("name".to_owned(), "Account top-up".to_owned()),
        (
            "money".to_owned(),
            format!("{}.{:02}", amount / 100, amount % 100),
        ),
        ("sign_type".to_owned(), "MD5".to_owned()),
    ]);
    fields.insert(
        "sign".to_owned(),
        epay_signature(&fields, &channel.webhook_secrets[0]),
    );
    let query = form_urlencoded::Serializer::new(String::new())
        .extend_pairs(&fields)
        .finish();
    let path = if channel.currency == "LDC" {
        "/pay/submit.php"
    } else {
        "/submit.php"
    };
    Ok(format!(
        "{}{path}?{query}",
        v.endpoint.trim_end_matches('/')
    ))
}

fn verify_epay(
    payload: &[u8],
    signature: &[u8],
    c: &Channel,
    order: &str,
    amount: i64,
) -> Result<Verified> {
    std::str::from_utf8(payload).map_err(|_| Error::Invalid)?;
    let mut fields = BTreeMap::new();
    for (key, value) in form_urlencoded::parse(payload) {
        if !valid_id(&key)
            || value.contains(['&', '\r', '\n', '\u{fffd}'])
            || fields
                .insert(key.into_owned(), value.into_owned())
                .is_some()
        {
            return Err(Error::Invalid);
        }
    }
    if fields.len() > 32 {
        return Err(Error::Invalid);
    }
    let field = |name: &str| fields.get(name).map(String::as_str).ok_or(Error::Invalid);
    let signature_type_valid = fields
        .get("sign_type")
        .map_or(c.currency == "LDC", |value| value == "MD5");
    if !signature_type_valid
        || field("pid")? != c.merchant
        || field("type")? != c.verification.payment_type
        || field("out_trade_no")? != order
        || field("trade_status")? != "TRADE_SUCCESS"
    {
        return Err(Error::Denied);
    }
    let received = unhex(signature)?;
    if unhex(field("sign")?.as_bytes())? != received || received.len() != 16 {
        return Err(Error::Denied);
    }
    let signed = c.webhook_secrets.iter().any(|key| {
        unhex(epay_signature(&fields, key).as_bytes())
            .is_ok_and(|expected| bool::from(expected.ct_eq(&received)))
    });
    if !signed {
        return Err(Error::Denied);
    }
    if decimal_minor(field("money")?)? != amount {
        return Err(Error::Conflict);
    }
    let transaction = field("trade_no")?;
    if !valid_id(transaction) {
        return Err(Error::Invalid);
    }
    Ok(paid(
        stable_key(&[transaction, "TRADE_SUCCESS"]),
        transaction.to_owned(),
        amount,
    ))
}

fn verify_creem(
    payload: &[u8],
    signature: &[u8],
    c: &Channel,
    order: &str,
    amount: i64,
) -> Result<Verified> {
    let signature = unhex(signature)?;
    let valid = c.webhook_secrets.iter().any(|secret| {
        let Ok(mut mac) = Hmac::<Sha256>::new_from_slice(secret.as_bytes()) else {
            return false;
        };
        mac.update(payload);
        mac.verify_slice(&signature).is_ok()
    });
    if !valid {
        return Err(Error::Denied);
    }
    let event = stripe::unique_json(payload)?;
    let object = event.get("object").ok_or(Error::Invalid)?;
    let payment = object.get("order").ok_or(Error::Invalid)?;
    let mode = if c.environment == "live" {
        "prod"
    } else {
        "test"
    };
    if text(&event, "eventType")? != "checkout.completed"
        || text(object, "object")? != "checkout"
        || text(object, "status")? != "completed"
        || text(object, "request_id")? != order
        || text(object, "mode")? != mode
        || text(payment, "status")? != "paid"
        || text(payment, "mode")? != mode
        || payment
            .get("type")
            .is_some_and(|v| v.as_str() != Some("onetime"))
        || text(payment, "currency")?.to_uppercase() != c.currency
        || payment.get("amount_paid").and_then(Value::as_i64) != Some(amount)
        || reference_id(object.get("product").ok_or(Error::Invalid)?)? != c.verification.product_id
        || payment
            .get("product")
            .is_some_and(|v| reference_id(v).ok() != Some(&c.verification.product_id))
        || object
            .get("metadata")
            .and_then(|v| v.get("lmm_merchant"))
            .and_then(Value::as_str)
            != Some(&c.merchant)
    {
        return Err(Error::Conflict);
    }
    for nested in [payment, object.get("product").unwrap_or(&Value::Null)] {
        if nested.get("mode").is_some_and(|v| v.as_str() != Some(mode)) {
            return Err(Error::Conflict);
        }
    }
    Ok(paid(
        id(&event, "id")?.to_owned(),
        id(payment, "transaction")?.to_owned(),
        amount,
    ))
}

// Coinbase's recommended v0 authenticates the raw body and timestamp; v1 also
// includes HTTP headers, which are not payment authority in this integration.
// https://docs.cdp.coinbase.com/webhooks/verify-signatures
fn verify_coinbase(
    payload: &[u8],
    signature: &[u8],
    c: &Channel,
    order: &str,
    amount: i64,
    now: i64,
) -> Result<Verified> {
    let header = std::str::from_utf8(signature).map_err(|_| Error::Invalid)?;
    let mut fields = BTreeMap::new();
    for field in header.split(',') {
        let (key, value) = field.trim().split_once('=').ok_or(Error::Invalid)?;
        if !matches!(key, "t" | "v0" | "h" | "v1") || fields.insert(key, value).is_some() {
            return Err(Error::Invalid);
        }
    }
    let timestamp = *fields.get("t").ok_or(Error::Invalid)?;
    if timestamp.is_empty() || !timestamp.bytes().all(|b| b.is_ascii_digit()) {
        return Err(Error::Invalid);
    }
    let seconds: i64 = timestamp.parse().map_err(|_| Error::Invalid)?;
    if seconds <= 0 || seconds.abs_diff(now) > 300 {
        return Err(Error::Denied);
    }
    let signature = unhex(fields.get("v0").ok_or(Error::Invalid)?.as_bytes())?;
    let valid = c.webhook_secrets.iter().any(|secret| {
        let Ok(mut mac) = Hmac::<Sha256>::new_from_slice(secret.as_bytes()) else {
            return false;
        };
        mac.update(timestamp.as_bytes());
        mac.update(b".");
        mac.update(payload);
        mac.verify_slice(&signature).is_ok()
    });
    if !valid {
        return Err(Error::Denied);
    }
    let event = stripe::unique_json(payload)?;
    let metadata = event.get("metadata").ok_or(Error::Invalid)?;
    let event_type = "checkout.payment.success";
    if text(&event, "eventType")? != event_type
        || text(&event, "status")? != "COMPLETED"
        || text(&event, "currency")? != "USDC"
        || decimal_minor(text(&event, "amount")?)? != amount
        || text(metadata, "lmm_order_id")? != order
        || text(metadata, "lmm_core_intent_id")? != order.trim_start_matches("top_")
        || text(metadata, "lmm_merchant")? != c.merchant
        || text(metadata, "lmm_environment")? != c.environment
    {
        return Err(Error::Conflict);
    }
    let transaction = id(&event, "id")?;
    Ok(paid(
        stable_key(&[transaction, event_type]),
        transaction.to_owned(),
        amount,
    ))
}

fn verify_waffo(
    payload: &[u8],
    signature: &[u8],
    c: &Channel,
    order: &str,
    amount: i64,
) -> Result<Verified> {
    rsa_verify(&c.webhook_secrets, payload, signature)?;
    let event = stripe::unique_json(payload)?;
    let result = event.get("result").ok_or(Error::Invalid)?;
    if text(&event, "eventType")? != "PAYMENT_NOTIFICATION"
        || text(result, "orderStatus")? != "PAY_SUCCESS"
        || text(result, "merchantOrderId")? != order
        || text(result, "paymentRequestId")? != order
        || text(result, "orderCurrency")? != c.currency
        || decimal_minor(text(result, "orderAmount")?)? != amount
        || result
            .get("merchantInfo")
            .and_then(|v| v.get("merchantId"))
            .and_then(Value::as_str)
            != Some(&c.merchant)
    {
        return Err(Error::Conflict);
    }
    let transaction = id(result, "acquiringOrderId")?;
    Ok(paid(
        stable_key(&[transaction, "PAYMENT_NOTIFICATION", "PAY_SUCCESS"]),
        transaction.to_owned(),
        amount,
    ))
}

fn verify_pancake(
    payload: &[u8],
    signature: &[u8],
    c: &Channel,
    order: &str,
    amount: i64,
    now: i64,
) -> Result<Verified> {
    let header = std::str::from_utf8(signature).map_err(|_| Error::Invalid)?;
    let mut fields = BTreeMap::new();
    for field in header.split(',') {
        let (key, value) = field.trim().split_once('=').ok_or(Error::Invalid)?;
        if !matches!(key, "t" | "v1") || fields.insert(key, value).is_some() {
            return Err(Error::Invalid);
        }
    }
    let timestamp = *fields.get("t").ok_or(Error::Invalid)?;
    let timestamp_number: i64 = timestamp.parse().map_err(|_| Error::Invalid)?;
    let age = now
        .checked_mul(1000)
        .and_then(|n| n.checked_sub(timestamp_number))
        .ok_or(Error::Invalid)?;
    if !(-60_000..=45 * 60 * 1000).contains(&age) {
        return Err(Error::Denied);
    }
    let mut signed = Vec::with_capacity(timestamp.len() + 1 + payload.len());
    signed.extend_from_slice(timestamp.as_bytes());
    signed.push(b'.');
    signed.extend_from_slice(payload);
    rsa_verify(
        &c.webhook_secrets,
        &signed,
        fields.get("v1").ok_or(Error::Invalid)?.as_bytes(),
    )?;
    let event = stripe::unique_json(payload)?;
    let data = event.get("data").ok_or(Error::Invalid)?;
    let mode = if c.environment == "live" {
        "prod"
    } else {
        "test"
    };
    if data.get("listPrice").is_some_and(|v| !v.is_null())
        && data.get("chargedAmount").is_none_or(Value::is_null)
    {
        return Err(Error::Invalid);
    }
    let charged = data
        .get("chargedAmount")
        .filter(|v| !v.is_null())
        .map_or_else(
            || text(data, "amount"),
            |v| v.as_str().ok_or(Error::Invalid),
        )?;
    if text(&event, "eventType")? != "order.completed"
        || text(&event, "mode")? != mode
        || text(&event, "storeId")? != c.verification.store_id
        || text(data, "orderMerchantExternalId")? != order
        || text(data, "currency")? != c.currency
        || decimal_minor(charged)? != amount
        || data
            .get("amount")
            .filter(|v| !v.is_null())
            .is_some_and(|v| v.as_str().and_then(|s| decimal_minor(s).ok()) != Some(amount))
        || data
            .get("orderStatus")
            .filter(|v| !v.is_null())
            .is_some_and(|v| v.as_str() != Some("completed"))
        || data
            .get("paymentStatus")
            .filter(|v| !v.is_null())
            .is_some_and(|v| v.as_str() != Some("succeeded"))
        || data
            .get("orderMetadata")
            .and_then(|v| v.get("lmm_product_id"))
            .and_then(Value::as_str)
            != Some(&c.verification.product_id)
        || data
            .get("orderMetadata")
            .and_then(|v| v.get("lmm_merchant"))
            .and_then(Value::as_str)
            != Some(&c.merchant)
    {
        return Err(Error::Conflict);
    }
    Ok(paid(
        id(&event, "id")?.to_owned(),
        id(data, "paymentId")?.to_owned(),
        amount,
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::payments::{PaymentsStore, ProviderVerification};
    use rand::{SeedableRng, rngs::StdRng};
    use rsa::{
        RsaPrivateKey,
        pkcs8::{EncodePrivateKey, EncodePublicKey},
    };
    use serde_json::json;
    use std::sync::OnceLock;

    const NOW: i64 = 1_900_000_000;
    const SECRET: &str = "core_only_epay_secret_for_fixture";

    fn rsa_key() -> &'static RsaPrivateKey {
        static KEY: OnceLock<RsaPrivateKey> = OnceLock::new();
        KEY.get_or_init(|| RsaPrivateKey::new(&mut StdRng::seed_from_u64(42), 2048).unwrap())
    }

    fn channel(provider: &str) -> Channel {
        let mut c = Channel {
            id: 7,
            provider: provider.into(),
            merchant: "merchant_fixture".into(),
            environment: "test".into(),
            currency: "USD".into(),
            webhook_secrets: vec![SECRET.into()],
            credits_per_minor: 5000,
            max_amount_minor: 10000,
            verification: ProviderVerification::default(),
        };
        match provider {
            "epay" => {
                c.merchant = "123".into();
                c.currency = "CNY".into();
                c.verification.payment_type = "alipay".into();
                c.verification.endpoint = "https://pay.example.test".into();
                c.verification.notify_url = "https://lmm.example.test/callbacks/7".into();
                c.verification.return_url = "https://lmm.example.test/wallet".into();
            }
            "creem" => {
                c.verification.product_id = "prod_fixture".into();
            }
            "waffo" | "waffo_pancake" => {
                c.webhook_secrets = vec![
                    rsa_key()
                        .to_public_key()
                        .to_public_key_pem(Default::default())
                        .unwrap(),
                ];
                if provider == "waffo_pancake" {
                    c.verification.store_id = "STO_fixture".into();
                    c.verification.product_id = "PROD_fixture".into();
                } else {
                    c.verification.waffo_inquiry = Some(std::sync::Arc::new(
                        crate::payments::waffo::Inquiry::new(
                            "core_only_inquiry_fixture".into(),
                            rsa_key()
                                .to_pkcs8_pem(Default::default())
                                .unwrap()
                                .to_string(),
                        )
                        .unwrap(),
                    ));
                }
            }
            _ => {}
        }
        c
    }

    fn signed_rsa(bytes: &[u8]) -> String {
        STANDARD.encode(
            rsa_key()
                .sign(Pkcs1v15Sign::new::<Sha256>(), &Sha256::digest(bytes))
                .unwrap(),
        )
    }

    fn creem_event() -> Value {
        json!({"id":"evt_creem_fixture", "eventType":"checkout.completed", "object":{
            "object":"checkout", "status":"completed", "request_id":"top_42", "mode":"test", "product":{"id":"prod_fixture"}, "metadata":{"lmm_merchant":"merchant_fixture"},
            "order":{"status":"paid", "type":"onetime", "mode":"test", "product":"prod_fixture",
                "currency":"USD", "amount_paid":100, "transaction":"tran_fixture"}}})
    }
    fn signed_creem(value: &Value) -> (Vec<u8>, String) {
        let bytes = serde_json::to_vec(value).unwrap();
        let mut mac = Hmac::<Sha256>::new_from_slice(SECRET.as_bytes()).unwrap();
        mac.update(&bytes);
        let signature = hex(&mac.finalize().into_bytes());
        (bytes, signature)
    }

    fn coinbase_event() -> Value {
        json!({"id":"69163c762331ed43dc64a6ef", "eventType":"checkout.payment.success",
            "status":"COMPLETED", "amount":"1.00", "currency":"USDC",
            "metadata":{"lmm_order_id":"top_42", "lmm_core_intent_id":"42",
                "lmm_merchant":"merchant_fixture", "lmm_environment":"test"}})
    }

    fn signed_coinbase(raw: &[u8], timestamp: i64) -> String {
        let mut mac = Hmac::<Sha256>::new_from_slice(SECRET.as_bytes()).unwrap();
        mac.update(format!("{timestamp}.").as_bytes());
        mac.update(raw);
        format!("t={timestamp},v0={}", hex(&mac.finalize().into_bytes()))
    }

    #[test]
    fn coinbase_accepts_independent_golden_signature_and_binds_signed_fields() {
        let raw = br#"{"id":"69163c762331ed43dc64a6ef","eventType":"checkout.payment.success","status":"COMPLETED","amount":"1.00","currency":"USDC","metadata":{"lmm_order_id":"top_42","lmm_core_intent_id":"42","lmm_merchant":"merchant_fixture","lmm_environment":"test"}}"#;
        // Produced independently with Python hmac/hashlib, not the verifier.
        let signature = "t=1900000000,v0=4c5c5bf802d9c05b618ea3c9a0940b9921219f1fe4437a6f5103b9b7183b6b23,h=content-type x-event-id,v1=unused";
        let c = channel("coinbase");
        let proof = verify(raw, signature.as_bytes(), &c, 42, "USD", 100, NOW).unwrap();
        assert_eq!(proof.transaction, "69163c762331ed43dc64a6ef");
        assert_eq!(
            proof.event_id,
            stable_key(&["69163c762331ed43dc64a6ef", "checkout.payment.success"])
        );
        for (field, value) in [
            ("amount", "0.99"),
            ("amount", "1.001"),
            ("currency", "USD"),
            ("status", "ACTIVE"),
            ("eventType", "checkout.payment.failed"),
            ("eventType", "checkout.refund.success"),
        ] {
            let mut event = coinbase_event();
            event[field] = json!(value);
            let bytes = serde_json::to_vec(&event).unwrap();
            assert!(
                verify(
                    &bytes,
                    signed_coinbase(&bytes, NOW).as_bytes(),
                    &c,
                    42,
                    "USD",
                    100,
                    NOW
                )
                .is_err()
            );
        }
        for field in [
            "lmm_order_id",
            "lmm_core_intent_id",
            "lmm_merchant",
            "lmm_environment",
        ] {
            let mut event = coinbase_event();
            event["metadata"][field] = json!("wrong");
            let bytes = serde_json::to_vec(&event).unwrap();
            assert!(
                verify(
                    &bytes,
                    signed_coinbase(&bytes, NOW).as_bytes(),
                    &c,
                    42,
                    "USD",
                    100,
                    NOW
                )
                .is_err()
            );
        }
        let mut changed = raw.to_vec();
        changed.push(b' ');
        assert!(verify(&changed, signature.as_bytes(), &c, 42, "USD", 100, NOW).is_err());
        let duplicate = String::from_utf8(raw.to_vec())
            .unwrap()
            .replacen("{", "{\"amount\":\"1.00\",", 1)
            .into_bytes();
        assert!(
            verify(
                &duplicate,
                signed_coinbase(&duplicate, NOW).as_bytes(),
                &c,
                42,
                "USD",
                100,
                NOW
            )
            .is_err()
        );
    }

    #[test]
    fn coinbase_rejects_missing_v0_duplicate_fields_and_expired_or_future_timestamps() {
        let c = channel("coinbase");
        let bytes = serde_json::to_vec(&coinbase_event()).unwrap();
        for delta in [-301, 301] {
            assert!(
                verify(
                    &bytes,
                    signed_coinbase(&bytes, NOW + delta).as_bytes(),
                    &c,
                    42,
                    "USD",
                    100,
                    NOW
                )
                .is_err()
            );
        }
        for delta in [-300, 300] {
            assert!(
                verify(
                    &bytes,
                    signed_coinbase(&bytes, NOW + delta).as_bytes(),
                    &c,
                    42,
                    "USD",
                    100,
                    NOW
                )
                .is_ok()
            );
        }
        let good = signed_coinbase(&bytes, NOW);
        for bad in [
            format!("{good},t={NOW}"),
            format!("{good},v0=00"),
            good.replace("v0=", "v1="),
            good.replace("t=1900000000", "t=1900000000junk"),
        ] {
            assert!(verify(&bytes, bad.as_bytes(), &c, 42, "USD", 100, NOW).is_err());
        }
    }

    #[tokio::test]
    async fn coinbase_requires_dedicated_environment_secret_and_exact_usd_policy() {
        let pool = sqlx::postgres::PgPoolOptions::new()
            .connect_lazy("postgresql://synthetic:synthetic@127.0.0.1:1/synthetic")
            .unwrap();
        let a = channel("coinbase");
        let mut b = a.clone();
        b.id = 8;
        b.environment = "live".into();
        assert!(PaymentsStore::new(pool.clone(), vec![a.clone(), b.clone()]).is_err());
        b.webhook_secrets = vec!["independent_live_coinbase_secret".into()];
        assert!(PaymentsStore::new(pool, vec![a.clone(), b]).is_ok());
        for case in 0..3 {
            let mut invalid = a.clone();
            match case {
                0 => invalid.currency = "CNY".into(),
                1 => invalid.credits_per_minor = 1,
                _ => invalid.max_amount_minor = 10_000_000_001,
            }
            assert!(!invalid.validate());
        }
    }

    #[test]
    fn epay_accepts_legacy_golden_signature_and_rejects_field_changes() {
        // Golden MD5 produced independently with Python hashlib for the former Go protocol.
        let signature = "89203adb25927f2c37b912f9a81ce712";
        let raw = format!(
            "pid=123&type=alipay&out_trade_no=top_42&trade_no=trade_fixture&money=1.00&trade_status=TRADE_SUCCESS&sign_type=MD5&sign={signature}"
        );
        let c = channel("epay");
        let proof = verify(
            raw.as_bytes(),
            signature.as_bytes(),
            &c,
            42,
            "CNY",
            100,
            NOW,
        )
        .unwrap();
        assert_eq!(proof.transaction, "trade_fixture");
        assert_eq!(
            proof.event_id,
            stable_key(&["trade_fixture", "TRADE_SUCCESS"])
        );
        for changed in [
            raw.replace("money=1.00", "money=1.01"),
            raw.replace("pid=123", "pid=456"),
            format!("{raw}&money=1.00"),
            raw.replace("top_42", "top_43"),
        ] {
            assert!(
                verify(
                    changed.as_bytes(),
                    signature.as_bytes(),
                    &c,
                    42,
                    "CNY",
                    100,
                    NOW
                )
                .is_err()
            );
        }
        let mut ldc = c.clone();
        ldc.currency = "LDC".into();
        assert!(!ldc.validate());
        ldc.verification.endpoint = "https://credit.linux.do/epay".into();
        ldc.verification.payment_type = "epay".into();
        let ldc_signature = "e4aceb080967f2a788b8cc31ff3e6231";
        let ldc_raw = raw
            .replace("type=alipay", "type=epay")
            .replace(signature, ldc_signature)
            .replace("&sign_type=MD5", "");
        assert!(
            verify(
                ldc_raw.as_bytes(),
                ldc_signature.as_bytes(),
                &ldc,
                42,
                "LDC",
                100,
                NOW
            )
            .is_ok()
        );
        assert!(
            verify(
                format!("{ldc_raw}&sign_type=SHA256").as_bytes(),
                ldc_signature.as_bytes(),
                &ldc,
                42,
                "LDC",
                100,
                NOW
            )
            .is_err()
        );
        ldc.currency = "CNY".into();
        assert!(!ldc.validate());
    }

    #[test]
    fn epay_checkout_signs_only_core_owned_fields() {
        let c = channel("epay");
        let url = epay_checkout(&c, "top_42", 100).unwrap();
        let parsed = reqwest::Url::parse(&url).unwrap();
        let query: BTreeMap<_, _> = parsed.query_pairs().into_owned().collect();
        assert_eq!(parsed.path(), "/submit.php");
        assert_eq!(query.get("money").unwrap(), "1.00");
        assert_eq!(query.get("out_trade_no").unwrap(), "top_42");
        assert_eq!(query.get("pid").unwrap(), "123");
        assert_eq!(query.get("notify_url").unwrap(), &c.verification.notify_url);
        assert!(!query.contains_key("trade_status"));
        assert!(!url.contains(SECRET));
        let mut ldc = c;
        ldc.currency = "LDC".into();
        ldc.verification.payment_type = "epay".into();
        ldc.verification.endpoint = "https://credit.linux.do/epay".into();
        let url = reqwest::Url::parse(&epay_checkout(&ldc, "top_42", 100).unwrap()).unwrap();
        assert_eq!(url.path(), "/epay/pay/submit.php");
        assert_eq!(
            url.query_pairs().find(|(key, _)| key == "type").unwrap().1,
            "epay"
        );
        ldc.verification.return_url = format!("https://lmm.example.test/{}", "x".repeat(100));
        assert!(!ldc.validate());
    }

    #[test]
    fn creem_requires_signature_paid_amount_product_and_mode() {
        let c = channel("creem");
        let mut unsupported_currency = c.clone();
        unsupported_currency.currency = "JPY".into();
        assert!(!unsupported_currency.validate());
        let original = creem_event();
        let (raw, signature) = signed_creem(&original);
        assert!(verify(&raw, signature.as_bytes(), &c, 42, "USD", 100, NOW).is_ok());
        assert!(verify(&raw, b"00", &c, 42, "USD", 100, NOW).is_err());
        for path in ["amount_paid", "product", "status"] {
            let mut changed = original.clone();
            changed["object"]["order"][path] = if path == "amount_paid" {
                json!(99)
            } else {
                json!("wrong")
            };
            let (raw, signature) = signed_creem(&changed);
            assert!(verify(&raw, signature.as_bytes(), &c, 42, "USD", 100, NOW).is_err());
        }
        let mut live = c;
        live.environment = "live".into();
        assert!(verify(&raw, signature.as_bytes(), &live, 42, "USD", 100, NOW).is_err());
    }

    #[test]
    fn waffo_rsa_proof_binds_merchant_order_currency_and_amount() {
        let c = channel("waffo");
        let event = json!({"eventType":"PAYMENT_NOTIFICATION", "result":{
            "merchantOrderId":"top_42", "paymentRequestId":"top_42", "acquiringOrderId":"acq_fixture",
            "orderStatus":"PAY_SUCCESS", "orderCurrency":"USD", "orderAmount":"1.00",
            "merchantInfo":{"merchantId":"merchant_fixture"}}});
        let raw = serde_json::to_vec(&event).unwrap();
        let signature = signed_rsa(&raw);
        assert!(verify(&raw, signature.as_bytes(), &c, 42, "USD", 100, NOW).is_ok());
        let mut changed = event;
        changed["result"]["merchantInfo"]["merchantId"] = json!("different_merchant");
        let changed = serde_json::to_vec(&changed).unwrap();
        assert!(
            verify(
                &changed,
                signed_rsa(&changed).as_bytes(),
                &c,
                42,
                "USD",
                100,
                NOW
            )
            .is_err()
        );
        assert!(verify(&raw, signature.as_bytes(), &c, 43, "USD", 100, NOW).is_err());
        assert!(verify(&raw, signature.as_bytes(), &c, 42, "USD", 101, NOW).is_err());
    }

    fn pancake_event() -> Value {
        json!({"id":"delivery_fixture", "eventId":"payment_fixture", "eventType":"order.completed",
            "mode":"test", "storeId":"STO_fixture", "data":{
                "orderMerchantExternalId":"top_42", "paymentId":"PAY_fixture", "orderStatus":"completed",
                "paymentStatus":"succeeded", "chargedAmount":"1.00", "amount":"1.00", "currency":"USD",
                "orderMetadata":{"lmm_product_id":"PROD_fixture", "lmm_merchant":"merchant_fixture"}}})
    }
    fn signed_pancake(raw: &[u8], timestamp: i64) -> String {
        let mut signed = format!("{timestamp}.").into_bytes();
        signed.extend_from_slice(raw);
        format!("t={timestamp},v1={}", signed_rsa(&signed))
    }

    #[test]
    fn pancake_respects_provider_retry_window_and_signed_store() {
        let c = channel("waffo_pancake");
        let raw = serde_json::to_vec(&pancake_event()).unwrap();
        for (delta, accepted) in [
            (0, true),
            (-31 * 60 * 1000, true),
            (-46 * 60 * 1000, false),
            (120_000, false),
        ] {
            let signature = signed_pancake(&raw, NOW * 1000 + delta);
            assert_eq!(
                verify(&raw, signature.as_bytes(), &c, 42, "USD", 100, NOW).is_ok(),
                accepted
            );
        }
        let mut changed = pancake_event();
        changed["storeId"] = json!("other_store");
        let changed = serde_json::to_vec(&changed).unwrap();
        let signature = signed_pancake(&changed, NOW * 1000);
        assert!(verify(&changed, signature.as_bytes(), &c, 42, "USD", 100, NOW).is_err());
        let mut changed = pancake_event();
        changed["data"]["chargedAmount"] = json!("0.01");
        let changed = serde_json::to_vec(&changed).unwrap();
        assert!(
            verify(
                &changed,
                signed_pancake(&changed, NOW * 1000).as_bytes(),
                &c,
                42,
                "USD",
                100,
                NOW
            )
            .is_err()
        );
    }

    #[test]
    fn fastpay_is_not_a_provider_or_epay_method() {
        assert!(!channel("fastpay").validate());
        let mut c = channel("epay");
        c.verification.payment_type = "FastPay".into();
        assert!(!c.validate());
    }

    #[test]
    fn waffo_both_environments_require_independent_account_binding() {
        let mut c = channel("waffo");
        assert!(c.validate());
        c.environment = "live".into();
        assert!(c.validate());
        c.verification.waffo_inquiry = None;
        assert!(!c.validate());
        c.environment = "test".into();
        assert!(!c.validate());
    }

    #[tokio::test]
    async fn products_share_one_merchant_secret_but_not_cross_environment() {
        let pool = sqlx::postgres::PgPoolOptions::new()
            .connect_lazy("postgresql://synthetic:synthetic@127.0.0.1:1/synthetic")
            .unwrap();
        let a = channel("creem");
        let mut b = a.clone();
        b.id = 8;
        b.verification.product_id = "prod_second".into();
        assert!(PaymentsStore::new(pool.clone(), vec![a.clone(), b.clone()]).is_ok());
        b.environment = "live".into();
        assert!(PaymentsStore::new(pool, vec![a, b]).is_err());
    }

    #[sqlx::test]
    #[ignore = "requires an explicitly isolated PostgreSQL DATABASE_URL"]
    async fn postgres_gateway_proof_credit_and_personal_payer(pool: sqlx::PgPool) {
        use crate::{identity::IdentityStore, internal_rpc::pb};
        use std::time::{SystemTime, UNIX_EPOCH};
        crate::core_schema::init(&pool).await.unwrap();
        let identity = IdentityStore::from_pool(pool.clone());
        let session = identity.bootstrap_user(7301, 0).await.unwrap();
        let admin = identity.bootstrap_user(7302, 5).await.unwrap();
        let payer = identity
            .authorize(&session.secret)
            .await
            .unwrap()
            .owner_account_id;
        let mutation = |key: String| {
            Some(pb::MutationContext {
                idempotency_key: key,
                expected_version: None,
            })
        };
        for (index, profile) in [
            "epay",
            "epay_ldc",
            "creem",
            "waffo",
            "waffo_pancake",
            "coinbase",
        ]
        .into_iter()
        .enumerate()
        {
            let provider = if profile == "epay_ldc" {
                "epay"
            } else {
                profile
            };
            let mut c = channel(provider);
            c.id = index as i64 + 1;
            if profile == "epay_ldc" {
                c.currency = "LDC".into();
                c.verification.endpoint = "https://credit.linux.do/epay".into();
                c.verification.payment_type = "epay".into();
            }
            let inquiry = if provider == "waffo" {
                let fixture =
                    crate::payments::waffo::tests::Fixture::start(rsa_key().clone()).await;
                c.verification.waffo_inquiry = Some(std::sync::Arc::new(fixture.inquiry()));
                Some(fixture)
            } else {
                None
            };
            let store = PaymentsStore::new(pool.clone(), vec![c.clone()]).unwrap();
            let intent = store
                .prepare_personal(
                    &session.secret,
                    pb::PreparePersonalTopupRequest {
                        mutation: mutation(format!("gateway_personal_quote_{profile}")),
                        payment_channel_id: c.id,
                        currency: c.currency.clone(),
                        amount_minor: 100,
                    },
                )
                .await
                .unwrap();
            assert_eq!(intent.account_id, payer);
            assert_eq!(intent.credit_units, 500_000);
            assert_eq!(
                store
                    .prepare_personal(
                        &session.secret,
                        pb::PreparePersonalTopupRequest {
                            mutation: mutation(format!("gateway_personal_quote_{profile}")),
                            payment_channel_id: c.id,
                            currency: c.currency.clone(),
                            amount_minor: 101,
                        }
                    )
                    .await,
                Err(Error::Conflict),
            );
            let order = format!("top_{}", intent.intent_id);
            let now = i64::try_from(
                SystemTime::now()
                    .duration_since(UNIX_EPOCH)
                    .unwrap()
                    .as_secs(),
            )
            .unwrap();
            let (raw, signature) = match provider {
                "epay" => {
                    let checkout = store.epay_checkout(intent.intent_id).await.unwrap();
                    assert_eq!(
                        checkout,
                        store.epay_checkout(intent.intent_id).await.unwrap()
                    );
                    assert_eq!(checkout.order_id, order);
                    assert!(!checkout.checkout_url.contains(SECRET));
                    assert_eq!(
                        checkout.method,
                        if profile == "epay_ldc" { "POST" } else { "GET" }
                    );
                    let mut values = BTreeMap::from([
                        ("pid".into(), c.merchant.clone()),
                        ("type".into(), c.verification.payment_type.clone()),
                        ("out_trade_no".into(), order.clone()),
                        ("trade_no".into(), format!("trade_pg_{profile}")),
                        ("money".into(), "1.00".into()),
                        ("trade_status".into(), "TRADE_SUCCESS".into()),
                        ("sign_type".into(), "MD5".into()),
                    ]);
                    if profile == "epay_ldc" {
                        values.remove("sign_type");
                    }
                    let signature = epay_signature(&values, SECRET);
                    values.insert("sign".into(), signature.clone());
                    (
                        form_urlencoded::Serializer::new(String::new())
                            .extend_pairs(values)
                            .finish()
                            .into_bytes(),
                        signature,
                    )
                }
                "creem" => {
                    let mut event = creem_event();
                    event["object"]["request_id"] = json!(order);
                    signed_creem(&event)
                }
                "coinbase" => {
                    let mut event = coinbase_event();
                    event["metadata"]["lmm_order_id"] = json!(order);
                    event["metadata"]["lmm_core_intent_id"] = json!(intent.intent_id.to_string());
                    let raw = serde_json::to_vec(&event).unwrap();
                    let signature = signed_coinbase(&raw, now);
                    (raw, signature)
                }
                "waffo" => {
                    let raw = serde_json::to_vec(&json!({"eventType":"PAYMENT_NOTIFICATION", "result":{
                        "merchantOrderId":order, "paymentRequestId":order, "acquiringOrderId":"acq_pg",
                        "orderStatus":"PAY_SUCCESS", "orderCurrency":"USD", "orderAmount":"1.00",
                        "merchantInfo":{"merchantId":c.merchant}}})).unwrap();
                    let signature = signed_rsa(&raw);
                    (raw, signature)
                }
                "waffo_pancake" => {
                    let mut event = pancake_event();
                    event["data"]["orderMerchantExternalId"] = json!(order);
                    let raw = serde_json::to_vec(&event).unwrap();
                    let signature = signed_pancake(&raw, now * 1000);
                    (raw, signature)
                }
                _ => unreachable!(),
            };
            let facts = verify(
                &raw,
                signature.as_bytes(),
                &c,
                intent.intent_id,
                &c.currency,
                100,
                now,
            )
            .unwrap();
            let mut evidence = pb::ProviderEvidence {
                payment: Some(pb::ExternalPaymentReference {
                    provider: provider.into(),
                    merchant: c.merchant.clone(),
                    environment: "test".into(),
                    transaction: facts.transaction,
                }),
                signed_payload: raw,
                signature: signature.into_bytes(),
                provider_event_id: facts.event_id,
                accepted_proof_id: String::new(),
            };
            let accept = pb::AcceptProviderProofRequest {
                intent_id: intent.intent_id,
                evidence: Some(evidence.clone()),
            };
            if let Some(inquiry) = &inquiry {
                inquiry.reply(crate::payments::waffo::tests::payment_response(
                    &order,
                    "acq_pg",
                    "different_merchant",
                ));
                assert_eq!(
                    store.accept_proof(accept.clone()).await,
                    Err(Error::Conflict)
                );
                let bound: Option<String> = sqlx::query_scalar(
                    "SELECT provider_transaction FROM core_payments.intents WHERE id=$1",
                )
                .bind(intent.intent_id)
                .fetch_one(&pool)
                .await
                .unwrap();
                assert!(
                    bound.is_none(),
                    "an inquiry mismatch must not bind or credit the intent"
                );
                inquiry.reply(crate::payments::waffo::tests::payment_response(
                    &order,
                    "acq_pg",
                    &c.merchant,
                ));
            }
            let proof = store.accept_proof(accept.clone()).await.unwrap();
            if let Some(inquiry) = &inquiry {
                assert_eq!(inquiry.calls(), 2);
                inquiry.reply(json!({"code":"E0001"}));
            }
            assert_eq!(store.accept_proof(accept).await.unwrap(), proof);
            if let Some(inquiry) = &inquiry {
                assert_eq!(
                    inquiry.calls(),
                    2,
                    "durable proof replay must not query the provider again"
                );
            }
            let before: i64 = sqlx::query_scalar(
                "SELECT count(*) FROM core_billing.ledger_journals WHERE kind='credit'",
            )
            .fetch_one(&pool)
            .await
            .unwrap();
            assert_eq!(
                before, index as i64,
                "proof admission must not create credit"
            );
            evidence.accepted_proof_id = proof.proof_id;
            let request = pb::CreditTopupRequest {
                mutation: mutation(format!("gateway_credit_{profile}_01")),
                intent_id: intent.intent_id,
                evidence: Some(evidence.clone()),
            };
            let credited = store.credit(request.clone()).await.unwrap();
            assert_eq!(credited.state, 1);
            assert_eq!(store.credit(request).await.unwrap(), credited);
            let other = store
                .prepare_personal(
                    &session.secret,
                    pb::PreparePersonalTopupRequest {
                        mutation: mutation(format!("gateway_other_quote_{profile}")),
                        payment_channel_id: c.id,
                        currency: c.currency.clone(),
                        amount_minor: 100,
                    },
                )
                .await
                .unwrap();
            assert!(
                store
                    .credit(pb::CreditTopupRequest {
                        mutation: mutation(format!("gateway_reuse_{profile}_01")),
                        intent_id: other.intent_id,
                        evidence: Some(evidence)
                    })
                    .await
                    .is_err()
            );
        }
        let journals: i64 = sqlx::query_scalar(
            "SELECT count(*) FROM core_billing.ledger_journals WHERE kind='credit'",
        )
        .fetch_one(&pool)
        .await
        .unwrap();
        assert_eq!(journals, 5);
        let balance: i64 = sqlx::query_scalar("SELECT balance_units FROM core_billing.balance_state b JOIN core_billing.ledger_accounts a ON a.id=b.ledger_account_id WHERE a.owner_account_id=$1 AND a.bucket='wallet'")
            .bind(payer).fetch_one(&pool).await.unwrap();
        assert_eq!(balance, 2_500_000);
        let unbalanced: i64 = sqlx::query_scalar("SELECT count(*) FROM (SELECT journal_id FROM core_billing.ledger_entries GROUP BY journal_id HAVING sum(delta_units)<>0) AS broken")
            .fetch_one(&pool).await.unwrap();
        assert_eq!(unbalanced, 0);
        let statistics = crate::history_read::HistoryStore::new(pool.clone())
            .site_statistics(&admin.secret)
            .await
            .unwrap();
        assert_eq!(statistics.total_balance_credits, "2500000");
        assert_eq!(statistics.recharge.confirmed_orders, 5);
        let virtual_units = &statistics.recharge.virtual_units;
        assert_eq!(virtual_units.len(), 1);
        assert_eq!(virtual_units[0].currency, "LDC");
        assert_eq!(virtual_units[0].gross_amount_micros, "1000000");
        assert_eq!(virtual_units[0].net_amount_micros, "1000000");
        assert_eq!(virtual_units[0].orders, 1);
        let cash = &statistics.recharge.currencies;
        assert_eq!(cash.len(), 2);
        assert_eq!(
            (
                &cash[0].currency[..],
                &cash[0].gross_amount_micros[..],
                cash[0].orders
            ),
            ("CNY", "1000000", 1)
        );
        assert_eq!(
            (
                &cash[1].currency[..],
                &cash[1].gross_amount_micros[..],
                cash[1].orders
            ),
            ("USD", "3000000", 3)
        );
        sqlx::query(
            "UPDATE core_identity.credentials SET revoked_at=clock_timestamp() WHERE user_id=7301",
        )
        .execute(&pool)
        .await
        .unwrap();
        let mut c = channel("epay");
        c.id = 1;
        let store = PaymentsStore::new(pool, vec![c]).unwrap();
        assert_eq!(
            store
                .prepare_personal(
                    &session.secret,
                    pb::PreparePersonalTopupRequest {
                        mutation: mutation("gateway_personal_quote_epay".into()),
                        payment_channel_id: 1,
                        currency: "CNY".into(),
                        amount_minor: 100,
                    }
                )
                .await,
            Err(Error::Denied)
        );
    }
}

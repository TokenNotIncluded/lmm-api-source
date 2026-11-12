//! Verifies the original fixed-main 3948a0a provider HMAC receipt inside Core.
//! Neither an extension's usage-source label nor its hash grants spending rights.
use crate::go_json::escape as go_escape;
use base64::{Engine as _, engine::general_purpose::STANDARD};
use hmac::{Hmac, Mac};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use sha2::{Digest, Sha256};
use std::{
    collections::{BTreeMap, BTreeSet},
    fmt,
    ops::Range,
    path::PathBuf,
};
use zeroize::Zeroizing;

const DOMAIN: &[u8] = b"lmm-tool-market-metering-v1\n";
const MAX_BODY: usize = 2 << 20;
const METRICS: &[&str] = &[
    "input_tokens",
    "output_tokens",
    "input_characters",
    "output_characters",
    "images",
    "audio_milliseconds",
    "video_milliseconds",
    "cpu_core_milliseconds",
    "memory_mib_seconds",
    "gpu_milliseconds",
    "vm_milliseconds",
    "storage_mib_seconds",
];

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AdapterConfig {
    pub id: String,
    pub service_id: String,
    pub version_id: String,
    pub tool_id: String,
    pub remote_digest: String,
    pub metrics: Vec<String>,
    pub key_file: PathBuf,
}
struct Adapter {
    config: AdapterConfig,
    key: Zeroizing<Vec<u8>>,
}
pub struct ReceiptVerifier {
    adapters: BTreeMap<String, Adapter>,
}
pub struct ExpectedReceipt<'a> {
    pub call_id: &'a str,
    pub service_id: &'a str,
    pub version_id: &'a str,
    pub tool_id: &'a str,
    pub input_digest: &'a str,
    /// Hash of original supplier billing rules, not Core's tagged settlement Price.
    pub pricing_digest: &'a str,
    pub remote_digest: &'a str,
    /// Reviewed maximum quantity for each expected metric.
    pub metrics: &'a BTreeMap<String, i64>,
}
pub struct VerifiedReceipt {
    result: Value,
    quantities: BTreeMap<String, i64>,
    receipt_hash: String,
    result_digest: String,
    raw_receipt_hash: String,
    successful: bool,
}
impl VerifiedReceipt {
    pub fn result(&self) -> &Value {
        &self.result
    }
    pub fn quantities(&self) -> &BTreeMap<String, i64> {
        &self.quantities
    }
    pub fn receipt_hash(&self) -> &str {
        &self.receipt_hash
    }
    pub fn result_digest(&self) -> &str {
        &self.result_digest
    }
    pub fn raw_receipt_hash(&self) -> &str {
        &self.raw_receipt_hash
    }
    /// Derived from the strictly parsed, signed original result, never a
    /// caller-supplied boolean. Verification alone does not mean billable success.
    pub fn successful(&self) -> bool {
        self.successful
    }
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ReceiptError {
    Configuration,
    Invalid,
    Denied,
}
impl fmt::Display for ReceiptError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::Configuration => "private metering configuration unavailable",
            Self::Invalid => "invalid metering receipt",
            Self::Denied => "metering receipt rejected",
        })
    }
}
impl std::error::Error for ReceiptError {}
type Result<T> = std::result::Result<T, ReceiptError>;

#[derive(Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct ReceiptContext {
    call_id: String,
    version_id: String,
    tool_id: String,
    input_digest: String,
    pricing_digest: String,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct SignedReceipt {
    adapter_id: String,
    context: ReceiptContext,
    quantities: BTreeMap<String, i64>,
    signature: String,
    result_digest: String,
}
// Field order exactly matches Go's toolMarketMeteringMessage struct. This
// message uses struct order; only nested map keys use sorted object encoding.
#[derive(Serialize)]
struct SignedMessage<'a> {
    adapter_id: &'a str,
    context: &'a ReceiptContext,
    quantities: &'a BTreeMap<String, i64>,
    result_digest: &'a str,
}
impl ReceiptVerifier {
    /// Reserve must reject unsupported paid metering before creating a hold.
    /// This is configuration availability, not a replacement for verification.
    pub fn supports(
        &self,
        service_id: &str,
        version_id: &str,
        tool_id: &str,
        remote_digest: &str,
        metrics: &BTreeMap<String, i64>,
    ) -> bool {
        if metrics.is_empty()
            || metrics.len() > METRICS.len()
            || metrics.iter().any(|(metric, max)| {
                !METRICS.contains(&metric.as_str()) || *max < 0 || *max > 1_000_000_000_000
            })
        {
            return false;
        }
        self.adapters.values().any(|adapter| {
            let cfg = &adapter.config;
            cfg.service_id == service_id
                && cfg.version_id == version_id
                && cfg.tool_id == tool_id
                && cfg.remote_digest == remote_digest
                && cfg.metrics.len() == metrics.len()
                && cfg
                    .metrics
                    .iter()
                    .all(|metric| metrics.contains_key(metric))
        })
    }
    pub fn from_adapters(configs: Vec<AdapterConfig>) -> Result<Self> {
        Self::load(configs, false)
    }
    pub fn from_base64_adapters(configs: Vec<AdapterConfig>) -> Result<Self> {
        Self::load(configs, true)
    }
    fn load(configs: Vec<AdapterConfig>, base64: bool) -> Result<Self> {
        if configs.len() > 100 {
            return Err(ReceiptError::Configuration);
        }
        let mut adapters = BTreeMap::new();
        let mut bindings = BTreeSet::new();
        for config in configs {
            if [
                &config.id,
                &config.service_id,
                &config.version_id,
                &config.tool_id,
                &config.remote_digest,
            ]
            .into_iter()
            .any(|s| !valid_text(s))
                || config.metrics.is_empty()
                || config.metrics.len() > METRICS.len()
            {
                return Err(ReceiptError::Configuration);
            }
            let metrics: BTreeSet<_> = config.metrics.iter().collect();
            if metrics.len() != config.metrics.len()
                || config
                    .metrics
                    .iter()
                    .any(|m| !METRICS.contains(&m.as_str()))
            {
                return Err(ReceiptError::Configuration);
            }
            let binding = (
                config.service_id.clone(),
                config.version_id.clone(),
                config.tool_id.clone(),
                config.remote_digest.clone(),
            );
            if !bindings.insert(binding) || adapters.contains_key(&config.id) {
                return Err(ReceiptError::Configuration);
            }
            let raw = Zeroizing::new(
                crate::config::private_file(&config.key_file, if base64 { 1024 } else { 256 })
                    .map_err(|_| ReceiptError::Configuration)?,
            );
            let key = if base64 {
                let text = std::str::from_utf8(&raw).map_err(|_| ReceiptError::Configuration)?;
                Zeroizing::new(
                    STANDARD
                        .decode(text.trim())
                        .map_err(|_| ReceiptError::Configuration)?,
                )
            } else {
                raw
            };
            if !(32..=256).contains(&key.len()) {
                return Err(ReceiptError::Configuration);
            }
            adapters.insert(config.id.clone(), Adapter { config, key });
        }
        Ok(Self { adapters })
    }
    /// Check decoded operator keys against credentials already loaded by Core.
    /// Common whitespace/base64/hex spellings are compared without returning
    /// secret bytes or including them in diagnostics.
    pub fn reject_credentials(&self, forbidden: &[&[u8]]) -> Result<()> {
        use subtle::ConstantTimeEq;
        for adapter in self.adapters.values() {
            let keys = credential_forms(&adapter.key);
            for credential in forbidden {
                let others = credential_forms(credential);
                for key in keys.iter() {
                    let a = Sha256::digest(key);
                    for other in others.iter() {
                        if bool::from(a.ct_eq(&Sha256::digest(other))) {
                            return Err(ReceiptError::Configuration);
                        }
                    }
                }
            }
        }
        Ok(())
    }
    pub(crate) fn reject_model_credentials(
        &self,
        gateway: &crate::model_http::ModelGateway,
    ) -> Result<()> {
        for adapter in self.adapters.values() {
            let forms = credential_forms(&adapter.key);
            for form in forms.iter() {
                if std::str::from_utf8(form).is_ok_and(|s| gateway.contains_credential(s)) {
                    return Err(ReceiptError::Configuration);
                }
            }
        }
        Ok(())
    }
    pub fn verify(
        &self,
        expected: &ExpectedReceipt<'_>,
        raw_result: &[u8],
    ) -> Result<VerifiedReceipt> {
        if [
            &expected.call_id,
            &expected.service_id,
            &expected.version_id,
            &expected.tool_id,
            &expected.input_digest,
            &expected.pricing_digest,
            &expected.remote_digest,
        ]
        .into_iter()
        .any(|s| !valid_text(s))
            || expected.metrics.is_empty()
            || expected.metrics.len() > METRICS.len()
            || expected.metrics.iter().any(|(m, max)| {
                !METRICS.contains(&m.as_str()) || *max <= 0 || *max > 1_000_000_000_000
            })
        {
            return Err(ReceiptError::Invalid);
        }
        let (mut result, span) = parse_result(raw_result)?;
        let successful = successful_result(&result);
        let receipt_bytes = &raw_result[span];
        let receipt: SignedReceipt =
            serde_json::from_slice(receipt_bytes).map_err(|_| ReceiptError::Invalid)?;
        let adapter = self
            .adapters
            .get(&receipt.adapter_id)
            .ok_or(ReceiptError::Denied)?;
        let cfg = &adapter.config;
        if cfg.service_id != expected.service_id
            || cfg.version_id != expected.version_id
            || cfg.tool_id != expected.tool_id
            || cfg.remote_digest != expected.remote_digest
            || cfg.metrics.len() != expected.metrics.len()
            || cfg
                .metrics
                .iter()
                .any(|m| !expected.metrics.contains_key(m))
            || receipt.context.call_id != expected.call_id
            || receipt.context.version_id != expected.version_id
            || receipt.context.tool_id != expected.tool_id
            || receipt.context.input_digest != expected.input_digest
            || receipt.context.pricing_digest != expected.pricing_digest
            || receipt.quantities.len() != expected.metrics.len()
            || receipt.quantities.iter().any(|(metric, count)| {
                *count < 0 || expected.metrics.get(metric).is_none_or(|max| count > max)
            })
        {
            return Err(ReceiptError::Denied);
        }
        let meta = result
            .get_mut("_meta")
            .and_then(Value::as_object_mut)
            .ok_or(ReceiptError::Invalid)?;
        meta.remove("lmm_metering");
        if meta.is_empty() {
            result
                .as_object_mut()
                .ok_or(ReceiptError::Invalid)?
                .remove("_meta");
        }
        let result_digest = hash(&go_value(&result)?);
        if receipt.result_digest != result_digest {
            return Err(ReceiptError::Denied);
        }
        let message = SignedMessage {
            adapter_id: &receipt.adapter_id,
            context: &receipt.context,
            quantities: &receipt.quantities,
            result_digest: &receipt.result_digest,
        };
        let serialized = serde_json::to_string(&message).map_err(|_| ReceiptError::Invalid)?;
        let mut signed = DOMAIN.to_vec();
        signed.extend_from_slice(go_escape(&serialized).as_bytes());
        let signature = decode_signature(&receipt.signature)?;
        let mut mac = Hmac::<Sha256>::new_from_slice(&adapter.key)
            .map_err(|_| ReceiptError::Configuration)?;
        mac.update(&signed);
        mac.verify_slice(&signature)
            .map_err(|_| ReceiptError::Denied)?;
        Ok(VerifiedReceipt {
            result,
            quantities: receipt.quantities,
            receipt_hash: hash(&signed),
            result_digest,
            raw_receipt_hash: hash(receipt_bytes),
            successful,
        })
    }
}
pub(super) fn successful_result(result: &Value) -> bool {
    fn successful_fields(value: &Value) -> bool {
        let Some(fields) = value.as_object() else {
            return false;
        };
        if fields
            .get("isError")
            .is_some_and(|v| v.as_bool() != Some(false))
            || fields
                .get("success")
                .is_some_and(|v| v.as_bool() != Some(true))
        {
            return false;
        }
        if let Some(status) = fields.get("status") {
            let Some(status) = status.as_str() else {
                return false;
            };
            if !matches!(
                status.to_ascii_lowercase().as_str(),
                "success" | "succeeded" | "completed" | "done" | "ok"
            ) {
                return false;
            }
        }
        true
    }
    if !successful_fields(result) {
        return false;
    }
    if result
        .get("structuredContent")
        .is_some_and(|v| v.is_object() && !successful_fields(v))
    {
        return false;
    }
    let content = result
        .get("content")
        .and_then(Value::as_array)
        .is_some_and(|v| !v.is_empty());
    let structured = result
        .get("structuredContent")
        .and_then(Value::as_object)
        .is_some_and(|v| !v.is_empty());
    content || structured
}
fn credential_forms(raw: &[u8]) -> Zeroizing<Vec<Vec<u8>>> {
    use base64::engine::general_purpose::{STANDARD_NO_PAD, URL_SAFE, URL_SAFE_NO_PAD};
    let mut out = Zeroizing::new(vec![raw.to_vec()]);
    let trimmed = std::str::from_utf8(raw)
        .ok()
        .map(str::trim)
        .map(str::as_bytes)
        .unwrap_or(raw);
    if trimmed != raw {
        out.push(trimmed.to_vec());
    }
    for engine in [&STANDARD, &STANDARD_NO_PAD, &URL_SAFE, &URL_SAFE_NO_PAD] {
        if let Ok(decoded) = engine.decode(trimmed)
            && !decoded.is_empty()
        {
            out.push(decoded);
        }
        out.push(engine.encode(trimmed).into_bytes());
    }
    if trimmed.len().is_multiple_of(2) && trimmed.iter().all(u8::is_ascii_hexdigit) {
        let decoded: Vec<u8> = trimmed
            .as_chunks::<2>()
            .0
            .iter()
            .map(|p| {
                let d = |v: u8| {
                    if v.is_ascii_digit() {
                        v - b'0'
                    } else {
                        v.to_ascii_lowercase() - b'a' + 10
                    }
                };
                d(p[0]) * 16 + d(p[1])
            })
            .collect();
        if !decoded.is_empty() {
            out.push(decoded);
        }
    }
    out.push(
        crate::hex_codec::encode(trimmed)
            .into_bytes(),
    );
    out.push(
        trimmed
            .iter()
            .map(|b| format!("{b:02X}"))
            .collect::<String>()
            .into_bytes(),
    );
    out
}
fn valid_text(s: &str) -> bool {
    !s.is_empty() && s.len() <= 256 && s.trim() == s && !s.chars().any(char::is_control)
}
fn hash(bytes: &[u8]) -> String {
    format!("{:x}", Sha256::digest(bytes))
}
fn go_value(value: &Value) -> Result<Vec<u8>> {
    fn write(v: &Value, out: &mut String) -> Result<()> {
        match v {
            Value::Object(map) => {
                out.push('{');
                let sorted: BTreeMap<_, _> = map.iter().collect();
                for (i, (k, v)) in sorted.into_iter().enumerate() {
                    if i > 0 {
                        out.push(',');
                    }
                    out.push_str(&go_escape(
                        &serde_json::to_string(k).map_err(|_| ReceiptError::Invalid)?,
                    ));
                    out.push(':');
                    write(v, out)?;
                }
                out.push('}');
            }
            Value::Array(values) => {
                out.push('[');
                for (i, v) in values.iter().enumerate() {
                    if i > 0 {
                        out.push(',');
                    }
                    write(v, out)?;
                }
                out.push(']');
            }
            _ => out.push_str(&go_escape(
                &serde_json::to_string(v).map_err(|_| ReceiptError::Invalid)?,
            )),
        };
        Ok(())
    }
    let mut out = String::new();
    write(value, &mut out)?;
    Ok(out.into_bytes())
}
fn decode_signature(text: &str) -> Result<[u8; 32]> {
    if text.len() != 64 {
        return Err(ReceiptError::Denied);
    }
    let mut out = [0u8; 32];
    zhex::decode(text.as_bytes(), &mut out).map_err(|_| ReceiptError::Denied)?;
    Ok(out)
}

// Borrow each original token before assembling objects explicitly: serde's
// arbitrary-precision Number sentinel must remain an ordinary object key.
fn parse_original(raw: &[u8]) -> Result<(Value, Option<Range<usize>>)> {
    use serde::de::{DeserializeSeed, Error, MapAccess, SeqAccess, Visitor};
    use serde_json::value::RawValue;

    if raw.is_empty() || raw.len() > MAX_BODY {
        return Err(ReceiptError::Invalid);
    }
    struct Original<'a> {
        source: &'a [u8],
        receipt: Option<Range<usize>>,
    }
    struct Node<'a, 'b> {
        original: &'b mut Original<'a>,
        path: &'b mut Vec<String>,
        depth: usize,
    }
    impl<'de> DeserializeSeed<'de> for Node<'de, '_> {
        type Value = Value;

        fn deserialize<D: serde::Deserializer<'de>>(
            self,
            deserializer: D,
        ) -> std::result::Result<Value, D::Error> {
            if self.depth > 128 {
                return Err(D::Error::custom("receipt nesting exceeds 128"));
            }
            let raw = <&'de RawValue>::deserialize(deserializer)?;
            let token = raw.get();
            if self.path.len() == 2 && self.path[0] == "_meta" && self.path[1] == "lmm_metering" {
                // RawValue borrows a subslice of source, excluding surrounding
                // whitespace. Pointer addresses give its byte offset safely.
                let start = token.as_ptr() as usize - self.original.source.as_ptr() as usize;
                self.original.receipt = Some(start..start + token.len());
            }
            let mut parser = serde_json::Deserializer::from_str(token);
            let value = match token.as_bytes()[0] {
                b'{' => serde::Deserializer::deserialize_map(&mut parser, self),
                b'[' => serde::Deserializer::deserialize_seq(&mut parser, self),
                b'-' | b'0'..=b'9' => {
                    // RawValue already checked JSON grammar; preserve -0 and
                    // exponent spelling rather than normalizing via Value.
                    return Ok(Value::Number(serde_json::Number::from_string_unchecked(
                        token.into(),
                    )));
                }
                _ => Value::deserialize(&mut parser),
            }
            .map_err(D::Error::custom)?;
            parser.end().map_err(D::Error::custom)?;
            Ok(value)
        }
    }
    impl<'de> Visitor<'de> for Node<'de, '_> {
        type Value = Value;

        fn expecting(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
            formatter.write_str("a receipt JSON object or array")
        }
        fn visit_map<M: MapAccess<'de>>(self, mut map: M) -> std::result::Result<Value, M::Error> {
            let mut object = serde_json::Map::new();
            while let Some(key) = map.next_key::<String>()? {
                if object.contains_key(&key) {
                    return Err(M::Error::custom("duplicate receipt object key"));
                }
                self.path.push(key.clone());
                let child = map.next_value_seed(Node {
                    original: self.original,
                    path: self.path,
                    depth: self.depth + 1,
                })?;
                self.path.pop();
                object.insert(key, child);
            }
            Ok(Value::Object(object))
        }
        fn visit_seq<S: SeqAccess<'de>>(self, mut seq: S) -> std::result::Result<Value, S::Error> {
            let mut array = Vec::new();
            self.path.push(String::new());
            while let Some(child) = seq.next_element_seed(Node {
                original: self.original,
                path: self.path,
                depth: self.depth + 1,
            })? {
                array.push(child);
            }
            self.path.pop();
            Ok(Value::Array(array))
        }
    }
    let mut original = Original {
        source: raw,
        receipt: None,
    };
    let mut parser = serde_json::Deserializer::from_slice(raw);
    let value = Node {
        original: &mut original,
        path: &mut Vec::new(),
        depth: 1,
    }
    .deserialize(&mut parser)
    .map_err(|_| ReceiptError::Invalid)?;
    parser.end().map_err(|_| ReceiptError::Invalid)?;
    if !value.is_object() {
        return Err(ReceiptError::Invalid);
    }
    Ok((value, original.receipt))
}

fn parse_result(raw: &[u8]) -> Result<(Value, Range<usize>)> {
    let (v, span) = parse_original(raw)?;
    Ok((v, span.ok_or(ReceiptError::Invalid)?))
}
pub(super) fn decode_original_result(raw: &[u8]) -> Result<Value> {
    parse_original(raw).map(|(v, _)| v)
}
pub(super) fn result_digest(result: &Value) -> Result<String> {
    let mut value = result.clone();
    if let Some(meta) = value.get_mut("_meta").and_then(Value::as_object_mut) {
        meta.remove("lmm_metering");
        if meta.is_empty() {
            value
                .as_object_mut()
                .ok_or(ReceiptError::Invalid)?
                .remove("_meta");
        }
    }
    Ok(hash(&go_value(&value)?))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::{
        fs,
        path::Path,
        sync::atomic::{AtomicU64, Ordering},
    };
    static NEXT: AtomicU64 = AtomicU64::new(0);
    const GOLDEN: &str = r#"{"_meta":{"lmm_metering":{"adapter_id":"collector","context":{"call_id":"call","version_id":"version","tool_id":"tool","input_digest":"input","pricing_digest":"pricing"},"quantities":{"input_tokens":3},"result_digest":"142cfedc79c163e03461c6a0920436a01e3c314c9b536425f12fe51bce7e2945","signature":"861428ed77088e614dfcd79f10ab4761a0610c94a58b6e8f53d18ffe6e967179"}},"structuredContent":{"exponent":1e+3,"html":"\u003c\u0026\u003e\u2028\u2029","negativezero":-0,"nested":{"a":2,"z":1},"number":9007199254740991}}"#;
    struct Fixture(PathBuf);
    impl Fixture {
        fn new() -> Self {
            let root = std::env::temp_dir().join(format!(
                "lmm-receipt-{}-{}",
                std::process::id(),
                NEXT.fetch_add(1, Ordering::Relaxed)
            ));
            fs::create_dir(&root).unwrap();
            #[cfg(unix)]
            {
                use std::os::unix::fs::PermissionsExt;
                fs::set_permissions(&root, fs::Permissions::from_mode(0o700)).unwrap();
            }
            Self(root)
        }
        fn key(&self, data: &[u8]) -> PathBuf {
            let path = self.0.join("collector.key");
            fs::write(&path, data).unwrap();
            #[cfg(unix)]
            {
                use std::os::unix::fs::PermissionsExt;
                fs::set_permissions(&path, fs::Permissions::from_mode(0o600)).unwrap();
            }
            path
        }
    }
    impl Drop for Fixture {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }
    fn adapter(path: &Path) -> AdapterConfig {
        AdapterConfig {
            id: "collector".into(),
            service_id: "service".into(),
            version_id: "version".into(),
            tool_id: "tool".into(),
            remote_digest: "remote".into(),
            metrics: vec!["input_tokens".into()],
            key_file: path.into(),
        }
    }
    fn expected(metrics: &BTreeMap<String, i64>) -> ExpectedReceipt<'_> {
        ExpectedReceipt {
            call_id: "call",
            service_id: "service",
            version_id: "version",
            tool_id: "tool",
            input_digest: "input",
            pricing_digest: "pricing",
            remote_digest: "remote",
            metrics,
        }
    }
    fn verifier() -> (Fixture, ReceiptVerifier) {
        let fixture = Fixture::new();
        let path = fixture.key(&[0x7b; 32]);
        let verifier = ReceiptVerifier::from_adapters(vec![adapter(&path)]).unwrap();
        (fixture, verifier)
    }
    #[test]
    fn receipt_matches_actual_go_golden_and_canonical_number_escapes() {
        // Generated by encoding/json + crypto/hmac in go-receipt-golden.go.
        // Covers exact >2^53 numbers, number exponent spelling, -0, sorted
        // object keys and Go's HTML/U+2028/U+2029 escaping.
        let parsed = decode_original_result(GOLDEN.as_bytes()).unwrap();
        let computed = result_digest(&parsed).unwrap();
        assert_eq!(
            computed,
            "142cfedc79c163e03461c6a0920436a01e3c314c9b536425f12fe51bce7e2945",
            "Go canonical fixture: {}",
            String::from_utf8(go_value(&parsed).unwrap()).unwrap()
        );
        let (_fixture, verifier) = verifier();
        let metrics = BTreeMap::from([("input_tokens".into(), 100)]);
        let receipt = verifier
            .verify(&expected(&metrics), GOLDEN.as_bytes())
            .unwrap();
        assert_eq!(receipt.quantities().get("input_tokens"), Some(&3));
        assert_eq!(
            receipt.result_digest(),
            "142cfedc79c163e03461c6a0920436a01e3c314c9b536425f12fe51bce7e2945"
        );
        assert_eq!(
            receipt.receipt_hash(),
            "7d26561ef7bebc95ecc07d9f921e593fdd3e7ce774933938b89a95b37757943c"
        );
        let spaced = GOLDEN.replace("\"adapter_id\":", "\"adapter_id\" : ");
        let other = verifier
            .verify(&expected(&metrics), spaced.as_bytes())
            .unwrap();
        assert_eq!(receipt.receipt_hash(), other.receipt_hash());
        assert_ne!(receipt.raw_receipt_hash(), other.raw_receipt_hash());
    }
    #[test]
    fn receipt_rejects_tampering_and_all_binding_dimensions() {
        let (_fixture, verifier) = verifier();
        let metrics = BTreeMap::from([("input_tokens".into(), 100)]);
        for field in [
            "call_id",
            "service_id",
            "version_id",
            "tool_id",
            "input_digest",
            "pricing_digest",
            "remote_digest",
        ] {
            let mut e = expected(&metrics);
            match field {
                "call_id" => e.call_id = "other",
                "service_id" => e.service_id = "other",
                "version_id" => e.version_id = "other",
                "tool_id" => e.tool_id = "other",
                "input_digest" => e.input_digest = "other",
                "pricing_digest" => e.pricing_digest = "other",
                _ => e.remote_digest = "other",
            };
            assert!(
                verifier.verify(&e, GOLDEN.as_bytes()).is_err(),
                "accepted {field}"
            );
        }
        for bad in [
            GOLDEN.replace("861428ed", "061428ed"),
            GOLDEN.replace("9007199254740991", "9007199254740990"),
            GOLDEN.replace("\"input_tokens\":3", "\"input_tokens\":-1"),
            GOLDEN.replace("\"input_tokens\":3", "\"input_tokens\":101"),
            GOLDEN.replace("\"input_tokens\":3", "\"images\":3"),
            GOLDEN.replace("\"input_tokens\":3", "\"input_tokens\":3,\"images\":0"),
            GOLDEN.replace("\"input_tokens\":3", "\"input_tokens\":3.0"),
            GOLDEN.replace("\"input_tokens\":3", "\"input_tokens\":9223372036854775808"),
            GOLDEN.replace("\"input_tokens\":3", "\"input_tokens\":null"),
            GOLDEN.replace("\"adapter_id\":\"collector\"", "\"adapter_id\":\"unknown\""),
            GOLDEN.replace(
                "\"adapter_id\":\"collector\"",
                "\"adapter_id\":\"collector\",\"extra\":0",
            ),
            GOLDEN.replace("\"call_id\":\"call\"", "\"call_id\":\"call\",\"extra\":0"),
            GOLDEN.replace(
                "\"input_tokens\":3",
                "\"input_tokens\":3,\"input_tokens\":3",
            ),
            GOLDEN.replace(
                "\"input_tokens\":3",
                "\"input_tokens\":3,\"input_\\u0074okens\":3",
            ),
            GOLDEN.replace(
                "\"call_id\":\"call\"",
                "\"call_id\":\"call\",\"call_id\":\"call\"",
            ),
            format!("{GOLDEN} {{}}"),
        ] {
            assert!(
                verifier
                    .verify(&expected(&metrics), bad.as_bytes())
                    .is_err()
            );
        }
        let missing_metric = BTreeMap::from([("images".into(), 100)]);
        assert!(
            verifier
                .verify(&expected(&missing_metric), GOLDEN.as_bytes())
                .is_err()
        );
        let empty = ReceiptVerifier::from_adapters(vec![]).unwrap();
        assert!(
            empty
                .verify(&expected(&metrics), GOLDEN.as_bytes())
                .is_err()
        );
        assert!(
            verifier
                .verify(&expected(&metrics), br#"{"usage":{"input_tokens":3}}"#)
                .is_err()
        );
    }
    #[test]
    fn receipt_private_keys_and_operator_scope_fail_closed() {
        let fixture = Fixture::new();
        let path = fixture.key(&[0x7b; 32]);
        for size in [0, 31, 257] {
            fixture.key(&vec![1; size]);
            assert!(ReceiptVerifier::from_adapters(vec![adapter(&path)]).is_err());
        }
        fixture.key(&[0x7b; 32]);
        assert!(ReceiptVerifier::from_adapters(vec![adapter(Path::new("relative.key"))]).is_err());
        let mut duplicate = adapter(&path);
        duplicate.id = "another".into();
        assert!(ReceiptVerifier::from_adapters(vec![adapter(&path), duplicate]).is_err());
        let mut duplicate_id = adapter(&path);
        duplicate_id.tool_id = "another".into();
        assert!(ReceiptVerifier::from_adapters(vec![adapter(&path), duplicate_id]).is_err());
        let mut bad_metrics = adapter(&path);
        bad_metrics.metrics.push("input_tokens".into());
        assert!(ReceiptVerifier::from_adapters(vec![bad_metrics]).is_err());
        let mut unknown = adapter(&path);
        unknown.metrics = vec!["money".into()];
        assert!(ReceiptVerifier::from_adapters(vec![unknown]).is_err());
        #[cfg(unix)]
        {
            use std::os::unix::fs::{PermissionsExt, symlink};
            fs::set_permissions(&path, fs::Permissions::from_mode(0o644)).unwrap();
            assert!(ReceiptVerifier::from_adapters(vec![adapter(&path)]).is_err());
            fs::set_permissions(&path, fs::Permissions::from_mode(0o600)).unwrap();
            let link = fixture.0.join("link");
            symlink(&path, &link).unwrap();
            assert!(ReceiptVerifier::from_adapters(vec![adapter(&link)]).is_err());
        }
        fixture.key(STANDARD.encode([0x7b; 32]).as_bytes());
        let verifier = ReceiptVerifier::from_base64_adapters(vec![adapter(&path)]).unwrap();
        let metrics = BTreeMap::from([("input_tokens".into(), 100)]);
        assert!(
            verifier
                .verify(&expected(&metrics), GOLDEN.as_bytes())
                .is_ok()
        );
        fixture.key(b"invalid base64");
        assert!(ReceiptVerifier::from_base64_adapters(vec![adapter(&path)]).is_err());
    }
    #[test]
    fn supplier_keys_cannot_reuse_loaded_role_credentials() {
        let fixture = Fixture::new();
        let role = b"shared-host-role-credential-32bytes";
        for raw in [
            role.to_vec(),
            [role.as_slice(), b"\n"].concat(),
            STANDARD.encode(role).into_bytes(),
            crate::hex_codec::encode(&role)
                .into_bytes(),
        ] {
            let path = fixture.key(&raw);
            let verifier = ReceiptVerifier::from_adapters(vec![adapter(&path)]).unwrap();
            assert!(verifier.reject_credentials(&[role]).is_err());
            assert!(
                verifier
                    .reject_credentials(&[b"independent-other-module-credential"])
                    .is_ok()
            );
        }
        let encoded = STANDARD.encode(role);
        let path = fixture.key(encoded.as_bytes());
        let verifier = ReceiptVerifier::from_base64_adapters(vec![adapter(&path)]).unwrap();
        assert!(verifier.reject_credentials(&[role]).is_err());
    }
    #[test]
    fn original_parser_preserves_spans_and_number_tokens() {
        let receipt = r#"{ "n": -0, "e": 1E-03 }"#;
        let raw = format!(
            " \n{{\"_meta\":{{\"lmm_\\u006detering\": \t{receipt} \r}},\"n\":[-0,1E+003,1e-03],\"other\":{{\"_meta\":{{\"lmm_metering\":false}}}}}} \t"
        );
        let (value, span) = parse_original(raw.as_bytes()).unwrap();
        assert_eq!(&raw[span.unwrap()], receipt);
        assert_eq!(go_value(&value["n"]).unwrap(), b"[-0,1E+003,1e-03]");
        for nested in [
            r#"{"$serde_json::private::Number":"123","x":1}"#,
            r#"{"$serde_json::private::Number":{"x":true}}"#,
        ] {
            let raw = format!(r#"{{"value":{nested}}}"#);
            let (value, _) = parse_original(raw.as_bytes()).unwrap();
            assert_eq!(go_value(&value["value"]).unwrap(), nested.as_bytes());
        }
    }
    #[test]
    fn original_parser_enforces_depth_size_and_json_grammar() {
        // Root is depth 1; the scalar in 126 nested arrays is depth 128.
        let nested =
            |arrays: usize| format!(r#"{{"x":{}0{}}}"#, "[".repeat(arrays), "]".repeat(arrays));
        assert!(parse_original(nested(126).as_bytes()).is_ok());
        assert!(parse_original(nested(127).as_bytes()).is_err());
        let mut boundary = b"{}".to_vec();
        boundary.resize(MAX_BODY, b' ');
        assert!(parse_original(&boundary).is_ok());
        boundary.push(b' ');
        assert!(parse_original(&boundary).is_err());
        for raw in [
            r#"{"a":[{"b":0,"\u0062":1}]}"#,
            r#"{"\u005fmeta":{},"_meta":{}}"#,
            r#"{"a":[1,]}"#,
            r#"{"a":01}"#,
            r#"{"a":1e+}"#,
            r#"{"a":"\uD800"}"#,
            r#"{"a":"\x20"}"#,
            r#"{"a":true false}"#,
            "{} {}",
            "",
        ] {
            assert!(parse_original(raw.as_bytes()).is_err(), "accepted {raw:?}");
        }
        assert!(parse_original(b"{\"a\":\"\xff\"}").is_err());
    }
    #[test]
    fn signature_hex_preserves_case_and_denied_errors() {
        assert_eq!(decode_signature(&"aB".repeat(32)).unwrap(), [0xab; 32]);
        for invalid in [
            "0".repeat(63),
            "0".repeat(65),
            "gg".repeat(32),
            "é".repeat(32),
        ] {
            assert_eq!(decode_signature(&invalid), Err(ReceiptError::Denied));
        }
    }
    #[test]
    fn receipt_json_parser_rejects_duplicates_missing_and_excessive_nesting() {
        for raw in [
            b"null".as_slice(),
            b"[]",
            br#"{"_meta":null}"#,
            br#"{"_meta":{"lmm_metering":{},"lmm_metering":{}}}"#,
            br#"{"a":{"b":1,"b":2},"_meta":{"lmm_metering":{}}}"#,
            br#"{"_meta":{"lmm_metering":{},}}"#,
        ] {
            assert!(parse_result(raw).is_err());
        }
        let deep = format!("{}0{}", "[".repeat(130), "]".repeat(130));
        assert!(parse_result(deep.as_bytes()).is_err());
        assert!(parse_result(&vec![b' '; MAX_BODY + 1]).is_err());
        let illegal = GOLDEN.replace("\"_meta\":", "\"_meta\":\u{000b}");
        assert!(parse_result(illegal.as_bytes()).is_err());
        let raw=br#"{"_meta":{"lmm_metering":{}},"structuredContent":{"$serde_json::private::Number":"123"}}"#;
        let (mut value, _) = parse_result(raw).unwrap();
        value.as_object_mut().unwrap().remove("_meta");
        assert_eq!(
            go_value(&value).unwrap(),
            br#"{"structuredContent":{"$serde_json::private::Number":"123"}}"#
        );
    }
}

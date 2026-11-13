//! Bounded text, image and function-call conversion. Unsupported inputs fail closed.
use super::{ClientProtocol, ImageInput, Limits, RelayError, Route, UpstreamProtocol};
use serde::{
    Deserialize,
    de::{MapAccess, Visitor},
};
use serde_json::{Map, Value, json};
#[cfg(test)]
use std::collections::BTreeMap;
use std::collections::BTreeSet;

#[derive(Clone, Debug)]
pub enum InputBlock {
    Text(String),
    Image(ImageInput),
    Refusal(String),
    Thinking {
        text: String,
        signature: Option<(UpstreamProtocol, String)>,
    },
    ToolCall {
        id: String,
        name: String,
        arguments: String,
        signature: Option<(UpstreamProtocol, String)>,
    },
    ToolResult {
        id: String,
        content: String,
    },
}
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Role {
    System,
    Developer,
    User,
    Assistant,
    Tool,
}
impl Role {
    fn parse(v: &Value) -> Result<Self, RelayError> {
        match v.as_str() {
            Some("system") => Ok(Self::System),
            Some("developer") => Ok(Self::Developer),
            Some("user") => Ok(Self::User),
            Some("assistant") => Ok(Self::Assistant),
            Some("tool") => Ok(Self::Tool),
            _ => Err(RelayError::Invalid("unsupported message role")),
        }
    }
    #[cfg(test)]
    fn name(self) -> &'static str {
        match self {
            Self::System => "system",
            Self::Developer => "developer",
            Self::User => "user",
            Self::Assistant => "assistant",
            Self::Tool => "tool",
        }
    }
}
#[derive(Clone, Debug)]
pub struct Message {
    pub role: Role,
    pub content: Vec<InputBlock>,
}
#[derive(Clone, Debug)]
pub struct FunctionTool {
    pub name: String,
    pub description: Option<String>,
    pub parameters: Value,
    pub strict: Option<bool>,
    /// Compact JSON for `parameters`, using the client's object-key order when
    /// that text still parses as the validated value.
    pub(crate) ordered_parameters: String,
}
#[derive(Clone, Debug)]
pub enum ToolChoice {
    Auto,
    None,
    Required,
    Named(String),
}
#[derive(Clone, Debug)]
pub struct Request {
    pub client: ClientProtocol,
    pub model: String,
    pub stream: bool,
    pub max_output_tokens: u32,
    /// The client's validated output limit before the gateway applies its default.
    /// Legacy precharge estimates use the original value, including its absence.
    pub explicit_max_output_tokens: Option<u32>,
    /// Original bounded Chat/Responses bytes for the source precharge estimator.
    /// Route projection and compatibility normalization must not change them.
    pub(crate) legacy_precharge_body: Option<std::sync::Arc<[u8]>>,
    pub messages: Vec<Message>,
    pub tools: Vec<FunctionTool>,
    pub temperature: Option<f64>,
    pub top_p: Option<f64>,
    pub stop: Vec<String>,
    pub tool_choice: ToolChoice,
    pub parallel_tool_calls: Option<bool>,
    pub reasoning_effort: Option<String>,
    /// Native Google Search only; its supplier contract is selected by Core.
    pub(crate) google_search: bool,
    /// Bounded Responses cache controls; never evidence of actual billed writes.
    pub prompt_cache_options: Option<Value>,
    /// Validated native prompt sections with cache breakpoints. Keep original
    /// block/message boundaries: normalization can change the cached prefix.
    pub(super) anthropic_cached_prompt: Option<Map<String, Value>>,
    /// Validated Chat image controls, only emitted on a Gemini route.
    image_config: Option<ImageConfig>,
    /// Explicit OpenRouter Chat controls, emitted only by its selected contract.
    openrouter: Map<String, Value>,
    pub(crate) include_usage: bool,
    /// Gemini Long Decode token. Present only when the native request sent a
    /// non-empty `continuationToken`. Other protocols cannot replay it.
    pub(crate) continuation_token: Option<String>,
    /// Untrusted compatibility values awaiting the actual selected route policy.
    compatibility: Map<String, Value>,
    /// OpenAI Chat `response_format` and Responses `text.format`. Gemini receives `responseSchema` or
    /// `responseJsonSchema`; other protocols cannot represent it.
    response_format: Option<ResponseFormat>,
}

#[derive(Clone, Debug)]
enum ResponseFormat {
    Text,
    JsonObject,
    JsonSchema(JsonSchemaFormat),
}

#[derive(Clone, Debug)]
struct JsonSchemaFormat {
    name: String,
    description: Option<String>,
    strict: Option<bool>,
    schema: Value,
    ordered: String,
}

impl ResponseFormat {
    fn to_chat_value(&self) -> Value {
        match self {
            Self::Text => json!({"type":"text"}),
            Self::JsonObject => json!({"type":"json_object"}),
            Self::JsonSchema(schema) => {
                let mut body = json!({"name": schema.name, "schema": schema.schema});
                if let Some(description) = &schema.description {
                    body["description"] = json!(description);
                }
                if let Some(strict) = schema.strict {
                    body["strict"] = json!(strict);
                }
                json!({"type":"json_schema","json_schema": body})
            }
        }
    }

    fn to_responses_value(&self) -> Value {
        let chat = self.to_chat_value();
        if let Self::JsonSchema(_) = self {
            let mut format = chat["json_schema"].clone();
            format["type"] = json!("json_schema");
            format
        } else {
            chat
        }
    }

    fn apply_gemini(&self, generation: &mut Value, client: ClientProtocol) {
        match self {
            Self::Text => {}
            Self::JsonObject => {
                generation["responseMimeType"] = json!("application/json");
            }
            Self::JsonSchema(schema) => {
                generation["responseMimeType"] = json!("application/json");
                if client == ClientProtocol::Responses {
                    // Keep Responses JSON Schema intact so the ordered-schema
                    // writer can match it, including additionalProperties.
                    generation["responseJsonSchema"] = schema.schema.clone();
                } else {
                    let placed = super::gemini_schema::response_schema(
                        &schema.schema,
                        super::gemini_schema::ACCEPTS_JSON_SCHEMA,
                    );
                    debug_assert!(placed.warnings.is_empty());
                    generation[placed.field] = placed.value;
                }
            }
        }
    }

    fn ordered_schema(&self) -> Option<&str> {
        match self {
            Self::JsonSchema(schema) => Some(schema.ordered.as_str()),
            Self::Text | Self::JsonObject => None,
        }
    }

    fn adopt_ordered_schema(&mut self, raw: &str) {
        let Self::JsonSchema(schema) = self else {
            return;
        };
        if let Some(text) = super::schema_order::ordered_json_matching(raw, &schema.schema) {
            schema.ordered = text;
        }
    }
}

fn object(v: &Value) -> Result<&Map<String, Value>, RelayError> {
    v.as_object()
        .ok_or(RelayError::Invalid("expected an object"))
}

/// Reject ambiguous top-level routing/control keys before Value can overwrite
/// a duplicate. Keys remain case-sensitive; case variants are never normalized.
struct RequestObject(Map<String, Value>);
impl<'de> Deserialize<'de> for RequestObject {
    fn deserialize<D: serde::Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        struct ObjectVisitor;
        impl<'de> Visitor<'de> for ObjectVisitor {
            type Value = RequestObject;
            fn expecting(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
                f.write_str("an unambiguous request object")
            }
            fn visit_map<A: MapAccess<'de>>(self, mut map: A) -> Result<Self::Value, A::Error> {
                let mut seen = BTreeSet::new();
                let mut values = Map::new();
                while let Some(key) = map.next_key::<String>()? {
                    if !seen.insert(key.to_ascii_lowercase()) {
                        return Err(serde::de::Error::custom("duplicate request key"));
                    }
                    let value = if key == "stream_options" {
                        map.next_value::<StreamOptions>()?.0
                    } else {
                        map.next_value()?
                    };
                    values.insert(key, value);
                }
                Ok(RequestObject(values))
            }
        }
        d.deserialize_map(ObjectVisitor)
    }
}
/// Parse only direct option keys on the existing deserializer so nested values
/// retain its depth limit. Never restart request parsing inside an option value.
struct StreamOptions(Value);
impl<'de> Deserialize<'de> for StreamOptions {
    fn deserialize<D: serde::Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        struct OptionsVisitor;
        impl<'de> Visitor<'de> for OptionsVisitor {
            type Value = StreamOptions;
            fn expecting(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
                f.write_str("null or an unambiguous stream options object")
            }
            fn visit_unit<E: serde::de::Error>(self) -> Result<Self::Value, E> {
                Ok(StreamOptions(Value::Null))
            }
            fn visit_map<A: MapAccess<'de>>(self, mut map: A) -> Result<Self::Value, A::Error> {
                let mut seen = BTreeSet::new();
                let mut values = Map::new();
                while let Some(key) = map.next_key::<String>()? {
                    if !seen.insert(key.to_ascii_lowercase()) {
                        return Err(serde::de::Error::custom("duplicate stream option key"));
                    }
                    if !matches!(key.as_str(), "include_usage" | "include_obfuscation") {
                        return Err(serde::de::Error::custom("unsupported stream option key"));
                    }
                    values.insert(key, map.next_value::<Value>()?);
                }
                Ok(StreamOptions(Value::Object(values)))
            }
        }
        d.deserialize_any(OptionsVisitor)
    }
}
fn keys(v: &Value, allowed: &[&str]) -> Result<(), RelayError> {
    if object(v)?.keys().any(|k| !allowed.contains(&k.as_str())) {
        return Err(RelayError::Unsupported(
            "request contains an unsupported field",
        ));
    }
    Ok(())
}
fn string(v: &Value) -> Result<String, RelayError> {
    v.as_str()
        .map(str::to_owned)
        .ok_or(RelayError::Invalid("expected a string"))
}
fn name(v: &Value) -> Result<String, RelayError> {
    let s = string(v)?;
    if s.is_empty()
        || s.len() > 128
        || !s
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b"-_".contains(&b))
    {
        return Err(RelayError::Invalid("invalid function name or call ID"));
    }
    Ok(s)
}
fn boolean(v: Option<&Value>) -> Result<Option<bool>, RelayError> {
    v.filter(|v| !v.is_null())
        .map(|v| v.as_bool().ok_or(RelayError::Invalid("expected a boolean")))
        .transpose()
}
fn number(v: Option<&Value>, max: f64) -> Result<Option<f64>, RelayError> {
    v.filter(|v| !v.is_null())
        .map(|v| {
            v.as_f64()
                .filter(|n| n.is_finite() && *n >= 0.0 && *n <= max)
                .ok_or(RelayError::Invalid("invalid sampling value"))
        })
        .transpose()
}
fn image_config_value(v: &Value) -> Result<String, RelayError> {
    let value = string(v)?;
    if value.is_empty()
        || value.len() > 32
        || value
            .bytes()
            .any(|b| b.is_ascii_control() || b.is_ascii_whitespace())
    {
        return Err(RelayError::Invalid("invalid Gemini image config value"));
    }
    Ok(value)
}

#[derive(Clone, Debug)]
struct ImageConfig {
    image_size: Option<String>,
    aspect_ratio: Option<String>,
}

impl ImageConfig {
    fn parse(v: Option<&Value>) -> Result<Option<Self>, RelayError> {
        let Some(v) = v.filter(|v| !v.is_null()) else {
            return Ok(None);
        };
        keys(v, &["imageSize", "aspectRatio"])?;
        let config = Self {
            image_size: v.get("imageSize").map(image_config_value).transpose()?,
            aspect_ratio: v.get("aspectRatio").map(image_config_value).transpose()?,
        };
        if config.image_size.is_none() && config.aspect_ratio.is_none() {
            return Ok(None);
        }
        Ok(Some(config))
    }

    fn to_gemini_value(&self) -> Value {
        let mut value = Map::new();
        if let Some(image_size) = &self.image_size {
            value.insert("imageSize".into(), json!(image_size));
        }
        if let Some(aspect_ratio) = &self.aspect_ratio {
            value.insert("aspectRatio".into(), json!(aspect_ratio));
        }
        Value::Object(value)
    }
}

fn openrouter_options(v: &Value) -> Result<Map<String, Value>, RelayError> {
    let mut options = Map::new();
    for name in ["modalities", "image_config", "usage", "reasoning"] {
        let Some(value) = v.get(name).filter(|v| !v.is_null()) else {
            continue;
        };
        match name {
            "modalities" => {
                let values = value
                    .as_array()
                    .ok_or(RelayError::Invalid("invalid modalities"))?;
                let mut seen = std::collections::BTreeSet::new();
                if values.is_empty()
                    || values.len() > 2
                    || values.iter().any(|v| {
                        !matches!(v.as_str(), Some("text" | "image"))
                            || !seen.insert(v.as_str().unwrap())
                    })
                {
                    return Err(RelayError::Invalid("invalid modalities"));
                }
            }
            "image_config" => {
                keys(value, &["aspect_ratio", "image_size"])?;
                for (_, v) in object(value)? {
                    image_config_value(v)?;
                }
            }
            "usage" => {
                keys(value, &["include"])?;
                if !value["include"].is_boolean() {
                    return Err(RelayError::Invalid("invalid usage include"));
                }
            }
            "reasoning" => {
                keys(value, &["enabled", "effort", "max_tokens", "exclude"])?;
                for key in ["enabled", "exclude"] {
                    if value.get(key).is_some_and(|v| !v.is_boolean()) {
                        return Err(RelayError::Invalid("invalid reasoning boolean"));
                    }
                }
                if value
                    .get("max_tokens")
                    .is_some_and(|v| v.as_u64().is_none_or(|n| n == 0 || n > u32::MAX as u64))
                {
                    return Err(RelayError::Invalid("invalid reasoning max_tokens"));
                }
                if let Some(effort) = value.get("effort") {
                    image_config_value(effort)?;
                }
                if value.get("effort").is_some() && value.get("max_tokens").is_some() {
                    return Err(RelayError::Invalid("ambiguous reasoning budget"));
                }
            }
            _ => unreachable!(),
        }
        options.insert(name.into(), value.clone());
    }
    Ok(options)
}

fn chat_image_config(v: &Value) -> Result<Option<ImageConfig>, RelayError> {
    ImageConfig::parse(v.get("imageConfig"))
}

fn response_format(v: &Value) -> Result<Option<ResponseFormat>, RelayError> {
    let Some(v) = v.get("response_format").filter(|v| !v.is_null()) else {
        return Ok(None);
    };
    keys(v, &["type", "json_schema"])?;
    let kind = v
        .get("type")
        .map(string)
        .transpose()?
        .ok_or(RelayError::Invalid("invalid response format"))?;
    match kind.as_str() {
        "text" => {
            if v.get("json_schema").is_some() {
                return Err(RelayError::Invalid("text response format has a schema"));
            }
            Ok(Some(ResponseFormat::Text))
        }
        "json_object" => {
            if v.get("json_schema").is_some() {
                return Err(RelayError::Invalid(
                    "json_object response format has a schema",
                ));
            }
            Ok(Some(ResponseFormat::JsonObject))
        }
        "json_schema" => {
            let schema = v.get("json_schema").ok_or(RelayError::Invalid(
                "json_schema response format requires a schema",
            ))?;
            Ok(Some(ResponseFormat::JsonSchema(json_schema_format(
                schema,
            )?)))
        }
        _ => Err(RelayError::Unsupported("unsupported response format")),
    }
}

fn responses_text_format(v: &Value) -> Result<Option<ResponseFormat>, RelayError> {
    let Some(text) = v.get("text").filter(|v| !v.is_null()) else {
        return Ok(None);
    };
    keys(text, &["format"])?;
    let Some(format) = text.get("format").filter(|v| !v.is_null()) else {
        return Ok(None);
    };
    if format["type"] == "json_schema" {
        keys(format, &["type", "name", "description", "schema", "strict"])?;
        let mut schema = object(format)?.clone();
        schema.remove("type");
        Ok(Some(ResponseFormat::JsonSchema(json_schema_format(
            &Value::Object(schema),
        )?)))
    } else {
        response_format(&json!({"response_format": format}))
    }
}

fn json_schema_format(v: &Value) -> Result<JsonSchemaFormat, RelayError> {
    keys(v, &["name", "description", "schema", "strict"])?;
    let name = schema_name(
        v.get("name")
            .ok_or(RelayError::Invalid("response schema requires a name"))?,
    )?;
    let schema = v
        .get("schema")
        .ok_or(RelayError::Invalid("response schema must be an object"))?;
    if !schema.is_object() {
        return Err(RelayError::Invalid("response schema must be an object"));
    }
    let description = v.get("description").map(string).transpose()?;
    let strict = boolean(v.get("strict"))?;
    let ordered = serde_json::to_string(schema)
        .map_err(|_| RelayError::Invalid("response schema encoding failed"))?;
    Ok(JsonSchemaFormat {
        name,
        description,
        strict,
        schema: schema.clone(),
        ordered,
    })
}

fn schema_name(v: &Value) -> Result<String, RelayError> {
    let name = string(v)?;
    if name.is_empty()
        || name.len() > 64
        || !name
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || b"-_".contains(&byte))
    {
        return Err(RelayError::Invalid("invalid response schema name"));
    }
    Ok(name)
}

fn signature(v: Option<&Value>) -> Result<Option<(UpstreamProtocol, String)>, RelayError> {
    let Some(v) = v.filter(|v| !v.is_null()) else {
        return Ok(None);
    };
    keys(v, &["protocol", "signature"])?;
    let protocol = match v["protocol"].as_str() {
        Some("anthropic") => UpstreamProtocol::Anthropic,
        Some("gemini") => UpstreamProtocol::Gemini,
        Some("chat") => UpstreamProtocol::Chat,
        Some("responses") => UpstreamProtocol::Responses,
        _ => return Err(RelayError::Invalid("invalid signature provider")),
    };
    Ok(Some((protocol, string(&v["signature"])?)))
}
fn arguments(v: &Value, limit: usize) -> Result<String, RelayError> {
    let text = string(v)?;
    if text.len() > limit {
        return Err(RelayError::Limit("tool arguments"));
    }
    let parsed: Value = serde_json::from_str(&text)
        .map_err(|_| RelayError::Invalid("tool arguments must be JSON"))?;
    if !parsed.is_object() {
        return Err(RelayError::Invalid("tool arguments must be an object"));
    }
    Ok(text)
}
fn text_blocks(
    v: &Value,
    role: Role,
    client: ClientProtocol,
) -> Result<Vec<InputBlock>, RelayError> {
    if v.is_null() {
        return Ok(Vec::new());
    }
    if let Some(s) = v.as_str() {
        return Ok(vec![InputBlock::Text(s.into())]);
    }
    let parts = v
        .as_array()
        .ok_or(RelayError::Invalid("invalid message content"))?;
    let mut out = Vec::new();
    for p in parts {
        match p["type"].as_str() {
            Some("text" | "input_text" | "output_text") => {
                keys(p, &["type", "text", "annotations"])?;
                if p.get("annotations")
                    .is_some_and(|a| a.as_array().is_none_or(|a| !a.is_empty()))
                {
                    return Err(RelayError::Unsupported("annotated input content"));
                }
                out.push(InputBlock::Text(string(&p["text"])?));
            }
            Some("image_url") if role == Role::User && client == ClientProtocol::Chat => {
                out.push(InputBlock::Image(ImageInput::openai(p, client)?));
            }
            Some("input_image") if role == Role::User && client == ClientProtocol::Responses => {
                out.push(InputBlock::Image(ImageInput::openai(p, client)?));
            }
            Some("refusal") if role == Role::Assistant => {
                keys(p, &["type", "refusal"])?;
                out.push(InputBlock::Refusal(string(&p["refusal"])?));
            }
            _ => {
                return Err(RelayError::Unsupported("unsupported message content block"));
            }
        }
    }
    Ok(out)
}

impl Request {
    /// The first counter contract covers only stateless, nonstreaming text.
    pub(crate) fn validate_exact_input_count(&self) -> Result<(), RelayError> {
        // @context-fold begin exact-count-request-contract
        if self.stream
            || !self.tools.is_empty()
            || !matches!(self.tool_choice, ToolChoice::Auto)
            || self.parallel_tool_calls.is_some()
            || self.prompt_cache_options.is_some()
            || self.anthropic_cached_prompt.is_some()
            || self.reasoning_effort.is_some()
            || self.google_search
            || self.temperature.is_some()
            || self.top_p.is_some()
            || !self.stop.is_empty()
            || self.continuation_token.is_some()
            || self.image_config.is_some()
            || !self.openrouter.is_empty()
            || self.response_format.is_some()
            || !self.compatibility.is_empty()
            || self.messages.iter().any(|message| {
                message
                    .content
                    .iter()
                    .any(|block| !matches!(block, InputBlock::Text(_)))
            })
        {
            return Err(RelayError::Unsupported(
                "exact input count requires stateless text",
            ));
        }
        Ok(())
        // @context-fold end exact-count-request-contract
    }

    /// Admit only the five legacy fields provisionally. No policy is authorized
    /// until `upstream` receives the immutable route selected by the bridge.
    pub(crate) fn parse_deferred_openai(
        client: ClientProtocol,
        body: &[u8],
        limits: &Limits,
    ) -> Result<Self, RelayError> {
        Self::parse_deferred_openai_mode(client, body, limits, false, false)
    }

    /// Only the admitted native WS envelope may supply these stateful facts.
    /// This does not synthesize history/tool calls to satisfy HTTP validation.
    pub(crate) fn parse_responses_ws(
        body: &[u8],
        limits: &Limits,
        has_previous_response: bool,
        generate: Option<bool>,
    ) -> Result<Self, RelayError> {
        Self::parse_deferred_openai_mode(
            ClientProtocol::Responses,
            body,
            limits,
            has_previous_response || generate == Some(false),
            has_previous_response,
        )
    }

    fn parse_deferred_openai_mode(
        client: ClientProtocol,
        body: &[u8],
        limits: &Limits,
        allow_empty_input: bool,
        allow_previous_tool_result: bool,
    ) -> Result<Self, RelayError> {
        if !matches!(client, ClientProtocol::Chat | ClientProtocol::Responses) {
            return Self::parse(client, body, limits);
        }
        limits.validate()?;
        if body.len() > limits.request_bytes {
            return Err(RelayError::Limit("request body"));
        }
        let mut fields = serde_json::from_slice::<RequestObject>(body)
            .map_err(|_| RelayError::Invalid("invalid or ambiguous JSON body"))?
            .0;
        let mut compatibility = Map::new();
        for name in [
            "service_tier",
            "inference_geo",
            "speed",
            "safety_identifier",
        ] {
            if let Some(value) = fields.remove(name) {
                compatibility.insert(name.into(), value);
            }
        }
        if let Some(options) = fields.get_mut("stream_options").filter(|v| !v.is_null()) {
            keys(
                options,
                if client == ClientProtocol::Chat {
                    &["include_usage", "include_obfuscation"]
                } else {
                    &["include_obfuscation"]
                },
            )?;
            if let Some(value) = options
                .as_object_mut()
                .and_then(|o| o.remove("include_obfuscation"))
            {
                compatibility.insert("include_obfuscation".into(), value);
            }
            if options.as_object().is_some_and(Map::is_empty) {
                fields.remove("stream_options");
            }
        }
        let bytes =
            serde_json::to_vec(&fields).map_err(|_| RelayError::Invalid("invalid request body"))?;
        let mut request = Self::parse_mode(
            client,
            &bytes,
            limits,
            allow_empty_input,
            allow_previous_tool_result,
        )?;
        request.compatibility = compatibility;
        if matches!(client, ClientProtocol::Chat | ClientProtocol::Responses) {
            request.legacy_precharge_body = Some(std::sync::Arc::from(body));
        }
        // The compatibility parser re-encodes fields; capture only after its
        // original duplicate/depth checks, from the original ordered schemas.
        request.capture_ordered_schemas(client, body);
        Ok(request)
    }

    pub fn parse(client: ClientProtocol, body: &[u8], limits: &Limits) -> Result<Self, RelayError> {
        Self::parse_mode(client, body, limits, false, false)
    }

    fn parse_mode(
        client: ClientProtocol,
        body: &[u8],
        limits: &Limits,
        allow_empty_input: bool,
        allow_previous_tool_result: bool,
    ) -> Result<Self, RelayError> {
        limits.validate()?;
        if client == ClientProtocol::Anthropic {
            let mut request = super::native_anthropic::parse(body, limits)?;
            request.client = client;
            return Ok(request);
        }
        if client == ClientProtocol::Gemini {
            return Err(RelayError::Invalid(
                "native Gemini requires its path model and operation",
            ));
        }
        if body.len() > limits.request_bytes {
            return Err(RelayError::Limit("request body"));
        }
        let client = match client {
            ClientProtocol::Chat => ClientProtocol::Chat,
            ClientProtocol::Responses => ClientProtocol::Responses,
            _ => unreachable!("native request dispatched above"),
        };
        let v = Value::Object(
            serde_json::from_slice::<RequestObject>(body)
                .map_err(|_| RelayError::Invalid("invalid or ambiguous JSON body"))?
                .0,
        );
        match client {
            ClientProtocol::Chat => keys(
                &v,
                &[
                    "model",
                    "messages",
                    "stream",
                    "stream_options",
                    "max_tokens",
                    "max_completion_tokens",
                    "temperature",
                    "top_p",
                    "stop",
                    "tools",
                    "tool_choice",
                    "parallel_tool_calls",
                    "n",
                    "reasoning_effort",
                    "store",
                    "imageConfig",
                    "modalities",
                    "image_config",
                    "usage",
                    "reasoning",
                    "response_format",
                ],
            )?,
            ClientProtocol::Anthropic | ClientProtocol::Gemini => keys(
                &v,
                &[
                    "model",
                    "messages",
                    "stream",
                    "stream_options",
                    "max_tokens",
                    "max_completion_tokens",
                    "temperature",
                    "top_p",
                    "stop",
                    "tools",
                    "tool_choice",
                    "parallel_tool_calls",
                    "n",
                    "reasoning_effort",
                    "store",
                ],
            )?,
            ClientProtocol::Responses => keys(
                &v,
                &[
                    "model",
                    "input",
                    "instructions",
                    "stream",
                    "max_output_tokens",
                    "temperature",
                    "top_p",
                    "tools",
                    "tool_choice",
                    "parallel_tool_calls",
                    "reasoning",
                    "store",
                    "prompt_cache_options",
                    "text",
                ],
            )?,
        }
        let model = string(&v["model"])?;
        if model.is_empty() || model.len() > 256 {
            return Err(RelayError::Invalid("invalid model"));
        }
        if boolean(v.get("store"))? == Some(true) {
            return Err(RelayError::Unsupported(
                "stored responses are not implemented",
            ));
        }
        if v.get("n").is_some_and(|n| n.as_u64() != Some(1)) {
            return Err(RelayError::Unsupported("only one candidate is supported"));
        }
        let stream = boolean(v.get("stream"))?.unwrap_or(false);
        let include_usage = if let Some(opts) = v.get("stream_options").filter(|v| !v.is_null()) {
            keys(opts, &["include_usage"])?;
            boolean(opts.get("include_usage"))?.unwrap_or(false)
        } else {
            false
        };
        let token_values: Vec<&Value> =
            ["max_tokens", "max_completion_tokens", "max_output_tokens"]
                .iter()
                .filter_map(|k| v.get(k))
                .filter(|v| !v.is_null())
                .collect();
        if token_values.len() > 1 {
            return Err(RelayError::Invalid("conflicting output limits"));
        }
        let explicit_max_output_tokens = token_values
            .first()
            .map(|n| {
                n.as_u64()
                    .and_then(|n| u32::try_from(n).ok())
                    .filter(|n| *n > 0 && *n <= limits.max_output_tokens)
                    .ok_or(RelayError::Invalid("invalid output token limit"))
            })
            .transpose()?;
        let max_output_tokens =
            explicit_max_output_tokens.unwrap_or(4096.min(limits.max_output_tokens));
        let mut stop = Vec::new();
        if let Some(s) = v.get("stop").filter(|s| !s.is_null()) {
            if let Some(s) = s.as_str() {
                stop.push(s.to_owned());
            } else {
                for s in s
                    .as_array()
                    .ok_or(RelayError::Invalid("invalid stop sequences"))?
                {
                    stop.push(string(s)?);
                }
            }
        }
        if stop.len() > 4 || stop.iter().any(|s| s.is_empty() || s.len() > 1024) {
            return Err(RelayError::Invalid("invalid stop sequences"));
        }
        let mut tools = Vec::new();
        let mut tool_names = BTreeSet::new();
        if let Some(raw) = v.get("tools").filter(|v| !v.is_null()) {
            for t in raw.as_array().ok_or(RelayError::Invalid("invalid tools"))? {
                if t["type"] != "function" {
                    return Err(RelayError::Unsupported("only function tools are supported"));
                }
                let f = match client {
                    ClientProtocol::Chat | ClientProtocol::Anthropic | ClientProtocol::Gemini => {
                        keys(t, &["type", "function"])?;
                        &t["function"]
                    }
                    ClientProtocol::Responses => t,
                };
                keys(f, &["type", "name", "description", "parameters", "strict"])?;
                let name = name(&f["name"])?;
                if !tool_names.insert(name.clone()) {
                    return Err(RelayError::Invalid("duplicate function name"));
                }
                let parameters = f
                    .get("parameters")
                    .cloned()
                    .unwrap_or_else(|| json!({"type":"object","properties":{}}));
                if !parameters.is_object() {
                    return Err(RelayError::Invalid("function parameters must be an object"));
                }
                let ordered_parameters = serde_json::to_string(&parameters)
                    .map_err(|_| RelayError::Invalid("function schema encoding failed"))?;
                tools.push(FunctionTool {
                    name,
                    parameters,
                    description: f.get("description").map(string).transpose()?,
                    strict: boolean(f.get("strict"))?,
                    ordered_parameters,
                });
            }
        }
        if tools.len() > limits.max_blocks {
            return Err(RelayError::Limit("too many tools"));
        }
        let tool_choice = match v.get("tool_choice").filter(|v| !v.is_null()) {
            None => ToolChoice::Auto,
            Some(Value::String(s)) => match s.as_str() {
                "auto" => ToolChoice::Auto,
                "none" => ToolChoice::None,
                "required" => ToolChoice::Required,
                _ => return Err(RelayError::Unsupported("unsupported tool choice")),
            },
            Some(t) => {
                if t["type"] != "function" {
                    return Err(RelayError::Unsupported("unsupported tool choice"));
                }
                let n = match client {
                    ClientProtocol::Chat | ClientProtocol::Anthropic | ClientProtocol::Gemini => {
                        keys(t, &["type", "function"])?;
                        keys(&t["function"], &["name"])?;
                        &t["function"]["name"]
                    }
                    ClientProtocol::Responses => {
                        keys(t, &["type", "name"])?;
                        &t["name"]
                    }
                };
                let n = name(n)?;
                if !tool_names.contains(&n) {
                    return Err(RelayError::Invalid("tool choice names an unknown function"));
                }
                ToolChoice::Named(n)
            }
        };
        if tools.is_empty() && matches!(tool_choice, ToolChoice::Required | ToolChoice::Named(_)) {
            return Err(RelayError::Invalid("tool choice requires tools"));
        }
        let reasoning_effort = match client {
            ClientProtocol::Chat | ClientProtocol::Anthropic | ClientProtocol::Gemini => v
                .get("reasoning_effort")
                .filter(|v| !v.is_null())
                .map(string)
                .transpose()?,
            ClientProtocol::Responses => {
                if let Some(r) = v.get("reasoning").filter(|v| !v.is_null()) {
                    keys(r, &["effort"])?;
                    r.get("effort").map(string).transpose()?
                } else {
                    None
                }
            }
        };
        if reasoning_effort.as_ref().is_some_and(|s| {
            !["none", "minimal", "low", "medium", "high", "xhigh", "max"].contains(&s.as_str())
        }) {
            return Err(RelayError::Unsupported("unsupported reasoning effort"));
        }
        // Keep the requested effort intact. DeepSeek accepts aliases such as
        // xhigh, but other OpenAI-compatible providers give them distinct
        // meanings. The selected provider interprets its own vocabulary.
        let mut messages = Vec::new();
        if let Some(s) = v.get("instructions").filter(|v| !v.is_null()) {
            messages.push(Message {
                role: Role::System,
                content: vec![InputBlock::Text(string(s)?)],
            });
        }
        match client {
            ClientProtocol::Chat | ClientProtocol::Anthropic | ClientProtocol::Gemini => {
                for m in v["messages"]
                    .as_array()
                    .ok_or(RelayError::Invalid("messages must be an array"))?
                {
                    keys(
                        m,
                        &[
                            "role",
                            "content",
                            "tool_calls",
                            "tool_call_id",
                            "reasoning_content",
                            "provider_metadata",
                            "refusal",
                        ],
                    )?;
                    let role = Role::parse(&m["role"])?;
                    if role == Role::Tool {
                        keys(m, &["role", "content", "tool_call_id"])?;
                    } else if m.get("tool_call_id").is_some() {
                        return Err(RelayError::Invalid("tool_call_id requires tool role"));
                    }
                    let mut content = Vec::new();
                    if role == Role::Tool {
                        content.push(InputBlock::ToolResult {
                            id: name(&m["tool_call_id"])?,
                            content: string(&m["content"])?,
                        });
                    } else {
                        if let Some(t) = m.get("reasoning_content").filter(|v| !v.is_null()) {
                            if role != Role::Assistant {
                                return Err(RelayError::Invalid(
                                    "reasoning requires assistant role",
                                ));
                            }
                            content.push(InputBlock::Thinking {
                                text: string(t)?,
                                signature: signature(m.get("provider_metadata"))?,
                            });
                        } else if m.get("provider_metadata").is_some() {
                            return Err(RelayError::Invalid("signature without reasoning content"));
                        }
                        content.extend(text_blocks(&m["content"], role, client)?);
                        if let Some(t) = m.get("refusal").filter(|v| !v.is_null()) {
                            if role != Role::Assistant {
                                return Err(RelayError::Invalid("refusal requires assistant role"));
                            }
                            content.push(InputBlock::Refusal(string(t)?));
                        }
                    }
                    if let Some(calls) = m.get("tool_calls").filter(|v| !v.is_null()) {
                        if role != Role::Assistant {
                            return Err(RelayError::Invalid("tool calls require assistant role"));
                        }
                        for c in calls
                            .as_array()
                            .ok_or(RelayError::Invalid("invalid tool calls"))?
                        {
                            keys(c, &["id", "type", "function", "provider_metadata"])?;
                            keys(&c["function"], &["name", "arguments"])?;
                            if c["type"] != "function" {
                                return Err(RelayError::Unsupported("unsupported tool call"));
                            }
                            content.push(InputBlock::ToolCall {
                                id: name(&c["id"])?,
                                name: name(&c["function"]["name"])?,
                                arguments: arguments(
                                    &c["function"]["arguments"],
                                    limits.tool_argument_bytes,
                                )?,
                                signature: signature(c.get("provider_metadata"))?,
                            });
                        }
                    }
                    messages.push(Message { role, content });
                }
            }
            ClientProtocol::Responses => {
                let input = &v["input"];
                if let Some(s) = input.as_str() {
                    messages.push(Message {
                        role: Role::User,
                        content: vec![InputBlock::Text(s.into())],
                    });
                } else {
                    let empty = Vec::new();
                    let items = if allow_empty_input && v.get("input").is_none() {
                        &empty
                    } else {
                        input
                            .as_array()
                            .ok_or(RelayError::Invalid("input must be a string or array"))?
                    };
                    for i in items {
                        let kind = match i.get("type") {
                            None => "message",
                            Some(kind) => kind
                                .as_str()
                                .ok_or(RelayError::Invalid("invalid Responses input type"))?,
                        };
                        match kind {
                            "message" => {
                                keys(i, &["type", "role", "content"])?;
                                let role = Role::parse(&i["role"])?;
                                if role == Role::Tool {
                                    return Err(RelayError::Invalid("use function_call_output"));
                                }
                                messages.push(Message {
                                    role,
                                    content: text_blocks(&i["content"], role, client)?,
                                });
                            }
                            "function_call" => {
                                keys(
                                    i,
                                    &["type", "call_id", "name", "arguments", "provider_metadata"],
                                )?;
                                messages.push(Message {
                                    role: Role::Assistant,
                                    content: vec![InputBlock::ToolCall {
                                        id: name(&i["call_id"])?,
                                        name: name(&i["name"])?,
                                        arguments: arguments(
                                            &i["arguments"],
                                            limits.tool_argument_bytes,
                                        )?,
                                        signature: signature(i.get("provider_metadata"))?,
                                    }],
                                });
                            }
                            "function_call_output" => {
                                keys(i, &["type", "call_id", "output"])?;
                                messages.push(Message {
                                    role: Role::Tool,
                                    content: vec![InputBlock::ToolResult {
                                        id: name(&i["call_id"])?,
                                        content: string(&i["output"])?,
                                    }],
                                });
                            }
                            _ => {
                                return Err(RelayError::Unsupported(
                                    "stateful, encrypted or non-text Responses input",
                                ));
                            }
                        }
                    }
                }
            }
        }
        if (messages.is_empty() && !allow_empty_input) || messages.len() > 1024 {
            return Err(RelayError::Limit("message count"));
        }
        let mut calls = BTreeSet::new();
        let mut results = BTreeSet::new();
        let mut count = 0usize;
        for m in &messages {
            for b in &m.content {
                count += 1;
                match b {
                    InputBlock::ToolCall { id, .. } if !calls.insert(id) => {
                        return Err(RelayError::Invalid("duplicate call ID"));
                    }
                    InputBlock::ToolResult { id, .. }
                        if (!calls.contains(id) && !allow_previous_tool_result)
                            || !results.insert(id) =>
                    {
                        return Err(RelayError::Invalid("orphan or duplicate tool result"));
                    }
                    _ => {}
                }
            }
        }
        if count > 4096 {
            return Err(RelayError::Limit("input block count"));
        }
        let image_config = if client == ClientProtocol::Chat {
            chat_image_config(&v)?
        } else {
            None
        };
        let prompt_cache_options = v
            .get("prompt_cache_options")
            .filter(|value| !value.is_null())
            .map(|options| {
                keys(options, &["mode", "ttl"])?;
                if options
                    .get("mode")
                    .is_some_and(|mode| mode.as_str() != Some("implicit"))
                {
                    return Err(RelayError::Unsupported(
                        "explicit cache breakpoints are not supported",
                    ));
                }
                if !matches!(
                    options.get("ttl").and_then(Value::as_str),
                    Some("30m" | "12h")
                ) {
                    return Err(RelayError::Invalid("cache lifetime must be 30m or 12h"));
                }
                Ok(options.clone())
            })
            .transpose()?;
        let mut request = Self {
            client,
            model,
            stream,
            max_output_tokens,
            explicit_max_output_tokens,
            legacy_precharge_body: matches!(
                client,
                ClientProtocol::Chat | ClientProtocol::Responses
            )
            .then(|| std::sync::Arc::from(body)),
            messages,
            tools,
            stop,
            tool_choice,
            include_usage,
            temperature: number(v.get("temperature"), 2.0)?,
            top_p: number(v.get("top_p"), 1.0)?,
            parallel_tool_calls: boolean(v.get("parallel_tool_calls"))?,
            reasoning_effort,
            google_search: false,
            prompt_cache_options,
            anthropic_cached_prompt: None,
            image_config,
            openrouter: if client == ClientProtocol::Chat {
                openrouter_options(&v)?
            } else {
                Map::new()
            },
            continuation_token: None,
            compatibility: Map::new(),
            response_format: if client == ClientProtocol::Responses {
                responses_text_format(&v)?
            } else {
                response_format(&v)?
            },
        };
        request.capture_ordered_schemas(client, body);
        Ok(request)
    }

    fn capture_ordered_schemas(&mut self, client: ClientProtocol, body: &[u8]) {
        let captured = match client {
            ClientProtocol::Chat => super::schema_order::chat_schemas(body),
            ClientProtocol::Responses => super::schema_order::responses_schemas(body),
            ClientProtocol::Anthropic | ClientProtocol::Gemini => None,
        };
        if let Some(captured) = captured {
            self.assign_ordered_schemas(&captured);
        }
        let response_schema = match client {
            ClientProtocol::Chat => super::schema_order::chat_response_schema(body),
            ClientProtocol::Responses => super::schema_order::responses_response_schema(body),
            ClientProtocol::Anthropic | ClientProtocol::Gemini => None,
        };
        if let Some(raw) = response_schema
            && let Some(format) = self.response_format.as_mut()
        {
            format.adopt_ordered_schema(&raw);
        }
    }

    pub(crate) fn assign_ordered_schemas(
        &mut self,
        captured: &[Option<super::schema_order::RawSchema>],
    ) {
        if captured.len() != self.tools.len() {
            return;
        }
        for (tool, item) in self.tools.iter_mut().zip(captured) {
            let Some(item) = item else { continue };
            let ordered = match item {
                super::schema_order::RawSchema::Json(raw) => {
                    super::schema_order::ordered_json_matching(raw, &tool.parameters)
                }
                super::schema_order::RawSchema::GeminiOpenApi(raw) => {
                    super::schema_order::ordered_gemini_schema(raw, &tool.parameters).or_else(
                        || super::schema_order::ordered_inbound_json_schema(raw, &tool.parameters),
                    )
                }
            };
            if let Some(text) = ordered {
                tool.ordered_parameters = text;
            }
        }
    }

    /// Encode only after applying the immutable selected-route payload policy.
    pub(crate) fn upstream_body(&self, route: &Route) -> Result<Vec<u8>, RelayError> {
        let payload = self.upstream(route)?;
        self.ordered_payload_body(&payload, route.protocol)
    }

    /// Strict protocol/model encoding for the isolated uncharged transport.
    /// It cannot authorize deferred compatibility fields or accept a Route.
    pub(crate) fn upstream_body_to(
        &self,
        protocol: UpstreamProtocol,
        model: &str,
    ) -> Result<Vec<u8>, RelayError> {
        let payload = self.upstream_to(protocol, model)?;
        self.ordered_payload_body(&payload, protocol)
    }

    fn ordered_payload_body(
        &self,
        payload: &Value,
        protocol: UpstreamProtocol,
    ) -> Result<Vec<u8>, RelayError> {
        let schemas: Vec<String> = self
            .tools
            .iter()
            .map(|tool| tool.ordered_parameters.clone())
            .collect();
        super::schema_order::encode_ordered_schemas(
            payload,
            protocol,
            &schemas,
            self.response_format
                .as_ref()
                .and_then(ResponseFormat::ordered_schema),
        )
    }

    pub(crate) fn upstream(&self, route: &Route) -> Result<Value, RelayError> {
        use crate::routing::PayloadPolicy;
        if route.payload_policy == PayloadPolicy::LegacyAnthropicHaikuSystemCacheV1 {
            if route.protocol != UpstreamProtocol::Anthropic {
                return Err(RelayError::Unsupported(
                    "legacy system cache policy requires Anthropic",
                ));
            }
            let mut payload = self.upstream_to(route.protocol, &route.model)?;
            if self.client == ClientProtocol::Chat
                && route.model.starts_with("claude-haiku-4-5")
                && let Some(last) = payload
                    .get_mut("system")
                    .and_then(Value::as_array_mut)
                    .and_then(|system| system.last_mut())
                    .and_then(Value::as_object_mut)
            {
                last.entry("cache_control")
                    .or_insert_with(|| json!({"type":"ephemeral"}));
            }
            return Ok(payload);
        }
        if route.payload_policy.openrouter_chat() {
            if !self.compatibility.is_empty() {
                return Err(RelayError::Unsupported(
                    "legacy OpenAI fields are not OpenRouter controls",
                ));
            }
            if self.client != ClientProtocol::Chat || route.protocol != UpstreamProtocol::Chat {
                return Err(RelayError::Unsupported("OpenRouter contract requires Chat"));
            }
            let mut payload = self.encode_upstream(route.protocol, &route.model)?;
            let object = payload.as_object_mut().unwrap();
            object.remove("stream_options");
            object.remove("reasoning_effort");
            object.insert("usage".into(), json!({"include":true}));
            if let Some(effort) = &self.reasoning_effort {
                if effort != "none" {
                    object.insert("reasoning".into(), json!({"enabled":true,"effort":effort}));
                }
            }
            object.extend(self.openrouter.clone());
            return Ok(payload);
        }
        if !self.openrouter.is_empty() {
            return Err(RelayError::Unsupported(
                "OpenRouter fields require its selected route contract",
            ));
        }
        let PayloadPolicy::LegacyOpenai {
            allow_service_tier,
            allow_inference_geo,
            allow_speed,
            allow_safety_identifier,
            allow_include_obfuscation,
        } = route.payload_policy
        else {
            return self.upstream_to(route.protocol, &route.model);
        };
        let same_wire = matches!(
            (self.client, route.protocol),
            (ClientProtocol::Chat, UpstreamProtocol::Chat)
                | (ClientProtocol::Responses, UpstreamProtocol::Responses)
        );
        if !same_wire {
            return Err(RelayError::Unsupported(
                "legacy OpenAI policy requires the same wire protocol",
            ));
        }
        let mut payload = self.encode_upstream(route.protocol, &route.model)?;
        for (name, allowed) in [
            ("service_tier", allow_service_tier),
            ("inference_geo", allow_inference_geo),
            ("speed", allow_speed),
            ("safety_identifier", allow_safety_identifier),
            ("include_obfuscation", allow_include_obfuscation),
        ] {
            let Some(value) = self.compatibility.get(name).filter(|_| allowed) else {
                continue;
            };
            if name == "include_obfuscation" {
                if !self.stream {
                    return Err(RelayError::Unsupported("stream options require streaming"));
                }
                if !value.is_boolean() {
                    return Err(RelayError::Invalid("include_obfuscation must be a boolean"));
                }
                if payload.get("stream_options").is_none() {
                    payload["stream_options"] = json!({});
                }
                payload["stream_options"][name] = value.clone();
            } else {
                // The pinned OpenAI contract permits explicit null for these fields.
                if value.is_null() && matches!(name, "service_tier" | "safety_identifier") {
                    payload[name] = Value::Null;
                    continue;
                }
                // Regional/speed values are provider extensions: bound the string
                // without inventing or translating a provider's enum vocabulary.
                let limit = if name == "safety_identifier" {
                    128
                } else {
                    256
                };
                let text = value
                    .as_str()
                    .filter(|s| {
                        !s.is_empty()
                            && s.chars().count() <= limit
                            && !s.chars().any(char::is_control)
                    })
                    .ok_or(RelayError::Invalid("invalid legacy OpenAI string field"))?;
                // This deployment has only standard prices. Selected provider
                // compatibility never authorizes an unpriced acceleration tier.
                if (name == "service_tier" && text != "default")
                    || (name == "speed" && text != "standard")
                {
                    return Err(RelayError::Unsupported(
                        "only standard service tier is enabled",
                    ));
                }
                payload[name] = value.clone();
            }
        }
        Ok(payload)
    }

    /// Encode the existing bounded request for a server-owned protocol/model.
    /// This carries neither routing prices nor spending authority.
    pub(crate) fn upstream_to(
        &self,
        protocol: UpstreamProtocol,
        model: &str,
    ) -> Result<Value, RelayError> {
        if !self.compatibility.is_empty() || !self.openrouter.is_empty() {
            return Err(RelayError::Unsupported(
                "legacy fields require an explicit selected route policy",
            ));
        }
        self.encode_upstream(protocol, model)
    }

    fn encode_upstream(
        &self,
        protocol: UpstreamProtocol,
        model: &str,
    ) -> Result<Value, RelayError> {
        if self.anthropic_cached_prompt.is_some() && protocol != UpstreamProtocol::Anthropic {
            return Err(RelayError::Unsupported(
                "Anthropic cache_control requires an Anthropic route",
            ));
        }
        if self.google_search
            && (self.client != ClientProtocol::Gemini
                || protocol != UpstreamProtocol::Gemini
                || !self.tools.is_empty()
                || self.continuation_token.is_some())
        {
            return Err(RelayError::Unsupported(
                "Google Search requires a native Gemini request without mixed tools or continuation",
            ));
        }
        if self.prompt_cache_options.is_some() && protocol != UpstreamProtocol::Responses {
            return Err(RelayError::Unsupported(
                "cache lifetime controls require a Responses route",
            ));
        }
        if self.continuation_token.is_some() && protocol != UpstreamProtocol::Gemini {
            return Err(RelayError::Unsupported(
                "Gemini continuation tokens require a Gemini route",
            ));
        }
        for m in &self.messages {
            for b in &m.content {
                if let InputBlock::Image(image) = b {
                    image.wire(protocol)?;
                }
                if protocol == UpstreamProtocol::Anthropic
                    && matches!(
                        b,
                        InputBlock::ToolCall {
                            signature: Some(_),
                            ..
                        }
                    )
                {
                    return Err(RelayError::Unsupported(
                        "Anthropic tool history cannot carry standalone signatures",
                    ));
                }
                let sig = match b {
                    InputBlock::Thinking { signature, .. }
                    | InputBlock::ToolCall { signature, .. } => signature,
                    _ => continue,
                };
                if sig.as_ref().is_some_and(|(p, _)| *p != protocol) {
                    return Err(RelayError::Unsupported(
                        "provider signatures cannot be replayed to a different protocol",
                    ));
                }
            }
        }
        if self.image_config.is_some() && protocol != UpstreamProtocol::Gemini {
            return Err(RelayError::Unsupported(
                "imageConfig requires a Gemini route",
            ));
        }
        if self.response_format.is_some()
            && !matches!(
                protocol,
                UpstreamProtocol::Chat | UpstreamProtocol::Responses | UpstreamProtocol::Gemini
            )
        {
            return Err(RelayError::Unsupported(
                "response_format requires a Chat, Responses or Gemini route",
            ));
        }
        if matches!(
            protocol,
            UpstreamProtocol::Anthropic | UpstreamProtocol::Gemini
        ) && (self.reasoning_effort.is_some()
            || self.tools.iter().any(|t| t.strict == Some(true)))
        {
            return Err(RelayError::Unsupported(
                "reasoning effort or strict tools require an OpenAI protocol route",
            ));
        }
        if protocol == UpstreamProtocol::Responses && !self.stop.is_empty() {
            return Err(RelayError::Unsupported("Responses stop sequences"));
        }
        if protocol == UpstreamProtocol::Anthropic && self.temperature.is_some_and(|t| t > 1.0) {
            return Err(RelayError::Unsupported("Anthropic temperature exceeds one"));
        }
        if protocol == UpstreamProtocol::Gemini && self.parallel_tool_calls == Some(false) {
            return Err(RelayError::Unsupported(
                "Gemini cannot enforce serial tool calls",
            ));
        }
        if matches!(
            protocol,
            UpstreamProtocol::Anthropic | UpstreamProtocol::Gemini
        ) {
            self.validate_system()?;
        }
        for block in self.messages.iter().flat_map(|message| &message.content) {
            match block {
                InputBlock::Thinking { signature, .. } => match protocol {
                    UpstreamProtocol::Responses => {
                        return Err(RelayError::Unsupported(
                            "Responses reasoning history requires native item identity",
                        ));
                    }
                    UpstreamProtocol::Chat if signature.is_some() => {
                        return Err(RelayError::Unsupported("opaque Chat reasoning signatures"));
                    }
                    UpstreamProtocol::Anthropic | UpstreamProtocol::Gemini
                        if signature.is_none() =>
                    {
                        return Err(RelayError::Unsupported(
                            "native thinking history needs its original signature",
                        ));
                    }
                    _ => {}
                },
                InputBlock::ToolCall {
                    signature: Some(_), ..
                } if matches!(
                    protocol,
                    UpstreamProtocol::Chat | UpstreamProtocol::Responses
                ) =>
                {
                    return Err(RelayError::Unsupported("opaque OpenAI tool signatures"));
                }
                _ => {}
            }
        }
        let mut converted = super::zconv_request::encode(self, protocol, model)?;
        // Core owns the selected supplier's controls and schema dialect policy.
        // Message bodies, tool definitions and tool choice come from zconv IR.
        if protocol == UpstreamProtocol::Chat {
            if let Some(format) = &self.response_format {
                converted["response_format"] = format.to_chat_value();
            }
            if self.stream {
                converted["stream_options"] = json!({"include_usage":true});
            }
        }
        if matches!(
            protocol,
            UpstreamProtocol::Chat | UpstreamProtocol::Responses
        ) {
            converted["store"] = json!(false);
        }
        if protocol == UpstreamProtocol::Responses {
            if let Some(format) = &self.response_format {
                converted["text"] = json!({"format": format.to_responses_value()});
            }
            if let Some(options) = &self.prompt_cache_options {
                converted["prompt_cache_options"] = options.clone();
            }
        }
        if protocol == UpstreamProtocol::Anthropic {
            // No system section is emitted for absence. The native parser
            // rejects an explicitly empty content array; do not invent one.
            if let Some(prompt) = &self.anthropic_cached_prompt {
                for (key, value) in prompt {
                    converted[key] = value.clone();
                }
            }
        }
        if protocol == UpstreamProtocol::Gemini {
            if let Some(image_config) = &self.image_config {
                converted["generationConfig"]["imageConfig"] = image_config.to_gemini_value();
            }
            if let Some(format) = &self.response_format {
                format.apply_gemini(&mut converted["generationConfig"], self.client);
            }
            if let Some(token) = &self.continuation_token {
                converted["continuationToken"] = json!(token);
            }
            if self.google_search {
                converted["tools"] = json!([{"googleSearch":{}}]);
            }
        }
        Ok(converted)
    }
    // Frozen legacy writers are test oracles only. Production encodes from IR.
    #[cfg(test)]
    fn openai_tools(&self, chat: bool) -> Vec<Value> {
        self.tools
            .iter()
            .map(|t| {
                let mut v = json!({"name":t.name,"parameters":t.parameters});
                if let Some(d) = &t.description {
                    v["description"] = json!(d);
                }
                if let Some(s) = t.strict {
                    v["strict"] = json!(s);
                }
                if chat {
                    json!({"type":"function","function":v})
                } else {
                    v["type"] = json!("function");
                    v
                }
            })
            .collect()
    }
    #[cfg(test)]
    fn openai_choice(&self, chat: bool) -> Value {
        match &self.tool_choice {
            ToolChoice::Auto => json!("auto"),
            ToolChoice::None => json!("none"),
            ToolChoice::Required => json!("required"),
            ToolChoice::Named(n) => {
                if chat {
                    json!({"type":"function","function":{"name":n}})
                } else {
                    json!({"type":"function","name":n})
                }
            }
        }
    }
    #[cfg(test)]
    fn sampling(&self, v: &mut Value) {
        if let Some(n) = self.temperature {
            v["temperature"] = json!(n);
        }
        if let Some(n) = self.top_p {
            v["top_p"] = json!(n);
        }
    }
    #[cfg(test)]
    fn chat(&self, model: &str) -> Result<Value, RelayError> {
        let mut messages = Vec::new();
        for m in &self.messages {
            let mut v = json!({"role":m.role.name(),"content":Value::Null});
            let has_images = m
                .content
                .iter()
                .any(|block| matches!(block, InputBlock::Image(_)));
            let mut text = String::new();
            let mut parts = Vec::new();
            let mut thoughts = String::new();
            let mut refusals = String::new();
            let mut calls = Vec::new();
            for b in &m.content {
                match b {
                    InputBlock::Text(s) => {
                        if has_images {
                            parts.push(json!({"type":"text", "text":s}));
                        } else {
                            text.push_str(s);
                        }
                    }
                    InputBlock::Image(image) => parts.push(image.wire(UpstreamProtocol::Chat)?),
                    InputBlock::Refusal(s) => refusals.push_str(s),
                    InputBlock::Thinking { text, signature } => {
                        thoughts.push_str(text);
                        if signature.is_some() {
                            return Err(RelayError::Unsupported(
                                "opaque Chat reasoning signatures",
                            ));
                        }
                    }
                    InputBlock::ToolCall {
                        id,
                        name,
                        arguments,
                        signature,
                    } => {
                        if signature.is_some() {
                            return Err(RelayError::Unsupported("opaque Chat tool signatures"));
                        }
                        calls.push(json!({"id":id,"type":"function","function":{"name":name,"arguments":arguments}}));
                    }
                    InputBlock::ToolResult { id, content } => {
                        v["tool_call_id"] = json!(id);
                        text.push_str(content);
                    }
                }
            }
            if has_images {
                v["content"] = json!(parts);
            } else if !text.is_empty() || m.role != Role::Assistant {
                v["content"] = json!(text);
            }
            if !thoughts.is_empty() {
                v["reasoning_content"] = json!(thoughts);
            }
            if !refusals.is_empty() {
                v["refusal"] = json!(refusals);
            }
            if !calls.is_empty() {
                v["tool_calls"] = json!(calls);
            }
            messages.push(v);
        }
        let mut v = json!({"model":model,"messages":messages,"stream":self.stream,"max_completion_tokens":self.max_output_tokens,"store":false});
        if self.stream {
            v["stream_options"] = json!({"include_usage":true});
        }
        if !self.tools.is_empty() {
            v["tools"] = json!(self.openai_tools(true));
            v["tool_choice"] = self.openai_choice(true);
        }
        if let Some(p) = self.parallel_tool_calls {
            v["parallel_tool_calls"] = json!(p);
        }
        if let Some(e) = &self.reasoning_effort {
            v["reasoning_effort"] = json!(e);
        }
        if !self.stop.is_empty() {
            v["stop"] = json!(self.stop);
        }
        if let Some(format) = &self.response_format {
            v["response_format"] = format.to_chat_value();
        }
        self.sampling(&mut v);
        Ok(v)
    }
    #[cfg(test)]
    fn responses(&self, model: &str) -> Result<Value, RelayError> {
        if !self.stop.is_empty() {
            return Err(RelayError::Unsupported("Responses stop sequences"));
        }
        let mut input = Vec::new();
        for m in &self.messages {
            if m.content
                .iter()
                .any(|block| matches!(block, InputBlock::Image(_)))
            {
                let content = m
                    .content
                    .iter()
                    .map(|block| match block {
                        InputBlock::Text(text) => Ok(json!({"type":"input_text", "text":text})),
                        InputBlock::Image(image) => image.wire(UpstreamProtocol::Responses),
                        _ => Err(RelayError::Unsupported("unsupported image message content")),
                    })
                    .collect::<Result<Vec<_>, _>>()?;
                input.push(json!({"role":m.role.name(), "content":content}));
                continue;
            }
            for b in &m.content {
                match b {
            InputBlock::Text(s) => input.push(json!({"role":m.role.name(),"content":[{"type":if m.role == Role::Assistant {"output_text"} else {"input_text"},"text":s}]})),
            InputBlock::Image(_) => unreachable!("image messages converted above"),
            InputBlock::Refusal(s) => input.push(json!({"role":"assistant","content":[{"type":"refusal","refusal":s}]})),
            InputBlock::Thinking { .. } => return Err(RelayError::Unsupported("Responses reasoning history requires native item identity")),
            InputBlock::ToolCall { id, name, arguments, signature } => {
                if signature.is_some() { return Err(RelayError::Unsupported("opaque Responses tool signatures")); }
                input.push(json!({"type":"function_call","call_id":id,"name":name,"arguments":arguments}));
            }
            InputBlock::ToolResult { id, content } => input.push(json!({"type":"function_call_output","call_id":id,"output":content})),
        }
            }
        }
        let mut v = json!({"model":model,"input":input,"stream":self.stream,"max_output_tokens":self.max_output_tokens,"store":false});
        if let Some(options) = &self.prompt_cache_options {
            v["prompt_cache_options"] = options.clone();
        }
        if !self.tools.is_empty() {
            v["tools"] = json!(self.openai_tools(false));
            v["tool_choice"] = self.openai_choice(false);
        }
        if let Some(p) = self.parallel_tool_calls {
            v["parallel_tool_calls"] = json!(p);
        }
        if let Some(e) = &self.reasoning_effort {
            v["reasoning"] = json!({"effort":e});
        }
        self.sampling(&mut v);
        Ok(v)
    }
    fn validate_system(&self) -> Result<(), RelayError> {
        let mut conversation = false;
        for message in &self.messages {
            if matches!(message.role, Role::System | Role::Developer) {
                if conversation {
                    return Err(RelayError::Unsupported(
                        "late system instructions cannot be reordered",
                    ));
                }
                if message
                    .content
                    .iter()
                    .any(|block| !matches!(block, InputBlock::Text(_)))
                {
                    return Err(RelayError::Unsupported("non-text system instruction"));
                }
            } else {
                conversation = true;
            }
        }
        Ok(())
    }
    #[cfg(test)]
    fn split_system(&self) -> Result<(Vec<Value>, Vec<&Message>), RelayError> {
        let mut system = Vec::new();
        let mut messages = Vec::new();
        for m in &self.messages {
            if matches!(m.role, Role::System | Role::Developer) {
                if !messages.is_empty() {
                    return Err(RelayError::Unsupported(
                        "late system instructions cannot be reordered",
                    ));
                }
                for b in &m.content {
                    match b {
                        InputBlock::Text(s) => system.push(json!({"type":"text","text":s})),
                        _ => return Err(RelayError::Unsupported("non-text system instruction")),
                    }
                }
            } else {
                messages.push(m);
            }
        }
        Ok((system, messages))
    }
    #[cfg(test)]
    fn anthropic(&self, model: &str) -> Result<Value, RelayError> {
        if self.temperature.is_some_and(|t| t > 1.0) {
            return Err(RelayError::Unsupported("Anthropic temperature exceeds one"));
        }
        let (system, source) = self.split_system()?;
        let mut messages: Vec<(Role, Vec<Value>)> = Vec::new();
        for m in source {
            let mut content = Vec::new();
            for b in &m.content {
                content.push(match b {
                InputBlock::Text(s) | InputBlock::Refusal(s) => json!({"type":"text","text":s}),
                InputBlock::Image(image) => image.wire(UpstreamProtocol::Anthropic)?,
                InputBlock::Thinking { text, signature: Some((UpstreamProtocol::Anthropic, s)) } => json!({"type":"thinking","thinking":text,"signature":s}),
                InputBlock::Thinking { .. } => return Err(RelayError::Unsupported("Anthropic thinking history needs its original signature")),
                InputBlock::ToolCall { id, name, arguments, .. } => json!({"type":"tool_use","id":id,"name":name,"input":serde_json::from_str::<Value>(arguments).map_err(|_| RelayError::Invalid("invalid tool JSON"))?}),
                InputBlock::ToolResult { id, content } => json!({"type":"tool_result","tool_use_id":id,"content":content}),
            });
            }
            let role = if m.role == Role::Assistant {
                Role::Assistant
            } else {
                Role::User
            };
            if let Some((last_role, last_content)) = messages.last_mut()
                && *last_role == role
            {
                last_content.extend(content);
            } else {
                messages.push((role, content));
            }
        }
        let messages = messages
            .into_iter()
            .map(|(role, content)| {
                json!({"role":if role == Role::Assistant {"assistant"} else {"user"},"content":content})
            })
            .collect::<Vec<_>>();
        let mut v = json!({"model":model,"system":system,"messages":messages,"stream":self.stream,"max_tokens":self.max_output_tokens});
        if !self.tools.is_empty() {
            v["tools"] = json!(
                self.tools
                    .iter()
                    .map(|t| {
                        let mut v = json!({"name":t.name,"input_schema":t.parameters});
                        if let Some(d) = &t.description {
                            v["description"] = json!(d);
                        }
                        v
                    })
                    .collect::<Vec<_>>()
            );
            v["tool_choice"] = match &self.tool_choice {
                ToolChoice::Auto => json!({"type":"auto"}),
                ToolChoice::None => json!({"type":"none"}),
                ToolChoice::Required => json!({"type":"any"}),
                ToolChoice::Named(n) => json!({"type":"tool","name":n}),
            };
            if let Some(p) = self.parallel_tool_calls {
                v["tool_choice"]["disable_parallel_tool_use"] = json!(!p);
            }
        }
        if !self.stop.is_empty() {
            v["stop_sequences"] = json!(self.stop);
        }
        self.sampling(&mut v);
        if let Some(prompt) = &self.anthropic_cached_prompt {
            for (key, value) in prompt {
                v[key] = value.clone();
            }
        }
        Ok(v)
    }
    #[cfg(test)]
    fn gemini(&self) -> Result<Value, RelayError> {
        if self.parallel_tool_calls == Some(false) {
            return Err(RelayError::Unsupported(
                "Gemini cannot enforce serial tool calls",
            ));
        }
        let (system, source) = self.split_system()?;
        let mut calls = BTreeMap::new();
        let mut contents = Vec::new();
        for m in source {
            let mut parts = Vec::new();
            for b in &m.content {
                parts.push(match b {
                InputBlock::Text(s) | InputBlock::Refusal(s) => json!({"text":s}),
                InputBlock::Image(image) => image.wire(UpstreamProtocol::Gemini)?,
                InputBlock::Thinking { text, signature: Some((UpstreamProtocol::Gemini, s)) } => json!({"text":text,"thought":true,"thoughtSignature":s}),
                InputBlock::Thinking { .. } => return Err(RelayError::Unsupported("Gemini thinking history needs its original signature")),
                InputBlock::ToolCall { id, name, arguments, signature } => {
                    calls.insert(id, name);
                    let mut p = json!({"functionCall":{"id":id,"name":name,"args":serde_json::from_str::<Value>(arguments).map_err(|_| RelayError::Invalid("invalid tool JSON"))?}});
                    if let Some((_, s)) = signature { p["thoughtSignature"] = json!(s); } p
                }
                InputBlock::ToolResult { id, content } => {
                    let name = calls.get(id).ok_or(RelayError::Invalid("Gemini tool result requires its original call"))?;
                    let result = serde_json::from_str::<Value>(content).ok().filter(Value::is_object).unwrap_or_else(|| json!({"output":content}));
                    json!({"functionResponse":{"id":id,"name":name,"response":result}})
                }
            });
            }
            contents.push(
                json!({"role":if m.role == Role::Assistant {"model"} else {"user"},"parts":parts}),
            );
        }
        let mut generation = json!({"maxOutputTokens":self.max_output_tokens,"candidateCount":1});
        if let Some(t) = self.temperature {
            generation["temperature"] = json!(t);
        }
        if let Some(t) = self.top_p {
            generation["topP"] = json!(t);
        }
        if !self.stop.is_empty() {
            generation["stopSequences"] = json!(self.stop);
        }
        if let Some(image_config) = &self.image_config {
            generation["imageConfig"] = image_config.to_gemini_value();
        }
        if let Some(format) = &self.response_format {
            format.apply_gemini(&mut generation, self.client);
        }
        let mut v = json!({"contents":contents,"generationConfig":generation});
        if let Some(token) = &self.continuation_token {
            v["continuationToken"] = json!(token);
        }
        if !system.is_empty() {
            v["systemInstruction"] = json!({"parts":system.iter().map(|v| json!({"text":v["text"]})).collect::<Vec<_>>()});
        }
        if !self.tools.is_empty() {
            v["tools"] = json!([{"functionDeclarations":self.tools.iter().map(|t| {
                let mut v = json!({"name":t.name});
                let placed = super::gemini_schema::function_parameters(&t.parameters, super::gemini_schema::ACCEPTS_JSON_SCHEMA);
                debug_assert!(placed.warnings.is_empty());
                if let Some(value) = placed.value {
                    v[placed.field] = value;
                }
                if let Some(d) = &t.description { v["description"] = json!(d); } v }).collect::<Vec<_>>()}]);
            v["toolConfig"] = match &self.tool_choice {
                ToolChoice::Auto => json!({"functionCallingConfig":{"mode":"AUTO"}}),
                ToolChoice::None => json!({"functionCallingConfig":{"mode":"NONE"}}),
                ToolChoice::Required => json!({"functionCallingConfig":{"mode":"ANY"}}),
                ToolChoice::Named(n) => {
                    json!({"functionCallingConfig":{"mode":"ANY","allowedFunctionNames":[n]}})
                }
            };
        }
        if self.google_search {
            v["tools"] = json!([{"googleSearch":{}}]);
        }
        Ok(v)
    }
}

#[cfg(test)]
mod output_limit_tests {
    use super::*;

    #[test]
    fn source_precharge_preserves_original_openai_bytes() {
        let body = br#"{ "model": "gpt-4o", "input": "hi", "service_tier": "auto" }"#;
        let request =
            Request::parse_deferred_openai(ClientProtocol::Responses, body, &Limits::default())
                .unwrap();
        assert_eq!(
            request.legacy_precharge_body.as_deref(),
            Some(body.as_slice())
        );
        let cloned = request.clone();
        assert!(std::sync::Arc::ptr_eq(
            request.legacy_precharge_body.as_ref().unwrap(),
            cloned.legacy_precharge_body.as_ref().unwrap(),
        ));
        let chat_body = br#"{ "model": "gpt-4o", "messages": [{"role":"user","content":"hi"}], "service_tier":"auto" }"#;
        let chat =
            Request::parse_deferred_openai(ClientProtocol::Chat, chat_body, &Limits::default())
                .unwrap();
        assert_eq!(
            chat.legacy_precharge_body.as_deref(),
            Some(chat_body.as_slice())
        );
        let native = Request::parse(
            ClientProtocol::Responses,
            br#"{ "model": "gpt-4o", "input": "hi" }"#,
            &Limits::default(),
        )
        .unwrap();
        assert_eq!(
            native.legacy_precharge_body.as_deref(),
            Some(br#"{ "model": "gpt-4o", "input": "hi" }"#.as_slice()),
        );
    }

    #[test]
    fn source_output_limit_stays_distinct_from_provider_default() {
        let limits = Limits::default();
        let chat = |body| Request::parse_deferred_openai(ClientProtocol::Chat, body, &limits);
        let implicit =
            chat(br#"{"model":"m","messages":[{"role":"user","content":"hi"}]}"#).unwrap();
        let explicit = chat(
            br#"{"model":"m","messages":[{"role":"user","content":"hi"}],"max_completion_tokens":4096}"#,
        )
        .unwrap();
        assert_eq!(implicit.max_output_tokens, explicit.max_output_tokens);
        assert_eq!(implicit.explicit_max_output_tokens, None);
        assert_eq!(explicit.explicit_max_output_tokens, Some(4096));

        let ws = Request::parse_responses_ws(
            br#"{"model":"m","input":"hi","max_output_tokens":32}"#,
            &limits,
            false,
            None,
        )
        .unwrap();
        assert_eq!(ws.explicit_max_output_tokens, Some(32));
        let anthropic = Request::parse(
            ClientProtocol::Anthropic,
            br#"{"model":"m","max_tokens":32,"messages":[{"role":"user","content":"hi"}]}"#,
            &limits,
        )
        .unwrap();
        assert_eq!(anthropic.explicit_max_output_tokens, Some(32));
        let gemini = crate::relay::native_gemini::parse(
            "m",
            false,
            br#"{"contents":[{"parts":[{"text":"hi"}]}]}"#,
            &limits,
        )
        .unwrap();
        assert_eq!(gemini.explicit_max_output_tokens, None);
        assert_eq!(gemini.max_output_tokens, 4096);
    }
}

#[cfg(test)]
mod responses_ws_parse_tests {
    use super::*;

    fn body(input: Option<Value>) -> Vec<u8> {
        let mut v = json!({"model":"m","max_output_tokens":32});
        if let Some(input) = input {
            v["input"] = input;
        }
        serde_json::to_vec(&v).unwrap()
    }

    #[test]
    fn warmup_and_previous_state_allow_empty_input_only_in_ws() {
        let limits = Limits::default();
        for input in [None, Some(json!([]))] {
            let bytes = body(input);
            assert!(Request::parse(ClientProtocol::Responses, &bytes, &limits).is_err());
            assert!(
                Request::parse_deferred_openai(ClientProtocol::Responses, &bytes, &limits).is_err()
            );
            assert!(Request::parse_responses_ws(&bytes, &limits, false, None).is_err());
            let warmup = Request::parse_responses_ws(&bytes, &limits, false, Some(false)).unwrap();
            assert!(warmup.messages.is_empty());
            let continuation = Request::parse_responses_ws(&bytes, &limits, true, None).unwrap();
            assert!(continuation.messages.is_empty());
        }
        assert!(
            Request::parse_responses_ws(&body(Some(Value::Null)), &limits, true, Some(false))
                .is_err()
        );
    }

    #[test]
    fn previous_turn_tool_result_keeps_duplicate_and_http_rejection() {
        let limits = Limits::default();
        let item = json!({"type":"function_call_output","call_id":"old-call","output":"ok"});
        let bytes = body(Some(json!([item.clone()])));
        assert!(Request::parse(ClientProtocol::Responses, &bytes, &limits).is_err());
        assert!(Request::parse_responses_ws(&bytes, &limits, false, Some(false)).is_err());
        let continued = Request::parse_responses_ws(&bytes, &limits, true, None).unwrap();
        assert!(
            matches!(&continued.messages[0].content[0], InputBlock::ToolResult {id,..} if id == "old-call")
        );
        assert!(
            Request::parse_responses_ws(
                &body(Some(json!([item.clone(), item]))),
                &limits,
                true,
                None
            )
            .is_err()
        );
    }
}

#[cfg(test)]
mod zconv_tests {
    use super::*;
    fn parse(protocol: UpstreamProtocol, body: &Value, stream: bool) -> Request {
        let raw = serde_json::to_vec(body).unwrap();
        match protocol {
            UpstreamProtocol::Chat => {
                Request::parse(ClientProtocol::Chat, &raw, &Limits::default()).unwrap()
            }
            UpstreamProtocol::Responses => {
                Request::parse(ClientProtocol::Responses, &raw, &Limits::default()).unwrap()
            }
            UpstreamProtocol::Anthropic => {
                Request::parse(ClientProtocol::Anthropic, &raw, &Limits::default()).unwrap()
            }
            UpstreamProtocol::Gemini => {
                super::super::native_gemini::parse("provider", stream, &raw, &Limits::default())
                    .unwrap()
            }
        }
    }
    #[test]
    fn direct_ir_covers_every_source_and_target() {
        let protocols = [
            UpstreamProtocol::Chat,
            UpstreamProtocol::Responses,
            UpstreamProtocol::Anthropic,
            UpstreamProtocol::Gemini,
        ];
        let canonical = parse(
            UpstreamProtocol::Chat,
            &json!({
                "model":"provider", "max_tokens":32,
                "messages":[{"role":"system","content":"be precise"},{"role":"user","content":"hello"}],
                "temperature":1,"top_p":1
            }),
            false,
        );
        for source in protocols {
            // Integral sampling values are accepted by every current native
            // admission visitor with serde_json arbitrary_precision enabled.
            let source_body = canonical.upstream_to(source, "provider").unwrap();
            let request = parse(source, &source_body, false);
            for target in protocols {
                let actual = request.upstream_to(target, "provider").unwrap();
                let projected = parse(target, &actual, false);
                assert_eq!(
                    format!("{:?}", projected.messages),
                    format!("{:?}", canonical.messages),
                    "{source:?}->{target:?}"
                );
                assert_eq!(projected.max_output_tokens, 32);
                assert_eq!(projected.temperature, Some(1.0));
                assert_eq!(projected.top_p, Some(1.0));
            }
        }
    }

    #[test]
    fn direct_ir_preserves_fractional_sampling_in_every_target() {
        let request = parse(
            UpstreamProtocol::Chat,
            &json!({
                "model":"provider", "messages":[{"role":"user","content":"hello"}],
                "temperature":0.5, "top_p":0.9
            }),
            false,
        );
        for target in [
            UpstreamProtocol::Chat,
            UpstreamProtocol::Responses,
            UpstreamProtocol::Anthropic,
            UpstreamProtocol::Gemini,
        ] {
            let output = request.upstream_to(target, "provider").unwrap();
            let (temperature, top_p) = if target == UpstreamProtocol::Gemini {
                (
                    &output["generationConfig"]["temperature"],
                    &output["generationConfig"]["topP"],
                )
            } else {
                (&output["temperature"], &output["top_p"])
            };
            assert_eq!(temperature.as_f64(), Some(0.5), "{target:?}");
            assert_eq!(top_p.as_f64(), Some(0.9), "{target:?}");
        }
    }

    #[test]
    fn direct_ir_omits_absent_anthropic_system() {
        let request = parse(
            UpstreamProtocol::Chat,
            &json!({
                "model":"provider", "messages":[{"role":"user","content":"hello"}]
            }),
            false,
        );
        let native = request
            .upstream_to(UpstreamProtocol::Anthropic, "provider")
            .unwrap();
        assert!(native.get("system").is_none());
        let back = parse(UpstreamProtocol::Anthropic, &native, false)
            .upstream_to(UpstreamProtocol::Chat, "provider")
            .unwrap();
        assert_eq!(back["messages"], json!([{"role":"user","content":"hello"}]));
    }

    #[test]
    fn direct_ir_keeps_native_signed_history_and_cache_boundaries() {
        for target in [UpstreamProtocol::Anthropic, UpstreamProtocol::Gemini] {
            let mut request = parse(
                UpstreamProtocol::Chat,
                &json!({
                    "model":"provider","messages":[{"role":"user","content":"hello"}]
                }),
                false,
            );
            request.client = if target == UpstreamProtocol::Anthropic {
                ClientProtocol::Anthropic
            } else {
                ClientProtocol::Gemini
            };
            request.messages.push(Message {
                role: Role::Assistant,
                content: vec![
                    InputBlock::Thinking {
                        text: "private thought".into(),
                        signature: Some((target, "signed-thought".into())),
                    },
                    InputBlock::ToolCall {
                        id: "call-1".into(),
                        name: "f".into(),
                        arguments: "{}".into(),
                        signature: if target == UpstreamProtocol::Gemini {
                            Some((target, "signed-call".into()))
                        } else {
                            None
                        },
                    },
                ],
            });
            let actual = request.upstream_to(target, "provider").unwrap();
            if target == UpstreamProtocol::Anthropic {
                assert_eq!(
                    actual["messages"][1]["content"][0]["signature"],
                    "signed-thought"
                );
                let system =
                    json!([{"type":"text","text":"cached","cache_control":{"type":"ephemeral"}}]);
                request.anthropic_cached_prompt =
                    Some(Map::from_iter([("system".into(), system.clone())]));
                assert_eq!(
                    request.upstream_to(target, "provider").unwrap()["system"],
                    system
                );
            } else {
                assert_eq!(
                    actual["contents"][1]["parts"][0]["thoughtSignature"],
                    "signed-thought"
                );
                assert_eq!(
                    actual["contents"][1]["parts"][1]["thoughtSignature"],
                    "signed-call"
                );
            }
            assert!(
                request
                    .upstream_to(UpstreamProtocol::Chat, "provider")
                    .is_err()
            );
        }
    }

    fn old(request: &Request, protocol: UpstreamProtocol) -> Value {
        match protocol {
            UpstreamProtocol::Chat => request.chat("provider"),
            UpstreamProtocol::Responses => request.responses("provider"),
            UpstreamProtocol::Anthropic => request.anthropic("provider"),
            UpstreamProtocol::Gemini => request.gemini(),
        }
        .unwrap()
    }
    #[test]
    fn zconv_six_directions_match_existing_text_tool_image_semantics() {
        for other in [
            UpstreamProtocol::Responses,
            UpstreamProtocol::Anthropic,
            UpstreamProtocol::Gemini,
        ] {
            for (source, target) in [
                (UpstreamProtocol::Chat, other),
                (other, UpstreamProtocol::Chat),
            ] {
                for stream in [false, true] {
                    for kind in ["text", "tool", "image"] {
                        let mut body = json!({"model":"provider","stream":stream,"max_tokens":32,"messages":[{"role":"system","content":"be precise"},{"role":"user","content":"hello"}]});
                        if kind == "tool" {
                            body["tools"] = json!([{"type":"function","function":{"name":"weather","description":"Get weather","parameters":{"type":"object","properties":{"city":{"type":"string"}},"required":["city"]}}}]);
                            body["tool_choice"] =
                                json!({"type":"function","function":{"name":"weather"}});
                            body["messages"].as_array_mut().unwrap().extend([
                                json!({"role":"assistant","content":null,"tool_calls":[{"id":"call-1","type":"function","function":{"name":"weather","arguments":"{\"city\":\"Taipei\"}"}}]}),
                                json!({"role":"tool","tool_call_id":"call-1","content":"{\"temp\":20}"})]);
                        }
                        if kind == "image" {
                            body["messages"][1]["content"] = json!([{"type":"text","text":"read"},{"type":"image_url","image_url":{"url":"data:image/png;base64,AQID"}}]);
                        }
                        let canonical = parse(UpstreamProtocol::Chat, &body, stream);
                        let source_body = old(&canonical, source);
                        let mut request = parse(source, &source_body, stream);
                        request.client = match source {
                            UpstreamProtocol::Chat => ClientProtocol::Chat,
                            UpstreamProtocol::Responses => ClientProtocol::Responses,
                            UpstreamProtocol::Anthropic => ClientProtocol::Anthropic,
                            UpstreamProtocol::Gemini => ClientProtocol::Gemini,
                        };
                        let expected = old(&request, target);
                        let actual = request.upstream_to(target, "provider").unwrap_or_else(|e| {
                            panic!("{source:?}->{target:?} {kind} stream={stream}: {e:?}")
                        });
                        println!(
                            "ZCONV_DIFF {source:?}->{target:?} {kind} stream={stream}\nOLD={expected}\nACTUAL={actual}"
                        );
                        let mut comparable = expected.clone();
                        if target == UpstreamProtocol::Chat {
                            for message in comparable["messages"].as_array_mut().unwrap() {
                                if message.get("tool_call_id").is_some() {
                                    // Old writer carried Anthropic's user role
                                    // into Chat, producing invalid wire. zconv
                                    // correctly emits Chat's required tool role.
                                    message["role"] = json!("tool");
                                }
                            }
                            for message in actual["messages"].as_array().unwrap() {
                                if message.get("tool_call_id").is_some() {
                                    assert_eq!(message["role"], "tool");
                                }
                            }
                        }
                        let mut expected_request = parse(target, &comparable, stream);
                        // zconv direct.rs preserves Chat's non-strict default;
                        // the old writer incorrectly left Responses' different
                        // default implicit. Do not erase the explicit false.
                        if source == UpstreamProtocol::Chat && target == UpstreamProtocol::Responses
                        {
                            for tool in &mut expected_request.tools {
                                if tool.strict.is_none() {
                                    tool.strict = Some(false);
                                }
                            }
                        }
                        let actual_request = parse(target, &actual, stream);
                        if source == UpstreamProtocol::Responses && target == UpstreamProtocol::Chat
                        {
                            for tool in &mut expected_request.tools {
                                if tool.strict.is_none() {
                                    tool.strict = Some(false);
                                }
                            }
                        }
                        assert_eq!(
                            format!("{:?}", actual_request.messages),
                            format!("{:?}", expected_request.messages),
                            "{source:?}->{target:?} {kind}"
                        );
                        assert_eq!(
                            format!("{:?}", actual_request.tools),
                            format!("{:?}", expected_request.tools)
                        );
                        assert_eq!(
                            format!("{:?}", actual_request.tool_choice),
                            format!("{:?}", expected_request.tool_choice)
                        );
                        assert_eq!(actual_request.max_output_tokens, 32);
                        assert_eq!(actual_request.stream, stream);
                        if target == UpstreamProtocol::Chat && stream {
                            assert_eq!(actual["stream_options"]["include_usage"], true);
                        }
                        if target != UpstreamProtocol::Gemini {
                            assert_eq!(actual["model"], "provider");
                        }
                    }
                }
            }
        }
    }
    #[test]
    fn zconv_provider_controls_and_strict_defaults() {
        for strict in [None, Some(false), Some(true)] {
            let mut body = json!({"model":"provider","messages":[{"role":"user","content":"hi"}],"tools":[{"type":"function","function":{"name":"f","parameters":{"type":"object","properties":{}}}}]});
            if let Some(value) = strict {
                body["tools"][0]["function"]["strict"] = json!(value);
            }
            let request = parse(UpstreamProtocol::Chat, &body, false);
            let actual = request
                .upstream_to(UpstreamProtocol::Responses, "provider")
                .unwrap();
            assert_eq!(actual["tools"][0]["strict"], json!(strict.unwrap_or(false)));
            println!("STRICT {strict:?} {actual}");
        }
        for effort in ["none", "minimal", "low", "medium", "high", "xhigh", "max"] {
            for source in [UpstreamProtocol::Chat, UpstreamProtocol::Responses] {
                let body = if source == UpstreamProtocol::Chat {
                    json!({"model":"provider","messages":[{"role":"user","content":"hi"}],"reasoning_effort":effort})
                } else {
                    json!({"model":"provider","input":"hi","reasoning":{"effort":effort}})
                };
                let request = parse(source, &body, false);
                let target = if source == UpstreamProtocol::Chat {
                    UpstreamProtocol::Responses
                } else {
                    UpstreamProtocol::Chat
                };
                let actual = request.upstream_to(target, "provider").unwrap();
                assert_eq!(
                    if target == UpstreamProtocol::Chat {
                        &actual["reasoning_effort"]
                    } else {
                        &actual["reasoning"]["effort"]
                    },
                    effort
                );
                println!("REASONING {source:?}->{target:?} {actual}");
            }
        }
        for parallel in [false, true] {
            let body = json!({"model":"provider","messages":[{"role":"user","content":"hi"}],"tools":[{"type":"function","function":{"name":"f","parameters":{"type":"object","properties":{}}}}],"parallel_tool_calls":parallel});
            let request = parse(UpstreamProtocol::Chat, &body, false);
            let anthropic = request
                .upstream_to(UpstreamProtocol::Anthropic, "provider")
                .unwrap();
            assert_eq!(
                anthropic["tool_choice"]["disable_parallel_tool_use"],
                !parallel
            );
            let back = parse(UpstreamProtocol::Anthropic, &anthropic, false)
                .upstream_to(UpstreamProtocol::Chat, "provider")
                .unwrap();
            assert_eq!(back["parallel_tool_calls"], parallel);
            assert_eq!(
                request
                    .upstream_to(UpstreamProtocol::Gemini, "provider")
                    .is_ok(),
                parallel
            );
            println!("PARALLEL {parallel} {anthropic} BACK={back}");
        }
        for strict in [
            None,
            Some(Value::Null),
            Some(json!(false)),
            Some(json!(true)),
        ] {
            let schema = json!({"type":"object","properties":{"optional":{"type":"string"}},"additionalProperties":true});
            let mut body = json!({"model":"provider","input":"hi","tools":[{"type":"function","name":"f","parameters":schema}]});
            if let Some(value) = &strict {
                body["tools"][0]["strict"] = value.clone();
            }
            let request = parse(UpstreamProtocol::Responses, &body, false);
            let actual = request
                .upstream_to(UpstreamProtocol::Chat, "provider")
                .unwrap();
            let resolved = strict.as_ref().and_then(Value::as_bool).unwrap_or(false);
            assert_eq!(actual["tools"][0]["function"]["strict"], resolved);
            assert_eq!(actual["tools"][0]["function"]["parameters"], schema);
            let native = request
                .upstream_to(UpstreamProtocol::Responses, "provider")
                .unwrap();
            assert_eq!(
                native["tools"][0].get("strict"),
                strict.as_ref().filter(|value| !value.is_null())
            );
            assert_eq!(native["tools"][0]["parameters"], schema);
            println!("CORE_CHAT_ROUTE_STRICT {strict:?} CHAT={actual} NATIVE={native}");
        }
        let preserved = json!({"type":"object","properties":{},"default":{"type":"image_url","image_url":{"url":"x","detail":"auto"}},"examples":[{"type":"input_image","detail":"auto","image_url":"x"}]});
        let schema_request = parse(
            UpstreamProtocol::Chat,
            &json!({"model":"provider","messages":[{"role":"user","content":"hi"}],"tools":[{"type":"function","function":{"name":"f","parameters":preserved}}]}),
            false,
        );
        let native = schema_request
            .upstream_to(UpstreamProtocol::Anthropic, "provider")
            .unwrap();
        assert_eq!(native["tools"][0]["input_schema"], preserved);
        println!("SCHEMA_DEFAULT_PRESERVED {native}");
        let schema = json!({"type":"object","properties":{"x":{"type":"string"}},"additionalProperties":false,"default":{"type":"image_url","image_url":{"url":"x","detail":"auto"}}});
        let body = json!({"model":"provider","messages":[{"role":"user","content":"hi"}],"tools":[{"type":"function","function":{"name":"f","parameters":schema}}],"response_format":{"type":"json_schema","json_schema":{"name":"s","schema":schema}}});
        let request = parse(UpstreamProtocol::Chat, &body, false);
        let gemini = request
            .upstream_to(UpstreamProtocol::Gemini, "provider")
            .unwrap();
        assert_eq!(
            gemini["tools"][0]["functionDeclarations"][0]["parametersJsonSchema"],
            schema
        );
        assert_eq!(
            gemini["generationConfig"],
            old(&request, UpstreamProtocol::Gemini)["generationConfig"]
        );
        // Native Gemini input has never admitted responseSchema controls.
        // Verify only its supported tool-schema reverse direction, retaining
        // the native admission boundary rather than adding a new protocol.
        assert!(
            super::super::native_gemini::parse(
                "provider",
                false,
                &serde_json::to_vec(&gemini).unwrap(),
                &Limits::default()
            )
            .is_err()
        );
        let mut tool_only = gemini.clone();
        for key in ["responseMimeType", "responseSchema", "responseJsonSchema"] {
            tool_only["generationConfig"]
                .as_object_mut()
                .unwrap()
                .remove(key);
        }
        let back = parse(UpstreamProtocol::Gemini, &tool_only, false)
            .upstream_to(UpstreamProtocol::Chat, "provider")
            .unwrap();
        assert_eq!(back["tools"][0]["function"]["parameters"], schema);
        println!("SCHEMA GEMINI={gemini} BACK={back}");
        for detail in ["auto", "high"] {
            let body = json!({"model":"provider","messages":[{"role":"user","content":[{"type":"image_url","image_url":{"url":"data:image/png;base64,AQID","detail":detail}}]}]});
            let request = parse(UpstreamProtocol::Chat, &body, false);
            for target in [UpstreamProtocol::Anthropic, UpstreamProtocol::Gemini] {
                let actual = request.upstream_to(target, "provider");
                assert_eq!(
                    actual.is_ok(),
                    detail == "auto",
                    "{target:?} {detail}: {actual:?}"
                );
                println!("IMAGE_DETAIL {target:?} {detail} {actual:?}");
            }
        }
    }
}

#[cfg(test)]
#[path = "responses_format_tests.rs"]
mod responses_format_tests;

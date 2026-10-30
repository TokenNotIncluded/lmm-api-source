//! Gemini tool and response schema placement.
//!
//! Tool behavior aligns with the closed historical tool-parameter retention
//! (constraints the typed Schema would drop stay intact) and with
//! QuantumNous/new-api `4b13d938` / `c10780e0`.
//!
//! This relay has no per-channel Gemini API version. Vertex and Gemini API
//! v1beta accept `parametersJsonSchema` / `responseJsonSchema`; Gemini API v1
//! does not, and a Vertex path can also contain `/v1`, so the version cannot
//! be inferred from the route URL. [`ACCEPTS_JSON_SCHEMA`] is therefore the
//! shared path: schemas keep the normalized JSON Schema representation,
//! including constructs the typed Schema cannot hold. Passing `false` is the
//! v1 sanitizer (cleaned typed Schema plus the warning names of what was
//! dropped). Nothing in the route config selects that caller.

use serde_json::{Map, Value};

/// Shared path for every Gemini route this core encodes.
pub(crate) const ACCEPTS_JSON_SCHEMA: bool = true;

pub(crate) const MAX_SCHEMA_DEPTH: usize = 64;

/// Fields the native typed `parameters` converter already accepts.
/// Order-preserving OpenAPI conversion uses the same set.
pub(crate) const INBOUND_TYPED_FIELDS: &[&str] = &[
    "type",
    "properties",
    "required",
    "description",
    "items",
    "enum",
    "format",
    "minimum",
    "maximum",
    "minItems",
    "maxItems",
];

const TYPED_FIELDS: &[&str] = &[
    "anyOf",
    "default",
    "description",
    "enum",
    "example",
    "format",
    "items",
    "maxItems",
    "maxLength",
    "maxProperties",
    "maximum",
    "minItems",
    "minLength",
    "minProperties",
    "minimum",
    "nullable",
    "pattern",
    "properties",
    "propertyOrdering",
    "required",
    "title",
    "type",
];

#[derive(Clone, Copy, PartialEq, Eq)]
enum SchemaRole {
    /// Tool parameters. `#485` keeps `additionalProperties` and `$schema`.
    Tool,
    /// Structured output. Those two annotations alone stay on typed Schema.
    Response,
}

pub(crate) struct PlacedSchema {
    pub field: &'static str,
    pub value: Option<Value>,
    /// Empty when nothing was removed. On the v1 sanitizer these name the
    /// dropped constructs (`gemini_schema_keyword_dropped`).
    pub warnings: Vec<String>,
}

pub(crate) struct ResponseSchemaWire {
    pub field: &'static str,
    pub value: Value,
    pub warnings: Vec<String>,
}

/// Place a function schema. `json_schema` is [`ACCEPTS_JSON_SCHEMA`] for the
/// live relay and `false` for the Gemini API v1 sanitizer.
pub(crate) fn function_parameters(schema: &Value, json_schema: bool) -> PlacedSchema {
    if json_schema {
        // Fitting schemas already travel as normalized JSON Schema
        // (`parametersJsonSchema`, lowercase types, client key order). Moving
        // them to uppercase OpenAPI `parameters` would change that wire.
        // Misfits stay on the same field, unchanged, instead of being cleaned.
        return PlacedSchema {
            field: "parametersJsonSchema",
            value: Some(schema.clone()),
            warnings: Vec::new(),
        };
    }
    let warnings = schema_misfits(schema, SchemaRole::Tool);
    if omit_empty_tool_object(schema) {
        return PlacedSchema {
            field: "parameters",
            value: None,
            warnings,
        };
    }
    PlacedSchema {
        field: "parameters",
        value: Some(clean_function_parameters(schema, 0)),
        warnings,
    }
}

/// Place `response_format.json_schema.schema`.
pub(crate) fn response_schema(schema: &Value, json_schema: bool) -> ResponseSchemaWire {
    let warnings = schema_misfits(schema, SchemaRole::Response);
    if json_schema && !warnings.is_empty() {
        return ResponseSchemaWire {
            field: "responseJsonSchema",
            value: schema.clone(),
            warnings: Vec::new(),
        };
    }
    ResponseSchemaWire {
        field: "responseSchema",
        value: remove_additional_properties(schema, 0),
        warnings: if json_schema { Vec::new() } else { warnings },
    }
}

/// Native `parameters` carries a construct the typed converter would reject
/// or drop (`const`, `oneOf`, `$ref` / `$defs`, type arrays, tuple `items`).
/// Non-string enums alone stay on the typed converter, which removes them.
pub(crate) fn inbound_json_schema_fallback(schema: &Value) -> bool {
    fallback_node(schema)
}

pub(crate) fn rewrite_gemini_types(schema: &Value) -> Value {
    match schema {
        Value::Array(items) => Value::Array(items.iter().map(rewrite_gemini_types).collect()),
        Value::Object(map) => {
            let mut out = Map::new();
            for (key, value) in map {
                if key == "type" {
                    out.insert(key.clone(), rewrite_type_value(value));
                } else {
                    out.insert(key.clone(), rewrite_gemini_types(value));
                }
            }
            Value::Object(out)
        }
        other => other.clone(),
    }
}

pub(crate) fn normalize_inbound_type(text: &str) -> String {
    match text {
        "OBJECT" => "object".into(),
        "ARRAY" => "array".into(),
        "STRING" => "string".into(),
        "NUMBER" => "number".into(),
        "INTEGER" => "integer".into(),
        "BOOLEAN" => "boolean".into(),
        other => other.to_owned(),
    }
}

pub(crate) fn is_string_enum(value: &Value) -> bool {
    value
        .as_array()
        .is_some_and(|items| items.iter().all(Value::is_string))
}

fn fallback_node(schema: &Value) -> bool {
    let Some(map) = schema.as_object() else {
        return schema.is_boolean();
    };
    if map
        .keys()
        .any(|key| !INBOUND_TYPED_FIELDS.contains(&key.as_str()))
    {
        return true;
    }
    if matches!(map.get("type"), Some(Value::Array(_))) {
        return true;
    }
    if matches!(map.get("items"), Some(Value::Array(_))) {
        return true;
    }
    if let Some(properties) = map.get("properties") {
        let Some(properties) = properties.as_object() else {
            return true;
        };
        if properties.values().any(fallback_node) {
            return true;
        }
    }
    if let Some(items) = map.get("items")
        && fallback_node(items)
    {
        return true;
    }
    false
}

fn rewrite_type_value(value: &Value) -> Value {
    match value {
        Value::String(text) => Value::String(normalize_inbound_type(text)),
        Value::Array(items) => Value::Array(items.iter().map(rewrite_type_value).collect()),
        other => other.clone(),
    }
}

fn schema_misfits(schema: &Value, role: SchemaRole) -> Vec<String> {
    let mut found = std::collections::BTreeSet::new();
    collect_misfits(schema, 0, role, &mut found);
    found.into_iter().collect()
}

fn collect_misfits(
    schema: &Value,
    depth: usize,
    role: SchemaRole,
    found: &mut std::collections::BTreeSet<String>,
) {
    if depth > MAX_SCHEMA_DEPTH {
        found.insert(format!("nesting deeper than {MAX_SCHEMA_DEPTH} levels"));
        return;
    }
    let Some(map) = schema.as_object() else {
        if schema.is_boolean() {
            found.insert("boolean schemas".into());
        }
        return;
    };
    for (key, item) in map {
        if !TYPED_FIELDS.contains(&key.as_str()) {
            let exempt = role == SchemaRole::Response
                && matches!(key.as_str(), "additionalProperties" | "$schema");
            if !exempt {
                found.insert(key.clone());
            }
            continue;
        }
        match key.as_str() {
            "type" if item.is_array() => {
                found.insert("type arrays".into());
            }
            "enum"
                if item
                    .as_array()
                    .is_some_and(|values| values.iter().any(|value| !value.is_string())) =>
            {
                found.insert("non-string enum values".into());
            }
            "properties" => {
                if let Some(properties) = item.as_object() {
                    for property in properties.values() {
                        collect_misfits(property, depth + 1, role, found);
                    }
                }
            }
            "items" => {
                if let Some(entries) = item.as_array() {
                    found.insert("tuple items".into());
                    for entry in entries {
                        collect_misfits(entry, depth + 1, role, found);
                    }
                } else {
                    collect_misfits(item, depth + 1, role, found);
                }
            }
            "anyOf" => {
                if let Some(variants) = item.as_array() {
                    for variant in variants {
                        collect_misfits(variant, depth + 1, role, found);
                    }
                }
            }
            _ => {}
        }
    }
}

/// `#485` omits a typed object that has no properties and no other constraint.
fn omit_empty_tool_object(schema: &Value) -> bool {
    let Some(map) = schema.as_object() else {
        return false;
    };
    if map.get("type").and_then(Value::as_str) != Some("object") {
        return false;
    }
    match map.get("properties").and_then(Value::as_object) {
        Some(properties) if properties.is_empty() => {}
        _ => return false,
    }
    map.keys().all(|key| {
        matches!(
            key.as_str(),
            "type" | "properties" | "title" | "description"
        )
    })
}

fn clean_function_parameters(params: &Value, depth: usize) -> Value {
    if params.is_null() {
        return Value::Null;
    }
    if depth >= MAX_SCHEMA_DEPTH {
        return clean_shallow(params);
    }
    match params {
        Value::Object(map) => clean_object(map, depth),
        Value::Array(items) => Value::Array(
            items
                .iter()
                .map(|item| clean_function_parameters(item, depth + 1))
                .collect(),
        ),
        other => other.clone(),
    }
}

fn clean_object(map: &Map<String, Value>, depth: usize) -> Value {
    let mut cleaned = Map::new();
    for (key, value) in map {
        if TYPED_FIELDS.contains(&key.as_str()) {
            cleaned.insert(key.clone(), value.clone());
        }
    }
    normalize_type_and_nullable(&mut cleaned);
    sanitize_enum(&mut cleaned);
    if let Some(Value::Object(properties)) = cleaned.get("properties").cloned() {
        let mut next = Map::new();
        for (name, property) in properties {
            next.insert(name, clean_function_parameters(&property, depth + 1));
        }
        cleaned.insert("properties".into(), Value::Object(next));
    }
    if let Some(items) = cleaned.get("items").cloned() {
        if let Value::Object(items) = items {
            cleaned.insert(
                "items".into(),
                clean_function_parameters(&Value::Object(items), depth + 1),
            );
        } else if let Value::Array(items) = items
            && let Some(first) = items.first()
        {
            cleaned.insert("items".into(), clean_function_parameters(first, depth + 1));
        }
    }
    if let Some(Value::Array(variants)) = cleaned.get("anyOf").cloned() {
        cleaned.insert(
            "anyOf".into(),
            Value::Array(
                variants
                    .iter()
                    .map(|variant| clean_function_parameters(variant, depth + 1))
                    .collect(),
            ),
        );
    }
    Value::Object(cleaned)
}

fn clean_shallow(params: &Value) -> Value {
    match params {
        Value::Object(map) => {
            let mut cleaned = Map::new();
            for (key, value) in map {
                if TYPED_FIELDS.contains(&key.as_str()) {
                    cleaned.insert(key.clone(), value.clone());
                }
            }
            normalize_type_and_nullable(&mut cleaned);
            sanitize_enum(&mut cleaned);
            cleaned.remove("properties");
            cleaned.remove("items");
            cleaned.remove("anyOf");
            Value::Object(cleaned)
        }
        Value::Array(_) => Value::Array(Vec::new()),
        other => other.clone(),
    }
}

fn sanitize_enum(schema: &mut Map<String, Value>) {
    let Some(raw) = schema.get("enum") else {
        return;
    };
    if !is_string_enum(raw) {
        schema.remove("enum");
    }
}

fn normalize_type_and_nullable(schema: &mut Map<String, Value>) {
    let Some(raw) = schema.get("type").cloned() else {
        return;
    };
    if raw.is_null() {
        return;
    }
    match raw {
        Value::String(text) => {
            let (normalized, is_null) = normalize_typed_name(&text);
            if is_null {
                schema.insert("nullable".into(), Value::Bool(true));
                schema.remove("type");
            } else {
                schema.insert("type".into(), Value::String(normalized));
            }
        }
        Value::Array(items) => {
            let mut nullable = false;
            let mut chosen = String::new();
            for item in items {
                let Some(text) = item.as_str() else {
                    continue;
                };
                let (normalized, is_null) = normalize_typed_name(text);
                if is_null {
                    nullable = true;
                    continue;
                }
                if chosen.is_empty() {
                    chosen = normalized;
                }
            }
            if nullable {
                schema.insert("nullable".into(), Value::Bool(true));
            }
            if chosen.is_empty() {
                schema.remove("type");
            } else {
                schema.insert("type".into(), Value::String(chosen));
            }
        }
        _ => {}
    }
}

fn normalize_typed_name(text: &str) -> (String, bool) {
    match text.trim().to_ascii_lowercase().as_str() {
        "object" => ("OBJECT".into(), false),
        "array" => ("ARRAY".into(), false),
        "string" => ("STRING".into(), false),
        "integer" => ("INTEGER".into(), false),
        "number" => ("NUMBER".into(), false),
        "boolean" => ("BOOLEAN".into(), false),
        "null" => (String::new(), true),
        _ => (text.to_owned(), false),
    }
}

fn remove_additional_properties(schema: &Value, depth: usize) -> Value {
    if depth >= 5 {
        return schema.clone();
    }
    let Some(map) = schema.as_object() else {
        return schema.clone();
    };
    if map.is_empty() {
        return schema.clone();
    }
    let mut value = map.clone();
    value.remove("title");
    value.remove("$schema");
    let keeps_nested = matches!(
        value.get("type").and_then(Value::as_str),
        Some("object" | "array")
    );
    if !keeps_nested {
        return Value::Object(value);
    }
    if value.get("type").and_then(Value::as_str) == Some("object") {
        value.remove("additionalProperties");
        if let Some(Value::Object(properties)) = value.get("properties").cloned() {
            let mut cleaned = Map::new();
            for (key, nested) in properties {
                cleaned.insert(key, remove_additional_properties(&nested, depth + 1));
            }
            value.insert("properties".into(), Value::Object(cleaned));
        }
        for field in ["allOf", "anyOf", "oneOf"] {
            if let Some(Value::Array(items)) = value.get(field).cloned() {
                let cleaned = items
                    .iter()
                    .map(|item| remove_additional_properties(item, depth + 1))
                    .collect();
                value.insert(field.into(), Value::Array(cleaned));
            }
        }
    } else if let Some(Value::Object(items)) = value.get("items").cloned() {
        value.insert(
            "items".into(),
            remove_additional_properties(&Value::Object(items), depth + 1),
        );
    }
    Value::Object(value)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::relay::{ClientProtocol, Limits, UpstreamProtocol, native_gemini, request::Request};
    use serde_json::json;

    fn declaration(parameters: Value) -> Value {
        let body = json!({
            "model": "m",
            "messages": [{"role": "user", "content": "hi"}],
            "tools": [{"type": "function", "function": {"name": "f", "parameters": parameters}}]
        });
        let request = Request::parse(
            ClientProtocol::Chat,
            &serde_json::to_vec(&body).unwrap(),
            &Limits::default(),
        )
        .unwrap();
        request.upstream_to(UpstreamProtocol::Gemini, "m").unwrap()["tools"][0]
            ["functionDeclarations"][0]
            .clone()
    }

    #[test]
    fn v1_sanitizer_drops_non_string_enums_and_keeps_string_enums() {
        let cases = [
            (
                json!({"type": "string", "enum": ["red", "green", "blue"]}),
                json!({"type": "STRING", "enum": ["red", "green", "blue"]}),
            ),
            (
                json!({"type": "boolean", "enum": [true]}),
                json!({"type": "BOOLEAN"}),
            ),
            (
                json!({"type": "string", "enum": ["on", true]}),
                json!({"type": "STRING"}),
            ),
            (
                json!({"type": "number", "enum": [1.0, 2.5]}),
                json!({"type": "NUMBER"}),
            ),
            (
                json!({
                    "type": "object",
                    "properties": {
                        "flag": {"type": "boolean", "enum": [true]},
                        "color": {"type": "string", "enum": ["red"]}
                    }
                }),
                json!({
                    "type": "OBJECT",
                    "properties": {
                        "flag": {"type": "BOOLEAN"},
                        "color": {"type": "STRING", "enum": ["red"]}
                    }
                }),
            ),
            (
                json!({
                    "type": "array",
                    "items": {"type": "boolean", "enum": [false]}
                }),
                json!({"type": "ARRAY", "items": {"type": "BOOLEAN"}}),
            ),
            (
                json!({
                    "anyOf": [
                        {"type": "boolean", "enum": [true]},
                        {"type": "string", "enum": ["yes", "no"]}
                    ]
                }),
                json!({
                    "anyOf": [
                        {"type": "BOOLEAN"},
                        {"type": "STRING", "enum": ["yes", "no"]}
                    ]
                }),
            ),
        ];
        for (input, expected) in cases {
            let placed = function_parameters(&input, false);
            assert_eq!(placed.field, "parameters");
            assert_eq!(placed.value.unwrap(), expected);
        }
        let shallow = function_parameters(&json!({"type": "boolean", "enum": [true]}), false);
        assert!(
            shallow
                .warnings
                .iter()
                .any(|warning| warning == "non-string enum values")
        );
        assert!(
            function_parameters(&json!({"type": "string", "enum": ["a", "b"]}), false)
                .warnings
                .is_empty()
        );
    }

    #[test]
    fn v1_sanitizer_warns_and_strips_keywords_the_typed_schema_cannot_hold() {
        let placed = function_parameters(
            &json!({
                "type": "object",
                "properties": {"mode": {"const": "fast"}},
                "oneOf": [{"type": "object"}],
                "$ref": "#/$defs/box",
                "$defs": {"box": {"type": "string"}}
            }),
            false,
        );
        let value = placed.value.unwrap();
        assert!(value.get("oneOf").is_none());
        assert!(value.get("$ref").is_none());
        assert!(value.get("$defs").is_none());
        assert!(value["properties"]["mode"].get("const").is_none());
        for keyword in ["$defs", "$ref", "const", "oneOf"] {
            assert!(
                placed.warnings.iter().any(|warning| warning == keyword),
                "{:?}",
                placed.warnings
            );
        }
    }

    #[test]
    fn v1_sanitizer_omits_a_genuinely_empty_object_and_keeps_required() {
        let empty = function_parameters(
            &json!({"type": "object", "properties": {}, "description": "none"}),
            false,
        );
        assert_eq!(empty.field, "parameters");
        assert!(empty.value.is_none());
        let required = function_parameters(
            &json!({"type": "object", "properties": {}, "required": ["a"]}),
            false,
        );
        assert_eq!(
            required.value.unwrap(),
            json!({"type": "OBJECT", "properties": {}, "required": ["a"]})
        );
    }

    #[test]
    fn v1_depth_cutoff_sanitizes_enums_without_keeping_deeper_nodes() {
        let mut schema = json!({
            "type": "object",
            "enum": [true],
            "properties": {"secret": {"type": "string", "description": "leaf"}}
        });
        for _ in 0..MAX_SCHEMA_DEPTH {
            schema = json!({"type": "object", "properties": {"child": schema}});
        }
        let placed = function_parameters(&schema, false);
        assert!(
            placed
                .warnings
                .iter()
                .any(|warning| warning == "non-string enum values")
        );
        assert!(
            placed
                .warnings
                .iter()
                .any(|warning| warning.contains("nesting deeper"))
        );
        let encoded = placed.value.unwrap().to_string();
        assert!(encoded.contains("\"type\":\"OBJECT\""));
        assert!(!encoded.contains("secret"));
        assert!(!encoded.contains("leaf"));
        assert!(!encoded.contains("true"));
    }

    #[test]
    fn live_tool_wire_keeps_constraints_on_parameters_json_schema() {
        let schema = json!({
            "type": "object",
            "additionalProperties": false,
            "properties": {
                "mode": {"const": "fast"},
                "choice": {"oneOf": [{"type": "string"}, {"type": "number"}]},
                "box": {"$ref": "#/$defs/box"},
                "flag": {"type": "boolean", "enum": [true]},
                "rank": {"type": ["integer", "null"]},
                "pair": {"type": "array", "items": [{"type": "string"}, {"const": 1}]}
            },
            "$defs": {"box": {"type": "string"}}
        });
        let wire = declaration(schema.clone());
        assert!(wire.get("parameters").is_none());
        assert_eq!(wire["parametersJsonSchema"], schema);
        let fitting = declaration(json!({
            "type": "object",
            "properties": {"zeta": {"type": "string"}, "alpha": {"type": "integer"}}
        }));
        assert_eq!(fitting["parametersJsonSchema"]["type"], "object");
        assert!(fitting.get("parameters").is_none());
    }

    #[test]
    fn response_schema_uses_json_schema_only_for_constructs_typed_schema_cannot_hold() {
        let fitting = response_schema(
            &json!({
                "type": "object",
                "additionalProperties": false,
                "$schema": "https://json-schema.org/draft/2020-12/schema",
                "properties": {"color": {"type": "string", "enum": ["red"]}}
            }),
            ACCEPTS_JSON_SCHEMA,
        );
        assert_eq!(fitting.field, "responseSchema");
        assert!(fitting.warnings.is_empty());
        assert!(fitting.value.get("additionalProperties").is_none());
        assert!(fitting.value.get("$schema").is_none());
        assert_eq!(fitting.value["properties"]["color"]["enum"], json!(["red"]));

        for (name, schema) in [
            (
                "oneOf",
                json!({"oneOf": [{"type": "string"}, {"const": "a"}]}),
            ),
            (
                "$ref",
                json!({"$ref": "#/$defs/box", "$defs": {"box": {"type": "string"}}}),
            ),
            ("const", json!({"const": "only"})),
            ("type array", json!({"type": ["string", "null"]})),
            (
                "non-string enum",
                json!({"type": "boolean", "enum": [true]}),
            ),
        ] {
            let placed = response_schema(&schema, ACCEPTS_JSON_SCHEMA);
            assert_eq!(placed.field, "responseJsonSchema", "{name}");
            assert_eq!(placed.value, schema, "{name}");
            assert!(placed.warnings.is_empty(), "{name}");
        }

        let v1 = response_schema(&json!({"const": 1, "oneOf": [{"type": "string"}]}), false);
        assert_eq!(v1.field, "responseSchema");
        assert!(v1.warnings.iter().any(|warning| warning == "oneOf"));
        assert!(v1.warnings.iter().any(|warning| warning == "const"));
    }

    #[test]
    fn chat_response_format_reaches_gemini_without_dropping_schema_order() {
        let schema = r#"{"type":"object","properties":{"zeta":{"type":"string"},"alpha":{"type":"integer"}},"oneOf":[{"required":["zeta"]},{"required":["alpha"]}]}"#;
        let body = format!(
            r#"{{"model":"m","messages":[{{"role":"user","content":"hi"}}],"response_format":{{"type":"json_schema","json_schema":{{"name":"answer","strict":true,"schema":{schema}}}}}}}"#
        );
        let request =
            Request::parse(ClientProtocol::Chat, body.as_bytes(), &Limits::default()).unwrap();
        let text = String::from_utf8(
            request
                .upstream_body_to(UpstreamProtocol::Gemini, "upstream")
                .unwrap(),
        )
        .unwrap();
        assert!(
            text.contains(&format!(r#""responseJsonSchema":{schema}"#)),
            "{text}"
        );
        assert!(
            text.contains("\"responseMimeType\":\"application/json\""),
            "{text}"
        );
        let chat = request.upstream_to(UpstreamProtocol::Chat, "m").unwrap();
        assert_eq!(chat["response_format"]["type"], "json_schema");
        assert_eq!(chat["response_format"]["json_schema"]["strict"], true);
        assert!(
            request
                .upstream_to(UpstreamProtocol::Anthropic, "m")
                .is_err()
        );
        let object = json!({
            "model": "m",
            "messages": [{"role": "user", "content": "hi"}],
            "response_format": {"type": "json_object"}
        });
        let request = Request::parse(
            ClientProtocol::Chat,
            &serde_json::to_vec(&object).unwrap(),
            &Limits::default(),
        )
        .unwrap();
        let gemini = request.upstream_to(UpstreamProtocol::Gemini, "m").unwrap();
        assert_eq!(
            gemini["generationConfig"]["responseMimeType"],
            "application/json"
        );
        assert!(gemini["generationConfig"].get("responseSchema").is_none());
        assert!(
            gemini["generationConfig"]
                .get("responseJsonSchema")
                .is_none()
        );
    }

    #[test]
    fn fitting_response_schema_drops_additional_properties_and_keeps_string_enums() {
        let body = json!({
            "model": "m",
            "messages": [{"role": "user", "content": "hi"}],
            "response_format": {
                "type": "json_schema",
                "json_schema": {
                    "name": "answer",
                    "schema": {
                        "type": "object",
                        "additionalProperties": false,
                        "properties": {"color": {"type": "string", "enum": ["red", "blue"]}}
                    }
                }
            }
        });
        let request = Request::parse(
            ClientProtocol::Chat,
            &serde_json::to_vec(&body).unwrap(),
            &Limits::default(),
        )
        .unwrap();
        let gemini = request.upstream_to(UpstreamProtocol::Gemini, "m").unwrap();
        let schema = &gemini["generationConfig"]["responseSchema"];
        assert!(schema.get("additionalProperties").is_none());
        assert_eq!(
            schema["properties"]["color"]["enum"],
            json!(["red", "blue"])
        );
        assert!(
            gemini["generationConfig"]
                .get("responseJsonSchema")
                .is_none()
        );
    }

    #[test]
    fn native_typed_parameters_drop_non_string_enums_and_keep_unsupported_keywords() {
        let dropped = native_gemini::parse(
            "m",
            false,
            br#"{"contents":[{"parts":[{"text":"hi"}]}],"tools":[{"functionDeclarations":[{"name":"f","parameters":{"type":"BOOLEAN","enum":[true],"description":"flag"}}]}]}"#,
            &Limits::default(),
        )
        .unwrap();
        let wire = dropped.upstream_to(UpstreamProtocol::Gemini, "m").unwrap();
        let schema = &wire["tools"][0]["functionDeclarations"][0]["parametersJsonSchema"];
        assert_eq!(schema["type"], "boolean");
        assert_eq!(schema["description"], "flag");
        assert!(schema.get("enum").is_none());

        let kept = native_gemini::parse(
            "m",
            false,
            br#"{"contents":[{"parts":[{"text":"hi"}]}],"tools":[{"functionDeclarations":[{"name":"f","parameters":{"type":"OBJECT","properties":{"zeta":{"const":1},"alpha":{"type":"INTEGER","enum":["a","b"]}}}}]}]}"#,
            &Limits::default(),
        )
        .unwrap();
        let text = String::from_utf8(
            kept.upstream_body_to(UpstreamProtocol::Gemini, "m")
                .unwrap(),
        )
        .unwrap();
        assert!(
            text.contains(
                r#""parametersJsonSchema":{"type":"object","properties":{"zeta":{"const":1},"alpha":{"type":"integer","enum":["a","b"]}}}"#
            ),
            "{text}"
        );
    }
}

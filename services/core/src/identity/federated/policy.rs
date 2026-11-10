//! Legacy custom OAuth access policies (go-v0.2.118 oauth/generic.go).
//! Policy data gates login; it never supplies Core roles or account ownership.
use crate::identity::{IdentityError, Result};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::cmp::Ordering;

pub(super) const MAX_JSON_BYTES: usize = 1_048_576;
pub(super) const MAX_PATH_BYTES: usize = 4096;

pub(super) fn bounded_json(raw: &str) -> Result<()> {
    if raw.len() > MAX_JSON_BYTES {
        return Err(IdentityError::Unauthorized);
    }
    let (mut depth, mut quoted, mut escaped) = (0_u16, false, false);
    for byte in raw.bytes() {
        if quoted {
            if escaped {
                escaped = false;
            } else if byte == b'\\' {
                escaped = true;
            } else if byte == b'"' {
                quoted = false;
            }
        } else {
            match byte {
                b'"' => quoted = true,
                b'{' | b'[' => {
                    depth += 1;
                    if depth > 128 {
                        return Err(IdentityError::Unauthorized);
                    }
                }
                b'}' | b']' => {
                    depth = depth.checked_sub(1).ok_or(IdentityError::Unauthorized)?;
                }
                _ => {}
            }
        }
    }
    if quoted || depth != 0 || !gjson::valid(raw) {
        return Err(IdentityError::Unauthorized);
    }
    Ok(())
}

#[derive(Deserialize, Serialize)]
pub(super) struct Policy {
    #[serde(default)]
    logic: String,
    #[serde(default)]
    conditions: Vec<Condition>,
    #[serde(default)]
    groups: Vec<Policy>,
}

#[derive(Deserialize, Serialize)]
struct Condition {
    field: String,
    op: String,
    #[serde(default)]
    value: Value,
}

pub(super) struct Failure {
    field: String,
    op: String,
    required: Value,
    current: Value,
}

/// Unknown modifiers must be rejected, never treated as a missing field: that
/// would silently authorize `not_exists`/`ne` policies. The upstream Rust port
/// currently implements these seven built-ins; additional legacy expressions
/// require explicit compatibility vectors before being enabled.
pub(super) fn validate_path(path: &str) -> Result<()> {
    if path.is_empty() || path.len() > MAX_PATH_BYTES || path.chars().any(char::is_control) {
        return Err(IdentityError::Invalid);
    }
    let mut quoted = false;
    let mut escaped = false;
    let mut component = true;
    for (index, ch) in path.char_indices() {
        if escaped {
            escaped = false;
            component = false;
            continue;
        }
        if ch == '\\' {
            escaped = true;
            continue;
        }
        if ch == '"' {
            quoted = !quoted;
            component = false;
            continue;
        }
        if quoted {
            continue;
        }
        if ch == '@' && component {
            let modifier: String = path[index + 1..]
                .chars()
                .take_while(|c| c.is_ascii_alphanumeric() || *c == '_')
                .collect();
            if !matches!(
                modifier.as_str(),
                "this" | "reverse" | "ugly" | "pretty" | "valid" | "flatten" | "join"
            ) {
                return Err(IdentityError::Invalid);
            }
        }
        component = matches!(ch, '.' | '|');
    }
    if quoted || escaped {
        return Err(IdentityError::Invalid);
    }
    Ok(())
}

pub(super) fn parse(raw: &str) -> Result<Option<Policy>> {
    if raw.trim().is_empty() {
        return Ok(None);
    }
    if raw.len() > MAX_JSON_BYTES {
        return Err(IdentityError::Invalid);
    }
    let mut policy: Policy = serde_json::from_str(raw).map_err(|_| IdentityError::Invalid)?;
    let mut nodes = 0;
    policy.validate(0, &mut nodes)?;
    Ok(Some(policy))
}

impl Policy {
    fn validate(&mut self, depth: usize, nodes: &mut usize) -> Result<()> {
        *nodes += 1 + self.conditions.len();
        if depth > 16 || *nodes > 512 || (self.conditions.is_empty() && self.groups.is_empty()) {
            return Err(IdentityError::Invalid);
        }
        self.logic = self.logic.trim().to_ascii_lowercase();
        if self.logic.is_empty() {
            self.logic = "and".into();
        }
        if !matches!(self.logic.as_str(), "and" | "or") {
            return Err(IdentityError::Invalid);
        }
        for condition in &mut self.conditions {
            condition.field = condition.field.trim().to_owned();
            condition.op = condition.op.trim().to_ascii_lowercase();
            validate_path(&condition.field)?;
            if !matches!(
                condition.op.as_str(),
                "eq" | "ne"
                    | "gt"
                    | "gte"
                    | "lt"
                    | "lte"
                    | "in"
                    | "not_in"
                    | "contains"
                    | "not_contains"
                    | "exists"
                    | "not_exists"
            ) || (matches!(condition.op.as_str(), "in" | "not_in")
                && !condition.value.is_array())
            {
                return Err(IdentityError::Invalid);
            }
        }
        for group in &mut self.groups {
            group.validate(depth + 1, nodes)?;
        }
        Ok(())
    }

    pub(super) fn evaluate(&self, json: &str) -> Result<Option<Failure>> {
        if json.len() > MAX_JSON_BYTES || !gjson::valid(json) {
            return Err(IdentityError::Unauthorized);
        }
        self.evaluate_valid(json)
    }

    fn evaluate_valid(&self, json: &str) -> Result<Option<Failure>> {
        let mut first_failure = None;
        for condition in &self.conditions {
            let failure = condition.evaluate(json)?;
            if self.logic == "or" && failure.is_none() {
                return Ok(None);
            }
            if self.logic == "and" && failure.is_some() {
                return Ok(failure);
            }
            if first_failure.is_none() {
                first_failure = failure;
            }
        }
        for group in &self.groups {
            let failure = group.evaluate_valid(json)?;
            if self.logic == "or" && failure.is_none() {
                return Ok(None);
            }
            if self.logic == "and" && failure.is_some() {
                return Ok(failure);
            }
            if first_failure.is_none() {
                first_failure = failure;
            }
        }
        if self.logic == "or" {
            Ok(first_failure)
        } else {
            Ok(None)
        }
    }
}

impl Condition {
    fn evaluate(&self, json: &str) -> Result<Option<Failure>> {
        validate_path(&self.field)?;
        let found = gjson::get(json, &self.field);
        let current = if found.exists() {
            serde_json::from_str(found.json()).map_err(|_| IdentityError::Unauthorized)?
        } else {
            Value::Null
        };
        let ordering = compare(&current, &self.value);
        let allowed = match self.op.as_str() {
            "exists" => found.exists(),
            "not_exists" => !found.exists(),
            "eq" => ordering == Ordering::Equal,
            "ne" => ordering != Ordering::Equal,
            "gt" => ordering == Ordering::Greater,
            "gte" => ordering != Ordering::Less,
            "lt" => ordering == Ordering::Less,
            "lte" => ordering != Ordering::Greater,
            "in" => includes(&self.value, &current),
            "not_in" => !includes(&self.value, &current),
            "contains" => contains(&current, &self.value),
            "not_contains" => !contains(&current, &self.value),
            _ => return Err(IdentityError::Invalid),
        };
        Ok((!allowed).then(|| Failure {
            field: self.field.clone(),
            op: self.op.clone(),
            required: self.value.clone(),
            current,
        }))
    }
}

fn numeric(value: &Value) -> Option<f64> {
    match value {
        Value::Number(n) => n.to_string().parse().ok(),
        Value::String(s) => s.trim().parse::<f64>().ok().filter(|n| {
            n.is_finite()
                || matches!(
                    s.trim(),
                    "NaN" | "Inf" | "+Inf" | "-Inf" | "Infinity" | "+Infinity" | "-Infinity"
                )
        }),
        _ => None,
    }
}

// Go's legacy comparison first coerces numeric strings to float64, then uses
// trimmed fmt.Sprint values. Preserve those rules, including nil/bool/arrays.
fn display(value: &Value) -> String {
    match value {
        Value::Null => "<nil>".into(),
        Value::String(s) => s.clone(),
        Value::Array(items) => format!(
            "[{}]",
            items.iter().map(display).collect::<Vec<_>>().join(" ")
        ),
        Value::Object(fields) => format!(
            "map[{}]",
            fields
                .iter()
                .map(|(key, value)| format!("{key}:{}", display(value)))
                .collect::<Vec<_>>()
                .join(" ")
        ),
        Value::Number(n) => {
            let n = n.to_string().parse::<f64>().unwrap_or(f64::NAN);
            if n.is_nan() {
                "NaN".into()
            } else if n == f64::INFINITY {
                "+Inf".into()
            } else if n == f64::NEG_INFINITY {
                "-Inf".into()
            } else if n != 0.0 && (n.abs() < 0.0001 || n.abs() >= 1_000_000.0) {
                let formatted = format!("{n:e}");
                let (mantissa, exponent) = formatted.split_once('e').unwrap_or((&formatted, "0"));
                let exponent = exponent.parse::<i32>().unwrap_or(0);
                format!(
                    "{mantissa}e{}{:02}",
                    if exponent >= 0 { "+" } else { "-" },
                    exponent.unsigned_abs()
                )
            } else {
                n.to_string()
            }
        }
        _ => value.to_string(),
    }
}

fn compare(left: &Value, right: &Value) -> Ordering {
    if let (Some(left), Some(right)) = (numeric(left), numeric(right)) {
        // The old code's final default is equality, including NaN.
        return left.partial_cmp(&right).unwrap_or(Ordering::Equal);
    }
    display(left).trim().cmp(display(right).trim())
}

fn includes(list: &Value, value: &Value) -> bool {
    list.as_array().is_some_and(|items| {
        items
            .iter()
            .any(|item| compare(item, value) == Ordering::Equal)
    })
}

fn contains(current: &Value, required: &Value) -> bool {
    match current {
        Value::String(s) => s.contains(display(required).trim()),
        Value::Array(_) => includes(current, required),
        _ => false,
    }
}

impl Failure {
    pub(super) fn message(&self, template: &str, provider: &str, json: &str) -> Result<String> {
        let mut text = template.trim().to_owned();
        if text.is_empty() {
            return Ok(
                "Access denied: your account does not meet this provider's access requirements."
                    .into(),
            );
        }
        for (name, value) in [
            ("provider", provider.to_owned()),
            ("field", self.field.clone()),
            ("op", self.op.clone()),
            ("required", display(&self.required)),
            ("current", display(&self.current)),
        ] {
            text = text.replace(&format!("{{{{{name}}}}}"), &value);
        }
        // As in legacy Go, expand current.path after the fixed placeholders.
        // Inserted current.path values are not interpreted a second time.
        let mut output = String::new();
        let mut rest = text.as_str();
        while let Some(start) = rest.find("{{current.") {
            output.push_str(&rest[..start]);
            let tail = &rest[start + 10..];
            let Some(end) = tail.find("}}") else {
                output.push_str(&rest[start..]);
                rest = "";
                break;
            };
            let path = tail[..end].trim();
            validate_path(path)?;
            let value = gjson::get(json, path);
            output.push_str(value.str().trim());
            rest = &tail[end + 2..];
        }
        output.push_str(rest);
        Ok(output)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn legacy_mapping_vectors_preserve_paths_and_raw_id() {
        let raw = r#"{"data":{"user":{"id":9007199254740993}},"fav.movie":"One","groups":[{"name":"admin"},{"name":"user"}],"verified":true}"#;
        for (path, expected) in [
            ("data.user.id", concat!("9007199254", "740993")),
            (r"fav\.movie", "One"),
            ("groups.#(name==\"admin\").name", "admin"),
            ("groups.#.name", r#"["admin","user"]"#),
            ("groups|@reverse|0.name", "user"),
        ] {
            validate_path(path).unwrap();
            assert_eq!(gjson::get(raw, path).str(), expected, "{path}");
        }
    }

    #[test]
    fn legacy_nested_policy_numeric_coercion_and_missing_null() {
        let policy = parse(r#"{"logic":"AND","conditions":[{"field":"level","op":"gte","value":10}],"groups":[{"logic":"or","conditions":[{"field":"groups","op":"contains","value":"staff"},{"field":"disabled","op":"not_exists"}]}]}"#).unwrap().unwrap();
        assert!(
            policy
                .evaluate(r#"{"level":"10","groups":["user"]}"#)
                .unwrap()
                .is_none()
        );
        assert!(
            policy
                .evaluate(r#"{"level":"9","groups":["staff"]}"#)
                .unwrap()
                .is_some()
        );
        assert!(
            policy
                .evaluate(r#"{"level":10,"disabled":null,"groups":["user"]}"#)
                .unwrap()
                .is_some()
        );
    }

    #[test]
    fn unsupported_modifier_cannot_turn_into_not_exists_authorization() {
        assert!(parse(r#"{"conditions":[{"field":"data|@keys","op":"not_exists"}]}"#).is_err());
        assert!(validate_path(&"a".repeat(MAX_PATH_BYTES + 1)).is_err());
        assert!(parse(r#"{"conditions":[{"field":"x","op":"in","value":false}]}"#).is_err());
    }
}

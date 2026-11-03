//! Public navigation contract shared by the settings writer and Web reader.
use serde_json::Value;

pub(super) fn validate(raw: &str) -> Result<(), &'static str> {
    if raw.len() > 65536 {
        return Err("Navigation configuration exceeds 64 KiB");
    }
    let parsed: Value =
        serde_json::from_str(raw).map_err(|_| "Navigation configuration must be a JSON object")?;
    let fields = parsed
        .as_object()
        .ok_or("Navigation configuration must be a JSON object")?;
    // Unknown extension fields survive editing, as in the Web contract.
    for key in [
        "directory",
        "developers",
        "guide",
        "scripts",
        "webmcp",
        "signal",
        "challenges",
        "home",
        "console",
        "docs",
        "about",
        "pricing",
        "rankings",
        "security",
    ] {
        let Some(value) = fields.get(key) else {
            continue;
        };
        if ["pricing", "rankings", "security"].contains(&key)
            && let Some(access) = value.as_object()
        {
            for field in ["enabled", "requireAuth"] {
                if let Some(value) = access.get(field) {
                    boolean(value)?;
                }
            }
        } else {
            boolean(value)?;
        }
    }
    Ok(())
}

fn boolean(value: &Value) -> Result<(), &'static str> {
    let valid = match value {
        Value::Bool(_) => true,
        Value::Number(n) => matches!(n.as_i64(), Some(0 | 1)),
        Value::String(s) => matches!(
            s.trim().to_ascii_lowercase().as_str(),
            "true" | "false" | "0" | "1"
        ),
        _ => false,
    };
    if valid {
        Ok(())
    } else {
        Err("Navigation switches must be boolean values")
    }
}

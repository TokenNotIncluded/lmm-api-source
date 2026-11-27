//! Report upstream tier metadata without changing the frozen billing speed.
use crate::routing::Speed;
use serde_json::Value;

pub(crate) fn warn_openai_reported_tier(expected: Speed, reported: Option<&Value>) {
    if let Some(tier) = tier_warning(expected, reported) {
        eprintln!(
            "core_model_warning code=openai_service_tier_receipt expected={expected:?} reported={tier}"
        );
    }
}

fn tier_warning(expected: Speed, reported: Option<&Value>) -> Option<&'static str> {
    let value = reported.filter(|value| !value.is_null())?;
    let (label, actual) = match value.as_str() {
        Some("default" | "standard") => ("standard", Some(Speed::Standard)),
        Some("priority" | "fast") => ("priority", Some(Speed::Fast)),
        Some("ultrafast") => ("ultrafast", Some(Speed::Ultrafast)),
        Some("auto") => ("auto", None),
        Some("flex") => ("flex", None),
        Some("scale") => ("scale", None),
        Some(_) => ("unknown", None),
        None => ("invalid_type", None),
    };
    (actual != Some(expected)).then_some(label)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn reports_only_unknown_or_mismatched_tiers() {
        assert_eq!(tier_warning(Speed::Standard, None), None);
        assert_eq!(tier_warning(Speed::Standard, Some(&Value::Null)), None);
        assert_eq!(tier_warning(Speed::Standard, Some(&json!("default"))), None);
        assert_eq!(tier_warning(Speed::Fast, Some(&json!("priority"))), None);
        assert_eq!(
            tier_warning(Speed::Standard, Some(&json!("priority"))),
            Some("priority")
        );
        assert_eq!(
            tier_warning(Speed::Standard, Some(&json!("auto"))),
            Some("auto")
        );
        assert_eq!(
            tier_warning(Speed::Standard, Some(&json!("vendor-specific"))),
            Some("unknown")
        );
        assert_eq!(
            tier_warning(Speed::Standard, Some(&json!(1))),
            Some("invalid_type")
        );
    }
}

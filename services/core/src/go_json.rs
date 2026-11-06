//! Go encoding/json's HTML and line-separator escaping, after JSON serialization.

pub(crate) fn escape(value: &str) -> String {
    let mut out = String::with_capacity(value.len());
    for c in value.chars() {
        match c {
            '&' => out.push_str("\\u0026"),
            '<' => out.push_str("\\u003c"),
            '>' => out.push_str("\\u003e"),
            '\u{2028}' => out.push_str("\\u2028"),
            '\u{2029}' => out.push_str("\\u2029"),
            _ => out.push(c),
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::escape;

    #[test]
    fn preserves_go_json_fingerprint_bytes() {
        assert_eq!(
            escape("{\"a\":\"<&>\u{2028}\u{2029}\",\"n\":9007199254740991}"),
            r#"{"a":"\u003c\u0026\u003e\u2028\u2029","n":9007199254740991}"#,
        );
        for value in [
            "",
            r#"{"quote":"\"","escape":"\u003c","number":1e+3}"#,
            "\"中文🦀\"",
        ] {
            assert_eq!(escape(value), value);
        }
    }
}

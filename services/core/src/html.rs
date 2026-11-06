//! Escaping for HTML text and quoted attribute values.

pub(crate) fn escape(value: &str) -> String {
    let mut out = String::with_capacity(value.len());
    for c in value.chars() {
        match c {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '"' => out.push_str("&quot;"),
            '\'' => out.push_str("&#39;"),
            _ => out.push(c),
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::escape;

    #[test]
    fn text_and_attribute_escaping_preserves_the_existing_representation() {
        for (input, expected) in [
            ("", ""),
            ("plain text 中文 🦀", "plain text 中文 🦀"),
            (
                "<a title=\"O'Reilly & Co.\">",
                "&lt;a title=&quot;O&#39;Reilly &amp; Co.&quot;&gt;",
            ),
            ("&lt; &#39; &amp;", "&amp;lt; &amp;#39; &amp;amp;"),
            ("中文<&>\"'🦀", "中文&lt;&amp;&gt;&quot;&#39;🦀"),
        ] {
            assert_eq!(escape(input), expected);
        }
    }
}

//! Escaping for HTML text and quoted attribute values.

pub(crate) fn escape(value: &str) -> String {
    // Keep the historical decimal spelling of the apostrophe entity.
    html_escape::encode_quoted_attribute(value).replace("&#x27;", "&#39;")
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

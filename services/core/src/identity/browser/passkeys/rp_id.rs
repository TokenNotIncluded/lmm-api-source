//! RP IDs use the verifier's domain relationship and Mozilla's ICANN/private PSL.
use webauthn_rs::prelude::{Url, WebauthnBuilder};

fn canonical_dns(name: &str) -> bool {
    !name.is_empty()
        && name.len() <= 253
        && name.split('.').all(|label| {
            !label.is_empty()
                && label.len() <= 63
                && label.as_bytes()[0].is_ascii_alphanumeric()
                && label.as_bytes()[label.len() - 1].is_ascii_alphanumeric()
                && label
                    .bytes()
                    .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-')
        })
}

pub(super) fn valid(rp_id: &str, origin: &Url) -> bool {
    // Require canonical DNS spelling. URL parsing alone accepts malformed DNS labels.
    if !canonical_dns(rp_id)
        || !origin.domain().is_some_and(canonical_dns)
        || rp_id.parse::<std::net::IpAddr>().is_ok()
    {
        return false;
    }
    // Keep the existing localhost development exception; no localhost parent/suffix.
    if rp_id == "localhost" {
        if origin.domain() != Some("localhost") {
            return false;
        }
    } else if psl::domain(rp_id.as_bytes()).is_none() {
        return false;
    }
    // This also rejects IP origins and sibling hosts using the existing WebAuthn rules.
    WebauthnBuilder::new(rp_id, origin).is_ok()
}

#[cfg(test)]
mod rp69_tests {
    use super::*;

    #[test]
    fn accepts_exact_host_and_registrable_parent() {
        for (origin, rp) in [
            ("https://api.example.com", "api.example.com"),
            ("https://api.example.com", "example.com"),
            ("https://login.example.co.uk", "example.co.uk"),
            ("https://foo.bar.github.io", "bar.github.io"),
            ("https://www.city.kawasaki.jp", "city.kawasaki.jp"),
            ("https://login.xn--bcher-kva.de", "xn--bcher-kva.de"),
            ("http://localhost:8080", "localhost"),
        ] {
            assert!(valid(rp, &Url::parse(origin).unwrap()), "{origin} / {rp}");
        }
    }

    #[test]
    fn rejects_public_private_wildcard_suffixes_and_unrelated_hosts() {
        for (origin, rp) in [
            ("https://evil.localhost", "localhost"),
            ("https://example.com", "com"),
            ("https://com", "com"),
            ("https://login.example.co.uk", "co.uk"),
            ("https://foo.github.io", "github.io"),
            ("https://a.foo.kawasaki.jp", "foo.kawasaki.jp"),
            ("https://api.example.com", "other.example.com"),
            ("https://badexample.com", "example.com"),
            ("https://example.com", "api.example.com"),
            ("https://_bad.example.com", "example.com"),
            ("https://127.0.0.1", "127.0.0.1"),
            ("https://127.0.0.1", "0.0.1"),
            ("https://[::1]", "::1"),
        ] {
            assert!(!valid(rp, &Url::parse(origin).unwrap()), "{origin} / {rp}");
        }
    }

    #[test]
    fn rejects_noncanonical_or_illegal_dns() {
        let origin = Url::parse("https://api.example.com").unwrap();
        for rp in [
            "",
            "EXAMPLE.com",
            ".example.com",
            "example.com.",
            "a..example.com",
            "-a.example.com",
            "a-.example.com",
            "a_b.example.com",
            "*.example.com",
            "https://example.com",
            "example.com:443",
            "bücher.de",
            " example.com",
        ] {
            assert!(!valid(rp, &origin), "{rp}");
        }
        assert!(!valid(&format!("{}.example.com", "a".repeat(64)), &origin));
    }
}
